# Roadmap

Ranked: work goes roughly top to bottom. Each item names what it depends on. The specification is
`docs/SPEC.md`.

## Now: M1 First flight

| # | Item | Depends on |
|---|---|---|
| 1 | Workspace, licences, CI, licence check (`cargo deny`), layer check (done) | nothing |
| 2 | ISA atmosphere and flat world with one runway (`world`) (done) | 1 |
| 3 | Corvid aircraft definition and coefficient tables (`aero`) (done) | 1 |
| 4 | 6-DOF rigid body, integrator, gear contact (`sim`) (done) | 2, 3 |
| 5 | Golden test: scripted takeoff, climb, and landing with a fixed trajectory hash (done) | 4 |
| 6 | Command registry with schemas and the session (`control`) (done) | 4 |
| 7 | Renderer: ground, runway, aircraft, chase camera (`render`) (done) | 4 |
| 8 | Desktop app: keyboard and gamepad, basic instruments, configurable shortcuts, theme tokens (done) | 6, 7 |
| 9 | Browser build of the same app (done) | 8 |

## Next: M2 Agent pilot

| # | Item | Depends on |
|---|---|---|
| 10 | Headless MCP server over stdio (`apps/mcp`) (done) | 6 |
| 11 | Agent test: a scripted client flies the traffic pattern and lands (done) | 5, 10 |
| 12 | MCP bridged to the running app, with screenshots (done) | 8, 10 |
| 13 | Model accuracy tests (stall, trim, phugoid, damping) (done) | 4 |
| 13a | Hold commands (altitude, heading, airspeed, climb rate) so an agent flies a pattern in tens of calls, not thousands (done: 32 calls) | 6 |
| 14 | Golden hash checked on macOS, Windows, and wasm in CI (done) | 5, 9 |

## Later

| # | Item | Depends on |
|---|---|---|
| 15 | Aircraft file format, versioned, hostile-input safe (done) | 3 |
| 16 | Constant wind, then gusts and turbulence (done) | 4 |
| 17 | Engine failure and other failures as commands (done) | 6 |
| 18 | Terrain tiles from SRTM for one region, streamed | 7, decision on data hosting |
| 19 | OpenStreetMap airfields and features with attribution in the app | 18 |
| 20 | A second fictional aircraft | 15 |
| 21 | Cockpit view and a fuller instrument panel | 8 |
| 22 | SolidCraft export format agreed and specified | decision with SolidCraft |
| 23 | Vortex-lattice coefficients from geometry | 15 |
| 24 | An aircraft modelled in SolidCraft flies | 22, 23 |
| 25 | Touch controls in the browser | 9 |
