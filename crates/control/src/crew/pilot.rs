use super::*;
use flightcraft_sim::{Controls, DT_S, autopilot::wrap};
impl Session {
    pub(crate) fn sequence_crew(&mut self) {
        if !self.crew_owns_controls() {
            return;
        }
        let mut crew = std::mem::take(&mut self.crew);
        crew.tick(self);
        self.crew = crew;
    }
}
impl Crew {
    pub(super) fn change(&mut self, s: &mut Session, phase: PilotPhase, reason: &str) {
        let adjusted = if self.pattern_side < 0. {
            reason.replace("left", "right")
        } else {
            reason.to_owned()
        };
        let reason = adjusted.as_str();
        if self.state.phase == phase {
            return;
        }
        self.state.phase = phase;
        if phase == PilotPhase::Hold {
            self.hold_heading_grid_rad = Some(s.sim.readout().heading_grid_rad);
        }
        self.phase_since = s.sim.state().time_s;
        self.state.next_step = phase.label().into();
        self.state.reason = reason.into();
        if phase != PilotPhase::Cruise {
            s.rate = 1.0;
        }
        let critical = matches!(
            phase,
            PilotPhase::Takeoff
                | PilotPhase::Final
                | PilotPhase::Flare
                | PilotPhase::Rollout
                | PilotPhase::GoAround
        );
        if (self.phase_since - self.last_speech >= 12. || critical)
            && !(phase == PilotPhase::Final
                && matches!(self.state.intent, Some(PilotIntent::Approach { .. })))
        {
            let text = match phase {
                PilotPhase::Takeoff => "Taking off.".to_owned(),
                PilotPhase::Flare => "Eyes down the runway.".to_owned(),
                PilotPhase::Rollout => "We're down.".to_owned(),
                PilotPhase::GoAround => "Going around. Full power.".to_owned(),
                PilotPhase::Final => "Final. Tracking the centerline.".to_owned(),
                _ => reason.to_owned(),
            };
            if critical {
                s.crew_callout(text);
            } else {
                s.crew_say(text);
            }
            self.last_speech = self.phase_since;
        }
        if let Some(camera) = &mut s.camera
            && self.state.director
            && !s.sim.hood()
        {
            camera.mode = match phase {
                PilotPhase::Takeoff | PilotPhase::GoAround => CameraMode::Cockpit,
                PilotPhase::Climb => CameraMode::Wing,
                PilotPhase::Cruise => CameraMode::Flyby,
                PilotPhase::Flare | PilotPhase::Rollout => CameraMode::Tower,
                _ => CameraMode::Chase,
            };
        }
        self.queue_position(s, phase);
    }
    fn safe_exit(&self, s: &Session, runway: &CrewRunway, along: f64) -> Option<(f64, [f64; 3])> {
        // A few widely spaced probes miss narrow, continuous exits in sampled terrain.
        // Search the usable runway at walking-pace intervals before declaring a hold.
        let remaining = (runway.length_m - along - 45.).clamp(0., 1800.);
        for advance_step in 0..=(remaining / 2.).floor() as u32 {
            let advance = f64::from(advance_step) * 2.;
            if along + advance + 35.0 > runway.length_m - 10.0 {
                continue;
            }
            for side in [-1.0, 1.0] {
                let across = side * (runway.width_m * 0.5 + 9.0);
                let mut previous = None;
                let mut safe = true;
                let length = advance + libm::hypot(25.0, across);
                for step in 0..=(libm::ceil(length * 2.0) as u32) {
                    let travelled = f64::from(step) * 0.5;
                    let fraction = ((travelled - advance) / libm::hypot(25.0, across)).clamp(0.0, 1.0);
                    let a = along + travelled.min(advance) + 25.0 * fraction;
                    let [n, e, _] = runway.point(a, across * fraction, 0.0);
                    let ground = s.sim.world().ground(n, e);
                    if ground.surface == flightcraft_world::Surface::Water
                        || ground.normal_ned.get(2).is_none_or(|d| *d > -0.99)
                        || previous.is_some_and(|h: f64| (ground.height_msl_m - h).abs() > 0.05)
                    {
                        safe = false;
                        break;
                    }
                    previous = Some(ground.height_msl_m);
                }
                if safe {
                    return Some((along + advance, runway.point(along + advance + 25.0, across, 0.0)));
                }
            }
        }
        None
    }
    fn tick(&mut self, s: &mut Session) {
        let now = s.sim.state().time_s;
        let frozen = self.last_step_time == Some(now);
        self.last_step_time = Some(now);
        let landing = matches!(self.state.phase, PilotPhase::Final | PilotPhase::Flare);
        // Full readouts include geography and aerodynamic loads. Sample guidance at 10 Hz;
        // retain fixed-step contact measurements on landing and detect a frozen crashed state.
        if !landing && !frozen && self.last_tick.is_some_and(|t| now - t < 0.1 - 1e-8) {
            return;
        }
        let r = s.sim.readout();
        if r.on_ground && !s.sim.canopy().secure() {
            use flightcraft_sim::canopy::CanopyAction;
            let result = s
                .sim
                .set_canopy(CanopyAction::Close, None)
                .and_then(|_| s.sim.set_canopy(CanopyAction::Latch, None));
            if result.is_err() {
                self.state.reason = "Holding brakes: canopy could not be closed and latched.".into();
                let _ = SetControls {
                    throttle: Some(0.),
                    brake: Some(1.),
                    ..Default::default()
                }
                .run(s);
                return;
            }
            s.crew_say("Before taxi: canopy closed and latched.".into());
        }
        if r.crashed {
            self.state.reason = "Flight stopped: aircraft crashed; reset before continuing".into();
            self.state.phase = PilotPhase::Refused;
            self.state.controls = ControlOwner::Pilot;
            s.rate = 1.;
            return;
        }
        if self.last_tick.is_none() && self.state.phase != PilotPhase::BeforeTakeoff {
            let phase = self.state.phase;
            let reason = self.state.reason.clone();
            self.state.phase = PilotPhase::Idle;
            self.change(s, phase, &reason);
        }
        if !r.on_ground {
            self.airborne_sink_mps = (-r.vertical_speed_mps).max(0.);
        }
        let touchdown_edge = r.on_ground && matches!(self.state.phase, PilotPhase::Final | PilotPhase::Flare);
        if self.last_tick.is_some_and(|t| r.time_s - t < 0.1 - 1e-8) && !touchdown_edge {
            return;
        }
        let previous_agl = self.previous_agl;
        self.previous_agl = Some(r.height_agl_m);
        let dt = self.last_tick.map_or(0.1, |t| (r.time_s - t).clamp(DT_S, 0.2));
        self.last_tick = Some(r.time_s);
        self.work_radio(s, r.on_ground);
        if r.on_ground
            && s.comms
                .atc
                .pending
                .iter()
                .chain(s.comms.atc.active.iter())
                .any(|p| p.heard && matches!(p.instruction, AtcInstruction::Taxi { hold_short: true, .. }))
        {
            self.state.reason = "Holding brakes for the runway hold-short instruction.".into();
            let _ = SetControls {
                throttle: Some(0.),
                brake: Some(1.),
                ..Default::default()
            }
            .run(s);
            return;
        }

        let Some(plan) = self.state.plan.clone() else {
            return;
        };
        let Some(performance) = s.aircraft_configuration().map(|c| c.variant.v_speeds.clone()) else {
            return;
        };
        let tailwheel = s.sim.aircraft().geometry.gear.iter().any(|g| {
            g.steering_rad.abs() > 0.0
                && s.sim
                    .aircraft()
                    .relative_point(g.point_m)
                    .first()
                    .is_some_and(|x| *x < 0.0)
        });
        let ground_pitch = if tailwheel { 0.18 } else { 0.0 };
        let position = s.sim.state().position_m;
        let [p, q, yaw_rate] = s.sim.state().angular_rate_rps;
        let arrival = plan.arrival.as_ref().or(plan.departure.as_ref());
        let (along, cross, height, runway_heading) =
            arrival.map_or((0., 0., r.height_agl_m, r.heading_grid_rad), |a| {
                let (along, cross) = a.relative(position);
                (
                    along,
                    cross,
                    r.altitude_msl_m + a.threshold_m.get(2).copied().unwrap_or(0.),
                    a.heading_grid_rad,
                )
            });
        let side = if self.pattern_side < 0. { -1. } else { 1. };
        let pattern_cross = cross * side;
        let pattern_height = arrival.map_or(304.8, |a| a.pattern_altitude_m + a.threshold_m[2]);
        let departure_reference = if self.state.phase == PilotPhase::GoAround {
            plan.arrival.as_ref()
        } else {
            plan.departure.as_ref()
        };
        let (dep_along, dep_cross, dep_height, dep_heading) =
            departure_reference.map_or((along, cross, height, runway_heading), |a| {
                let (along, cross) = a.relative(position);
                (
                    along,
                    cross,
                    r.altitude_msl_m + a.threshold_m.get(2).copied().unwrap_or(0.),
                    a.heading_grid_rad,
                )
            });
        let mut phase = self.state.phase;
        if phase == PilotPhase::Takeoff && r.on_ground && !departure_attitude_valid(s) {
            let waiting = "Waiting for the attitude display to align.";
            self.last_speech = r.time_s;
            self.change(s, PilotPhase::BeforeTakeoff, waiting);
            s.crew_say(waiting.into());
            phase = self.state.phase;
        }
        if matches!(phase, PilotPhase::BeforeTakeoff)
            && r.time_s - self.phase_since > 8.
            && self.departure_authorized(s)
        {
            let systems = s.sim.systems();
            if !s.sim.canopy().secure()
                || !systems.engine_running
                || r.failures.engine_power_fraction < 0.95
                || !systems.controls.master
                || !systems.controls.avionics
            {
                self.state.reason =
                    "Before-takeoff checks failed: canopy, engine, power, master, or avionics".into();
                self.state.phase = PilotPhase::Refused;
                return;
            }
            if !departure_attitude_valid(s) {
                let waiting = "Waiting for the attitude display to align.";
                if self.state.reason != waiting {
                    s.crew_say(waiting.into());
                    self.last_speech = r.time_s;
                }
                self.state.reason = waiting.into();
                // Continue through the normal BeforeTakeoff guidance below: brakes held,
                // idle power. A failed source cannot release this gate either.
            } else {
                self.change(s,PilotPhase::Takeoff,"Checks complete: engine, electrical power, runway alignment, performance, and wind. Taking off.");
                phase = self.state.phase;
            }
        }
        if phase == PilotPhase::Takeoff && dep_height > 15. && !r.on_ground {
            self.change(
                s,
                PilotPhase::Climb,
                &format!(
                    "Climbing at {} knots, best rate. We'll level at {} feet.",
                    crate::comms::phraseology::digits((performance.vy_mps / 0.514444).round() as u32, 1),
                    crate::comms::phraseology::altitude((plan.altitude_m / 0.3048).round() as u32)
                ),
            );
            phase = self.state.phase;
        }
        if matches!(phase, PilotPhase::Climb | PilotPhase::GoAround)
            && !(matches!(self.state.intent, Some(PilotIntent::Approach { .. }))
                && s.sim.procedure_receiver().missed_approach)
            && dep_height
                > plan
                    .departure
                    .as_ref()
                    .or(arrival)
                    .map_or(290., |a| a.pattern_altitude_m + a.threshold_m[2] - 15.)
            && dep_along > 2800.
        {
            if phase == PilotPhase::GoAround || plan.legs.is_empty() && plan.arrival.is_some() {
                self.change(
                    s,
                    PilotPhase::Crosswind,
                    "Turning left crosswind, clear of the runway at pattern altitude.",
                );
            } else if plan.legs.is_empty() {
                self.change(
                    s,
                    PilotPhase::Hold,
                    "Takeoff complete. Holding the departure heading and planned altitude.",
                );
            } else {
                self.change(
                    s,
                    PilotPhase::Cruise,
                    "Turning on course. I'll follow the checked route.",
                );
            }
            phase = self.state.phase;
        }
        if phase == PilotPhase::Crosswind && pattern_cross < -1200. {
            self.change(
                s,
                PilotPhase::Downwind,
                "Joining left downwind at pattern altitude.",
            );
            phase = self.state.phase;
        }
        if phase == PilotPhase::Downwind && along < -2400. && !self.radio.extend_downwind {
            self.change(
                s,
                PilotPhase::Base,
                "Turning left base. Reducing speed and descending toward final.",
            );
            phase = self.state.phase;
        }
        if phase == PilotPhase::Base && pattern_cross > -250. {
            self.change(
                s,
                PilotPhase::Final,
                "Turning final and tracking the runway centerline.",
            );
            phase = self.state.phase;
        }
        if matches!(phase, PilotPhase::Final | PilotPhase::Flare)
            && along > -1200.
            && !matches!(self.state.intent, Some(PilotIntent::Approach { .. }))
            && !self.landing_authorized(s)
        {
            self.change(
                s,
                PilotPhase::GoAround,
                "No landing clearance received. Going around and remaining in the circuit.",
            );
            phase = self.state.phase;
        }
        if matches!(phase, PilotPhase::Final | PilotPhase::Flare) && r.on_ground {
            let touchdown = Touchdown {
                time_s: r.time_s,
                distance_m: along,
                centreline_m: cross,
                sink_mps: self.airborne_sink_mps,
                within_limits: arrival.is_some_and(|a| {
                    (0. ..a.length_m.min(600.)).contains(&along)
                        && cross.abs() <= a.width_m.min(20.) * 0.5
                        && self.airborne_sink_mps <= 2.
                }),
            };
            self.state.touchdown = Some(touchdown);
            self.change(
                s,
                PilotPhase::Rollout,
                "Maintaining centerline and braking to taxi speed.",
            );
            phase = self.state.phase;
        }
        if phase == PilotPhase::Final && along > -80. && r.height_agl_m < 8. {
            self.change(s, PilotPhase::Flare, "Reducing sink rate for touchdown.");
            phase = self.state.phase;
        }
        if phase == PilotPhase::Rollout && r.ground_speed_mps < 2.0 {
            self.taxi_exit = arrival.and_then(|a| self.safe_exit(s, a, along));
            if self.taxi_exit.is_some() {
                self.change(
                    s,
                    PilotPhase::TaxiClear,
                    "Taxiing clear on a terrain-checked exit at walking pace.",
                );
            } else {
                self.change(
                    s,
                    PilotPhase::Complete,
                    "Stopped on the runway: no safe taxi-off path in the terrain data. I'll hold the brakes.",
                );
            }
            phase = self.state.phase;
        }
        if phase == PilotPhase::TaxiClear && arrival.is_some_and(|a| cross.abs() > a.width_m * 0.5 + 6.0) {
            self.state.taxi_clear = true;
            self.change(
                s,
                PilotPhase::Complete,
                "Clear of the runway. Bringing us to a stop.",
            );
            phase = self.state.phase;
        }
        if phase == PilotPhase::Parking
            && self.radio.parking_target.is_some_and(|[tn, te, _]| {
                let [n, e, _] = position;
                libm::hypot(tn - n, te - e) < 2.
            })
        {
            self.change(s, PilotPhase::Complete, "At parking. Holding brakes.");
            phase = self.state.phase;
        }
        if phase == PilotPhase::Complete && self.radio.parking_authorized && r.ground_speed_mps < 0.2 {
            self.state.parked = true;
        }
        if self.state.director
            && phase == PilotPhase::Final
            && along > -250.
            && let Some(camera) = &mut s.camera
        {
            camera.mode = CameraMode::Tower;
        }
        let mut desired_heading = r.heading_grid_rad;
        let mut desired_altitude = plan.altitude_m;
        let mut desired_speed = (performance.vno_mps * 0.73).max(performance.vy_mps);
        let approach = 1.3 * performance.vso_mps;
        match phase {
            PilotPhase::BeforeTakeoff | PilotPhase::Takeoff | PilotPhase::Climb | PilotPhase::GoAround => {
                desired_heading = dep_heading + (-dep_cross / 500.).clamp(-0.35, 0.35);
                desired_speed = performance.vy_mps;
                if phase == PilotPhase::GoAround {
                    desired_altitude = r.altitude_msl_m + (pattern_height - r.height_agl_m).max(0.);
                }
            }
            PilotPhase::Crosswind => {
                desired_heading = runway_heading
                    - side * (core::f64::consts::FRAC_PI_2 + ((along - 3100.) / 600.).clamp(-0.5, 0.5));
                desired_altitude = r.altitude_msl_m + pattern_height - height;
            }
            PilotPhase::Downwind => {
                desired_heading = runway_heading
                    + core::f64::consts::PI
                    + side * ((pattern_cross + 1500.) / 600.).clamp(-0.5, 0.5);
                desired_altitude = r.altitude_msl_m + pattern_height - height;
            }
            PilotPhase::Base => {
                desired_heading = runway_heading
                    + side * (core::f64::consts::FRAC_PI_2 + ((along + 2700.) / 600.).clamp(-0.5, 0.5));
                desired_altitude = r.altitude_msl_m + 190. - height;
                desired_speed = (approach + 5.).min(performance.vy_mps);
            }
            PilotPhase::Final | PilotPhase::Flare | PilotPhase::Rollout => {
                desired_heading = runway_heading + (-cross * 0.004).clamp(-0.4, 0.4);
                desired_altitude = r.altitude_msl_m + ((50. - along) * 0.055 + 1.).max(1.) - height;
                if phase == PilotPhase::Final && along < -80. {
                    let [n, e, _] = position;
                    desired_altitude = desired_altitude.max(s.sim.world().ground(n, e).height_msl_m + 8.0);
                }
                desired_speed = approach;
                // A missed stabilisation gate calls a real go-around, never a position reset.
                if phase == PilotPhase::Final
                    && along > -900.
                    && height > 80.
                    && !matches!(self.state.intent, Some(PilotIntent::Approach { .. }))
                {
                    self.change(
                        s,
                        PilotPhase::GoAround,
                        "High at the stabilisation gate. Going around for another pattern.",
                    );
                }
            }
            PilotPhase::Cruise | PilotPhase::Descent | PilotPhase::Entry | PilotPhase::Orbit => {
                if let Some(target) = plan.legs.get(self.state.leg) {
                    let [tn, te, td] = target.position_m;
                    let [n, e, _] = position;
                    let distance = libm::hypot(tn - n, te - e);
                    desired_heading = libm::atan2(te - e, tn - n);
                    let entry = target.name.contains("downwind") || target.name.contains("intercept");
                    desired_altitude = -td;
                    if phase == PilotPhase::Cruise && !entry {
                        if self
                            .cruise_level_m
                            .is_some_and(|old| (old - desired_altitude).abs() > 30.)
                            && r.time_s - self.last_speech >= 12.
                        {
                            let course = s
                                .sim
                                .heading_reference()
                                .grid_to_magnetic(desired_heading)
                                .rem_euclid(core::f64::consts::TAU);
                            s.crew_say(format!(
                                "Turning {}bound. We'll use {} feet for this cruising leg.",
                                if course < core::f64::consts::PI {
                                    "east"
                                } else {
                                    "west"
                                },
                                crate::comms::phraseology::altitude(
                                    (desired_altitude / 0.3048).round() as u32
                                )
                            ));
                            self.last_speech = r.time_s;
                        }
                        self.cruise_level_m = Some(desired_altitude);
                    }
                    if let Some(arrival) = &plan.arrival {
                        let mut remaining = distance;
                        for (a, b) in plan
                            .legs
                            .iter()
                            .skip(self.state.leg)
                            .zip(plan.legs.iter().skip(self.state.leg.saturating_add(1)))
                        {
                            let [an, ae, _] = a.position_m;
                            let [bn, be, _] = b.position_m;
                            remaining += libm::hypot(bn - an, be - ae);
                        }
                        let pattern_altitude = arrival.pattern_altitude_m;
                        desired_altitude = desired_altitude.min(pattern_altitude + remaining * 0.04);
                        if !entry {
                            let terrain = s.sim.world().ground(n, e).height_msl_m;
                            desired_altitude = desired_altitude.max(terrain + 304.8);
                        }
                    }
                    if entry {
                        desired_altitude = desired_altitude.min(plan.altitude_m);
                    }
                    if phase == PilotPhase::Cruise
                        && plan.arrival.is_some()
                        && desired_altitude < plan.altitude_m - 100.
                    {
                        let feet = plan
                            .arrival
                            .as_ref()
                            .map_or(plan.altitude_m, |a| a.pattern_altitude_m)
                            / 0.3048;
                        self.change(
                            s,
                            PilotPhase::Descent,
                            &format!(
                                "Descending for pattern entry at {} feet.",
                                crate::comms::phraseology::altitude(feet.round() as u32)
                            ),
                        );
                    }
                    if distance < 350. {
                        self.state.leg += 1;
                        if self.state.leg >= plan.legs.len() {
                            match self.state.intent {
                                Some(PilotIntent::Orbit { .. }) => self.state.leg = 0,
                                Some(PilotIntent::JoinPattern { .. }) => self.change(
                                    s,
                                    PilotPhase::Downwind,
                                    "Pattern entry complete; following the left circuit.",
                                ),
                                _ if plan.arrival.is_some() => self.change(
                                    s,
                                    match self.radio.entry {
                                        Some(crate::PatternEntry::StraightIn) => PilotPhase::Final,
                                        Some(
                                            crate::PatternEntry::LeftBase | crate::PatternEntry::RightBase,
                                        ) => PilotPhase::Base,
                                        _ => PilotPhase::Downwind,
                                    },
                                    "Established left downwind. I'll fly the base and final turns.",
                                ),
                                _ => self.change(
                                    s,
                                    PilotPhase::Hold,
                                    "Route complete. Holding our last heading and altitude.",
                                ),
                            }
                        } else if entry {
                            self.change(
                                s,
                                PilotPhase::Entry,
                                "Joining on a forty-five-degree entry to left downwind.",
                            );
                        }
                    }
                    self.state.reason = format!(
                        "{}: {:.1} NM remaining; target {:.0} ft MSL{}",
                        target.name,
                        distance / 1852.,
                        desired_altitude / 0.3048,
                        self.last_radio_error
                            .as_ref()
                            .map_or(String::new(), |e| format!("; radio: {e}"))
                    );
                }
            }
            PilotPhase::Hold => {
                desired_heading = if let Some(PilotIntent::Hold { heading_rad, .. }) = self.state.intent {
                    s.sim.heading_reference().magnetic_to_grid(heading_rad)
                } else {
                    self.hold_heading_grid_rad.unwrap_or(dep_heading)
                };
            }
            _ => {}
        }
        let instrument = self.instrument_guidance(s, phase);
        if let Some((track, altitude, speed)) = instrument {
            desired_heading = track;
            desired_altitude = altitude;
            desired_speed = speed;
        }
        let (assigned_heading, assigned_altitude) = self.radio_guidance(s);
        if let Some(heading) = assigned_heading {
            desired_heading = heading;
        }
        if let Some(altitude) = assigned_altitude {
            desired_altitude = altitude;
        }
        // Aircraft V-speeds are indicated. Guidance and wind correction use true speed.
        if r.indicated_airspeed_mps > 5.0 {
            desired_speed *= (r.true_airspeed_mps / r.indicated_airspeed_mps).clamp(0.8, 1.4);
        }
        // Track guidance includes measured wind: heading is crabbed into the crosswind.
        let (sin, cos) = libm::sincos(desired_heading);
        let [wn, we, _] = r.wind_ned_mps;
        let wind_cross = -wn * sin + we * cos;
        let crab = libm::asin((-wind_cross / desired_speed.max(15.)).clamp(-0.5, 0.5));
        if !r.on_ground
            && (phase != PilotPhase::Hold || matches!(self.state.intent, Some(PilotIntent::HoldAtFix { .. })))
            && self.radio.heading.is_none()
        {
            desired_heading += crab;
        }
        let path_descent = if phase == PilotPhase::Final {
            self.instrument_angle(s).unwrap_or(0.055) * r.ground_speed_mps
        } else {
            0.0
        };
        let desired_climb = ((desired_altitude - r.altitude_msl_m) * 0.10 - path_descent)
            .clamp(if instrument.is_some() { -4.5 } else { -3. }, 3.5);
        let desired_climb = if phase == PilotPhase::Flare {
            let [n, e, _] = position;
            let [gn, ge, gd] = s.sim.world().ground(n, e).normal_ned;
            let [vn, ve, _] = s.sim.state().velocity_mps;
            let surface_rate = if gd.abs() > 0.1 {
                (gn * vn + ge * ve) / gd
            } else {
                0.0
            };
            (surface_rate - (0.55 + r.height_agl_m * 0.12).clamp(0.55, 1.2)).clamp(-1.8, 0.5)
        } else {
            desired_climb
        };
        let bank = if r.on_ground {
            0.0
        } else {
            (wrap(desired_heading - r.heading_grid_rad) * 1.1).clamp(-0.40, 0.40)
        };
        // Alpha follows the air mass; the flare sink target is relative to the ground.
        // Cancel measured vertical wind so an updraft cannot float us along the runway.
        let vertical_wind = if matches!(phase, PilotPhase::Final | PilotPhase::Flare) {
            r.wind_ned_mps[2]
        } else {
            0.0
        };
        let mut target_pitch = (r.alpha_rad
            + (desired_climb + vertical_wind) / r.true_airspeed_mps.max(25.)
            + (desired_climb - r.vertical_speed_mps) * 0.015)
            .clamp(-0.12, 0.26);
        if matches!(phase, PilotPhase::BeforeTakeoff | PilotPhase::Takeoff) {
            target_pitch = if r.indicated_airspeed_mps < performance.vr_mps {
                if tailwheel { 0.08 } else { 0.0 }
            } else {
                0.20
            };
        }
        let pitch_error = target_pitch - r.pitch_rad;
        if !r.on_ground
            && !matches!(
                phase,
                PilotPhase::Rollout | PilotPhase::TaxiClear | PilotPhase::Parking | PilotPhase::Complete
            )
        {
            self.pitch_trim = (self.pitch_trim + pitch_error * 0.4 * dt).clamp(-0.25, 0.35);
            self.throttle_trim =
                (self.throttle_trim + (desired_speed - r.true_airspeed_mps) * 0.02 * dt).clamp(-0.6, 0.4);
        }
        self.instrument_coupling(s, phase);
        let mut elevator = self.pitch_trim + 2.2 * pitch_error - 0.8 * q;
        let mut throttle =
            (0.6 + self.throttle_trim + 0.08 * (desired_speed - r.true_airspeed_mps)).clamp(0., 1.);
        if phase == PilotPhase::Takeoff
            || phase == PilotPhase::GoAround
                && !(instrument.is_some() && s.sim.procedure_receiver().missed_approach)
        {
            throttle = 1.;
        }
        if phase == PilotPhase::Flare && r.height_agl_m < 4. {
            throttle = 0.;
        }
        let mut rudder = if r.on_ground {
            wrap(desired_heading - r.heading_grid_rad) * 0.9
                - 0.7 * yaw_rate
                - if matches!(phase, PilotPhase::BeforeTakeoff | PilotPhase::Takeoff) {
                    dep_cross
                } else {
                    cross
                } * 0.02
        } else {
            0.65 * (9.80665 * libm::tan(r.roll_rad) / r.true_airspeed_mps.max(15.) - yaw_rate) - r.beta_rad
        };
        let mut brake = 0.;
        if matches!(
            phase,
            PilotPhase::BeforeTakeoff | PilotPhase::Rollout | PilotPhase::Complete | PilotPhase::Refused
        ) {
            let rollout_pitch = if tailwheel {
                ground_pitch
            } else if r.ground_speed_mps > 10.0 {
                0.06
            } else {
                0.0
            };
            elevator = self.pitch_trim + 2.2 * (rollout_pitch - r.pitch_rad) - 0.8 * q;
            throttle = 0.;
            brake = if tailwheel { 0.25 } else { 0.5 };
        }
        if matches!(phase, PilotPhase::TaxiClear | PilotPhase::Parking) {
            elevator = self.pitch_trim + 2.2 * (ground_pitch - r.pitch_rad) - 0.8 * q;
            throttle = (0.10 + (1.5 - r.ground_speed_mps) * 0.12).clamp(0.0, 0.25);
            brake = if r.ground_speed_mps > 2.0 { 0.2 } else { 0.0 };
            let taxi_heading = if phase == PilotPhase::Parking {
                self.radio.parking_target.map_or(runway_heading, |[tn, te, _]| {
                    let [n, e, _] = position;
                    libm::atan2(te - e, tn - n)
                })
            } else {
                self.taxi_exit
                    .map_or(runway_heading, |(exit_along, [tn, te, _])| {
                        if along < exit_along - 3.0 {
                            runway_heading + (-cross / 20.0).clamp(-0.3, 0.3)
                        } else {
                            let [n, e, _] = position;
                            libm::atan2(te - e, tn - n)
                        }
                    })
            };
            rudder = wrap(taxi_heading - r.heading_grid_rad) * 1.6 - 0.8 * yaw_rate;
        }
        let flaps = if r.indicated_airspeed_mps < performance.vfe_mps - 2.0 {
            match phase {
                PilotPhase::Base => 0.5,
                PilotPhase::Final | PilotPhase::Flare | PilotPhase::Rollout => 1.0,
                PilotPhase::GoAround if r.height_agl_m < 15.0 => 0.5,
                _ => 0.0,
            }
        } else {
            0.0
        };
        let controls = Controls {
            elevator,
            aileron: 1.6 * (bank - r.roll_rad) - 0.4 * p,
            rudder,
            throttle,
            flaps,
            brake,
        }
        .clamped();
        let command = SetControls {
            elevator: Some(controls.elevator),
            aileron: Some(controls.aileron),
            rudder: Some(controls.rudder),
            throttle: Some(controls.throttle),
            flaps: Some(controls.flaps),
            brake: Some(controls.brake),
        };
        if let Err(error) = command.run(s) {
            self.state.reason = error.to_string();
        }
        // Brief descending callouts at measured heights, with no invented "minimums".
        if matches!(phase, PilotPhase::Final | PilotPhase::Flare) && r.vertical_speed_mps < 0. {
            for (bit, threshold, text) in [(1, 152.4, "Five hundred."), (2, 30.48, "One hundred.")] {
                if self.callouts & bit == 0
                    && previous_agl.is_some_and(|h| h > threshold)
                    && r.height_agl_m <= threshold
                {
                    s.crew_callout(text.into());
                    self.callouts |= bit;
                }
            }
        }
        self.narrate_landmarks(s, phase, position, r.heading_grid_rad, r.time_s);
    }
}

/// Check every installed attitude source, including independent standby computers.
fn departure_attitude_valid(s: &Session) -> bool {
    let systems = s.sim.systems();
    let installed = |id: &str| systems.electrical.loads.iter().any(|load| load.id == id);
    let glass = s.sim.glass_computers();
    (!installed("glass_ahrs") || glass.primary.attitude_valid)
        && (!installed("glass_pfd") || !glass.pfd_failed)
        && (!installed("glass_standby") || glass.standby.attitude_valid)
        && (!systems.avionics.installed
            || (systems.avionics.electronic_attitude.attitude_valid
                && systems.avionics.electronic_hsi.reversion_attitude.attitude_valid))
        && (installed("glass_ahrs") || systems.avionics.installed || systems.sensed.attitude_valid)
}
