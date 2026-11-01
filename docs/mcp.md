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
fields, null optional values, non-finite numbers, and out-of-range values are errors.

Each connection owns one session, initially paused on the runway at rate 1. Reset preserves rate
and resets simulated time. Explicit stepping works while paused and always uses the fixed 1/120 s
timestep; rate and pause are settings for the app's real-time loop. This headless server has no
background stepping loop. Unspecified controls remain unchanged, and invalid calls leave the
session unchanged.

For example, after initialization, send these lines to advance two simulated seconds:

```jsonl
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"sim_reset","arguments":{"start":{"kind":"runway"}}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"sim_set_controls","arguments":{"throttle":0.8}}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"sim_step","arguments":{"steps":240}}}
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"sim_state","arguments":{}}}
```

The current simulation stub advances time only. Flight-quality checks, including the ignored
traffic-pattern test, await the flight model.

Run `cargo xtask schemas` after changing command types to regenerate `schema/commands.json`.
`cargo xtask schemas --check` detects stale schemas and runs as part of `cargo xtask ci`.
CI also runs `cargo xtask determinism`, which rejects standard float transcendental calls in
the world, aero, and sim source and allows `libm` calls and test modules.
