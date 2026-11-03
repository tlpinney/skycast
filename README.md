# weather

Two things live here:

- **`weather` (Rust, ratatui)**: a terminal weather widget meant to sit in a herdr pane.
  Three screens:
  - **Today**: summary, current conditions, air quality, rain timing, temperature and
    precipitation charts (when the pane is tall enough) and an hourly strip to midnight.
  - **7 days**: one row per day with a high/low range bar, plus a highs-and-lows bar chart.
  - **Radar**: the last two hours of precipitation radar drawn in Braille over coastlines and
    state/country borders, with zoom and animation.

  Active US weather alerts show as a banner on every screen.
- **`weather.py` (Python)**: an MCP server that exposes `get_forecast`.

Data sources (none need an API key):

- Forecast, geocoding and air quality: [Open-Meteo](https://open-meteo.com/)
- Weather alerts (US only): [National Weather Service](https://www.weather.gov/documentation/services-web-api)
- Radar: [RainViewer](https://www.rainviewer.com/api.html)
- Map outlines: [Natural Earth](https://www.naturalearthdata.com/) 1:50m (public domain), converted
  to `assets/*.bin` by `tools/ne2bin.py`

## Running the widget

```sh
cargo run --release                         # Herndon, VA, °F
cargo run --release -- -l "Paris"           # any place name
cargo run --release -- -l "Portland, OR"    # "City, ST" or "City, Country"
cargo run --release -- --lat 38.97 --lon -77.39 --metric
```

Keys:

| Key | Does |
| --- | --- |
| `Tab` / `←` `→` | switch screens (or `1` Today, `2` 7 days, `3` Radar) |
| `a` | weather alert details |
| `r` | refresh |
| `q` / `Esc` | quit |
| `+` / `-` | radar: zoom in / out (150–2400 km across) |
| `space` | radar: play / pause the last two hours |
| `,` / `.` | radar: step back / forward one scan |

The forecast refreshes on its own every 15 minutes, the radar every 5.

## License

MIT. See [LICENSE](LICENSE).
