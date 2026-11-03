//! Tides screen: current level, next high and low, a two-day curve, and a high/low table.

use chrono::{Duration, NaiveDateTime, Timelike, Utc};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Flex, Layout, Rect},
    style::{Color, Style},
    symbols::Marker,
    text::{Line, Span},
    widgets::{Axis, Chart, Dataset, GraphType, Paragraph, Wrap},
};

use super::{blank_x_labels, draw_x_labels, panel};
use crate::TideState;
use crate::api::Forecast;
use crate::tides::{TideEvent, Tides};

const WATER: Color = Color::Indexed(39); // bright blue
const HIGH: Color = Color::Indexed(45);
const LOW: Color = Color::Indexed(67);

fn dim(s: impl Into<String>) -> Span<'static> {
    Span::styled(s.into(), Style::new().dark_gray())
}

fn local_now(fc: &Forecast) -> NaiveDateTime {
    (Utc::now() + Duration::seconds(fc.utc_offset_secs as i64)).naive_utc()
}

fn until(d: Duration) -> String {
    let m = d.num_minutes().max(0);
    if m >= 60 { format!("in {}h {:02}m", m / 60, m % 60) } else { format!("in {m}m") }
}

fn short(t: NaiveDateTime) -> String {
    let h = t.hour();
    let suffix = if h < 12 { "a" } else { "p" };
    let h12 = match h % 12 {
        0 => 12,
        x => x,
    };
    format!("{h12}:{:02}{suffix}", t.minute())
}

pub fn draw(f: &mut Frame, area: Rect, fc: &Forecast, state: &TideState, place_name: &str) {
    let message = |f: &mut Frame, text: String, style: Style| {
        let p = Paragraph::new(text).style(style).alignment(Alignment::Center).wrap(Wrap { trim: true });
        f.render_widget(p.block(panel("Tides")), area);
    };
    match (&state.data, &state.error) {
        (Some(Some(t)), _) => draw_tides(f, area, fc, t),
        (Some(None), _) => message(
            f,
            format!(
                "No tide data near {place_name}.\n\nThere's no NOAA tide station within 80 km, and no sea in \
                 Open-Meteo's marine model within 50 km."
            ),
            Style::new().dark_gray(),
        ),
        (None, Some(e)) => message(f, format!("Couldn't load tides: {e}"), Style::new().red()),
        (None, None) => message(f, "Loading tide predictions…".into(), Style::new().dark_gray()),
    }
}

fn draw_tides(f: &mut Frame, area: Rect, fc: &Forecast, t: &Tides) {
    let now = local_now(fc);
    let show_chart = area.height >= 6 + 4 + 8;
    let [top, chart, table] = Layout::vertical([
        Constraint::Length(6),
        if show_chart { Constraint::Max(22) } else { Constraint::Length(0) },
        Constraint::Length(4),
    ])
    .flex(Flex::Start)
    .areas(area);

    draw_summary(f, top, t, now);
    if show_chart {
        draw_chart(f, chart, t, now);
    }
    draw_table(f, table, t, now);
}

fn draw_summary(f: &mut Frame, area: Rect, t: &Tides, now: NaiveDateTime) {
    let level = t.height_at(now);
    let trend = match (level, t.height_at(now + Duration::minutes(10))) {
        (Some(a), Some(b)) if b > a + 0.001 => ("rising ↑", WATER),
        (Some(a), Some(b)) if b < a - 0.001 => ("falling ↓", LOW),
        (Some(_), Some(_)) => ("slack", Color::Gray),
        _ => ("", Color::Gray),
    };
    let mut lines = vec![match level {
        Some(h) => Line::from(vec![
            dim("Now   "),
            Span::styled(format!("{h:.1} {}", t.unit), Style::new().fg(WATER).bold()),
            Span::raw("  "),
            Span::styled(trend.0, Style::new().fg(trend.1)),
        ]),
        None => Line::from(dim("No prediction for right now")),
    }];
    let next_of = |high: bool| t.next_events(now).find(|e| e.high == high);
    let mut nexts: Vec<&TideEvent> = [next_of(true), next_of(false)].into_iter().flatten().collect();
    nexts.sort_by_key(|e| e.time);
    for e in nexts {
        let (word, color) = if e.high { ("High  ", HIGH) } else { ("Low   ", LOW) };
        lines.push(Line::from(vec![
            dim(word),
            Span::styled(format!("{:.1} {}", e.height, t.unit), Style::new().fg(color).bold()),
            Span::raw(format!(" at {}", e.time.format("%-I:%M %p"))),
            dim(format!("  {}", until(e.time - now))),
        ]));
    }
    let source = if t.source == "NOAA" {
        format!("NOAA station {} · {:.0} km away · above {}", t.station, t.distance_km, t.datum)
    } else {
        format!("{} · {} · {:.0} km away · relative to {}", t.source, t.station, t.distance_km, t.datum)
    };
    lines.push(Line::from(dim(source)));
    f.render_widget(Paragraph::new(lines).block(panel("Tides")), area);
}

fn draw_chart(f: &mut Frame, area: Rect, t: &Tides, now: NaiveDateTime) {
    // Today and tomorrow, as hours since today's midnight.
    let start = now.date().and_hms_opt(0, 0, 0).unwrap();
    let end = start + Duration::days(2);
    let x = |time: NaiveDateTime| (time - start).num_minutes() as f64 / 60.0;
    let curve: Vec<(f64, f64)> =
        t.curve.iter().filter(|(ct, _)| *ct >= start && *ct <= end).map(|&(ct, h)| (x(ct), h)).collect();
    if curve.is_empty() {
        return;
    }
    let in_range = |e: &&TideEvent| e.time >= start && e.time <= end;
    let highs: Vec<(f64, f64)> = t.events.iter().filter(in_range).filter(|e| e.high).map(|e| (x(e.time), e.height)).collect();
    let lows: Vec<(f64, f64)> = t.events.iter().filter(in_range).filter(|e| !e.high).map(|e| (x(e.time), e.height)).collect();

    let lo = curve.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let hi = curve.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    let pad = ((hi - lo) * 0.1).max(0.1);
    let (lo, hi) = (lo - pad, hi + pad);
    let now_x = x(now);
    let now_line: Vec<(f64, f64)> = (0..=20).map(|i| (now_x, lo + (hi - lo) * i as f64 / 20.0)).collect();
    let midnight = [(24.0, lo), (24.0, hi)];

    let datasets = vec![
        Dataset::default().marker(Marker::Braille).graph_type(GraphType::Line).style(Style::new().fg(Color::Indexed(238))).data(&midnight),
        Dataset::default().marker(Marker::Braille).graph_type(GraphType::Line).style(Style::new().dark_gray()).data(&now_line),
        Dataset::default().marker(Marker::Braille).graph_type(GraphType::Line).style(Style::new().fg(WATER)).data(&curve),
        Dataset::default().marker(Marker::Dot).graph_type(GraphType::Scatter).style(Style::new().fg(HIGH).bold()).data(&highs),
        Dataset::default().marker(Marker::Dot).graph_type(GraphType::Scatter).style(Style::new().fg(LOW).bold()).data(&lows),
    ];
    let title = Line::from(vec![
        Span::styled(" Today ", Style::new().bold()),
        dim("│ "),
        Span::styled("Tomorrow ", Style::new().bold()),
    ]);
    let y_labels: Vec<Line> =
        [lo, (lo + hi) / 2.0, hi].into_iter().map(|v| Line::from(format!("{v:.1} {}", t.unit))).collect();
    let block = panel("").title(title);
    let inner = block.inner(area);
    let chart = Chart::new(datasets)
        .block(block)
        .x_axis(Axis::default().bounds([0.0, 48.0]).labels(blank_x_labels()).style(Style::new().dark_gray()))
        .y_axis(Axis::default().bounds([lo, hi]).labels(y_labels.clone()).style(Style::new().dark_gray()));
    f.render_widget(chart, area);
    const TICKS: [(f64, &str); 9] = [
        (0.0, "12a"), (6.0, "6a"), (12.0, "12p"), (18.0, "6p"), (24.0, "12a"),
        (30.0, "6a"), (36.0, "12p"), (42.0, "6p"), (48.0, "12a"),
    ];
    draw_x_labels(f, inner, &y_labels, [0.0, 48.0], &TICKS);
}

fn draw_table(f: &mut Frame, area: Rect, t: &Tides, now: NaiveDateTime) {
    let row = |label: &'static str, date: chrono::NaiveDate| {
        let mut spans = vec![Span::styled(format!("{label:<9}"), Style::new().bold())];
        for e in t.events.iter().filter(|e| e.time.date() == date) {
            let past = e.time < now;
            let (tag, color) = if e.high { ("H ", HIGH) } else { ("L ", LOW) };
            let style = if past { Style::new().dark_gray() } else { Style::new().fg(color) };
            spans.push(Span::styled(tag, style.bold()));
            spans.push(Span::styled(format!("{} {:.1}  ", short(e.time), e.height), style));
        }
        Line::from(spans)
    };
    let today = now.date();
    let lines = vec![row("Today", today), row("Tomorrow", today + Duration::days(1))];
    f.render_widget(Paragraph::new(lines).block(panel(&format!("High and low tides ({})", t.unit))), area);
}
