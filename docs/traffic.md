# Busy-airspace traffic

`traffic.density` fills the airspace around a towered airport with simulated aircraft. The tower and
approach rules of [tower and approach training](atc.md) control them on the real NASR frequencies,
using the same typed instructions, [shared phraseology](phraseology.md), and readback checker that
the user's own calls go through. You hear them when your radios can: each transmission passes the
same tuning, power, range, and terrain line-of-sight test as every other [radio](radio.md) call.

This page covers the engine and the [displays](#displays). Voices, traffic advisories, and
radio-discipline grading build on the same data and are separate work.

```json
{"command":"traffic.density","input":{"density":"busy","airport":"KABE"}}
{"command":"traffic.state","input":{}}
```

| Density | Aircraft |
|---|---|
| `off` | none; nothing runs and nothing is recorded |
| `light` | 4 |
| `moderate` | 8 |
| `busy` | 14 |

`airport` is optional: the configured ATC airport, else the nearest towered airport. The airport needs
an imported tower and approach control. `traffic.set` is unchanged: it still starts up to three CTAF
circuit aircraft, and the two can run together.

## What flies

| Flow | Route | Frequencies |
|---|---|---|
| Arrival | Transition entry fix from the CIFP approach for the active runway, the initial approach fix, the final approach fix, the threshold. Spawned about 30 NM out on the rear side of the runway. | Approach high (above 3,000 ft), approach low, tower, then ground |
| Departure | Holds short, is cleared for takeoff when the runway and the landing order allow, climbs straight out then turns away from the circuit side. | Tower, departure, approach high |
| Flight following | A VFR track across the approach area at least 6 NM from the field, at the hemispheric VFR altitude (14 CFR 91.159). | Approach high |
| Circuit | Left-hand pattern touch-and-goes ending in a full stop. | Tower |

The runway comes from the same wind rule the ATIS uses. Facility identity, frequency, sector, and
spoken name come from the imported station records; nothing is hard-coded. Approach high and low are
the stations whose NASR sector reads `ABV 3000 FT` and `3000 FT & BLW`. The approach fixes and
transition entries come from the region's procedure catalogue. Each aircraft's airspace class is looked up
in the region's airspace file when state is read.

## Aircraft

Each has a callsign, type, position, altitude, speed, heading, intent, and tuned frequency, flown by a
bounded kinematic model (turn, climb, descent, and acceleration limits per type). It is not the 6-DOF
simulation. There are twelve types, from light singles to a business jet. Winds are not applied.

Callsigns are fictional. General aviation uses a generated FAA-format N-number (checked by the same
validator as the user's), spoken with the type: `Cessna Four Two Seven Kilo Papa`. A few operator
names are invented (`Tamarack`, `Brindle`, `Quillon`, `Harrow`, `Fenwick`); no real airline is used.
No two aircraft share a spoken or abbreviated callsign, and none matches the user's.

Everything is seeded from the session weather seed and the airport, with no clock or hash iteration.
The same seed gives the same transcript. When the density is `off` no traffic code runs and the
simulation golden hashes are untouched.

## How ATC drives them

Each aircraft is a small state machine. A stage waits for geometry (distance to the field, altitude,
distance to go on final, runway state) and then the controller of the aircraft's current frequency
speaks. For an arrival:

1. The pilot checks in with approach high with the ATIS letter.
2. Approach: `radar contact, squawk ..., descend and maintain four thousand`.
3. At 22 NM: `contact Allentown Approach one one niner point six five`.
4. Approach low: `descend and maintain three thousand`.
5. About 11 NM out: `Number two, following the Skyhawk on a four mile final. Cleared ILS runway zero six
   approach. Contact Allentown Tower one two zero point five.`
6. Tower: `cleared to land` once the runway is free, then `exit at the next available taxiway, contact
   ground`.

Every controller transmission is a list of `AtcInstruction` values rendered by the shared renderer. The
AI pilot reads it back with the shared readback renderer, and the checker used for the user decides
whether the readback is acceptable (`parse_pilot` and `readback_matches`). About 7% of readbacks carry
one wrong value; the controller says `negative` and repeats the instruction, and the pilot reads back
again. Only an accepted readback changes the aircraft's behavior, so its track follows what was said:
altitude and speed targets, headings, approach and landing clearance, the frequency change, and the
takeoff roll all happen after the accepted readback.

Two instruction types were added for this: `speed` (`Reduce speed to niner zero.`, or resume normal
speed) and `touch_and_go`.

### Sequencing and separation

All landing traffic (arrivals and cleared circuit aircraft) share one landing order sorted by estimated
time to the threshold. A new arrival is only spawned if its slot is at least 125 s clear of the others.
An aircraft closer than 100 s behind its leader gets a speed reduction; when the gap opens past 170 s the
controller cancels it. Circuit aircraft are told to extend downwind until their base turn fits a gap,
and the departure release needs a free runway, 75 s since the last release, and no landing within 110 s.
`sequence` in the call data names the aircraft followed. A last-resort physics guard
(`stats.safety_slowdowns`) slows an aircraft that gets within 1.3 NM and 140 m of another; tests require
it to stay at zero.

For the user, circuit and final-approach traffic is merged into `traffic_state()`, so the existing
tower rules say `Number two, follow the Cherokee on left base` against real positions, and hold the user
for traffic on final.

### The frequency

Each frequency has one speaker at a time. Transmissions queue with priorities (corrections, readbacks,
instructions, then check-ins), the controller waits 1.5 to 3.5 s between exchanges, and nothing starts
while the user (or a queued reply to the user) occupies the carrier. The user can transmit in the gaps.
The traffic keeps running when its transmissions cannot be heard, but only heard transmissions become
events.

## Data for other features

`traffic.state` returns for every aircraft: `callsign`, `registration`, `aircraft_type`, latitude and
longitude, `altitude_msl_ft`, `position_m` (region NED), true and magnetic track, `ground_speed_kt`,
`vertical_rate_fpm`, `squawk`, `frequency_khz`, the controlling `facility_id` and spoken `facility`,
`intent` (flow, phase, airport, runway), assigned altitude, heading, and speed, clearance flag,
`airspace_class`, `voice`, range, true bearing and relative altitude from the user, and
`on_user_frequency`. It also lists the active frequencies with their aircraft counts.

Transmissions are ordinary flight events with `meaning.type = "fleet"` and a `call`:

- `speaker` is `facility` for the controller and `traffic` (with the spoken callsign) for the pilot;
  `speaker_name` is the spoken facility or callsign.
- `call.exchange` links an instruction, its readback, any correction, and the second readback.
- `call.callsign`, `call.aircraft`, `call.voice` (0-7, stable per aircraft or facility), `call.role`,
  `call.kind`, `call.instructions` (typed), `call.facility`, `call.frequency_khz`, and `call.sequence`.
- `call.for_user` is false: simulated traffic never addresses the user. The user answers only calls
  that contain their own callsign.

## Displays

Every display reads one merged list (`Session::traffic_targets`): the busy-airspace fleet plus any
`traffic.set` circuit aircraft, nearest first, each with its symbol class, a stable 24-bit address
(`0xF20000 + aircraft id` for the fleet), relative altitude, and whether a controller just named it.

**Symbols.** Other traffic is a hollow arrow along its track, proximate traffic (within 6 NM and
1,200 ft) a filled arrow, and a traffic alert (within 2 NM and 500 ft, or on course to pass inside 1
NM and 500 ft within 45 s) an amber filled symbol. The relative-altitude tag is in hundreds of feet
with a sign, and a climb or descent arrow appears above 500 fpm. Aircraft on the ground never alert.

| Where | What it shows |
|---|---|
| Glass MFD map | ADS-B symbols, tags, range ring, `TFC` filter (normal, above, below, unrestricted) and show/hide key. Only when the aircraft has an ADS-B In installation. See [glass](glass.md#traffic). |
| Glass PFD | `TRAFFIC` annunciation while any alert is active. |
| Planning map and PFD map inset | Arrow, registration and type, altitude and speed, a one-minute trend vector, and a hover or click card with callsign, type, altitude, speed, assigned altitude, heading and speed, frequency, squawk, and intent. The inset draws the symbol only. The planning map always shows traffic; the glass-hosted map shows it only with a receiver. |
| Traffic screen page | A heading-up radar (2 to 80 NM, three rings, ownship at the centre) and a table sorted by range. Open it as a page in [multiple screens](multiple-screens.md#traffic-page) (`traffic`). Not gated by equipment, being an instructor-style aid. |
| Tablet EFB | A GDL 90 Traffic Report (ID `0x14`) and an `XTRAFFIC` line per aircraft, once a second. See [EFB](efb-and-charts.md). |

**Matching what the controller says.** When a heard call names an aircraft, that aircraft is
highlighted on all three screens (magenta pulsing ring, magenta row on the Traffic page) from the
start of the call until 20 s after it ends. A call names an aircraft through any of:

- an aircraft id in a field named `target_aircraft`, `target_aircraft_id(s)`, `traffic_aircraft`, or
  `aircraft_ids` (a number or a list, anywhere in the event's meaning);
- a sequencing callsign: the `follow` of a pattern instruction (`Number two, follow the Cherokee on
  left base`), or the `sequence.follows` of a fleet controller call;
- the `callsign` of a `traffic_advisory`.

Callsigns match the spoken callsign or the registration. A call's own speaker is never a
reference, nothing an AI pilot says names anyone, and blocked or unheard calls highlight nothing.
`traffic_references` (in `comms`) extracts the names from any event, so a new `Meaning` that carries
`target_aircraft` is highlighted with no display change.

## Limits

No wake model, wind, weather diversion, taxi movement (departures appear at the hold line and arrivals
leave the runway), ground or clearance delivery exchanges, go-around sequencing beyond a simple
circuit re-entry, or spoken traffic advisories to the user. Separation is a training model, not a certified
one. Traffic exists only around one airport at a time.
