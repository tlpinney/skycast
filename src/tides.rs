//! Tide predictions. US coasts use NOAA CO-OPS station predictions (the nearest station
//! within reach); elsewhere, Open-Meteo's marine model at the nearest sea grid point.

use std::path::PathBuf;
use std::time::{Duration as StdDuration, SystemTime};

use anyhow::{Context, Result, bail};
use chrono::{Duration, NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};

use crate::api::{Place, Units};

const NOAA_STATIONS_URL: &str =
    "https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi/stations.json?type=tidepredictions";
const NOAA_DATA_URL: &str = "https://api.tidesandcurrents.noaa.gov/api/prod/datagetter";
const MARINE_URL: &str = "https://marine-api.open-meteo.com/v1/marine";

/// Use a NOAA station only if it's this close; tides change a lot along a coast.
const MAX_STATION_KM: f64 = 80.0;
/// How far out to look for a sea grid point in the marine model.
const MARINE_RINGS_KM: [f64; 3] = [10.0, 25.0, 50.0];
/// Consecutive highs and lows are never this far apart (even diurnal tides are ~12–13 h);
/// a larger gap means bad data, and filling it would build an enormous curve.
const MAX_EVENT_GAP: Duration = Duration::hours(30);
const STATION_CACHE_MAX_AGE: StdDuration = StdDuration::from_secs(30 * 24 * 3600);

#[derive(Debug, Clone)]
pub struct TideEvent {
    pub time: NaiveDateTime,
    pub height: f64,
    pub high: bool,
}

#[derive(Debug, Clone)]
pub struct Tides {
    /// "NOAA" or "Open-Meteo marine model".
    pub source: &'static str,
    /// Station name, or a description of the grid point.
    pub station: String,
    pub distance_km: f64,
    /// Height reference: "MLLW" (NOAA) or "MSL" (model).
    pub datum: &'static str,
    /// "ft" or "m".
    pub unit: &'static str,
    /// Highs and lows, oldest first, covering at least yesterday through tomorrow.
    pub events: Vec<TideEvent>,
    /// Heights every 10 minutes over the same span, in local time at the place.
    pub curve: Vec<(NaiveDateTime, f64)>,
}

impl Tides {
    /// Interpolated height at `t`, if inside the curve.
    pub fn height_at(&self, t: NaiveDateTime) -> Option<f64> {
        let i = self.curve.iter().position(|(ct, _)| *ct > t)?;
        if i == 0 {
            return None;
        }
        let ((t0, h0), (t1, h1)) = (self.curve[i - 1], self.curve[i]);
        let f = (t - t0).num_seconds() as f64 / (t1 - t0).num_seconds().max(1) as f64;
        Some(h0 + (h1 - h0) * f)
    }

    pub fn next_events(&self, t: NaiveDateTime) -> impl Iterator<Item = &TideEvent> {
        self.events.iter().filter(move |e| e.time > t)
    }
}

fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent("weather-tui/0.1")
        .timeout(std::time::Duration::from_secs(30))
        .build()?)
}

/// Great-circle distance in km.
pub fn distance_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dp = p2 - p1;
    let dl = (lon2 - lon1).to_radians();
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    6371.0 * 2.0 * a.sqrt().asin()
}

/// Tides for the place over yesterday..=tomorrow (local), or None if there's no coast nearby.
pub fn fetch(place: &Place, units: Units, today: NaiveDate) -> Result<Option<Tides>> {
    if let Some(t) = fetch_noaa(place, units, today)? {
        return Ok(Some(t));
    }
    fetch_marine(place, units, today)
}

// ---------- NOAA ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Station {
    id: String,
    name: String,
    #[serde(default)]
    state: Option<String>,
    lat: f64,
    lng: f64,
}

#[derive(Deserialize)]
struct StationList {
    stations: Vec<Station>,
}

fn cache_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("weather-tui").join("noaa-tide-stations.json"))
}

/// NOAA's tide prediction stations, cached for a month (the full list is ~2 MB).
fn stations() -> Result<Vec<Station>> {
    let path = cache_path();
    if let Some(p) = &path {
        let fresh = std::fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| SystemTime::now().duration_since(m).ok())
            .is_some_and(|age| age < STATION_CACHE_MAX_AGE);
        if fresh
            && let Ok(bytes) = std::fs::read(p)
            && let Ok(list) = serde_json::from_slice::<Vec<Station>>(&bytes)
            && !list.is_empty()
        {
            return Ok(list);
        }
    }
    let list: StationList = client()?
        .get(NOAA_STATIONS_URL)
        .send()?
        .error_for_status()?
        .json()
        .context("bad NOAA station list")?;
    if let Some(p) = &path {
        // Best effort: a failed cache write only costs a re-download next time.
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // Write then rename, so another instance never reads a half-written file.
        if let Ok(json) = serde_json::to_vec(&list.stations) {
            let tmp = p.with_extension(format!("json.{}.tmp", std::process::id()));
            if std::fs::write(&tmp, json).is_ok() && std::fs::rename(&tmp, p).is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
        }
    }
    Ok(list.stations)
}

#[derive(Deserialize)]
struct NoaaPredictions {
    predictions: Option<Vec<NoaaPoint>>,
    error: Option<NoaaError>,
}

#[derive(Deserialize)]
struct NoaaError {
    message: String,
}

#[derive(Deserialize)]
struct NoaaPoint {
    t: String,
    v: String,
    #[serde(rename = "type")]
    kind: Option<String>,
}

fn fetch_noaa(place: &Place, units: Units, today: NaiveDate) -> Result<Option<Tides>> {
    let stations = stations()?;
    let Some((st, km)) = stations
        .iter()
        .map(|s| (s, distance_km(place.lat, place.lon, s.lat, s.lng)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
    else {
        return Ok(None);
    };
    if km > MAX_STATION_KM {
        return Ok(None);
    }
    // Station ids are 7 digits; never splice anything else into the URL.
    if st.id.len() > 10 || !st.id.chars().all(|c| c.is_ascii_alphanumeric()) {
        bail!("unexpected NOAA station id {:?}", st.id);
    }

    let begin = today - Duration::days(1);
    let resp: NoaaPredictions = client()?
        .get(NOAA_DATA_URL)
        .query(&[
            ("product", "predictions"),
            ("application", "weather-tui"),
            ("begin_date", &begin.format("%Y%m%d").to_string()),
            ("range", "96"),
            ("datum", "MLLW"),
            ("station", &st.id),
            ("time_zone", "lst_ldt"),
            ("units", if units == Units::Imperial { "english" } else { "metric" }),
            ("interval", "hilo"),
            ("format", "json"),
        ])
        .send()?
        .error_for_status()?
        .json()
        .context("bad NOAA predictions")?;
    if let Some(e) = resp.error {
        bail!("NOAA: {}", e.message);
    }
    let mut events: Vec<TideEvent> = resp
        .predictions
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| {
            Some(TideEvent {
                time: NaiveDateTime::parse_from_str(&p.t, "%Y-%m-%d %H:%M").ok()?,
                height: p.v.trim().parse().ok()?,
                high: p.kind.as_deref()? == "H",
            })
        })
        .collect();
    events.sort_by_key(|e| e.time);
    if events.len() < 2 {
        return Ok(None);
    }
    let mut name = title_case(&st.name);
    if let Some(state) = st.state.as_deref().filter(|s| !s.is_empty() && !name.contains(*s)) {
        name = format!("{name}, {state}");
    }
    Ok(Some(Tides {
        source: "NOAA",
        station: name,
        distance_km: km,
        datum: "MLLW",
        unit: if units == Units::Imperial { "ft" } else { "m" },
        curve: curve_from_events(&events),
        events,
    }))
}

/// "WASHINGTON, Washington Channel" -> "Washington, Washington Channel".
fn title_case(s: &str) -> String {
    s.split(' ')
        .map(|w| {
            if w.chars().any(|c| c.is_lowercase()) || w.len() <= 2 {
                return w.to_string();
            }
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_string() + &c.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A smooth curve through alternating highs and lows (half-cosine between each pair),
/// sampled every 10 minutes. This is how tide tables are usually drawn between extremes.
pub fn curve_from_events(events: &[TideEvent]) -> Vec<(NaiveDateTime, f64)> {
    let mut out = Vec::new();
    for pair in events.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if b.time <= a.time || b.time - a.time > MAX_EVENT_GAP {
            continue;
        }
        let span = (b.time - a.time).num_seconds() as f64;
        let mut t = a.time;
        while t < b.time {
            let f = (t - a.time).num_seconds() as f64 / span;
            let h = a.height + (b.height - a.height) * (1.0 - (std::f64::consts::PI * f).cos()) / 2.0;
            out.push((t, h));
            t += Duration::minutes(10);
        }
    }
    if let Some(last) = events.last() {
        out.push((last.time, last.height));
    }
    out
}

// ---------- Open-Meteo marine ----------

#[derive(Deserialize)]
struct MarinePoint {
    latitude: f64,
    longitude: f64,
    hourly: Option<MarineHourly>,
}

#[derive(Deserialize)]
struct MarineHourly {
    time: Vec<String>,
    sea_level_height_msl: Vec<Option<f64>>,
}

/// Points at the place and on rings around it, so a coastal town finds the sea next to it.
fn ring_points(lat: f64, lon: f64) -> Vec<(f64, f64)> {
    let mut pts = vec![(lat, lon)];
    for km in MARINE_RINGS_KM {
        for k in 0..8 {
            let bearing = (k as f64 * 45.0).to_radians();
            let dlat = km / 111.0 * bearing.cos();
            let dlon = km / (111.0 * lat.to_radians().cos().max(0.05)) * bearing.sin();
            pts.push((lat + dlat, lon + dlon));
        }
    }
    pts
}

fn fetch_marine(place: &Place, units: Units, today: NaiveDate) -> Result<Option<Tides>> {
    let pts = ring_points(place.lat, place.lon);
    let join = |f: fn(&(f64, f64)) -> f64| pts.iter().map(|p| format!("{:.4}", f(p))).collect::<Vec<_>>().join(",");
    let resp: Vec<MarinePoint> = client()?
        .get(MARINE_URL)
        .query(&[
            ("latitude", join(|p| p.0)),
            ("longitude", join(|p| p.1)),
            ("hourly", "sea_level_height_msl".into()),
            ("timezone", "auto".into()),
            ("past_days", "1".into()),
            ("forecast_days", "3".into()),
        ])
        .send()?
        .error_for_status()?
        .json()
        .context("bad marine response")?;

    // Nearest point that actually has sea level data.
    let Some((p, km)) = resp
        .iter()
        .filter(|p| {
            p.hourly.as_ref().is_some_and(|h| h.sea_level_height_msl.iter().filter(|v| v.is_some()).count() > 24)
        })
        .map(|p| (p, distance_km(place.lat, place.lon, p.latitude, p.longitude)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
    else {
        return Ok(None);
    };
    let h = p.hourly.as_ref().unwrap();
    let scale = if units == Units::Imperial { 3.280_84 } else { 1.0 };
    let hourly: Vec<(NaiveDateTime, f64)> = h
        .time
        .iter()
        .zip(&h.sea_level_height_msl)
        .filter_map(|(t, v)| Some((NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M").ok()?, (*v)? * scale)))
        .filter(|(t, _)| t.date() >= today - Duration::days(1) && t.date() <= today + Duration::days(2))
        .collect();
    let events = extremes(&hourly);
    if events.len() < 2 {
        return Ok(None);
    }
    Ok(Some(Tides {
        source: "Open-Meteo marine model",
        station: format!("sea grid point {:.2}, {:.2}", p.latitude, p.longitude),
        distance_km: km,
        datum: "MSL",
        unit: if units == Units::Imperial { "ft" } else { "m" },
        curve: resample(&hourly),
        events,
    }))
}

/// Highs and lows of an hourly series, with times refined by fitting a parabola through
/// each extreme and its neighbours.
pub fn extremes(series: &[(NaiveDateTime, f64)]) -> Vec<TideEvent> {
    let mut out = Vec::new();
    for w in series.windows(3) {
        let ((_, a), (t, b), (_, c)) = (w[0], w[1], w[2]);
        let high = b > a && b >= c;
        let low = b < a && b <= c;
        if !(high || low) {
            continue;
        }
        let denom = a - 2.0 * b + c;
        let (shift, peak) = if denom.abs() > 1e-9 {
            let s = 0.5 * (a - c) / denom; // in hours, within ±0.5
            (s.clamp(-0.5, 0.5), b - 0.25 * (a - c) * s)
        } else {
            (0.0, b)
        };
        out.push(TideEvent { time: t + Duration::seconds((shift * 3600.0) as i64), height: peak, high });
    }
    out
}

/// Linear resample of an hourly series to 10-minute steps.
fn resample(series: &[(NaiveDateTime, f64)]) -> Vec<(NaiveDateTime, f64)> {
    let mut out = Vec::new();
    for w in series.windows(2) {
        let ((t0, h0), (t1, h1)) = (w[0], w[1]);
        let span = (t1 - t0).num_seconds() as f64;
        let mut t = t0;
        while t < t1 {
            let f = (t - t0).num_seconds() as f64 / span;
            out.push((t, h0 + (h1 - h0) * f));
            t += Duration::minutes(10);
        }
    }
    if let Some(&last) = series.last() {
        out.push(last);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(d: u32, h: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, d).unwrap().and_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn distance_is_sane() {
        // Herndon to the Washington Channel tide station: about 34 km.
        let km = distance_km(38.96955, -77.3861, 38.8733, -77.0217);
        assert!((km - 33.5).abs() < 2.0, "{km}");
    }

    #[test]
    fn curve_passes_through_events() {
        let ev = vec![
            TideEvent { time: at(3, 2, 22), height: 2.8, high: true },
            TideEvent { time: at(3, 9, 42), height: 0.4, high: false },
            TideEvent { time: at(3, 15, 9), height: 2.8, high: true },
        ];
        let c = curve_from_events(&ev);
        let tides = Tides {
            source: "test",
            station: String::new(),
            distance_km: 0.0,
            datum: "MLLW",
            unit: "ft",
            events: ev,
            curve: c,
        };
        assert!((tides.height_at(at(3, 2, 22) + Duration::seconds(1)).unwrap() - 2.8).abs() < 0.01);
        let mid = tides.height_at(at(3, 6, 2)).unwrap();
        assert!((mid - 1.6).abs() < 0.05, "{mid}");
        assert!(tides.height_at(at(3, 1, 0)).is_none());
        assert_eq!(tides.next_events(at(3, 10, 0)).next().map(|e| e.high), Some(true));
    }

    #[test]
    fn curve_skips_implausible_gaps() {
        // Events years apart (bad data) must not be filled with 10-minute points.
        let far = at(3, 0, 0) + Duration::days(3650);
        let ev = vec![
            TideEvent { time: at(3, 2, 0), height: 2.0, high: true },
            TideEvent { time: at(3, 8, 0), height: 0.0, high: false },
            TideEvent { time: far, height: 2.0, high: true },
        ];
        assert!(curve_from_events(&ev).len() < 50);
    }

    #[test]
    fn finds_extremes_in_hourly_series() {
        // A 12.42 h tide sampled hourly: highs at 3.1 h and 15.5 h, lows at 9.3 h and 21.7 h.
        let series: Vec<_> = (0..24)
            .map(|h| (at(3, h, 0), (2.0 * std::f64::consts::PI * (h as f64 - 3.1) / 12.42).cos()))
            .collect();
        let ev = extremes(&series);
        assert_eq!(ev.iter().map(|e| e.high).collect::<Vec<_>>(), vec![true, false, true, false]);
        let first = (ev[0].time - at(3, 3, 6)).num_minutes().abs();
        assert!(first <= 6, "high at {}", ev[0].time);
    }

    #[test]
    fn title_cases_noaa_names() {
        assert_eq!(title_case("WASHINGTON, Washington Channel"), "Washington, Washington Channel");
    }
}
