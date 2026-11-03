//! Sun screen: today's sun events, the moon, the sun's path, and daylight through the year.

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Timelike, Utc};
use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Style, Stylize},
    symbols::Marker,
    text::{Line, Span},
    widgets::{Axis, Chart, Dataset, GraphType, Paragraph},
};

use super::{DAY_TICKS, blank_x_labels, draw_x_labels, panel};
use crate::api::{Forecast, Place, compass};
use crate::solar::{self, Crossing, SunDay};

const SUN: Color = Color::Yellow;
const NIGHT: Color = Color::Indexed(61); // muted blue

fn dim(s: impl Into<String>) -> Span<'static> {
    Span::styled(s.into(), Style::new().dark_gray())
}

fn clock(t: Option<NaiveDateTime>) -> String {
    t.map_or("—".into(), |t| t.format("%-I:%M %p").to_string())
}

fn short_clock(t: Option<NaiveDateTime>) -> String {
    t.map_or("—".into(), |t| t.format("%-I:%M").to_string())
}

fn hm(d: Duration) -> String {
    format!("{}h {:02}m", d.num_hours(), d.num_minutes() % 60)
}

/// "+2m 04s" / "−1m 58s".
fn signed_ms(d: Duration) -> String {
    let s = d.num_seconds();
    let sign = if s < 0 { "−" } else { "+" };
    format!("{sign}{}m {:02}s", s.abs() / 60, s.abs() % 60)
}

fn pair(c: Crossing) -> String {
    format!("{} / {}", clock(c.up), clock(c.down))
}

/// The place's local time right now.
fn local_now(fc: &Forecast) -> NaiveDateTime {
    (Utc::now() + Duration::seconds(fc.utc_offset_secs as i64)).naive_utc()
}

fn to_utc(fc: &Forecast, local: NaiveDateTime) -> chrono::DateTime<Utc> {
    (local - Duration::seconds(fc.utc_offset_secs as i64)).and_utc()
}

pub fn draw(f: &mut Frame, area: Rect, fc: &Forecast, place: &Place) {
    let now = local_now(fc);
    let today = now.date();
    let day = solar::sun_day(today, place.lat, place.lon, fc.utc_offset_secs);
    let yesterday = solar::sun_day(today - Duration::days(1), place.lat, place.lon, fc.utc_offset_secs);

    if area.height < 10 {
        draw_compact(f, area, &day, &yesterday);
        return;
    }

    let charts = area.height.saturating_sub(10);
    let (path_h, year_h) = match charts {
        h if h >= 22 => (h.min(34) * 3 / 5, h.min(34) * 2 / 5),
        h if h >= 8 => (h, 0),
        _ => (0, 0),
    };
    let [top, path, year] =
        Layout::vertical([Constraint::Length(10), Constraint::Length(path_h), Constraint::Length(year_h)])
            .flex(Flex::Start)
            .areas(area);
    let [sun, moon] = Layout::horizontal([Constraint::Fill(3), Constraint::Fill(2)]).areas(top);
    draw_sun_panel(f, sun, fc, place, &day, &yesterday, now);
    draw_moon_panel(f, moon, fc);
    if path_h > 0 {
        draw_path(f, path, fc, place, &day, now);
    }
    if year_h > 0 {
        draw_year(f, year, fc, place, today);
    }
}

fn sun_state(elev: f64) -> (&'static str, Color) {
    match elev {
        e if e >= 6.0 => ("daylight", SUN),
        e if e >= -0.833 => ("golden hour", Color::Indexed(214)),
        e if e >= -6.0 => ("civil twilight", Color::Indexed(173)),
        e if e >= -12.0 => ("nautical twilight", Color::Indexed(67)),
        e if e >= -18.0 => ("astronomical twilight", NIGHT),
        _ => ("night", NIGHT),
    }
}

fn draw_sun_panel(
    f: &mut Frame,
    area: Rect,
    fc: &Forecast,
    place: &Place,
    day: &SunDay,
    yesterday: &SunDay,
    now: NaiveDateTime,
) {
    let pos = solar::sun_position(to_utc(fc, now), place.lat, place.lon);
    let (state, color) = sun_state(pos.elevation);
    let where_ = if pos.elevation >= -0.833 {
        format!("{:.0}° up in the {} ({:.0}°)", pos.elevation, compass(pos.azimuth), pos.azimuth)
    } else {
        format!("{:.0}° below the horizon", -pos.elevation)
    };
    let length = match (day.day_length(), yesterday.day_length()) {
        (Some(d), Some(y)) => Line::from(vec![
            dim("Day length   "),
            Span::raw(hm(d)).bold(),
            dim(format!("  {} vs yesterday", signed_ms(d - y))),
        ]),
        (Some(d), None) => Line::from(vec![dim("Day length   "), Span::raw(hm(d)).bold()]),
        (None, _) => Line::from(vec![dim("Day length   "), Span::raw("polar night").bold()]),
    };
    let golden = format!(
        "{}–{}  ·  {}–{}",
        short_clock(day.golden_low.up),
        clock(day.golden_high.up),
        short_clock(day.golden_high.down),
        clock(day.golden_low.down)
    );
    let lines = vec![
        Line::from(vec![
            Span::styled("☀ ", Style::new().fg(color)),
            Span::styled(state, Style::new().fg(color).bold()),
            dim(format!("  {where_}")),
        ]),
        Line::from(vec![
            dim("Sunrise      "),
            Span::styled(clock(day.sun.up), Style::new().fg(SUN)),
            dim("   Sunset "),
            Span::styled(clock(day.sun.down), Style::new().light_red()),
        ]),
        length,
        Line::from(vec![
            dim("Solar noon   "),
            Span::raw(clock(Some(day.noon))),
            dim(format!(", {:.0}° high", day.noon_elevation)),
        ]),
        Line::from(vec![dim("Golden hour  "), Span::styled(golden, Style::new().fg(Color::Indexed(214)))]),
        Line::from(vec![dim("Civil        "), Span::raw(pair(day.civil))]),
        Line::from(vec![dim("Nautical     "), Span::raw(pair(day.nautical))]),
        Line::from(vec![dim("Astronomical "), Span::raw(pair(day.astronomical))]),
    ];
    f.render_widget(Paragraph::new(lines).block(panel("Sun")), area);
}

fn draw_moon_panel(f: &mut Frame, area: Rect, fc: &Forecast) {
    let m = solar::moon_phase(Utc::now());
    let local = |t: chrono::DateTime<Utc>| {
        (t + Duration::seconds(fc.utc_offset_secs as i64)).naive_utc().format("%a %b %-d, %-I:%M %p").to_string()
    };
    let mut next = [("Full moon", m.next_full), ("New moon ", m.next_new)];
    next.sort_by_key(|(_, t)| *t);
    let lines = vec![
        Line::from(vec![Span::raw(format!("{} ", m.icon)), Span::raw(m.name).bold()]),
        Line::from(vec![
            Span::raw(format!("{:.0}%", m.illumination * 100.0)),
            dim(" illuminated"),
        ]),
        Line::from(dim(format!("Day {:.0} of {:.1}", m.phase * solar::synodic_month(), solar::synodic_month()))),
        Line::raw(""),
        Line::from(vec![dim(format!("{}  ", next[0].0)), Span::raw(local(next[0].1))]),
        Line::from(vec![dim(format!("{}  ", next[1].0)), Span::raw(local(next[1].1))]),
    ];
    f.render_widget(Paragraph::new(lines).block(panel("Moon")), area);
}

/// Contiguous runs of points on the same side of the horizon, so each can get its color.
fn split_by_horizon(points: &[(f64, f64)]) -> Vec<(bool, Vec<(f64, f64)>)> {
    let mut runs: Vec<(bool, Vec<(f64, f64)>)> = Vec::new();
    for &p in points {
        let up = p.1 >= 0.0;
        match runs.last_mut() {
            Some((u, run)) if *u == up => run.push(p),
            Some((_, run)) => {
                // Share the crossing point so the two colors meet without a gap.
                let last = *run.last().unwrap();
                runs.push((up, vec![last, p]));
            }
            None => runs.push((up, vec![p])),
        }
    }
    runs
}

fn draw_path(f: &mut Frame, area: Rect, fc: &Forecast, place: &Place, day: &SunDay, now: NaiveDateTime) {
    let midnight = now.date().and_hms_opt(0, 0, 0).unwrap();
    let points: Vec<(f64, f64)> = (0..=144)
        .map(|k| {
            let t = midnight + Duration::minutes(10 * k);
            (k as f64 / 6.0, solar::sun_position(to_utc(fc, t), place.lat, place.lon).elevation)
        })
        .collect();
    let lo = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min).min(-20.0).floor();
    let hi = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max).max(10.0).ceil() + 5.0;
    let runs = split_by_horizon(&points);
    let horizon = [(0.0, 0.0), (24.0, 0.0)];
    let now_x = now.hour() as f64 + now.minute() as f64 / 60.0;
    let now_elev = solar::sun_position(to_utc(fc, now), place.lat, place.lon).elevation;
    let now_pt = [(now_x, now_elev)];

    let mut datasets = vec![
        Dataset::default()
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::new().fg(Color::Indexed(240)))
            .data(&horizon),
    ];
    for (up, run) in &runs {
        datasets.push(
            Dataset::default()
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::new().fg(if *up { SUN } else { NIGHT }))
                .data(run),
        );
    }
    datasets.push(
        Dataset::default()
            .marker(Marker::Block)
            .graph_type(GraphType::Scatter)
            .style(Style::new().fg(if now_elev >= 0.0 { SUN } else { NIGHT }))
            .data(&now_pt),
    );

    let title = Line::from(vec![
        Span::styled(" Sun path today ", Style::new().bold()),
        dim(format!("noon {:.0}° · ", day.noon_elevation)),
        Span::styled("█", Style::new().fg(SUN)),
        dim(" now "),
    ]);
    let y_labels: Vec<Line> = [lo, 0.0, hi].into_iter().map(|v| Line::from(format!("{v:.0}°"))).collect();
    let block = panel("").title(title);
    let inner = block.inner(area);
    let chart = Chart::new(datasets)
        .block(block)
        .x_axis(Axis::default().bounds([0.0, 24.0]).labels(blank_x_labels()).style(Style::new().dark_gray()))
        .y_axis(Axis::default().bounds([lo, hi]).labels(y_labels.clone()).style(Style::new().dark_gray()));
    f.render_widget(chart, area);
    draw_x_labels(f, inner, &y_labels, [0.0, 24.0], &DAY_TICKS);
}

fn draw_year(f: &mut Frame, area: Rect, fc: &Forecast, place: &Place, today: NaiveDate) {
    let year = today.year();
    let start = NaiveDate::from_ymd_opt(year, 1, 1).unwrap();
    let days = if NaiveDate::from_ymd_opt(year, 12, 31).unwrap().ordinal() == 366 { 366 } else { 365 };
    let hours = |d: NaiveDate| {
        let sd = solar::sun_day(d, place.lat, place.lon, fc.utc_offset_secs);
        sd.day_length().map_or(0.0, |l| l.num_seconds() as f64 / 3600.0)
    };
    let points: Vec<(f64, f64)> = (0..days).map(|i| (i as f64, hours(start + Duration::days(i)))).collect();
    let (min_i, min_h) = points.iter().copied().min_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
    let (max_i, max_h) = points.iter().copied().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
    let lo = (min_h - 1.0).max(0.0).floor();
    let hi = (max_h + 1.0).min(24.0).ceil();
    let t = (today.ordinal0() as f64, hours(today));
    let today_pt = [t];
    let today_line = [(t.0, lo), (t.0, hi)];

    let datasets = vec![
        Dataset::default()
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::new().dark_gray())
            .data(&today_line),
        Dataset::default()
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::new().fg(SUN))
            .data(&points),
        Dataset::default().marker(Marker::Block).graph_type(GraphType::Scatter).style(Style::new().white()).data(&today_pt),
    ];
    let date = |i: f64| (start + Duration::days(i as i64)).format("%b %-d").to_string();
    let title = Line::from(vec![
        Span::styled(" Daylight through the year ", Style::new().bold()),
        dim(format!(
            "shortest {} {} · longest {} {} ",
            date(min_i),
            hm(Duration::seconds((min_h * 3600.0) as i64)),
            date(max_i),
            hm(Duration::seconds((max_h * 3600.0) as i64)),
        )),
    ]);
    let y_labels: Vec<Line> = [lo, (lo + hi) / 2.0, hi].into_iter().map(|v| Line::from(format!("{v:.0}h"))).collect();
    let bounds = [0.0, (days - 1) as f64];
    let block = panel("").title(title);
    let inner = block.inner(area);
    let chart = Chart::new(datasets)
        .block(block)
        .x_axis(Axis::default().bounds(bounds).labels(blank_x_labels()).style(Style::new().dark_gray()))
        .y_axis(Axis::default().bounds([lo, hi]).labels(y_labels.clone()).style(Style::new().dark_gray()));
    f.render_widget(chart, area);
    // A label at the start of each month; ones that would collide are skipped.
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let ticks: Vec<(f64, &str)> = (1..=12)
        .map(|m| (NaiveDate::from_ymd_opt(year, m, 1).unwrap().ordinal0() as f64, MONTHS[m as usize - 1]))
        .collect();
    draw_x_labels(f, inner, &y_labels, bounds, &ticks);
}

fn draw_compact(f: &mut Frame, area: Rect, day: &SunDay, yesterday: &SunDay) {
    let m = solar::moon_phase(Utc::now());
    let mut lines = vec![
        Line::from(vec![
            Span::styled(" ☀ ", Style::new().fg(SUN)),
            Span::raw(format!("{} → {}", clock(day.sun.up), clock(day.sun.down))),
            dim(match (day.day_length(), yesterday.day_length()) {
                (Some(d), Some(y)) => format!("  {} ({})", hm(d), signed_ms(d - y)),
                _ => String::new(),
            }),
        ]),
        Line::from(vec![dim(" Solar noon "), Span::raw(clock(Some(day.noon))), dim(format!(", {:.0}°", day.noon_elevation))]),
        Line::from(vec![Span::raw(format!(" {} {}", m.icon, m.name)), dim(format!(" · {:.0}% lit", m.illumination * 100.0))]),
    ];
    lines.truncate(area.height as usize);
    f.render_widget(Paragraph::new(lines), area);
}
