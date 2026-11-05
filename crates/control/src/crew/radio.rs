//! Pilot radio procedures consume heard typed messages and transmit through the normal radio.
use super::*;
use crate::comms::phraseology as p;
use flightcraft_world::{AirspaceClass, ComStation, FacilityKind};
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Stage {
    #[default]
    None,
    Atis,
    DepartureTower,
    Radar,
    Tower,
    Ground,
    Parked,
}
#[derive(Clone, Debug, Default)]
pub(super) struct CrewRadio {
    airport: Option<String>,
    station: Option<ComStation>,
    stage: Stage,
    after_atis: Option<Stage>,
    information: Option<String>,
    listening_after: u64,
    last_information_request: u64,
    request_sent: bool,
    responding_instruction: bool,
    code_received: bool,
    awaiting: Option<PendingInstruction>,
    pub entry: Option<PatternEntry>,
    pub extend_downwind: bool,
    pub heading: Option<f64>,
    pub altitude_m: Option<f64>,
    pub approach_level_m: Option<f64>,
    pub blocked: bool,
    hold_point: Option<[f64; 2]>,
    inbound_announced: bool,
    departure_controlled: bool,
    pub parking_target: Option<[f64; 3]>,
    pub parking_authorized: bool,
}
impl Crew {
    fn station(s: &Session, airport: &str, kind: FacilityKind) -> Option<ComStation> {
        // Use the ATC sector selection so the same imported NASR frequency owns the contact.
        if matches!(kind, FacilityKind::Approach | FacilityKind::Departure) {
            s.sector_station(airport, kind)
        } else {
            s.stations()
                .into_iter()
                .find(|st| st.airport == airport && st.kind == kind)
        }
    }
    pub(super) fn towered(s: &Session, airport: &str) -> bool {
        Self::station(s, airport, FacilityKind::Tower).is_some()
    }
    fn tune(s: &mut Session, st: &ComStation) -> Result<(), CommandError> {
        CommsTune {
            radio: 1,
            channel_khz: Some(st.channel_khz),
            ..Default::default()
        }
        .run(s)?;
        CommsAudioPanel {
            transmit: Some(1),
            receive_com1: Some(true),
            ..Default::default()
        }
        .run(s)?;
        Ok(())
    }
    fn begin_contact(&mut self, s: &mut Session, airport: &str, departure: bool) -> Result<(), CommandError> {
        ConfigureAtc {
            external_controller: None,
            airport: airport.into(),
            destination: None,
        }
        .run(s)?;
        self.radio.airport = Some(airport.into());
        self.radio.information = None;
        self.radio.request_sent = false;
        self.radio.departure_controlled = departure;
        let next = if departure {
            Stage::DepartureTower
        } else {
            Stage::Radar
        };
        self.radio.station = Self::station(
            s,
            airport,
            if departure {
                FacilityKind::Tower
            } else {
                FacilityKind::Approach
            },
        )
        .or_else(|| Self::station(s, airport, FacilityKind::Tower));
        if let Some(atis) = Self::station(s, airport, FacilityKind::Atis) {
            self.radio.listening_after = s.flight_record().events.last().map_or(0, |e| e.id);
            CommsTune {
                radio: 2,
                channel_khz: Some(atis.channel_khz),
                ..Default::default()
            }
            .run(s)?;
            CommsAudioPanel {
                receive_com2: Some(true),
                ..Default::default()
            }
            .run(s)?;
            self.radio.after_atis = Some(next);
            self.radio.stage = Stage::Atis;
            s.crew_say(format!(
                "I'll listen to {} before we call the controller.",
                p::facility(&atis)
            ));
        } else {
            self.radio.stage = next;
            if let Some(st) = self.radio.station.clone() {
                Self::tune(s, &st)?;
            }
        }
        Ok(())
    }
    fn apply_instruction(
        &mut self,
        s: &mut Session,
        pending: &PendingInstruction,
    ) -> Result<(), CommandError> {
        match &pending.instruction {
            AtcInstruction::Squawk { code, .. } | AtcInstruction::IfrClearance { code, .. } => {
                SetAvionics {
                    setting: AvionicsSetting::Glass {
                        setting: GlassSetting::Transponder {
                            code: code.clone(),
                            mode: TransponderMode::Alt,
                        },
                    },
                }
                .run(s)?;
                // Continue an inbound radar check-in after flight following assigned the code.
                self.radio.code_received = true;
                if self.radio.stage == Stage::Radar {
                    self.radio.request_sent = false;
                }
            }
            AtcInstruction::Vector {
                heading_deg,
                altitude_ft,
            }
            | AtcInstruction::Bravo {
                cleared: true,
                heading_deg,
                altitude_ft,
            } => {
                self.radio.heading = Some(
                    s.sim
                        .heading_reference()
                        .magnetic_to_grid(f64::from(*heading_deg).to_radians()),
                );
                self.radio.altitude_m = Some(f64::from(*altitude_ft) * 0.3048);
            }
            AtcInstruction::Altitude { altitude_ft } => {
                self.radio.altitude_m = Some(f64::from(*altitude_ft) * 0.3048)
            }
            AtcInstruction::Pattern { runway, entry, .. } => {
                let already_established = self.state.phase == PilotPhase::Downwind
                    && self.radio.entry == Some(*entry)
                    && self
                        .state
                        .plan
                        .as_ref()
                        .and_then(|p| p.arrival.as_ref())
                        .is_some_and(|a| a.runway == *runway);
                if !already_established {
                    self.assign_arrival(s, &pending.airport, runway, *entry)?;
                }
                self.radio.heading = None;
                self.radio.altitude_m = None;
                self.radio.extend_downwind = false;
            }
            AtcInstruction::Approach {
                airport,
                runway,
                intercept,
                ..
            } => {
                if !matches!(self.state.intent, Some(PilotIntent::Approach { .. })) {
                    self.assign_arrival(s, airport, runway, PatternEntry::StraightIn)?;
                }
                self.radio.heading = intercept.as_ref().map(|v| {
                    s.sim
                        .heading_reference()
                        .magnetic_to_grid(f64::from(v.heading_deg).to_radians())
                });
                self.radio.altitude_m = intercept.as_ref().map(|v| f64::from(v.altitude_ft) * 0.3048);
                self.radio.approach_level_m = self.radio.altitude_m;
            }
            AtcInstruction::Land { runway } => {
                self.radio.heading = None;
                self.radio.altitude_m = None;
                if self
                    .state
                    .plan
                    .as_ref()
                    .and_then(|p| p.arrival.as_ref())
                    .is_some_and(|a| a.runway != *runway)
                {
                    self.assign_arrival(s, &pending.airport, runway, PatternEntry::LeftDownwind)?;
                }
            }
            AtcInstruction::Takeoff { runway, .. } => {
                if let Some(plan) = &self.state.plan
                    && plan.departure.as_ref().is_some_and(|r| r.runway != *runway)
                {
                    return Err(invalid(
                        "crew.intent",
                        "ATC departure runway differs from aircraft position; remain stopped and reposition before departure",
                    ));
                }
            }
            AtcInstruction::Handoff { station_id, .. } => {
                if let Some(st) = s.stations().into_iter().find(|st| st.id == *station_id) {
                    Self::tune(s, &st)?;
                    self.radio.stage = match st.kind {
                        FacilityKind::Tower => Stage::Tower,
                        FacilityKind::Ground => Stage::Ground,
                        _ => Stage::Radar,
                    };
                    self.radio.station = Some(st);
                    self.radio.request_sent = false;
                }
            }
            AtcInstruction::ExitRunway { ground_khz, .. } => {
                if let Some(st) = s.stations().into_iter().find(|st| {
                    st.airport == pending.airport
                        && st.kind == FacilityKind::Ground
                        && st.channel_khz == *ground_khz
                }) {
                    Self::tune(s, &st)?;
                    self.radio.station = Some(st);
                    self.radio.stage = Stage::Ground;
                    self.radio.request_sent = false;
                }
            }
            AtcInstruction::MissedApproach { .. } => {
                if !s.sim.procedure_receiver().missed_approach {
                    s.execute(
                        "avionics.set",
                        serde_json::json!({"setting":{"kind":"missed_approach"}}),
                    )?;
                }
                self.change(s, PilotPhase::GoAround, "Flying the published missed approach.");
            }
            AtcInstruction::ExtendDownwind => self.radio.extend_downwind = true,
            AtcInstruction::GoAround { runway } => {
                self.assign_arrival(s, &pending.airport, runway, PatternEntry::LeftDownwind)?;
                self.change(
                    s,
                    PilotPhase::GoAround,
                    "Tower instructed a go-around. Full power, then another circuit.",
                );
            }
            AtcInstruction::Parking => {
                self.radio.parking_authorized = true;
                self.radio.stage = Stage::Parked;
                if let Some(target) = self.parking_point(s) {
                    self.radio.parking_target = Some(target);
                    self.change(
                        s,
                        PilotPhase::Parking,
                        "Ground cleared us to parking. Taxiing at walking pace.",
                    );
                } else {
                    self.state.reason =
                        "Ground cleared parking, but no safe path was found. Holding position.".into();
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn assign_arrival(
        &mut self,
        s: &mut Session,
        airport: &str,
        runway: &str,
        entry: PatternEntry,
    ) -> Result<(), CommandError> {
        let arrival = planning::runway(s, airport, runway)?;
        let changed = self
            .state
            .plan
            .as_ref()
            .and_then(|p| p.arrival.as_ref())
            .is_some_and(|old| old.runway != runway);
        let side = if matches!(entry, PatternEntry::RightDownwind | PatternEntry::RightBase) {
            -1.
        } else {
            1.
        };
        self.pattern_side = side;
        let height = arrival.pattern_altitude_m + arrival.threshold_m[2];
        let mut legs = planning::arrival_legs(s, &arrival, entry == PatternEntry::StraightIn);
        if matches!(entry, PatternEntry::LeftBase | PatternEntry::RightBase) {
            legs = vec![planning::leg(
                s,
                "Join base",
                arrival.point(-2700., -1500. * side, height * 0.6),
            )];
        } else if side < 0. {
            for leg in &mut legs {
                let (a, c) = arrival.relative(leg.position_m);
                leg.position_m = arrival.point(a, -c, height);
                leg.position = planning::geographic(s, leg.position_m);
                leg.name = leg.name.replace("left", "right");
            }
        }
        let (along, cross) = arrival.relative(s.sim.state().position_m);
        let distant = libm::hypot(along, cross) > 12. * 1852.;
        if let Some(plan) = &mut self.state.plan {
            if distant {
                let mut prefix: Vec<_> = plan
                    .legs
                    .iter()
                    .skip(self.state.leg)
                    .filter(|l| !l.name.contains("downwind") && !l.name.contains("intercept"))
                    .cloned()
                    .collect();
                prefix.extend(legs);
                legs = prefix;
            }
            plan.arrival = Some(arrival);
            plan.legs = legs;
        }
        self.state.leg = 0;
        self.radio.entry = Some(entry);
        self.radio_queue.clear();
        if !s.sim.readout().on_ground && !distant {
            self.state.phase = PilotPhase::Entry;
            self.state.next_step = "Following the assigned pattern entry".into();
        }
        if changed {
            s.crew_say(format!(
                "ATC is using runway {}. I'll amend our arrival and fly the assigned entry.",
                p::runway(runway)
            ));
        }
        Ok(())
    }
    pub(super) fn departure_authorized(&self, s: &Session) -> bool {
        !self.radio.departure_controlled || self.state.plan.as_ref().and_then(|p|p.departure.as_ref()).is_some_and(|r|
            s.comms.atc.takeoff && s.comms.atc.active.iter().any(|p|p.readback && p.airport==r.airport && matches!(&p.instruction,AtcInstruction::Takeoff{runway,..} if *runway==r.runway)))
    }
    pub(super) fn landing_authorized(&self, s: &Session) -> bool {
        self.state
            .plan
            .as_ref()
            .and_then(|p| p.arrival.as_ref())
            .is_none_or(|r| {
                !Self::towered(s, &r.airport)
                    || s.comms.atc.landing
                        && s.comms.atc.active.iter().any(|p| {
                            p.readback
                                && p.airport == r.airport
                                && matches!(&p.instruction,AtcInstruction::Land{runway} if *runway==r.runway)
                        })
            })
    }
    pub(super) fn queue_position(&mut self, s: &mut Session, phase: PilotPhase) {
        if self.radio.responding_instruction && phase == PilotPhase::GoAround {
            return;
        }
        let runway = self.state.plan.as_ref().and_then(|p| {
            if phase == PilotPhase::Takeoff {
                p.departure.clone()
            } else {
                p.arrival.clone()
            }
        });
        let Some(r) = runway else {
            return;
        };
        // NASR can publish an after-hours CTAF on the tower's carrier. While the ATC
        // engine operates that tower, use tower procedures rather than self-announcing.
        let ctaf = if Self::towered(s, &r.airport) {
            None
        } else {
            Self::station(s, &r.airport, FacilityKind::Ctaf)
        };
        let st = ctaf.clone().or_else(|| {
            self.radio
                .station
                .clone()
                .filter(|s| s.kind == FacilityKind::Tower)
        });
        let Some(st) = st else {
            return;
        };
        if ctaf.is_none() {
            let cleared = self.landing_authorized(s);
            if phase == PilotPhase::Base
                || phase == PilotPhase::Final
                    && (cleared || self.radio.entry != Some(PatternEntry::StraightIn))
                || phase == PilotPhase::Downwind && cleared
            {
                return;
            }
        }
        if phase == PilotPhase::GoAround
            && matches!(self.state.intent, Some(PilotIntent::Approach { .. }))
            && s.sim.procedure_receiver().missed_approach
        {
            self.radio_queue.push_back(p::pilot_request(
                &st,
                &s.pilot_callsign(),
                "missed approach",
                None,
            ));
            return;
        }
        let right = self.pattern_side < 0.;
        let report = match phase {
            PilotPhase::Takeoff if ctaf.is_some() => Some("departing"),
            PilotPhase::Downwind => Some(if right { "right downwind" } else { "left downwind" }),
            PilotPhase::Base => Some(if right { "right base" } else { "left base" }),
            PilotPhase::Final => Some("final"),
            PilotPhase::GoAround => Some("going around"),
            PilotPhase::Complete if self.state.taxi_clear => Some("clear of"),
            _ => None,
        };
        if let Some(report) = report {
            if ctaf.is_some() {
                let _ = Self::tune(s, &st);
            }
            self.radio_queue
                .push_back(p::pilot_position(&st, &s.pilot_callsign(), &r.runway, report));
        }
    }
    pub(super) fn work_radio(&mut self, s: &mut Session, on_ground: bool) {
        if let Err(error) = self.radio_step(s, on_ground) {
            self.state.reason = error.to_string();
        }
        self.airspace_gate(s, on_ground);
    }
    fn radio_step(&mut self, s: &mut Session, on_ground: bool) -> Result<(), CommandError> {
        let plan = self.state.plan.clone();
        if self.radio.airport.is_none() {
            if on_ground && self.state.phase == PilotPhase::BeforeTakeoff {
                if let Some(dep) = plan
                    .as_ref()
                    .and_then(|p| p.departure.as_ref())
                    .filter(|r| Self::towered(s, &r.airport))
                {
                    self.begin_contact(s, &dep.airport, true)?;
                }
            } else if !on_ground
                && let Some(arr) = plan
                    .as_ref()
                    .and_then(|p| p.arrival.as_ref())
                    .filter(|r| Self::towered(s, &r.airport))
            {
                let (a, c) = arr.relative(s.sim.state().position_m);
                if libm::hypot(a, c) < 30. * 1852. {
                    self.begin_contact(s, &arr.airport, false)?;
                }
            }
        }
        if let Some(waiting) = self.radio.awaiting.clone() {
            if s.comms.atc.pending.iter().any(|p| p.id == waiting.id) {
                return Ok(());
            }
            if s.comms
                .atc
                .active
                .iter()
                .any(|p| p.id == waiting.id && p.readback)
            {
                self.radio.responding_instruction = true;
                let result = self.apply_instruction(s, &waiting);
                self.radio.responding_instruction = false;
                result?;
            }
            self.radio.awaiting = None;
        }
        if let Some(pending) = s.atc_state().pending_instructions.into_iter().find(|p| p.heard) {
            if let Some(st) = s.stations().into_iter().find(|s| s.id == pending.station_id) {
                Self::tune(s, &st)?;
                let text = s.atc_readback(&pending.instruction);
                if s.crew_transmit(text).is_ok() {
                    self.radio.awaiting = Some(pending);
                }
            }
            return Ok(());
        }
        if !s.comms.atc.pending.is_empty() || !s.comms.atc.replies.is_empty() {
            return Ok(());
        }
        // A new ATIS letter (time or weather/runway update) must be heard before retrying.
        if self.radio.stage != Stage::Atis {
            let verify = s.flight_record().events.iter().rev().find(|e| {
                e.id > self.radio.last_information_request
                    && e.reception == Reception::Heard
                    && matches!(e.speaker, Speaker::Facility { .. })
                    && e.text.contains("Verify you have information")
            });
            if let Some(event) = verify {
                self.radio.last_information_request = event.id;
                if let Some(atis) = self
                    .radio
                    .airport
                    .as_ref()
                    .and_then(|a| Self::station(s, a, FacilityKind::Atis))
                {
                    self.radio.listening_after = s.flight_record().events.last().map_or(0, |e| e.id);
                    self.radio.after_atis = Some(self.radio.stage);
                    self.radio.stage = Stage::Atis;
                    self.radio.request_sent = false;
                    CommsTune {
                        radio: 2,
                        channel_khz: Some(atis.channel_khz),
                        ..Default::default()
                    }
                    .run(s)?;
                    CommsAudioPanel {
                        receive_com2: Some(true),
                        ..Default::default()
                    }
                    .run(s)?;
                    s.crew_say(
                        "The information has changed. I'll listen to the current ATIS before we continue."
                            .into(),
                    );
                }
            }
        }
        if self.radio.stage == Stage::Atis {
            let report = s
                .flight_record()
                .events
                .iter()
                .filter(|e| e.id > self.radio.listening_after && e.reception == Reception::Heard)
                .find_map(|e| {
                    if let Meaning::Atis { report } = &e.meaning {
                        (Some(&report.airport) == self.radio.airport.as_ref()).then_some(report.clone())
                    } else {
                        None
                    }
                });
            if let Some(report) = report {
                self.radio.information = Some(report.information);
                self.radio.stage = self.radio.after_atis.take().unwrap_or(Stage::Radar);
                CommsAudioPanel {
                    receive_com2: Some(false),
                    ..Default::default()
                }
                .run(s)?;
                if let Some(st) = self.radio.station.clone() {
                    Self::tune(s, &st)?;
                }
                // A local practice circuit requests its departure runway explicitly; other arrivals follow ATIS.
                let local = plan.as_ref().is_some_and(|p| {
                    p.departure
                        .as_ref()
                        .zip(p.arrival.as_ref())
                        .is_some_and(|(d, a)| d.airport == a.airport)
                });
                if !local && !on_ground && !matches!(self.state.intent, Some(PilotIntent::Approach { .. })) {
                    let airport = report.airport;
                    self.assign_arrival(s, &airport, &report.runway, PatternEntry::LeftDownwind)?;
                }
            } else {
                return Ok(());
            }
        }
        if !self.radio.request_sent
            && let Some(st) = self.radio.station.clone()
        {
            let text = match self.radio.stage {
                Stage::DepartureTower => plan.as_ref().and_then(|p| p.departure.as_ref()).map(|r| {
                    p::pilot_request(
                        &st,
                        &s.pilot_callsign(),
                        &format!("ready for departure, request runway {}", p::runway(&r.runway)),
                        self.radio.information.as_deref(),
                    )
                }),
                Stage::Radar | Stage::Tower => {
                    let arr = plan.as_ref().and_then(|p| p.arrival.as_ref());
                    if self.radio.stage == Stage::Radar
                        && s.atc_state().ifr_clearance
                        && matches!(self.state.intent, Some(PilotIntent::Approach { .. }))
                    {
                        Some(p::pilot_ifr_checkin(
                            &st,
                            &s.pilot_callsign(),
                            (s.sim.readout().altitude_msl_m / 0.3048 / 100.).round().max(0.) as u32 * 100,
                            self.radio.information.as_deref(),
                        ))
                    } else if self.radio.stage == Stage::Radar && !self.radio.code_received {
                        Some(p::pilot_request(
                            &st,
                            &s.pilot_callsign(),
                            "request flight following",
                            self.radio.information.as_deref(),
                        ))
                    } else if self.radio.stage == Stage::Radar
                        && matches!(self.state.intent, Some(PilotIntent::Approach { .. }))
                    {
                        arr.map(|arr| {
                            p::pilot_request(
                                &st,
                                &s.pilot_callsign(),
                                &format!(
                                    "request {} approach runway {}",
                                    match &self.state.intent {
                                        Some(PilotIntent::Approach {
                                            mode: InstrumentApproach::Ils,
                                            ..
                                        }) => "ILS",
                                        Some(PilotIntent::Approach {
                                            mode: InstrumentApproach::Localizer,
                                            ..
                                        }) => "localizer",
                                        _ => "RNAV",
                                    },
                                    p::runway(&arr.runway)
                                ),
                                self.radio.information.as_deref(),
                            )
                        })
                    } else {
                        arr.map(|arr| {
                            let (miles, direction) = position_from(s, arr);
                            let mut text = p::pilot_inbound(
                                &st,
                                &s.pilot_callsign(),
                                &arr.runway,
                                miles,
                                &direction,
                                self.radio.information.as_deref(),
                            );
                            let local = plan.as_ref().is_some_and(|p| {
                                p.departure.as_ref().is_some_and(|d| d.airport == arr.airport)
                            });
                            if local {
                                text = text.trim_end_matches('.').to_owned()
                                    + &format!(", request runway {}.", p::runway(&arr.runway));
                            }
                            text
                        })
                    }
                }
                Stage::Ground if self.state.taxi_clear => {
                    plan.as_ref().and_then(|p| p.arrival.as_ref()).map(|r| {
                        p::pilot_request(
                            &st,
                            &s.pilot_callsign(),
                            &format!(
                                "clear of runway {}, request taxi to parking",
                                p::runway(&r.runway)
                            ),
                            None,
                        )
                    })
                }
                _ => None,
            };
            if let Some(text) = text
                && s.crew_transmit(text).is_ok()
            {
                self.radio.request_sent = true;
            }
            return Ok(());
        }
        // Position calls are deferred behind every instruction/readback, never sent as accidental readbacks.
        if let Some(text) = self.radio_queue.front().cloned()
            && s.crew_transmit(text).is_ok()
        {
            self.radio_queue.pop_front();
        }
        self.untowered_inbound(s)?;
        Ok(())
    }
    fn airspace_gate(&mut self, s: &Session, on_ground: bool) {
        self.radio.blocked = false;
        let [n, e, d] = s.sim.state().position_m;
        let [vn, ve, vd] = s.sim.state().velocity_mps;
        if !on_ground {
            self.radio.blocked = [0., 15., 30., 45.].iter().any(|seconds| {
                s.sim
                    .world()
                    .airspace_at(n + vn * seconds, e + ve * seconds, -d - vd * seconds)
                    .is_ok_and(|q| {
                        q.containing.iter().any(|v| match v.class {
                            Some(AirspaceClass::B) => {
                                !(s.atc_state().bravo_clearance && v.name.to_uppercase().contains("NEW YORK"))
                            }
                            Some(AirspaceClass::C | AirspaceClass::D) => !s.provider_contact_for(&v.name),
                            _ => false,
                        })
                    })
            });
        }
        if self.radio.blocked {
            let speed = libm::hypot(vn, ve).max(1.);
            self.radio
                .hold_point
                .get_or_insert([n - vn / speed * 600., e - ve / speed * 600.]);
            self.state.reason =
                "Holding outside controlled airspace until the required contact or clearance is received."
                    .into();
        } else {
            self.radio.hold_point = None;
        }
    }
    fn untowered_inbound(&mut self, s: &mut Session) -> Result<(), CommandError> {
        if self.radio.inbound_announced
            || !matches!(
                self.state.phase,
                PilotPhase::Cruise | PilotPhase::Descent | PilotPhase::Entry
            )
        {
            return Ok(());
        }
        let Some(plan) = &self.state.plan else {
            return Ok(());
        };
        if self.state.leg < plan.legs.len().saturating_sub(3) {
            return Ok(());
        }
        let Some(arr) = &plan.arrival else {
            return Ok(());
        };
        if Self::towered(s, &arr.airport) {
            return Ok(());
        }
        let Some(st) = Self::station(s, &arr.airport, FacilityKind::Ctaf) else {
            return Ok(());
        };
        let (a, c) = arr.relative(s.sim.state().position_m);
        if libm::hypot(a, c) <= 10.5 * 1852. {
            Self::tune(s, &st)?;
            let (miles, direction) = position_from(s, arr);
            let text = p::pilot_inbound(&st, &s.pilot_callsign(), &arr.runway, miles, &direction, None);
            if s.crew_transmit(text).is_ok() {
                self.radio.inbound_announced = true;
            }
        }
        Ok(())
    }
    pub(super) fn radio_guidance(&self, s: &Session) -> (Option<f64>, Option<f64>) {
        if let Some([hn, he]) = self.radio.hold_point {
            let [n, e, _] = s.sim.state().position_m;
            let radial = libm::atan2(e - he, n - hn);
            return (
                Some(
                    radial
                        + core::f64::consts::FRAC_PI_2
                        + ((libm::hypot(n - hn, e - he) - 600.) / 600.).clamp(-0.7, 0.7),
                ),
                self.radio.altitude_m,
            );
        }
        (self.radio.heading, self.radio.altitude_m)
    }
    fn parking_point(&self, s: &Session) -> Option<[f64; 3]> {
        let runway = self.state.plan.as_ref()?.arrival.as_ref()?;
        let (a, c) = runway.relative(s.sim.state().position_m);
        for forward in [10., 20., 30.] {
            let target = runway.point(a + forward, c.signum() * (c.abs() + 2.), 0.);
            if self.ground_path_safe(s, s.sim.state().position_m, target) {
                return Some(target);
            }
        }
        None
    }
    pub(super) fn ground_path_safe(&self, s: &Session, from: [f64; 3], to: [f64; 3]) -> bool {
        let [n, e, _] = from;
        let [tn, te, _] = to;
        let length = libm::hypot(tn - n, te - e);
        let steps = (libm::ceil(length * 2.) as u32).max(1);
        let mut previous = None;
        for i in 0..=steps {
            let f = f64::from(i) / f64::from(steps);
            let g = s.sim.world().ground(n + (tn - n) * f, e + (te - e) * f);
            if g.surface == flightcraft_world::Surface::Water
                || g.normal_ned[2] > -0.99
                || previous.is_some_and(|h: f64| (g.height_msl_m - h).abs() > 0.05)
            {
                return false;
            }
            previous = Some(g.height_msl_m);
        }
        true
    }
}
fn position_from(s: &Session, r: &CrewRunway) -> (u32, String) {
    let [n, e, _] = s.sim.state().position_m;
    let [rn, re, _] = r.threshold_m;
    let angle = libm::atan2(e - re, n - rn).to_degrees().rem_euclid(360.);
    let index = ((angle / 45.).round() as usize) % 8;
    let direction = [
        "north",
        "northeast",
        "east",
        "southeast",
        "south",
        "southwest",
        "west",
        "northwest",
    ]
    .get(index)
    .copied()
    .unwrap_or("north");
    (
        (libm::hypot(n - rn, e - re) / 1852.).round().max(1.) as u32,
        direction.into(),
    )
}

#[cfg(test)]
mod instruction_tests {
    use super::*;
    use crate as flightcraft_control;
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/atc_fixture.rs"));
    #[test]
    fn assigned_heading_altitude_and_squawk_drive_the_real_aircraft() {
        let mut s = Session::new().unwrap();
        s.execute(
            "sim.reset",
            serde_json::json!({"start":{"kind":"air","height_m":700.,"airspeed_mps":40.,"heading_rad":0.}}),
        )
        .unwrap();
        s.execute("crew.mode", serde_json::json!({"mode":"watching"}))
            .unwrap();
        s.execute(
            "crew.intent",
            serde_json::json!({"intent":{"kind":"hold","altitude_m":700.,"heading_rad":0.}}),
        )
        .unwrap();
        s.execute("crew.handover", serde_json::json!({"to":"crew","actor":"crew"}))
            .unwrap();
        s.execute(
            "crew.handover",
            serde_json::json!({"to":"crew","actor":"pilot","confirm":true}),
        )
        .unwrap();
        let make = |instruction| PendingInstruction {
            id: 1,
            station_id: "test".into(),
            airport: "default".into(),
            instruction,
            phrase: String::new(),
            heard: true,
            readback: true,
            issued_s: 0.,
            reminder_s: 10.,
        };
        let before = *s.sim.state();
        let mut crew = std::mem::take(&mut s.crew);
        crew.apply_instruction(
            &mut s,
            &make(AtcInstruction::Vector {
                heading_deg: 90,
                altitude_ft: 3000,
            }),
        )
        .unwrap();
        crew.apply_instruction(
            &mut s,
            &make(AtcInstruction::Squawk {
                code: "4321".into(),
                ident: false,
            }),
        )
        .unwrap();
        s.crew = crew;
        assert_eq!(*s.sim.state(), before);
        assert_eq!(s.glass.transponder_code, "4321");
        s.run(Step { steps: 18000 }).unwrap();
        let r = s.sim.readout();
        assert!(!r.crashed);
        assert!((r.altitude_msl_m - 3000. * 0.3048).abs() < 10., "{r:?}");
        assert!(
            flightcraft_sim::autopilot::wrap(r.heading_rad - core::f64::consts::FRAC_PI_2).abs() < 0.05,
            "{r:?}"
        );
    }
    #[test]
    fn towered_landing_requires_accepted_clearance_for_the_actual_runway() {
        let mut s = atc_session("KABE");
        let mut crew = Crew::default();
        crew.state.plan = Some(CrewPlan {
            arrival: Some(planning::runway(&s, "KABE", "06").unwrap()),
            briefing: String::new(),
            legs: vec![],
            departure: None,
            altitude_m: 0.,
            checks: vec![],
        });
        assert!(!crew.landing_authorized(&s));
        s.comms.atc.landing = true;
        assert!(!crew.landing_authorized(&s));
        s.comms.atc.active.push(PendingInstruction {
            id: 1,
            station_id: "test".into(),
            airport: "KABE".into(),
            instruction: AtcInstruction::Land { runway: "24".into() },
            phrase: String::new(),
            heard: true,
            readback: true,
            issued_s: 0.,
            reminder_s: 10.,
        });
        assert!(!crew.landing_authorized(&s));
        s.comms.atc.active.last_mut().unwrap().instruction = AtcInstruction::Land { runway: "06".into() };
        s.comms.atc.active.last_mut().unwrap().readback = false;
        assert!(!crew.landing_authorized(&s));
        s.comms.atc.active.last_mut().unwrap().readback = true;
        assert!(crew.landing_authorized(&s));
        s.comms.atc.active.last_mut().unwrap().airport = "KMMU".into();
        assert!(!crew.landing_authorized(&s));
        s.comms.atc.active.last_mut().unwrap().airport = "KABE".into();
        crew.assign_arrival(&mut s, "KABE", "24", PatternEntry::RightDownwind)
            .unwrap();
        assert!(!crew.landing_authorized(&s));
        assert_eq!(crew.pattern_side, -1.);
        let r = crew.state.plan.as_ref().unwrap().arrival.as_ref().unwrap();
        assert!(
            crew.state
                .plan
                .as_ref()
                .unwrap()
                .legs
                .iter()
                .all(|l| r.relative(l.position_m).1 > 0.)
        );
    }
    #[test]
    fn published_pattern_altitude_overrides_rounded_field_elevation_fallback() {
        let s = atc_session("KABE");
        let fallback = planning::runway(&s, "KABE", "06").unwrap();
        let field = crate::regions::airport(s.sim.world(), "KABE")
            .unwrap()
            .elevation_m;
        assert_eq!(
            (fallback.pattern_altitude_m / 0.3048).round(),
            ((field / 0.3048 + 1000.) / 100.).round() * 100.
        );
        let world = s.sim.world().clone();
        let mut airports = world.airports().unwrap().clone();
        let a = airports
            .airports
            .iter_mut()
            .find(|a| a.icao.as_deref() == Some("KABE") || a.ident == "KABE")
            .unwrap();
        a.traffic_pattern_altitude_ft = Some(1650);
        let mut s = Session::new().unwrap();
        s.install_region(world.with_airports(airports).unwrap()).unwrap();
        s.execute(
            "sim.reset",
            serde_json::json!({"world":"atc-fixture","start":{"runway":{"airport":"KABE","end":"06"}}}),
        )
        .unwrap();
        assert_eq!(
            (planning::runway(&s, "KABE", "06").unwrap().pattern_altitude_m / 0.3048).round(),
            1650.
        );
    }
    #[test]
    fn crew_holds_before_controlled_airspace_without_provider_contact() {
        use flightcraft_world::{
            AirspaceAltitude, AirspaceFile, AirspaceKind, AirspaceVolume, AltitudeReference,
            geo::LocalProjection,
        };
        let world = atc_world();
        let origin = &world.region().unwrap().origin;
        let p = LocalProjection::new(origin.latitude_deg, origin.longitude_deg).unwrap();
        let ring = [
            [-4000., 1500.],
            [4000., 1500.],
            [4000., 8000.],
            [-4000., 8000.],
            [-4000., 1500.],
        ]
        .into_iter()
        .map(|[n, e]| p.unproject(n, e).unwrap())
        .collect();
        let world = world
            .with_airspace(AirspaceFile {
                attribution: "Original fixture".into(),
                volumes: vec![AirspaceVolume {
                    id: "test-d".into(),
                    name: "TEST CLASS D".into(),
                    class: Some(AirspaceClass::D),
                    kind: AirspaceKind::Class,
                    sector: None,
                    floor: AirspaceAltitude {
                        value_ft: 0.,
                        reference: AltitudeReference::SFC,
                    },
                    ceiling: AirspaceAltitude {
                        value_ft: 3000.,
                        reference: AltitudeReference::MSL,
                    },
                    polygons: vec![vec![ring]],
                }],
            })
            .unwrap();
        let mut s = Session::new().unwrap();
        s.install_region(world).unwrap();
        s.execute("sim.reset",serde_json::json!({"world":"atc-fixture","start":{"kind":"air","height_m":700.,"airspeed_mps":40.,"heading_rad":core::f64::consts::FRAC_PI_2}})).unwrap();
        s.execute("crew.mode", serde_json::json!({"mode":"watching"}))
            .unwrap();
        s.execute(
            "crew.intent",
            serde_json::json!({"intent":{"kind":"hold","altitude_m":700.,"heading_rad":core::f64::consts::FRAC_PI_2}}),
        )
        .unwrap();
        s.execute("crew.handover", serde_json::json!({"to":"crew","actor":"crew"}))
            .unwrap();
        s.execute(
            "crew.handover",
            serde_json::json!({"to":"crew","actor":"pilot","confirm":true}),
        )
        .unwrap();
        for _ in 0..90 {
            s.run(Step { steps: 120 }).unwrap();
            let [n, e, d] = s.sim.state().position_m;
            assert!(!s.sim.readout().crashed);
            assert!(
                s.sim.world().airspace_at(n, e, -d).unwrap().containing.is_empty(),
                "entered controlled airspace at {n},{e}"
            );
        }
        assert!(
            s.crew
                .state
                .reason
                .contains("Holding outside controlled airspace")
        );
    }
    #[test]
    fn published_after_hours_ctaf_does_not_replace_an_operating_tower() {
        let mut s = atc_session("KMMU");
        assert!(Crew::station(&s, "KMMU", FacilityKind::Ctaf).is_some());
        let r = planning::runway(&s, "KMMU", "05").unwrap();
        let mut crew = Crew::default();
        crew.state.plan = Some(CrewPlan {
            briefing: String::new(),
            legs: vec![],
            arrival: Some(r.clone()),
            departure: Some(r),
            altitude_m: 400.,
            checks: vec![],
        });
        crew.radio.station = Crew::station(&s, "KMMU", FacilityKind::Tower);
        crew.queue_position(&mut s, PilotPhase::Takeoff);
        assert!(crew.radio_queue.is_empty());
        crew.queue_position(&mut s, PilotPhase::Downwind);
        assert!(crew.radio_queue.front().unwrap().starts_with("Morristown Tower,"));
        assert!(!crew.radio_queue.front().unwrap().contains("Traffic"));
    }
}
