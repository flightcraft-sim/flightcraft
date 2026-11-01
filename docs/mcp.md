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
| `commands_list` | `commands.list` | `{}`. Returns `commands`, including dotted names, descriptions, and both schemas. |

Units are SI and appear in field names: seconds (`time_s`), metres (`height_m`, `position_m`),
metres per second (`airspeed_mps`, `velocity_mps`), radians (`heading_rad`), and radians per second
(`angular_rate_rps`). Controls, quaternions, playback rate, and load factor are dimensionless.
The existing readout field `engine_rpm` reports revolutions per minute. Air starts require
nonnegative height and airspeed, and heading between 0 and 2π radians inclusive. Unknown input
fields, non-finite numbers, and out-of-range values are errors. Optional values generally reject
null; hold targets explicitly accept null or false to disengage.

Each connection owns one session, initially paused on the runway at rate 1. Reset preserves rate
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

Each wait condition has a `kind` and the fields below. Comparisons are inclusive, heading uses
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

Run `cargo xtask schemas` after changing command types to regenerate `schema/commands.json`.
`cargo xtask schemas --check` detects stale schemas and runs as part of `cargo xtask ci`.
CI also runs `cargo xtask determinism`, which rejects standard float transcendental calls in
the world, aero, and sim source and allows `libm` calls and test modules.
