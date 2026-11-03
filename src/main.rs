mod api;
mod basemap;
mod precip;
mod radar;
mod solar;
mod tides;
mod ui;
mod wmo;

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;

use api::{Forecast, Place, Units};

const DEFAULT_LOCATION: &str = "Herndon, VA";
const AUTO_REFRESH: Duration = Duration::from_secs(15 * 60);
/// After a failed fetch, try again this soon instead of waiting for the next refresh.
const ERROR_RETRY: Duration = Duration::from_secs(60);
/// Pauses before quick retries of a temporary failure (5xx, timeout, no connection).
const TRANSIENT_RETRIES: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(5)];

/// Worth retrying soon: the server hiccuped or the network did, rather than a bad request.
fn transient(e: &anyhow::Error) -> bool {
    e.chain().any(|c| {
        c.downcast_ref::<reqwest::Error>().is_some_and(|r| {
            r.is_timeout() || r.is_connect() || r.status().is_some_and(|s| s.is_server_error())
        })
    })
}

const USAGE: &str = "\
weather - today's forecast in the terminal

Usage: weather [OPTIONS]

Options:
  -l, --location <NAME>   City, optionally with state or country (default: \"Herndon, VA\")
      --lat <LAT> --lon <LON>
                          Use coordinates instead of a place name
  -m, --metric            Celsius, km/h, mm (default: Fahrenheit, mph, inches)
  -h, --help              Show this help

Keys: Tab / ←→ switch screens (1 Today, 2 7 days, 3 Radar, 4 Sun, 5 Tides), a alerts,
      r refresh, q / Esc quit
Radar: + / - zoom, space play/pause the last two hours, , / . step one scan";

enum Location {
    Name(String),
    Coords(f64, f64),
}

struct Args {
    location: Location,
    units: Units,
}

fn parse_args() -> Result<Args> {
    let mut name = None;
    let (mut lat, mut lon) = (None, None);
    let mut units = Units::Imperial;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut val = |flag: &str| it.next().with_context(|| format!("{flag} needs a value"));
        match arg.as_str() {
            "-l" | "--location" => name = Some(val("--location")?),
            "--lat" => lat = Some(val("--lat")?.parse::<f64>().context("bad --lat")?),
            "--lon" => lon = Some(val("--lon")?.parse::<f64>().context("bad --lon")?),
            "-m" | "--metric" => units = Units::Metric,
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => bail!("unknown argument '{other}'\n\n{USAGE}"),
        }
    }
    let location = match (lat, lon, name) {
        (Some(a), Some(o), _) => {
            if !(-90.0..=90.0).contains(&a) {
                bail!("--lat must be between -90 and 90");
            }
            if !(-180.0..=180.0).contains(&o) {
                bail!("--lon must be between -180 and 180");
            }
            Location::Coords(a, o)
        }
        (None, None, n) => Location::Name(n.unwrap_or_else(|| DEFAULT_LOCATION.into())),
        _ => bail!("--lat and --lon must be given together"),
    };
    Ok(Args { location, units })
}

pub enum Status {
    Loading,
    Ready(Box<Forecast>),
    Error(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Today,
    Week,
    Radar,
    Sun,
    Tides,
}

impl Screen {
    const ALL: [Screen; 5] = [Screen::Today, Screen::Week, Screen::Radar, Screen::Sun, Screen::Tides];

    fn step(self, by: isize) -> Self {
        let i = Self::ALL.iter().position(|s| *s == self).unwrap_or(0) as isize;
        Self::ALL[(i + by).rem_euclid(Self::ALL.len() as isize) as usize]
    }
}

/// How often to re-check for new radar scans (RainViewer adds one every 10 minutes).
const RADAR_REFRESH: Duration = Duration::from_secs(5 * 60);
const RADAR_STEP: Duration = Duration::from_millis(400);
/// Pause on the latest scan before the animation loops.
const RADAR_HOLD: Duration = Duration::from_millis(1500);

pub struct RadarState {
    pub frames: Option<radar::Frames>,
    frames_fetched: Option<Instant>,
    frames_pending: bool,
    pub error: Option<String>,
    /// Index into frames; the last one is the latest scan.
    pub frame: usize,
    pub playing: bool,
    last_step: Instant,
    pub zoom: usize,
    /// None marks a tile that failed to load.
    pub tiles: HashMap<radar::TileKey, Option<radar::Tile>>,
    pending: HashSet<radar::TileKey>,
    /// The map's drawing area, recorded by the UI so the main loop knows which tiles to fetch.
    pub area: Cell<Rect>,
}

impl RadarState {
    fn new() -> Self {
        RadarState {
            frames: None,
            frames_fetched: None,
            frames_pending: false,
            error: None,
            frame: 0,
            playing: false,
            last_step: Instant::now(),
            zoom: radar::DEFAULT_ZOOM,
            tiles: HashMap::new(),
            pending: HashSet::new(),
            area: Cell::new(Rect::ZERO),
        }
    }

    pub fn view(&self, place: &Place) -> Option<radar::View> {
        let a = self.area.get();
        if a.width == 0 || a.height == 0 {
            return None;
        }
        Some(radar::View::new(
            place.lon,
            place.lat,
            radar::ZOOM_KM[self.zoom],
            a.width as usize * 2,
            a.height as usize * 4,
        ))
    }

    pub fn current(&self) -> Option<&radar::RadarFrame> {
        self.frames.as_ref().and_then(|f| f.frames.get(self.frame))
    }

    pub fn loading(&self) -> bool {
        self.frames_pending || !self.pending.is_empty()
    }
}

/// Tide predictions, fetched when the Tides tab is first opened and again each new day.
#[derive(Default)]
pub struct TideState {
    /// Some(None): looked, and there's no tide data near this place.
    pub data: Option<Option<tides::Tides>>,
    pub error: Option<String>,
    pub pending: bool,
    fetched_for: Option<chrono::NaiveDate>,
}

enum RadarReq {
    Frames,
    Tile(String, radar::TileKey),
    Tides(Place, Units, chrono::NaiveDate),
}

enum RadarResp {
    Frames(Result<radar::Frames, String>),
    Tile(radar::TileKey, Result<radar::Tile, String>),
    Tides(chrono::NaiveDate, Result<Option<tides::Tides>, String>),
}

/// A few threads so a screen's worth of tiles loads in parallel.
fn spawn_radar_workers(n: usize) -> (mpsc::Sender<RadarReq>, mpsc::Receiver<RadarResp>) {
    let (req_tx, req_rx) = mpsc::channel::<RadarReq>();
    let (res_tx, res_rx) = mpsc::channel::<RadarResp>();
    let req_rx = Arc::new(Mutex::new(req_rx));
    for _ in 0..n {
        let (req_rx, res_tx) = (req_rx.clone(), res_tx.clone());
        thread::spawn(move || {
            loop {
                let Ok(req) = req_rx.lock().unwrap().recv() else { break };
                let resp = match req {
                    RadarReq::Frames => RadarResp::Frames(radar::fetch_frames().map_err(|e| format!("{e:#}"))),
                    RadarReq::Tile(host, key) => {
                        let r = radar::fetch_tile(&host, &key).map_err(|e| format!("{e:#}"));
                        RadarResp::Tile(key, r)
                    }
                    RadarReq::Tides(place, units, day) => {
                        RadarResp::Tides(day, tides::fetch(&place, units, day).map_err(|e| format!("{e:#}")))
                    }
                };
                if res_tx.send(resp).is_err() {
                    break;
                }
            }
        });
    }
    (req_tx, res_rx)
}

/// Ask for whatever the radar screen is missing and advance the animation.
fn radar_tick(app: &mut App, tx: &mpsc::Sender<RadarReq>) -> Result<()> {
    let r = &mut app.radar;
    let stale = r.frames_fetched.is_none_or(|t| t.elapsed() >= RADAR_REFRESH);
    if stale && !r.frames_pending {
        r.frames_pending = true;
        tx.send(RadarReq::Frames)?;
    }
    let (Some(frames), Some(place)) = (&r.frames, &app.place) else { return Ok(()) };
    let Some(view) = r.view(place) else { return Ok(()) };

    // While playing, load every frame; otherwise just the one on screen.
    let wanted: Vec<usize> = if r.playing { (0..frames.frames.len()).collect() } else { vec![r.frame] };
    for i in wanted {
        let Some(fr) = frames.frames.get(i) else { continue };
        for (x, y) in view.tiles() {
            let key = radar::TileKey { path: fr.path.clone(), z: view.z, x, y };
            if !r.tiles.contains_key(&key) && r.pending.insert(key.clone()) {
                tx.send(RadarReq::Tile(frames.host.clone(), key))?;
            }
        }
    }

    if r.playing {
        let last = frames.frames.len().saturating_sub(1);
        let wait = if r.frame == last { RADAR_HOLD } else { RADAR_STEP };
        let next = if r.frame >= last { 0 } else { r.frame + 1 };
        // Don't step onto a scan that's still loading; it would flash blank.
        let next_ready = frames.frames.get(next).is_some_and(|fr| {
            view.tiles().iter().all(|&(x, y)| {
                r.tiles.contains_key(&radar::TileKey { path: fr.path.clone(), z: view.z, x, y })
            })
        });
        if r.last_step.elapsed() >= wait && next_ready {
            r.frame = next;
            r.last_step = Instant::now();
        }
    }
    Ok(())
}

/// Ask for tide predictions if the Tides tab needs them.
fn tides_tick(app: &mut App, tx: &mpsc::Sender<RadarReq>) -> Result<()> {
    let (Some(place), Some(fc)) = (&app.place, app.last_good.as_deref()) else { return Ok(()) };
    let today = fc.now.date();
    let t = &mut app.tides;
    if !t.pending && t.fetched_for != Some(today) {
        t.pending = true;
        tx.send(RadarReq::Tides(place.clone(), app.units, today))?;
    }
    Ok(())
}

fn radar_receive(app: &mut App, rx: &mpsc::Receiver<RadarResp>) {
    let r = &mut app.radar;
    while let Ok(resp) = rx.try_recv() {
        match resp {
            RadarResp::Tides(day, res) => {
                let t = &mut app.tides;
                t.pending = false;
                t.fetched_for = Some(day);
                match res {
                    Ok(data) => {
                        t.data = Some(data);
                        t.error = None;
                    }
                    Err(e) => t.error = Some(e),
                }
            }
            RadarResp::Frames(Ok(frames)) => {
                let following_latest =
                    r.frames.as_ref().is_none_or(|old| r.frame + 1 >= old.frames.len());
                // Drop tiles for scans that have aged out.
                let live: HashSet<&str> = frames.frames.iter().map(|f| f.path.as_str()).collect();
                r.tiles.retain(|k, _| live.contains(k.path.as_str()));
                if following_latest || r.frame >= frames.frames.len() {
                    r.frame = frames.frames.len() - 1;
                }
                r.frames = Some(frames);
                r.frames_fetched = Some(Instant::now());
                r.frames_pending = false;
                r.error = None;
            }
            RadarResp::Frames(Err(e)) => {
                r.frames_pending = false;
                r.frames_fetched = Some(Instant::now());
                r.error = Some(e);
            }
            RadarResp::Tile(key, res) => {
                r.pending.remove(&key);
                if let Err(e) = &res {
                    r.error = Some(e.clone());
                }
                r.tiles.insert(key, res.ok());
            }
        }
    }
}

pub struct App {
    pub screen: Screen,
    pub radar: RadarState,
    pub tides: TideState,
    /// Alert details popup.
    pub show_alerts: bool,
    pub alert_scroll: u16,
    pub place: Option<Place>,
    pub units: Units,
    pub status: Status,
    /// Kept so a failed refresh still shows the last good forecast.
    pub last_good: Option<Box<Forecast>>,
    pub updated: Option<DateTime<Local>>,
    pub fetching: bool,
}

type FetchResult = Result<(Place, Forecast), String>;

/// Worker thread: resolves the place once, then fetches whenever asked.
fn spawn_worker(location: Location, units: Units) -> (mpsc::Sender<()>, mpsc::Receiver<FetchResult>) {
    let (req_tx, req_rx) = mpsc::channel::<()>();
    let (res_tx, res_rx) = mpsc::channel::<FetchResult>();
    thread::spawn(move || {
        let mut place: Option<Place> = None;
        while req_rx.recv().is_ok() {
            let result = (|| -> Result<(Place, Forecast)> {
                let p = match &place {
                    Some(p) => p.clone(),
                    None => {
                        let p = match &location {
                            Location::Name(n) => api::geocode(n)?,
                            Location::Coords(lat, lon) => Place {
                                name: format!("{lat:.3}, {lon:.3}"),
                                lat: *lat,
                                lon: *lon,
                            },
                        };
                        place = Some(p.clone());
                        p
                    }
                };
                let mut result = api::fetch_today(&p, units);
                for pause in TRANSIENT_RETRIES {
                    match &result {
                        Err(e) if transient(e) => {
                            thread::sleep(pause);
                            result = api::fetch_today(&p, units);
                        }
                        _ => break,
                    }
                }
                Ok((p, result?))
            })();
            if res_tx.send(result.map_err(|e| format!("{e:#}"))).is_err() {
                break;
            }
        }
    });
    (req_tx, res_rx)
}

fn main() -> Result<()> {
    let args = parse_args()?;
    let (req_tx, res_rx) = spawn_worker(args.location, args.units);
    let (radar_tx, radar_rx) = spawn_radar_workers(6);

    let mut app = App {
        screen: Screen::Today,
        radar: RadarState::new(),
        tides: TideState::default(),
        show_alerts: false,
        alert_scroll: 0,
        place: None,
        units: args.units,
        status: Status::Loading,
        last_good: None,
        updated: None,
        fetching: true,
    };
    req_tx.send(())?;
    let mut last_request = Instant::now();

    let mut terminal = ratatui::init();
    let result = (|| -> Result<()> {
        loop {
            while let Ok(res) = res_rx.try_recv() {
                app.fetching = false;
                match res {
                    Ok((place, forecast)) => {
                        app.place = Some(place);
                        let f = Box::new(forecast);
                        app.last_good = Some(f.clone());
                        app.status = Status::Ready(f);
                        app.updated = Some(Local::now());
                    }
                    Err(e) => app.status = Status::Error(e),
                }
            }

            radar_receive(&mut app, &radar_rx);

            terminal.draw(|f| ui::draw(f, &app))?;

            if app.screen == Screen::Radar {
                radar_tick(&mut app, &radar_tx)?;
            }
            if app.screen == Screen::Tides {
                tides_tick(&mut app, &radar_tx)?;
            }

            let tick = if app.screen == Screen::Radar && (app.radar.playing || app.radar.loading()) {
                Duration::from_millis(100)
            } else {
                Duration::from_millis(250)
            };
            if event::poll(tick)?
                && let Event::Key(k) = event::read()? {
                    if k.kind != KeyEventKind::Press {
                        continue;
                    }
                    if app.show_alerts {
                        match k.code {
                            KeyCode::Char('a') | KeyCode::Esc => app.show_alerts = false,
                            KeyCode::Char('q') => break,
                            KeyCode::Down | KeyCode::Char('j') => app.alert_scroll += 1,
                            KeyCode::Up | KeyCode::Char('k') => {
                                app.alert_scroll = app.alert_scroll.saturating_sub(1)
                            }
                            KeyCode::PageDown | KeyCode::Char(' ') => app.alert_scroll += 10,
                            KeyCode::PageUp => app.alert_scroll = app.alert_scroll.saturating_sub(10),
                            _ => {}
                        }
                        continue;
                    }
                    match k.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Char('a') => {
                            app.show_alerts = true;
                            app.alert_scroll = 0;
                        }
                        KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => break,
                        KeyCode::Tab | KeyCode::Right => app.screen = app.screen.step(1),
                        KeyCode::BackTab | KeyCode::Left => app.screen = app.screen.step(-1),
                        KeyCode::Char('1') | KeyCode::Char('t') => app.screen = Screen::Today,
                        KeyCode::Char('2') | KeyCode::Char('w') => app.screen = Screen::Week,
                        KeyCode::Char('3') => app.screen = Screen::Radar,
                        KeyCode::Char('4') => app.screen = Screen::Sun,
                        KeyCode::Char('5') => app.screen = Screen::Tides,
                        KeyCode::Char('r') if !app.fetching => {
                            app.fetching = true;
                            req_tx.send(())?;
                            last_request = Instant::now();
                            app.radar.frames_fetched = None; // re-check radar on the next tick
                        }
                        // Radar controls.
                        KeyCode::Char('+') | KeyCode::Char('=') if app.screen == Screen::Radar => {
                            app.radar.zoom = (app.radar.zoom + 1).min(radar::ZOOM_KM.len() - 1);
                        }
                        KeyCode::Char('-') | KeyCode::Char('_') if app.screen == Screen::Radar => {
                            app.radar.zoom = app.radar.zoom.saturating_sub(1);
                        }
                        KeyCode::Char(' ') | KeyCode::Char('p') if app.screen == Screen::Radar => {
                            app.radar.playing = !app.radar.playing;
                            app.radar.last_step = Instant::now();
                        }
                        KeyCode::Char(',') | KeyCode::Char('.') if app.screen == Screen::Radar => {
                            let n = app.radar.frames.as_ref().map_or(0, |f| f.frames.len());
                            if n > 0 {
                                app.radar.playing = false;
                                let by = if k.code == KeyCode::Char('.') { 1 } else { n - 1 };
                                app.radar.frame = (app.radar.frame + by) % n;
                            }
                        }
                        _ => {}
                    }
                }

            let interval = if matches!(app.status, Status::Error(_)) { ERROR_RETRY } else { AUTO_REFRESH };
            if !app.fetching && last_request.elapsed() >= interval {
                app.fetching = true;
                req_tx.send(())?;
                last_request = Instant::now();
            }
        }
        Ok(())
    })();
    ratatui::restore();
    result
}
