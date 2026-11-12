mod api;
mod basemap;
mod config;
mod herdr;
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
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;

use api::{Forecast, Place, Units};

/// Wait this long after the last keystroke before searching for places.
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(300);
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
skycast - weather, radar, sun, moon and tides in the terminal

Usage: skycast [OPTIONS]

Options:
  -l, --location <NAME>   City, optionally with state or country, for this run only
      --lat <LAT> --lon <LON>
                          Use coordinates instead of a place name, for this run only
  -m, --metric            Celsius, km/h, mm (default: Fahrenheit, mph, inches)
  -h, --help              Show this help

With neither, skycast uses your saved location, or asks for one the first time.
Press l to search for a different place; the one you choose is saved.

Keys: Tab / ←→ switch screens (1 Today, 2 7 days, 3 Radar, 4 Sun, 5 Tides), a alerts,
      l change location, r refresh, ? data sources, q / Esc quit
Radar: + / - zoom, space play/pause the last two hours, , / . step one scan";

enum Location {
    Name(String),
    Coords(f64, f64),
}

struct Args {
    location: Option<Location>,
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
            Some(Location::Coords(a, o))
        }
        (None, None, n) => n.map(Location::Name),
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
    Search(u64, String),
}

enum RadarResp {
    Frames(Result<radar::Frames, String>),
    Tile(radar::TileKey, Result<radar::Tile, String>),
    Tides(chrono::NaiveDate, Result<Option<tides::Tides>, String>),
    Search(u64, Result<Vec<api::PlaceMatch>, String>),
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
                    RadarReq::Search(seq, query) => {
                        RadarResp::Search(seq, api::search(&query).map(|(r, _)| r).map_err(|e| format!("{e:#}")))
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
            RadarResp::Search(seq, res) => {
                let s = &mut app.search;
                if seq != s.seq {
                    continue; // an older query's answer
                }
                s.searching = false;
                s.selected = 0;
                match res {
                    Ok(results) => {
                        s.message = results.is_empty().then(|| "No places match".to_string());
                        s.results = results;
                    }
                    Err(e) => {
                        s.results.clear();
                        s.message = Some(format!("Search failed: {e}"));
                    }
                }
            }
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

/// The place search popup.
#[derive(Default)]
pub struct SearchState {
    pub open: bool,
    pub query: String,
    pub results: Vec<api::PlaceMatch>,
    pub selected: usize,
    pub searching: bool,
    /// Shown above the results: "No places match", an error, or why the popup opened.
    pub message: Option<String>,
    /// When the query last changed; a search goes out once it has been still for a moment.
    changed_at: Option<Instant>,
    /// Numbers each search so a slow, outdated answer can't replace a newer one.
    seq: u64,
}

impl SearchState {
    fn open_with(&mut self, query: &str, message: Option<String>) {
        *self = SearchState { open: true, query: query.into(), message, seq: self.seq, ..Default::default() };
        if !query.is_empty() {
            self.changed_at = Some(Instant::now() - SEARCH_DEBOUNCE);
        }
    }

    fn edited(&mut self) {
        self.changed_at = Some(Instant::now());
        self.message = None;
    }
}

pub struct App {
    pub screen: Screen,
    pub search: SearchState,
    pub radar: RadarState,
    pub tides: TideState,
    /// Alert details popup.
    pub show_alerts: bool,
    /// The data sources popup, and how far it's scrolled.
    pub show_sources: bool,
    pub sources_scroll: u16,
    pub alert_scroll: u16,
    pub place: Option<Place>,
    pub units: Units,
    pub status: Status,
    /// Kept so a failed refresh still shows the last good forecast.
    pub last_good: Option<Box<Forecast>>,
    pub updated: Option<DateTime<Local>>,
    pub fetching: bool,
    /// A one-off message for the footer.
    pub notice: Option<String>,
}

type FetchResult = (Place, Result<Forecast, String>);

/// A moment, timed on both clocks. On macOS `Instant` stops while the computer sleeps, so
/// after waking it would be up to a full refresh interval late; the wall clock isn't.
#[derive(Clone, Copy)]
struct Since {
    mono: Instant,
    wall: SystemTime,
}

impl Since {
    fn now() -> Self {
        Self { mono: Instant::now(), wall: SystemTime::now() }
    }

    fn elapsed(&self) -> Duration {
        // A wall clock set backwards errs, and then the monotonic one still counts.
        self.mono.elapsed().max(self.wall.elapsed().unwrap_or_default())
    }
}

/// Worker thread: fetches the forecast for whichever place it's sent.
fn spawn_worker(units: Units) -> (mpsc::Sender<Place>, mpsc::Receiver<FetchResult>) {
    let (req_tx, req_rx) = mpsc::channel::<Place>();
    let (res_tx, res_rx) = mpsc::channel::<FetchResult>();
    thread::spawn(move || {
        while let Ok(place) = req_rx.recv() {
            let mut result = api::fetch_today(&place, units);
            for pause in TRANSIENT_RETRIES {
                match &result {
                    Err(e) if transient(e) => {
                        thread::sleep(pause);
                        result = api::fetch_today(&place, units);
                    }
                    _ => break,
                }
            }
            if res_tx.send((place, result.map_err(|e| format!("{e:#}")))).is_err() {
                break;
            }
        }
    });
    (req_tx, res_rx)
}

/// Where to start: the command line, then the saved location, then ask. A name that finds
/// nothing opens the search with that name filled in.
fn starting_place(location: Option<Location>, search: &mut SearchState) -> Option<Place> {
    match location {
        Some(Location::Coords(lat, lon)) => Some(Place { name: format!("{lat:.3}, {lon:.3}"), lat, lon }),
        Some(Location::Name(n)) => match api::geocode(&n) {
            Ok(p) => Some(p),
            Err(e) => {
                search.open_with(&n, Some(format!("{e:#}")));
                None
            }
        },
        None => {
            let saved = config::load().location;
            if saved.is_none() {
                search.open_with("", Some("Where are you? Type a city to search.".into()));
            }
            saved
        }
    }
}

/// Switch to a place chosen in the search: remember it, clear the old place's data and fetch.
fn choose_place(app: &mut App, place: Place, tx: &mpsc::Sender<Place>) -> Result<()> {
    let config = config::Config { location: Some(place.clone()) };
    // Not fatal: the place still works for this run, and the footer says it wasn't saved.
    app.notice = config::save(&config).err().map(|e| format!("location not saved: {e:#}"));
    app.place = Some(place.clone());
    app.search.open = false;
    app.status = Status::Loading;
    app.last_good = None;
    app.updated = None;
    app.tides = TideState::default();
    app.show_alerts = false;
    app.fetching = true;
    tx.send(place)?;
    Ok(())
}

fn main() -> Result<()> {
    let args = parse_args()?;
    let (req_tx, res_rx) = spawn_worker(args.units);
    let (radar_tx, radar_rx) = spawn_radar_workers(6);

    let mut search = SearchState::default();
    let place = starting_place(args.location, &mut search);
    let mut app = App {
        screen: Screen::Today,
        search,
        radar: RadarState::new(),
        tides: TideState::default(),
        show_alerts: false,
        show_sources: false,
        sources_scroll: 0,
        alert_scroll: 0,
        place: place.clone(),
        units: args.units,
        status: Status::Loading,
        last_good: None,
        updated: None,
        fetching: place.is_some(),
        notice: None,
    };
    if let Some(p) = place {
        req_tx.send(p)?;
    }
    let mut last_request = Since::now();
    let mut notifier = herdr::Notifier::from_env();

    let mut terminal = ratatui::init();
    let result = (|| -> Result<()> {
        loop {
            while let Ok((place, res)) = res_rx.try_recv() {
                // An answer for a place we've since moved away from.
                if app.place.as_ref() != Some(&place) {
                    continue;
                }
                app.fetching = false;
                match res {
                    Ok(forecast) => {
                        notifier.alerts(forecast.alerts.as_deref().unwrap_or(&[]));
                        let f = Box::new(forecast);
                        app.last_good = Some(f.clone());
                        app.status = Status::Ready(f);
                        app.updated = Some(Local::now());
                    }
                    Err(e) => app.status = Status::Error(e),
                }
            }

            radar_receive(&mut app, &radar_rx);

            // Search once the query has been still for a moment.
            if let Some(t) = app.search.changed_at
                && t.elapsed() >= SEARCH_DEBOUNCE
            {
                let s = &mut app.search;
                s.changed_at = None;
                s.seq += 1;
                if s.query.trim().chars().count() >= 2 {
                    s.searching = true;
                    radar_tx.send(RadarReq::Search(s.seq, s.query.trim().to_string()))?;
                } else {
                    s.searching = false;
                    s.results.clear();
                }
            }

            terminal.draw(|f| ui::draw(f, &app))?;

            if app.screen == Screen::Radar {
                radar_tick(&mut app, &radar_tx)?;
            }
            if app.screen == Screen::Tides {
                tides_tick(&mut app, &radar_tx)?;
            }

            let busy = app.search.changed_at.is_some() || app.search.searching;
            let tick = if busy || (app.screen == Screen::Radar && (app.radar.playing || app.radar.loading())) {
                Duration::from_millis(100)
            } else {
                Duration::from_millis(250)
            };
            if event::poll(tick)?
                && let Event::Key(k) = event::read()? {
                    if k.kind != KeyEventKind::Press {
                        continue;
                    }
                    if app.search.open {
                        let s = &mut app.search;
                        match k.code {
                            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => break,
                            KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                                s.query.clear();
                                s.edited();
                            }
                            KeyCode::Esc => {
                                // Nothing to go back to on first run: Esc quits.
                                if app.place.is_none() {
                                    break;
                                }
                                s.open = false;
                            }
                            KeyCode::Enter => {
                                if let Some(m) = s.results.get(s.selected).cloned() {
                                    choose_place(&mut app, m.place, &req_tx)?;
                                    last_request = Since::now();
                                }
                            }
                            KeyCode::Down => s.selected = (s.selected + 1).min(s.results.len().saturating_sub(1)),
                            KeyCode::Up => s.selected = s.selected.saturating_sub(1),
                            KeyCode::Backspace => {
                                s.query.pop();
                                s.edited();
                            }
                            KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) && s.query.chars().count() < 80 => {
                                s.query.push(c);
                                s.edited();
                            }
                            _ => {}
                        }
                        continue;
                    }
                    if app.show_sources {
                        match k.code {
                            KeyCode::Char('?') | KeyCode::Esc => app.show_sources = false,
                            KeyCode::Char('q') => break,
                            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => break,
                            KeyCode::Down | KeyCode::Char('j') => app.sources_scroll += 1,
                            KeyCode::Up | KeyCode::Char('k') => app.sources_scroll = app.sources_scroll.saturating_sub(1),
                            _ => {}
                        }
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
                        KeyCode::Char('l') | KeyCode::Char('/') => app.search.open_with("", None),
                        KeyCode::Char('?') => {
                            app.show_sources = true;
                            app.sources_scroll = 0;
                        }
                        KeyCode::Tab | KeyCode::Right => app.screen = app.screen.step(1),
                        KeyCode::BackTab | KeyCode::Left => app.screen = app.screen.step(-1),
                        KeyCode::Char('1') | KeyCode::Char('t') => app.screen = Screen::Today,
                        KeyCode::Char('2') | KeyCode::Char('w') => app.screen = Screen::Week,
                        KeyCode::Char('3') => app.screen = Screen::Radar,
                        KeyCode::Char('4') => app.screen = Screen::Sun,
                        KeyCode::Char('5') => app.screen = Screen::Tides,
                        KeyCode::Char('r') if !app.fetching && app.place.is_some() => {
                            app.fetching = true;
                            req_tx.send(app.place.clone().unwrap())?;
                            last_request = Since::now();
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
            if !app.fetching
                && last_request.elapsed() >= interval
                && let Some(p) = app.place.clone()
            {
                app.fetching = true;
                req_tx.send(p)?;
                last_request = Since::now();
            }
        }
        Ok(())
    })();
    ratatui::restore();
    result
}
