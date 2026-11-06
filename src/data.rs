//! Runtime loader for the preprocessed JMA forecast-area polygons and epicenter names.

pub type Ring = Vec<(f32, f32)>;

pub struct Area {
    pub code: u16,
    pub name: String,
    pub bbox: (f32, f32, f32, f32),
    pub rings: Vec<Ring>,
    pub parent: Option<usize>,
}

pub struct Geo {
    pub regions: Vec<Area>,
    pub prefs: Vec<Area>,
}

pub struct Epicenter {
    pub name: String,
    /// polygon -> rings (first ring is the outer boundary)
    pub polys: Vec<Vec<Ring>>,
}

struct Reader<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn u16(&mut self) -> u16 {
        let v = u16::from_le_bytes([self.d[self.p], self.d[self.p + 1]]);
        self.p += 2;
        v
    }
    fn u32(&mut self) -> u32 {
        let v = u32::from_le_bytes(self.d[self.p..self.p + 4].try_into().unwrap());
        self.p += 4;
        v
    }
    fn f32(&mut self) -> f32 {
        let v = f32::from_le_bytes(self.d[self.p..self.p + 4].try_into().unwrap());
        self.p += 4;
        v
    }
    fn string(&mut self, n: usize) -> String {
        let s = String::from_utf8_lossy(&self.d[self.p..self.p + n]).into_owned();
        self.p += n;
        s
    }
    fn rings(&mut self, count: usize) -> Vec<Ring> {
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            let n = self.u32() as usize;
            let mut ring = Vec::with_capacity(n);
            for _ in 0..n {
                let x = self.f32();
                let y = self.f32();
                ring.push((x, y));
            }
            out.push(ring);
        }
        out
    }
}

pub fn load_geo(data: &'static [u8]) -> Geo {
    assert_eq!(&data[..5], b"EEWG1");
    let mut r = Reader { d: data, p: 5 };
    let nreg = r.u32() as usize;
    let npref = r.u32() as usize;
    let mut areas = Vec::with_capacity(nreg + npref);
    for i in 0..nreg + npref {
        let code = r.u16();
        let nl = r.u16() as usize;
        let name = r.string(nl);
        let bbox = (r.f32(), r.f32(), r.f32(), r.f32());
        let nr = r.u32() as usize;
        let rings = r.rings(nr);
        let parent = if i >= nreg { Some(r.u16() as usize) } else { None };
        areas.push(Area { code, name, bbox, rings, parent });
    }
    let prefs = areas.split_off(nreg);
    Geo { regions: areas, prefs }
}

pub fn load_epicenters(data: &'static [u8]) -> Vec<Epicenter> {
    assert_eq!(&data[..5], b"EEWE1");
    let mut r = Reader { d: data, p: 5 };
    let n = r.u32() as usize;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let nl = r.u16() as usize;
        let name = r.string(nl);
        let np = r.u32() as usize;
        let mut polys = Vec::with_capacity(np);
        for _ in 0..np {
            let nr = r.u32() as usize;
            let mut rings = Vec::with_capacity(nr);
            for _ in 0..nr {
                let cnt = r.u32() as usize;
                let mut ring = Vec::with_capacity(cnt);
                for _ in 0..cnt {
                    let x = r.f32();
                    let y = r.f32();
                    ring.push((x, y));
                }
                rings.push(ring);
            }
            polys.push(rings);
        }
        out.push(Epicenter { name, polys });
    }
    out
}

fn in_ring(x: f32, y: f32, ring: &Ring) -> bool {
    let mut inside = false;
    let n = ring.len();
    if n < 3 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = ring[i];
        let (xj, yj) = ring[j];
        if (yi > y) != (yj > y) {
            let xx = (xj - xi) * (y - yi) / (yj - yi) + xi;
            if x < xx {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

pub fn find_epicenter(list: &[Epicenter], lon: f64, lat: f64) -> Option<String> {
    let (x, y) = (lon as f32, lat as f32);
    for e in list {
        for poly in &e.polys {
            if poly.is_empty() {
                continue;
            }
            if in_ring(x, y, &poly[0]) && !poly[1..].iter().any(|h| in_ring(x, y, h)) {
                return Some(e.name.clone());
            }
        }
    }
    // fallback: nearest polygon bounding-box centre
    let mut best: Option<(f32, &str)> = None;
    for e in list {
        for poly in &e.polys {
            if poly.is_empty() || poly[0].is_empty() {
                continue;
            }
            let mut minx = f32::INFINITY;
            let mut maxx = f32::NEG_INFINITY;
            let mut miny = f32::INFINITY;
            let mut maxy = f32::NEG_INFINITY;
            for &(px, py) in &poly[0] {
                minx = minx.min(px); maxx = maxx.max(px);
                miny = miny.min(py); maxy = maxy.max(py);
            }
            let d = ((minx + maxx) / 2.0 - x).powi(2) + ((miny + maxy) / 2.0 - y).powi(2);
            if best.map_or(true, |(bd, _)| d < bd) {
                best = Some((d, e.name.as_str()));
            }
        }
    }
    best.map(|(_, n)| n.to_string())
}

impl Geo {
    pub fn pref_index(&self, name: &str) -> Option<usize> {
        self.prefs.iter().position(|a| a.name == name)
    }

    /// API / user supplied prefecture names may carry a 都道府県 suffix.
    /// Map them to the exact 府県予報区 names used by the GIS data.
    pub fn normalize_pref(&self, name: &str) -> Option<String> {
        if self.pref_index(name).is_some() {
            return Some(name.to_string());
        }
        for suffix in ["県", "府", "都", "道"] {
            if let Some(base) = name.strip_suffix(suffix) {
                if self.pref_index(base).is_some() {
                    return Some(base.to_string());
                }
            }
        }
        // e.g. "福岡県北西沖" is not a forecast area; try a prefix match
        for area in &self.prefs {
            if name.starts_with(&area.name) || area.name.starts_with(name) {
                return Some(area.name.clone());
            }
        }
        None
    }

    pub fn normalize_prefs(&self, names: &[String]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for n in names {
            let v = self.normalize_pref(n.trim()).unwrap_or_else(|| n.trim().to_string());
            if !v.is_empty() && !out.contains(&v) {
                out.push(v);
            }
        }
        out
    }

    /// Group prefecture forecast areas into regional forecast areas,
    /// preserving first-appearance order.
    ///
    /// NHK's on-screen rules (SC-Sendai-City, 2021):
    ///   1. show forecast areas (prefectures) by default
    ///   2. once they no longer fit two lines, switch to regional names
    ///   3. but a region contributing only one area keeps the area name
    ///   4. 新潟 and the island regions count as regions of their own
    pub fn consolidate(&self, pref_names: &[String]) -> Vec<String> {
        let mut groups: Vec<(String, usize, String)> = Vec::new(); // (group, count, first area)
        for n in pref_names {
            let Some(i) = self.pref_index(n) else { continue };
            // rule 4: 新潟 is displayed independently of 北陸
            let group = if n == "新潟" {
                "新潟".to_string()
            } else {
                match self.prefs[i].parent {
                    Some(p) => self.regions[p].name.clone(),
                    None => n.clone(),
                }
            };
            match groups.iter_mut().find(|g| g.0 == group) {
                Some(g) => g.1 += 1,
                None => groups.push((group, 1, n.clone())),
            }
        }
        // rule 3: a group with a single area keeps that area's own name
        groups.into_iter()
            .map(|(group, count, first)| if count == 1 { first } else { group })
            .collect()
    }

    /// Bounding box of the largest land ring of each selected area (ignores remote islets).
    pub fn main_bbox(&self, pref_names: &[String]) -> Option<(f32, f32, f32, f32)> {
        let mut box_: Option<(f32, f32, f32, f32)> = None;
        for n in pref_names {
            let Some(i) = self.pref_index(n) else { continue };
            let Some(ring) = self.prefs[i].rings.iter().max_by_key(|r| r.len()) else { continue };
            if ring.is_empty() { continue }
            let (mut x0, mut y0, mut x1, mut y1) = (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
            for &(x, y) in ring {
                x0 = x0.min(x); y0 = y0.min(y); x1 = x1.max(x); y1 = y1.max(y);
            }
            box_ = Some(match box_ {
                None => (x0, y0, x1, y1),
                Some((a, b, c, d)) => (a.min(x0), b.min(y0), c.max(x1), d.max(y1)),
            });
        }
        box_
    }
}
