//! Standby mode: poll P2P地震情報 for new code-556 reports and write PNGs.
//!
//! 设计要点（后台常驻）：
//! * 单次巡检包在 `catch_unwind` 里，任何解析/绘制异常都不会让待机线程退出
//! * 网络错误只在「健康→故障」和「故障→恢复」时各记一次，避免日志刷屏
//! * 每 20 分钟写一条心跳，便于确认进程还活着

use std::collections::{HashSet, VecDeque};
use std::fs;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use crate::data::{load_epicenters, load_geo, Epicenter, Geo};
use crate::font::Font;
use crate::log;
use crate::net::{self, EewEvent};
use crate::obs;
use crate::obsws;
use crate::render::{self, Input, FONT_HEAVY, FONT_MEDIUM};

const GEO_BIN: &[u8] = include_bytes!("../data/geometry.bin");
const EPICENTER_BIN: &[u8] = include_bytes!("../data/epicenters.bin");

pub struct AutoState {
    pub out_dir: PathBuf,
    pub interval: u64,
    pub include_test: bool,
    pub process_latest_on_start: bool,
    pub overlay: obs::Shared,
    pub obs_ws: Option<obsws::Config>,
    pub obs_hold: u64,
}

/// 生成一张（或一张取消画面）。成功时顺带返回一份可直接推给 OBS 的播出事件；
/// 离线回放调用方不需要这份事件时忽略返回值即可。
pub fn write_png(dir: &PathBuf, event: &EewEvent, geo: &Geo, eps: &[Epicenter],
                 heavy: &Font, medium: &Font) -> Option<obs::OverlayEvent> {
    let prefs = geo.normalize_prefs(&event.prefs);
    let input = Input {
        lat: if event.has_coords { event.lat } else { 35.0 },
        lon: if event.has_coords { event.lon } else { 135.0 },
        prefs,
        epicenter: event.epicenter.clone(),
        cancel: event.cancelled,
    };
    let out = render::render(&input, geo, eps, heavy, medium);
    let name = net::safe_filename(event);
    let path = dir.join(&name);
    match fs::write(&path, &out.png) {
        Ok(_) => {
            log::info(&format!(
                "[自动] {}  (eventId={} serial={} 取消={} 区域={}) {}",
                name, event.event_id, event.serial, event.cancelled,
                out.lines.iter().flatten().cloned().collect::<Vec<_>>().join("·"),
                out.note
            ));
            let _ = fs::write(dir.join("latest.png"), &out.png);
            Some(obs::event(
                0, event.cancelled, event.test,
                &event.event_id, &event.serial,
                &out.epicenter, &out.note, out.lines.clone(), out.png.clone(),
            ))
        }
        Err(e) => {
            log::warn(&format!("[自动] 写入失败 {}: {}", path.display(), e));
            None
        }
    }
}

struct Ctx {
    geo: Geo,
    eps: Vec<Epicenter>,
    heavy: Font,
    medium: Font,
    seen: HashSet<String>,
    order: VecDeque<String>,
    first: bool,
}

/// 一次巡检：拉取 → 去重 → 出图。返回本次新生成的张数；错误通过 Err 返回。
fn tick(ctx: &mut Ctx, state: &AutoState) -> Result<usize, String> {
    let events = net::fetch()?;
    let mut written = 0usize;
    if ctx.first {
        if state.process_latest_on_start {
            if let Some(e) = events.first() {
                if !(e.test && !state.include_test) {
                    if let Some(ev) = write_png(&state.out_dir, e, &ctx.geo, &ctx.eps, &ctx.heavy, &ctx.medium) {
                        written += 1;
                        obs::publish_and_arm(&state.overlay, ev, state.obs_ws.clone(), state.obs_hold);
                    }
                }
            }
        } else {
            log::info(&format!("[自动] 已同步 {} 条历史记录，等待新报…", events.len()));
        }
        for e in &events {
            ctx.seen.insert(e.id.clone());
            ctx.order.push_back(e.id.clone());
        }
        ctx.first = false;
    } else {
        for e in &events {
            if ctx.seen.contains(&e.id) {
                continue;
            }
            ctx.seen.insert(e.id.clone());
            ctx.order.push_back(e.id.clone());
            if e.test && !state.include_test {
                log::info(&format!("[自动] 跳过测试报 {}", e.id));
                continue;
            }
            if let Some(ev) = write_png(&state.out_dir, e, &ctx.geo, &ctx.eps, &ctx.heavy, &ctx.medium) {
                written += 1;
                obs::publish_and_arm(&state.overlay, ev, state.obs_ws.clone(), state.obs_hold);
            }
        }
    }
    while ctx.order.len() > 400 {
        if let Some(old) = ctx.order.pop_front() {
            ctx.seen.remove(&old);
        }
    }
    Ok(written)
}

pub fn run(state: AutoState) {
    fs::create_dir_all(&state.out_dir).ok();
    let mut ctx = Ctx {
        geo: load_geo(GEO_BIN),
        eps: load_epicenters(EPICENTER_BIN),
        heavy: Font::new(FONT_HEAVY),
        medium: Font::new(FONT_MEDIUM),
        seen: HashSet::new(),
        order: VecDeque::new(),
        first: true,
    };
    let started = Instant::now();
    let mut polls: u64 = 0;
    let mut total: u64 = 0;
    let mut failing = false;
    let mut last_beat = Instant::now();

    log::info(&format!(
        "[自动] 待机中：每 {} 秒检查一次 code=556（緊急地震速報・警報）",
        state.interval
    ));
    log::info(&format!("[自动] 输出目录: {}", state.out_dir.display()));

    loop {
        polls += 1;
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| tick(&mut ctx, &state)));
        match result {
            Ok(Ok(n)) => {
                total += n as u64;
                if failing {
                    log::info("[自动] 网络/接口已恢复");
                    failing = false;
                }
            }
            Ok(Err(e)) => {
                if !failing {
                    log::warn(&format!("[自动] 取数失败（将持续重试）: {}", e));
                    failing = true;
                }
            }
            Err(_) => {
                log::warn("[自动] 本次巡检出现异常，已捕获并继续待机");
            }
        }

        if last_beat.elapsed() >= Duration::from_secs(20 * 60) {
            last_beat = Instant::now();
            log::info(&format!(
                "[自动] 心跳：已运行 {}，轮询 {} 次，累计出图 {} 张，状态={}",
                human_duration(started.elapsed()),
                polls,
                total,
                if failing { "取数异常" } else { "正常" }
            ));
        }

        thread::sleep(Duration::from_secs(state.interval.max(2)));
    }
}

fn human_duration(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}天{}小时{}分", s / 86_400, (s % 86_400) / 3600, (s % 3600) / 60)
}
