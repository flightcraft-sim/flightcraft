# FlightCraft MCP server

Build and run the headless stdio server from the repository root:

```sh
cargo build -p flightcraft-mcp
cargo run --quiet -p flightcraft-mcp
```

The server reads one JSON-RPC 2.0 request per line and writes one response per line. Standard output
contains protocol messages only; diagnostics go to standard error. No async runtime, display, or
network service is needed. Lines are limited to 1 MiB. EOF exits cleanly.

A generic MCP client configuration looks like this; replace the command with the absolute path to
the built executable (or put `flightcraft-mcp` on your PATH):

```json
{
  "mcpServers": {
    "flightcraft": {
      "command": "flightcraft-mcp",
      "args": []
    }
  }
}
```

The client sends `initialize` with `protocolVersion`, an object `capabilities`, and `clientInfo`
containing `name` and `version`. The server negotiates protocol version `2025-06-18` and also accepts
`2025-03-26` and `2024-11-05`. Send the `notifications/initialized` notification next. Notifications
have no response. `ping` returns an empty object. `tools/list` returns all tools in one page, with
their generated input and output schemas. `tools/call` accepts `name` and an `arguments` object;
omitting `arguments` means `{}`. Successful calls return `structuredContent` and a JSON text copy
in `content`. Invalid command arguments or unknown tools return JSON-RPC error `-32602`; execution
failures return a tool result with `isError: true`. Unknown methods return `-32601`, malformed
requests `-32600`, and malformed JSON `-32700`.

| Tool | Command | Arguments and result |
|---|---|---|
| `sim_reset` | `sim.reset` | `start: {kind: "runway"}` or `{kind: "air", height_m, airspeed_mps, heading_rad}`; optional `paused` (defaults to `true`). Returns state, readout, and playback settings. |
| `sim_set_controls` | `sim.set_controls` | Any subset of `elevator`, `aileron`, `rudder` (-1 to 1), `throttle`, `flaps`, and `brake` (0 to 1). Returns all six controls. |
| `sim_step` | `sim.step` | Integer `steps` from 0 to 72000 (ten minutes at 120 Hz). Returns the readout. |
| `autopilot_set` | `autopilot.set` | Any subset of `pitch_rad`, `altitude_m`, `vertical_speed_mps`, `heading_rad`, `airspeed_mps`, `wings_level`, `bank_limit_rad`, and `airspeed_mode` (`throttle` or `pitch`). Returns all engaged holds and targets. |
| `autopilot_off` | `autopilot.off` | `{}`. Disengages all holds, leaving the last controls in place. |
| `sim_run_until` | `sim.run_until` | `conditions` array (up to 16), optional `max_steps` (0 to 72000, default 72000). Returns `state`, `readout`, actual `steps`, and `fired`. |
| `sim_state` | `sim.state` | `{}`. Returns full `state`, instrument `readout`, `paused`, and `rate`. |
| `sim_pause` | `sim.pause` | `{}`. Pauses the app's real-time loop; returns playback settings. |
| `sim_resume` | `sim.resume` | `{}`. Resumes the app's real-time loop; returns playback settings. |
| `sim_set_rate` | `sim.set_rate` | `rate` from 0.25 to 8. Returns playback settings. |
| `sim_wait` | `sim.wait` | `seconds` from 0 to 60. Waits for simulated time to advance in the running app; returns state, readout, and playback settings. App only. |
| `view_set_camera` | `view.set_camera` | `mode`: `chase` or `cockpit` (placeholder), optional chase `distance_m` (2 to 200) and `height_m` (0 to 100). Returns camera settings. App only. |
| `app_screenshot` | `app.screenshot` | Optional `directory` for a new PNG, and `base64` (default false). Direct calls require a directory or base64. MCP always requests base64 and returns PNG image content, plus `path` and `png_base64` in structured content (`path` is null without a directory). App only. |
| `commands_list` | `commands.list` | `{}`. Returns `commands`, including dotted names, descriptions, and both schemas. |

Units are SI and appear in field names: seconds (`time_s`), metres (`height_m`, `position_m`),
metres per second (`airspeed_mps`, `velocity_mps`), radians (`heading_rad`), and radians per second
(`angular_rate_rps`). Controls, quaternions, playback rate, and load factor are dimensionless.
The existing readout field `engine_rpm` reports revolutions per minute. Air starts require
nonnegative height and airspeed, and heading between 0 and 2π radians inclusive. Unknown input
fields, non-finite numbers, and out-of-range values are errors. Optional values generally reject
null; hold targets explicitly accept null or false to disengage.

Each headless server owns one session, initially paused on the runway at rate 1. Reset preserves rate
and resets simulated time. Explicit stepping works while paused and always uses the fixed 1/120 s
timestep; rate and pause are settings for the app's real-time loop. This headless server has no
background stepping loop. Unspecified controls remain unchanged, and invalid calls leave the
session unchanged.

Holds run at the fixed simulation step. Omitted targets keep their previous settings. Numeric
hold bounds are pitch -0.25 to 0.35 rad, altitude 2 to 10000 m MSL, vertical speed -8 to 8 m/s,
heading 0 to 2π rad, and true airspeed 25 to 80 m/s. The bank limit defaults to 0.4 rad and accepts
0.05 to 0.6 rad. Engage one vertical hold (`pitch_rad`, `altitude_m`, or `vertical_speed_mps`)
and one lateral hold (`heading_rad` or `wings_level`) at a time; disengage conflicting holds in
the same call. Throttle-mode airspeed owns throttle and works alongside a vertical hold.
Pitch-mode airspeed owns elevator and commands full throttle to climb at the selected speed;
disengage other vertical holds first. Heading and wings level own aileron and rudder. Flaps,
brakes, and controls without an engaged owner remain with the pilot. Readouts include the
`autopilot` targets, and reset disengages all holds.

Each `sim_run_until` condition has a `kind` and the fields below. Comparisons are inclusive, heading uses
the shortest angular distance across north, and elapsed time starts at entry to each call.

| Condition kind | Fields |
|---|---|
| `altitude_above`, `altitude_below` | `altitude_m` (0 to 20000 m MSL) |
| `airspeed_above`, `airspeed_below` | `airspeed_mps` (0 to 150 m/s, true airspeed) |
| `heading_within` | `heading_rad` (0 to 2π), `tolerance_rad` (0.0001 to π) |
| `on_ground` | No additional fields; any wheel touching ground |
| `time_elapsed` | `seconds` (0 to 600), relative simulated time |

`sim_run_until` checks the initial state before stepping and stops at the first matching condition
in input order. `fired` is `{kind: "condition", index, condition}`, `{kind: "step_cap"}`, or
`{kind: "crashed"}`. A crash takes priority and stops the wait immediately. Empty `conditions`
run to the cap. A condition already satisfied on entry returns zero steps. Explicit waits work
while paused and do not change playback settings.

For example, after initialization, send these lines to advance two simulated seconds:

```jsonl
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"sim_reset","arguments":{"start":{"kind":"runway"}}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"sim_set_controls","arguments":{"throttle":0.8}}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"sim_step","arguments":{"steps":240}}}
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"sim_state","arguments":{}}}
```

The JSON command flight test takes off, climbs above 300 m AGL, flies four right turns, lands,
and stops on the runway. The MCP flight test drives the same pilot through JSON-RPC tool calls
for takeoff and a climb to 150 m AGL. Both use only JSON observations and command inputs.

A hold-based loop can wait for entire flight phases in one call. For example, from an air start,
climb to 300 m, level off, turn east, and wait for the heading:

```jsonl
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"sim_reset","arguments":{"start":{"kind":"air","height_m":150,"airspeed_mps":36,"heading_rad":0}}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"autopilot_set","arguments":{"airspeed_mps":36,"airspeed_mode":"pitch","heading_rad":0}}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"sim_run_until","arguments":{"conditions":[{"kind":"altitude_above","altitude_m":300}],"max_steps":24000}}}
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"autopilot_set","arguments":{"airspeed_mode":"throttle","altitude_m":300,"heading_rad":1.5707963267948966,"bank_limit_rad":0.35}}}
{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"sim_run_until","arguments":{"conditions":[{"kind":"heading_within","heading_rad":1.5707963267948966,"tolerance_rad":0.03}],"max_steps":12000}}}
```

Check `fired` and the returned readout, then choose the next hold targets and wait condition.
`crates/control/tests/pattern_holds.rs` flies takeoff, the climb above 300 m, four turns, descent,
landing, and the stop on the runway in fewer than 60 JSON command calls.

## Drive the running desktop

Start the desktop and connect the stdio MCP server to the same port:

```sh
flightcraft --control 7890
flightcraft-mcp --connect 7890
```

Use this client configuration (with an absolute executable path when needed):

```json
{
  "mcpServers": {
    "flightcraft": {
      "command": "flightcraft-mcp",
      "args": ["--connect", "7890"]
    }
  }
}
```

Bridged and headless modes expose the same tool list and schemas. Every bridged tool call goes to
one session owned by the desktop UI thread, including state reads. Disconnecting leaves that flight
running; reconnecting observes the same session. The desktop starts running, whereas a headless
server starts paused. App-only tools return a clear "needs the app" error in headless mode.

The control socket binds exclusively to IPv4 `127.0.0.1`; flags accept only a numeric port from
1 to 65535, never a host or address. The listener also rejects any other bind address. The channel
is available to local processes and has no authentication. Native networking is compiled out on
wasm. No async runtime is used.

Direct clients send one JSON object per line:

```jsonl
{"id":1,"command":"sim.pause","input":{}}
{"id":2,"command":"view.set_camera","input":{"mode":"chase","distance_m":25,"height_m":9}}
{"id":3,"command":"app.screenshot","input":{"directory":"captures","base64":false}}
```

A response is `{"id":1,"ok":{...}}` or `{"id":1,"error":"message"}`. IDs are strings,
numbers, or null. Unknown envelope fields, missing fields, malformed JSON, invalid UTF-8, and
oversized lines receive errors; unusable IDs become null. Request lines are capped at 1 MiB,
including the newline. Oversized lines are drained before the next request. At most eight clients
connect at once, each with one outstanding request; the channel queue and pending app work are also
bounded. Commands execute at the start of a UI frame through `Session::execute`. Threads move
lines and responses through standard channels and never access the simulation. Calls on one
connection are ordered; a waiting client can be interrupted by another client's pause or reset.
The synchronous MCP stdio server handles one call at a time.

### Real-time rules

- In the desktop, `sim.step` and `sim.run_until` require pause. While running they fail with an
  instruction to use `sim.pause` first or `sim.wait` for real-time flight. Scripts use the same rule.
  Explicit stepping while paused still updates the visible flight on the following render.
- `sim.wait` waits for 0 to 60 simulated seconds to pass in the app's frame loop. It does no explicit
  stepping and requires a running session. `sim.set_rate` scales that loop while preserving fixed
  steps. Pause, reset, a crash, or a 120-second wall-clock deadline interrupts an outstanding wait with an
  error. The response contains the state observed when the target time is reached, which can be
  slightly later than requested because frames contain several fixed steps.
- Keyboard and gamepad flight controls are suppressed while a script has commands remaining,
  while any hold is engaged, or for three seconds after a successful external `sim.set_controls`,
  `autopilot.set`, `autopilot.off`, or `sim.reset`. State reads and waits do not renew this interval.
  Keep sending controls within three seconds for manual remote flight, or engage holds. Human
  pause, reset, and camera actions remain available.
- `app.screenshot` requests an eframe viewport capture and replies over the next frames when its
  matching renderer event arrives. It captures the window, including instruments, and keeps the
  app open. A requested directory is created if necessary; a fresh PNG is created there without
  overwriting files, and the returned path is absolute. PNGs are capped at 16 MiB. Base64-only
  capture writes no file. Screenshot requests time out after 120 seconds if no image arrives.
- `view.set_camera` controls chase or the current cockpit placeholder. Chase defaults are
  `distance_m: 19` and `height_m: 7`; omitted offsets retain their settings. Camera buttons and
  shortcuts call this same command.

For visible real-time flight, set controls or holds, call `sim_resume`, then `sim_wait` and
`sim_state`. For deterministic explicit flight, call `sim_pause`, then use `sim_step` or
`sim_run_until`. To see the result, call `app_screenshot` with `{"directory":"captures"}`;
its MCP response includes both an image block and the saved path.

Run `cargo xtask schemas` after changing command types to regenerate `schema/commands.json`.
`cargo xtask schemas --check` detects stale schemas and runs as part of `cargo xtask ci`.
CI also runs `cargo xtask determinism`, which rejects standard float transcendental calls in
the world, aero, and sim source and allows `libm` calls and test modules.

## Airspace

| Tool | Command | Arguments and result |
|---|---|---|
| `airspace_list` | `airspace.list` | `{}`. Returns `attribution` and `volumes` with `id`, `name`, `class`, `kind`, optional `sector`, `floor`, and `ceiling`. Mode C has a null class. No polygon payloads. |
| `airspace_at` | `airspace.at` | Supply all of `north_m`, `east_m`, and `altitude_msl_m`, or omit all three for the aircraft. Returns every `containing` volume and `nearest`, the nearest B/C shelf above or alongside the point. Partial coordinates and nulls are errors. |
| `view_set_airspace` | `view.set_airspace` | Required `visible` boolean. Returns `visible`. Controls the app's curtains and outlines; works headlessly without dialogs. Default is true. |

`nearest` is null when no B/C entry boundary is available. Otherwise it contains `id`, `class`,
`lateral_distance_m` (zero under the shelf), `floor_msl_m`, `floor_above_m` (nonnegative height to the
floor), and `warning`. The entry warning activates within 1 nm at shelf altitude or within 300 ft
below a floor when underneath or within 1 nm of its footprint. The nearest shelf minimises combined
lateral and vertical distance, breaking ties by the lower floor. Queries exclude shelves already
containing the point and shelves entirely below it from this entry calculation. All containing
volumes, including Mode C veils, remain in `containing`.

When the active region has an airspace sidecar, `sim.state` and reset results also include `airspace`,
an array of `{id, class}` memberships, empty when outside every loaded volume. The field is omitted
for worlds without airspace. SFC floors follow terrain height; ceilings are exclusive. STD limits
use nominal MSL height. See [the data and rendering reference](airspace.md) for dataset coverage,
limits, attribution, and offline conversion instructions.
## Geographic regions and airports

`world_geodetic` (`world.geodetic`) accepts either `{"north_m":0,"east_m":0}` or
`{"latitude_deg":40.97,"longitude_deg":-75.0}` in the active region. It returns `north_m`,
`east_m`, `latitude_deg`, and `longitude_deg`. Built-in worlds have no geodetic origin and return
an error. Local coordinates are bounded to ±250 km.

`world_airports` (`world.airports`) accepts `{}` or `{"nearest_to_m":[0,0]}`. It returns the active
region's airport catalogue and its attribution, optionally sorted by distance from a local
position. Airports contain runway dimensions, surfaces, and endpoint coordinates, elevations,
and true headings. Built-in worlds return an empty catalogue.

`sim_reset` also accepts `{"world":"new-york","start":{"runway":{"airport":"1N7","end":"07"}}}`.
This parks at the selected threshold and aligns with its centreline. For an airport-relative air
start, use `{"world":"new-york","start":{"air":{"airport":"JFK","distance_m":5000,
"bearing_deg":220,"height_m":600,"airspeed_mps":45,"heading_rad":0.7}}}`. Distance is horizontal,
bearing is true degrees clockwise from north, height is metres above airport elevation, and
heading is a local grid heading in radians. Airport identifiers may be FAA or ICAO idents.
Existing `kind` starts remain supported, including the air start's absolute MSL altitude.
Invalid starts leave the active flight intact. `sim_state` and its readout include optional
`latitude_deg` and `longitude_deg`, null without an active geographic position. The state output
also includes optional `altitude_msl_m`; the readout retains its always-available MSL altitude.
See [terrain and data import](terrain.md) for catalogue schemas, provenance, and offline loading.

The same commands support `eastern-pa` (Eastern Pennsylvania and New Jersey), including
`KABE`, `KMPO`, `KAVP`, and `KRDG`. Both regions share the Blairstown runway 07 origin.
