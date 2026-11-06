//! 极简文件日志：同时输出到 stdout 与日志文件，时间戳取系统本地时间。
//!
//! 待机模式以后台隐藏窗口运行，stdout 不可见，因此必须有日志文件才能确认存活。

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

struct Logger {
    file: Option<File>,
    path: PathBuf,
    size: u64,
}

static LOGGER: OnceLock<Mutex<Logger>> = OnceLock::new();

/// 日志超过这个大小就轮转成 `.1`
const MAX_BYTES: u64 = 4 * 1024 * 1024;

pub fn init(path: PathBuf) {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).ok();
    }
    let (file, size) = open(&path);
    let display = path.display().to_string();
    let logger = Logger { file, path, size };
    if LOGGER.set(Mutex::new(logger)).is_ok() {
        info(&format!("日志开始记录: {}", display));
    }
}

fn open(path: &PathBuf) -> (Option<File>, u64) {
    let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let f = OpenOptions::new().create(true).append(true).open(path).ok();
    (f, size)
}

fn rotate(l: &mut Logger) {
    let _ = l.file.take();
    let mut bak = l.path.clone().into_os_string();
    bak.push(".1");
    let bak = PathBuf::from(bak);
    let _ = fs::remove_file(&bak);
    let _ = fs::rename(&l.path, &bak);
    let (f, size) = open(&l.path);
    l.file = f;
    l.size = size;
}

fn emit(level: &str, msg: &str) {
    let line = format!("[{}] [{}] {}", now(), level, msg);
    println!("{}", line);
    if let Some(lock) = LOGGER.get() {
        if let Ok(mut l) = lock.lock() {
            if l.size > MAX_BYTES {
                rotate(&mut l);
            }
            if let Some(f) = l.file.as_mut() {
                let _ = writeln!(f, "{}", line);
                let _ = f.flush();
                l.size += line.len() as u64 + 1;
            }
        }
    }
}

pub fn info(msg: &str) {
    emit("INFO", msg);
}

pub fn warn(msg: &str) {
    emit("WARN", msg);
}

/// 本地时间 `YYYY-MM-DD HH:MM:SS`
pub fn now() -> String {
    let t = local_time();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        t.0, t.1, t.2, t.3, t.4, t.5
    )
}

#[cfg(windows)]
#[repr(C)]
struct SystemTime {
    year: u16,
    month: u16,
    day_of_week: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    milliseconds: u16,
}

#[cfg(windows)]
extern "system" {
    fn GetLocalTime(out: *mut SystemTime);
}

#[cfg(windows)]
fn local_time() -> (u16, u16, u16, u16, u16, u16) {
    let mut st = SystemTime {
        year: 0,
        month: 0,
        day_of_week: 0,
        day: 0,
        hour: 0,
        minute: 0,
        second: 0,
        milliseconds: 0,
    };
    unsafe { GetLocalTime(&mut st) };
    (st.year, st.month, st.day, st.hour, st.minute, st.second)
}

#[cfg(not(windows))]
fn local_time() -> (u16, u16, u16, u16, u16, u16) {
    // 非 Windows 平台回退到 UTC 的粗略换算（本项目主要运行于 Windows）
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    let mut y = 1970i64;
    let mut d = days as i64;
    loop {
        let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        let n = if leap { 366 } else { 365 };
        if d < n {
            break;
        }
        d -= n;
        y += 1;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let ml = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut m = 0usize;
    while d >= ml[m] {
        d -= ml[m];
        m += 1;
    }
    (
        y as u16,
        (m + 1) as u16,
        (d + 1) as u16,
        ((secs % 86_400) / 3600) as u16,
        ((secs % 3600) / 60) as u16,
        (secs % 60) as u16,
    )
}
