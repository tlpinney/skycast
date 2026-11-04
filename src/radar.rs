//! Precipitation radar from RainViewer (worldwide, no key, tiles up to zoom 7),
//! sampled onto a Braille dot grid in Web Mercator pixel space.

use std::f64::consts::PI;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

const MAPS_URL: &str = "https://api.rainviewer.com/public/weather-maps.json";
/// The only tile host we fetch from, whatever the index says.
const TILE_HOST: &str = "https://tilecache.rainviewer.com";
pub const TILE: usize = 256;
/// RainViewer serves a "Zoom Level Not Supported" image above this.
pub const MAX_ZOOM: u8 = 7;

/// Web Mercator can't show the poles; views are centered no further out than this.
pub const MAX_LAT: f64 = 85.0;

/// Selectable view widths, widest first.
pub const ZOOM_KM: [f64; 5] = [2400.0, 1200.0, 600.0, 300.0, 150.0];
pub const DEFAULT_ZOOM: usize = 3;

#[derive(Debug, Clone)]
pub struct RadarFrame {
    /// Unix seconds.
    pub time: i64,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct Frames {
    pub host: String,
    /// Oldest first; the last one is the latest scan.
    pub frames: Vec<RadarFrame>,
}

#[derive(Deserialize)]
struct RawMaps {
    host: String,
    radar: RawRadar,
}

#[derive(Deserialize)]
struct RawRadar {
    past: Vec<RawFrame>,
}

#[derive(Deserialize)]
struct RawFrame {
    time: i64,
    path: String,
}

fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent("skycast/0.1")
        .timeout(std::time::Duration::from_secs(15))
        .build()?)
}

pub fn fetch_frames() -> Result<Frames> {
    let raw: RawMaps = client()?
        .get(MAPS_URL)
        .send()?
        .error_for_status()?
        .json()
        .context("bad radar index")?;
    if raw.host != TILE_HOST {
        bail!("unexpected radar tile host {:?}", raw.host);
    }
    // Drop anything that isn't a plain "/v2/radar/<id>" path, since it's spliced into tile URLs.
    let frames: Vec<RadarFrame> = raw
        .radar
        .past
        .into_iter()
        .filter(|f| valid_path(&f.path))
        .map(|f| RadarFrame { time: f.time, path: f.path })
        .collect();
    if frames.is_empty() {
        bail!("no radar frames available");
    }
    Ok(Frames { host: raw.host, frames })
}

fn valid_path(path: &str) -> bool {
    path.strip_prefix("/v2/radar/")
        .is_some_and(|id| !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric()))
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TileKey {
    pub path: String,
    pub z: u8,
    pub x: u32,
    pub y: u32,
}

/// Precipitation level for each pixel (row-major, TILE × TILE): 0 none … 6 extreme.
pub struct Tile {
    pub levels: Vec<u8>,
}

impl Tile {
    fn level(&self, x: usize, y: usize) -> u8 {
        self.levels[y * TILE + x]
    }
}

pub fn fetch_tile(host: &str, key: &TileKey) -> Result<Tile> {
    // Color scheme 2 (the only one RainViewer still serves), smoothing on, no snow coloring.
    let url = format!("{host}{}/{TILE}/{}/{}/{}/2/1_0.png", key.path, key.z, key.x, key.y);
    let bytes = client()?.get(&url).send()?.error_for_status()?.bytes()?;

    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().context("bad radar tile")?;
    let mut buf = vec![0; reader.output_buffer_size().context("radar tile too large")?];
    let info = reader.next_frame(&mut buf).context("bad radar tile")?;
    if info.width as usize != TILE || info.height as usize != TILE {
        bail!("unexpected radar tile size {}x{}", info.width, info.height);
    }
    let rgba = |i: usize| -> (u8, u8, u8, u8) {
        match info.color_type {
            png::ColorType::Rgba => (buf[i * 4], buf[i * 4 + 1], buf[i * 4 + 2], buf[i * 4 + 3]),
            png::ColorType::Rgb => (buf[i * 3], buf[i * 3 + 1], buf[i * 3 + 2], 255),
            png::ColorType::GrayscaleAlpha => (buf[i * 2], buf[i * 2], buf[i * 2], buf[i * 2 + 1]),
            _ => (buf[i], buf[i], buf[i], 255),
        }
    };
    if !matches!(info.color_type, png::ColorType::Rgba) {
        // Anything other than RGBA is RainViewer's error placeholder, not radar.
        bail!("radar tile unavailable");
    }
    let levels = (0..TILE * TILE)
        .map(|i| {
            let (r, g, b, a) = rgba(i);
            classify(r, g, b, a)
        })
        .collect();
    Ok(Tile { levels })
}

/// Map a "Universal Blue" radar color to a level. The scheme goes from translucent beige
/// (drizzle) through light and dark blue (rain), then yellow, orange/red and purple/white.
pub fn classify(r: u8, g: u8, b: u8, a: u8) -> u8 {
    if a < 40 {
        return 0;
    }
    if a < 255 {
        return 1;
    }
    let (r, g, b) = (r as f32, g as f32, b as f32);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let light = (max + min) / 510.0;
    if d < 20.0 {
        return if light > 0.85 { 6 } else { 1 };
    }
    let hue = if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let sat = d / max;
    match hue {
        // Opaque beige is still light precipitation, not the yellow of heavy rain.
        h if (30.0..75.0).contains(&h) && sat < 0.45 => 1,
        h if (170.0..260.0).contains(&h) => {
            if light >= 0.5 {
                2
            } else {
                3
            }
        }
        h if (45.0..170.0).contains(&h) => 4,
        h if !(45.0..330.0).contains(&h) => 5,
        _ => 6,
    }
}

/// Web Mercator: lon/lat to global pixel coordinates at zoom `z`.
pub fn project(lon: f64, lat: f64, z: u8) -> (f64, f64) {
    let world = TILE as f64 * f64::from(1u32 << z);
    let lat = lat.clamp(-85.05, 85.05).to_radians();
    let x = (lon + 180.0) / 360.0 * world;
    let y = (1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / PI) / 2.0 * world;
    (x, y)
}

pub fn unproject(x: f64, y: f64, z: u8) -> (f64, f64) {
    let world = TILE as f64 * f64::from(1u32 << z);
    let lon = x / world * 360.0 - 180.0;
    let lat = (PI * (1.0 - 2.0 * y / world)).sinh().atan().to_degrees();
    (lon, lat)
}

/// What part of the world a Braille grid shows, in global pixel coordinates at zoom `z`.
#[derive(Debug, Clone, Copy)]
pub struct View {
    pub z: u8,
    pub left: f64,
    pub top: f64,
    pub px_per_dot: f64,
    pub dots_w: usize,
    pub dots_h: usize,
}

impl View {
    /// Center on (lon, lat) and span about `km` across `dots_w` dots.
    pub fn new(lon: f64, lat: f64, km: f64, dots_w: usize, dots_h: usize) -> View {
        let dots_w = dots_w.max(1);
        let dots_h = dots_h.max(1);
        // At ±90° the scale factor is 0 and the view would be infinitely wide.
        let lat = lat.clamp(-MAX_LAT, MAX_LAT);
        let m_per_px_z0 = 156_543.034 * lat.to_radians().cos();
        // Lowest zoom whose pixels are at least as fine as the dots, capped at MAX_ZOOM.
        let want = (dots_w as f64 * m_per_px_z0 / (km * 1000.0)).log2().ceil();
        let z = want.clamp(1.0, MAX_ZOOM as f64) as u8;
        let span_px = km * 1000.0 / (m_per_px_z0 / f64::from(1u32 << z));
        let px_per_dot = span_px / dots_w as f64;
        let (cx, cy) = project(lon, lat, z);
        View {
            z,
            left: cx - span_px / 2.0,
            top: cy - dots_h as f64 * px_per_dot / 2.0,
            px_per_dot,
            dots_w,
            dots_h,
        }
    }

    pub fn width_px(&self) -> f64 {
        self.dots_w as f64 * self.px_per_dot
    }

    pub fn height_px(&self) -> f64 {
        self.dots_h as f64 * self.px_per_dot
    }

    /// Tiles (x, y) covering the view. x wraps around the antimeridian.
    pub fn tiles(&self) -> Vec<(u32, u32)> {
        let n = 1i64 << self.z;
        let t = TILE as f64;
        let x0 = (self.left / t).floor() as i64;
        // A view wider than the world still only needs each column of tiles once.
        let x1 = (((self.left + self.width_px()) / t).floor() as i64).min(x0 + n - 1);
        let (y0, y1) = ((self.top / t).floor() as i64, ((self.top + self.height_px()) / t).floor() as i64);
        let mut out = Vec::new();
        for y in y0.max(0)..=y1.min(n - 1) {
            for x in x0..=x1 {
                out.push((x.rem_euclid(n) as u32, y as u32));
            }
        }
        out
    }

    /// Lon/lat bounds of the view: (west, east, south, north).
    pub fn bounds(&self) -> (f64, f64, f64, f64) {
        let (w, n) = unproject(self.left, self.top, self.z);
        let (e, s) = unproject(self.left + self.width_px(), self.top + self.height_px(), self.z);
        (w, e, s, n)
    }

    /// The level for dot (i, j), sampling up to 3×3 pixels inside it. Any precipitation
    /// lights the dot (so small showers don't vanish when zoomed out), colored by the most
    /// common level among the wet samples rather than the heaviest, which would let one
    /// heavy pixel paint the whole dot. `tile` returns None for tiles not loaded yet.
    pub fn sample<'a>(&self, i: usize, j: usize, tile: &impl Fn(u32, u32) -> Option<&'a Tile>) -> u8 {
        let n = 1i64 << self.z;
        let steps = (self.px_per_dot.ceil() as usize).clamp(1, 3);
        let mut counts = [0u8; 7];
        for sy in 0..steps {
            for sx in 0..steps {
                let px = self.left + (i as f64 + (sx as f64 + 0.5) / steps as f64) * self.px_per_dot;
                let py = self.top + (j as f64 + (sy as f64 + 0.5) / steps as f64) * self.px_per_dot;
                if py < 0.0 || py >= (n * TILE as i64) as f64 {
                    continue;
                }
                let (tx, ty) = ((px / TILE as f64).floor() as i64, (py / TILE as f64).floor() as i64);
                if let Some(t) = tile(tx.rem_euclid(n) as u32, ty as u32) {
                    let lx = (px - (tx * TILE as i64) as f64) as usize;
                    let ly = (py - (ty * TILE as i64) as f64) as usize;
                    counts[t.level(lx.min(TILE - 1), ly.min(TILE - 1)) as usize] += 1;
                }
            }
        }
        majority(&counts)
    }
}

/// The most common non-zero level in `counts` (indexed by level), heavier on ties; 0 if
/// there is none.
pub fn majority(counts: &[u8; 7]) -> u8 {
    (1..7u8).filter(|&l| counts[l as usize] > 0).max_by_key(|&l| (counts[l as usize], l)).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_round_trips() {
        let (x, y) = project(-77.39, 38.97, 7);
        let (lon, lat) = unproject(x, y, 7);
        assert!((lon + 77.39).abs() < 1e-9 && (lat - 38.97).abs() < 1e-9);
        // Herndon is in tile (36, 48) at zoom 7 (y ≈ 48.93).
        assert_eq!(((x / 256.0) as u32, (y / 256.0) as u32), (36, 48));
    }

    #[test]
    fn classifies_universal_blue() {
        assert_eq!(classify(0, 0, 0, 0), 0);
        assert_eq!(classify(0xb6, 0xa9, 0x7e, 130), 1); // translucent beige
        assert_eq!(classify(0x88, 0xdd, 0xee, 255), 2); // light blue
        assert_eq!(classify(0x00, 0x88, 0xbf, 255), 3); // dark blue
        assert_eq!(classify(0xff, 0xee, 0x00, 255), 4); // yellow
        assert_eq!(classify(0xff, 0x30, 0x00, 255), 5); // red
        assert_eq!(classify(0xc0, 0x00, 0xc0, 255), 6); // purple
    }

    #[test]
    fn view_is_bounded_at_the_poles() {
        for lat in [90.0, -90.0, 89.99] {
            let v = View::new(0.0, lat, 2400.0, 400, 200);
            assert!(v.width_px().is_finite());
            assert!(v.tiles().len() <= 1 << (2 * v.z as usize));
        }
    }

    #[test]
    fn only_plain_frame_paths_are_accepted() {
        assert!(valid_path("/v2/radar/321e19982727"));
        assert!(!valid_path("/v2/radar/"));
        assert!(!valid_path("/v2/radar/../../x"));
        assert!(!valid_path("/v2/radar/abc?x=1"));
        assert!(!valid_path("https://evil.example/v2/radar/abc"));
    }

    #[test]
    fn majority_prefers_common_then_heavier() {
        assert_eq!(majority(&[9, 0, 0, 0, 0, 0, 0]), 0);
        // Mostly light rain with one heavy pixel reads as light rain.
        assert_eq!(majority(&[0, 0, 5, 0, 0, 1, 0]), 2);
        // A tie goes to the heavier level, so real heavy cores still show.
        assert_eq!(majority(&[0, 2, 0, 0, 2, 0, 0]), 4);
    }

    #[test]
    fn view_picks_zoom_and_tiles() {
        let v = View::new(-77.39, 38.97, 300.0, 156, 160);
        assert!(v.z <= MAX_ZOOM);
        assert!(!v.tiles().is_empty() && v.tiles().len() <= 9);
        let (w, e, s, n) = v.bounds();
        assert!(w < -77.39 && e > -77.39 && s < 38.97 && n > 38.97);
    }
}
