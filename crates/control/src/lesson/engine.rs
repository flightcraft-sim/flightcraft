use super::*;
use flightcraft_world::{geo::LocalProjection, procedures::ProcedureKind};
/// Answered traffic advisories needed before the radio task can complete.
const RADIO_ADVISORIES_REQUIRED: u32 = 2;
#[derive(Clone, Debug)]
pub(crate) struct ApproachPath {
    pub endpoint: GeoPoint,
    pub course_deg: f64,
    pub angle_deg: f64,
    pub end_altitude_m: f64,
    pub faf_distance_nm: f64,
}
impl Session {
    pub(crate) fn prepare_instrument_receiver(
        &mut self,
        airport: &str,
        procedure: &str,
        mode: InstrumentApproach,
    ) -> Result<(), CommandError> {
        self.execute("avionics.set", serde_json::json!({"setting":{"kind":"load_procedure","airport":airport,"procedure":procedure,"vectors_to_final":true}}))?;
        self.execute(
            "avionics.set",
            serde_json::json!({"setting":{"kind":"vectors_to_final"}}),
        )?;
        self.execute("avionics.set", serde_json::json!({"setting":{"kind":"approach_service","lnav_only":mode == InstrumentApproach::Lnav}}))?;
        let radio_mode = matches!(mode, InstrumentApproach::Ils | InstrumentApproach::Localizer);
        if radio_mode {
            let data = self
                .sim
                .world()
                .procedures()
                .ok_or_else(|| invalid("lesson", "procedures unavailable"))?;
            let runway = data
                .procedures
                .iter()
                .find(|p| p.airport == airport && p.ident == procedure)
                .and_then(|p| p.runway.as_ref());
            let loc = data
                .localizers
                .iter()
                .find(|l| l.airport == airport && Some(&l.runway) == runway)
                .cloned()
                .ok_or_else(|| invalid("lesson", "localizer unavailable"))?;
            for setting in [
                serde_json::json!({"kind":"standby","radio":1,"frequency_hz":loc.frequency_hz as u32}),
                serde_json::json!({"kind":"swap","radio":1}),
                serde_json::json!({"kind":"course","radio":1,"degrees":loc.course_magnetic_deg}),
            ] {
                self.execute(
                    "avionics.set",
                    serde_json::json!({"setting":{"kind":"nav_radio","setting":setting}}),
                )?;
            }
        }
        self.execute(
            "avionics.set",
            serde_json::json!({"setting":{"kind":"cdi_source","source":if radio_mode {"nav1"} else {"gps"}}}),
        )?;
        Ok(())
    }
}
impl ApproachPath {
    pub(crate) fn find(s: &Session, airport: &str, procedure: &str) -> Result<Self, CommandError> {
        let d = s
            .sim()
            .world()
            .procedures()
            .ok_or_else(|| invalid("lesson", "region has no procedures"))?;
        let p = d
            .procedures
            .iter()
            .find(|p| p.airport == airport && p.ident == procedure && p.kind == ProcedureKind::Approach)
            .ok_or_else(|| invalid("lesson", "approach unavailable in this region"))?;
        let route = p
            .routes
            .iter()
            .find(|r| r.transition.is_empty())
            .ok_or_else(|| invalid("lesson", "approach has no final route"))?;
        let final_leg = route
            .legs
            .iter()
            .rev()
            .find(|l| !l.missed && l.fix.is_some())
            .ok_or_else(|| invalid("lesson", "no final approach endpoint"))?;
        let endpoint = d
            .point(final_leg.fix.as_deref().unwrap_or(""))
            .ok_or_else(|| invalid("lesson", "unresolved final endpoint"))?
            .position;
        let faf = route
            .legs
            .iter()
            .find(|l| l.waypoint_description.as_bytes().get(3) == Some(&b'F'))
            .and_then(|l| d.point(l.fix.as_deref()?))
            .ok_or_else(|| invalid("lesson", "no final approach fix"))?
            .position;
        let (distance, course) = faf.inverse(endpoint);
        Ok(Self {
            endpoint,
            course_deg: course,
            angle_deg: final_leg.vertical_angle_deg.unwrap_or(-3.).abs().max(0.1),
            end_altitude_m: final_leg.altitude1_m.unwrap_or(0.),
            faf_distance_nm: distance / 1852.,
        })
    }
    pub(crate) fn remaining(&self, position: GeoPoint) -> f64 {
        let (distance, bearing) = self.endpoint.inverse(position);
        -distance * libm::cos((bearing - self.course_deg).to_radians())
    }
    pub(crate) fn altitude(&self, remaining_m: f64) -> f64 {
        self.end_altitude_m + remaining_m.max(0.) * libm::tan(self.angle_deg.to_radians())
    }
}
fn validate(doc: &Lesson, s: &Session) -> Result<(), CommandError> {
    let bad = |m| invalid(LessonStart::NAME, m);
    if doc.version != 1
        || doc.id.is_empty()
        || doc.id.len() > 96
        || doc.title.is_empty()
        || doc.title.len() > 256
        || doc.briefing.len() > 1024
        || doc.tasks.is_empty()
        || doc.tasks.len() > 16
        || doc.setup.failures.len() > 8
    {
        return Err(bad("invalid lesson bounds"));
    }
    range(LessonStart::NAME, "minimum_ft", doc.setup.minimum_ft, 0., 18000.)?;
    range(
        LessonStart::NAME,
        "start_distance_nm",
        doc.setup.start_distance_nm,
        3.,
        20.,
    )?;
    range(LessonStart::NAME, "airspeed_kt", doc.setup.airspeed_kt, 50., 160.)?;
    range(
        LessonStart::NAME,
        "cross_track_nm",
        doc.setup.cross_track_nm,
        -1.8,
        1.8,
    )?;
    if let Some(altitude) = doc.setup.start_altitude_ft {
        range(LessonStart::NAME, "start_altitude_ft", altitude, 1000., 18000.)?;
    }
    doc.setup.conditions.validate().map_err(SimError::from)?;
    doc.setup.wind.validate().map_err(SimError::from)?;
    for task in &doc.tasks {
        let t = task.tolerances;
        if task.acs != task.kind.acs()
            || task.title.is_empty()
            || task.title.len() > 256
            || t.altitude_ft != 100.
            || t.heading_deg != 10.
            || t.airspeed_kt != 10.
            || t.cdi_scale != 0.75
        {
            return Err(bad("task code or ACS tolerance does not match FAA-S-ACS-8"));
        }
    }
    let report = crate::scenario::validate(&doc.scenario, s);
    if !report.valid {
        return Err(bad(&report.errors.join("; ")));
    }
    ApproachPath::find(s, &doc.setup.airport, &doc.setup.procedure)?;
    Ok(())
}
impl Command for LessonStart {
    const NAME: &'static str = "lesson.start";
    const DESCRIPTION: &'static str = "Validate and start an IFR lesson atomically, applying its region, aircraft, approach setup, weather, hood, failures, and scenario.";
    type Output = LessonDebrief;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        // Build on a separate session so hostile content cannot partially reset the current flight.
        let mut next = Session::new()?;
        next.navigation.date = s.navigation.date;
        next.navigation.pins = s.navigation.pins.clone();
        next.regions = s.regions.clone();
        let world = next.resolve_world(WorldChoice::Region(self.lesson.setup.region.clone()))?;
        next.sim = Sim::new_with_aircraft_weather_world(
            &Start::Runway,
            next.selected_aircraft.clone(),
            Default::default(),
            world,
        )?;
        validate(&self.lesson, &next)?;
        next.execute(
            "aircraft.load",
            serde_json::json!({"source":{"kind":"installed","id":self.lesson.setup.aircraft},"reset":true}),
        )?;
        let setup = &self.lesson.setup;
        let path = ApproachPath::find(&next, &setup.airport, &setup.procedure)?;
        let holding = self
            .lesson
            .tasks
            .iter()
            .any(|t| t.kind == InstrumentTaskKind::Holding);
        let hold_setup = if holding {
            Some(PublishedHold::find(
                &next,
                &setup.airport,
                &setup.procedure,
                setup
                    .hold_fix
                    .as_deref()
                    .ok_or_else(|| invalid(Self::NAME, "holding lesson must declare its published fix"))?,
            )?)
        } else {
            None
        };
        let position = hold_setup.as_ref().map_or_else(
            || {
                path.endpoint
                    .destination(path.course_deg + 180., setup.start_distance_nm * 1852.)
                    .destination(path.course_deg + 90., setup.cross_track_nm * 1852.)
            },
            |h| h.position.destination(h.inbound_true_deg + 180., 2. * 1852.),
        );
        let region = next
            .sim
            .world()
            .region()
            .ok_or_else(|| invalid(Self::NAME, "region missing"))?;
        let projection = LocalProjection::new(region.origin.latitude_deg, region.origin.longitude_deg)
            .map_err(SimError::from)?;
        let [n, e] = projection
            .project(position.latitude_deg, position.longitude_deg)
            .map_err(SimError::from)?;
        let altitude = hold_setup.as_ref().map_or_else(
            || {
                setup.start_altitude_ft.map_or_else(
                    || path.altitude(setup.start_distance_nm * 1852.),
                    |ft| ft * 0.3048,
                )
            },
            |h| h.altitude_m,
        );
        let ground = next.sim.world().ground(n, e).height_msl_m;
        let reference = flightcraft_sim::magnetic::reference_at(
            next.sim.world(),
            [n, e, -altitude],
            next.simulation_date(),
        )
        .map_err(SimError::from)?;
        let (_, grid_north_bearing) = position.inverse(GeoPoint {
            latitude_deg: projection
                .unproject(n + 10., e)
                .map_err(SimError::from)?
                .first()
                .copied()
                .unwrap_or(position.latitude_deg),
            longitude_deg: projection
                .unproject(n + 10., e)
                .map_err(SimError::from)?
                .get(1)
                .copied()
                .unwrap_or(position.longitude_deg),
        });
        let heading = reference
            .grid_to_magnetic((path.course_deg - grid_north_bearing).to_radians())
            .rem_euclid(core::f64::consts::TAU);
        next.execute("sim.reset", serde_json::json!({"world":setup.region,"weather":setup.wind,"start":{"kind":"geographic_air","position":position,"height_agl_m":altitude-ground,"airspeed_mps":setup.airspeed_kt*0.514444,"heading_rad":heading}}))?;
        // An airborne lesson starts with the preflight instrument preparation completed.
        next.sim.prepare_departure();
        next.prepare_instrument_receiver(&setup.airport, &setup.procedure, setup.mode)?;
        // Airborne IFR continuation: the filed clearance and assigned code precede this lesson.
        next.comms.atc.airport = Some(setup.airport.clone());
        next.comms.atc.ifr_clearance = true;
        next.execute("avionics.set", serde_json::json!({"setting":{"kind":"glass","setting":{"kind":"transponder","code":"4201","mode":"alt"}}}))?;
        next.sim.set_conditions(setup.conditions.clone())?;
        next.sim.set_hood(setup.hood);
        for failure in &setup.failures {
            apply_failure(&mut next, *failure)?;
        }
        next.sequence_comms();
        if let Some(traffic) = setup.traffic {
            next.start_lesson_traffic(&setup.airport, traffic.density)?;
        }
        next.run(ScenarioStart {
            scenario: self.lesson.scenario.clone(),
        })?;
        let geometry = geometry(&next, &setup.airport, &setup.procedure);
        let tasks = self
            .lesson
            .tasks
            .iter()
            .map(|t| TaskGrade {
                title: t.title.clone(),
                kind: t.kind,
                acs: t.acs.clone(),
                tolerances: t.tolerances,
                result: TaskResult::Pending,
                reasons: vec![],
                observed_s: 0.,
                completed: false,
            })
            .collect();
        next.lesson = LessonRuntime {
            document: Some(self.lesson.clone()),
            debrief: LessonDebrief {
                title: Some(self.lesson.title),
                briefing: self.lesson.briefing,
                tasks,
                geometry,
                notes: vec!["FAA-S-ACS-8 task tolerances; simulated practice only.".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        if self
            .lesson
            .tasks
            .iter()
            .any(|t| t.kind == InstrumentTaskKind::Holding)
        {
            let hold = PublishedHold::find(
                &next,
                &setup.airport,
                &setup.procedure,
                setup
                    .hold_fix
                    .as_deref()
                    .ok_or_else(|| invalid(Self::NAME, "holding lesson must declare its published fix"))?,
            )?;
            next.lesson.hold_monitor.spec = Some(hold);
            next.lesson.debrief.hold = Some(Default::default());
        }
        next.crew.state.mode = CrewMode::Instructing;
        next.camera = s.camera.map(|mut c| {
            c.mode = CameraMode::Cockpit;
            c
        });
        next.screens = s.screens.clone();
        next.paused = true;
        next.flight_generation = s.flight_generation.wrapping_add(1);
        next.sync_comms_generation();
        next.scenario.generation = next.flight_generation;
        // Preserve host/bridge and user preferences; only replace flight-owned state.
        if s.flight_archive.autosave_enabled {
            s.save_flight()?;
        }
        s.sim = next.sim;
        s.crew = next.crew;
        s.scenario = next.scenario;
        s.lesson = next.lesson;
        s.comms = next.comms;
        s.glass = next.glass;
        s.touch = next.touch;
        s.navigation = next.navigation;
        s.selected_aircraft = next.selected_aircraft;
        s.selected_configuration = next.selected_configuration;
        s.active_configuration = next.active_configuration;
        s.camera = next.camera;
        s.paused = true;
        s.rate = 1.;
        s.flight_generation = next.flight_generation;
        s.start_local(
            Channel::Intercom,
            Speaker::Instructor,
            s.lesson.debrief.briefing.clone(),
            Meaning::Text,
            EventSource::Rules,
        );
        Ok(s.lesson.debrief.clone())
    }
}
fn geometry(s: &Session, airport: &str, procedure: &str) -> Vec<ProcedureGeometry> {
    let Some(d) = s.sim.world().procedures() else {
        return vec![];
    };
    d.procedures
        .iter()
        .filter(|p| p.airport == airport && p.ident == procedure)
        .flat_map(|p| &p.routes)
        .filter(|r| r.transition.is_empty())
        .flat_map(|r| &r.legs)
        .filter_map(|l| {
            Some(ProcedureGeometry {
                name: d.point(l.fix.as_deref()?)?.ident.clone(),
                missed: l.missed,
                position: d.point(l.fix.as_deref()?)?.position,
            })
        })
        .collect()
}
fn apply_failure(s: &mut Session, failure: InstrumentFailure) -> Result<(), CommandError> {
    match failure {
        InstrumentFailure::Vacuum => {
            s.run(SetFailure {
                failure: Failure::VacuumPump {},
            })?;
        }
        InstrumentFailure::Ahrs | InstrumentFailure::Display => {
            s.execute("avionics.set", serde_json::json!({"setting":{"kind":"glass","setting":{"kind":"failure","target":if matches!(failure, InstrumentFailure::Ahrs) {"ahrs"} else {"pfd"},"failed":true}}}))?;
        }
    }
    s.record_lesson_event(InstrumentEvent::Failure { failure });
    Ok(())
}
impl Command for InstructorInstrumentFailure {
    const NAME: &'static str = "lesson.instructor_failure";
    const DESCRIPTION: &'static str = "Schedule a vacuum, AHRS, or primary display failure at an exact or seeded moment in simulated seconds.";
    type Output = LessonDebrief;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        range(Self::NAME, "earliest_s", self.earliest_s, 0., 3600.)?;
        range(Self::NAME, "latest_s", self.latest_s, self.earliest_s, 3600.)?;
        let mixed = self.seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let delay =
            self.earliest_s + (self.latest_s - self.earliest_s) * f64::from(mixed) / f64::from(u32::MAX);
        if delay == 0. {
            apply_failure(s, self.failure)?;
        } else {
            if s.lesson.failure_due.len() >= 8 {
                return Err(invalid(
                    Self::NAME,
                    "at most eight instrument failures may be scheduled",
                ));
            }
            s.lesson
                .failure_due
                .push((s.sim.state().time_s + delay, self.failure));
        }
        Ok(s.lesson.debrief.clone())
    }
}
impl Command for InstructorAttitude {
    const NAME: &'static str = "lesson.instructor_attitude";
    const DESCRIPTION: &'static str = "Set an airborne unusual attitude under the hood and freeze for instructor handback; preserves systems, weather, and flight time.";
    type Output = LessonDebrief;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        range(Self::NAME, "pitch_deg", self.pitch_deg, -25., 25.)?;
        range(Self::NAME, "bank_deg", self.bank_deg, -60., 60.)?;
        if s.sim.readout().on_ground || s.sim.readout().height_agl_m < 609.6 || s.crew_owns_controls() {
            return Err(invalid(
                Self::NAME,
                "pilot must have controls, airborne at least 2,000 feet above terrain",
            ));
        }
        if s.sim.installed_autopilot().is_some() {
            s.execute(
                "avionics.set",
                serde_json::json!({"setting":{"kind":"autopilot","setting":{"kind":"disconnect"}}}),
            )?;
        }
        s.sim
            .set_training_attitude(self.pitch_deg.to_radians(), self.bank_deg.to_radians())?;
        s.sim.set_hood(true);
        s.paused = true;
        s.record_lesson_event(InstrumentEvent::Attitude {
            pitch_deg: self.pitch_deg,
            bank_deg: self.bank_deg,
        });
        s.lesson.recovery_started_s = Some(s.sim.state().time_s);
        s.start_local(
            Channel::Intercom,
            Speaker::Instructor,
            "Recover. Your controls.".into(),
            Meaning::Text,
            EventSource::Rules,
        );
        Ok(s.lesson.debrief.clone())
    }
}
impl Session {
    pub(crate) fn sequence_lesson(&mut self) {
        let now = self.sim.state().time_s;
        let scheduled = std::mem::take(&mut self.lesson.failure_due);
        for (due, failure) in scheduled {
            if now >= due {
                if let Err(e) = apply_failure(self, failure)
                    && self.lesson.debrief.notes.len() < 64
                {
                    self.lesson.debrief.notes.push(e.to_string());
                }
            } else {
                self.lesson.failure_due.push((due, failure));
            }
        }
        if self.lesson.document.is_none()
            || self.lesson.debrief.complete
            || self.lesson.last_sample_s.is_some_and(|t| now - t < 0.5 - 1e-8)
        {
            return;
        }
        let mut lesson = std::mem::take(&mut self.lesson);
        lesson.sample(self);
        self.lesson = lesson;
    }
}
impl LessonRuntime {
    fn sample(&mut self, s: &mut Session) {
        let Some(doc) = &self.document else {
            return;
        };
        let setup = &doc.setup;
        let r = s.sim.readout();
        let dt = self.last_sample_s.replace(r.time_s).map_or(0.5, |t| r.time_s - t);
        let (Some(latitude_deg), Some(longitude_deg)) = (r.latitude_deg, r.longitude_deg) else {
            return;
        };
        let position = GeoPoint {
            latitude_deg,
            longitude_deg,
        };
        let Ok(path) = ApproachPath::find(s, &setup.airport, &setup.procedure) else {
            return;
        };
        let remaining = path.remaining(position);
        let precision = matches!(setup.mode, InstrumentApproach::Ils | InstrumentApproach::Lpv);
        let proc = s.sim.procedure_receiver().clone();
        let missed = proc.missed_approach;
        if missed && self.missed_started_s.is_none() {
            self.missed_started_s = Some(r.time_s);
            if self
                .debrief
                .samples
                .last()
                .is_some_and(|p| p.altitude_ft <= setup.minimum_ft + 10.)
            {
                self.minimum_called = true;
            }
        }
        let altitude_ft = r.altitude_msl_m / 0.3048;
        let visual = !s.sim.conditions().in_cloud(s.sim.state().position_m)
            && remaining.abs() <= s.sim.conditions().effective_visibility_m()
            && !s.sim.hood();
        if !self.minimum_called
            && altitude_ft <= setup.minimum_ft + if precision { 0. } else { 1. }
            && !missed
        {
            self.minimum_called = true;
            if setup.look_up && s.sim.hood() {
                s.sim.set_hood(false);
                s.start_local(
                    Channel::Intercom,
                    Speaker::Instructor,
                    "Minimums. Look up.".into(),
                    Meaning::Text,
                    EventSource::Rules,
                );
            }
        }
        let radios = s.sim.radio_state();
        let radio = radios.first();
        let radio_mode = matches!(
            setup.mode,
            InstrumentApproach::Ils | InstrumentApproach::Localizer
        );
        let signal = if radio_mode {
            radio.is_some_and(|r| r.signal_valid)
        } else {
            proc.valid
        };
        let cdi = if radio_mode {
            radio.and_then(|r| r.lateral_dots).unwrap_or(2.5) / 2.5
        } else {
            proc.cross_track_nm / proc.cdi_full_scale_nm.max(0.001)
        };
        let vertical = if setup.mode == InstrumentApproach::Ils {
            radio.and_then(|r| r.vertical_dots).map(|d| d / 2.5)
        } else if setup.mode == InstrumentApproach::Lpv {
            proc.vertical_deviation_m.map(|m| m / 30.)
        } else {
            None
        };
        let grading = remaining > 0. && remaining / 1852. <= path.faf_distance_nm && !visual && !missed;
        let sample = InstrumentSample {
            time_s: r.time_s,
            interval_s: dt,
            position: Some(position),
            altitude_ft,
            altitude_error_ft: (r.altitude_msl_m
                - if precision {
                    path.altitude(remaining)
                } else {
                    (setup.minimum_ft * 0.3048).max(path.altitude(remaining))
                })
                / 0.3048,
            airspeed_error_kt: r.indicated_airspeed_mps / 0.514444 - setup.airspeed_kt,
            heading_error_deg: wrap_deg(r.heading_rad.to_degrees() - proc.desired_track_rad.to_degrees()),
            cdi_scale: cdi,
            vertical_scale: vertical,
            signal_valid: signal,
            grading,
            level_segment: !precision && path.altitude(remaining) <= setup.minimum_ft * 0.3048,
            at_minimum: altitude_ft < setup.minimum_ft - 5.,
            visual,
            missed,
            hood: s.sim.hood(),
        };
        if let Some(out) = &mut self.debrief.hold {
            self.hold_monitor
                .sample(position, r.heading_rad.to_degrees(), r.time_s, dt, out);
        }
        for grade in &mut self.debrief.tasks {
            match grade.kind {
                InstrumentTaskKind::Precision
                | InstrumentTaskKind::Nonprecision
                | InstrumentTaskKind::PartialPanel => {
                    let partial = r.failures.vacuum_pump
                        || s.sim.glass_computers().ahrs_failed
                        || s.sim.glass_computers().pfd_failed;
                    if grade.kind != InstrumentTaskKind::PartialPanel || partial {
                        grade.observe(&sample, setup.minimum_ft, precision);
                    }
                    if self.minimum_called
                        && grade.observed_s >= 10.
                        && (grade.kind != InstrumentTaskKind::PartialPanel || partial)
                    {
                        grade.done();
                    }
                }
                InstrumentTaskKind::Radio => {
                    grade.observed_s += dt;
                    for g in s.comms.atc.compliance.iter().filter(|g| {
                        !g.passed && (g.item.starts_with("radio_discipline:") || g.item == "missing_readback")
                    }) {
                        grade.fail(&g.observed);
                    }
                    let tally = s.radio_discipline();
                    if tally.advisories_answered >= RADIO_ADVISORIES_REQUIRED {
                        grade.done();
                    }
                }
                InstrumentTaskKind::LookUp => {
                    if self.minimum_called
                        && !s.sim.hood()
                        && !s.sim.conditions().in_cloud(s.sim.state().position_m)
                    {
                        grade.done();
                    }
                }
                InstrumentTaskKind::Missed => {
                    if sample.at_minimum && !visual && !missed && r.vertical_speed_mps < -0.3 {
                        grade.fail("Missed approach was not initiated at DA/MDA.");
                    }
                    if missed {
                        grade.observed_s += dt;
                        let [vn, ve, _] = s.sim.state().velocity_mps;
                        let track_true = libm::atan2(ve, vn) + r.grid_north_true_rad;
                        if !precision
                            && remaining > 0.3 * 1852.
                            && wrap_deg(track_true.to_degrees() - path.course_deg).abs() > 10.
                        {
                            grade.fail("Turned off the final course before the missed approach point.");
                        }
                        if !proc.valid {
                            grade.fail("Published missed-approach guidance unavailable.");
                        }
                        if !proc
                            .loaded
                            .iter()
                            .any(|p| p.airport == setup.airport && p.ident == setup.procedure)
                        {
                            grade.fail("Missed approach used a different procedure.");
                        }
                        if sample.airspeed_error_kt.abs() > grade.tolerances.airspeed_kt {
                            grade.fail("Missed-approach airspeed exceeded ±10 kt.");
                        }
                        if proc.cross_track_nm.abs() > proc.cdi_full_scale_nm * 0.75 {
                            grade.fail("Missed approach exceeded ¾-scale CDI.");
                        }
                        if let Some(leg) = proc.legs.get(proc.active_leg) {
                            if !leg.leg.missed {
                                grade.fail("Missed approach did not follow the published missed legs.");
                            }
                            if let Some(altitude) = leg.leg.altitude1_m
                                && r.altitude_msl_m > altitude + 30.48
                            {
                                grade.fail("Missed-approach altitude exceeded ±100 ft.");
                            }
                            if matches!(leg.leg.path, flightcraft_world::procedures::PathTerminator::HM)
                                && r.altitude_msl_m >= leg.leg.altitude1_m.unwrap_or(0.) - 30.48
                                && grade.observed_s >= 10.
                                && leg
                                    .endpoint
                                    .is_some_and(|fix| position.inverse(fix).0 <= 0.5 * 1852.)
                            {
                                grade.done();
                            }
                        }
                    }
                }
                InstrumentTaskKind::Holding => {
                    if let Some(out) = &self.debrief.hold {
                        if self
                            .hold_monitor
                            .spec
                            .as_ref()
                            .is_some_and(|h| (r.altitude_msl_m - h.altitude_m).abs() > 30.48)
                        {
                            grade.fail("Holding altitude exceeded ±100 ft.");
                        }
                        if sample.airspeed_error_kt.abs() > grade.tolerances.airspeed_kt {
                            grade.fail("Holding airspeed exceeded ±10 kt.");
                        }
                        for reason in &out.reasons {
                            grade.fail(reason);
                        }
                        if out.laps > 0 && out.actual_entry.is_some() {
                            grade.done();
                        }
                    }
                }
                InstrumentTaskKind::UnusualAttitude => {
                    if let Some(start) = self.recovery_started_s {
                        grade.observed_s += dt;
                        if r.crashed {
                            grade.fail("Aircraft control was lost during recovery.");
                        }
                        if r.time_s - start > 2.
                            && r.roll_rad.abs() < 0.0873
                            && r.pitch_rad.abs() < 0.0873
                            && r.vertical_speed_mps.abs() < 1.5
                        {
                            grade.done();
                        }
                    }
                }
            }
            if r.crashed {
                grade.fail("Aircraft crashed.");
            }
        }
        if self.debrief.samples.len() < 14400 {
            self.debrief.samples.push(sample);
        } else {
            for g in &mut self.debrief.tasks {
                g.fail("Recording limit reached; finish or restart the lesson.");
            }
            self.finish();
        }
        if r.on_ground || r.crashed {
            self.finish();
        }
    }
}
