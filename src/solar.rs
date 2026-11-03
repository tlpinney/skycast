//! Sun and moon, computed locally (no API): the NOAA solar position equations and a
//! low-precision lunar longitude (Meeus, *Astronomical Algorithms*, ch. 25 and 47).
//! Good to about a minute for sun events and well under an hour for moon phases.

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, Utc};

const SYNODIC_MONTH: f64 = 29.530_588_853;

fn sin_d(x: f64) -> f64 {
    x.to_radians().sin()
}
fn cos_d(x: f64) -> f64 {
    x.to_radians().cos()
}

fn julian_day(utc: DateTime<Utc>) -> f64 {
    utc.timestamp() as f64 / 86_400.0 + 2_440_587.5
}

/// Julian centuries since J2000.0.
fn century(utc: DateTime<Utc>) -> f64 {
    (julian_day(utc) - 2_451_545.0) / 36_525.0
}

/// The sun's declination (degrees), apparent longitude (degrees) and the equation of
/// time (minutes) at the given instant.
fn sun_terms(utc: DateTime<Utc>) -> (f64, f64, f64) {
    let t = century(utc);
    let l0 = (280.466_46 + t * (36_000.769_83 + t * 0.000_303_2)).rem_euclid(360.0);
    let m = 357.529_11 + t * (35_999.050_29 - 0.000_153_7 * t);
    let e = 0.016_708_634 - t * (0.000_042_037 + 0.000_000_126_7 * t);
    let c = sin_d(m) * (1.914_602 - t * (0.004_817 + 0.000_014 * t))
        + sin_d(2.0 * m) * (0.019_993 - 0.000_101 * t)
        + sin_d(3.0 * m) * 0.000_289;
    let omega = 125.04 - 1_934.136 * t;
    let app_long = l0 + c - 0.005_69 - 0.004_78 * sin_d(omega);
    let mean_obliq = 23.0 + (26.0 + (21.448 - t * (46.815 + t * (0.000_59 - t * 0.001_813))) / 60.0) / 60.0;
    let obliq = mean_obliq + 0.002_56 * cos_d(omega);
    let decl = (sin_d(obliq) * sin_d(app_long)).asin().to_degrees();
    let y = (obliq / 2.0).to_radians().tan().powi(2);
    let eq_time = 4.0
        * (y * sin_d(2.0 * l0) - 2.0 * e * sin_d(m) + 4.0 * e * y * sin_d(m) * cos_d(2.0 * l0)
            - 0.5 * y * y * sin_d(4.0 * l0)
            - 1.25 * e * e * sin_d(2.0 * m))
            .to_degrees();
    (decl, app_long.rem_euclid(360.0), eq_time)
}

#[derive(Debug, Clone, Copy)]
pub struct SunPosition {
    /// Degrees above the horizon (negative below), including atmospheric refraction.
    pub elevation: f64,
    /// Degrees clockwise from north.
    pub azimuth: f64,
}

pub fn sun_position(utc: DateTime<Utc>, lat: f64, lon: f64) -> SunPosition {
    let (decl, _, eq_time) = sun_terms(utc);
    let minutes = utc.time().signed_duration_since(NaiveTime::MIN).num_seconds() as f64 / 60.0;
    let true_solar = (minutes + eq_time + 4.0 * lon).rem_euclid(1440.0);
    let hour_angle = true_solar / 4.0 - 180.0;
    let cos_zen = (sin_d(lat) * sin_d(decl) + cos_d(lat) * cos_d(decl) * cos_d(hour_angle)).clamp(-1.0, 1.0);
    let zenith = cos_zen.acos().to_degrees();
    let elevation = 90.0 - zenith;

    let sin_zen = sin_d(zenith);
    let azimuth = if sin_zen.abs() < 1e-9 || cos_d(lat).abs() < 1e-9 {
        180.0
    } else {
        let a = ((sin_d(lat) * cos_zen - sin_d(decl)) / (cos_d(lat) * sin_zen)).clamp(-1.0, 1.0);
        let a = a.acos().to_degrees();
        if hour_angle > 0.0 { (a + 180.0).rem_euclid(360.0) } else { (540.0 - a).rem_euclid(360.0) }
    };
    SunPosition { elevation: elevation + refraction(elevation), azimuth }
}

/// Atmospheric refraction in degrees (NOAA's approximation).
fn refraction(elev: f64) -> f64 {
    let te = elev.to_radians().tan();
    let arcsec = if elev > 85.0 {
        0.0
    } else if elev > 5.0 {
        58.1 / te - 0.07 / te.powi(3) + 0.000_086 / te.powi(5)
    } else if elev > -0.575 {
        1_735.0 + elev * (-518.2 + elev * (103.4 + elev * (-12.79 + elev * 0.711)))
    } else {
        -20.772 / te
    };
    arcsec / 3600.0
}

/// When the sun crosses an elevation going up and going down, in local time. None means it
/// never crosses that day (midnight sun or polar night for that elevation).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crossing {
    pub up: Option<NaiveDateTime>,
    pub down: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, Copy)]
pub struct SunDay {
    pub noon: NaiveDateTime,
    pub noon_elevation: f64,
    /// Sunrise / sunset (upper limb at the horizon with refraction: −0.833°).
    pub sun: Crossing,
    pub civil: Crossing,
    pub nautical: Crossing,
    pub astronomical: Crossing,
    /// The sun at 6°: golden hour runs from here down to −4° (evening) and back (morning).
    pub golden_high: Crossing,
    pub golden_low: Crossing,
}

impl SunDay {
    /// Length of daylight, or None for polar night; 24 h for midnight sun.
    pub fn day_length(&self) -> Option<Duration> {
        match (self.sun.up, self.sun.down) {
            (Some(u), Some(d)) => Some(d - u),
            _ if self.noon_elevation > 0.0 => Some(Duration::hours(24)),
            _ => None,
        }
    }
}

/// Sun events for a local calendar date at a place, given the place's UTC offset.
pub fn sun_day(date: NaiveDate, lat: f64, lon: f64, utc_offset_secs: i32) -> SunDay {
    let offset = Duration::seconds(utc_offset_secs as i64);
    let local_midnight = date.and_time(NaiveTime::MIN);
    // Evaluate declination and equation of time near local noon.
    let noon_guess = (local_midnight + Duration::hours(12) - offset).and_utc();
    let (decl, _, eq_time) = sun_terms(noon_guess);
    // Solar noon in minutes after UTC midnight of `date`, then shifted to local.
    let noon_utc_min = 720.0 - 4.0 * lon - eq_time;
    let to_local = |utc_min: f64| {
        local_midnight + Duration::milliseconds((utc_min * 60_000.0).round() as i64) + offset
    };
    let noon = to_local(noon_utc_min);
    let noon_elevation = 90.0 - (lat - decl).abs();

    let crossing = |elev: f64| {
        let cos_h = (sin_d(elev) - sin_d(lat) * sin_d(decl)) / (cos_d(lat) * cos_d(decl));
        if !(-1.0..=1.0).contains(&cos_h) {
            return Crossing { up: None, down: None };
        }
        let h = cos_h.acos().to_degrees();
        Crossing { up: Some(to_local(noon_utc_min - 4.0 * h)), down: Some(to_local(noon_utc_min + 4.0 * h)) }
    };
    SunDay {
        noon,
        noon_elevation,
        sun: crossing(-0.833),
        civil: crossing(-6.0),
        nautical: crossing(-12.0),
        astronomical: crossing(-18.0),
        golden_high: crossing(6.0),
        golden_low: crossing(-4.0),
    }
}

/// The moon's apparent ecliptic longitude in degrees (main periodic terms only).
fn moon_longitude(utc: DateTime<Utc>) -> f64 {
    let t = century(utc);
    let l = 218.316_4 + 481_267.881_23 * t;
    let d = 297.850_2 + 445_267.111_5 * t;
    let m = 357.529_1 + 35_999.050_3 * t;
    let mp = 134.963_4 + 477_198.867_6 * t;
    let f = 93.272_1 + 483_202.017_5 * t;
    let lon = l + 6.288_774 * sin_d(mp) + 1.274_027 * sin_d(2.0 * d - mp) + 0.658_314 * sin_d(2.0 * d)
        + 0.213_618 * sin_d(2.0 * mp)
        - 0.185_116 * sin_d(m)
        - 0.114_332 * sin_d(2.0 * f)
        + 0.058_793 * sin_d(2.0 * d - 2.0 * mp)
        + 0.057_066 * sin_d(2.0 * d - m - mp)
        + 0.053_322 * sin_d(2.0 * d + mp)
        + 0.045_758 * sin_d(2.0 * d - m)
        - 0.040_923 * sin_d(m - mp)
        - 0.034_720 * sin_d(d)
        - 0.030_383 * sin_d(m + mp);
    lon.rem_euclid(360.0)
}

/// Moon minus sun longitude, 0..360: 0 new, 90 first quarter, 180 full, 270 last quarter.
fn elongation(utc: DateTime<Utc>) -> f64 {
    let (_, sun_long, _) = sun_terms(utc);
    (moon_longitude(utc) - sun_long).rem_euclid(360.0)
}

#[derive(Debug, Clone)]
pub struct MoonPhase {
    /// 0..1 through the cycle (0 new, 0.5 full).
    pub phase: f64,
    /// Lit fraction of the disc, 0..1.
    pub illumination: f64,
    pub name: &'static str,
    pub icon: &'static str,
    pub next_new: DateTime<Utc>,
    pub next_full: DateTime<Utc>,
}

/// Next time after `from` when the elongation reaches `target` degrees.
fn next_elongation(from: DateTime<Utc>, target: f64) -> DateTime<Utc> {
    let behind = |t: DateTime<Utc>| (target - elongation(t)).rem_euclid(360.0);
    // The moon gains ~12.2° a day on the sun; jump close, then refine by bisection.
    let mut t = from + Duration::minutes((behind(from) / 12.19 * 1440.0) as i64 - 12 * 60);
    if t < from {
        t = from;
    }
    // Step until we pass the target (behind() jumps from ~0 to ~360), then bisect.
    let step = Duration::hours(2);
    let mut prev = behind(t);
    for _ in 0..(40 * 12) {
        let next = t + step;
        let b = behind(next);
        if b > prev + 180.0 {
            let (mut lo, mut hi) = (t, next);
            for _ in 0..20 {
                let mid = lo + (hi - lo) / 2;
                if behind(mid) > 180.0 { hi = mid } else { lo = mid }
            }
            return lo + (hi - lo) / 2;
        }
        t = next;
        prev = b;
    }
    t
}

pub fn moon_phase(utc: DateTime<Utc>) -> MoonPhase {
    let e = elongation(utc);
    let phase = e / 360.0;
    let illumination = (1.0 - cos_d(e)) / 2.0;
    // Named phases: the four principal ones get a ±1-day window (about ±12°).
    let (name, icon) = match e {
        x if !(12.0..348.0).contains(&x) => ("New moon", "🌑"),
        x if x < 78.0 => ("Waxing crescent", "🌒"),
        x if x < 102.0 => ("First quarter", "🌓"),
        x if x < 168.0 => ("Waxing gibbous", "🌔"),
        x if x < 192.0 => ("Full moon", "🌕"),
        x if x < 258.0 => ("Waning gibbous", "🌖"),
        x if x < 282.0 => ("Last quarter", "🌗"),
        _ => ("Waning crescent", "🌘"),
    };
    MoonPhase {
        phase,
        illumination,
        name,
        icon,
        next_new: next_elongation(utc, 0.0),
        next_full: next_elongation(utc, 180.0),
    }
}

/// Approximate days in a synodic month, for display.
pub fn synodic_month() -> f64 {
    SYNODIC_MONTH
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn hm(t: Option<NaiveDateTime>) -> String {
        t.map(|t| t.format("%H:%M").to_string()).unwrap_or_default()
    }

    fn close(t: Option<NaiveDateTime>, expect: &str, minutes: i64) -> bool {
        let t = t.expect("event");
        let e = NaiveDateTime::parse_from_str(&format!("{} {expect}", t.date()), "%Y-%m-%d %H:%M").unwrap();
        (t - e).num_seconds().abs() <= minutes * 60 + 59
    }

    #[test]
    fn herndon_matches_open_meteo() {
        // Open-Meteo: 2026-11-03 sunrise 06:39, sunset 17:06, daylight 10:27:28, UTC-5.
        let d = sun_day(NaiveDate::from_ymd_opt(2026, 11, 3).unwrap(), 38.96955, -77.3861, -18_000);
        assert!(close(d.sun.up, "06:39", 1), "sunrise {}", hm(d.sun.up));
        assert!(close(d.sun.down, "17:06", 1), "sunset {}", hm(d.sun.down));
        let len = d.day_length().unwrap().num_seconds();
        assert!((len - 37_648).abs() < 120, "day length {len}");
        // Twilights are ordered outward from the horizon.
        assert!(d.astronomical.up < d.nautical.up && d.nautical.up < d.civil.up && d.civil.up < d.sun.up);
    }

    #[test]
    fn sydney_matches_open_meteo() {
        // Open-Meteo: 2026-11-04 sunrise 05:52, sunset 19:25, UTC+11.
        let d = sun_day(NaiveDate::from_ymd_opt(2026, 11, 4).unwrap(), -33.87, 151.21, 39_600);
        assert!(close(d.sun.up, "05:52", 1), "sunrise {}", hm(d.sun.up));
        assert!(close(d.sun.down, "19:25", 1), "sunset {}", hm(d.sun.down));
    }

    #[test]
    fn polar_night_and_midnight_sun() {
        let winter = sun_day(NaiveDate::from_ymd_opt(2026, 12, 21).unwrap(), 78.22, 15.65, 3_600);
        assert_eq!(winter.sun, Crossing { up: None, down: None });
        assert!(winter.day_length().is_none());
        let summer = sun_day(NaiveDate::from_ymd_opt(2026, 6, 21).unwrap(), 78.22, 15.65, 7_200);
        assert_eq!(summer.day_length(), Some(Duration::hours(24)));
    }

    #[test]
    fn position_peaks_at_solar_noon() {
        let d = sun_day(NaiveDate::from_ymd_opt(2026, 11, 3).unwrap(), 38.96955, -77.3861, -18_000);
        let noon_utc = (d.noon + Duration::hours(5)).and_utc();
        let p = sun_position(noon_utc, 38.96955, -77.3861);
        assert!((p.elevation - d.noon_elevation).abs() < 0.3, "{} vs {}", p.elevation, d.noon_elevation);
        assert!((p.azimuth - 180.0).abs() < 2.0, "azimuth {}", p.azimuth);
        let morning = sun_position(noon_utc - Duration::hours(3), 38.96955, -77.3861);
        assert!(morning.azimuth < 180.0 && morning.elevation < p.elevation);
    }

    #[test]
    fn moon_phases_match_known_events() {
        // New moon of the 2024-04-08 total eclipse, 18:21 UTC.
        let new = Utc.with_ymd_and_hms(2024, 4, 8, 18, 21, 0).unwrap();
        let found = next_elongation(new - Duration::days(3), 0.0);
        assert!((found - new).num_minutes().abs() < 60, "new moon {found}");
        assert_eq!(moon_phase(new).name, "New moon");
        // Full moon of 2024-03-25, 07:00 UTC.
        let full = Utc.with_ymd_and_hms(2024, 3, 25, 7, 0, 0).unwrap();
        let found = next_elongation(full - Duration::days(10), 180.0);
        assert!((found - full).num_minutes().abs() < 60, "full moon {found}");
        let m = moon_phase(full);
        assert_eq!(m.name, "Full moon");
        assert!(m.illumination > 0.99);
    }
}
