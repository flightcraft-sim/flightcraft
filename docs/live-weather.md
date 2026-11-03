# Live station weather

Open **Weather → Live**, enter an airport or choose one from the loaded region, and choose a
source. **Test / refresh** makes a background GET and shows its exact matched text, parsed
values, source, UTC observation time, and age. **Use now** applies a fresh observation once.
**Follow** refreshes at the source's interval and blends wind, pressure, and cloud heights over
one minute of simulation time. **Stop following** leaves the current conditions in place.
Load the airport's region before applying station temperatures or AGL cloud heights: conversion
requires the airport elevation. METAR winds are true; the panel labels them as such. AWOS uses
station WMM magnetic declination at the simulation date. An unavailable direction is spoken as
unavailable, rather than invented.

1N7 has no NWS-disseminated METAR, including under K1N7. Its built-in source is the local
[Belfort DigiWx Advisor](http://www.digiwx-1n7.com/Default.aspx), which is **advisory only, not
for flight planning**. The station serves HTTP; HTTPS is unavailable. FlightCraft displays an
unencrypted-connection warning and the advisory disclaimer with its observations and speech.
The pinned NASR cycle has no AWOS/ASOS channel for 1N7; 123.000 is CTAF/UNICOM.
The app provides the 1N7 weather-by-phone transcript without inventing a radio channel.
No third-party page is bundled. The worked example is
[1n7-digiwx.json](../examples/weather-sources/1n7-digiwx.json).

## Nearby official observations and cross-checks

**Find / compare nearest official reporting stations** queries the Aviation Weather Center's
METAR API for actual reporting stations around the selected airport. Testing a source also
starts this comparison when the airport's region is loaded. The four nearest reports within
150 nm are offered with their own identifier, distance in nautical miles, and compass direction.
For 1N7 these normally include KFWN, K12N, KMPO, and KSMQ. Availability depends on current reports.
The newest report from each station is used. They always remain the reporting station's weather; selecting one never relabels it as 1N7.
The lookup uses the METAR bounding-box API, without depending on the stationinfo endpoint.

The comparison table shows local and nearby altimeters, true winds, ceilings AGL, and ages.
It flags a local altimeter more than **0.05 inHg** from the mean of fresh nearby altimeters,
and a local observation more than **60 minutes** older than the newest fresh nearby report.
Unknown fields and absent timestamps do not become invented comparisons. These checks indicate
disagreement; they do not determine which instrument is correct. Local terrain, fronts, and
station faults can all cause differences. Nearby weather does not establish conditions at a field.

## Add a local endpoint

**Add a source…** prefills a name from the airport. Enter the URL and choose a preset, or a
METAR element id or regex. You can edit the name. Enter an
advisory disclaimer, opt into HTTP if necessary, and press **Test**. Inspect the exact matched
text and parsed values. **Save tested source** becomes available only after these exact settings
have parsed successfully. Changing settings requires a new test. Saving creates a new file;
it never replaces an existing file. Edit existing source files directly, then restart the app.

Native source files live in `weather-sources/*.json` under the user configuration directory:

- Linux: `$XDG_CONFIG_HOME/flightcraft`, or `~/.config/flightcraft`.
- macOS: `~/Library/Application Support/FlightCraft`.
- Windows: `%APPDATA%/FlightCraft`.

The browser stores user sources in local storage. It fetches only sources marked `cors: true`,
and the server must send an appropriate `Access-Control-Allow-Origin` header. Browsers also
block HTTP from an HTTPS application. A failed browser request explains that the source works
in the desktop app. There is no proxy. Browser redirects are rejected because Fetch hides the
redirect count; use the final direct URL. Browsers set their own User-Agent.

For a page like DigiWx, `extraction: {"kind":"digiwx"}` selects `MetarLine` and supplements
missing values from `lblWind`, `lblGusts`, `lblAltimeter`, `lblVIZ`, `lblCeilometer`,
`lblTemperature`, `lblDewPoint`, and `lblIndicatedRunway`; `lblDate` and `lblTime` establish age.
The `metar_text` preset reads the whole body. `aviationweather` reads `/0/rawOb` from the
HTTPS API's JSON array. For a loaded airport with an ICAO identifier, this is the default
source; an empty response means that identifier has no current official report.

Advanced configurations follow [weather-source.json](../schema/weather-source.json), with
`format_version: 1`. Required metadata includes `id`, `name`, `airport`, `url`, `allow_http`,
`refresh_seconds` (60–86,400), `timeout_seconds` (1–120), `advisory_only`, and `disclaimer`.
URLs must be HTTP or HTTPS, without credentials or fragments. HTTP requires `allow_http: true`.
Advisory-only sources require nonempty disclaimer text. Use `cors: true` only for a server that
supports browser requests.

A custom METAR extraction can also supplement selected fields:

```json
"extraction": {
  "kind": "metar",
  "selector": {"kind":"element_id","element_id":"current-report"},
  "fields": {
    "runway_in_use": {"kind":"css_like","css_like":".runway.current"}
  }
}
```

Selectors support `element_id`, `css_like` (`#id`, `.class`, or combinations on one element),
`regex` (exactly one capture group), `body`, and `json_pointer`. There are no tag selectors,
combinators, arbitrary scripts, or XPath expressions. Regex patterns are limited to 2,048 bytes,
a 256 KiB compiled expression, and bounded nesting; Rust's regex engine has linear matching time.
Matches are limited to 16 KiB. Source files are limited to 64 KiB, and response bodies to 1 MiB.

A `fields` extraction contains the same `fields` object without a METAR selector. Supported keys
are `wind`, `gust`, `altimeter`, `visibility`, `layers`, `temperature`, `dew_point`,
`ceiling`, `runway_in_use`, `date`, and `time`. Wind accepts METAR syntax or `270° @ 8 KT` (also MPS or
M/S). Altimeters accept inHg or hPa, visibility accepts SM fractions, meters, or km, layers use
METAR coverage/height groups or CLR/SKC, ceilings accept feet or meters, and temperatures accept Celsius or Fahrenheit.
Wind directions in these source fields are true; convert a provider's magnetic wind before using
this format. Pressure, speed, and height units displayed to pilots are inHg, knots, and feet.
Missing fields supplement rather than overwrite a parsed METAR. Unknown fields remain unknown. Variable winds retain the simulator's current mean direction.
Live gusts are displayed and spoken; the simulator retains its existing turbulence and shear
settings.

## Failure behavior, records, and privacy

Requests have timeouts, a 1 MiB body cap, and at most three native redirects. HTTP 5xx, including
503 and 504, are retryable failures. Failures use exponential backoff, capped at an hour (or a
longer configured refresh interval). Tests respect the same gate. Last-good data remains visible
with its age and the fetch error. Reports older than 90 minutes are stale and cannot be applied
automatically or through Use now. A report without an observation timestamp clearly uses age
since retrieval. A timestamp in the future is rejected.

Only GET requests to the configured URL and redirects are sent: no form postback, cookies,
credentials, aircraft position, flight plan, or simulation state. Native requests identify as
`FlightCraft/<version> (+https://github.com/flightcraft-sim/flightcraft)`. The server sees your IP
address, the supplied URL, and normal HTTP headers. Finding nearby reports sends a bounding box
around the **selected airport** to aviationweather.gov. It does not send the aircraft's location.
HTTP contents can be observed or modified in transit. Keep private tokens out of shareable source
files: URLs and matched observation text can appear in source tests, screen snapshots, and records.
No page scripts run and no linked resources are fetched.

The flight record stores each applied observation, source metadata, resolved grid wind, MSL
conditions, initial conditions, simulation time, and transition duration. Stopping halfway through a blend
records the frozen conditions as a new application, preserving the exact earlier interpolation. `weather.live`'s `replay`
action installs that recorded transition without contacting a source. Replay clients submit these
records in time order at their recorded simulation times, alongside recorded flight controls.
Live weather is disabled until explicitly selected, so golden simulations never access a network.
This is simulator weather, never a substitute for an official briefing or flight-planning source.

## Commands and previews

All commands have input and output schemas in [commands.json](../schema/commands.json):

- `weather.sources`: list sources and test results; optional `airport` filters and includes nearby reports.
- `weather.source_test`: supply exactly one of `source_id` or a complete `source`; starts a background
  test. For MCP clients, poll weather.source_test until last_good is set, using the same input.
  While `fetching` is true, wait before saving or applying. Check `error` if the test fails;
  `weather.sources` also exposes test status, matched text, and parsed fields.
- `weather.source_save`: save an exactly matching successfully tested `source`.
- `weather.nearby`: start nearby discovery for a loaded `airport`; poll `weather.sources`.
- `weather.live`: `{"action":"use_now","source_id":"1n7-digiwx"}`, `follow`, `stop`, or
  `{"action":"replay","record":{...}}` with a recorded application.
- `weather.phone`: `{"airport":"1N7"}` returns the AWOS/phone transcript without making a call.

The active NASR AWOS/ASOS frequency uses the same live observation and shared phrase renderer,
including gusts, observation time, missing fields, stale status, and advisory disclaimer. It uses
the existing radio voice and FAA airport name (for example, “Blairstown automated weather
observation”). Spoken wind directions are magnetic, rounded to the nearest ten degrees;
the parsed observation retains its exact true direction. The phone transcript is also visible in the Live panel.

To open the 1N7 capture in the desktop app, click **Weather → Live…** in the top menu.
The panel defaults to airport **1N7** and source **Blairstown DigiWx Advisor**. If needed,
enter `1N7` in **Airport**, and select that source. Click **Test / refresh**, then wait for
**Fetching…** to finish. **Use now**, **Follow**, and **Save tested source** stay disabled
during their source's fetch. The test also starts the nearby comparison; wait for its
**Finding nearby official METARs…** message to finish, or click **Find / compare nearest
official reporting stations**. Capture the current time and age, advisory and HTTP warnings,
parsed values, nearby table, and the **AWOS / weather-by-phone transcript** (scroll if needed).

The ignored `render_live_weather_fixture_preview` GPU test generates an original fixture preview
under `out/live-weather/`; it does not claim to show current weather. The ignored
`render_live_weather_network_preview` test fetches DigiWx for a current offscreen capture when
network access is available. Neither runs in CI. Real-window shots to capture: **Weather → Live**
with 1N7 DigiWx selected, current time/age, advisory/HTTP warnings, and nearby comparison; and the
1N7 weather-by-phone transcript in the panel. For radio, capture nearby KFWN ASOS at its
published NASR 118.525 channel after testing KFWN's official source.
