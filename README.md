# skycast

![skycast showing Miami: today's forecast, the radar, sun and moon, and tides](https://raw.githubusercontent.com/tlpinney/skycast/main/docs/demo.gif)

A terminal weather widget built with [ratatui](https://ratatui.rs), sized to sit in a
terminal pane alongside your work.

## Install

### Homebrew (macOS and Linux)

```sh
brew tap tlpinney/tap
brew trust --formula tlpinney/tap/skycast
brew install tlpinney/tap/skycast
```

Homebrew asks you to trust formulas from taps outside Homebrew itself;
`brew trust --formula` trusts just this one. On macOS it installs the release's universal
binary, signed with an Apple Developer ID and notarized; on Linux x86-64, the release
binary; on Linux ARM it builds from source (Homebrew installs Rust for the build). Update
with `brew upgrade skycast`.

### Cargo

From [crates.io](https://crates.io/crates/skycast), with Rust 1.89 or newer:

```sh
cargo install --locked skycast
```

`--locked` builds with the dependency versions skycast was tested with. Run the same
command again to update.

### Prebuilt binaries

Linux x86-64 and macOS (one universal binary for Apple silicon and Intel, signed with an
Apple Developer ID and notarized) are on the
[releases page](https://github.com/tlpinney/skycast/releases), with a signed checksum
file. Each release's notes show how to verify them.

## Usage

```sh
skycast                                 # your saved location; asks the first time
skycast -l "Paris"                      # any place name, just for this run
skycast -l "Portland, OR"               # "City, ST" or "City, Country"
skycast --lat 38.97 --lon -77.39 --metric
```

The first time, skycast asks where you are: type a city, pick it from the matches, and it's
saved in `~/.config/skycast/config.json`. Press `l` to search for a different place.
`--location` and `--lat`/`--lon` don't change the saved one.

From a checkout, `cargo run --release -- <options>` does the same.

Keys:

| Key | Does |
| --- | --- |
| `Tab` / `←` `→` | switch screens (or `1` Today, `2` 7 days, `3` Radar, `4` Sun, `5` Tides) |
| `l` or `/` | search for a location (↑↓ to pick, Enter to choose, Esc to close) |
| `a` | weather alert details |
| `r` | refresh |
| `q` / `Esc` | quit |
| `+` / `-` | radar: zoom in / out (150–2400 km across) |
| `space` | radar: play / pause the last two hours |
| `,` / `.` | radar: step back / forward one scan |

The forecast refreshes on its own every 15 minutes, the radar every 5.

## Screens

Five screens:

- **Today**: summary, current conditions, air quality, rain timing, temperature and
  precipitation charts (when the pane is tall enough) and an hourly strip to midnight.
- **7 days**: one row per day with a high/low range bar, plus a highs-and-lows bar chart.
- **Radar**: the last two hours of precipitation radar drawn in Braille over coastlines and
  state/country borders, with zoom and animation.
- **Sun**: sunrise and sunset, day length and its change since yesterday, solar noon, golden
  hour and the three twilights; the moon's phase, moonrise and moonset, and the next new and
  full moon; the sun's and moon's paths through today and daylight through the year.
- **Tides**: the current level and whether it's rising or falling, the next high and low,
  a two-day tide curve and a high/low table.

Active US weather alerts show as a banner on every screen.

Short panes get compact layouts: under 18 rows the Today screen becomes a dense summary,
and at 1–3 rows the whole widget becomes a strip with no header or footer, e.g.

```
 ☾ 63° · H 73° L 44° · AQI 54 · Clear sky · Herndon
 now ☾63°  7p ☾62°  8p ☾61°  9p ☾59°  10p ☾58°  11p ☾58°
```

## Data sources and attribution

None of these need an API key. In skycast, `?` lists them with their credits, and the
footer credits the source of the screen you're on.

| Data | Source | Credit and terms |
|---|---|---|
| Forecast, place search | [Open-Meteo](https://open-meteo.com) | Weather data by Open-Meteo.com, [CC BY 4.0](https://open-meteo.com/en/licence) |
| Air quality | [Copernicus Atmosphere Monitoring Service](https://atmosphere.copernicus.eu), via Open-Meteo | Contains modified Copernicus Atmosphere Monitoring Service information |
| Tides, outside the US | [Copernicus Marine Service](https://marine.copernicus.eu), via the [Open-Meteo marine API](https://open-meteo.com/en/docs/marine-weather-api) | Generated using E.U. Copernicus Marine Service Information |
| Tides, US coasts | [NOAA Tides & Currents](https://tidesandcurrents.noaa.gov) (CO-OPS) | US public domain |
| Weather alerts, US | [National Weather Service](https://www.weather.gov) | US public domain |
| Radar | [RainViewer](https://www.rainviewer.com) | RainViewer's API is free for personal and educational use |
| Map outlines | [Natural Earth](https://www.naturalearthdata.com) 1:50m | Public domain |
| Sun and moon | Computed locally (NOAA solar equations; Meeus, *Astronomical Algorithms*) | No network |

Tides come from the nearest NOAA station within 80 km, otherwise from the nearest sea
point in Open-Meteo's marine model within 50 km. NOAA's station list is cached in
`~/.cache/skycast/` for a month. The map outlines are converted to `assets/*.bin` by
`tools/ne2bin.py`.

## License

MIT. See [LICENSE](LICENSE).
