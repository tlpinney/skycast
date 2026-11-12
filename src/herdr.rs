//! Weather alerts as herdr toasts, when skycast is running inside a herdr pane.

use std::collections::HashSet;
use std::ffi::OsString;
use std::process::{Command, Stdio};
use std::thread;

use crate::api::{Alert, Severity};
use crate::ui::alert_until;

/// Shows a herdr notification for each alert the first time skycast sees it.
pub struct Notifier {
    /// The herdr binary, or None when not running inside herdr.
    bin: Option<OsString>,
    /// IDs of the alerts already shown, kept to the ones still active.
    seen: HashSet<String>,
}

impl Notifier {
    /// Turned on only inside a herdr pane, which herdr marks with `HERDR_ENV=1` and a
    /// pane ID.
    pub fn from_env() -> Self {
        Self::new(
            std::env::var_os("HERDR_ENV"),
            std::env::var_os("HERDR_PANE_ID"),
            std::env::var_os("HERDR_BIN_PATH"),
        )
    }

    fn new(env: Option<OsString>, pane: Option<OsString>, bin: Option<OsString>) -> Self {
        let inside = env.as_deref() == Some("1".as_ref()) && pane.is_some_and(|p| !p.is_empty());
        let bin = inside.then(|| {
            bin.filter(|b| !b.is_empty())
                .unwrap_or_else(|| "herdr".into())
        });
        Self {
            bin,
            seen: HashSet::new(),
        }
    }

    /// Toasts the alerts not seen before, and forgets the ones no longer active.
    pub fn alerts(&mut self, alerts: &[Alert]) {
        let Some(bin) = self.bin.clone() else { return };
        for a in self.fresh(alerts) {
            let mut cmd = Command::new(&bin);
            cmd.args(["notification", "show", &format!("⚠ {}{}", a.event, alert_until(a))])
                .args(["--body", &body(a)])
                .args(["--sound", sound(a.severity)])
                // Keep herdr's output off the TUI.
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            // Wait on a thread so the child is reaped without holding up the UI. A failed
            // toast isn't worth reporting: the banner still shows the alert.
            thread::spawn(move || {
                let _ = cmd.status();
            });
        }
    }

    /// The alerts not yet shown, marking them shown.
    fn fresh<'a>(&mut self, alerts: &'a [Alert]) -> Vec<&'a Alert> {
        self.seen.retain(|k| alerts.iter().any(|a| &key(a) == k));
        alerts.iter().filter(|a| self.seen.insert(key(a))).collect()
    }
}

/// The NWS ID, or the alert's wording if it came without one.
fn key(a: &Alert) -> String {
    if a.id.is_empty() {
        format!("{}\n{}", a.event, a.headline.as_deref().unwrap_or(""))
    } else {
        a.id.clone()
    }
}

/// Longest toast body, in characters; the alerts popup has the full text.
const BODY_MAX: usize = 240;

/// The description's first paragraph says what's happening ("Localized dense fog…"),
/// where the headline often only names the office that sent it.
fn body(a: &Alert) -> String {
    let first = a.description.split("\n\n").map(str::trim).find(|p| !p.is_empty());
    let text = match first {
        Some(p) => p.split_whitespace().collect::<Vec<_>>().join(" "),
        None => return a.headline.clone().unwrap_or_default(),
    };
    if text.chars().count() <= BODY_MAX {
        return text;
    }
    let cut: String = text.chars().take(BODY_MAX).collect();
    format!("{}…", cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head))
}

/// Only the alerts that call for action make a sound.
fn sound(s: Severity) -> &'static str {
    match s {
        Severity::Extreme | Severity::Severe => "request",
        _ => "none",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alert(id: &str) -> Alert {
        Alert {
            id: id.into(),
            event: "Dense Fog Advisory".into(),
            severity: Severity::Minor,
            headline: None,
            description: String::new(),
            instruction: None,
            area: String::new(),
            until: None,
            sent: None,
        }
    }

    #[test]
    fn only_inside_herdr() {
        let on = |env: Option<&str>, pane: Option<&str>| {
            Notifier::new(env.map(Into::into), pane.map(Into::into), None)
                .bin
                .is_some()
        };
        assert!(on(Some("1"), Some("wA:pPH")));
        assert!(!on(None, None));
        assert!(!on(Some("1"), None));
        assert!(!on(Some("1"), Some("")));
        assert!(!on(Some("0"), Some("wA:pPH")));
        assert!(!on(None, Some("wA:pPH")));
    }

    #[test]
    fn herdr_binary_from_env_or_path() {
        let bin = |b: Option<&str>| {
            Notifier::new(Some("1".into()), Some("p".into()), b.map(Into::into)).bin
        };
        assert_eq!(bin(Some("/opt/herdr")), Some("/opt/herdr".into()));
        assert_eq!(bin(None), Some("herdr".into()));
        assert_eq!(bin(Some("")), Some("herdr".into()));
    }

    #[test]
    fn each_alert_once_while_active() {
        let mut n = Notifier::new(None, None, None);
        let ids = |v: Vec<&Alert>| v.iter().map(|a| a.id.clone()).collect::<Vec<_>>();
        let (a, b) = (alert("a"), alert("b"));
        let (ab, just_a, just_b) = ([a.clone(), b.clone()], [a.clone()], [b.clone()]);
        assert_eq!(ids(n.fresh(&just_a)), ["a"]);
        assert!(n.fresh(&just_a).is_empty());
        assert_eq!(ids(n.fresh(&ab)), ["b"]);
        // Once "a" expires it's forgotten, so a reissue under the same ID shows again.
        assert!(n.fresh(&just_b).is_empty());
        assert_eq!(ids(n.fresh(&ab)), ["a"]);
    }

    #[test]
    fn body_is_the_first_paragraph() {
        let mut a = alert("a");
        a.headline = Some("Special Weather Statement issued by NWS".into());
        a.description = "Localized dense fog has\nformed.\n\nMotorists should use caution.".into();
        assert_eq!(body(&a), "Localized dense fog has formed.");
        a.description = "word ".repeat(100);
        let b = body(&a);
        assert!(b.ends_with("word…") && b.chars().count() <= BODY_MAX + 1);
        a.description.clear();
        assert_eq!(body(&a), "Special Weather Statement issued by NWS");
    }
}
