use super::*;
use parser::PilotRequest as R;
impl Session {
    pub(super) fn handle_pilot(&mut self, event: &Transmission, st: ComStation) {
        let m = parse_pilot(&event.text, &self.pilot_callsign());
        self.comms.atc.current = Some(st.clone());
        if self.comms.atc.airport.is_none() {
            self.comms.atc.airport = Some(st.airport.clone());
        }
        if !m.callsign_present {
            if self.comms.atc.replies.len() < 64 {
                self.comms.atc.replies.push_back(Reply {
                    station: st.clone(),
                    text: "Aircraft calling, say again callsign.".into(),
                    meaning: Meaning::Text,
                });
            }
            return;
        }
        if (m.normalized.contains("say again") || m.normalized.contains("repeat instruction"))
            && let Some(p) = self.comms.atc.pending.iter_mut().find(|p| p.station_id == st.id)
        {
            p.id = 0;
            p.heard = false;
            p.reminder_s = f64::MAX;
            let phrase = p.phrase.clone();
            let instruction = p.instruction.clone();
            self.atc_reply(&st, phrase, Meaning::Atc { instruction });
            return;
        }
        if let Some(index) = self
            .comms
            .atc
            .pending
            .iter()
            .position(|p| p.heard && p.station_id == st.id)
        {
            let pending = self.comms.atc.pending.get(index).cloned();
            if let Some(mut p) = pending {
                if readback_matches(&p.instruction, &m) {
                    self.comms.atc.pending.remove(index);
                    p.readback = true;
                    p.issued_s = self.sim.state().time_s;
                    self.comms
                        .atc
                        .cue_queue
                        .retain(|g| g.instruction_id != Some(p.id));
                    self.atc_grade(Some(p.id), "readback", true, "Correct readback.".into());
                    self.accept_instruction(&p);
                    if self.comms.atc.active.len() >= 32 {
                        self.comms.atc.active.remove(0);
                    }
                    self.comms.atc.active.push(p);
                } else {
                    self.atc_grade(
                        Some(p.id),
                        "readback",
                        false,
                        "Incorrect or incomplete readback.".into(),
                    );
                    self.atc_reply(
                        &st,
                        format!("Negative. {}", p.phrase),
                        Meaning::Atc {
                            instruction: p.instruction,
                        },
                    );
                    if let Some(p) = self.comms.atc.pending.get_mut(index) {
                        p.reminder_s = self.sim.state().time_s + 20.;
                    }
                }
            }
            return;
        }
        if !self.comms.atc.pending.is_empty() {
            self.atc_reply(
                &st,
                "Read back the outstanding instruction on the issuing frequency first.".into(),
                Meaning::Text,
            );
            return;
        }
        if let Some(address) = &m.addressed_facility {
            let call = st
                .metadata
                .as_ref()
                .map_or(st.name.as_str(), |v| v.callsign.as_str());
            let valid = address.contains(&words(&st.name))
                || (!call.is_empty() && address.contains(&words(call)))
                || (new_york_controller(&st) && (address.contains("new york") || address.contains("newark")));
            if !valid {
                self.atc_reply(
                    &st,
                    format!("This is {}. Say again request.", phraseology::facility(&st)),
                    Meaning::Text,
                );
                return;
            }
        }
        if let Some(info) = &m.information {
            let airport = self
                .comms
                .atc
                .airport
                .clone()
                .unwrap_or_else(|| st.airport.clone());
            if self.atis_report(&airport).is_some_and(|r| r.information == *info) {
                self.comms.atc.reported = Some((airport, info.clone()));
            }
        }
        if self.comms.atc.external_controller {
            return;
        }
        let request = m.request;
        let airport = self
            .comms
            .atc
            .airport
            .clone()
            .unwrap_or_else(|| st.airport.clone());
        let needs_atis = matches!(
            request,
            Some(R::Taxi | R::VfrDeparture | R::IfrClearance | R::Ready | R::Inbound | R::Approach)
        );
        if needs_atis
            && let Some(atis) = self.atis_report(&airport)
            && self.comms.atc.reported.as_ref() != Some(&(airport.clone(), atis.information.clone()))
        {
            self.atc_reply(
                &st,
                format!(
                    "Verify you have information {}. Report with information {}.",
                    atis.information, atis.information
                ),
                Meaning::Text,
            );
            return;
        }
        let requested_runway = m
            .runway
            .as_ref()
            .filter(|_| m.normalized.contains("request runway"))
            .filter(|r| {
                crate::regions::airport(self.sim.world(), &airport)
                    .is_ok_and(|a| a.runways.iter().flat_map(|r| &r.ends).any(|e| e.ident == **r))
            });
        let Some(runway) = requested_runway.cloned().or_else(|| self.active_runway(&airport)) else {
            self.atc_reply(&st, "Unable, no airport runway data.".into(), Meaning::Text);
            return;
        };
        // A position report continues the assigned runway. A wind-preferred runway does not
        // silently amend an already accepted pattern or approach clearance.
        let runway = if st.kind == FacilityKind::Tower
            && matches!(
                request,
                Some(R::Position | R::Inbound | R::GoAround | R::ClearRunway)
            ) {
            self.comms
                .atc
                .active
                .iter()
                .rev()
                .find_map(|p| match &p.instruction {
                    AtcInstruction::Pattern { runway, .. }
                    | AtcInstruction::Approach { runway, .. }
                    | AtcInstruction::Land { runway }
                    | AtcInstruction::GoAround { runway }
                    | AtcInstruction::Takeoff { runway, .. } => Some(runway.clone()),
                    _ => None,
                })
                .unwrap_or(runway)
        } else {
            runway
        };
        match (st.kind, request) {
            (FacilityKind::Clearance, Some(R::IfrClearance)) => {
                let Some(destination) = self.comms.atc.destination.clone() else {
                    self.atc_reply(&st, "Say destination airport.".into(), Meaning::Text);
                    return;
                };
                let Some(dep) = self.next_station(&airport, FacilityKind::Departure) else {
                    self.atc_reply(
                        &st,
                        "Unable, departure frequency unavailable.".into(),
                        Meaning::Text,
                    );
                    return;
                };
                let code = self.assign_squawk();
                let altitude_ft = self.training_altitude(&airport);
                let route = "as filed".to_string();
                self.issue(
                    &st,
                    AtcInstruction::IfrClearance {
                        destination_name: self.airport_radio_name(&destination),
                        expected_altitude_ft: 5000,
                        expect_minutes: 10,
                        destination,
                        route,
                        altitude_ft,
                        departure_khz: dep.channel_khz,
                        code,
                    },
                );
            }
            (FacilityKind::Clearance, Some(R::VfrDeparture)) => {
                self.comms.atc.ifr_clearance = false;
                let code = self.assign_squawk();
                self.issue(&st, AtcInstruction::Squawk { code: code.clone() });
            }
            (FacilityKind::Ground, Some(R::Taxi | R::VfrDeparture)) => {
                self.issue(
                    &st,
                    AtcInstruction::Taxi {
                        runway: runway.clone(),
                        route: Vec::new(),
                        hold_short: true,
                    },
                );
            }
            (FacilityKind::Tower, Some(R::Ready | R::ClosedTraffic)) => {
                if self.runway_traffic(&airport) {
                    self.issue(
                        &st,
                        AtcInstruction::Taxi {
                            runway: runway.clone(),
                            route: Vec::new(),
                            hold_short: true,
                        },
                    );
                } else {
                    let closed = request == Some(R::ClosedTraffic) || self.comms.atc.closed_traffic;
                    self.issue(
                        &st,
                        AtcInstruction::Takeoff {
                            runway: runway.clone(),
                            closed_traffic: closed,
                        },
                    );
                }
            }
            (FacilityKind::Tower, Some(R::Inbound))
                if self
                    .comms
                    .atc
                    .active
                    .iter()
                    .any(|p| matches!(&p.instruction, AtcInstruction::Approach { .. }))
                    && !self.runway_traffic(&airport) =>
            {
                self.issue(
                    &st,
                    AtcInstruction::Land {
                        runway: runway.clone(),
                    },
                )
            }
            (FacilityKind::Tower, Some(R::Inbound)) => self.pattern_entry(&st, &m, &airport, &runway),
            (FacilityKind::Tower, Some(R::Position)) => {
                if self.comms.atc.landing && self.comms.atc.active.iter().any(|p| p.readback && p.airport == airport && matches!(&p.instruction, AtcInstruction::Land {runway: cleared} if *cleared == runway)) {
                    return;
                }
                if m.normalized.contains("downwind") && self.runway_traffic(&airport) {
                    self.issue(&st, AtcInstruction::ExtendDownwind);
                } else if m.normalized.contains("final")
                    || m.normalized.contains("base")
                    || m.normalized.contains("downwind")
                        && self
                            .comms
                            .atc
                            .active
                            .iter()
                            .any(|p| matches!(p.instruction, AtcInstruction::Pattern { .. }))
                {
                    if self.runway_traffic(&airport) {
                        self.issue(
                            &st,
                            AtcInstruction::GoAround {
                                runway: runway.clone(),
                            },
                        );
                    } else {
                        self.issue(
                            &st,
                            AtcInstruction::Land {
                                runway: runway.clone(),
                            },
                        );
                    }
                } else {
                    self.pattern_entry(&st, &m, &airport, &runway);
                }
            }
            (FacilityKind::Tower | FacilityKind::Approach, Some(R::GoAround))
                if m.normalized.contains("missed approach") =>
            {
                if let Some(loaded) = self
                    .sim
                    .procedure_receiver()
                    .loaded
                    .iter()
                    .find(|p| p.kind == flightcraft_world::procedures::ProcedureKind::Approach)
                {
                    self.issue(
                        &st,
                        AtcInstruction::MissedApproach {
                            airport: loaded.airport.clone(),
                            procedure: loaded.ident.clone(),
                        },
                    );
                }
            }
            (FacilityKind::Tower, Some(R::GoAround)) => {
                self.issue(
                    &st,
                    AtcInstruction::GoAround {
                        runway: runway.clone(),
                    },
                );
            }
            (FacilityKind::Tower, Some(R::ClearRunway)) => {
                if let Some(ground) = self.next_station(&airport, FacilityKind::Ground) {
                    self.issue(
                        &st,
                        AtcInstruction::ExitRunway {
                            runway,
                            ground_khz: ground.channel_khz,
                        },
                    );
                }
            }
            (FacilityKind::Ground, Some(R::ClearRunway)) => {
                self.issue(&st, AtcInstruction::Parking);
            }
            (FacilityKind::Approach | FacilityKind::Departure, Some(R::Bravo)) => {
                let code_ok=self.comms.atc.active.iter().any(|p|matches!(&p.instruction,AtcInstruction::Squawk {code} if *code==self.glass.transponder_code));
                let cleared =
                    code_ok && self.comms.atc.two_way.contains(&provider(&st)) && new_york_controller(&st);
                let heading_deg = self
                    .sim
                    .readout()
                    .heading_rad
                    .to_degrees()
                    .round()
                    .rem_euclid(360.) as u16;
                let altitude_ft = m.altitude_ft.unwrap_or_else(|| {
                    (-self.sim.state().position_m[2] / 0.3048 / 100.).round().max(10.) as u32 * 100
                });
                self.issue(
                    &st,
                    AtcInstruction::Bravo {
                        cleared,
                        heading_deg,
                        altitude_ft,
                    },
                );
            }
            (
                FacilityKind::Approach | FacilityKind::Departure,
                Some(R::FlightFollowing | R::VfrDeparture),
            ) => {
                let code = self.assign_squawk();
                self.issue(&st, AtcInstruction::Squawk { code: code.clone() });
            }
            (FacilityKind::Approach | FacilityKind::Departure, Some(R::Position | R::Inbound)) => {
                if self.comms.atc.radar_facility.as_deref() != Some(radar_facility(&st).as_str()) {
                    self.atc_reply(
                        &st,
                        self.radar_position_report(&airport),
                        Meaning::RadarContact {
                            station_id: st.id.clone(),
                        },
                    );
                }
                if st.kind == FacilityKind::Approach
                    && self.comms.atc.ifr_clearance
                    && self.issue_lesson_intercept(&st)
                {
                    return;
                }
                self.traffic_advisory(&st);
                if request == Some(R::Inbound) {
                    self.comms.atc.inbound = true;
                    let near = crate::regions::airport(self.sim.world(), &airport)
                        .ok()
                        .and_then(|a| self.radio_project(a.latitude_deg, a.longitude_deg))
                        .is_some_and(|[n, e]| {
                            let [pn, pe, _] = self.sim.state().position_m;
                            libm::hypot(n - pn, e - pe) <= 10. * 1852.
                        });
                    if near && let Some(tower) = self.next_station(&airport, FacilityKind::Tower) {
                        self.comms.atc.inbound = false;
                        self.handoff(&st, &tower);
                    }
                } else if let Some(next) = self.sector_station(&airport, st.kind)
                    && next.id != st.id
                    && next.channel_khz != st.channel_khz
                    && radar_facility(&next) != radar_facility(&st)
                {
                    self.handoff(&st, &next);
                } else if self.comms.atc.ifr_clearance {
                    let assigned = if st.kind == FacilityKind::Approach {
                        self.training_altitude(&airport)
                    } else {
                        5000
                    };
                    if !self.comms.atc.active.iter().any(|p|matches!(p.instruction,AtcInstruction::Altitude{altitude_ft} if altitude_ft==assigned)) {
                        self.issue(&st,AtcInstruction::Altitude{altitude_ft:assigned});
                    }
                } else {
                    let heading_deg = self.runway_heading(&airport, &runway);
                    let altitude_ft = self.training_altitude(&airport);
                    self.issue(
                        &st,
                        AtcInstruction::Vector {
                            heading_deg,
                            altitude_ft,
                        },
                    );
                }
            }
            (FacilityKind::Approach, Some(R::Approach)) => self.clear_approach(&st, &m, &airport, &runway),
            (_, Some(R::Handoff)) => {
                let target = match st.kind {
                    FacilityKind::Clearance => FacilityKind::Ground,
                    FacilityKind::Ground => FacilityKind::Tower,
                    FacilityKind::Tower => FacilityKind::Departure,
                    _ => FacilityKind::Approach,
                };
                let destination = if matches!(st.kind, FacilityKind::Approach | FacilityKind::Departure) {
                    self.comms.atc.destination.as_deref().unwrap_or(&airport)
                } else {
                    &airport
                };
                if let Some(next) = self.next_station(destination, target) {
                    self.handoff(&st, &next);
                } else {
                    self.atc_reply(
                        &st,
                        "Unable, no published frequency for requested handoff.".into(),
                        Meaning::Text,
                    );
                }
            }
            _ => self.atc_reply(&st, "Say again request.".into(), Meaning::Text),
        }
    }
    fn accept_instruction(&mut self, p: &PendingInstruction) {
        match &p.instruction {
            AtcInstruction::Takeoff { closed_traffic, .. } => {
                self.comms.atc.takeoff = true;
                self.comms.atc.closed_traffic = *closed_traffic;
            }
            AtcInstruction::Land { .. } => self.comms.atc.landing = true,
            AtcInstruction::Approach { .. } => self.comms.atc.inbound = true,
            AtcInstruction::GoAround { .. } | AtcInstruction::MissedApproach { .. } => {
                self.comms.atc.landing = false;
                self.comms.atc.takeoff = false;
            }
            AtcInstruction::Bravo { cleared: false, .. } => self.comms.atc.bravo = None,
            AtcInstruction::ExitRunway { .. } => {
                self.comms.atc.landing = false;
                self.comms.atc.ifr_clearance = false;
            }
            AtcInstruction::Handoff { .. } => self.comms.atc.takeoff = false,
            _ => {}
        }
        // A replacement vector supersedes an older vector; hold-short ends at a takeoff clearance.
        self.comms.atc.active.retain(|old| {
            if matches!(
                p.instruction,
                AtcInstruction::Approach { .. }
                    | AtcInstruction::Land { .. }
                    | AtcInstruction::Pattern { .. }
            ) && matches!(
                old.instruction,
                AtcInstruction::Vector { .. }
                    | AtcInstruction::Bravo { .. }
                    | AtcInstruction::Altitude { .. }
            ) {
                return false;
            }
            !matches!(
                (&p.instruction, &old.instruction),
                (AtcInstruction::Takeoff { .. }, AtcInstruction::Taxi { .. })
                    | (AtcInstruction::Taxi { .. }, AtcInstruction::Taxi { .. })
                    | (AtcInstruction::Vector { .. }, AtcInstruction::Vector { .. })
                    | (AtcInstruction::Squawk { .. }, AtcInstruction::Squawk { .. })
                    | (AtcInstruction::Squawk { .. }, AtcInstruction::IfrClearance { .. })
                    | (AtcInstruction::Handoff { .. }, AtcInstruction::Handoff { .. })
            )
        });
    }
    fn radar_position_report(&self, airport: &str) -> String {
        let Some([an, ae]) = crate::regions::airport(self.sim.world(), airport)
            .ok()
            .and_then(|a| self.radio_project(a.latitude_deg, a.longitude_deg))
        else {
            return "Radar contact.".into();
        };
        let [n, e, _] = self.sim.state().position_m;
        let bearing = libm::atan2(e - ae, n - an).rem_euclid(std::f64::consts::TAU);
        let index = (bearing / std::f64::consts::FRAC_PI_4 + 0.5).floor() as usize % 8;
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
        phraseology::radar_contact(
            (libm::hypot(n - an, e - ae) / 1852.).round() as u32,
            direction,
            &self.airport_radio_name(airport),
        )
    }
    fn assign_squawk(&self) -> String {
        format!("{:04o}", 0o4000 + (self.comms.next_id % 0o3000) as u32)
    }
    pub(super) fn next_station(&self, airport: &str, kind: FacilityKind) -> Option<ComStation> {
        self.sector_station(airport, kind)
    }
    fn training_altitude(&self, airport: &str) -> u32 {
        crate::regions::airport(self.sim.world(), airport).map_or(3000, |a| {
            ((a.elevation_m / 0.3048 + 2000.) / 100.).ceil() as u32 * 100
        })
    }
    fn runway_heading(&self, airport: &str, runway: &str) -> u16 {
        crate::regions::airport(self.sim.world(), airport)
            .ok()
            .and_then(|a| a.runways.iter().flat_map(|r| &r.ends).find(|e| e.ident == runway))
            .map_or(0, |e| {
                self.sim
                    .heading_reference()
                    .true_to_magnetic(e.heading_deg.to_radians())
                    .to_degrees()
                    .round()
                    .rem_euclid(360.) as u16
            })
    }
    pub(super) fn runway_traffic(&self, airport: &str) -> bool {
        self.traffic_state().airport.as_deref() == Some(airport)
            && self
                .traffic_state()
                .aircraft
                .iter()
                .any(|a| matches!(a.leg, PatternLeg::Final | PatternLeg::Departing))
    }
    fn pattern_entry(&mut self, st: &ComStation, m: &PilotMessage, airport: &str, runway: &str) {
        let entry = if m.normalized.contains("straight in") {
            PatternEntry::StraightIn
        } else if m.normalized.contains("right base") {
            PatternEntry::RightBase
        } else if m.normalized.contains("base") {
            PatternEntry::LeftBase
        } else if m.normalized.contains("right downwind") {
            PatternEntry::RightDownwind
        } else {
            PatternEntry::LeftDownwind
        };
        let traffic = self.traffic_state();
        let follow = traffic
            .aircraft
            .iter()
            .filter(|_| traffic.airport.as_deref() == Some(airport))
            .min_by_key(|a| match a.leg {
                PatternLeg::Base => 0,
                PatternLeg::Final => 1,
                _ => 2,
            });
        let number = if follow.is_some() { 2 } else { 1 };
        self.issue(
            st,
            AtcInstruction::Pattern {
                runway: runway.into(),
                entry,
                number,
                follow: follow.map(|a| a.callsign.clone()),
            },
        );
    }
    pub(super) fn handoff(&mut self, st: &ComStation, next: &ComStation) {
        self.comms.atc.last_sector = Some(next.id.clone());
        self.issue(
            st,
            AtcInstruction::Handoff {
                station_id: next.id.clone(),
                channel_khz: next.channel_khz,
            },
        );
    }
    pub(crate) fn sector_station(&self, airport: &str, kind: FacilityKind) -> Option<ComStation> {
        let [pn, pe, pd] = self.sim.state().position_m;
        self.stations()
            .into_iter()
            .filter(|st| st.airport == airport && st.kind == kind)
            .min_by_key(|st| {
                let sector = st
                    .metadata
                    .as_ref()
                    .map_or("", |m| m.sector.as_str())
                    .to_uppercase();
                let [n, e, _] = self.station_origin(st).unwrap_or([pn, pe, pd]);
                let bearing = libm::atan2(pe - e, pn - n).to_degrees().rem_euclid(360.) as u16;
                let altitude = -pd / 0.3048;
                let mut mismatch = 0u8;
                if sector.contains("BLW") && altitude > 3000. {
                    mismatch += 2;
                }
                if sector.contains("ABV") && altitude <= 3000. {
                    mismatch += 2;
                }
                if sector.contains("WEST") && pe > e {
                    mismatch += 2;
                }
                if let Some((a, b)) = sector
                    .split_once('-')
                    .and_then(|(a, b)| Some((a.parse::<u16>().ok()?, b.parse::<u16>().ok()?)))
                {
                    let inside = if a <= b {
                        (a..=b).contains(&bearing)
                    } else {
                        bearing >= a || bearing <= b
                    };
                    if !inside {
                        mismatch += 2;
                    }
                }
                (
                    mismatch,
                    u8::from(sector.is_empty()),
                    st.channel_khz,
                    st.id.clone(),
                )
            })
    }
    pub(super) fn clear_approach(&mut self, st: &ComStation, m: &PilotMessage, airport: &str, runway: &str) {
        let desired = m.runway.as_deref().unwrap_or(runway);
        if m.normalized.contains("visual") {
            let visible = crate::regions::airport(self.sim.world(), airport)
                .ok()
                .and_then(|a| self.radio_project(a.latitude_deg, a.longitude_deg))
                .is_some_and(|[n, e]| {
                    let [pn, pe, _] = self.sim.state().position_m;
                    libm::hypot(n - pn, e - pe) <= self.sim.conditions().effective_visibility_m()
                });
            if !m.normalized.contains("airport in sight")
                || !visible
                || self.sim.readout().on_ground
                || self.sim.conditions().in_cloud(self.sim.state().position_m)
            {
                self.atc_reply(
                    st,
                    "Report the airport in sight before requesting a visual approach.".into(),
                    Meaning::Text,
                );
                return;
            }
            self.issue(
                st,
                AtcInstruction::Approach {
                    airport: airport.into(),
                    intercept: None,
                    procedure: "visual".into(),
                    name: format!("visual runway {desired}"),
                    runway: desired.into(),
                },
            );
            return;
        }
        let loaded = self.sim.procedure_receiver().loaded.iter().find(|p| {
            p.airport == airport && p.kind == flightcraft_world::procedures::ProcedureKind::Approach
        });
        let procedure = loaded.and_then(|loaded| {
            self.sim.world().procedures()?.procedures.iter().find(|p| {
                p.airport == airport
                    && p.ident == loaded.ident
                    && p.runway.as_deref().map(|r| r.trim_start_matches("RW")) == Some(desired)
            })
        });
        let Some(p) = procedure else {
            self.atc_reply(
                st,
                "Unable, load the requested published approach in the GPS first.".into(),
                Meaning::Text,
            );
            return;
        };
        let kind = if p.ident.starts_with('I') {
            if m.normalized.contains("localizer") || m.normalized.split_whitespace().any(|w| w == "loc") {
                "LOC"
            } else {
                "ILS"
            }
        } else if p.ident.starts_with('V') {
            "VOR"
        } else if p.ident.starts_with('L') {
            "LOC"
        } else if p.ident.starts_with('N') {
            "NDB"
        } else if p.ident.starts_with('R') {
            "RNAV"
        } else {
            self.atc_reply(
                st,
                "Unable, approach family unsupported by training rules.".into(),
                Meaning::Text,
            );
            return;
        };
        if (m.normalized.contains("ils") && kind != "ILS")
            || (m.normalized.contains("vor") && kind != "VOR")
            || (m.normalized.contains("rnav") && kind != "RNAV")
            || ((m.normalized.contains("localizer") || m.normalized.split_whitespace().any(|w| w == "loc"))
                && kind != "LOC")
            || (m.normalized.contains("ndb") && kind != "NDB")
        {
            self.atc_reply(
                st,
                "Unable, requested approach differs from loaded procedure.".into(),
                Meaning::Text,
            );
            return;
        }
        let path = crate::lesson::engine::ApproachPath::find(self, airport, &p.ident);
        if let Ok(path) = path {
            let r = self.sim.readout();
            if let (Some(latitude_deg), Some(longitude_deg)) = (r.latitude_deg, r.longitude_deg) {
                let remaining = path.remaining(flightcraft_world::GeoPoint {
                    latitude_deg,
                    longitude_deg,
                });
                let angle = crate::lesson::wrap_deg(r.heading_true_rad.to_degrees() - path.course_deg).abs();
                if angle > 30. || r.altitude_msl_m > path.altitude(remaining) + 30.48 {
                    self.atc_reply(
                        st,
                        "Unable approach clearance, establish a final intercept at or below the glidepath."
                            .into(),
                        Meaning::Text,
                    );
                    return;
                }
            }
        }
        let name = format!("{kind} runway {desired}");
        self.issue(
            st,
            AtcInstruction::Approach {
                airport: airport.into(),
                intercept: None,
                procedure: p.ident.clone(),
                name: name.clone(),
                runway: desired.into(),
            },
        );
    }
    fn traffic_advisory(&mut self, st: &ComStation) {
        let [pn, pe, pd] = self.sim.state().position_m;
        let heading = self.sim.readout().heading_grid_rad;
        if let Some(a) = self.traffic_state().aircraft.iter().min_by(|a, b| {
            distance(a.position_m, [pn, pe, pd]).total_cmp(&distance(b.position_m, [pn, pe, pd]))
        }) {
            let [n, e, _] = a.position_m;
            let nm = libm::hypot(n - pn, e - pe) / 1852.;
            if nm <= 15. {
                let clock = ((libm::atan2(e - pe, n - pn) - heading)
                    .to_degrees()
                    .rem_euclid(360.)
                    / 30.)
                    .round() as u8;
                let clock = if clock.is_multiple_of(12) { 12 } else { clock % 12 };
                self.atc_reply(
                    st,
                    format!(
                        "Traffic, {} o'clock, {} miles, {}, {}.",
                        phraseology::digits(u32::from(clock), 1),
                        phraseology::digits(nm.round().max(1.) as u32, 1),
                        a.aircraft_type,
                        phraseology::altitude(
                            (a.altitude_msl_m / 0.3048 / 100.).round().max(0.) as u32 * 100
                        )
                    ),
                    Meaning::TrafficAdvisory {
                        callsign: a.callsign.clone(),
                        clock,
                        miles: nm,
                        altitude_ft: (a.altitude_msl_m / 0.3048).max(0.) as u32,
                    },
                );
            }
        }
    }
}
fn distance([an, ae, _]: [f64; 3], [bn, be, _]: [f64; 3]) -> f64 {
    libm::hypot(an - bn, ae - be)
}
pub(super) fn entry_phrase(e: PatternEntry) -> &'static str {
    match e {
        PatternEntry::StraightIn => "straight in",
        PatternEntry::LeftDownwind => "left downwind",
        PatternEntry::RightDownwind => "right downwind",
        PatternEntry::LeftBase => "left base",
        PatternEntry::RightBase => "right base",
    }
}
pub(super) fn readback_matches(i: &AtcInstruction, m: &PilotMessage) -> bool {
    let t = &m.normalized;
    if t.split_whitespace()
        .any(|w| matches!(w, "negative" | "unable" | "not"))
    {
        return false;
    }
    match i {
        AtcInstruction::Hold {
            fix,
            inbound_deg,
            altitude_ft,
            efc_utc,
            right_turns,
            leg_time_s,
            leg_distance_nm,
        } => {
            t.contains(&words(&phraseology::hold_legs(*leg_time_s, *leg_distance_nm)))
                && t.contains(&words(fix))
                && m.altitude_ft == Some(*altitude_ft)
                && t.contains(&words(&phraseology::heading(*inbound_deg)))
                && t.contains(if *right_turns { "right turns" } else { "left turns" })
                && t.contains(&words(&phraseology::digits(u32::from(*efc_utc), 4)))
        }
        AtcInstruction::MissedApproach { .. } => t.contains("published missed approach"),
        AtcInstruction::Parking => t.contains("taxi to parking"),
        AtcInstruction::Taxi {
            runway, hold_short, ..
        } => {
            m.runway.as_ref() == Some(runway)
                && (!hold_short || t.contains("hold short") || t.contains("holding short"))
        }
        AtcInstruction::Takeoff {
            runway,
            closed_traffic,
        } => {
            m.runway.as_ref() == Some(runway)
                && t.contains("cleared for takeoff")
                && (!closed_traffic || t.contains("left closed traffic"))
        }
        AtcInstruction::Land { runway } => m.runway.as_ref() == Some(runway) && t.contains("cleared to land"),
        AtcInstruction::Pattern {
            runway,
            entry,
            number,
            ..
        } => {
            m.runway.as_ref() == Some(runway)
                && t.contains(entry_phrase(*entry))
                && (*number == 1
                    || t.contains(&format!("number {}", phraseology::digits(u32::from(*number), 1)))
                    || t.contains(&format!("number {number}")))
        }
        AtcInstruction::ExtendDownwind => t.contains("extend downwind") || t.contains("extending downwind"),
        AtcInstruction::GoAround { runway } => {
            m.runway.as_ref() == Some(runway) && (t.contains("go around") || t.contains("going around"))
        }
        AtcInstruction::Vector {
            heading_deg,
            altitude_ft,
        } => {
            m.heading_deg.is_some_and(|h| h % 360 == *heading_deg % 360)
                && m.altitude_ft == Some(*altitude_ft)
        }
        AtcInstruction::Altitude { altitude_ft } => m.altitude_ft == Some(*altitude_ft),
        AtcInstruction::Squawk { code } => m.squawk.as_ref() == Some(code),
        AtcInstruction::Handoff { channel_khz, .. } => m.channel_khz == Some(*channel_khz),
        AtcInstruction::Bravo {
            cleared,
            heading_deg,
            altitude_ft,
        } => {
            if *cleared {
                t.contains("cleared into the class bravo")
                    && m.heading_deg.is_some_and(|h| h % 360 == *heading_deg % 360)
                    && m.altitude_ft == Some(*altitude_ft)
            } else {
                t.contains("remain outside") || t.contains("remaining outside")
            }
        }
        AtcInstruction::Approach {
            name,
            runway,
            intercept,
            ..
        } => {
            intercept.as_ref().is_none_or(|v| {
                m.heading_deg.is_some_and(|h| h % 360 == v.heading_deg % 360)
                    && m.altitude_ft == Some(v.altitude_ft)
                    && t.contains("until established")
                    && t.contains(if name.starts_with("ILS") || name.starts_with("LOC") {
                        "localizer"
                    } else {
                        "final approach course"
                    })
            }) && m.runway.as_ref() == Some(runway)
                && t.contains("cleared")
                && t.contains("approach")
                && name
                    .split_whitespace()
                    .next()
                    .is_some_and(|kind| t.contains(&kind.to_lowercase()))
        }
        AtcInstruction::IfrClearance {
            destination,
            destination_name,
            expected_altitude_ft,
            expect_minutes,
            route,
            altitude_ft,
            departure_khz,
            code,
        } => {
            (t.contains(&destination.to_lowercase()) || t.contains(&words(destination_name)))
                && m.expected_altitude_ft == Some(*expected_altitude_ft)
                && m.expect_minutes == Some(*expect_minutes)
                && t.contains(route)
                && m.altitude_ft == Some(*altitude_ft)
                && m.channel_khz == Some(*departure_khz)
                && m.squawk.as_ref() == Some(code)
        }
        AtcInstruction::ExitRunway { ground_khz, .. } => {
            t.contains("ground") && m.channel_khz == Some(*ground_khz)
        }
    }
}
