# NHK 緊急地震速報 画面生成器

用 Rust 从零实现的仿 NHK「緊急地震速報（気象庁）」画面生成器。不依赖任何图像库或 GUI 框架：
PNG 编解码、矢量填充与抗锯齿、CFF 字体轮廓解析、地图投影绘制、HTTP 服务全部为手写实现，
只用一个 `ttf-parser` 解析字体表。

![示例](docs/demo.png)

## 能力

- **手动生成**：浏览器界面输入震源经纬度、勾选警报府県予報区，实时预览并下载 PNG
- **实时待机**：轮询 [P2P地震情報 API](https://www.p2pquake.net/develop/json_api_v2/) 的 `code=556`
  （緊急地震速報／警報），新报文自动出图，文件名为 `<eventId>_<serial>.png`；取消报自动输出取消画面
- **震央地名自动判定**：按 `震央地名.geojson` 的多边形反查，无需手填
- **地区名自动整合**：完全按 NHK 的排版法则处理（见下）

## 快速开始

```bash
cargo build --release

# 手动界面（默认 http://127.0.0.1:18787，端口被占用时自动顺延）
./target/release/eew-generator

# 实时待机（轮询 556，自动出图到 output/auto）
./target/release/eew-generator --auto

# 复现仓库中的示例图
./target/release/eew-generator --demo

# 离线回放 556 报文，验证命名与渲染
./target/release/eew-generator --from-json sample_556.json --outdir output/replay
```

其余参数见 `--help`：`--lat/--lon/--name/--prefs/--cancel/--out/--outdir/--interval/--port`。

## 地区名排版法则

默认显示府県予報区名，每行最多 **14 字**（词间空隙计 1 字），最多两行；超过两行时切换为
地方予報区名，并遵循：

1. 地方内只有 1 个区域时，保留该区域名（如「福井」不并入「北陸」、「福島」不并入「東北」）
2. 新潟、伊豆諸島、小笠原、奄美等岛屿区域独立列出，不并入所属地方
3. 整合后仍超两行，再以 2px 递减字号压缩

验证用例（`--prefs` 传入对应府县即可复现）：

| 场景 | 输出 |
|---|---|
| 宮城 岩手 福島 秋田 山形 | 一行 5 个府県名 |
| 熊本（九州 + 中国 + 四国） | `九州 中国 四国` |
| 播磨灘 | `近畿 四国 中国 福井 東海` |
| 千葉県南方沖 | `伊豆諸島 関東 東海 福島` / `甲信 新潟` |
| 奈良県 | `近畿 東海 四国 中国 北陸 甲信` / `関東 伊豆諸島 新潟 九州` |

## 实现要点

| 模块 | 说明 |
|---|---|
| `src/png.rs` | 手写 PNG 编码器（filter + 固定 Huffman deflate + CRC32/Adler32）与解码器 |
| `src/font.rs` | 用 `ttf-parser` 取字形轮廓，把二次/三次贝塞尔展平为多边形 |
| `src/canvas.rs` | 扫描线 + 4×4 子采样抗锯齿填充，支持 even-odd 与 alpha 混合、描边、位图叠加 |
| `src/render.rs` | 版式常量、移动视野投影、府県名自动整合与换行、黄色副标题排版 |
| `src/data.rs` | 读取 `data/geometry.bin`（简化后的预报区多边形）与 `data/epicenters.bin` |
| `src/net.rs` | 通过系统 `curl` 取 HTTPS（避免 Windows 下 TLS 依赖 C 编译器），`serde_json` 解析 |
| `src/server.rs` | 手写 HTTP/1.1 服务，内嵌单页界面 + `POST /api/render` 返回 base64 PNG |
| `src/auto.rs` | 轮询、去重、按 `eventId_serial` 命名落盘 |

静态底版（渐变、边框、标题、取消文案）由 `tools/build_background.py` 从示例图分离并拟合重建，
导出为 `assets/bg_alert.bin` / `assets/bg_cancel.bin`；地图、文字、震源标记则在运行时绘制。

## 数据与素材

| 路径 | 内容 |
|---|---|
| `data/geometry.bin` | 府県予報区 / 地方予報区多边形（由气象厅 GIS shapefile 简化而来） |
| `data/epicenters.bin` | 震央地名多边形索引（源：`震央地名.geojson`） |
| `assets/fonts/*.otf` | UD 新ゴシック 6 个字重 |
| `assets/bg_*.bin` | 静态底版 RGBA |
| `assets/x_marker.bin` | 震源 × 标记精灵（始终绘制在最上层） |

原始 GIS shapefile（`data/gis/`）与第三方压缩包未入库：单个 `.shp` 超过 100MB，
且运行时已不再需要。重建方式：

```bash
python tools/extract_assets.py    # 解压字体 / GIS
python tools/build_geometry.py    # 重新生成 data/*.bin
```

## 说明

本项目仅用于技术演示与画面还原研究，非气象厅官方产品，输出图像不得用于实际防灾发布。
