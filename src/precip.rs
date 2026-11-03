//! Turns today's hourly precipitation chances into one line, like "Rain likely 3–6p".

use chrono::{NaiveDateTime, Timelike};

use crate::api::Hour;

/// An hour at or above this chance counts as "likely".
const LIKELY: u8 = 50;
/// An hour at or above this chance counts as "possible".
const POSSIBLE: u8 = 25;
/// At most this many separate windows are listed.
const MAX_WINDOWS: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outlook {
    pub text: String,
    /// True when some window reaches the "likely" threshold.
    pub likely: bool,
    /// True when nothing reaches "possible" for the rest of the day.
    pub dry: bool,
}

/// What falls during a set of hours, judged by their WMO codes.
fn kind(codes: &[u8]) -> &'static str {
    let count = |f: fn(u8) -> bool| codes.iter().filter(|c| f(**c)).count();
    if count(|c| (95..=99).contains(&c)) > 0 {
        "Storms"
    } else if count(|c| matches!(c, 56 | 57 | 66 | 67)) > 0 {
        "Freezing rain"
    } else if count(|c| matches!(c, 71..=77 | 85 | 86)) * 2 > codes.len() {
        "Snow"
    } else {
        "Rain"
    }
}

fn hour_label(h: u32) -> String {
    match h % 24 {
        0 => "12a".into(),
        12 => "12p".into(),
        h if h < 12 => format!("{h}a"),
        h => format!("{}p", h - 12),
    }
}

/// "3–6p", "10a–2p", "11p–12a". `end` is exclusive, so hours 15..=17 read as 3–6p.
fn range_label(start: u32, end: u32) -> String {
    let (a, b) = (hour_label(start), hour_label(end));
    let same_half = (start < 12) == (end % 24 < 12) && end < 24 && end != 12;
    if same_half {
        format!("{}–{b}", a.trim_end_matches(['a', 'p']))
    } else {
        format!("{a}–{b}")
    }
}

/// Summarize precipitation for the rest of today (from `now`'s hour on).
pub fn outlook(hours: &[Hour], now: NaiveDateTime) -> Outlook {
    let rest: Vec<&Hour> = hours.iter().filter(|h| h.time.hour() >= now.hour()).collect();

    // Runs of consecutive hours at or above POSSIBLE.
    let mut runs: Vec<Vec<&Hour>> = Vec::new();
    for h in &rest {
        if h.precip_prob >= POSSIBLE {
            match runs.last_mut() {
                Some(run) if run.last().is_some_and(|p| p.time.hour() + 1 == h.time.hour()) => run.push(h),
                _ => runs.push(vec![h]),
            }
        }
    }
    if runs.is_empty() {
        return Outlook { text: "No rain expected rest of today".into(), likely: false, dry: true };
    }

    // If any run is likely, only list the likely ones; otherwise list the possible ones.
    let likely = runs.iter().any(|r| r.iter().any(|h| h.precip_prob >= LIKELY));
    if likely {
        runs.retain(|r| r.iter().any(|h| h.precip_prob >= LIKELY));
    }
    let codes: Vec<u8> = runs.iter().flatten().map(|h| h.code).collect();
    let peak = runs.iter().flatten().map(|h| h.precip_prob).max().unwrap_or(0);

    let starts_now = runs[0][0].time.hour() == now.hour();
    let windows: Vec<String> = runs
        .iter()
        .take(MAX_WINDOWS)
        .enumerate()
        .map(|(i, r)| {
            let start = r[0].time.hour();
            let end = r[r.len() - 1].time.hour() + 1;
            if i == 0 && starts_now {
                format!("until {}", hour_label(end))
            } else {
                range_label(start, end)
            }
        })
        .collect();
    let more = if runs.len() > MAX_WINDOWS { " +more" } else { "" };
    let word = if likely { "likely" } else { "possible" };
    let text = format!("{} {word} {}{more} ({peak}%)", kind(&codes), windows.join(", "));
    Outlook { text, likely, dry: false }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(h: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, 3).unwrap().and_hms_opt(h, 0, 0).unwrap()
    }

    /// A day of hours with the given (hour, chance) overrides; everything else is 0% and clear.
    fn day(probs: &[(u32, u8)], code: u8) -> Vec<Hour> {
        (0..24)
            .map(|h| {
                let p = probs.iter().find(|(ph, _)| *ph == h).map(|(_, p)| *p).unwrap_or(0);
                Hour { time: at(h), temp: 50.0, precip_prob: p, code: if p > 0 { code } else { 0 }, is_day: true, aqi: None }
            })
            .collect()
    }

    #[test]
    fn dry_day() {
        let o = outlook(&day(&[(15, 10)], 61), at(8));
        assert!(o.dry);
        assert_eq!(o.text, "No rain expected rest of today");
    }

    #[test]
    fn afternoon_rain() {
        let o = outlook(&day(&[(15, 60), (16, 80), (17, 55)], 61), at(8));
        assert_eq!(o.text, "Rain likely 3–6p (80%)");
        assert!(o.likely);
    }

    #[test]
    fn crosses_noon() {
        let o = outlook(&day(&[(10, 60), (11, 60), (12, 60), (13, 60)], 61), at(8));
        assert_eq!(o.text, "Rain likely 10a–2p (60%)");
    }

    #[test]
    fn ends_at_midnight() {
        let o = outlook(&day(&[(22, 60), (23, 60)], 61), at(8));
        assert_eq!(o.text, "Rain likely 10p–12a (60%)");
    }

    #[test]
    fn raining_now() {
        let o = outlook(&day(&[(8, 70), (9, 70), (10, 40)], 63), at(8));
        assert_eq!(o.text, "Rain likely until 11a (70%)");
    }

    #[test]
    fn past_rain_ignored() {
        let o = outlook(&day(&[(3, 90), (4, 90)], 61), at(8));
        assert!(o.dry);
    }

    #[test]
    fn possible_only() {
        let o = outlook(&day(&[(14, 30), (15, 35)], 80), at(8));
        assert_eq!(o.text, "Rain possible 2–4p (35%)");
        assert!(!o.likely);
    }

    #[test]
    fn likely_hides_possible_windows() {
        let o = outlook(&day(&[(9, 30), (15, 70)], 61), at(8));
        assert_eq!(o.text, "Rain likely 3–4p (70%)");
    }

    #[test]
    fn two_windows_and_more() {
        let o = outlook(&day(&[(9, 60), (13, 60), (18, 60)], 61), at(8));
        assert_eq!(o.text, "Rain likely 9–10a, 1–2p +more (60%)");
    }

    #[test]
    fn snow_and_storms() {
        assert!(outlook(&day(&[(15, 60)], 73), at(8)).text.starts_with("Snow likely"));
        assert!(outlook(&day(&[(15, 60)], 95), at(8)).text.starts_with("Storms likely"));
    }
}
