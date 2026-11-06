//! RGBA canvas with anti-aliased polygon fills, strokes and alpha compositing.

#[derive(Clone, Copy, Debug)]
pub struct Color(pub u8, pub u8, pub u8);

pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub buf: Vec<u8>,
}

impl Canvas {
    pub fn new(w: usize, h: usize, rgba: &[u8]) -> Self {
        let mut buf = vec![0u8; w * h * 4];
        buf.copy_from_slice(rgba);
        Self { w, h, buf }
    }

    #[inline]
    pub fn blend(&mut self, x: i32, y: i32, c: Color, a: u8) {
        if a == 0 || x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return;
        }
        let i = (y as usize * self.w + x as usize) * 4;
        let alpha = a as f32 / 255.0;
        let inv = 1.0 - alpha;
        self.buf[i] = (self.buf[i] as f32 * inv + c.0 as f32 * alpha).round() as u8;
        self.buf[i + 1] = (self.buf[i + 1] as f32 * inv + c.1 as f32 * alpha).round() as u8;
        self.buf[i + 2] = (self.buf[i + 2] as f32 * inv + c.2 as f32 * alpha).round() as u8;
        self.buf[i + 3] = self.buf[i + 3].max(a);
    }

    /// Composite a coverage mask at (dx,dy).
    pub fn fill_mask(&mut self, mask: &[u8], mw: usize, mh: usize, dx: i32, dy: i32, color: Color) {
        for my in 0..mh {
            let py = dy + my as i32;
            if py < 0 || py >= self.h as i32 {
                continue;
            }
            for mx in 0..mw {
                let a = mask[my * mw + mx];
                if a != 0 {
                    self.blend(dx + mx as i32, py, color, a);
                }
            }
        }
    }

    /// Draw a translucent RGBA sprite, clipped to a rectangle.
    /// The epicenter marker is composited last so it sits above every other layer.
    pub fn draw_sprite_clip(&mut self, sprite: &[u8], sw: usize, sh: usize,
                            dx: i32, dy: i32, clip: (i32, i32, i32, i32)) {
        for sy in 0..sh {
            let py = dy + sy as i32;
            if py < clip.1 || py > clip.3 {
                continue;
            }
            for sx in 0..sw {
                let px = dx + sx as i32;
                if px < clip.0 || px > clip.2 {
                    continue;
                }
                let i = (sy * sw + sx) * 4;
                let a = sprite[i + 3];
                if a != 0 {
                    self.blend(px, py, Color(sprite[i], sprite[i + 1], sprite[i + 2]), a);
                }
            }
        }
    }

    /// Anti-aliased stroke of polyline rings (used for map boundaries).
    pub fn stroke_rings(&mut self, rings: &[Vec<(f32, f32)>], width: f32, color: Color,
                        clip: (i32, i32, i32, i32)) {
        let half = width / 2.0;
        for ring in rings {
            if ring.len() < 2 {
                continue;
            }
            for i in 0..ring.len() {
                let (x0, y0) = ring[i];
                let (x1, y1) = ring[(i + 1) % ring.len()];
                let minx = (x0.min(x1) - half - 1.0).floor() as i32;
                let maxx = (x0.max(x1) + half + 1.0).ceil() as i32;
                let miny = (y0.min(y1) - half - 1.0).floor() as i32;
                let maxy = (y0.max(y1) + half + 1.0).ceil() as i32;
                let vx = x1 - x0;
                let vy = y1 - y0;
                let len2 = vx * vx + vy * vy;
                for py in miny.max(clip.1)..=maxy.min(clip.3) {
                    for px in minx.max(clip.0)..=maxx.min(clip.2) {
                        // 2x2 supersampling
                        let mut hits = 0u8;
                        for sy in 0..2 {
                            for sx in 0..2 {
                                let qx = px as f32 + (sx as f32 + 0.5) / 2.0;
                                let qy = py as f32 + (sy as f32 + 0.5) / 2.0;
                                let t = if len2 == 0.0 {
                                    0.0
                                } else {
                                    (((qx - x0) * vx + (qy - y0) * vy) / len2).clamp(0.0, 1.0)
                                };
                                let cxp = x0 + t * vx;
                                let cyp = y0 + t * vy;
                                let d = ((qx - cxp).powi(2) + (qy - cyp).powi(2)).sqrt();
                                if d <= half {
                                    hits += 1;
                                }
                            }
                        }
                        if hits > 0 {
                            self.blend(px, py, color, (hits as u32 * 255 / 4) as u8);
                        }
                    }
                }
            }
        }
    }
}

/// Even-odd rasterization of closed contours into an anti-aliased coverage mask.
/// Coordinates are already in mask pixel space.
pub fn even_odd_mask(w: usize, h: usize, contours: &[Vec<(f32, f32)>], subs: usize) -> Vec<u8> {
    let mut acc = vec![0f32; w * h];
    let inv = 1.0 / subs as f32;
    // (x0, y0, x1, y1, ymin, ymax) sorted by ymin for an active-edge scan
    let mut edges: Vec<(f32, f32, f32, f32, f32, f32)> = Vec::new();
    for ring in contours {
        if ring.len() < 2 {
            continue;
        }
        for i in 0..ring.len() {
            let (x0, y0) = ring[i];
            let (x1, y1) = ring[(i + 1) % ring.len()];
            if y0 == y1 {
                continue;
            }
            let (lo, hi) = (y0.min(y1), y0.max(y1));
            if hi < 0.0 || lo > h as f32 {
                continue;
            }
            edges.push((x0, y0, x1, y1, lo, hi));
        }
    }
    edges.sort_by(|a, b| a.4.partial_cmp(&b.4).unwrap_or(std::cmp::Ordering::Equal));
    let mut next = 0usize;
    let mut active: Vec<usize> = Vec::new();
    for py in 0..h {
        for s in 0..subs {
            let sy = py as f32 + (s as f32 + 0.5) * inv;
            while next < edges.len() && edges[next].4 <= sy {
                active.push(next);
                next += 1;
            }
            active.retain(|&i| edges[i].5 > sy);
            let mut crossings: Vec<f32> = Vec::with_capacity(active.len());
            for &i in &active {
                let e = &edges[i];
                if (e.1 <= sy && sy < e.3) || (e.3 <= sy && sy < e.1) {
                    crossings.push(e.0 + (sy - e.1) * (e.2 - e.0) / (e.3 - e.1));
                }
            }
            if crossings.len() < 2 {
                continue;
            }
            crossings.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            for pair in crossings.chunks(2) {
                if pair.len() < 2 {
                    continue;
                }
                let xa = pair[0].max(0.0);
                let xb = pair[1].min(w as f32);
                if xb <= xa {
                    continue;
                }
                let start = xa.floor() as i32;
                let end = xb.ceil() as i32;
                for px in start..end {
                    if px < 0 || px >= w as i32 {
                        continue;
                    }
                    let l = (px as f32).max(xa);
                    let r = ((px + 1) as f32).min(xb);
                    let cov = (r - l).max(0.0) * inv;
                    acc[py * w + px as usize] += cov;
                }
            }
        }
    }
    acc.into_iter().map(|v| (v.min(1.0) * 255.0).round() as u8).collect()
}

/// Separable square max-filter dilation (for text outlines).
pub fn dilate(src: &[u8], w: usize, h: usize, r: usize) -> Vec<u8> {
    if r == 0 {
        return src.to_vec();
    }
    let mut tmp = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut m = 0u8;
            let x0 = x.saturating_sub(r);
            let x1 = (x + r).min(w - 1);
            for xx in x0..=x1 {
                m = m.max(src[y * w + xx]);
            }
            tmp[y * w + x] = m;
        }
    }
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        let y0 = y.saturating_sub(r);
        let y1 = (y + r).min(h - 1);
        for x in 0..w {
            let mut m = 0u8;
            for yy in y0..=y1 {
                m = m.max(tmp[yy * w + x]);
            }
            out[y * w + x] = m;
        }
    }
    out
}
