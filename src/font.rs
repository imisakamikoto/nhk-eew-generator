//! CFF/OTF glyph rasterization and styled text layout (no external rasterizer).

use std::sync::Mutex;
use std::collections::HashMap;

use ttf_parser::{Face, GlyphId, OutlineBuilder};

use crate::canvas::{even_odd_mask, Canvas, Color};

pub struct Font {
    data: &'static [u8],
    cache: Mutex<HashMap<(char, u32, u32), Glyph>>,
}

#[derive(Clone)]
pub struct Glyph {
    pub w: usize,
    pub h: usize,
    /// offset of the bitmap origin from the pen, in pixels
    pub ox: f32,
    pub oy: f32,
    pub advance: f32,
    pub mask: Vec<u8>,
}

pub struct Layout {
    /// (glyph, x_in_ink_space, y_in_ink_space)
    pub glyphs: Vec<(Glyph, f32, f32)>,
    pub width: f32,
    pub height: f32,
}

struct Builder {
    contours: Vec<Vec<(f32, f32)>>, // transformed to pixel space (y down)
    cur: Vec<(f32, f32)>,
    last_raw: (f32, f32),          // last point in font units
    sx: f32,
    sy: f32,
}

impl Builder {
    fn push(&mut self, x: f32, y: f32) {
        self.cur.push((x * self.sx, -y * self.sy));
        self.last_raw = (x, y);
    }

    fn flush(&mut self) {
        if self.cur.len() >= 3 {
            self.contours.push(std::mem::take(&mut self.cur));
        } else {
            self.cur.clear();
        }
    }

    /// De Casteljau flattening of a quadratic/cubic Bezier given its control points
    /// (excluding the current point, including the endpoint).
    fn flatten(&mut self, ctrl: &[(f32, f32)]) {
        let mut work = vec![self.last_raw];
        work.extend_from_slice(ctrl);
        let mut raw_len = 0.0f32;
        for i in 0..work.len() - 1 {
            raw_len += ((work[i + 1].0 - work[i].0).powi(2)
                      + (work[i + 1].1 - work[i].1).powi(2)).sqrt();
        }
        let n = ((raw_len * self.sy / 1.5).ceil() as usize).clamp(4, 48);
        for k in 1..=n {
            let t = k as f32 / n as f32;
            let mut pts = work.clone();
            for level in (1..pts.len()).rev() {
                for j in 0..level {
                    pts[j] = (pts[j].0 * (1.0 - t) + pts[j + 1].0 * t,
                              pts[j].1 * (1.0 - t) + pts[j + 1].1 * t);
                }
            }
            self.push(pts[0].0, pts[0].1);
        }
    }
}

impl OutlineBuilder for Builder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.flush();
        self.push(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.push(x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.flatten(&[(x1, y1), (x, y)]);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.flatten(&[(x1, y1), (x2, y2), (x, y)]);
    }
    fn close(&mut self) {
        self.flush();
    }
}

impl Font {
    pub fn new(data: &'static [u8]) -> Self {
        Self { data, cache: Mutex::new(HashMap::new()) }
    }

    pub fn glyph(&self, ch: char, size: f32, sx: f32) -> Glyph {
        let key = (ch, (size * 10.0) as u32, (sx * 1000.0) as u32);
        if let Some(g) = self.cache.lock().ok().and_then(|c| c.get(&key).cloned()) {
            return g;
        }
        let face = Face::parse(self.data, 0).expect("font parse");
        let upem = face.units_per_em() as f32;
        let scale = size / upem;
        let gid = face.glyph_index(ch).unwrap_or(GlyphId(0));
        let mut b = Builder {
            contours: Vec::new(),
            cur: Vec::new(),
            last_raw: (0.0, 0.0),
            sx: scale * sx,
            sy: scale,
        };
        face.outline_glyph(gid, &mut b);
        b.flush();

        let mut minx = f32::INFINITY;
        let mut miny = f32::INFINITY;
        let mut maxx = f32::NEG_INFINITY;
        let mut maxy = f32::NEG_INFINITY;
        for c in &b.contours {
            for &(x, y) in c {
                minx = minx.min(x);
                miny = miny.min(y);
                maxx = maxx.max(x);
                maxy = maxy.max(y);
            }
        }
        let g = if !minx.is_finite() {
            Glyph { w: 1, h: 1, ox: 0.0, oy: 0.0, mask: vec![0], advance: size * sx }
        } else {
            let ox = minx.floor() - 1.0;
            let oy = miny.floor() - 1.0;
            let w = (maxx.ceil() - ox + 2.0).max(1.0) as usize;
            let h = (maxy.ceil() - oy + 2.0).max(1.0) as usize;
            let shifted: Vec<Vec<(f32, f32)>> = b.contours.iter()
                .map(|c| c.iter().map(|&(x, y)| (x - ox, y - oy)).collect())
                .collect();
            let mask = even_odd_mask(w, h, &shifted, 6);
            let advance = face.glyph_hor_advance(gid).unwrap_or(0) as f32 * scale * sx;
            Glyph { w, h, ox, oy, mask, advance }
        };
        if let Ok(mut c) = self.cache.lock() {
            c.insert(key, g.clone());
        }
        g
    }

    pub fn layout(&self, text: &str, size: f32, sx: f32, tracking: f32) -> Layout {
        let mut items: Vec<(Glyph, f32)> = Vec::new();
        let mut pen = 0.0f32;
        for ch in text.chars() {
            let g = self.glyph(ch, size, sx);
            let start = pen;
            pen += g.advance + tracking;
            items.push((g, start));
        }
        let mut x0 = f32::INFINITY;
        let mut x1 = f32::NEG_INFINITY;
        let mut y0 = f32::INFINITY;
        let mut y1 = f32::NEG_INFINITY;
        for (g, p) in &items {
            x0 = x0.min(p + g.ox);
            x1 = x1.max(p + g.ox + g.w as f32);
            y0 = y0.min(g.oy);
            y1 = y1.max(g.oy + g.h as f32);
        }
        if !x0.is_finite() {
            return Layout { glyphs: Vec::new(), width: 0.0, height: 0.0 };
        }
        let glyphs = items.into_iter().map(|(g, p)| (g.clone(), p + g.ox - x0, g.oy - y0)).collect();
        Layout { glyphs, width: x1 - x0, height: y1 - y0 }
    }
}

/// Draw a laid-out line: solid outline (dilated mask) then the fill on top.
pub fn draw_styled(canvas: &mut Canvas, layout: &Layout, x: f32, top_y: f32,
                   fill: Color, outline: Color, radius: usize) {
    if layout.glyphs.is_empty() {
        return;
    }
    let p = radius + 2;
    let w = layout.width.ceil() as usize + 2 * p;
    let h = layout.height.ceil() as usize + 2 * p;
    let mut base = vec![0u8; w * h];
    for (g, gx, gy) in &layout.glyphs {
        let dx = (gx + p as f32).round() as i32;
        let dy = (gy + p as f32).round() as i32;
        for yy in 0..g.h {
            let py = dy + yy as i32;
            if py < 0 || py >= h as i32 {
                continue;
            }
            for xx in 0..g.w {
                let px = dx + xx as i32;
                if px < 0 || px >= w as i32 {
                    continue;
                }
                let v = g.mask[yy * g.w + xx];
                let i = py as usize * w + px as usize;
                if v > base[i] {
                    base[i] = v;
                }
            }
        }
    }
    let thick = crate::canvas::dilate(&base, w, h, radius);
    let dx = x.round() as i32 - p as i32;
    let dy = top_y.round() as i32 - p as i32;
    canvas.fill_mask(&thick, w, h, dx, dy, outline);
    canvas.fill_mask(&base, w, h, dx, dy, fill);
}
