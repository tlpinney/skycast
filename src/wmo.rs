//! WMO weather interpretation codes, as used by Open-Meteo.

use ratatui::style::Color;

pub fn describe(code: u8) -> &'static str {
    match code {
        0 => "Clear sky",
        1 => "Mainly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 | 48 => "Fog",
        51 => "Light drizzle",
        53 => "Drizzle",
        55 => "Heavy drizzle",
        56 | 57 => "Freezing drizzle",
        61 => "Light rain",
        63 => "Rain",
        65 => "Heavy rain",
        66 | 67 => "Freezing rain",
        71 => "Light snow",
        73 => "Snow",
        75 => "Heavy snow",
        77 => "Snow grains",
        80 => "Light showers",
        81 => "Showers",
        82 => "Violent showers",
        85 | 86 => "Snow showers",
        95 => "Thunderstorm",
        96 | 99 => "Thunderstorm, hail",
        _ => "Unknown",
    }
}

pub fn icon(code: u8, is_day: bool) -> &'static str {
    match code {
        0 | 1 if is_day => "☀",
        0 | 1 => "☾",
        2 => "⛅",
        3 => "☁",
        45 | 48 => "≋",
        51..=67 | 80..=82 => "☂",
        71..=77 | 85 | 86 => "❄",
        95..=99 => "⚡",
        _ => "?",
    }
}

pub fn color(code: u8) -> Color {
    match code {
        0 | 1 => Color::Yellow,
        2 | 3 | 45 | 48 => Color::Gray,
        51..=67 | 80..=82 => Color::Blue,
        71..=77 | 85 | 86 => Color::White,
        95..=99 => Color::Magenta,
        _ => Color::Reset,
    }
}
