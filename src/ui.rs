//! Two screens: Today (summary, now, charts, hourly strip) and 7 days (daily rows and a chart).

use std::collections::HashMap;

use chrono::{Datelike, NaiveDateTime, Timelike};
use ratatui::{
    Frame,
    layout::{Alignment, Flex, Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    symbols::Marker,
    text::{Line, Span},
    widgets::canvas::{Canvas, Line as CanvasLine, Points},
    widgets::{Axis, Bar, BarChart, BarGroup, Block, Clear, Padding, BorderType, Chart, Dataset, GraphType, Paragraph, Wrap},
};

use crate::api::{Alert, Forecast, Severity, Units, aqi_category, compass};
use crate::wmo;
use crate::{App, Screen, Status};

mod compact;

const HOUR_COL_WIDTH: u16 = 6;
/// Below this many rows the Today screen switches to its compact layout.
const FULL_TODAY_MIN_HEIGHT: u16 = 16;
/// At or below this many rows the whole window becomes a strip with no header or footer.
const STRIP_MAX_HEIGHT: u16 = 3;

fn temp_color(t: f64, units: Units) -> Color {
    let f = match units {
        Units::Imperial => t,
        Units::Metric => t * 9.0 / 5.0 + 32.0,
    };
    match f {
        f if f < 32.0 => Color::LightBlue,
        f if f < 50.0 => Color::Cyan,
        f if f < 68.0 => Color::Green,
        f if f < 85.0 => Color::Yellow,
        _ => Color::LightRed,
    }
}

/// EPA AQI colors, approximated in the 256-color palette.
fn aqi_color(aqi: u16) -> Color {
    match aqi {
        0..=50 => Color::Green,
        51..=100 => Color::Yellow,
        101..=150 => Color::Indexed(208), // orange
        151..=200 => Color::Red,
        201..=300 => Color::Magenta,
        _ => Color::Indexed(88), // maroon
    }
}

fn aqi_span(aqi: u16) -> Span<'static> {
    Span::styled(format!("{aqi} {}", aqi_category(aqi)), Style::new().fg(aqi_color(aqi)).bold())
}

fn deg(t: f64) -> String {
    format!("{:.0}°", t)
}

fn clock(t: NaiveDateTime) -> String {
    t.format("%-I:%M %p").to_string()
}

fn short_hour(t: NaiveDateTime) -> String {
    match t.hour() {
        0 => "12a".into(),
        12 => "12p".into(),
        h if h < 12 => format!("{h}a"),
        h => format!("{}p", h - 12),
    }
}

fn panel(title: &str) -> Block<'_> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().dark_gray())
        .padding(Padding::horizontal(1))
        .title(Span::styled(format!(" {title} "), Style::new().bold()))
}

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    if area.height <= STRIP_MAX_HEIGHT {
        draw_strip(f, area, app);
        return;
    }
    let [header, body, footer] =
        Layout::vertical([Constraint::Length(1), Constraint::Fill(1), Constraint::Length(1)])
            .areas(area);

    draw_header(f, header, app);
    draw_footer(f, footer, app);

    let forecast = match &app.status {
        Status::Ready(fc) => Some(fc.as_ref()),
        _ => app.last_good.as_deref(),
    };
    // Active alerts get a banner across the top of every screen.
    let alerts: &[Alert] = forecast.and_then(|fc| fc.alerts.as_deref()).unwrap_or(&[]);
    let body = if alerts.is_empty() {
        body
    } else {
        let shown = alerts.len().min(3) as u16;
        let [banner, rest] =
            Layout::vertical([Constraint::Length(shown), Constraint::Fill(1)]).areas(body);
        draw_alert_banner(f, banner, alerts);
        rest
    };

    match (forecast, &app.status) {
        (Some(fc), _) => match app.screen {
            Screen::Today if body.height < FULL_TODAY_MIN_HEIGHT => compact::draw_today(f, body, fc, app.units),
            Screen::Today => draw_today(f, body, fc, app.units),
            Screen::Week => draw_week(f, body, fc, app.units),
            Screen::Radar => draw_radar(f, body, app),
        },
        (None, Status::Error(e)) => {
            let p = Paragraph::new(format!("Couldn't load the forecast:\n\n{e}\n\nPress r to retry."))
                .red()
                .wrap(Wrap { trim: true })
                .block(panel("Error"));
            f.render_widget(p, body);
        }
        _ => {
            let p = Paragraph::new("Fetching forecast…").alignment(Alignment::Center).block(panel("Today"));
            f.render_widget(p, body);
        }
    }

    if app.show_alerts {
        draw_alert_popup(f, area, forecast, app.alert_scroll);
    }
}

fn draw_strip(f: &mut Frame, area: Rect, app: &App) {
    let forecast = match &app.status {
        Status::Ready(fc) => Some(fc.as_ref()),
        _ => app.last_good.as_deref(),
    };
    match (forecast, &app.status) {
        (Some(fc), _) => {
            let place = app.place.as_ref().map_or("", |p| p.name.split(',').next().unwrap_or(&p.name));
            compact::draw_strip(f, area, fc, app.units, place);
        }
        (None, Status::Error(e)) => f.render_widget(Line::styled(format!(" weather: {e}"), Style::new().red()), area),
        _ => f.render_widget(Line::styled(" Fetching forecast…", Style::new().dark_gray()), area),
    }
}

fn severity_color(s: Severity) -> Color {
    match s {
        Severity::Extreme => Color::Magenta,
        Severity::Severe => Color::Red,
        Severity::Moderate => Color::Indexed(208), // orange
        Severity::Minor | Severity::Unknown => Color::Yellow,
    }
}

fn alert_until(a: &Alert) -> String {
    a.until.map(|t| format!(" until {}", t.format("%a %-I:%M %p"))).unwrap_or_default()
}

fn draw_alert_banner(f: &mut Frame, area: Rect, alerts: &[Alert]) {
    let shown = area.height as usize;
    for (i, a) in alerts.iter().take(shown).enumerate() {
        let row = Rect { y: area.y + i as u16, height: 1, ..area };
        let style = Style::new().fg(Color::Black).bg(severity_color(a.severity)).bold();
        let mut spans = vec![Span::styled(format!(" ⚠ {}{}", a.event, alert_until(a)), style)];
        // The last banner line also says how many more there are and how to read them.
        if i + 1 == shown {
            let more = alerts.len() - shown;
            let hint = if more > 0 { format!("  +{more} more · a details ") } else { "  a details ".into() };
            spans.push(Span::styled(hint, style.not_bold()));
        }
        f.render_widget(Paragraph::new(Line::from(spans)).style(style), row);
    }
}

fn draw_alert_popup(f: &mut Frame, area: Rect, fc: Option<&Forecast>, scroll: u16) {
    let popup = area.inner(ratatui::layout::Margin { horizontal: 2, vertical: 1 });
    f.render_widget(Clear, popup);

    let dim = Style::new().dark_gray();
    let mut lines: Vec<Line> = Vec::new();
    match fc.and_then(|fc| fc.alerts.as_ref()) {
        None => lines.push(Line::styled("Weather alerts come from the US National Weather Service and aren't available for this location.", dim)),
        Some(alerts) if alerts.is_empty() => lines.push(Line::styled("No active weather alerts for this area.", Style::new().green())),
        Some(alerts) => {
            for (i, a) in alerts.iter().enumerate() {
                if i > 0 {
                    lines.push(Line::raw(""));
                    lines.push(Line::styled("─".repeat(popup.width.saturating_sub(4) as usize), dim));
                    lines.push(Line::raw(""));
                }
                lines.push(Line::from(vec![
                    Span::styled(format!(" {} ", a.event), Style::new().fg(Color::Black).bg(severity_color(a.severity)).bold()),
                    Span::styled(format!(" {:?}{}", a.severity, alert_until(a)), Style::new().bold()),
                ]));
                if let Some(h) = &a.headline {
                    lines.push(Line::raw(h.clone()));
                }
                lines.push(Line::styled(a.area.clone(), dim));
                lines.push(Line::raw(""));
                // NWS text is hard-wrapped at ~70 columns; rejoin paragraphs so it re-wraps to the popup.
                for para in a.description.split("\n\n") {
                    lines.push(Line::raw(para.replace('\n', " ")));
                    lines.push(Line::raw(""));
                }
                if let Some(ins) = &a.instruction {
                    lines.push(Line::styled("What to do", Style::new().bold()));
                    for para in ins.split("\n\n") {
                        lines.push(Line::raw(para.replace('\n', " ")));
                    }
                }
            }
        }
    }

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().yellow())
        .padding(Padding::horizontal(1))
        .title(Span::styled(" Weather alerts ", Style::new().bold()))
        .title_bottom(Line::styled(" ↑↓ scroll · a / Esc close ", dim).right_aligned());
    let p = Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((scroll, 0)).block(block);
    f.render_widget(p, popup);
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let place = app.place.as_ref().map(|p| p.name.as_str()).unwrap_or("…");
    let date = match &app.status {
        Status::Ready(fc) => fc.now.format("%A, %B %-d").to_string(),
        _ => app.last_good.as_ref().map(|fc| fc.now.format("%A, %B %-d").to_string()).unwrap_or_default(),
    };
    let tab = |label: &'static str, screen: Screen| {
        if app.screen == screen {
            Span::styled(format!(" {label} "), Style::new().bold().black().on_cyan())
        } else {
            Span::styled(format!(" {label} "), Style::new().gray())
        }
    };
    let left = Line::from(vec![
        Span::styled(format!(" {place}  "), Style::new().bold().cyan()),
        tab("Today", Screen::Today),
        Span::raw(" "),
        tab("7 days", Screen::Week),
        Span::raw(" "),
        tab("Radar", Screen::Radar),
    ]);
    let right = Line::from(Span::styled(format!("{date} "), Style::new().gray())).alignment(Alignment::Right);
    f.render_widget(left, area);
    f.render_widget(right, area);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let keys: &[(&str, &str)] = if app.screen == Screen::Radar {
        &[("tab", "switch"), ("+/-", "zoom"), ("space", "play"), ("q", "quit")]
    } else {
        &[("tab", "switch"), ("r", "refresh"), ("q", "quit")]
    };
    let mut spans = vec![Span::raw(" ")];
    for (k, what) in keys {
        spans.push(Span::styled(*k, Style::new().bold()));
        spans.push(Span::raw(format!(" {what}  ")));
    }
    let keys_w = Line::from(spans.clone()).width() as u16;
    let status = if app.fetching {
        Span::styled("updating… ", Style::new().yellow())
    } else if let (Status::Error(e), Some(_)) = (&app.status, &app.last_good) {
        Span::styled(format!("refresh failed: {e} "), Style::new().red())
    } else if let Some(t) = app.updated {
        Span::styled(format!("updated {} ", t.format("%-I:%M %p")), Style::new().dark_gray())
    } else {
        Span::raw("")
    };
    let [keys, right] = Layout::horizontal([Constraint::Length(keys_w), Constraint::Fill(1)]).areas(area);
    f.render_widget(Line::from(spans).dark_gray(), keys);
    f.render_widget(Line::from(status).alignment(Alignment::Right), right);
}

fn draw_today(f: &mut Frame, area: Rect, fc: &Forecast, units: Units) {
    // Summary row is fixed height; the chart only appears when there is room for it.
    let show_chart = area.height >= 24;
    let [top, chart, hourly] = Layout::vertical([
        Constraint::Length(9),
        // Charts stop growing past this, so tall panes don't stretch them.
        if show_chart { Constraint::Max(30) } else { Constraint::Length(0) },
        Constraint::Length(7),
    ])
    .flex(Flex::Start)
    .areas(area);

    let [today, now] = Layout::horizontal([Constraint::Fill(3), Constraint::Fill(2)]).areas(top);
    draw_summary(f, today, fc, units);
    draw_now(f, now, fc, units);
    if show_chart {
        // Add a precipitation chart below the temperature chart when there's room for both.
        if chart.height >= 16 {
            let [temp, precip] =
                Layout::vertical([Constraint::Fill(2), Constraint::Fill(1)]).areas(chart);
            draw_chart(f, temp, fc, units);
            draw_precip_chart(f, precip, fc);
        } else {
            draw_chart(f, chart, fc, units);
        }
    }
    draw_hourly(f, hourly, fc, units);
}

fn draw_summary(f: &mut Frame, area: Rect, fc: &Forecast, units: Units) {
    let label = |s: &str| Span::styled(s.to_string(), Style::new().dark_gray());
    let lines = vec![
        Line::from(vec![
            Span::styled(format!("{} ", wmo::icon(fc.day_code, true)), Style::new().fg(wmo::color(fc.day_code))),
            Span::styled(wmo::describe(fc.day_code), Style::new().bold()),
        ]),
        Line::from(vec![
            label("High "),
            Span::styled(format!("{}{}", deg(fc.high), units.temp().trim_start_matches('°')), Style::new().fg(temp_color(fc.high, units)).bold()),
            label("   Low "),
            Span::styled(format!("{}{}", deg(fc.low), units.temp().trim_start_matches('°')), Style::new().fg(temp_color(fc.low, units)).bold()),
        ]),
        Line::from(vec![
            label("Precip "),
            Span::styled(format!("{}%", fc.precip_prob_max), Style::new().blue()),
            Span::raw(format!("  {:.2} {}", fc.precip_sum, units.precip())),
        ]),
        {
            let o = crate::precip::outlook(&fc.hours, fc.now);
            let style = if o.dry {
                Style::new().dark_gray()
            } else if o.likely {
                Style::new().blue().bold()
            } else {
                Style::new().blue()
            };
            let icon = if o.dry { "" } else { "☂ " };
            Line::styled(format!("{icon}{}", o.text), style)
        },
        Line::from(vec![
            label("Wind "),
            Span::raw(format!("up to {:.0} {}", fc.wind_max, units.speed())),
            label("   UV "),
            Span::raw(format!("{:.0}", fc.uv_max)),
        ]),
        Line::from(vec![
            label("Sun "),
            Span::styled(format!("↑ {}", clock(fc.sunrise)), Style::new().yellow()),
            Span::raw("  "),
            Span::styled(format!("↓ {}", clock(fc.sunset)), Style::new().light_red()),
        ]),
        match &fc.air {
            Some(a) => Line::from(vec![
                label("Air peak "),
                aqi_span(a.peak_aqi),
                label(&format!(" at {}", short_hour(a.peak_time))),
            ]),
            None => Line::from(label("Air quality unavailable")),
        },
    ];
    f.render_widget(Paragraph::new(lines).block(panel("Today")), area);
}

fn draw_now(f: &mut Frame, area: Rect, fc: &Forecast, units: Units) {
    let label = |s: &str| Span::styled(s.to_string(), Style::new().dark_gray());
    let lines = vec![
        Line::from(vec![
            Span::styled(format!("{}{}", deg(fc.current_temp), units.temp().trim_start_matches('°')), Style::new().fg(temp_color(fc.current_temp, units)).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(wmo::icon(fc.current_code, fc.current_is_day), Style::new().fg(wmo::color(fc.current_code))),
            Span::raw(format!(" {}", wmo::describe(fc.current_code))),
        ]),
        Line::from(vec![label("Feels like "), Span::raw(deg(fc.feels_like))]),
        Line::from(vec![label("Humidity "), Span::raw(format!("{}%", fc.humidity))]),
        Line::from({
            let mut spans = vec![label("Wind "), Span::raw(format!("{:.0} {}", fc.current_wind, units.speed()))];
            if let Some(d) = fc.wind_dir {
                spans.push(Span::raw(format!(" {}", compass(d))));
            }
            // Gusts only matter when they're well above the steady wind.
            if let Some(g) = fc.gusts.filter(|g| *g >= fc.current_wind + 5.0) {
                spans.push(label(&format!(", gusts {g:.0}")));
            }
            spans
        }),
        Line::from({
            let mut spans = Vec::new();
            // Kept short: the Now panel is narrow. The unit (inHg / hPa) says it's pressure.
            if let Some(p) = fc.pressure_hpa {
                spans.push(Span::raw(units.pressure(p)));
            }
            if let Some(c) = fc.cloud_cover {
                spans.push(label(if spans.is_empty() { "clouds " } else { " · clouds " }));
                spans.push(Span::raw(format!("{c}%")));
            }
            spans
        }),
        match &fc.air {
            Some(a) => {
                let mut spans = vec![label("Air "), aqi_span(a.current_aqi)];
                if let Some(pm) = a.pm2_5 {
                    spans.push(label(&format!("  PM2.5 {pm:.0}")));
                }
                Line::from(spans)
            }
            None => Line::from(label("Air quality unavailable")),
        },
        Line::from(label_time(fc.now)),
    ];
    f.render_widget(Paragraph::new(lines).block(panel("Now")), area);
}

fn label_time(t: NaiveDateTime) -> Span<'static> {
    Span::styled(format!("as of {}", clock(t)), Style::new().dark_gray())
}

fn draw_chart(f: &mut Frame, area: Rect, fc: &Forecast, units: Units) {
    let temps: Vec<(f64, f64)> = fc
        .hours
        .iter()
        .map(|h| (h.time.hour() as f64, h.temp))
        .collect();
    if temps.is_empty() {
        return;
    }
    let lo = temps.iter().map(|p| p.1).fold(f64::INFINITY, f64::min).floor() - 2.0;
    let hi = temps.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max).ceil() + 2.0;
    let now_x = fc.now.hour() as f64 + fc.now.minute() as f64 / 60.0;
    let now_line: Vec<(f64, f64)> = (0..=20).map(|i| (now_x, lo + (hi - lo) * i as f64 / 20.0)).collect();
    // Daylight: a bar along the top of the chart from sunrise to sunset.
    let frac = |t: NaiveDateTime| t.hour() as f64 + t.minute() as f64 / 60.0;
    let daylight = [(frac(fc.sunrise), hi), (frac(fc.sunset), hi)];

    let datasets = vec![
        Dataset::default()
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::new().yellow())
            .data(&daylight),
        Dataset::default()
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::new().dark_gray())
            .data(&now_line),
        Dataset::default()
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::new().fg(temp_color(fc.high, units)))
            .data(&temps),
    ];
    let x_labels: Vec<Line> = ["12a", "6a", "12p", "6p", "12a"].into_iter().map(Line::from).collect();
    let y_labels: Vec<Line> = [lo, (lo + hi) / 2.0, hi].into_iter().map(|v| Line::from(deg(v))).collect();
    let block = panel("Temperature").title(Line::from(vec![
        Span::styled("━", Style::new().yellow()),
        Span::styled(
            format!(" daylight {} – {} ", fc.sunrise.format("%-I:%M"), fc.sunset.format("%-I:%M")),
            Style::new().dark_gray(),
        ),
    ]));
    let chart = Chart::new(datasets)
        .block(block)
        .x_axis(Axis::default().bounds([0.0, 24.0]).labels(x_labels).style(Style::new().dark_gray()))
        .y_axis(Axis::default().bounds([lo, hi]).labels(y_labels).style(Style::new().dark_gray()));
    f.render_widget(chart, area);
}

fn draw_precip_chart(f: &mut Frame, area: Rect, fc: &Forecast) {
    // Bar graph: each hour is drawn as a vertical line from 0 up to its probability.
    let probs: Vec<(f64, f64)> =
        fc.hours
            .iter()
            .filter(|h| h.precip_prob > 0) // a 0% bar still draws a block, which reads as "some"
            .map(|h| (h.time.hour() as f64, h.precip_prob as f64))
            .collect();
    let now_x = fc.now.hour() as f64 + fc.now.minute() as f64 / 60.0;
    let now_line: Vec<(f64, f64)> = (0..=20).map(|i| (now_x, i as f64 * 5.0)).collect();
    let datasets = vec![
        Dataset::default()
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::new().dark_gray())
            .data(&now_line),
        Dataset::default()
            .marker(Marker::HalfBlock)
            .graph_type(GraphType::Bar)
            .style(Style::new().blue())
            .data(&probs),
    ];
    let x_labels: Vec<Line> = ["12a", "6a", "12p", "6p", "12a"].into_iter().map(Line::from).collect();
    let y_labels: Vec<Line> = ["0%", "50%", "100%"].into_iter().map(Line::from).collect();
    let chart = Chart::new(datasets)
        .block(panel("Chance of precipitation"))
        .x_axis(Axis::default().bounds([0.0, 24.0]).labels(x_labels).style(Style::new().dark_gray()))
        .y_axis(Axis::default().bounds([0.0, 100.0]).labels(y_labels).style(Style::new().dark_gray()));
    f.render_widget(chart, area);
}

fn draw_hourly(f: &mut Frame, area: Rect, fc: &Forecast, units: Units) {
    let block = panel("Hourly");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let now_hour = fc.now.hour();
    let fits = (inner.width / HOUR_COL_WIDTH).max(1) as usize;
    let upcoming: Vec<_> = fc.hours.iter().filter(|h| h.time.hour() >= now_hour).collect();
    let hours = &upcoming[..upcoming.len().min(fits)];

    let cols = Layout::horizontal(vec![Constraint::Length(HOUR_COL_WIDTH); hours.len()]).split(inner);
    for (h, col) in hours.iter().zip(cols.iter()) {
        let is_now = h.time.hour() == now_hour;
        let time_style = if is_now { Style::new().bold().cyan() } else { Style::new().gray() };
        // The current hour shows the live reading rather than the top-of-hour forecast.
        let (code, is_day, temp) =
            if is_now { (fc.current_code, fc.current_is_day, fc.current_temp) } else { (h.code, h.is_day, h.temp) };
        let lines = vec![
            Line::styled(if is_now { "now".to_string() } else { short_hour(h.time) }, time_style),
            Line::styled(wmo::icon(code, is_day), Style::new().fg(wmo::color(code))),
            Line::styled(deg(temp), Style::new().fg(temp_color(temp, units)).bold()),
            Line::styled(
                format!("{}%", h.precip_prob),
                if h.precip_prob >= 30 { Style::new().blue() } else { Style::new().dark_gray() },
            ),
            match h.aqi {
                Some(a) => Line::styled(format!("{a}"), Style::new().fg(aqi_color(a))),
                None => Line::styled("–", Style::new().dark_gray()),
            },
        ];
        f.render_widget(Paragraph::new(lines).alignment(Alignment::Center), *col);
    }
}

fn day_name(fc: &Forecast, date: chrono::NaiveDate) -> String {
    if date == fc.now.date() {
        "Today".into()
    } else {
        date.format("%a %-d").to_string()
    }
}

fn draw_week(f: &mut Frame, area: Rect, fc: &Forecast, units: Units) {
    let n = fc.days.len() as u16;
    if n == 0 {
        f.render_widget(Paragraph::new("No daily forecast.").block(panel("7 days")), area);
        return;
    }
    // Two lines per day when there's room, and a chart below when there's even more.
    let row_h: u16 = if area.height >= n * 2 + 2 { 2 } else { 1 };
    let table_h = n * row_h + 2;
    let show_chart = area.height >= table_h + 12;
    let [table, chart] = Layout::vertical([
        Constraint::Length(table_h),
        if show_chart { Constraint::Fill(1) } else { Constraint::Length(0) },
    ])
    .flex(Flex::Start)
    .areas(area);

    let block = panel("Next 7 days");
    let inner = block.inner(table);
    f.render_widget(block, table);

    // Narrow panes drop the description and wind columns first.
    // The range bar should keep at least ~16 cells, so the extras go first.
    let show_desc = inner.width >= 70;
    let show_wind = inner.width >= 90;
    let mut cols = vec![
        Constraint::Length(8), // day
        Constraint::Length(3), // icon
    ];
    if show_desc {
        cols.push(Constraint::Length(17));
    }
    cols.extend([
        Constraint::Length(5), // low
        Constraint::Min(8),    // range bar
        Constraint::Length(5), // high
        Constraint::Length(4), // precip %
        Constraint::Length(8), // precip amount
    ]);
    if show_wind {
        cols.push(Constraint::Length(8));
    }

    let week_lo = fc.days.iter().map(|d| d.low).fold(f64::INFINITY, f64::min);
    let week_hi = fc.days.iter().map(|d| d.high).fold(f64::NEG_INFINITY, f64::max);
    let dim = Style::new().dark_gray();

    for (i, day) in fc.days.iter().enumerate() {
        let row = Rect { y: inner.y + i as u16 * row_h, height: 1, ..inner };
        if row.y >= inner.bottom() {
            break;
        }
        let cells = Layout::horizontal(cols.clone()).spacing(1).split(row);
        let mut c = cells.iter().copied();
        let is_today = day.date == fc.now.date();
        let name_style = if is_today { Style::new().bold().cyan() } else { Style::new().bold() };
        let weekend = matches!(day.date.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun);
        let name_style = if weekend && !is_today { name_style.fg(Color::Gray) } else { name_style };

        f.render_widget(Line::styled(day_name(fc, day.date), name_style), c.next().unwrap());
        f.render_widget(
            Line::styled(wmo::icon(day.code, true), Style::new().fg(wmo::color(day.code))),
            c.next().unwrap(),
        );
        if show_desc {
            f.render_widget(Line::raw(wmo::describe(day.code)), c.next().unwrap());
        }
        f.render_widget(
            Line::styled(deg(day.low), Style::new().fg(temp_color(day.low, units))).alignment(Alignment::Right),
            c.next().unwrap(),
        );
        let bar = c.next().unwrap();
        let now_temp = is_today.then_some(fc.current_temp);
        f.render_widget(range_bar(day.low, day.high, week_lo, week_hi, bar.width, now_temp, units), bar);
        f.render_widget(
            Line::styled(deg(day.high), Style::new().fg(temp_color(day.high, units)).bold()),
            c.next().unwrap(),
        );
        f.render_widget(
            Line::styled(
                format!("{}%", day.precip_prob),
                if day.precip_prob >= 30 { Style::new().blue().bold() } else { dim },
            )
            .alignment(Alignment::Right),
            c.next().unwrap(),
        );
        let amount = if day.precip_sum > 0.0 {
            Span::styled(format!("{:.2} {}", day.precip_sum, units.precip()), Style::new().blue())
        } else {
            Span::styled("–", dim)
        };
        f.render_widget(Line::from(amount).alignment(Alignment::Right), c.next().unwrap());
        if show_wind {
            f.render_widget(
                Line::styled(format!("{:.0} {}", day.wind_max, units.speed()), dim).alignment(Alignment::Right),
                c.next().unwrap(),
            );
        }
    }

    if show_chart {
        draw_week_chart(f, chart, fc, units);
    }
}

/// A bar spanning the week's coldest to warmest temperature, with this day's low-to-high filled in.
/// Today's row also marks the current temperature with a dot.
fn range_bar(lo: f64, hi: f64, week_lo: f64, week_hi: f64, width: u16, now: Option<f64>, units: Units) -> Line<'static> {
    let w = width as usize;
    if w == 0 {
        return Line::raw("");
    }
    let span = (week_hi - week_lo).max(1.0);
    let pos = |t: f64| (((t - week_lo) / span) * (w - 1) as f64).round().clamp(0.0, (w - 1) as f64) as usize;
    let (a, b) = (pos(lo), pos(hi));
    let now_pos = now.map(pos);
    let spans: Vec<Span> = (0..w)
        .map(|i| {
            if Some(i) == now_pos {
                Span::styled("●", Style::new().white().bold())
            } else if (a..=b).contains(&i) {
                // Shade each cell by the temperature it stands for.
                let t = week_lo + span * i as f64 / (w - 1).max(1) as f64;
                Span::styled("━", Style::new().fg(temp_color(t, units)))
            } else {
                Span::styled("─", Style::new().fg(Color::Indexed(238)))
            }
        })
        .collect();
    Line::from(spans)
}

fn draw_week_chart(f: &mut Frame, area: Rect, fc: &Forecast, _units: Units) {
    const LOW: Color = Color::Indexed(75); // blue
    const HIGH: Color = Color::Indexed(209); // orange

    // Bars grow from a baseline just under the week's low rather than from 0°, so a
    // 10° difference reads as a real difference. Labels show the actual temperatures.
    let week_lo = fc.days.iter().map(|d| d.low).fold(f64::INFINITY, f64::min);
    let base = (week_lo / 10.0).floor() * 10.0 - 5.0;
    let height = |t: f64| (t - base).max(1.0).round() as u64;

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().dark_gray())
        .padding(Padding::horizontal(1))
        .title(Line::from(vec![
            Span::styled(" Highs and lows  ", Style::new().bold()),
            Span::styled("■", Style::new().fg(HIGH)),
            Span::raw(" high  "),
            Span::styled("■", Style::new().fg(LOW)),
            Span::raw(" low "),
        ]));
    let inner_w = block.inner(area).width;

    // Two bars per day. Make the bars as wide as fit (labels like "71°" need 3 cells),
    // then spread the leftover width between days so the chart spans the panel.
    let n = fc.days.len().max(1) as u16;
    // ratatui counts bar_gap after every bar, including a group's last one.
    let per_group = inner_w.saturating_sub(n - 1) / n;
    let bar_gap = if per_group >= 10 { 1 } else { 0 };
    let bar_w = (per_group / 2).saturating_sub(bar_gap).clamp(1, 12);
    let group_gap =
        (inner_w.saturating_sub(n * 2 * (bar_w + bar_gap)) / (n - 1).max(1)).clamp(1, 10);

    let mut chart = BarChart::default()
        .block(block)
        .bar_width(bar_w)
        .bar_gap(bar_gap)
        .group_gap(group_gap);
    for d in &fc.days {
        let label = if d.date == fc.now.date() { "Today".to_string() } else { d.date.format("%a").to_string() };
        let bar = |t: f64, color: Color| {
            Bar::default()
                .value(height(t))
                .text_value(deg(t))
                .style(Style::new().fg(color))
                .value_style(Style::new().fg(Color::Black).bg(color).bold())
        };
        let bars = [bar(d.high, HIGH), bar(d.low, LOW)];
        chart = chart.data(BarGroup::default().label(Line::from(label).centered()).bars(&bars));
    }
    f.render_widget(chart, area);
}

/// Radar levels 1..=6 in the usual green → yellow → red → magenta progression.
fn radar_color(level: u8) -> Color {
    match level {
        1 => Color::Indexed(28),  // dark green: drizzle
        2 => Color::Indexed(40),  // green: light rain
        3 => Color::Indexed(220), // yellow: moderate
        4 => Color::Indexed(208), // orange: heavy
        5 => Color::Indexed(196), // red: very heavy
        _ => Color::Indexed(201), // magenta: extreme / hail
    }
}

fn draw_radar(f: &mut Frame, area: Rect, app: &App) {
    use crate::{basemap, radar};

    let r = &app.radar;
    let dim = Style::new().dark_gray();

    // Title: scan time, how old it is, and where we are in the loop.
    let mut title = vec![Span::styled(" Radar ", Style::new().bold())];
    if let (Some(fr), Some(frames)) = (r.current(), &r.frames) {
        if let Some(t) = chrono::DateTime::from_timestamp(fr.time, 0) {
            let local = t.with_timezone(&chrono::Local);
            let mins = (chrono::Utc::now() - t).num_minutes().max(0);
            title.push(Span::raw(format!("{} ", local.format("%-I:%M %p"))));
            title.push(Span::styled(format!("({mins} min ago) "), dim));
        }
        let state = if r.playing { "▶" } else { "❚❚" };
        title.push(Span::styled(format!("{state} {}/{} ", r.frame + 1, frames.frames.len()), dim));
    }
    if r.loading() {
        title.push(Span::styled("loading… ", Style::new().yellow()));
    }
    let km = radar::ZOOM_KM[r.zoom];
    let mut legend = vec![Span::styled(" light ", dim)];
    legend.extend((1..=6).map(|l| Span::styled("■", Style::new().fg(radar_color(l)))));
    legend.push(Span::styled(format!(" heavy · {km:.0} km across "), dim));

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(dim)
        .title(Line::from(title))
        .title_bottom(Line::from(legend).right_aligned());
    let inner = block.inner(area);
    r.area.set(inner);
    f.render_widget(block, area);

    let (Some(place), Some(view)) = (&app.place, app.place.as_ref().and_then(|p| r.view(p))) else {
        return;
    };
    if r.frames.is_none() {
        let msg = match &r.error {
            Some(e) => format!("Couldn't load radar: {e}"),
            None => "Loading radar…".into(),
        };
        f.render_widget(Paragraph::new(msg).alignment(Alignment::Center).wrap(Wrap { trim: true }), inner);
        return;
    }

    // Tiles for the current scan, looked up by (x, y).
    let tiles: HashMap<(u32, u32), &radar::Tile> = match r.current() {
        Some(fr) => view
            .tiles()
            .into_iter()
            .filter_map(|(x, y)| {
                let key = radar::TileKey { path: fr.path.clone(), z: view.z, x, y };
                r.tiles.get(&key).and_then(|t| t.as_ref()).map(|t| ((x, y), t))
            })
            .collect(),
        None => HashMap::new(),
    };
    let lookup = |x: u32, y: u32| tiles.get(&(x, y)).copied();

    // Canvas coordinates are Mercator pixels, with y flipped so north is up.
    let (left, top, s) = (view.left, view.top, view.px_per_dot);
    let mut by_level: Vec<Vec<(f64, f64)>> = vec![Vec::new(); 7];
    for j in 0..view.dots_h {
        for i in 0..view.dots_w {
            let level = view.sample(i, j, &lookup);
            if level > 0 {
                by_level[level as usize].push((left + (i as f64 + 0.5) * s, -(top + (j as f64 + 0.5) * s)));
            }
        }
    }

    let (w, e, so, n) = view.bounds();
    let margin = (e - w) * 0.1;
    let z = view.z;
    let (right, bottom) = (left + view.width_px(), top + view.height_px());
    let (cx, cy) = radar::project(place.lon, place.lat, z);
    let name = place.name.split(',').next().unwrap_or(&place.name).to_string();

    let canvas = Canvas::default()
        .marker(Marker::Braille)
        .x_bounds([left, right])
        .y_bounds([-bottom, -top])
        .paint(move |ctx| {
            let lines = |ctx: &mut ratatui::widgets::canvas::Context, set: &[basemap::Polyline], color: Color| {
                for pl in set.iter().filter(|p| p.intersects(w - margin, e + margin, so - margin, n + margin)) {
                    let pts: Vec<(f64, f64)> =
                        pl.points.iter().map(|&(lon, lat)| radar::project(lon as f64, lat as f64, z)).collect();
                    for seg in pts.windows(2) {
                        let ((x1, y1), (x2, y2)) = (seg[0], seg[1]);
                        let outside = (x1 < left && x2 < left)
                            || (x1 > right && x2 > right)
                            || (y1 < top && y2 < top)
                            || (y1 > bottom && y2 > bottom);
                        if !outside {
                            ctx.draw(&CanvasLine { x1, y1: -y1, x2, y2: -y2, color });
                        }
                    }
                }
            };
            lines(ctx, basemap::borders(), Color::Indexed(239));
            lines(ctx, basemap::coast(), Color::Indexed(245));
            for (level, coords) in by_level.iter().enumerate().skip(1) {
                if !coords.is_empty() {
                    ctx.draw(&Points { coords, color: radar_color(level as u8) });
                }
            }
            ctx.print(cx, -cy, Span::styled("◉", Style::new().white().bold()));
            ctx.print(cx + 2.5 * s, -cy, Span::styled(name.clone(), Style::new().white()));
        });
    f.render_widget(canvas, inner);
}
