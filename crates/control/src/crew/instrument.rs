//! Procedure guidance drives ordinary controls, with the installed autopilot optional.
use super::*;
use crate::lesson::engine::ApproachPath;
use flightcraft_sim::autopilot::wrap;
use flightcraft_world::procedures::{ApproachCapability, Turn};
pub(super) fn plan(s: &Session, intent: &PilotIntent) -> Result<(CrewPlan, PilotPhase), CommandError> {
    let fail = |m| invalid(CrewIntent::NAME, m);
    if s.sim.readout().on_ground {
        return Err(fail(
            "instrument demonstration requires an airborne approach or hold setup",
        ));
    }
    match intent {
        PilotIntent::Approach {
            airport,
            procedure,
            mode,
            minimum_ft,
            coupled,
        } => {
            range(CrewIntent::NAME, "minimum_ft", *minimum_ft, 0., 18000.)?;
            let d = s
                .sim
                .world()
                .procedures()
                .ok_or_else(|| fail("procedure data required"))?;
            let p = d
                .procedures
                .iter()
                .find(|p| p.airport == *airport && p.ident == *procedure)
                .ok_or_else(|| fail("procedure unavailable"))?;
            if (*mode == InstrumentApproach::Lpv && !p.capabilities.contains(&ApproachCapability::LPV))
                || (*mode == InstrumentApproach::Lnav && !p.capabilities.contains(&ApproachCapability::LNAV))
                || (matches!(mode, InstrumentApproach::Ils | InstrumentApproach::Localizer)
                    && !p.ident.starts_with('I')
                    && !p.ident.starts_with('L'))
            {
                return Err(fail("selected approach capability unavailable"));
            }
            if *coupled && s.sim.installed_autopilot().is_none() {
                return Err(fail("coupled demonstration requires an installed autopilot"));
            }
            let runway = planning::runway(
                s,
                airport,
                p.runway.as_deref().unwrap_or("").trim_start_matches("RW"),
            )?;
            let path = ApproachPath::find(s, airport, procedure)?;
            let [n, e, _] = s.sim.state().position_m;
            let position =
                planning::geographic(s, [n, e, 0.]).ok_or_else(|| fail("regional projection unavailable"))?;
            let remaining = path.remaining(position);
            let (along, cross) = runway.relative(s.sim.state().position_m);
            if remaining < 2. * 1852. || cross.abs() > 2. * 1852. || along > 0. {
                return Err(fail(
                    "position on the inbound final course at least two miles before the runway",
                ));
            }
            let altitude_m = s.sim.readout().altitude_msl_m;
            Ok((
                CrewPlan {
                    briefing: format!(
                        "I'll fly the published approach to {} feet MSL, {}. I'll announce capture and minimums, and land only with visual reference and a landing clearance.",
                        minimum_ft.round(),
                        if *coupled {
                            "using the installed autopilot"
                        } else {
                            "by hand"
                        }
                    ),
                    legs: vec![planning::leg(
                        s,
                        "Final approach",
                        planning::local(s, path.endpoint, path.end_altitude_m)?,
                    )],
                    arrival: Some(runway),
                    departure: None,
                    altitude_m,
                    checks: vec![
                        "Published regional final approach geometry and receiver guidance.".into(),
                        "IFR demonstration crosswind envelope: 15 knots (engineering criterion).".into(),
                    ],
                },
                PilotPhase::Final,
            ))
        }
        PilotIntent::HoldAtFix {
            airport,
            procedure,
            fix,
        } => {
            let hold = PublishedHold::find(s, airport, procedure, fix)?;
            let point = planning::local(s, hold.position, hold.altitude_m)?;
            Ok((
                CrewPlan {
                    briefing: format!(
                        "I'll fly to {}, enter the published {}-turn hold, and correct the legs for wind.",
                        hold.fix,
                        if hold.turn == Turn::R { "right" } else { "left" }
                    ),
                    legs: vec![planning::leg(s, &hold.fix, point)],
                    arrival: None,
                    departure: None,
                    altitude_m: hold.altitude_m,
                    checks: vec![
                        "Hold course, altitude, turn direction, and leg specification from regional CIFP."
                            .into(),
                    ],
                },
                PilotPhase::Hold,
            ))
        }
        _ => Err(fail("not an instrument intent")),
    }
}
impl Crew {
    pub(super) fn instrument_angle(&self, s: &Session) -> Option<f64> {
        let PilotIntent::Approach {
            airport,
            procedure,
            mode,
            minimum_ft,
            ..
        } = self.state.intent.as_ref()?
        else {
            return None;
        };
        let path = ApproachPath::find(s, airport, procedure).ok()?;
        if self.instrument_calls & 1 == 0
            || self.radio.altitude_m.is_some()
            || self.radio.approach_level_m.is_some()
        {
            return Some(0.);
        }
        if matches!(mode, InstrumentApproach::Lnav | InstrumentApproach::Localizer)
            && self.instrument_calls & 4 == 0
        {
            let position = planning::geographic(s, s.sim.state().position_m)?;
            if path.altitude(path.remaining(position)) <= minimum_ft * 0.3048 {
                return Some(0.);
            }
        }
        Some(libm::tan(path.angle_deg.to_radians()))
    }
    pub(super) fn instrument_guidance(
        &mut self,
        s: &mut Session,
        phase: PilotPhase,
    ) -> Option<(f64, f64, f64)> {
        let intent = self.state.intent.clone()?;
        match intent {
            PilotIntent::Approach {
                airport,
                procedure,
                mode,
                minimum_ft,
                ..
            } if phase == PilotPhase::Final => {
                let path = ApproachPath::find(s, &airport, &procedure).ok()?;
                let r = s.sim.readout();
                if s.atc_state().ifr_clearance
                    && !s
                        .comms
                        .atc
                        .active
                        .iter()
                        .any(|p| p.readback && matches!(p.instruction, AtcInstruction::Approach { .. }))
                {
                    return Some((
                        path.course_deg.to_radians() - r.grid_north_true_rad,
                        self.state.plan.as_ref()?.altitude_m,
                        80. * 0.514444,
                    ));
                }
                let position = planning::geographic(s, s.sim.state().position_m)?;
                let remaining = path.remaining(position);
                let radio_mode = matches!(mode, InstrumentApproach::Ils | InstrumentApproach::Localizer);
                let radios = s.sim.radio_state();
                let signal = if radio_mode {
                    radios.first()?.guidance
                } else {
                    s.sim.procedure_receiver().guidance
                };
                let guidance = signal?;
                let heading = guidance.desired_track_rad;
                let track = heading + (-guidance.cross_track_m / 350.).clamp(-0.35, 0.35);
                let precision = matches!(mode, InstrumentApproach::Ils | InstrumentApproach::Lpv);
                let mut altitude = if precision || self.instrument_calls & 4 != 0 {
                    guidance
                        .vertical_deviation_m
                        .map_or_else(|| path.altitude(remaining), |dev| r.altitude_msl_m - dev)
                } else {
                    path.altitude(remaining).max(minimum_ft * 0.3048)
                };
                if guidance.cross_track_m.abs() < 250. {
                    self.radio.heading = None;
                    let [vn, ve, _] = s.sim.state().velocity_mps;
                    let actual_track = libm::atan2(ve, vn);
                    if guidance.cross_track_m.abs() < 100.
                        && wrap(actual_track - heading).abs() < 10_f64.to_radians()
                    {
                        self.radio.altitude_m = None;
                        if !precision {
                            self.radio.approach_level_m = None;
                        }
                    }
                }
                if precision && let Some(level) = self.radio.approach_level_m {
                    if altitude > level {
                        altitude = level;
                    } else {
                        self.radio.approach_level_m = None;
                    }
                }
                if self.instrument_calls & 1 == 0 && guidance.cross_track_m.abs() < 350. {
                    s.crew_callout(
                        if radio_mode {
                            "Localizer alive."
                        } else {
                            "Final approach course alive."
                        }
                        .into(),
                    );
                    self.instrument_calls |= 1;
                }
                if precision
                    && self.instrument_calls & 1 != 0
                    && self.instrument_calls & 2 == 0
                    && self.radio.heading.is_none()
                    && self.radio.altitude_m.is_none()
                    && self.radio.approach_level_m.is_none()
                    && guidance.vertical_deviation_m.is_some_and(|v| v.abs() < 15.)
                {
                    s.crew_callout(
                        if mode == InstrumentApproach::Ils {
                            "Glideslope captured."
                        } else {
                            "Glidepath captured."
                        }
                        .into(),
                    );
                    self.instrument_calls |= 2;
                }
                if self.instrument_calls & 4 == 0
                    && r.altitude_msl_m <= (minimum_ft + if precision { 0. } else { 1. }) * 0.3048
                {
                    self.instrument_calls |= 4;
                    // The instructor reveals the view; weather still determines whether a runway is visible.
                    s.sim.set_hood(false);
                    let visual = !s.sim.conditions().in_cloud(s.sim.state().position_m)
                        && remaining.abs() < s.sim.conditions().effective_visibility_m();
                    if visual && self.landing_authorized(s) {
                        s.crew_callout("Minimums, runway in sight, landing.".into());
                    } else {
                        s.crew_callout(
                            "Minimums, no landing reference or clearance. Missed approach.".into(),
                        );
                        let _ = s.execute(
                            "avionics.set",
                            serde_json::json!({"setting":{"kind":"missed_approach"}}),
                        );
                        self.change(s, PilotPhase::GoAround, "Flying the published missed approach.");
                    }
                }
                Some((track, altitude, 80. * 0.514444))
            }
            PilotIntent::Approach { .. }
                if phase == PilotPhase::GoAround && s.sim.procedure_receiver().missed_approach =>
            {
                let receiver = s.sim.procedure_receiver();
                let g = receiver.guidance?;
                let track = g.desired_track_rad + (-g.cross_track_m / 600.).clamp(-0.5, 0.5);
                let altitude = receiver
                    .legs
                    .get(receiver.active_leg)
                    .and_then(|l| l.leg.altitude1_m)
                    .unwrap_or(s.sim.readout().altitude_msl_m + 100.);
                Some((track, altitude, 80. * 0.514444))
            }
            PilotIntent::HoldAtFix {
                airport,
                procedure,
                fix,
            } => self.hold_guidance(s, &airport, &procedure, &fix),
            _ => None,
        }
    }
    pub(super) fn instrument_coupling(&mut self, s: &mut Session, phase: PilotPhase) {
        let Some(PilotIntent::Approach { coupled: true, .. }) = self.state.intent else {
            return;
        };
        let Some(ap) = s.sim.installed_autopilot() else {
            return;
        };
        let engaged = ap.engaged;
        let receiver_ready = s.sim.radio_state().first().is_some_and(|r| {
            r.signal_valid
                && r.guidance
                    .is_some_and(|g| g.vertical_deviation_m.is_some_and(|v| v.abs() < 15.))
        }) || s
            .sim
            .procedure_receiver()
            .vertical_deviation_m
            .is_some_and(|v| v.abs() < 15.);
        if phase == PilotPhase::Final
            && self.instrument_calls & 2 != 0
            && self.instrument_calls & 4 == 0
            && !engaged
            && receiver_ready
        {
            for setting in [
                serde_json::json!({"kind":"engage","on":true}),
                serde_json::json!({"kind":"vertical","mode":"alt"}),
                serde_json::json!({"kind":"lateral","mode":"apr"}),
            ] {
                let _ = s.execute(
                    "avionics.set",
                    serde_json::json!({"setting":{"kind":"autopilot","setting":setting}}),
                );
            }
        } else if (self.instrument_calls & 4 != 0 || phase != PilotPhase::Final) && engaged {
            let _ = s.execute(
                "avionics.set",
                serde_json::json!({"setting":{"kind":"autopilot","setting":{"kind":"disconnect"}}}),
            );
        }
    }
    fn hold_guidance(
        &mut self,
        s: &Session,
        airport: &str,
        procedure: &str,
        fix: &str,
    ) -> Option<(f64, f64, f64)> {
        let hold = PublishedHold::find(s, airport, procedure, fix).ok()?;
        let origin = planning::local(s, hold.position, hold.altitude_m).ok()?;
        let r = s.sim.readout();
        let [n, e, _] = s.sim.state().position_m;
        let [hn, he, _] = origin;
        let inbound = s
            .sim
            .heading_reference()
            .magnetic_to_grid(hold.inbound_deg.to_radians());
        let sign = if hold.turn == Turn::R { 1. } else { -1. };
        let speed = 80. * 0.514444;
        let radius = speed / (3_f64.to_radians());
        let leg = hold
            .distance_nm
            .map_or(speed * hold.time_s.unwrap_or(60.), |nm| nm * 1852.);
        let (sin, cos) = libm::sincos(inbound);
        if self.hold_path.is_empty() {
            if libm::hypot(hn - n, he - e) > 150. {
                return Some((libm::atan2(he - e, hn - n), hold.altitude_m, speed));
            }
            let [vn, ve, _] = s.sim.state().velocity_mps;
            let arrival_grid = libm::atan2(ve, vn);
            let arrival = s
                .sim
                .heading_reference()
                .grid_to_magnetic(arrival_grid)
                .to_degrees();
            let entry =
                flightcraft_sim::avionics::procedures::hold_entry(hold.inbound_deg, arrival, hold.turn);
            let point = |along: f64, right: f64| {
                [
                    hn + along * cos - right * sin,
                    he + along * sin + right * cos,
                    -hold.altitude_m,
                ]
            };
            match entry {
                flightcraft_sim::avionics::procedures::HoldEntry::Teardrop => {
                    let outbound = inbound + core::f64::consts::PI - sign * 30_f64.to_radians();
                    let (os, oc) = libm::sincos(outbound);
                    let [en, ee, _] = [hn + leg * oc, he + leg * os, -hold.altitude_m];
                    self.hold_path.push([en, ee, -hold.altitude_m]);
                    let cn = en - sign * radius * os;
                    let ce = ee + sign * radius * oc;
                    let radial = outbound - sign * core::f64::consts::FRAC_PI_2;
                    for step in 1..=28 {
                        let angle = radial + sign * f64::from(step) * 210_f64.to_radians() / 28.;
                        self.hold_path.push([
                            cn + radius * libm::cos(angle),
                            ce + radius * libm::sin(angle),
                            -hold.altitude_m,
                        ]);
                    }
                    self.hold_path.push(point(-leg * 0.5, 0.));
                    self.hold_path.push(origin);
                }
                flightcraft_sim::avionics::procedures::HoldEntry::Parallel => {
                    self.hold_path.push(point(-leg, -sign * 150.));
                    let outbound = inbound + core::f64::consts::PI;
                    let [en, ee, _] = point(-leg, -sign * 150.);
                    let turn = -sign;
                    let cn = en - turn * radius * libm::sin(outbound);
                    let ce = ee + turn * radius * libm::cos(outbound);
                    let radial = outbound - turn * core::f64::consts::FRAC_PI_2;
                    for step in 1..=30 {
                        let angle = radial + turn * f64::from(step) * 225_f64.to_radians() / 30.;
                        self.hold_path.push([
                            cn + radius * libm::cos(angle),
                            ce + radius * libm::sin(angle),
                            -hold.altitude_m,
                        ]);
                    }
                    self.hold_path.push(point(-leg * 0.5, 0.));
                    self.hold_path.push(origin);
                }
                flightcraft_sim::avionics::procedures::HoldEntry::Direct => {}
            }
            self.hold_loop_start = self.hold_path.len();
            // Construct both constant-radius turns and straight published legs in the holding plane.
            for i in 1..=24 {
                let a = f64::from(i) * core::f64::consts::PI / 24.;
                let along = radius * libm::sin(a);
                let right = sign * radius * (1. - libm::cos(a));
                self.hold_path.push([
                    hn + along * cos - right * sin,
                    he + along * sin + right * cos,
                    -hold.altitude_m,
                ]);
            }
            self.hold_path.push([
                hn - leg * cos - sign * 2. * radius * sin,
                he - leg * sin + sign * 2. * radius * cos,
                -hold.altitude_m,
            ]);
            for i in 1..=24 {
                let a = f64::from(i) * core::f64::consts::PI / 24.;
                let along = -leg - radius * libm::sin(a);
                let right = sign * radius * (1. + libm::cos(a));
                self.hold_path.push([
                    hn + along * cos - right * sin,
                    he + along * sin + right * cos,
                    -hold.altitude_m,
                ]);
            }
            self.hold_path.push(origin);
            self.state.leg = 0;
        }
        let [tn, te, _] = *self.hold_path.get(self.state.leg)?;
        if libm::hypot(tn - n, te - e) < 120. {
            self.state.leg = if self.state.leg + 1 >= self.hold_path.len() {
                self.hold_loop_start
            } else {
                self.state.leg + 1
            };
        }
        let desired = libm::atan2(te - e, tn - n);
        Some((
            r.heading_grid_rad + wrap(desired - r.heading_grid_rad),
            hold.altitude_m,
            speed,
        ))
    }
}
