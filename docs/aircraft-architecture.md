# Aircraft, systems, and avionics architecture

Status: draft, 2026-11-01. The infrastructure for modelling certified and kit light aircraft as a
pilot knows them: one airframe across model years, each with the panel it was fitted with, flown
to its published numbers. This builds on `docs/SPEC.md` and `docs/aircraft-format.md`.

## 1. What a pilot means by "the same plane"

A training fleet often has one type in many forms: older and newer model years with different
engines, flaps, and weights, each with a different panel (round gauges, electronic standby units,
or a full integrated glass suite) and with or without an autopilot. They fly almost alike but are
operated differently. The model has to express that difference precisely and without copying
whole files.

## 2. Layers of an aircraft definition

| Layer | Holds | Changes between |
|---|---|---|
| Airframe family | geometry, aerodynamic tables, gear, control surfaces | types |
| Variant | model year: weights and CG envelope, flap stages and limits, fuel capacity, engine choice, V-speeds | model years |
| Engine | power curve, induction (carburettor or injection), propeller, mixture, carb ice, fuel flow | engine options |
| Systems | electrical buses, alternator and battery, vacuum, pitot-static, fuel tanks and selector, lights, breakers | variants and panels |
| Panel | which instruments and avionics are installed, and where | individual aircraft |
| Avionics | instrument and display behaviour, each a module | equipment models |
| Autopilot | the modes, their logic, and the servos | equipment models |

Files compose by reference: a variant names its family and engine, a panel names its avionics
modules, and an aircraft ("this tail number at the school") is a variant plus a panel plus a
weight-and-balance load. Each layer is versioned and checked on load like today's format.

## 3. Systems

Systems are deterministic state in `sim`, stepped with the flight model, so failures and their
cascades are reproducible and testable.

- **Vacuum** drives the gyro attitude and heading indicators in a round-gauge panel; a vacuum
  failure makes them precess and topple slowly, as real gyros do.
- **Electrical**: battery, alternator, buses, and breakers; electronic instruments draw from them
  and have their own backup batteries where installed.
- **Pitot-static**: blocked pitot or static ports give the published wrong indications (airspeed
  acting as an altimeter, frozen altimeter and VSI); alternate static changes the error.
- **Fuel**: tanks, selector, quantity gauges with their lag, fuel flow from the engine model, and
  starvation.
- **Engine operation**: magnetos and the mag check drop, primer, mixture leaning to peak, carb
  heat, and carb ice in the right humidity.

Every switch, knob, and selector is a command, so a person clicking it, a script, and an agent all
work the same panel.

## 4. Instruments and avionics

Each instrument or avionics unit is a module with one interface: inputs from the sim and systems
(true values plus the sensor errors that unit suffers), its own internal state (gyro precession,
filter lags, menus and pages), commands for its buttons and knobs, and a drawing on the panel.

Families to build:

1. **Round gauges**: airspeed, attitude, altimeter (with Kollsman setting), turn coordinator,
   heading indicator (with gyro drift and the set knob), VSI, plus the engine gauges and the
   magnetic compass with its turning and acceleration errors.
2. **Electronic standby units**: a compact attitude display and an electronic HSI that replace the
   vacuum gyros, with their own battery and failure behaviour.
3. **Integrated glass**: a primary flight display (airspeed and altitude tapes, attitude, HSI,
   trend vectors) and a multi-function display (moving map, engine page, flight plan), driven by
   an air-data and attitude computer whose failure modes are modelled.
4. **Navigation radios and GPS**: VOR, localiser and glideslope, and GPS navigation with a
   flight plan, all reading the navigation database.
5. **Autopilot**: modes as a state machine (roll and heading, navigation, approach with
   glideslope or glidepath, altitude hold, vertical speed, indicated-airspeed climb, level, and
   flight director), with mode annunciation and the servo limits that make it fly like the real
   one.

General display conventions come from public FAA guidance on electronic flight displays.

## 5. Navigation data

From public-domain US government data: the FAA's National Airspace System Resource (NASR)
subscription for airports, runways, navaids, fixes, and airways, and the FAA's coded instrument
flight procedures (CIFP) for approaches and departures, refreshed each 28-day cycle by an xtask.
Runways and airports from this data also place the airfields in the terrain.

## 6. Fidelity and its proof

"Accurate" is measured, not asserted. For each variant, tests compare the model with that type's
published performance: stall speeds at each flap setting, takeoff and landing distances, rate of
climb, cruise speed and fuel flow at power settings, and best-glide speed and ratio. Where a test
can only be judged by flying (trim changes with flap, adverse yaw, stall behaviour), a pilot who
knows the type flies a scripted checklist of manoeuvres and records findings, which become tests
where they can.

Sources: the FAA type certificate data sheet for each type (public), public NASA and NACA
reports, textbook aerodynamics, and the published performance figures used only as test targets.

## 7. Order of work

1. The layered definition format (family, variant, engine, systems, panel), with the existing two
   aircraft converted.
2. Systems in `sim`: vacuum, electrical, pitot-static, fuel, and engine operation.
3. The instrument module interface; round gauges rebuilt on it with their sensor errors.
4. Navigation database import and radio navigation.
5. Electronic standby units, then the integrated glass PFD and MFD.
6. The autopilot mode machine.
7. Each real type, variant by variant, against its published numbers.

## 8. Decisions

1. **Real aircraft live in separate aircraft packs.** This repository, the engine, stays
   brand-neutral: no manufacturer names, logos, or type names in its code, assets, or docs. Each real
   type is its own pack that the app loads, named descriptively, with a note that it is not
   affiliated with or endorsed by the manufacturer.
2. **Avionics are behaviour plus a skin.** Every unit's behaviour (buttons, knobs, modes, pages,
   procedures, and failure logic) is one module with its tests. How it looks is a separate skin
   drawn on top, and both run on the same modules, so behaviour is written and tested once.
   - The engine's built-in skins are our own generic design on public FAA display conventions.
   - A real unit's look ships as a separate **skin pack**, its own repository loaded like an
     aircraft pack. It may match the real unit's layout geometry, element placement, colour
     meanings, page, menu, and soft-key structure, knob behaviour, and short functional labels, all
     taken from public manufacturer documentation and cited.
   - Its artwork is drawn from scratch with an openly licensed font. It carries no manufacturer
     logos or trademarks in the interface, no proprietary fonts, and no traced or copied graphics.
     Its README names the real unit only to say what it trains for, and that it is not affiliated.
   - A pixel-exact reproduction (logos, fonts, original graphics) needs the manufacturer's written
     permission and is not published without it.
