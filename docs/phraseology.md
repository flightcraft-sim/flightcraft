# Shared radio phraseology

`flightcraft_control::comms::phraseology` is the stateless renderer for ATC, weather radio,
traffic, and scenario authors. Use its primitives when constructing a transmission; the typed
command/event values keep their numeric units. These functions do not issue clearances, parse
pilot intent, change the simulation, or select frequencies. Phraseology follows the number
conventions in FAA JO 7110.65 chapter 2, section 4, and the AIM radio chapter.

| Function | Input | Example output |
|---|---|---|
| `digits(value, width)` | Unsigned number, minimum digit width | `digits(4003, 4)` → `four zero zero three` |
| `runway(designator)` | FAA runway designator | `05` → `zero five`; `04L` → `zero four left` |
| `heading(degrees)` | Magnetic heading, degrees | `48` → `zero four eight`; north → `three six zero` |
| `altitude(feet)` | Feet below flight levels | `2200` → `two thousand two hundred`; `10000` → `one zero thousand` |
| `altitude_assignment(assigned_ft, actual_ft)` | Feet MSL | `climb and maintain`, `descend and maintain`, or `maintain`, with the spoken altitude |
| `frequency(khz)` | Published COM channel in kHz | `119200` → `one one niner point two`; insignificant fractional zeros are omitted |
| `altimeter(hundredths_inhg)` | Hundredths of inches Hg | `2992` → `two niner niner two` |
| `wind(from_deg, knots)` | Magnetic direction and knots | `wind zero six zero at eight`; zero knots → `wind calm` |
| `facility(station)` | Imported `ComStation` | NASR radio call plus role, such as `Morristown Tower` or `New York Approach` |
| `normal_case(text)` | Imported uppercase name | Normal case, expanding `INTL` and `MUNI` |

`facility_kind`, `speaker`, and `reception` provide readable labels for UI/debrief clients.
`Transmission.speaker_name` stores the label at emission, so saved records do not require the
original region to show a facility's radio call. Older records without this optional field fall
back to a readable role. Do not render Rust `Debug` values into captions or transcripts.

`scripted_transmission(station, pilot, instruction, wind)` renders validated scenario `Instruction`
values and live `comms.atc` transmissions. It addresses the spoken callsign and NASR facility,
then uses these same runway, altitude, and wind primitives. Altitudes are metres MSL and wind
is the pilot magnetic weather report. Scenario authors do not need a separate phrasing adapter.

For complete typed ATC instructions, `Session::atc_readback(&AtcInstruction)` produces a short
model readback with key items and the aircraft callsign at the end. The checker also accepts
callsign-first readbacks and equivalent spoken or numeric values. It checks the clearance limit,
route, initial altitude, expected altitude/time, departure channel, and code in a CRAFT readback.
A scenario should use the values actually issued, rather than copying the illustrative numbers
in documentation. Numeric schemas remain the source of input bounds; these formatting helpers
are deliberately small and accept already validated values.

This API currently covers terminal training below flight levels. It does not implement every
FAA phrase, international radiotelephony convention, or flight-level/Mach-number rendering.
See [tower and approach training](atc.md) and [the radio service](radio.md).
