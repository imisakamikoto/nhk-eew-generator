//! Compose the NHK-style emergency earthquake warning card.

use crate::canvas::{even_odd_mask, Canvas, Color};
use crate::data::Geo;
use crate::font::{draw_styled, Font, Layout};
use crate::png;

pub const CARD_W: usize = 1215;
pub const CARD_H: usize = 411;
pub const CANCEL_W: usize = 886;
pub const CANCEL_H: usize = 411;

const MAP_X0: i32 = 15;
const MAP_Y0: i32 = 13;
const MAP_X1: i32 = 332;
const MAP_Y1: i32 = 399;

const COL_LAND: Color = Color(131, 131, 131);
const COL_BORDER: Color = Color(89, 89, 89);
const COL_HL: Color = Color(255, 255, 0);
const COL_NAVY: Color = Color(44, 86, 148);
const COL_WHITE: Color = Color(253, 255, 255);
const COL_YELLOW: Color = Color(247, 252, 95);

const X_SPRITE_W: usize = 66;
const X_SPRITE_H: usize = 66;

pub const BG_ALERT: &[u8] = include_bytes!("../assets/bg_alert.bin");
pub const BG_CANCEL: &[u8] = include_bytes!("../assets/bg_cancel.bin");
const X_SPRITE: &[u8] = include_bytes!("../assets/x_marker.bin");
pub const FONT_HEAVY: &[u8] = include_bytes!("../assets/fonts/udshingo_heavy.otf");
pub const FONT_MEDIUM: &[u8] = include_bytes!("../assets/fonts/udshingo_medium.otf");
pub const FONT_BOLD: &[u8] = include_bytes!("../assets/fonts/udshingo_bold.otf");
pub const FONT_DEBOLD: &[u8] = include_bytes!("../assets/fonts/udshingo_debold.otf");

fn tune_f(name: &str, default: f32) -> f32 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}
fn tune_i(name: &str, default: usize) -> usize {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

// Typography measured from the reference screenshots
// Note: vertical rhythm re-measured from a 2-line broadcast frame — line pitch
// is 1.474x the cap height (95.7px at 1215 width), not the 1.2x it used to be.
const PREF_SIZE: f32 = 66.0;
const PREF_SX: f32 = 0.82;
const PREF_GAP: f32 = 53.0;
const PREF_LEFT: f32 = 353.0;
const PREF_TOP: f32 = 198.0;
const PREF_PITCH: f32 = 96.0;
const PREF_OUTLINE: usize = 0;

const SUB_SIZE: f32 = 56.0;
const SUB_SX: f32 = 0.72;
const SUB_GAP: f32 = 12.0;
const SUB_LEFT: f32 = 350.0;
const SUB_TOP: f32 = 103.0;
const SUB_OUTLINE: usize = 0;

const MAX_WIDTH: f32 = 836.0;
/// NHK rule 2: at most 14 characters per line for prefecture areas.
const PREF_LINE_CHARS: usize = 14;
/// Regional names are short, so a line holds more of them.
const REGION_LINE_CHARS: usize = 20;

pub struct Input {
    pub lat: f64,
    pub lon: f64,
    pub prefs: Vec<String>,
    pub epicenter: Option<String>,
    pub cancel: bool,
}

pub struct Output {
    pub png: Vec<u8>,
    pub epicenter: String,
    pub lines: Vec<Vec<String>>,
    pub consolidated: bool,
    pub note: String,
}

#[derive(Clone, Copy)]
pub struct View {
    pub lon0: f64,
    pub lat0: f64,
    pub sx: f64,
    pub sy: f64,
}

impl View {
    pub fn project(&self, lon: f64, lat: f64) -> (f32, f32) {
        let x = MAP_X0 as f64 + (lon - self.lon0) * self.sx;
        let y = MAP_Y0 as f64 + (self.lat0 - lat) * self.sy;
        (x as f32, y as f32)
    }
}

fn view_from_bbox(bbox: (f32, f32, f32, f32)) -> View {
    let span_lon = (bbox.2 - bbox.0) as f64;
    let span_lat = (bbox.3 - bbox.1) as f64;
    let ratio = 56.23 / 48.0;
    let sx = 48.0f64
        .min((MAP_X1 - MAP_X0) as f64 / (span_lon + 3.0))
        .min((MAP_Y1 - MAP_Y0) as f64 / ((span_lat + 3.0) * ratio));
    let sy = sx * ratio;
    let cx = (bbox.0 as f64 + bbox.2 as f64) / 2.0 + 0.10;
    let cy = (bbox.1 as f64 + bbox.3 as f64) / 2.0 + 0.08;
    View {
        lon0: cx - (MAP_X1 - MAP_X0) as f64 / (2.0 * sx),
        lat0: cy + (MAP_Y1 - MAP_Y0) as f64 / (2.0 * sy),
        sx,
        sy,
    }
}

pub fn compute_view(geo: &Geo, prefs: &[String], lat: f64, lon: f64) -> View {
    let bbox = geo.main_bbox(prefs).unwrap_or((130.0, 33.0, 131.0, 34.0));
    let view = view_from_bbox(bbox);
    let (x, y) = view.project(lon, lat);
    let inside = x > MAP_X0 as f32 + 12.0 && x < MAP_X1 as f32 - 12.0
        && y > MAP_Y0 as f32 + 12.0 && y < MAP_Y1 as f32 - 12.0;
    if inside {
        return view;
    }
    // only zoom out when the epicenter would otherwise be off-map
    let mut ext = bbox;
    ext.0 = ext.0.min(lon as f32 - 0.5);
    ext.1 = ext.1.min(lat as f32 - 0.5);
    ext.2 = ext.2.max(lon as f32 + 0.5);
    ext.3 = ext.3.max(lat as f32 + 0.5);
    view_from_bbox(ext)
}

fn draw_map(canvas: &mut Canvas, geo: &Geo, view: &View, selected: &[String]) {
    let mut stroke_rings: Vec<Vec<(f32, f32)>> = Vec::new();
    for area in &geo.prefs {
        let mut transformed = Vec::with_capacity(area.rings.len());
        let mut minx = f32::INFINITY;
        let mut miny = f32::INFINITY;
        let mut maxx = f32::NEG_INFINITY;
        let mut maxy = f32::NEG_INFINITY;
        for ring in &area.rings {
            let t: Vec<(f32, f32)> = ring.iter()
                .map(|&(x, y)| view.project(x as f64, y as f64))
                .collect();
            for &(x, y) in &t {
                minx = minx.min(x); miny = miny.min(y);
                maxx = maxx.max(x); maxy = maxy.max(y);
            }
            transformed.push(t);
        }
        if !minx.is_finite() || maxx < MAP_X0 as f32 || minx > MAP_X1 as f32
            || maxy < MAP_Y0 as f32 || miny > MAP_Y1 as f32 {
            continue;
        }
        let ox = minx.floor().max(MAP_X0 as f32) - 1.0;
        let oy = miny.floor().max(MAP_Y0 as f32) - 1.0;
        let ex = maxx.ceil().min(MAP_X1 as f32) + 1.0;
        let ey = maxy.ceil().min(MAP_Y1 as f32) + 1.0;
        let w = ((ex - ox).max(1.0)) as usize;
        let h = ((ey - oy).max(1.0)) as usize;
        let local: Vec<Vec<(f32, f32)>> = transformed.iter()
            .map(|r| r.iter().map(|&(x, y)| (x - ox, y - oy)).collect())
            .collect();
        let mask = even_odd_mask(w, h, &local, 4);
        let color = if selected.iter().any(|n| n == &area.name) { COL_HL } else { COL_LAND };
        canvas.fill_mask(&mask, w, h, ox as i32, oy as i32, color);
        stroke_rings.extend(transformed);
    }
    canvas.stroke_rings(&stroke_rings, 1.3, COL_BORDER, (MAP_X0, MAP_Y0, MAP_X1 - 1, MAP_Y1 - 1));
}

/// Pack area names into lines.
///
/// `budget` is the NHK character budget for one line (1 line = at most 14
/// characters; the gap between two names counts as one character), while
/// `maxw` keeps the line inside the card in pixels.
fn pack(names: &[String], font: &Font, size: f32, sx: f32, gap: f32,
        maxw: f32, budget: usize) -> Vec<Vec<(String, f32, f32)>> {
    let mut lines: Vec<Vec<(String, f32, f32)>> = Vec::new();
    let mut cur: Vec<(String, f32, f32)> = Vec::new();
    let mut x = 0.0f32;
    let mut used = 0usize;
    for n in names {
        let w = font.layout(n, size, sx, 0.0).width;
        let chars = n.chars().count();
        let cost = chars + if cur.is_empty() { 0 } else { 1 };
        let nx = if cur.is_empty() { 0.0 } else { x + gap };
        if !cur.is_empty() && (used + cost > budget || nx + w > maxw) {
            lines.push(std::mem::take(&mut cur));
            cur.push((n.clone(), 0.0, w));
            x = w;
            used = chars;
        } else {
            cur.push((n.clone(), nx, w));
            x = nx + w;
            used += cost;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// Decide what the area block shows.
///
/// NHK rules: prefecture areas first; if they need more than two lines switch
/// to regional names (keeping single-area regions as they are); if that still
/// needs more than two lines, shrink the type.
fn layout_names(geo: &Geo, input_prefs: &[String], font: &Font,
                base_size: f32, sx: f32, gap: f32, maxw: f32)
    -> (Vec<String>, Vec<Vec<(String, f32, f32)>>, bool, f32) {
    let names = input_prefs.to_vec();
    let lines = pack(&names, font, base_size, sx, gap, maxw, PREF_LINE_CHARS);
    if lines.len() <= 2 {
        return (names, lines, false, base_size);
    }
    // rule 2/3/4: switch to regional names
    let mut regional = geo.consolidate(input_prefs);
    if regional.is_empty() {
        regional = names.clone();
    }
    let mut size = base_size;
    let mut lines = pack(&regional, font, size, sx, gap, maxw, REGION_LINE_CHARS);
    // rule 5: still too long -> shrink
    while lines.len() > 2 && size > 46.0 {
        size -= 2.0;
        lines = pack(&regional, font, size, sx, gap, maxw, REGION_LINE_CHARS);
    }
    (regional, lines, true, size)
}

pub fn render(input: &Input, geo: &Geo, epicenters: &[crate::data::Epicenter],
              heavy: &Font, _medium: &Font) -> Output {
    if input.cancel {
        let png = png::encode(CANCEL_W, CANCEL_H, BG_CANCEL);
        return Output {
            png,
            epicenter: String::new(),
            lines: Vec::new(),
            consolidated: false,
            note: "気象庁は先程の緊急地震速報を取り消しました".to_string(),
        };
    }

    // Runtime-tunable typography (advanced use; defaults reproduce the reference)
    let sub_size = tune_f("EEW_SUB_SIZE", SUB_SIZE);
    let sub_sx = tune_f("EEW_SUB_SX", SUB_SX);
    let sub_left = tune_f("EEW_SUB_LEFT", SUB_LEFT);
    let sub_top = tune_f("EEW_SUB_TOP", SUB_TOP);
    let sub_outline = tune_i("EEW_SUB_OUTLINE", SUB_OUTLINE);
    let pref_size = tune_f("EEW_PREF_SIZE", PREF_SIZE);
    let pref_sx = tune_f("EEW_PREF_SX", PREF_SX);
    let pref_gap = tune_f("EEW_PREF_GAP", PREF_GAP);
    let pref_left = tune_f("EEW_PREF_LEFT", PREF_LEFT);
    let pref_top = tune_f("EEW_PREF_TOP", PREF_TOP);
    let pref_pitch = tune_f("EEW_PREF_PITCH", PREF_PITCH);
    let pref_outline = tune_i("EEW_PREF_OUTLINE", PREF_OUTLINE);
    let sub_font = match std::env::var("EEW_SUB_FONT").as_deref() {
        Ok("medium") => Font::new(FONT_MEDIUM),
        Ok("bold") => Font::new(FONT_BOLD),
        Ok("heavy") => Font::new(FONT_HEAVY),
        _ => Font::new(FONT_DEBOLD),
    };
    let pref_font = match std::env::var("EEW_PREF_FONT").as_deref() {
        Ok("bold") => Font::new(FONT_BOLD),
        Ok("debold") => Font::new(FONT_DEBOLD),
        Ok("medium") => Font::new(FONT_MEDIUM),
        _ => Font::new(FONT_HEAVY),
    };

    let epic_name = input.epicenter.clone().unwrap_or_else(|| {
        crate::data::find_epicenter(epicenters, input.lon, input.lat)
            .unwrap_or_else(|| "震源地".to_string())
    });
    let mut canvas = Canvas::new(CARD_W, CARD_H, BG_ALERT);
    let view = compute_view(geo, &input.prefs, input.lat, input.lon);
    draw_map(&mut canvas, geo, &view, &input.prefs);

    // yellow sub-headline: "<epicenter>で地震" + narrow fixed gap + "強い揺れに警戒".
    // Splitting avoids the over-wide ASCII space of the font and reproduces the
    // exact 18px gap used in the NHK reference.
    let head = format!("{}で地震", epic_name);
    let tail = "強い揺れに警戒";
    let gap = tune_f("EEW_SUB_GAP", SUB_GAP);
    let mut sx = sub_sx;
    let (mut l1, mut l2) = (sub_font.layout(&head, sub_size, sx, 0.0),
                            sub_font.layout(tail, sub_size, sx, 0.0));
    let total = l1.width + gap + l2.width;
    if total > 848.0 {
        sx *= 848.0 / total;
        l1 = sub_font.layout(&head, sub_size, sx, 0.0);
        l2 = sub_font.layout(tail, sub_size, sx, 0.0);
    }
    draw_styled(&mut canvas, &l1, sub_left, sub_top, COL_YELLOW, COL_NAVY, sub_outline);
    draw_styled(&mut canvas, &l2, sub_left + l1.width + gap, sub_top,
                COL_YELLOW, COL_NAVY, sub_outline);

    // warning area names, with automatic consolidation to regional names
    let (names, lines, consolidated, size) =
        layout_names(geo, &input.prefs, &pref_font, pref_size, pref_sx, pref_gap, MAX_WIDTH);
    for (i, line) in lines.iter().enumerate() {
        for (name, x, _) in line {
            let l = pref_font.layout(name, size, pref_sx, 0.0);
            draw_styled(&mut canvas, &l, pref_left + x, pref_top + i as f32 * pref_pitch,
                        COL_WHITE, COL_NAVY, pref_outline);
        }
    }

    // The epicenter marker is composited LAST: it sits above the map, the
    // highlighted areas and every text layer, clipped to the map panel.
    let (ex, ey) = view.project(input.lon, input.lat);
    canvas.draw_sprite_clip(X_SPRITE, X_SPRITE_W, X_SPRITE_H,
                            ex.round() as i32 - X_SPRITE_W as i32 / 2,
                            ey.round() as i32 - X_SPRITE_H as i32 / 2,
                            (MAP_X0, MAP_Y0, MAP_X1 - 1, MAP_Y1 - 1));

    let note = if consolidated {
        format!("府県予報区名が収まらないため地方予報区名に統合: {}", names.join(" "))
    } else {
        format!("{} / 対象 {}", epic_name, names.len())
    };
    Output {
        png: png::encode(CARD_W, CARD_H, &canvas.buf),
        epicenter: epic_name,
        lines: lines.iter().map(|l| l.iter().map(|t| t.0.clone()).collect()).collect(),
        consolidated,
        note,
    }
}
