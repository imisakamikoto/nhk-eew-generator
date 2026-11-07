//! obs-websocket v5 客户端：让 OBS 里的指定来源在新报到来时显示、播出结束后隐藏。
//!
//! 只在 `--obs-source` 给定时启用；默认关闭。本地 `ws://127.0.0.1:4455` 是明文，
//! 所以不用 TLS（这个工程刻意不依赖任何本地 TLS 库）。握手、帧编解码、SHA256
//! 认证全部手写，不新增 crate。任何失败都只是记一条日志，绝不影响出图。

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::log;
use crate::server::b64;

#[derive(Clone, Debug)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub password: String,
    pub scene: String,
    pub source: String,
}

impl Config {
    pub fn enabled(&self) -> bool {
        !self.source.trim().is_empty()
    }
}

// ---------------------------------------------------------------- SHA-256

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut msg = data.to_vec();
    let bits = (msg.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g; g = f; f = e; e = d.wrapping_add(t1);
            d = c; c = b; b = a; a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a); h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c); h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e); h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g); h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 32];
    for i in 0..8 {
        out[4 * i..4 * i + 4].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}

// ------------------------------------------------------------ WebSocket I/O

struct Ws {
    s: TcpStream,
    pending: Vec<u8>,
}

impl Ws {
    fn more(&mut self) -> io::Result<()> {
        let mut buf = [0u8; 4096];
        let n = self.s.read(&mut buf)?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "连接已关闭"));
        }
        self.pending.extend_from_slice(&buf[..n]);
        Ok(())
    }
    fn fill(&mut self, n: usize) -> io::Result<()> {
        while self.pending.len() < n {
            self.more()?;
        }
        Ok(())
    }
    fn handshake(&mut self) -> io::Result<()> {
        loop {
            if let Some(i) = self.pending.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&self.pending[..i + 4]).to_lowercase();
                self.pending.drain(..i + 4);
                if !head.starts_with("http/1.1 101") {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "WebSocket 升级被拒绝"));
                }
                return Ok(());
            }
            self.more()?;
        }
    }
    fn frame(&mut self) -> io::Result<(u8, Vec<u8>)> {
        self.fill(2)?;
        let b0 = self.pending[0];
        let b1 = self.pending[1];
        let op = b0 & 0x0f;
        let masked = b1 & 0x80 != 0;
        let mut len = (b1 & 0x7f) as usize;
        let mut skip = 2usize;
        if len == 126 {
            self.fill(4)?;
            len = u16::from_be_bytes([self.pending[2], self.pending[3]]) as usize;
            skip = 4;
        } else if len == 127 {
            self.fill(10)?;
            let mut arr = [0u8; 8];
            arr.copy_from_slice(&self.pending[2..10]);
            len = u64::from_be_bytes(arr) as usize;
            skip = 10;
        }
        let mask = if masked {
            self.fill(skip + 4)?;
            let mut m = [0u8; 4];
            m.copy_from_slice(&self.pending[skip..skip + 4]);
            skip += 4;
            Some(m)
        } else {
            None
        };
        if len > 8 << 20 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "帧过大"));
        }
        self.fill(skip + len)?;
        let mut data: Vec<u8> = self.pending.drain(..skip + len).collect();
        data.drain(..skip);
        if let Some(m) = mask {
            for (i, b) in data.iter_mut().enumerate() {
                *b ^= m[i % 4];
            }
        }
        Ok((op, data))
    }
    fn text(&mut self, payload: &str) -> io::Result<()> {
        let data = payload.as_bytes();
        let mut head = vec![0x81u8];
        if data.len() < 126 {
            head.push(0x80 | data.len() as u8);
        } else if data.len() < 65536 {
            head.push(0x80 | 126);
            head.extend_from_slice(&(data.len() as u16).to_be_bytes());
        } else {
            head.push(0x80 | 127);
            head.extend_from_slice(&(data.len() as u64).to_be_bytes());
        }
        // 客户端帧必须掩码
        let seed = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos() as u64
            ^ (std::process::id() as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let mut x = seed;
        let mut mask = [0u8; 4];
        for m in mask.iter_mut() {
            x ^= x << 13; x ^= x >> 7; x ^= x << 17;
            *m = x as u8;
        }
        head.extend_from_slice(&mask);
        let mut body = Vec::with_capacity(data.len());
        for (i, b) in data.iter().enumerate() {
            body.push(b ^ mask[i % 4]);
        }
        self.s.write_all(&head)?;
        self.s.write_all(&body)?;
        self.s.flush()
    }
    /// 读一条 JSON 消息（op, d），跳过事件与其它无关帧
    fn msg(&mut self) -> io::Result<(u8, Value)> {
        loop {
            let (op, data) = self.frame()?;
            match op {
                1 => {
                    let v: Value = serde_json::from_slice(&data)
                        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
                    return Ok((v["op"].as_u64().unwrap_or(99) as u8, v["d"].clone()));
                }
                8 => return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "服务端关闭")),
                9 => {} // ping：obs-websocket 不要求回应，忽略
                _ => {}
            }
        }
    }
}

fn request(ws: &mut Ws, id: &str, kind: &str, data: Value) -> Result<Value, String> {
    let msg = json!({"op": 6, "d": {"requestType": kind, "requestId": id, "requestData": data}});
    ws.text(&msg.to_string()).map_err(|e| e.to_string())?;
    loop {
        let (op, d) = ws.msg().map_err(|e| e.to_string())?;
        if op == 7 && d["requestId"].as_str() == Some(id) {
            if d["requestStatus"]["result"].as_bool() != Some(true) {
                return Err(format!("{} 失败: {}", kind, d["requestStatus"]));
            }
            return Ok(d["responseData"].clone());
        }
    }
}

fn run(cfg: &Config, visible: bool) -> Result<(), String> {
    let addr = format!("{}:{}", cfg.host, cfg.port);
    let s = TcpStream::connect(&addr).map_err(|e| format!("连接 {} 失败: {}", addr, e))?;
    s.set_read_timeout(Some(Duration::from_secs(3))).ok();
    s.set_write_timeout(Some(Duration::from_secs(3))).ok();
    let mut ws = Ws { s, pending: Vec::new() };

    let mut key_src = [0u8; 16];
    let seed = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos() as u64;
    for (i, b) in key_src.iter_mut().enumerate() {
        *b = (seed.wrapping_shr((i % 8 * 8) as u32) as u8) ^ (i as u8).wrapping_mul(37);
    }
    let handshake = format!(
        "GET / HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\n\r\n",
        addr, b64(&key_src));
    ws.s.write_all(handshake.as_bytes()).map_err(|e| e.to_string())?;
    ws.s.flush().map_err(|e| e.to_string())?;
    ws.handshake().map_err(|e| e.to_string())?;

    // Hello (op 0) → Identify (op 1) → Identified (op 2)
    let (op, hello) = ws.msg().map_err(|e| e.to_string())?;
    if op != 0 {
        return Err("首个帧不是 Hello".to_string());
    }
    let mut identify = json!({"rpcVersion": 1, "eventSubscriptions": 0});
    if let Some(auth) = hello.get("authentication") {
        if cfg.password.is_empty() {
            return Err("OBS 要求密码，请加 --obs-password".to_string());
        }
        let salt = auth["salt"].as_str().unwrap_or("");
        let challenge = auth["challenge"].as_str().unwrap_or("");
        let secret = b64(&sha256(format!("{}{}", cfg.password, salt).as_bytes()));
        let response = b64(&sha256(format!("{}{}", secret, challenge).as_bytes()));
        identify["authentication"] = json!(response);
    }
    ws.text(&json!({"op": 1, "d": identify}).to_string()).map_err(|e| e.to_string())?;
    loop {
        let (op, _) = ws.msg().map_err(|e| e.to_string())?;
        if op == 2 { break; }
        if op == 5 || op == 7 { continue; }
    }

    let scene = if cfg.scene.trim().is_empty() {
        let r = request(&mut ws, "eew-scene", "GetCurrentProgramScene", json!({}))?;
        r["currentProgramSceneName"].as_str().unwrap_or("").to_string()
    } else {
        cfg.scene.clone()
    };
    if scene.is_empty() {
        return Err("无法确定当前场景（可用 --obs-scene 指定）".to_string());
    }
    let r = request(&mut ws, "eew-id", "GetSceneItemId",
                    json!({"sceneName": scene, "sourceName": cfg.source}))?;
    let item_id = r["sceneItemId"].as_i64()
        .ok_or_else(|| format!("场景「{}」里找不到来源「{}」", scene, cfg.source))?;
    let _ = request(&mut ws, "eew-vis", "SetSceneItemEnabled",
                    json!({"sceneName": scene, "sceneItemId": item_id, "sceneItemEnabled": visible}))?;
    Ok(())
}

pub fn set_visible(cfg: &Config, visible: bool) {
    if !cfg.enabled() {
        return;
    }
    match run(cfg, visible) {
        Ok(_) => log::info(&format!("[OBS-WS] 来源「{}」已{}", cfg.source,
                                    if visible { "显示" } else { "隐藏" })),
        Err(e) => log::warn(&format!("[OBS-WS] 联动失败（不影响出图）: {}", e)),
    }
}
