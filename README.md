# skycast

![skycast showing Miami: today's forecast, the radar, sun and moon, and tides](https://raw.githubusercontent.com/tlpinney/skycast/main/docs/demo.gif)

A terminal weather widget built with [ratatui](https://ratatui.rs), sized to sit in a
terminal pane alongside your work. Five screens:

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

Data sources (none need an API key):

- Forecast, geocoding and air quality: [Open-Meteo](https://open-meteo.com/)
- Weather alerts (US only): [National Weather Service](https://www.weather.gov/documentation/services-web-api)
- Radar: [RainViewer](https://www.rainviewer.com/api.html)
- Tides: [NOAA CO-OPS](https://tidesandcurrents.noaa.gov/web_services_info.html) predictions from
  the nearest station within 80 km (US coasts), otherwise the
  [Open-Meteo marine model](https://open-meteo.com/en/docs/marine-weather-api) at the nearest sea
  grid point within 50 km. NOAA's station list is cached in `~/.cache/skycast/` for a month.
- Sun and moon: computed locally (NOAA solar equations, Meeus lunar longitude), no network
- Map outlines: [Natural Earth](https://www.naturalearthdata.com/) 1:50m (public domain), converted
  to `assets/*.bin` by `tools/ne2bin.py`

## Install

skycast is in alpha. From [crates.io](https://crates.io/crates/skycast) (pre-releases need
the version spelled out):

```sh
cargo install skycast --version 0.1.0-alpha.2
```

Linux binaries are on the [releases page](https://github.com/tlpinney/skycast/releases),
with a signed checksum file.

## Usage

```sh
skycast                                 # Herndon, VA, °F
skycast -l "Paris"                      # any place name
skycast -l "Portland, OR"               # "City, ST" or "City, Country"
skycast --lat 38.97 --lon -77.39 --metric
```

From a checkout, `cargo run --release -- <options>` does the same.

Keys:

| Key | Does |
| --- | --- |
| `Tab` / `←` `→` | switch screens (or `1` Today, `2` 7 days, `3` Radar, `4` Sun, `5` Tides) |
| `a` | weather alert details |
| `r` | refresh |
| `q` / `Esc` | quit |
| `+` / `-` | radar: zoom in / out (150–2400 km across) |
| `space` | radar: play / pause the last two hours |
| `,` / `.` | radar: step back / forward one scan |

The forecast refreshes on its own every 15 minutes, the radar every 5.

## License

MIT. See [LICENSE](LICENSE).
