//! 緊急地震速報（NHK 风格）图片生成器
//!
//! * 手动模式：浏览器界面 / CLI 指定经纬度与警报府县
//! * 自动模式：轮询 P2P地震情報 JSON API v2 (code=556)，新报自动生成
//!   `<eventId>_<serial>.png` 并持续待机

mod auto;
mod canvas;
mod data;
mod font;
mod log;
mod net;
mod png;
mod render;
mod server;

use std::fs;
use std::path::PathBuf;
use std::thread;

use data::{load_epicenters, load_geo};
use font::Font;
use render::{Input, FONT_HEAVY, FONT_MEDIUM};

const GEO_BIN: &[u8] = include_bytes!("../data/geometry.bin");
const EPICENTER_BIN: &[u8] = include_bytes!("../data/epicenters.bin");

#[derive(Default)]
struct Cli {
    help: bool,
    demo: bool,
    cancel: bool,
    lat: Option<f64>,
    lon: Option<f64>,
    name: Option<String>,
    prefs: Vec<String>,
    out: Option<PathBuf>,
    auto: bool,
    no_serve: bool,
    port: u16,
    outdir: PathBuf,
    interval: u64,
    include_test: bool,
    process_latest: bool,
    fetch_once: bool,
    from_json: Option<PathBuf>,
    log_file: Option<PathBuf>,
}

fn parse_args() -> Cli {
    let mut c = Cli { port: 18787, outdir: PathBuf::from("output/auto"), interval: 3, ..Default::default() };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let val = || -> String { args.get(i + 1).cloned().unwrap_or_default() };
        match a {
            "-h" | "--help" => c.help = true,
            "--demo" => c.demo = true,
            "--cancel" => c.cancel = true,
            "--auto" => c.auto = true,
            "--no-serve" => c.no_serve = true,
            "--include-test" => c.include_test = true,
            "--process-latest" => c.process_latest = true,
            "--fetch-once" => c.fetch_once = true,
            "--from-json" => { c.from_json = Some(PathBuf::from(val())); i += 1; }
            "--log" => { c.log_file = Some(PathBuf::from(val())); i += 1; }
            "--lat" => { c.lat = val().parse().ok(); i += 1; }
            "--lon" => { c.lon = val().parse().ok(); i += 1; }
            "--name" => { c.name = Some(val()); i += 1; }
            "--prefs" => {
                c.prefs = val().split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                i += 1;
            }
            "--out" => { c.out = Some(PathBuf::from(val())); i += 1; }
            "--outdir" => { c.outdir = PathBuf::from(val()); i += 1; }
            "--port" => { c.port = val().parse().unwrap_or(18787); i += 1; }
            "--interval" => { c.interval = val().parse().unwrap_or(3); i += 1; }
            _ => eprintln!("未知参数: {}（用 --help 查看用法）", a),
        }
        i += 1;
    }
    c
}

fn help() {
    println!(r#"緊急地震速報 画像ジェネレーター (Rust)

用法:
  eew-generator                         启动手动生成界面 http://127.0.0.1:18787
  eew-generator --auto                  界面 + 实时待机（轮询 code=556，自动生成 PNG）
  eew-generator --auto --no-serve       仅实时待机，不开界面
  eew-generator --demo                  复现示例图，输出 output/demo.png 与 demo_cancel.png
  eew-generator --lat 34.446 --lon 129.065 --prefs 福岡,長崎,佐賀,大分,山口,熊本 \
                 --name 福岡県北西沖 --out output/manual.png
  eew-generator --cancel --out output/cancel.png
  eew-generator --fetch-once            拉取一次实时数据并打印
  eew-generator --from-json eew.json    离线回放一条/一组 556 报文（自动命名出图）

参数:
  --lat / --lon      震源纬度 / 经度
  --name             震央地名（留空则按 震央地名.geojson 自动判定）
  --prefs            警报府県予報区，逗号分隔
  --cancel           输出取消速报画面
  --out              手动模式输出文件
  --outdir           自动模式输出目录（默认 output/auto）
  --interval         自动模式轮询秒数（默认 3，限速 60 次/分）
  --log              日志文件（默认 <outdir>/standby.log，超过 4MB 自动轮转）
  --port             手动界面端口（默认 18787，占用时自动顺延）
  --include-test     自动模式也处理测试报
  --process-latest   自动模式启动时处理最新一条历史报
"#);
}

fn one_shot(cli: &Cli, demo: bool) {
    let geo = load_geo(GEO_BIN);
    let eps = load_epicenters(EPICENTER_BIN);
    let heavy = Font::new(FONT_HEAVY);
    let medium = Font::new(FONT_MEDIUM);
    let (lat, lon, name, prefs, cancel) = if demo {
        (34.446, 129.065, Some("福岡県北西沖".to_string()),
         vec!["福岡".into(), "長崎".into(), "佐賀".into(), "大分".into(), "山口".into(), "熊本".into()], false)
    } else {
        (cli.lat.unwrap_or(34.446), cli.lon.unwrap_or(129.065), cli.name.clone(),
         geo.normalize_prefs(&cli.prefs), cli.cancel)
    };
    let input = Input { lat, lon, prefs, epicenter: name, cancel };
    let out = render::render(&input, &geo, &eps, &heavy, &medium);
    let path = cli.out.clone().unwrap_or_else(|| PathBuf::from(
        if demo { "output/demo.png" } else if cancel { "output/cancel.png" } else { "output/manual.png" }));
    if let Some(p) = path.parent() { fs::create_dir_all(p).ok(); }
    fs::write(&path, &out.png).expect("写入 PNG 失败");
    println!("已生成: {}  ({} 字节)", path.display(), out.png.len());
    println!("震央: {} / {}", out.epicenter, out.note);

    if demo {
        let cancel_out = render::render(
            &Input { lat, lon, prefs: Vec::new(), epicenter: None, cancel: true },
            &geo, &eps, &heavy, &medium);
        fs::create_dir_all("output").ok();
        fs::write("output/demo_cancel.png", &cancel_out.png).ok();
        println!("已生成: output/demo_cancel.png");
    }
}

fn main() {
    let cli = parse_args();
    if cli.help {
        help();
        return;
    }
    if cli.fetch_once {
        match net::fetch() {
            Ok(events) => {
                println!("code=556 记录数: {}", events.len());
                for e in &events {
                    println!("  id={} eventId={} serial={} time={} 取消={} 测试={} 震源=({:.3},{:.3}) 府県={}",
                             e.id, e.event_id, e.serial, e.time, e.cancelled, e.test,
                             e.lat, e.lon, e.prefs.join("·"));
                }
            }
            Err(e) => eprintln!("获取失败: {}", e),
        }
        return;
    }
    if let Some(path) = &cli.from_json {
        let text = match fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => { eprintln!("读取 {} 失败: {}", path.display(), e); return; }
        };
        match net::parse(&text) {
            Ok(events) => {
                if events.is_empty() {
                    eprintln!("报文里没有 code=556 的记录");
                    return;
                }
                let geo = load_geo(GEO_BIN);
                let eps = load_epicenters(EPICENTER_BIN);
                let heavy = Font::new(FONT_HEAVY);
                let medium = Font::new(FONT_MEDIUM);
                fs::create_dir_all(&cli.outdir).ok();
                println!("回放 {} 条记录 → {}", events.len(), cli.outdir.display());
                for e in &events {
                    auto::write_png(&cli.outdir, e, &geo, &eps, &heavy, &medium);
                }
            }
            Err(e) => eprintln!("解析失败: {}", e),
        }
        return;
    }
    if cli.demo || cli.lat.is_some() || cli.lon.is_some() || cli.cancel || cli.out.is_some() {
        one_shot(&cli, cli.demo);
        return;
    }
    if cli.auto {
        let log_path = cli
            .log_file
            .clone()
            .unwrap_or_else(|| cli.outdir.join("standby.log"));
        log::init(log_path);
        log::info(&format!(
            "==== 进程启动 pid={} exe={} cwd={} interval={}s ====",
            std::process::id(),
            std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default(),
            std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default(),
            cli.interval
        ));
        let st = auto::AutoState {
            out_dir: cli.outdir.clone(),
            interval: cli.interval,
            include_test: cli.include_test,
            process_latest_on_start: cli.process_latest,
        };
        thread::spawn(move || auto::run(st));
    }
    if !cli.no_serve {
        server::run(server::ServerState {
            port: cli.port,
            out_dir: cli.outdir.clone(),
            auto_enabled: cli.auto,
        });
    } else if cli.auto {
        loop {
            thread::sleep(std::time::Duration::from_secs(3600));
        }
    } else {
        help();
    }
}
