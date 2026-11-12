//! Open-Meteo client: geocoding plus today's forecast. No API key needed.

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, Utc};
use std::collections::HashSet;
use serde::{Deserialize, Serialize};

const GEOCODE_URL: &str = "https://geocoding-api.open-meteo.com/v1/search";
const FORECAST_URL: &str = "https://api.open-meteo.com/v1/forecast";
const ALERTS_URL: &str = "https://api.weather.gov/alerts";
/// How far back the alerts popup lists alerts that have ended, so one that came and went
/// while skycast or the computer was asleep is still there to read. A day covers a fog
/// statement at dawn looked for after work.
pub const RECENT_ALERTS: chrono::Duration = chrono::Duration::hours(24);
/// How far back to ask for alert history. The NWS filters on when an alert was sent, and
/// one sent days ago can have ended within [`RECENT_ALERTS`].
const ALERT_HISTORY: chrono::Duration = chrono::Duration::days(3);
const AIR_URL: &str = "https://air-quality-api.open-meteo.com/v1/air-quality";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Units {
    Imperial,
    Metric,
}

impl Units {
    pub fn temp(self) -> &'static str {
        match self {
            Units::Imperial => "°F",
            Units::Metric => "°C",
        }
    }
    pub fn speed(self) -> &'static str {
        match self {
            Units::Imperial => "mph",
            Units::Metric => "km/h",
        }
    }
    /// Pressure from hPa: inHg for imperial, hPa for metric.
    pub fn pressure(self, hpa: f64) -> String {
        match self {
            Units::Imperial => format!("{:.2} inHg", hpa * 0.029_53),
            Units::Metric => format!("{hpa:.0} hPa"),
        }
    }
    pub fn precip(self) -> &'static str {
        match self {
            Units::Imperial => "in",
            Units::Metric => "mm",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Hour {
    pub time: NaiveDateTime,
    pub temp: f64,
    pub precip_prob: u8,
    pub code: u8,
    pub is_day: bool,
    /// US AQI for the hour, if the air quality API answered.
    pub aqi: Option<u16>,
}

#[derive(Debug, Clone)]
pub struct Day {
    pub date: NaiveDate,
    pub code: u8,
    pub high: f64,
    pub low: f64,
    pub precip_prob: u8,
    pub precip_sum: f64,
    pub wind_max: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Unknown,
    Minor,
    Moderate,
    Severe,
    Extreme,
}

#[derive(Debug, Clone)]
pub struct Alert {
    /// The NWS alert ID. An update to an alert is issued under a new ID.
    pub id: String,
    pub event: String,
    pub severity: Severity,
    pub headline: Option<String>,
    pub description: String,
    pub instruction: Option<String>,
    pub area: String,
    /// When it ends (or, failing that, expires), in the issuing office's local time.
    pub until: Option<DateTime<FixedOffset>>,
    /// When the NWS sent it.
    pub sent: Option<DateTime<FixedOffset>>,
}

#[derive(Debug, Clone)]
pub struct Air {
    pub current_aqi: u16,
    pub pm2_5: Option<f64>,
    pub ozone: Option<f64>,
    pub peak_aqi: u16,
    pub peak_time: NaiveDateTime,
}

#[derive(Debug, Clone)]
pub struct Forecast {
    /// Local time at the location, per the API.
    pub now: NaiveDateTime,
    /// The location's offset from UTC, in seconds.
    pub utc_offset_secs: i32,
    pub current_temp: f64,
    pub feels_like: f64,
    pub humidity: u8,
    pub current_code: u8,
    pub current_wind: f64,
    pub current_is_day: bool,
    /// Degrees the wind is coming from.
    pub wind_dir: Option<f64>,
    pub gusts: Option<f64>,
    /// Sea-level pressure in hPa (Open-Meteo has no unit option for it).
    pub pressure_hpa: Option<f64>,
    pub cloud_cover: Option<u8>,

    pub high: f64,
    pub low: f64,
    pub day_code: u8,
    pub precip_prob_max: u8,
    pub precip_sum: f64,
    pub wind_max: f64,
    pub uv_max: f64,
    pub sunrise: NaiveDateTime,
    pub sunset: NaiveDateTime,

    /// Today's hours only.
    pub hours: Vec<Hour>,
    /// Today plus the next six days.
    pub days: Vec<Day>,
    /// None when the air quality API failed; the weather still shows.
    pub air: Option<Air>,
    /// Active NWS alerts, most severe first. None outside the US or when the NWS API failed.
    pub alerts: Option<Vec<Alert>>,
    /// Alerts that ended within [`RECENT_ALERTS`], newest first.
    pub recent_alerts: Vec<Alert>,
}

fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent("skycast/0.1")
        .timeout(std::time::Duration::from_secs(15))
        .build()?)
}

#[derive(Deserialize)]
struct GeoResponse {
    #[serde(default)]
    results: Vec<GeoResult>,
}

#[derive(Deserialize)]
struct GeoResult {
    name: String,
    latitude: f64,
    longitude: f64,
    admin1: Option<String>,
    country: Option<String>,
    country_code: Option<String>,
}

/// A search result: the place, plus a longer label for telling places apart in a list
/// ("Portland, Oregon, United States" vs "Portland, Maine, United States").
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceMatch {
    pub place: Place,
    pub label: String,
}

/// Places matching "City" or "City, Qualifier" (state, state abbreviation or country), best
/// first. Open-Meteo only matches the bare name, so results matching the qualifier are
/// moved to the front. The flag says whether the first result matched the qualifier (true
/// when there was none).
pub fn search(query: &str) -> Result<(Vec<PlaceMatch>, bool)> {
    let mut parts = query.splitn(2, ',').map(str::trim);
    let name = parts.next().unwrap_or_default();
    let qualifier = parts.next().filter(|q| !q.is_empty());
    if name.is_empty() {
        bail!("empty location");
    }

    let resp: GeoResponse = client()?
        .get(GEOCODE_URL)
        .query(&[("name", name), ("count", "20"), ("language", "en"), ("format", "json")])
        .send()?
        .error_for_status()?
        .json()
        .context("bad geocoding response")?;
    let (results, matched) = rank(resp.results, qualifier);
    Ok((results.into_iter().map(to_match).collect(), matched))
}

/// Results matching the qualifier first, otherwise in the API's order (by population).
fn rank(results: Vec<GeoResult>, qualifier: Option<&str>) -> (Vec<GeoResult>, bool) {
    let Some(q) = qualifier else { return (results, true) };
    let q = expand_state(q).to_lowercase();
    let is_match = |r: &GeoResult| {
        [&r.admin1, &r.country, &r.country_code].into_iter().flatten().any(|s| s.to_lowercase() == q)
    };
    let (mut hits, rest): (Vec<_>, Vec<_>) = results.into_iter().partition(is_match);
    let matched = !hits.is_empty();
    hits.extend(rest);
    (hits, matched)
}

fn to_match(r: GeoResult) -> PlaceMatch {
    let mut name = r.name.clone();
    if let Some(region) = r.admin1.as_ref().or(r.country.as_ref()) {
        name = format!("{name}, {region}");
    }
    let mut label = name.clone();
    if let Some(country) = r.country.as_ref().filter(|c| Some(*c) != r.admin1.as_ref()) {
        label = format!("{label}, {country}");
    }
    PlaceMatch { place: Place { name, lat: r.latitude, lon: r.longitude }, label }
}

/// The single best place for a query, for `--location`. With a qualifier ("Herndon, VA"),
/// only a place matching it counts.
pub fn geocode(query: &str) -> Result<Place> {
    let (results, matched) = search(query)?;
    match results.into_iter().next() {
        Some(m) if matched => Ok(m.place),
        _ => Err(anyhow!("location '{query}' not found")),
    }
}

fn expand_state(q: &str) -> &str {
    const STATES: &[(&str, &str)] = &[
        ("AL", "Alabama"), ("AK", "Alaska"), ("AZ", "Arizona"), ("AR", "Arkansas"),
        ("CA", "California"), ("CO", "Colorado"), ("CT", "Connecticut"), ("DE", "Delaware"),
        ("DC", "District of Columbia"), ("FL", "Florida"), ("GA", "Georgia"), ("HI", "Hawaii"),
        ("ID", "Idaho"), ("IL", "Illinois"), ("IN", "Indiana"), ("IA", "Iowa"),
        ("KS", "Kansas"), ("KY", "Kentucky"), ("LA", "Louisiana"), ("ME", "Maine"),
        ("MD", "Maryland"), ("MA", "Massachusetts"), ("MI", "Michigan"), ("MN", "Minnesota"),
        ("MS", "Mississippi"), ("MO", "Missouri"), ("MT", "Montana"), ("NE", "Nebraska"),
        ("NV", "Nevada"), ("NH", "New Hampshire"), ("NJ", "New Jersey"), ("NM", "New Mexico"),
        ("NY", "New York"), ("NC", "North Carolina"), ("ND", "North Dakota"), ("OH", "Ohio"),
        ("OK", "Oklahoma"), ("OR", "Oregon"), ("PA", "Pennsylvania"), ("RI", "Rhode Island"),
        ("SC", "South Carolina"), ("SD", "South Dakota"), ("TN", "Tennessee"), ("TX", "Texas"),
        ("UT", "Utah"), ("VT", "Vermont"), ("VA", "Virginia"), ("WA", "Washington"),
        ("WV", "West Virginia"), ("WI", "Wisconsin"), ("WY", "Wyoming"),
    ];
    STATES
        .iter()
        .find(|(abbr, _)| abbr.eq_ignore_ascii_case(q))
        .map(|(_, full)| *full)
        .unwrap_or(q)
}

#[derive(Deserialize)]
struct RawForecast {
    utc_offset_seconds: i32,
    current: RawCurrent,
    hourly: RawHourly,
    daily: RawDaily,
}

#[derive(Deserialize)]
struct RawCurrent {
    time: String,
    temperature_2m: f64,
    apparent_temperature: f64,
    relative_humidity_2m: f64,
    weather_code: u8,
    wind_speed_10m: f64,
    is_day: u8,
    wind_direction_10m: Option<f64>,
    wind_gusts_10m: Option<f64>,
    pressure_msl: Option<f64>,
    cloud_cover: Option<f64>,
}

#[derive(Deserialize)]
struct RawHourly {
    time: Vec<String>,
    temperature_2m: Vec<Option<f64>>,
    precipitation_probability: Vec<Option<f64>>,
    weather_code: Vec<Option<u8>>,
    is_day: Vec<Option<u8>>,
}

#[derive(Deserialize)]
struct RawDaily {
    time: Vec<String>,
    weather_code: Vec<Option<u8>>,
    temperature_2m_max: Vec<Option<f64>>,
    temperature_2m_min: Vec<Option<f64>>,
    precipitation_probability_max: Vec<Option<f64>>,
    precipitation_sum: Vec<Option<f64>>,
    wind_speed_10m_max: Vec<Option<f64>>,
    uv_index_max: Vec<Option<f64>>,
    sunrise: Vec<String>,
    sunset: Vec<String>,
}

fn parse_time(s: &str) -> Result<NaiveDateTime> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M").with_context(|| format!("bad time '{s}'"))
}

fn first<T: Copy>(v: &[Option<T>]) -> Option<T> {
    v.first().copied().flatten()
}

pub fn fetch_today(place: &Place, units: Units) -> Result<Forecast> {
    let mut q: Vec<(&str, String)> = vec![
        ("latitude", place.lat.to_string()),
        ("longitude", place.lon.to_string()),
        ("timezone", "auto".into()),
        ("forecast_days", "7".into()),
        (
            "current",
            "temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,wind_speed_10m,is_day,\
             wind_direction_10m,wind_gusts_10m,pressure_msl,cloud_cover"
                .into(),
        ),
        ("hourly", "temperature_2m,precipitation_probability,weather_code,is_day".into()),
        (
            "daily",
            "weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max,\
             precipitation_sum,wind_speed_10m_max,uv_index_max,sunrise,sunset"
                .into(),
        ),
    ];
    if units == Units::Imperial {
        q.push(("temperature_unit", "fahrenheit".into()));
        q.push(("wind_speed_unit", "mph".into()));
        q.push(("precipitation_unit", "inch".into()));
    }

    let raw: RawForecast = client()?
        .get(FORECAST_URL)
        .query(&q)
        .send()?
        .error_for_status()?
        .json()
        .context("bad forecast response")?;

    let now = parse_time(&raw.current.time)?;
    let h = &raw.hourly;
    let mut hours: Vec<Hour> = h
        .time
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            Some(Hour {
                time: parse_time(t).ok()?,
                temp: h.temperature_2m.get(i).copied().flatten()?,
                precip_prob: h.precipitation_probability.get(i).copied().flatten().unwrap_or(0.0)
                    as u8,
                code: h.weather_code.get(i).copied().flatten().unwrap_or(0),
                is_day: h.is_day.get(i).copied().flatten().unwrap_or(1) == 1,
                aqi: None,
            })
        })
        .collect();

    hours.retain(|h| h.time.date() == now.date());
    let air = fetch_air(place, &mut hours).ok();
    let alerts = fetch_alerts(place, &[]).ok().flatten();
    // Not worth failing the forecast over: the popup just has no history.
    let recent_alerts = match &alerts {
        Some(active) => fetch_recent_alerts(place, active).unwrap_or_default(),
        None => Vec::new(),
    };

    let d = &raw.daily;
    let days = d
        .time
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            let get = |v: &[Option<f64>]| v.get(i).copied().flatten();
            Some(Day {
                date: NaiveDate::parse_from_str(t, "%Y-%m-%d").ok()?,
                code: d.weather_code.get(i).copied().flatten().unwrap_or(0),
                high: get(&d.temperature_2m_max)?,
                low: get(&d.temperature_2m_min)?,
                precip_prob: get(&d.precipitation_probability_max).unwrap_or(0.0) as u8,
                precip_sum: get(&d.precipitation_sum).unwrap_or(0.0),
                wind_max: get(&d.wind_speed_10m_max).unwrap_or(0.0),
            })
        })
        .collect();
    let c = &raw.current;
    Ok(Forecast {
        now,
        utc_offset_secs: raw.utc_offset_seconds,
        current_temp: c.temperature_2m,
        feels_like: c.apparent_temperature,
        humidity: c.relative_humidity_2m as u8,
        current_code: c.weather_code,
        current_wind: c.wind_speed_10m,
        current_is_day: c.is_day == 1,
        wind_dir: c.wind_direction_10m,
        gusts: c.wind_gusts_10m,
        pressure_hpa: c.pressure_msl,
        cloud_cover: c.cloud_cover.map(|v| v as u8),
        high: first(&d.temperature_2m_max).context("missing high")?,
        low: first(&d.temperature_2m_min).context("missing low")?,
        day_code: first(&d.weather_code).unwrap_or(0),
        precip_prob_max: first(&d.precipitation_probability_max).unwrap_or(0.0) as u8,
        precip_sum: first(&d.precipitation_sum).unwrap_or(0.0),
        wind_max: first(&d.wind_speed_10m_max).unwrap_or(0.0),
        uv_max: first(&d.uv_index_max).unwrap_or(0.0),
        sunrise: parse_time(d.sunrise.first().context("missing sunrise")?)?,
        sunset: parse_time(d.sunset.first().context("missing sunset")?)?,
        hours,
        days,
        air,
        alerts,
        recent_alerts,
    })
}

#[derive(Deserialize)]
struct RawAir {
    current: RawAirCurrent,
    hourly: RawAirHourly,
}

#[derive(Deserialize)]
struct RawAirCurrent {
    us_aqi: Option<f64>,
    pm2_5: Option<f64>,
    ozone: Option<f64>,
}

#[derive(Deserialize)]
struct RawAirHourly {
    time: Vec<String>,
    us_aqi: Vec<Option<f64>>,
}

/// Fetch US AQI, filling in each hour's value and returning today's summary.
fn fetch_air(place: &Place, hours: &mut [Hour]) -> Result<Air> {
    let raw: RawAir = client()?
        .get(AIR_URL)
        .query(&[
            ("latitude", place.lat.to_string()),
            ("longitude", place.lon.to_string()),
            ("timezone", "auto".into()),
            ("forecast_days", "1".into()),
            ("current", "us_aqi,pm2_5,ozone".into()),
            ("hourly", "us_aqi".into()),
        ])
        .send()?
        .error_for_status()?
        .json()
        .context("bad air quality response")?;

    let hourly: Vec<(NaiveDateTime, u16)> = raw
        .hourly
        .time
        .iter()
        .zip(&raw.hourly.us_aqi)
        .filter_map(|(t, v)| Some((parse_time(t).ok()?, (*v)? as u16)))
        .collect();
    for h in hours.iter_mut() {
        h.aqi = hourly.iter().find(|(t, _)| *t == h.time).map(|(_, v)| *v);
    }
    let (peak_time, peak_aqi) =
        hourly
            .iter()
            .copied()
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0))) // earliest hour wins ties
            .context("no hourly AQI")?;
    Ok(Air {
        current_aqi: raw.current.us_aqi.context("no current AQI")? as u16,
        pm2_5: raw.current.pm2_5,
        ozone: raw.current.ozone,
        peak_aqi,
        peak_time,
    })
}

/// US EPA AQI category name.
pub fn aqi_category(aqi: u16) -> &'static str {
    match aqi {
        0..=50 => "Good",
        51..=100 => "Moderate",
        101..=150 => "Unhealthy for sensitive groups",
        151..=200 => "Unhealthy",
        201..=300 => "Very unhealthy",
        _ => "Hazardous",
    }
}

#[derive(Deserialize)]
struct RawAlerts {
    features: Vec<RawAlertFeature>,
}

#[derive(Deserialize)]
struct RawAlertFeature {
    #[serde(default)]
    id: String,
    properties: RawAlert,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAlert {
    /// "Alert", "Update" or "Cancel".
    message_type: Option<String>,
    sent: Option<String>,
    /// The earlier messages this one updates or cancels.
    #[serde(default)]
    references: Vec<RawAlertRef>,
    event: String,
    severity: Option<String>,
    headline: Option<String>,
    description: Option<String>,
    instruction: Option<String>,
    area_desc: Option<String>,
    ends: Option<String>,
    expires: Option<String>,
}

#[derive(Deserialize)]
struct RawAlertRef {
    identifier: String,
}

/// National Weather Service alerts for the point: the active ones with no `query`, or
/// those matching it. Ok(None) means the point is outside NWS coverage (the API answers
/// 400 "out of bounds").
fn fetch_alerts(place: &Place, query: &[(&str, String)]) -> Result<Option<Vec<Alert>>> {
    let url = if query.is_empty() { format!("{ALERTS_URL}/active") } else { ALERTS_URL.to_string() };
    let resp = client()?
        .get(url)
        .header("Accept", "application/geo+json")
        .query(&[("point", format!("{:.4},{:.4}", place.lat, place.lon))])
        .query(query)
        .send()?;
    if resp.status() == reqwest::StatusCode::BAD_REQUEST {
        return Ok(None);
    }
    let raw: RawAlerts = resp.error_for_status()?.json().context("bad alerts response")?;
    if !query.is_empty() {
        return Ok(Some(ended(raw.features, Utc::now())));
    }
    let mut alerts: Vec<Alert> = raw.features.into_iter().map(alert).collect();
    alerts.sort_by_key(|a| std::cmp::Reverse(a.severity));
    Ok(Some(alerts))
}

/// Alerts that ended within [`RECENT_ALERTS`], leaving out the active ones.
fn fetch_recent_alerts(place: &Place, active: &[Alert]) -> Result<Vec<Alert>> {
    let start = (Utc::now() - ALERT_HISTORY).format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let recent = fetch_alerts(place, &[("start", start), ("status", "actual".into())])?.unwrap_or_default();
    Ok(recent.into_iter().filter(|a| !active.iter().any(|b| b.id == a.id)).collect())
}

/// From an alert history, the alerts that ended within [`RECENT_ALERTS`] of `now`, newest first. A message
/// that a later one updates or cancels is left out (the later one stands for it), and so
/// are the cancellations themselves.
fn ended(history: Vec<RawAlertFeature>, now: DateTime<Utc>) -> Vec<Alert> {
    let replaced: HashSet<String> =
        history.iter().flat_map(|f| f.properties.references.iter().map(|r| r.identifier.clone())).collect();
    let mut alerts: Vec<Alert> = history
        .into_iter()
        .filter(|f| !replaced.contains(&f.id) && f.properties.message_type.as_deref() != Some("Cancel"))
        .map(alert)
        .filter(|a| a.until.is_some_and(|t| t <= now && t > now - RECENT_ALERTS))
        .collect();
    alerts.sort_by_key(|a| std::cmp::Reverse(a.sent));
    alerts
}

fn alert(f: RawAlertFeature) -> Alert {
    let p = f.properties;
    let time = |t: Option<String>| t.and_then(|t| DateTime::parse_from_rfc3339(&t).ok());
    Alert {
        id: f.id,
        severity: match p.severity.as_deref() {
            Some("Extreme") => Severity::Extreme,
            Some("Severe") => Severity::Severe,
            Some("Moderate") => Severity::Moderate,
            Some("Minor") => Severity::Minor,
            _ => Severity::Unknown,
        },
        event: p.event,
        headline: p.headline,
        description: p.description.unwrap_or_default(),
        instruction: p.instruction,
        area: p.area_desc.unwrap_or_default(),
        until: time(p.ends.or(p.expires)),
        sent: time(p.sent),
    }
}

/// 8-point compass name for a "wind from" bearing in degrees.
pub fn compass(deg: f64) -> &'static str {
    const NAMES: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    NAMES[((deg.rem_euclid(360.0) + 22.5) / 45.0) as usize % 8]
}

#[cfg(test)]
mod tests {
    use super::{GeoResult, RawAlerts, compass, ended, rank};
    use chrono::{DateTime, Utc};

    fn geo(name: &str, admin1: &str, country: &str, code: &str) -> GeoResult {
        GeoResult {
            name: name.into(),
            latitude: 0.0,
            longitude: 0.0,
            admin1: Some(admin1.into()),
            country: Some(country.into()),
            country_code: Some(code.into()),
        }
    }

    #[test]
    fn ended_alerts_skip_replaced_and_cancelled() {
        // a1 expired; b1 was updated by b2 (expired); c1 was cancelled; e1 ended over a day
        // ago; d1 runs till later.
        let raw: RawAlerts = serde_json::from_str(
            r#"{"features": [
            {"id": "a1", "properties": {"messageType": "Alert", "event": "Dense Fog Advisory",
                "sent": "2026-11-12T05:00:00-05:00", "expires": "2026-11-12T08:30:00-05:00"}},
            {"id": "b1", "properties": {"messageType": "Alert", "event": "Wind Advisory",
                "sent": "2026-11-12T01:00:00-05:00", "ends": "2026-11-12T06:00:00-05:00"}},
            {"id": "b2", "properties": {"messageType": "Update", "event": "Wind Advisory",
                "sent": "2026-11-12T04:00:00-05:00", "ends": "2026-11-12T07:00:00-05:00",
                "references": [{"identifier": "b1"}]}},
            {"id": "c1", "properties": {"messageType": "Alert", "event": "Frost Advisory",
                "sent": "2026-11-12T02:00:00-05:00", "ends": "2026-11-12T05:00:00-05:00"}},
            {"id": "c2", "properties": {"messageType": "Cancel", "event": "Frost Advisory",
                "sent": "2026-11-12T03:00:00-05:00", "expires": "2026-11-12T04:00:00-05:00",
                "references": [{"identifier": "c1"}]}},
            {"id": "e1", "properties": {"messageType": "Alert", "event": "Heat Advisory",
                "sent": "2026-11-10T09:00:00-05:00", "ends": "2026-11-11T09:00:00-05:00"}},
            {"id": "d1", "properties": {"messageType": "Alert", "event": "Flood Watch",
                "sent": "2026-11-12T06:00:00-05:00", "ends": "2026-11-13T06:00:00-05:00"}}
        ]}"#,
        )
        .unwrap();
        let now = DateTime::parse_from_rfc3339("2026-11-12T12:00:00-05:00").unwrap().with_timezone(&Utc);
        let ids: Vec<String> = ended(raw.features, now).into_iter().map(|a| a.id).collect();
        assert_eq!(ids, ["a1", "b2"]);
    }

    #[test]
    fn qualifier_moves_matches_first() {
        let results = || {
            vec![
                geo("Portland", "Oregon", "United States", "US"),
                geo("Portland", "Maine", "United States", "US"),
                geo("Portland", "Victoria", "Australia", "AU"),
            ]
        };
        let (r, matched) = rank(results(), Some("ME"));
        assert!(matched);
        assert_eq!(r[0].admin1.as_deref(), Some("Maine"));
        assert_eq!(r.len(), 3);
        let (r, matched) = rank(results(), Some("Australia"));
        assert!(matched && r[0].admin1.as_deref() == Some("Victoria"));
        let (r, matched) = rank(results(), Some("Narnia"));
        assert!(!matched && r[0].admin1.as_deref() == Some("Oregon"));
        assert!(rank(results(), None).1);
    }

    #[test]
    fn labels_tell_places_apart() {
        let m = super::to_match(geo("Portland", "Maine", "United States", "US"));
        assert_eq!(m.place.name, "Portland, Maine");
        assert_eq!(m.label, "Portland, Maine, United States");
    }

    #[test]
    fn compass_points() {
        assert_eq!(compass(0.0), "N");
        assert_eq!(compass(22.4), "N");
        assert_eq!(compass(22.6), "NE");
        assert_eq!(compass(135.0), "SE");
        assert_eq!(compass(350.0), "N");
        assert_eq!(compass(-90.0), "W");
    }
}
