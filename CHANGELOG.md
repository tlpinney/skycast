# Changelog

All notable changes to skycast are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0-alpha.2] - 2026-11-03

The second alpha. From crates.io:

```sh
cargo install skycast --version 0.1.0-alpha.2
```

### Added

- A location search: type a city and pick from the matches (`l` or `/` anytime). The
  choice is saved in `~/.config/skycast/config.json` and used next time.
- A demo GIF in the README (`docs/demo.tape`, recorded with vhs).

### Changed

- No built-in default location: the first run asks where you are. `--location` and
  `--lat`/`--lon` still work, for that run only; a name that finds nothing opens the
  search with it filled in.
- Radar colors follow the most common precipitation level instead of the heaviest. One
  heavy pixel no longer colors a whole dot, and one heavy dot no longer colors a whole
  terminal cell, so moderate showers stop looking like downpours when zoomed out.
- Releases go to crates.io from the release workflow (trusted publishing, after the
  maintainer approves), alongside the GitHub release.

## [0.1.0-alpha.1] - 2026-11-03

The first alpha, for private testing. Not published to crates.io; install from git:

```sh
cargo install --git https://github.com/tlpinney/skycast --tag v0.1.0-alpha.1
```

### Screens

- **Today**: current conditions, high and low, rain timing ("Rain likely 3–6p"), air
  quality, wind direction and gusts, pressure and clouds; temperature and precipitation
  charts with a daylight bar; an hourly strip.
- **7 days**: one row per day with a high/low range bar, and a highs-and-lows bar chart.
- **Radar**: the last two hours from RainViewer in Braille over Natural Earth coastlines
  and borders, with zoom and animation.
- **Sun**: sunrise and sunset, day length, solar noon, golden hour and twilights; moon
  phase, moonrise and moonset; the sun's and moon's paths today and daylight through the
  year. Computed locally.
- **Tides**: NOAA station predictions on US coasts, Open-Meteo's marine model elsewhere.

### Also

- US weather alerts from the National Weather Service as a banner, with details on `a`.
- Compact layouts for short panes, down to a one-line strip.
- Retries for temporary network failures.
- Data from Open-Meteo, the National Weather Service, RainViewer and NOAA; no API keys.

[Unreleased]: https://github.com/tlpinney/skycast/compare/v0.1.0-alpha.2...HEAD
[0.1.0-alpha.2]: https://github.com/tlpinney/skycast/compare/v0.1.0-alpha.1...v0.1.0-alpha.2
[0.1.0-alpha.1]: https://github.com/tlpinney/skycast/releases/tag/v0.1.0-alpha.1
