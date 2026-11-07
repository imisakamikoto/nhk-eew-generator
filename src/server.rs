//! Tiny dependency-free HTTP server for the manual generation UI and OBS overlay.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use crate::data::{load_epicenters, load_geo, Epicenter, Geo};
use crate::font::Font;
use crate::log;
use crate::obs;
use crate::obsws;
use crate::render::{self, Input, FONT_HEAVY, FONT_MEDIUM};

const GEO_BIN: &[u8] = include_bytes!("../data/geometry.bin");
const EPICENTER_BIN: &[u8] = include_bytes!("../data/epicenters.bin");

// ---- PWA 资源（内嵌，无外部依赖） ----
const ICON_192: &[u8] = include_bytes!("../assets/web/icon-192.png");
const ICON_512: &[u8] = include_bytes!("../assets/web/icon-512.png");
const ICON_MASKABLE: &[u8] = include_bytes!("../assets/web/icon-maskable-512.png");
const ICON_APPLE: &[u8] = include_bytes!("../assets/web/apple-touch-icon.png");
const ICON_FAVICON: &[u8] = include_bytes!("../assets/web/favicon.png");

const MANIFEST: &str = r##"{
  "name": "緊急地震速報 画像ジェネレーター",
  "short_name": "EEW",
  "description": "仿 NHK 緊急地震速報 画面生成器：输入震源经纬度与警报府县即可出图",
  "lang": "zh-CN",
  "dir": "ltr",
  "start_url": "/",
  "scope": "/",
  "id": "/",
  "display": "standalone",
  "display_override": ["window-controls-overlay", "standalone", "minimal-ui"],
  "orientation": "any",
  "background_color": "#0b1220",
  "theme_color": "#10151f",
  "categories": ["utilities", "productivity"],
  "icons": [
    { "src": "/icon-192.png", "sizes": "192x192", "type": "image/png", "purpose": "any" },
    { "src": "/icon-512.png", "sizes": "512x512", "type": "image/png", "purpose": "any" },
    { "src": "/icon-maskable-512.png", "sizes": "512x512", "type": "image/png", "purpose": "maskable" }
  ],
  "shortcuts": [
    { "name": "示例：福岡県北西沖", "short_name": "示例", "url": "/?demo=1" }
  ]
}"##;

const SW_JS: &str = r#"const CACHE = 'eew-shell-v1';
const SHELL = ['/', '/manifest.webmanifest', '/icon-192.png', '/icon-512.png', '/icon-maskable-512.png', '/apple-touch-icon.png'];

self.addEventListener('install', e => {
  e.waitUntil(caches.open(CACHE).then(c => c.addAll(SHELL)).then(() => self.skipWaiting()));
});

self.addEventListener('activate', e => {
  e.waitUntil(
    caches.keys()
      .then(ks => Promise.all(ks.filter(k => k !== CACHE).map(k => caches.delete(k))))
      .then(() => self.clients.claim())
  );
});

self.addEventListener('fetch', e => {
  const req = e.request;
  const url = new URL(req.url);
  if (url.origin !== location.origin) return;

  if (req.method !== 'GET') {
    if (url.pathname === '/api/render') {
      e.respondWith(fetch(req).catch(() => new Response(
        JSON.stringify({ ok: false, note: '离线：本地生成服务未运行' }),
        { status: 503, headers: { 'Content-Type': 'application/json' } }
      )));
    }
    return;
  }

  if (req.mode === 'navigate') {
    e.respondWith(
      fetch(req)
        .then(r => { const cp = r.clone(); caches.open(CACHE).then(c => c.put('/', cp)); return r; })
        .catch(() => caches.match('/'))
    );
    return;
  }

  e.respondWith(
    caches.match(req).then(hit => hit || fetch(req).then(r => {
      const cp = r.clone();
      caches.open(CACHE).then(c => c.put(req, cp));
      return r;
    }))
  );
});
"#;


pub struct ServerState {
    pub port: u16,
    pub out_dir: PathBuf,
    pub auto_enabled: bool,
    pub overlay: obs::Shared,
    pub obs_ws: Option<obsws::Config>,
    pub obs_hold: u64,
}

struct ServerCtx {
    geo: Geo,
    eps: Vec<Epicenter>,
    heavy: Font,
    medium: Font,
    state: ServerState,
}

pub fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity((data.len() + 2) / 3 * 4);
    for c in data.chunks(3) {
        let n = ((c[0] as u32) << 16)
            | ((c.get(1).copied().unwrap_or(0) as u32) << 8)
            | c.get(2).copied().unwrap_or(0) as u32;
        s.push(T[((n >> 18) & 63) as usize] as char);
        s.push(T[((n >> 12) & 63) as usize] as char);
        s.push(if c.len() > 1 { T[((n >> 6) & 63) as usize] as char } else { '=' });
        s.push(if c.len() > 2 { T[(n & 63) as usize] as char } else { '=' });
    }
    s
}

pub fn json_escape(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => { out.push(b' '); i += 1; }
            b'%' if i + 2 < bytes.len() => {
                let h = |b: u8| -> Option<u8> {
                    match b {
                        b'0'..=b'9' => Some(b - b'0'),
                        b'a'..=b'f' => Some(b - b'a' + 10),
                        b'A'..=b'F' => Some(b - b'A' + 10),
                        _ => None,
                    }
                };
                if let (Some(a), Some(b)) = (h(bytes[i + 1]), h(bytes[i + 2])) {
                    out.push(a * 16 + b);
                    i += 3;
                } else {
                    out.push(bytes[i]); i += 1;
                }
            }
            b => { out.push(b); i += 1; }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_form(body: &str) -> HashMap<String, String> {
    let mut m = HashMap::new();
    for part in body.split('&') {
        if part.is_empty() { continue; }
        let mut it = part.splitn(2, '=');
        let k = percent_decode(it.next().unwrap_or(""));
        let v = percent_decode(it.next().unwrap_or(""));
        m.insert(k, v);
    }
    m
}

fn pref_html(geo: &Geo) -> String {
    let mut html = String::new();
    for (ri, region) in geo.regions.iter().enumerate() {
        let list: Vec<&str> = geo.prefs.iter()
            .filter(|p| p.parent == Some(ri))
            .map(|p| p.name.as_str())
            .collect();
        if list.is_empty() { continue; }
        html.push_str(&format!(
            r#"<section class="grp"><h3><label><input type="checkbox" class="grpall" data-g="g{}"> {}</label></h3><div class="items" id="g{}">"#,
            ri, region.name, ri));
        for (i, n) in list.iter().enumerate() {
            html.push_str(&format!(
                r#"<label class="item"><input type="checkbox" name="pref" value="{}" id="p{}_{}"> {}</label>"#,
                n, ri, i, n));
        }
        html.push_str("</div></section>");
    }
    html
}

fn page(geo: &Geo, state: &ServerState) -> String {
    let auto = if state.auto_enabled {
        format!("实时待机已开启 · 输出目录 {}", state.out_dir.display())
    } else {
        "实时待机未开启（启动时加 --auto 开启）".to_string()
    };
    HTML.replace("__PREFS__", &pref_html(geo))
        .replace("__AUTO__", &auto)
        .replace("__PORT__", &state.port.to_string())
        .replace("__HOLDMSG__", &state.obs_hold.to_string())
}

fn obs_page(state: &ServerState) -> String {
    OBS_HTML.replace("__HOLD__", &state.obs_hold.to_string())
}

/// 当前播出状态的轻量 JSON（不含图片），浏览器源每 250ms 拉一次。
fn overlay_state_json(sh: &obs::Shared) -> String {
    let (seq, hidden, cur) = match sh.lock() {
        Ok(g) => (g.seq, g.hidden, g.current.clone()),
        Err(_) => return "{\"seq\":0,\"hidden\":true,\"has\":false}".to_string(),
    };
    let lines = cur.as_ref().map(|e| {
        e.lines.iter()
            .map(|l| format!("[{}]", l.iter().map(|s| format!("\"{}\"", json_escape(s)))
                .collect::<Vec<_>>().join(",")))
            .collect::<Vec<_>>().join(",")
    }).unwrap_or_default();
    match cur {
        Some(e) => format!(
            "{{\"seq\":{},\"hidden\":{},\"has\":true,\"cancel\":{},\"test\":{},\"at\":\"{}\",\"eventId\":\"{}\",\"serial\":\"{}\",\"epicenter\":\"{}\",\"note\":\"{}\",\"lines\":[{}]}}",
            seq, hidden, e.cancel, e.is_test,
            json_escape(&e.at), json_escape(&e.event_id), json_escape(&e.serial),
            json_escape(&e.epicenter), json_escape(&e.note), lines),
        None => format!("{{\"seq\":{},\"hidden\":{},\"has\":false}}", seq, hidden),
    }
}

fn respond_cached(stream: &mut TcpStream, code: &str, ctype: &str, cache: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: {}\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
        code, ctype, body.len(), cache);
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

fn respond(stream: &mut TcpStream, code: &str, ctype: &str, body: &[u8]) {
    respond_cached(stream, code, ctype, "no-store", body);
}

fn handle(mut stream: TcpStream, ctx: &Arc<ServerCtx>) {
    let state = &ctx.state;
    let geo = &ctx.geo;
    let eps = &ctx.eps;
    let heavy = &ctx.heavy;
    let medium = &ctx.medium;
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let header_end;
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => return,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    header_end = i + 4;
                    break;
                }
                if buf.len() > 1 << 20 { return; }
            }
            Err(_) => return,
        }
    }
    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let first = lines.next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let raw_path = parts.next().unwrap_or("/").to_string();
    let (path, _query) = match raw_path.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (raw_path, String::new()),
    };
    let mut content_len = 0usize;
    for l in lines {
        if let Some(v) = l.to_ascii_lowercase().strip_prefix("content-length:") {
            content_len = v.trim().parse().unwrap_or(0);
        }
    }
    while buf.len() < header_end + content_len {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(_) => break,
        }
    }
    let body = String::from_utf8_lossy(&buf[header_end..header_end + content_len.min(buf.len() - header_end)]).into_owned();

    if method == "GET" && (path == "/" || path == "/index.html") {
        let html = page(geo, state);
        respond(&mut stream, "200 OK", "text/html; charset=utf-8", html.as_bytes());
        return;
    }
    // ---- PWA ----
    if method == "GET" {
        let pwa: Option<(&[u8], &str, &str)> = match path.as_str() {
            "/manifest.webmanifest" | "/manifest.json" => {
                Some((MANIFEST.as_bytes(), "application/manifest+json; charset=utf-8", "no-cache"))
            }
            // 必须允许频繁刷新，否则新版本 SW 无法生效
            "/sw.js" => Some((SW_JS.as_bytes(), "text/javascript; charset=utf-8", "no-cache")),
            "/icon-192.png" => Some((ICON_192, "image/png", "public, max-age=604800")),
            "/icon-512.png" => Some((ICON_512, "image/png", "public, max-age=604800")),
            "/icon-maskable-512.png" => Some((ICON_MASKABLE, "image/png", "public, max-age=604800")),
            "/apple-touch-icon.png" => Some((ICON_APPLE, "image/png", "public, max-age=604800")),
            "/favicon.ico" | "/favicon.png" => Some((ICON_FAVICON, "image/png", "public, max-age=604800")),
            _ => None,
        };
        if let Some((body, ctype, cache)) = pwa {
            respond_cached(&mut stream, "200 OK", ctype, cache, body);
            return;
        }
    }
    // ---- OBS 播出联动 ----
    if method == "GET" && path == "/obs" {
        respond(&mut stream, "200 OK", "text/html; charset=utf-8", obs_page(state).as_bytes());
        return;
    }
    if method == "GET" && path == "/api/overlay/state" {
        respond(&mut stream, "200 OK", "application/json; charset=utf-8",
                overlay_state_json(&state.overlay).as_bytes());
        return;
    }
    if method == "GET" && path == "/api/overlay/png" {
        let cur = match state.overlay.lock() { Ok(g) => g.current.clone(), Err(_) => None };
        match cur {
            Some(e) => respond_cached(&mut stream, "200 OK", "image/png", "no-store", &e.png),
            None => respond(&mut stream, "404 Not Found", "text/plain; charset=utf-8", b"no alert"),
        }
        return;
    }
    if method == "POST" && path == "/api/overlay/test" {
        let f = parse_form(&body);
        let cancel = f.get("mode").map(|v| v == "cancel").unwrap_or(false);
        let lat = f.get("lat").and_then(|v| v.parse::<f64>().ok()).unwrap_or(34.446);
        let lon = f.get("lon").and_then(|v| v.parse::<f64>().ok()).unwrap_or(129.065);
        let name = f.get("name").cloned().unwrap_or_default();
        let raw: Vec<String> = f.get("prefs")
            .map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
            .unwrap_or_else(|| vec!["福岡", "長崎", "佐賀", "大分", "山口", "熊本"]
                .into_iter().map(String::from).collect());
        let input = Input {
            lat, lon,
            prefs: geo.normalize_prefs(&raw),
            epicenter: if name.trim().is_empty() { None } else { Some(name.trim().to_string()) },
            cancel,
        };
        let out = render::render(&input, geo, eps, heavy, medium);
        let (lines, png, epic, note) = (out.lines, out.png, out.epicenter, out.note);
        let ev = obs::event(0, cancel, true, "REHEARSAL", "1", &epic, &note, lines, png);
        let seq = obs::publish_and_arm(&state.overlay, ev, state.obs_ws.clone(), state.obs_hold);
        respond(&mut stream, "200 OK", "application/json; charset=utf-8",
                format!("{{\"ok\":true,\"seq\":{}}}", seq).as_bytes());
        return;
    }
    if method == "POST" && path == "/api/overlay/hide" {
        let seq = obs::hide(&state.overlay);
        respond(&mut stream, "200 OK", "application/json; charset=utf-8",
                format!("{{\"ok\":true,\"seq\":{}}}", seq).as_bytes());
        return;
    }
    if method == "POST" && path == "/api/render" {
        let f = parse_form(&body);
        let lat = f.get("lat").and_then(|v| v.parse::<f64>().ok()).unwrap_or(34.446);
        let lon = f.get("lon").and_then(|v| v.parse::<f64>().ok()).unwrap_or(129.065);
        let cancel = f.get("mode").map(|v| v == "cancel").unwrap_or(false);
        let name = f.get("name").cloned().unwrap_or_default();
        let raw: Vec<String> = f.get("prefs").map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default();
        let prefs = geo.normalize_prefs(&raw);
        let input = Input {
            lat, lon,
            prefs,
            epicenter: if name.trim().is_empty() { None } else { Some(name.trim().to_string()) },
            cancel,
        };
        let out = render::render(&input, geo, eps, heavy, medium);
        let lines = out.lines.iter()
            .map(|l| format!("[{}]", l.iter().map(|s| format!("\"{}\"", json_escape(s))).collect::<Vec<_>>().join(",")))
            .collect::<Vec<_>>().join(",");
        let json = format!(
            "{{\"ok\":true,\"png\":\"{}\",\"note\":\"{}\",\"epicenter\":\"{}\",\"consolidated\":{},\"lines\":[{}]}}",
            b64(&out.png), json_escape(&out.note), json_escape(&out.epicenter), out.consolidated, lines);
        respond(&mut stream, "200 OK", "application/json; charset=utf-8", json.as_bytes());
        return;
    }
    respond(&mut stream, "404 Not Found", "text/plain; charset=utf-8", b"not found");
}

pub fn run(mut state: ServerState) {
    let geo = load_geo(GEO_BIN);
    let eps = load_epicenters(EPICENTER_BIN);
    let heavy = Font::new(FONT_HEAVY);
    let medium = Font::new(FONT_MEDIUM);
    let mut listener = None;
    for p in state.port..state.port + 12 {
        if let Ok(l) = TcpListener::bind(("127.0.0.1", p)) {
            state.port = p;
            listener = Some(l);
            break;
        }
    }
    let listener = listener.unwrap_or_else(|| panic!("无法在 {}~{} 找到空闲端口", state.port, state.port + 11));
    // 浏览器源会持续轮询，必须并发处理，否则一个连接就把整个服务堵死
    let ctx = Arc::new(ServerCtx { geo, eps, heavy, medium, state });
    log::info(&format!("[手动] 界面已就绪: http://127.0.0.1:{}", ctx.state.port));
    log::info(&format!("[OBS] 浏览器源: http://127.0.0.1:{}/obs", ctx.state.port));
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let c = ctx.clone();
                thread::spawn(move || handle(s, &c));
            }
            Err(e) => log::warn(&format!("[手动] 连接错误: {}", e)),
        }
    }
}

/// OBS 浏览器源：透明背景、瞬间硬切显示/消失、无字幕。
/// NHK 的速报是「啪」地一下覆盖到节目画面上，所以这里刻意不写任何
/// transition / animation / opacity 渐变；图片预加载完成后直接 display:block。
const OBS_HTML: &str = r##"<!DOCTYPE html>
<html lang="ja"><head><meta charset="utf-8">
<title>EEW Overlay</title>
<style>
html,body{margin:0;padding:0;width:100%;height:100%;background:transparent;overflow:hidden}
#card{position:absolute;left:50%;top:70%;width:63%;transform:translate(-50%,-50%);display:none}
#card img{display:block;width:100%;height:auto}
body.preview{background:#0d1b2a}
body.preview #card{filter:drop-shadow(0 18px 40px rgba(0,0,0,.55))}
#hint{display:none;position:fixed;left:14px;bottom:12px;font:12px/1.5 monospace;color:#8fb3d9;opacity:.7}
body.preview #hint{display:block}
</style></head><body>
<div id="card"><img id="shot" alt=""></div>
<div id="hint">PREVIEW MODE — 背景与这行文字在 OBS 里不会出现</div>
<script>
const q=new URLSearchParams(location.search);
const DEF_HOLD=__HOLD__;
const HOLD=q.has('hold')?Math.max(1,parseInt(q.get('hold'))||DEF_HOLD):DEF_HOLD;
const SOUND=q.get('sound')!=='0'&&q.get('sound')!=='false';
const W=q.get('full')==='1'?100:Math.min(100,Math.max(15,parseFloat(q.get('w')||'63')||63));
const Y=Math.min(100,Math.max(0,parseFloat(q.get('y')||'70')||70));
const REPLAY=q.get('replay')==='1', PREVIEW=q.get('preview')==='1', POLL=250;
const card=document.getElementById('card'), shot=document.getElementById('shot');
card.style.width=W+'%'; card.style.top=Y+'%';
if(PREVIEW)document.body.classList.add('preview');
let last=-1, hideTimer=null, ac=null, first=true;

function hideNow(){clearTimeout(hideTimer);card.style.display='none'}
function chime(){
  if(!SOUND)return;
  try{
    const AC=window.AudioContext||window.webkitAudioContext;
    if(!AC)return;
    if(!ac)ac=new AC();
    if(ac.state==='suspended')ac.resume();
    const t=ac.currentTime+0.005;
    // NHK 緊急地震速報のチャイム：上行二音を二度
    [[880,0],[1174.66,0.22],[880,0.55],[1174.66,0.77]].forEach(([f,dt])=>{
      const o=ac.createOscillator(), h=ac.createOscillator(), g=ac.createGain(), hg=ac.createGain();
      o.type='sine';o.frequency.setValueAtTime(f,t+dt);
      h.type='sine';h.frequency.setValueAtTime(f*2,t+dt);hg.gain.value=.16;
      h.connect(hg).connect(g);o.connect(g);g.connect(ac.destination);
      g.gain.setValueAtTime(.0001,t+dt);
      g.gain.exponentialRampToValueAtTime(.22,t+dt+.008);
      g.gain.exponentialRampToValueAtTime(.0001,t+dt+.19);
      o.start(t+dt);o.stop(t+dt+.21);h.start(t+dt);h.stop(t+dt+.21);
    });
  }catch(e){}
}
function show(s){
  const url='/api/overlay/png?v='+s.seq;
  const pre=new Image();
  pre.onload=()=>{
    if(s.seq!==last)return;            // 加载期间又来新报，丢弃这一帧
    shot.src=url;
    card.style.display='block';        // 瞬间硬切，无过渡
    chime();
    clearTimeout(hideTimer);
    hideTimer=setTimeout(hideNow,(s.cancel?Math.min(HOLD,10):HOLD)*1000);
  };
  pre.onerror=()=>{};
  pre.src=url;
}
async function poll(){
  try{
    const r=await fetch('/api/overlay/state?t='+Date.now(),{cache:'no-store'});
    const s=await r.json();
    if(first){
      first=false;last=s.seq;          // 浏览器源重启时不重播旧警报
      if(REPLAY&&s.has&&!s.hidden)show(s);
    }else if(s.seq>last){
      last=s.seq;
      if(s.hidden)hideNow();else if(s.has)show(s);
    }
  }catch(e){}
  setTimeout(poll,POLL);
}
// 预览时点一下可解锁音频自动播放（OBS CEF 通常无需）
addEventListener('pointerdown',()=>{if(ac&&ac.state==='suspended')ac.resume()},{once:true});
poll();
</script></body></html>"##;

const HTML: &str = r##"<!DOCTYPE html>
<html lang="zh-CN"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<title>緊急地震速報ジェネレーター</title>
<link rel="manifest" href="/manifest.webmanifest">
<meta name="theme-color" content="#10151f">
<meta name="color-scheme" content="dark">
<link rel="icon" type="image/png" sizes="192x192" href="/icon-192.png">
<link rel="apple-touch-icon" href="/apple-touch-icon.png">
<meta name="mobile-web-app-capable" content="yes">
<meta name="apple-mobile-web-app-capable" content="yes">
<meta name="apple-mobile-web-app-status-bar-style" content="black-translucent">
<meta name="apple-mobile-web-app-title" content="EEW">
<style>
:root{--bg:#10151f;--panel:#1a2230;--line:#2c3849;--fg:#e8eef7;--sub:#93a3b8;--red:#e63960;--blue:#3b73c6;--yellow:#f7fc5f}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--fg);font:14px/1.6 "Segoe UI","Microsoft YaHei",sans-serif}
header{padding:18px 26px;border-bottom:1px solid var(--line);display:flex;align-items:center;gap:14px;flex-wrap:wrap}
header h1{font-size:18px;margin:0;letter-spacing:.5px}header .tag{font-size:12px;color:var(--sub);border:1px solid var(--line);padding:2px 9px;border-radius:20px}
.wrap{display:grid;grid-template-columns:minmax(340px,430px) 1fr;gap:20px;padding:20px 26px;max-width:1600px;margin:0 auto}
.card{background:var(--panel);border:1px solid var(--line);border-radius:12px;padding:18px}
.card h2{font-size:14px;margin:0 0 14px;color:var(--sub);font-weight:600;text-transform:uppercase;letter-spacing:1px}
.row{display:flex;gap:10px;margin-bottom:12px}.row>div{flex:1}
label.f{display:block;font-size:12px;color:var(--sub);margin-bottom:4px}
input[type=text],input[type=number]{width:100%;background:#0e1420;border:1px solid var(--line);color:var(--fg);border-radius:7px;padding:8px 10px;font-size:14px;outline:none}
input:focus{border-color:var(--blue)}
.modes{display:flex;gap:8px;margin:4px 0 16px}.modes label{flex:1;text-align:center;border:1px solid var(--line);border-radius:7px;padding:8px;cursor:pointer;color:var(--sub)}
.modes input{display:none}.modes input:checked+span{color:var(--fg)}.modes label:has(input:checked){border-color:var(--red);background:#2a1720}
.tools{display:flex;gap:8px;flex-wrap:wrap;margin-bottom:10px}
button{background:#243044;color:var(--fg);border:1px solid var(--line);border-radius:7px;padding:7px 13px;cursor:pointer;font-size:13px}
button:hover{border-color:var(--blue)}button.primary{background:var(--red);border-color:var(--red);font-weight:600}
button.primary:hover{filter:brightness(1.12)}
.areas{max-height:330px;overflow:auto;border:1px solid var(--line);border-radius:8px;padding:10px;background:#0e1420}
.grp{margin-bottom:10px}.grp h3{font-size:13px;margin:0 0 6px;color:var(--fg);font-weight:600}
.grp h3 label{cursor:pointer}.items{display:flex;flex-wrap:wrap;gap:5px 12px}
.item{font-size:13px;color:var(--sub);cursor:pointer;white-space:nowrap}.item input{accent-color:var(--red);margin-right:4px}
.preview{background:#0b1019;border:1px solid var(--line);border-radius:10px;padding:16px;min-height:220px;display:flex;flex-direction:column;align-items:center;justify-content:center}
.preview img{max-width:100%;border-radius:6px;display:block;box-shadow:0 10px 40px #0008}
.note{margin-top:12px;font-size:13px;color:var(--sub);text-align:center;width:100%}
.note b{color:var(--yellow)}
.foot{padding:0 26px 24px;color:var(--sub);font-size:12px}
#install{margin-left:auto;background:transparent;border-color:var(--yellow);color:var(--yellow)}
#install[hidden]{display:none}
@media (max-width:860px){
  .wrap{grid-template-columns:1fr;padding:14px;gap:14px}
  header{padding:14px 16px 14px calc(16px + env(safe-area-inset-left))}
  header h1{font-size:16px}
  .foot{padding:0 16px calc(20px + env(safe-area-inset-bottom))}
  .areas{max-height:44vh}
  body{overscroll-behavior-y:contain}
}
@media (display-mode:standalone){
  header{padding-top:calc(14px + env(safe-area-inset-top))}
}
</style></head><body>
<header><h1>緊急地震速報 画像ジェネレーター</h1><span class="tag">Rust / P2P地震情報 v2</span><span class="tag" id="autotag">__AUTO__</span><button id="install" hidden>安装到桌面</button></header>
<div class="wrap">
 <div class="card">
  <h2>手動生成</h2>
  <div class="row"><div><label class="f">震源 緯度 (lat)</label><input type="number" step="0.001" id="lat" value="34.446"></div>
  <div><label class="f">震源 経度 (lon)</label><input type="number" step="0.001" id="lon" value="129.065"></div></div>
  <div class="row"><div><label class="f">震央地名（留空则按经纬度自动判定）</label><input type="text" id="name" value="福岡県北西沖" placeholder="例：福岡県北西沖"></div></div>
  <div class="modes"><label><input type="radio" name="mode" value="alert" checked><span>警報</span></label><label><input type="radio" name="mode" value="cancel"><span>取消</span></label></div>
  <h2>警報 府県予報区</h2>
  <div class="tools"><button type="button" id="demo">复现示例</button><button type="button" id="all">全选</button><button type="button" id="none">清空</button><button type="button" id="many">测试整合</button></div>
  <div class="areas" id="areas">__PREFS__</div>
  <div class="tools" style="margin-top:14px"><button class="primary" id="gen">生成图片</button><button id="dl">下载 PNG</button></div>
 </div>
 <div class="card">
  <h2>プレビュー</h2>
  <div class="preview"><img id="img" alt="生成结果"><div class="note" id="note">点击「生成图片」开始</div></div>
  <div class="note" style="margin-top:16px;text-align:left;color:var(--sub)">
   实时模式：启动参数加 <code>--auto</code>，程序每 3 秒轮询 <code>code=556</code>，新报自动写入输出目录，文件名为 <code>eventId_serial.png</code>；取消报（<code>cancelled=true</code>）自动输出取消画面。
  </div>
 </div>
</div>
<div class="card" style="margin:0 26px 20px">
 <h2>OBS 播出联动（NHK 式瞬间弹出）</h2>
 <div style="font-size:13px;color:var(--sub);line-height:1.9">
  在 OBS 添加「浏览器源」，URL 填 <code style="color:var(--yellow)">http://127.0.0.1:__PORT__/obs</code>，
  宽 1920 / 高 1080，勾选透明背景。新报到来时蓝框瞬间硬切覆盖到节目画面上，
  播放提示音，保持 __HOLDMSG__ 秒后瞬间消失；取消报自动缩短。<b>没有滚动字幕、没有淡入动画。</b>
 </div>
 <div class="tools" style="margin-top:12px">
  <a href="/obs?preview=1&replay=1" target="_blank" style="color:var(--yellow);font-size:13px;text-decoration:none;border:1px solid var(--line);border-radius:7px;padding:7px 13px">预览叠加层</a>
  <button id="obsAlert">演练：警报弹出</button>
  <button id="obsCancel">演练：取消</button>
  <button id="obsHide">立即收起</button>
  <span id="obsState" style="font-size:12px;color:var(--sub);align-self:center">—</span>
 </div>
 <div style="font-size:12px;color:var(--sub);margin-top:10px">
  图像源联动（无需 WebSocket）：把 OBS「图像源」指向
  <code>output/obs/overlay.png</code>，该文件在每次新报时原子替换、自动刷新。
 </div>
</div>
<div class="foot">素材：NHK 屏模板 / 取消画面 · 字体 A-OTF UD新ゴ Pr6N · GIS 気象庁 予報区 · 震央地名 GeoJSON · 数据 P2P地震情報 JSON API v2</div>
<script>
const $=s=>document.querySelector(s), $$=s=>[...document.querySelectorAll(s)];
const demoPrefs=['福岡','長崎','佐賀','大分','山口','熊本'];
let lastPng=null;
function setPrefs(a){$$('input[name=pref]').forEach(x=>x.checked=a.includes(x.value));syncGroups()}
function syncGroups(){$$('.grpall').forEach(g=>{const it=$$('#'+g.dataset.g+' input');g.checked=it.length>0&&it.every(x=>x.checked);g.indeterminate=!g.checked&&it.some(x=>x.checked)})}
$('#demo').onclick=()=>{$('#lat').value='34.446';$('#lon').value='129.065';$('#name').value='福岡県北西沖';$$('input[name=mode]')[0].checked=true;setPrefs(demoPrefs);gen()};
$('#all').onclick=()=>{$$('input[name=pref]').forEach(x=>x.checked=true);syncGroups()};
$('#none').onclick=()=>setPrefs([]);
$('#many').onclick=()=>{$$('input[name=mode]')[0].checked=true;setPrefs($$('input[name=pref]').slice(0,24).map(x=>x.value));gen()};
$$('.grpall').forEach(g=>g.onchange=()=>{$$('#'+g.dataset.g+' input').forEach(x=>x.checked=g.checked);g.indeterminate=false});
$$('input[name=pref]').forEach(x=>x.onchange=syncGroups);
$('#dl').onclick=()=>{if(!lastPng)return;const a=document.createElement('a');a.href='data:image/png;base64,'+lastPng;a.download='eew_'+Date.now()+'.png';a.click()};
async function gen(){
  const p=new URLSearchParams();
  p.set('lat',$('#lat').value);p.set('lon',$('#lon').value);p.set('name',$('#name').value);
  p.set('mode',$$('input[name=mode]:checked')[0].value);
  p.set('prefs',$$('input[name=pref]:checked').map(x=>x.value).join(','));
  $('#note').textContent='生成中…';
  try{
    const r=await fetch('/api/render',{method:'POST',headers:{'Content-Type':'application/x-www-form-urlencoded'},body:p});
    const j=await r.json();
    $('#img').src='data:image/png;base64,'+j.png;lastPng=j.png;
    $('#note').innerHTML=(j.consolidated?'<b>已自动整合为地方予報区名</b> · ':'')+j.note+' · 震央：'+(j.epicenter||'—');
  }catch(e){$('#note').textContent='生成失败: '+e}
}
$('#gen').onclick=gen;
syncGroups();

// ---- PWA ----
if('serviceWorker' in navigator){
  navigator.serviceWorker.register('/sw.js').catch(()=>{});
}
let deferredPrompt=null;
const ib=$('#install');
function showInstall(label){if(!ib)return;ib.hidden=false;ib.textContent=label}
addEventListener('beforeinstallprompt',e=>{
  e.preventDefault();deferredPrompt=e;showInstall('安装到桌面');
});
if(ib)ib.onclick=async()=>{
  if(deferredPrompt){
    deferredPrompt.prompt();
    await deferredPrompt.userChoice;
    deferredPrompt=null;ib.hidden=true;return;
  }
  alert('Safari：点击底部「分享」→「添加到主屏幕」');
};
if(/iPad|iPhone|iPod/.test(navigator.userAgent)&&!navigator.standalone){showInstall('添加到主屏幕')}
addEventListener('appinstalled',()=>{if(ib)ib.hidden=true});
if(location.search.indexOf('demo=1')>=0){$('#demo').click()}

// ---- OBS 播出演练 ----
function obsPayload(mode){
  const p=new URLSearchParams();
  p.set('mode',mode);p.set('lat',$('#lat').value);p.set('lon',$('#lon').value);p.set('name',$('#name').value);
  p.set('prefs',$$('input[name=pref]:checked').map(x=>x.value).join(','));
  return p;
}
async function obsPost(url,p){
  try{
    const r=await fetch(url,{method:'POST',headers:{'Content-Type':'application/x-www-form-urlencoded'},body:p});
    const j=await r.json();
    $('#obsState').textContent='已推送 seq='+j.seq;
  }catch(e){$('#obsState').textContent='推送失败: '+e}
}
$('#obsAlert').onclick=()=>obsPost('/api/overlay/test',obsPayload('alert'));
$('#obsCancel').onclick=()=>obsPost('/api/overlay/test',obsPayload('cancel'));
$('#obsHide').onclick=()=>obsPost('/api/overlay/hide',new URLSearchParams());
setInterval(async()=>{
  try{
    const j=await (await fetch('/api/overlay/state',{cache:'no-store'})).json();
    $('#obsState').textContent='seq='+j.seq+' · '+(j.hidden?'已收起':j.has?'播出中':'待机');
  }catch(e){}
},1500);
</script></body></html>"##;
