//! The saved location, in `~/.config/skycast/config.json` (or `$XDG_CONFIG_HOME/skycast/`).
//! Choosing a place in the search saves it; `--location` and `--lat`/`--lon` don't.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::api::Place;

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub location: Option<Place>,
}

pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("skycast").join("config.json"))
}

/// The saved config, or the default when there's none or it can't be read.
pub fn load() -> Config {
    path().map(|p| load_from(&p)).unwrap_or_default()
}

pub fn load_from(path: &Path) -> Config {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save(config: &Config) -> Result<()> {
    save_to(&path().context("no home directory for the config file")?, config)
}

/// Write then rename, so a crash mid-write never leaves a broken config.
pub fn save_to(path: &Path, config: &Config) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, serde_json::to_vec_pretty(config)?).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("saving {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_tolerates_junk() {
        let dir = std::env::temp_dir().join(format!("skycast-config-test-{}", std::process::id()));
        let path = dir.join("config.json");
        assert_eq!(load_from(&path), Config::default());

        let config = Config { location: Some(Place { name: "Herndon, Virginia".into(), lat: 38.97, lon: -77.39 }) };
        save_to(&path, &config).unwrap();
        assert_eq!(load_from(&path), config);

        std::fs::write(&path, b"not json").unwrap();
        assert_eq!(load_from(&path), Config::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
