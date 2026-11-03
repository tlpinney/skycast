//! Layouts for short panes: a dense Today view when the full one doesn't fit, and a
//! 1–3 line strip (no header or footer) for very thin panes.

use chrono::Timelike;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use super::{aqi_color, deg, draw_hourly, severity_color, short_hour, temp_color};
use crate::api::{Forecast, Units, aqi_category, compass};
use crate::precip;
use crate::wmo;

const SEP: &str = " · ";

fn dim(s: impl Into<String>) -> Span<'static> {
    Span::styled(s.into(), Style::new().dark_gray())
}

fn temp(t: f64, units: Units) -> Span<'static> {
    Span::styled(deg(t), Style::new().fg(temp_color(t, units)).bold())
}

fn wind(fc: &Forecast, units: Units) -> String {
    let mut s = format!("{:.0} {}", fc.current_wind, units.speed());
    if let Some(d) = fc.wind_dir {
        s = format!("{s} {}", compass(d));
    }
    if let Some(g) = fc.gusts.filter(|g| *g >= fc.current_wind + 5.0) {
        s = format!("{s}, gusts {g:.0}");
    }
    s
}

/// "☀ 49° Clear sky · feels 46° · wind 3 mph NW"
fn now_line(fc: &Forecast, units: Units) -> Line<'static> {
    Line::from(vec![
        Span::styled(wmo::icon(fc.current_code, fc.current_is_day), Style::new().fg(wmo::color(fc.current_code))),
        Span::raw(" "),
        temp(fc.current_temp, units),
        Span::raw(format!(" {}", wmo::describe(fc.current_code))),
        dim(format!("{SEP}feels {}{SEP}wind {}", deg(fc.feels_like), wind(fc, units))),
    ])
}

/// "Today ☁ 71° / 44° Mainly clear · ↑6:39 ↓5:06"
fn today_line(fc: &Forecast, units: Units) -> Line<'static> {
    Line::from(vec![
        dim("Today "),
        Span::styled(wmo::icon(fc.day_code, true), Style::new().fg(wmo::color(fc.day_code))),
        Span::raw(" "),
        temp(fc.high, units),
        dim(" / "),
        temp(fc.low, units),
        Span::raw(format!(" {}", wmo::describe(fc.day_code))),
        dim(format!(
            "{SEP}↑{} ↓{}",
            fc.sunrise.format("%-I:%M"),
            fc.sunset.format("%-I:%M")
        )),
    ])
}

fn rain_line(fc: &Forecast) -> Line<'static> {
    let o = precip::outlook(&fc.hours, fc.now);
    if o.dry {
        Line::from(dim(o.text))
    } else {
        let style = if o.likely { Style::new().blue().bold() } else { Style::new().blue() };
        Line::styled(format!("☂ {}", o.text), style)
    }
}

/// "AQI 51 Moderate · peak 55 at 8p · 30.06 inHg · clouds 20%"
fn air_line(fc: &Forecast, units: Units) -> Line<'static> {
    let mut spans = Vec::new();
    if let Some(a) = &fc.air {
        spans.push(dim("AQI "));
        spans.push(Span::styled(
            format!("{} {}", a.current_aqi, aqi_category(a.current_aqi)),
            Style::new().fg(aqi_color(a.current_aqi)),
        ));
        spans.push(dim(format!(" · peak {} at {}", a.peak_aqi, short_hour(a.peak_time))));
    }
    let mut extra = Vec::new();
    if let Some(p) = fc.pressure_hpa {
        extra.push(units.pressure(p));
    }
    if let Some(c) = fc.cloud_cover {
        extra.push(format!("clouds {c}%"));
    }
    if !extra.is_empty() {
        let lead = if spans.is_empty() { "" } else { SEP };
        spans.push(dim(format!("{lead}{}", extra.join(SEP))));
    }
    Line::from(spans)
}

/// "now ☀47°  9a ☀54°  10a ☀57° …", as many hours as fit.
fn hourly_line(fc: &Forecast, units: Units, width: u16) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    let mut used = 1usize;
    for h in fc.hours.iter().filter(|h| h.time.hour() >= fc.now.hour()) {
        let is_now = h.time.hour() == fc.now.hour();
        let label = if is_now { "now".to_string() } else { short_hour(h.time) };
        let t = if is_now { fc.current_temp } else { h.temp };
        let rain = if h.precip_prob >= 30 { format!(" {}%", h.precip_prob) } else { String::new() };
        // Icons can be two cells wide, so budget generously.
        let cell = label.len() + 1 + 2 + deg(t).chars().count() + rain.len() + 2;
        if used + cell > width as usize {
            break;
        }
        used += cell;
        spans.push(dim(format!("{label} ")));
        spans.push(Span::styled(wmo::icon(h.code, h.is_day), Style::new().fg(wmo::color(h.code))));
        spans.push(temp(t, units));
        if !rain.is_empty() {
            spans.push(Span::styled(rain, Style::new().blue()));
        }
        spans.push(Span::raw("  "));
    }
    Line::from(spans)
}

/// "Wed ☁ 72/51  Thu ☂ 76/64 …" for the days after today.
fn week_line(fc: &Forecast, units: Units, width: u16) -> Line<'static> {
    let mut spans = Vec::new();
    let mut used = 0usize;
    for d in fc.days.iter().filter(|d| d.date > fc.now.date()) {
        // "Wed ☁ 74/53  ": the icon may be two cells wide.
        let cell = 3 + 1 + 2 + 1 + 5 + 2;
        if used + cell > width as usize {
            break;
        }
        used += cell;
        spans.push(dim(format!("{} ", d.date.format("%a"))));
        spans.push(Span::styled(wmo::icon(d.code, true), Style::new().fg(wmo::color(d.code))));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(format!("{:.0}", d.high), Style::new().fg(temp_color(d.high, units))));
        spans.push(dim("/"));
        spans.push(Span::styled(format!("{:.0}", d.low), Style::new().fg(temp_color(d.low, units))));
        spans.push(Span::raw("  "));
    }
    Line::from(spans)
}

/// Everything important on one line, dropping the least important pieces to fit `width`.
fn one_line(fc: &Forecast, units: Units, place: &str, width: u16) -> Line<'static> {
    let mut segments: Vec<Vec<Span<'static>>> = Vec::new();
    if let Some(a) = fc.alerts.as_ref().and_then(|a| a.first()) {
        let style = Style::new().fg(Color::Black).bg(severity_color(a.severity)).bold();
        segments.push(vec![Span::styled(format!(" ⚠ {} ", a.event), style)]);
    }
    segments.push(vec![
        Span::styled(wmo::icon(fc.current_code, fc.current_is_day), Style::new().fg(wmo::color(fc.current_code))),
        Span::raw(" "),
        temp(fc.current_temp, units),
    ]);
    segments.push(vec![dim("H "), temp(fc.high, units), dim(" L "), temp(fc.low, units)]);
    let o = precip::outlook(&fc.hours, fc.now);
    if !o.dry {
        segments.push(vec![Span::styled(format!("☂ {}", o.text), Style::new().blue())]);
    }
    if let Some(a) = &fc.air {
        segments.push(vec![dim("AQI "), Span::styled(a.current_aqi.to_string(), Style::new().fg(aqi_color(a.current_aqi)))]);
    }
    segments.push(vec![Span::raw(wmo::describe(fc.current_code))]);
    segments.push(vec![Span::styled(place.to_string(), Style::new().cyan())]);

    let mut spans = vec![Span::raw(" ")];
    let mut used = 1;
    for seg in segments {
        let w: usize = seg.iter().map(|s| s.width()).sum::<usize>() + SEP.chars().count();
        if used + w > width as usize {
            continue; // a later, shorter piece may still fit
        }
        if used > 1 {
            spans.push(dim(SEP));
        }
        used += w;
        spans.extend(seg);
    }
    Line::from(spans)
}

/// A pane only 1–3 rows tall: no header, footer or tabs, just the most useful lines.
pub fn draw_strip(f: &mut Frame, area: Rect, fc: &Forecast, units: Units, place: &str) {
    let lines = match area.height {
        0 => return,
        1 => vec![one_line(fc, units, place, area.width)],
        2 => vec![one_line(fc, units, place, area.width), hourly_line(fc, units, area.width.saturating_sub(1))],
        _ => vec![
            one_line(fc, units, place, area.width),
            rain_line(fc),
            hourly_line(fc, units, area.width.saturating_sub(1)),
        ],
    };
    f.render_widget(Paragraph::new(lines), area);
}

/// The Today screen when the pane is too short for panels and charts.
pub fn draw_today(f: &mut Frame, area: Rect, fc: &Forecast, units: Units) {
    const HOURLY_PANEL: u16 = 7;
    let w = area.width.saturating_sub(2);
    let summary = [now_line(fc, units), today_line(fc, units), rain_line(fc), air_line(fc, units)];
    let n = (summary.len() as u16).min(area.height);
    let inner = Rect { x: area.x + 1, width: area.width.saturating_sub(1), height: n, ..area };
    f.render_widget(Paragraph::new(summary[..n as usize].to_vec()), inner);

    // Below the summary: the full hourly panel if it fits, otherwise a one-line version,
    // then the rest of the week on one line.
    let mut y = area.y + n;
    let mut left = area.height - n;
    if left >= HOURLY_PANEL {
        draw_hourly(f, Rect { y, height: HOURLY_PANEL, ..area }, fc, units);
        y += HOURLY_PANEL;
        left -= HOURLY_PANEL;
    } else if left >= 2 {
        f.render_widget(hourly_line(fc, units, w), Rect { y: y + 1, height: 1, ..area });
        y += 2;
        left -= 2;
    }
    if left >= 2 {
        f.render_widget(week_line(fc, units, w), Rect { x: area.x + 1, y: y + 1, height: 1, ..area });
    }
}
