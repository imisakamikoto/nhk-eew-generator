//! Tiny dependency-free HTTP server for the manual generation UI.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;

use crate::data::{load_epicenters, load_geo, Epicenter, Geo};
use crate::font::Font;
use crate::render::{self, Input, FONT_HEAVY, FONT_MEDIUM};

const GEO_BIN: &[u8] = include_bytes!("../data/geometry.bin");
const EPICENTER_BIN: &[u8] = include_bytes!("../data/epicenters.bin");

pub struct ServerState {
    pub port: u16,
    pub out_dir: PathBuf,
    pub auto_enabled: bool,
}

fn b64(data: &[u8]) -> String {
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

fn json_escape(s: &str) -> String {
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
}

fn respond(stream: &mut TcpStream, code: &str, ctype: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
        code, ctype, body.len());
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

fn handle(mut stream: TcpStream, geo: &Geo, eps: &[Epicenter], heavy: &Font, medium: &Font,
          state: &ServerState) {
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
    let path = parts.next().unwrap_or("/").to_string();
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
    println!("[手动] 界面已就绪: http://127.0.0.1:{}", state.port);
    for stream in listener.incoming() {
        match stream {
            Ok(s) => handle(s, &geo, &eps, &heavy, &medium, &state),
            Err(e) => eprintln!("[手动] 连接错误: {}", e),
        }
    }
}

const HTML: &str = r#"<!DOCTYPE html>
<html lang="zh-CN"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>緊急地震速報ジェネレーター</title>
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
</style></head><body>
<header><h1>緊急地震速報 画像ジェネレーター</h1><span class="tag">Rust / P2P地震情報 v2</span><span class="tag" id="autotag">__AUTO__</span></header>
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
</script></body></html>"#;
