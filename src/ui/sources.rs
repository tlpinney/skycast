//! The Sources popup (`?`): where skycast's data comes from, with the credit each source
//! asks for.

use chrono::Datelike;
use ratatui::{
    Frame,
    layout::{Margin, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, Padding, Paragraph, Wrap},
};

struct Source {
    what: &'static str,
    credit: String,
    link: &'static str,
    terms: &'static str,
}

fn sources() -> Vec<Source> {
    let year = chrono::Utc::now().year();
    vec![
        Source {
            what: "Forecast, place search and model tides",
            credit: "Weather data by Open-Meteo.com".into(),
            link: "https://open-meteo.com",
            terms: "CC BY 4.0",
        },
        Source {
            what: "Air quality (via Open-Meteo)",
            credit: format!("Contains modified Copernicus Atmosphere Monitoring Service information {year}"),
            link: "https://atmosphere.copernicus.eu",
            terms: "Copernicus licence",
        },
        Source {
            what: "Model tides, outside the US (via Open-Meteo)",
            credit: "Generated using E.U. Copernicus Marine Service Information".into(),
            link: "https://marine.copernicus.eu",
            terms: "Copernicus Marine licence",
        },
        Source {
            what: "Tide predictions, US coasts",
            credit: "NOAA Tides & Currents (CO-OPS)".into(),
            link: "https://tidesandcurrents.noaa.gov",
            terms: "US public domain",
        },
        Source {
            what: "Weather alerts, US",
            credit: "National Weather Service".into(),
            link: "https://www.weather.gov",
            terms: "US public domain",
        },
        Source {
            what: "Radar",
            credit: "RainViewer".into(),
            link: "https://www.rainviewer.com",
            terms: "free for personal and educational use",
        },
        Source {
            what: "Map outlines",
            credit: "Made with Natural Earth".into(),
            link: "https://www.naturalearthdata.com",
            terms: "public domain",
        },
        Source {
            what: "Sun and moon",
            credit: "Computed locally (NOAA solar equations; Meeus, Astronomical Algorithms)".into(),
            link: "",
            terms: "",
        },
    ]
}

pub fn draw(f: &mut Frame, area: Rect, scroll: u16) {
    let popup = area.inner(Margin { horizontal: 2, vertical: 1 });
    f.render_widget(Clear, popup);
    let dim = Style::new().dark_gray();
    let mut lines = Vec::new();
    for (i, s) in sources().into_iter().enumerate() {
        if i > 0 {
            lines.push(Line::raw(""));
        }
        lines.push(Line::styled(s.what, Style::new().bold()));
        lines.push(Line::raw(format!("  {}", s.credit)));
        if !s.link.is_empty() {
            lines.push(Line::from(vec![
                Span::styled(format!("  {}", s.link), Style::new().cyan()),
                Span::styled(format!("  ·  {}", s.terms), dim),
            ]));
        }
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().cyan())
        .padding(Padding::horizontal(1))
        .title(Span::styled(" Data sources ", Style::new().bold()))
        .title_bottom(Line::styled(" ↑↓ scroll · ? / Esc close ", dim).right_aligned());
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((scroll, 0)).block(block), popup);
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_source_has_a_credit() {
        for s in super::sources() {
            assert!(!s.credit.is_empty() && !s.what.is_empty());
            assert!(s.link.is_empty() || s.link.starts_with("https://"));
        }
    }
}
