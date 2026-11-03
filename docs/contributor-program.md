# Contributor program: golden samples, agent skills, and claims

Status: plan. It starts after the core training loop is done and the v1 formats are frozen (see
"Order" below). Until then, `CONTRIBUTING.md` covers code contributions.

The goal: anyone can tell their coding agent "contribute an aircraft to FlightCraft" or "write a
scenario for KMPO", and the agent knows what to make, how to check it, and how to submit it. The
project's tests decide whether a contribution is accepted, not a reviewer's opinion.

## 1. What contributors make

| Contribution | Value | Golden sample | Gate (what CI checks) |
|---|---|---|---|
| **Aircraft packs** | highest | [`flightcraft-sim/aircraft-sling-2`](https://github.com/flightcraft-sim/aircraft-sling-2): a real aircraft built from its POH, with a panel matched to a reference photo | flies within POH tolerances: stall speeds, cruise table, climb rates, takeoff and landing distances; schema-valid; every number has a source or an entry in the estimate register; no logos or copied screen software |
| **Scenarios and lessons** | high, easiest (no code) | the scenario library in `scenarios/` (engine failure after takeoff at 1N7, vacuum failure in IMC on the KABE ILS 06, and others) | `scenario.validate` passes: every facility, frequency, runway, fix, and procedure exists in FAA data; triggers are reachable; no instruction below terrain or into airspace without a clearance |
| **Airport detail** | medium | 1N7 and KABE | taxiways, buildings, and lighting match FAA airport data and diagrams; no hand-edited generated files |
| **Instrument skins** | medium | the six-pack, the electronic standby, and the glass displays | renders through the skin registry; caption-overlap test passes; original artwork only |
| **Hardware profiles** | medium | a Honeycomb yoke and quadrant profile, and a sample custom-board driver | device profile schema; the binding test drives every mapped control |

**Regions are not a contribution type.** Terrain, airports, airspace, navigation, frequencies, and
charts come from scripts run over public FAA and USGS data (`cargo xtask region build`, see
`docs/terrain.md`). They cover the whole US without hand work. Contributors add detail on top of
a region; they do not build regions.

## 2. Golden samples

A golden sample is the reference an agent copies. Each one is complete. It has:

- the content itself;
- a README that explains every file;
- tests that prove it is right;
- a provenance record for every number and asset;
- previews.

| Type | Sample | What makes it the reference |
|---|---|---|
| Real aircraft, modern glass | Sling 2 (912 iS, advanced IFR two-display panel) | POH-sourced flight model; 10 fidelity tests; estimate register; panel measured from a photo and checked with `cargo xtask panel-compare`; registration and spoken callsign; audio on the navigator |
| Fictional aircraft, conventional gauges | Corvid | six-pack and engine cluster; vacuum and electrical systems; fictional POH emergency procedures |
| Fictional aircraft, kit glass | Larrow | dual-lane engine; touch displays; breakers |
| Scenario | KABE "three-mile final" | triggers, typed ATC actions, readback waits, and a branch |
| Emergency scenario | Engine failure after takeoff, 1N7 runway 07 | failure trigger, POH checklist grading, and the glide-footprint debrief |
| Airport detail | 1N7 (untowered) and KABE (towered) | |

When a golden sample changes, its tests change with it, so the reference never drifts from what CI
enforces.

## 3. Agent skills

One skill per contribution type, under `skills/` in the repository, written for coding agents and
humans alike:

- `contribute-aircraft`:
  1. find the POH and published data;
  2. copy the Sling 2 pack's structure;
  3. fill in the facts, with provenance;
  4. tune the model until the fidelity tests pass;
  5. build the panel;
  6. render previews;
  7. open a PR.

  It lists what never to do: copy logos or screen software, use another simulator's data, or
  invent POH numbers.
- `contribute-scenario`: pick a region and airport, write the file from the template, run
  `scenario.validate`, fly it headless through the MCP server, and attach the transcript.
- `contribute-airport-detail`, `contribute-skin`, and `contribute-hardware-profile`, in the same
  shape.

Each skill ends with the exact commands CI will run, so an agent can pass them before opening a PR.

## 4. Claims

Modelled on SolidCraft's claim bot, so two people do not build the same aircraft:

- `/claim` on an issue assigns it. A draft PR that references the issue also claims it.
- If two PRs race for the same issue, the lowest-numbered PR wins.
- `/unclaim` releases a claim. A daily sweep frees claims with no activity for 14 days, after a
  warning comment.
- Wanted aircraft and scenarios are issues with labels (`aircraft`, `scenario`,
  `airport-detail`), so contributors can browse what is open.

## 5. Versioned formats

Contributions must not break when the engine changes. Before the program opens:

- the aircraft pack, panel, scenario, lesson, skin, and hardware profile schemas are frozen as v1,
  each carrying `format_version`;
- the engine reads every released version, and breaking changes need a new version plus a
  migration (`cargo xtask migrate`);
- golden samples and every merged contribution are tested in CI against engine `main`, so a
  breaking change is caught before it merges.

## 6. Order

1. **Core training loop first** (now): towered ATC; scenarios and emergencies; then speech input,
   AI-played ATC, instrument training tasks, and a stability pass. Formats change during this work,
   so opening contributions earlier would break contributors' work.
2. **Freeze v1 formats** with versioning and migration (section 5).
3. **Golden samples finalized:** the Sling 2 is declared the aircraft reference, with its README
   and tests aligned to the skill.
4. **Skills and templates** (section 3), then a contributor guide on the website.
5. **Claims bot and issue labels** (section 4), and seed issues for wanted aircraft and scenarios.

Roadmap items 27 and 28 track steps 3 to 5; item 35 tracks step 2.
