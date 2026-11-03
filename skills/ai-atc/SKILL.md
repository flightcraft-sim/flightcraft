---
name: ai-atc
description: Play a simulated FlightCraft tower or radar facility, or coach an instructor session, over the existing MCP connection. Use when asked to be ATC or an instructor in FlightCraft.
---

# FlightCraft AI ATC

Use the connected FlightCraft MCP server. Tool names replace dots with underscores
(`comms.listen` becomes `comms_listen`). Inspect the server's schemas before sending commands.
[Radio](../../docs/radio.md) and [ATC](../../docs/atc.md) describe the training model's limits.

## Join the frequency

Read `comms.facilities`, `comms.audio_panel`, `atc.state`, and `sim.state`. Select only a
facility ID, role, and channel published together in the active region's FAA NASR catalogue.
Never infer a frequency from memory, rename a facility, or invent a taxiway. If the required
facility is absent, explain that it is unavailable in the installed region.

Configure the training airport with `atc.configure`, setting `external_controller:true` to play
live ATC. This stops automatic clearance issuance and handoffs while keeping ATIS, readbacks,
and flight compliance monitoring. Restore `external_controller:false` when handing back to
built-in ATC. Changing mode is appropriate when the user has asked you to play the facility.

Drain `comms.listen`, then read `atc.state`. Wait for completed, heard pilot calls. Do not act on
recognition previews: `Meaning::Recognition` is local, ungraded evidence awaiting student review.
If a student asks for another attempt, allow it; never complete a readback on their behalf.
In headless sessions, use bounded `sim.step` calls to advance radio time. In a running app, let
its session clock advance. Wait for channel idle before speaking.

## Send a typed instruction

Send `comms.atc` with the exact catalogue facility, channel, and role, and an instruction:

```json
{
  "facility": "<ID returned by comms.facilities>",
  "channel_khz": 0,
  "role": "tower",
  "instruction": {
    "kind": "controller",
    "instruction": {"kind":"land","runway":"06"}
  }
}
```

Replace the ID and channel with the returned record; zero is deliberately invalid. The app
renders FAA wording, speaks with the facility's voice and radio effect, and tags the transcript
as **Simulated ATC** with source `ai`. Do not send free-form control instructions through advisory
text. After speaking, verify that `comms.listen` reports it heard; an untuned or blocked call
has not established contact. Wait for the student's correct readback before issuing another
instruction. A wrong readback stays outstanding and is graded by the shared rules.

## Roles and validation

- **Tower:** use `takeoff`, `land`, `go_around`, `pattern`, `extend_downwind`, `exit_runway`, and
  published `handoff` instructions. Runways must exist at that tower's airport. Pattern numbers
  are one through four; a following callsign must identify actual simulated traffic. Inspect
  runway occupancy and pilot position before a clearance; the validator is not a separation
  controller. Use closed traffic and monitor downwind, base, final, and runway-clear reports.
- **Ground:** taxi requires an existing runway, an empty route, and `hold_short:true`. The region
  has no surveyed taxiway graph. Use the runway/hold-short fallback and `parking`.
- **Approach/departure:** use `squawk`, `vector`, `altitude`, and a handoff to an actual published
  facility/channel pair. Squawks are four octal digits. Headings are zero through 360 degrees;
  altitudes are feet, within 1,000–60,000 ft, and at least 1,000 ft above sampled ownship terrain.
  This check does not establish minimum vectoring altitudes or clearance along a route. Bravo,
  IFR, approach, hold, and missed-approach instructions currently require built-in rules mode and are refused by the live
  typed-controller validator.
- **Instructor:** keep coaching on the intercom using `crew.say` with text and optional
  `critical:true` for an urgent cue; never pose as a facility. Read recorded grades
  from `atc.state` and `flight.events`. Explain the observed deviation and the next action in
  plain English. Use `crew.handover` for a positive exchange: the pilot cannot acknowledge as
  the crew, and the crew cannot acknowledge as the pilot. Do not issue flight-control commands
  merely because you can read state.

The server refuses mismatched facilities/frequencies/roles, unknown runways, invented taxiways,
unsupported authority, invalid numerical values, COM failure, and an outstanding instruction.
On refusal, inspect fresh state and correct the actual input. Do not retry by putting the same
instruction into an advisory or changing roles. Stop issuing instructions if the data is missing.
The student practices speaking aloud using PTT; typed entry remains an accessibility fallback.
