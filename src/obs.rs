//! OBS 播出联动：共享「当前播出中警报」状态 + 固定路径热更新 PNG。
//!
//! 自动待机线程与 HTTP 服务线程通过 `Shared` 交换一条警报：
//! * auto 出图后调用 `publish()`，递增 seq、原子替换 `output/obs/overlay.png`
//!   （OBS 的「图像源」指向这个固定路径即可自动刷新）
//! * HTTP 侧的 `/obs` 浏览器源轮询 `/api/overlay/state`，靠 seq 判断是否有新报
//!
//! 原子替换很重要：OBS 每秒都在重读这个文件，直接覆盖会读到半张 PNG。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::log;

#[derive(Clone, Debug)]
pub struct OverlayEvent {
    /// 每次发布递增；浏览器源只认比本地 lastSeq 更大的值
    pub seq: u64,
    pub at: String,
    pub cancel: bool,
    pub is_test: bool,
    pub event_id: String,
    pub serial: String,
    pub epicenter: String,
    pub note: String,
    pub lines: Vec<Vec<String>>,
    pub png: Vec<u8>,
}

pub struct Overlay {
    pub seq: u64,
    pub current: Option<OverlayEvent>,
    pub dir: PathBuf,
    /// 手动「立即收起」标志；hidden 也带一个 seq，让浏览器源能感知状态变化
    pub hidden: bool,
}

pub type Shared = Arc<Mutex<Overlay>>;

pub fn shared(dir: PathBuf) -> Shared {
    Arc::new(Mutex::new(Overlay { seq: 0, current: None, hidden: false, dir }))
}

/// Windows 上 `rename` 不能覆盖已存在的文件，所以先删旧文件再改名。
/// 中间的极小空窗由 OBS 下一次读取自然补上，比写入半张 PNG 安全得多。
fn atomic_replace(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_file_name(format!(
        "{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    fs::write(&tmp, data)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

pub fn event(seq: u64, cancel: bool, is_test: bool, event_id: &str, serial: &str,
             epicenter: &str, note: &str, lines: Vec<Vec<String>>, png: Vec<u8>) -> OverlayEvent {
    OverlayEvent {
        seq,
        at: log::now(),
        cancel,
        is_test,
        event_id: event_id.to_string(),
        serial: serial.to_string(),
        epicenter: epicenter.to_string(),
        note: note.to_string(),
        lines,
        png,
    }
}

/// 发布一条警报：写固定路径 PNG / 元数据 JSON，并更新共享状态。
/// 返回本次的 seq，0 表示发布失败。
pub fn publish(sh: &Shared, mut ev: OverlayEvent) -> u64 {
    let mut g = match sh.lock() {
        Ok(g) => g,
        Err(_) => {
            log::warn("[OBS] 状态锁已失效，跳过本次播出推送");
            return 0;
        }
    };
    g.seq += 1;
    ev.seq = g.seq;
    let dir = g.dir.clone();
    let png_path = dir.join("overlay.png");
    if let Err(e) = atomic_replace(&png_path, &ev.png) {
        log::warn(&format!("[OBS] 热更新图写入失败: {}", e));
    }
    // 供外部脚本 / OBS 插件读取的轻量元数据（不含图片）
    let meta = format!(
        "{{\"seq\":{},\"at\":\"{}\",\"cancel\":{},\"test\":{},\"eventId\":\"{}\",\"serial\":\"{}\",\"epicenter\":\"{}\",\"note\":\"{}\"}}",
        ev.seq, ev.at, ev.cancel, ev.is_test,
        crate::server::json_escape(&ev.event_id),
        crate::server::json_escape(&ev.serial),
        crate::server::json_escape(&ev.epicenter),
        crate::server::json_escape(&ev.note));
    let _ = atomic_replace(&dir.join("overlay.json"), meta.as_bytes());
    log::info(&format!(
        "[OBS] 播出推送 seq={} 取消={} 演练={} → {}",
        ev.seq, ev.cancel, ev.is_test, png_path.display()
    ));
    g.hidden = false;
    g.current = Some(ev);
    g.seq
}

/// 立即收起（演练用）：递增 seq 让浏览器源收到一次「隐藏」指令。
pub fn hide(sh: &Shared) -> u64 {
    match sh.lock() {
        Ok(mut g) => {
            g.seq += 1;
            g.hidden = true;
            log::info(&format!("[OBS] 手动收起 seq={}", g.seq));
            g.seq
        }
        Err(_) => 0,
    }
}

pub fn snapshot(sh: &Shared) -> (u64, Option<OverlayEvent>) {
    match sh.lock() {
        Ok(g) => (g.seq, g.current.clone()),
        Err(_) => (0, None),
    }
}

pub fn overlay_png_path(sh: &Shared) -> PathBuf {
    sh.lock().map(|g| g.dir.join("overlay.png")).unwrap_or_else(|_| PathBuf::from("output/obs/overlay.png"))
}

/// 发布并安排 obs-websocket 的「显示 → 保持 → 隐藏」。
/// 隐藏前会核对 seq：若期间又来了新报，旧计时器不许把画面关掉。
/// 取消报按较短时长收起（NHK 的取消画面不会长时间挂着）。
pub fn publish_and_arm(sh: &Shared, ev: OverlayEvent,
                       ws: Option<crate::obsws::Config>, hold: u64) -> u64 {
    let cancel = ev.cancel;
    let seq = publish(sh, ev);
    if seq == 0 {
        return 0;
    }
    if let Some(cfg) = ws {
        if cfg.enabled() {
            let sh2 = sh.clone();
            let dur = if cancel { hold.min(10).max(3) } else { hold.max(3) };
            std::thread::spawn(move || {
                crate::obsws::set_visible(&cfg, true);
                std::thread::sleep(std::time::Duration::from_secs(dur));
                let (cur, _) = snapshot(&sh2);
                if cur == seq {
                    crate::obsws::set_visible(&cfg, false);
                }
            });
        }
    }
    seq
}
