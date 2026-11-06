//! Standby mode: poll P2P地震情報 for new code-556 reports and write PNGs.

use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use crate::data::{load_epicenters, load_geo, Epicenter, Geo};
use crate::font::Font;
use crate::net::{self, EewEvent};
use crate::render::{self, Input, FONT_HEAVY, FONT_MEDIUM};

const GEO_BIN: &[u8] = include_bytes!("../data/geometry.bin");
const EPICENTER_BIN: &[u8] = include_bytes!("../data/epicenters.bin");

pub struct AutoState {
    pub out_dir: PathBuf,
    pub interval: u64,
    pub include_test: bool,
    pub process_latest_on_start: bool,
}

pub fn write_png(dir: &PathBuf, event: &EewEvent, geo: &Geo, eps: &[Epicenter],
                 heavy: &Font, medium: &Font) {
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
            println!("[自动] {}  (eventId={} serial={} 取消={} 区域={}) {}",
                     name, event.event_id, event.serial, event.cancelled,
                     out.lines.iter().flatten().cloned().collect::<Vec<_>>().join("·"),
                     out.note);
            let _ = fs::write(dir.join("latest.png"), &out.png);
        }
        Err(e) => eprintln!("[自动] 写入失败 {}: {}", path.display(), e),
    }
}

pub fn run(state: AutoState) {
    fs::create_dir_all(&state.out_dir).ok();
    let geo: Geo = load_geo(GEO_BIN);
    let eps: Vec<Epicenter> = load_epicenters(EPICENTER_BIN);
    let heavy = Font::new(FONT_HEAVY);
    let medium = Font::new(FONT_MEDIUM);
    let mut seen: HashSet<String> = HashSet::new();
    let mut order: VecDeque<String> = VecDeque::new();
    let mut first = true;
    println!("[自动] 待机中：每 {} 秒检查一次 code=556（緊急地震速報・警報）", state.interval);
    println!("[自动] 输出目录: {}", state.out_dir.display());
    loop {
        match net::fetch() {
            Ok(events) => {
                if first {
                    if state.process_latest_on_start {
                        if let Some(e) = events.first() {
                            if !(e.test && !state.include_test) {
                                write_png(&state.out_dir, e, &geo, &eps, &heavy, &medium);
                            }
                        }
                    } else {
                        println!("[自动] 已同步 {} 条历史记录，等待新报…", events.len());
                    }
                    for e in &events {
                        seen.insert(e.id.clone());
                        order.push_back(e.id.clone());
                    }
                    first = false;
                } else {
                    for e in &events {
                        if seen.contains(&e.id) {
                            continue;
                        }
                        seen.insert(e.id.clone());
                        order.push_back(e.id.clone());
                        if e.test && !state.include_test {
                            println!("[自动] 跳过测试报 {}", e.id);
                            continue;
                        }
                        write_png(&state.out_dir, e, &geo, &eps, &heavy, &medium);
                    }
                }
                while order.len() > 400 {
                    if let Some(old) = order.pop_front() {
                        seen.remove(&old);
                    }
                }
            }
            Err(e) => eprintln!("[自动] {}", e),
        }
        thread::sleep(Duration::from_secs(state.interval.max(2)));
    }
}
