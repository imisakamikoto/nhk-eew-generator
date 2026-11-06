//! P2P地震情報 JSON API v2 client (code 556 = 緊急地震速報（警報）).
//!
//! HTTPS transport uses the system `curl` binary so the project stays
//! dependency-free and builds without a C toolchain.

use std::collections::HashSet;
use std::process::Command;

use serde_json::Value;

pub const HISTORY_URL: &str =
    "https://api.p2pquake.net/v2/history?codes=556&limit=20";

#[derive(Clone, Debug)]
pub struct EewEvent {
    pub id: String,
    pub event_id: String,
    pub serial: String,
    pub time: String,
    pub cancelled: bool,
    pub test: bool,
    pub lat: f64,
    pub lon: f64,
    pub has_coords: bool,
    pub epicenter: Option<String>,
    pub prefs: Vec<String>,
}

fn child_output() -> Result<Vec<u8>, String> {
    let cmds: &[&[&str]] = &[
        &["curl", "-sS", "--compressed", "--max-time", "20",
          "-H", "User-Agent: eew-generator/1.0"],
    ];
    for base in cmds {
        let mut c = Command::new(base[0]);
        for a in &base[1..] {
            c.arg(a);
        }
        c.arg(HISTORY_URL);
        if let Ok(out) = c.output() {
            if out.status.success() && !out.stdout.is_empty() {
                return Ok(out.stdout);
            }
        }
    }
    Err("HTTP 请求失败（未找到可用的 curl 或网络不可达）".to_string())
}

fn s(v: &Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

fn f(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(|x| x.as_f64()).unwrap_or(-200.0)
}

pub fn fetch() -> Result<Vec<EewEvent>, String> {
    let bytes = child_output()?;
    let text = String::from_utf8_lossy(&bytes);
    parse(&text)
}

/// Parse a P2P地震情報 JSON payload (array of records, or a single record).
/// Shared by the live poller and the offline `--from-json` replay path.
pub fn parse(text: &str) -> Result<Vec<EewEvent>, String> {
    let data: Value = serde_json::from_str(text).map_err(|e| format!("JSON 解析失败: {}", e))?;
    let single;
    let arr = match data.as_array() {
        Some(a) => a,
        None => {
            single = vec![data.clone()];
            &single
        }
    };
    let mut out = Vec::new();
    for item in arr {
        if item.get("code").and_then(|c| c.as_i64()).unwrap_or(0) != 556 {
            continue;
        }
        let issue = item.get("issue").cloned().unwrap_or(Value::Null);
        let quake = item.get("earthquake").cloned().unwrap_or(Value::Null);
        let hypo = quake.get("hypocenter").cloned().unwrap_or(Value::Null);
        let lat = f(&hypo, "latitude");
        let lon = f(&hypo, "longitude");
        let has_coords = lat > -190.0 && lon > -190.0 && lat.abs() <= 90.0 && lon.abs() <= 180.0;
        let epic_name = {
            let n = s(&hypo, "name");
            if n.is_empty() { None } else { Some(n) }
        };
        // Every 細分区域 in a code-556 payload belongs to the warning, so the
        // card lists every unique 府県予報区 in the order the API reports them
        // (this is the order NHK uses as well).
        let mut prefs: Vec<String> = Vec::new();
        let mut seen = HashSet::new();
        if let Some(areas) = item.get("areas").and_then(|a| a.as_array()) {
            for a in areas {
                let pref = s(a, "pref");
                if pref.is_empty() || seen.contains(&pref) {
                    continue;
                }
                seen.insert(pref.clone());
                prefs.push(pref);
            }
        }
        out.push(EewEvent {
            id: s(item, "id"),
            event_id: s(&issue, "eventId"),
            serial: s(&issue, "serial"),
            time: s(item, "time"),
            cancelled: item.get("cancelled").and_then(|c| c.as_bool()).unwrap_or(false),
            test: item.get("test").and_then(|c| c.as_bool()).unwrap_or(false),
            lat: if has_coords { lat } else { 0.0 },
            lon: if has_coords { lon } else { 0.0 },
            has_coords,
            epicenter: epic_name,
            prefs,
        });
    }
    // newest first
    out.sort_by(|a, b| b.time.cmp(&a.time));
    Ok(out)
}

pub fn safe_filename(event: &EewEvent) -> String {
    let raw = format!("{}_{}", event.event_id, event.serial);
    let clean: String = raw.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' })
        .collect();
    if clean.trim_matches('_').is_empty() {
        format!("{}.png", event.id.replace(|c: char| !c.is_ascii_alphanumeric(), "_"))
    } else {
        format!("{}.png", clean.trim_matches('_'))
    }
}
