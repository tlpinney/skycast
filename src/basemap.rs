//! Coastlines, lakes, and country/state borders for the radar map, from Natural Earth
//! 1:50m (public domain), converted to a compact form by `tools/ne2bin.py`.

use std::sync::OnceLock;

static COAST_BIN: &[u8] = include_bytes!("../assets/coast.bin");
static BORDERS_BIN: &[u8] = include_bytes!("../assets/borders.bin");

pub struct Polyline {
    /// (west, east, south, north), for skipping lines outside the view.
    pub bbox: (f32, f32, f32, f32),
    pub points: Vec<(f32, f32)>,
}

impl Polyline {
    pub fn intersects(&self, w: f64, e: f64, s: f64, n: f64) -> bool {
        let (lw, le, ls, ln) = self.bbox;
        (le as f64) >= w && (lw as f64) <= e && (ln as f64) >= s && (ls as f64) <= n
    }
}

/// Format: u32 line count, then per line u32 point count and that many (f32 lon, f32 lat), little endian.
fn parse(bytes: &[u8]) -> Vec<Polyline> {
    let mut pos = 0;
    let u32_at = |pos: &mut usize| {
        let v = u32::from_le_bytes(bytes[*pos..*pos + 4].try_into().unwrap());
        *pos += 4;
        v
    };
    let lines = u32_at(&mut pos);
    (0..lines)
        .map(|_| {
            let n = u32_at(&mut pos) as usize;
            let points: Vec<(f32, f32)> = (0..n)
                .map(|k| {
                    let at = pos + k * 8;
                    let f = |o: usize| f32::from_le_bytes(bytes[at + o..at + o + 4].try_into().unwrap());
                    (f(0), f(4))
                })
                .collect();
            pos += n * 8;
            let bbox = points.iter().fold(
                (f32::INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::NEG_INFINITY),
                |(w, e, s, n), &(x, y)| (w.min(x), e.max(x), s.min(y), n.max(y)),
            );
            Polyline { bbox, points }
        })
        .collect()
}

pub fn coast() -> &'static [Polyline] {
    static C: OnceLock<Vec<Polyline>> = OnceLock::new();
    C.get_or_init(|| parse(COAST_BIN))
}

pub fn borders() -> &'static [Polyline] {
    static B: OnceLock<Vec<Polyline>> = OnceLock::new();
    B.get_or_init(|| parse(BORDERS_BIN))
}

#[cfg(test)]
mod tests {
    #[test]
    fn data_loads() {
        assert!(super::coast().len() > 1000);
        assert!(super::borders().len() > 500);
    }
}
