use super::*;
impl Session {
    pub(super) fn atc_grade(&mut self, id: Option<u64>, item: &str, passed: bool, observed: String) {
        let observed = if observed.ends_with(['.', '!', '?']) {
            observed
        } else {
            format!("{observed}.")
        };
        let grade = ComplianceGrade {
            time_s: self.sim.state().time_s,
            instruction_id: id,
            item: item.into(),
            passed,
            observed,
        };
        if self.comms.atc.compliance.len() >= 512 {
            self.comms.atc.compliance.remove(0);
        }
        self.comms.atc.compliance.push(grade.clone());
        if passed {
            self.comms
                .atc
                .cue_queue
                .retain(|g| g.item != item || g.instruction_id != id);
        } else if self.comms.atc.cue_queue.len() < 32 {
            self.comms
                .atc
                .cue_queue
                .retain(|g| g.item != item || g.instruction_id != id);
            self.comms.atc.cue_queue.push(grade.clone());
        }
        self.start_local(
            Channel::Intercom,
            Speaker::Instructor,
            grade.observed.clone(),
            Meaning::Compliance { grade },
            EventSource::Rules,
        );
    }
    pub(super) fn flush_atc_feedback(&mut self) {
        let now = self.sim.state().time_s;
        if now < self.comms.atc.next_feedback_s
            || self.comms.pending.iter().any(|p| p.channel == Channel::Intercom)
        {
            return;
        }
        let priority = |g: &ComplianceGrade| {
            if g.item == "hold_short" || g.item.starts_with("airspace:") || g.item == "landing_clearance" {
                0
            } else if g.item == "altitude" {
                1
            } else if g.item == "heading" {
                2
            } else if g.item.contains("readback") {
                3
            } else {
                4
            }
        };
        if let Some(i) = self
            .comms
            .atc
            .cue_queue
            .iter()
            .enumerate()
            .min_by_key(|(_, g)| priority(g))
            .map(|(i, _)| i)
        {
            let g = self.comms.atc.cue_queue.remove(i);
            self.comms.atc.next_feedback_s = now + 8.;
            self.start_local(
                Channel::Intercom,
                Speaker::Instructor,
                g.observed.clone(),
                Meaning::InstructorFeedback { message: g.observed },
                EventSource::Rules,
            );
        }
    }
    fn observation(&mut self, id: Option<u64>, item: &str, passed: bool, observed: String) {
        let key = format!("{id:?}:{item}");
        if self.comms.atc.last_observed.get(&key) != Some(&passed) {
            if self.comms.atc.last_observed.len() >= 1024 {
                self.comms.atc.last_observed.clear();
            }
            self.comms.atc.last_observed.insert(key, passed);
            self.atc_grade(id, item, passed, observed);
        }
    }
    pub(super) fn monitor_atc(&mut self) {
        let now = self.sim.state().time_s;
        let missing: Vec<_> = self
            .comms
            .atc
            .pending
            .iter()
            .filter(|p| {
                p.heard
                    && now >= p.reminder_s
                    && !self.comms.pending.iter().any(|t| t.channel == Channel::Com)
                    && self.comms.atc.replies.is_empty()
            })
            .cloned()
            .collect();
        for p in missing {
            if let Some(st) = self.stations().into_iter().find(|st| st.id == p.station_id) {
                self.atc_grade(
                    Some(p.id),
                    "missing_readback",
                    false,
                    "ATC is waiting for your readback.".into(),
                );
                self.atc_reply(
                    &st,
                    format!("Say again readback. {}", p.phrase),
                    Meaning::Atc {
                        instruction: p.instruction,
                    },
                );
            }
            if let Some(p) = self.comms.atc.pending.iter_mut().find(|i| i.id == p.id) {
                p.reminder_s = now + 30.;
            }
        }
        let [n, e, d] = self.sim.state().position_m;
        let r = self.sim.readout();
        let instructions: Vec<_> = self
            .comms
            .atc
            .active
            .iter()
            .chain(self.comms.atc.pending.iter())
            .cloned()
            .collect();
        for p in instructions {
            let elapsed = now - p.issued_s;
            match &p.instruction {
                AtcInstruction::Taxi {
                    runway,
                    hold_short: true,
                    ..
                } if p.heard => {
                    let inside = self.on_atc_runway(&p.airport, runway);
                    self.observation(
                        Some(p.id),
                        "hold_short",
                        !inside,
                        if inside {
                            format!(
                                "You crossed the hold-short line at runway {}.",
                                runway.trim_start_matches('0')
                            )
                        } else {
                            "You are holding short of the runway.".into()
                        },
                    );
                }
                AtcInstruction::Vector {
                    heading_deg,
                    altitude_ft,
                }
                | AtcInstruction::Bravo {
                    cleared: true,
                    heading_deg,
                    altitude_ft,
                } if p.readback && elapsed >= 30. => {
                    let actual = r.heading_rad.to_degrees().rem_euclid(360.);
                    let error = ((actual - f64::from(*heading_deg) + 180.).rem_euclid(360.) - 180.).abs();
                    self.observation(
                        Some(p.id),
                        "heading",
                        error <= 10.,
                        format!(
                            "Assigned heading {heading_deg:03}, you're on {:03}.",
                            actual.round() as u16
                        ),
                    );
                    self.check_atc_altitude(&p, *altitude_ft);
                }
                AtcInstruction::Altitude { altitude_ft } if p.readback && elapsed >= 30. => {
                    self.check_atc_altitude(&p, *altitude_ft)
                }
                AtcInstruction::Squawk { code } | AtcInstruction::IfrClearance { code, .. }
                    if p.readback && elapsed >= 10. =>
                {
                    self.observation(
                        Some(p.id),
                        "squawk",
                        self.glass.transponder_code == *code,
                        if self.glass.transponder_code == *code {
                            format!("Squawk {code} is set.")
                        } else {
                            format!("Squawk is still {} — set {code}.", self.glass.transponder_code)
                        },
                    );
                    if let AtcInstruction::IfrClearance { altitude_ft, .. } = &p.instruction
                        && !r.on_ground
                        && self.comms.atc.was_airborne
                        && elapsed >= 30.
                    {
                        self.check_atc_altitude(&p, *altitude_ft);
                    }
                }
                AtcInstruction::Handoff { channel_khz, .. }
                | AtcInstruction::ExitRunway {
                    ground_khz: channel_khz,
                    ..
                } if p.readback => {
                    let tuned = self
                        .comms
                        .radios
                        .get(usize::from(self.comms.panel.transmit.saturating_sub(1)))
                        .map(|r| r.active_khz);
                    if elapsed >= 10. || tuned == Some(*channel_khz) {
                        self.observation(
                            Some(p.id),
                            "frequency_change",
                            tuned == Some(*channel_khz),
                            if tuned == Some(*channel_khz) {
                                format!(
                                    "Frequency change complete: {}.",
                                    phraseology::frequency(*channel_khz)
                                )
                            } else {
                                format!(
                                    "Tune {}{}.",
                                    phraseology::frequency(*channel_khz),
                                    tuned.map_or(String::new(), |v| format!(
                                        "; the selected channel is {}",
                                        phraseology::frequency(v)
                                    ))
                                )
                            },
                        );
                    }
                }
                _ => {}
            }
        }
        self.comms.atc.active.retain(|p| {
            let channel = match p.instruction {
                AtcInstruction::Handoff { channel_khz, .. } => Some(channel_khz),
                AtcInstruction::ExitRunway { ground_khz, .. } => Some(ground_khz),
                _ => None,
            };
            !channel.is_some_and(|khz| {
                self.comms
                    .radios
                    .get(usize::from(self.comms.panel.transmit.saturating_sub(1)))
                    .is_some_and(|r| r.active_khz == khz)
            })
        });
        if !r.on_ground
            && let Ok(query) = self.sim.world().airspace_at(n, e, -d)
        {
            for volume in query.containing {
                let item = format!("airspace:{}", volume.id);
                let passed = match volume.class {
                    Some(AirspaceClass::B) => {
                        self.comms.atc.ifr_clearance
                            || self.comms.atc.bravo.as_ref().is_some_and(|p| {
                                p == "NEW YORK CLASS B" && volume.name.to_uppercase().contains("NEW YORK")
                            })
                    }
                    Some(AirspaceClass::C | AirspaceClass::D) => self.provider_contact_for(&volume.name),
                    _ => continue,
                };
                self.observation(
                    None,
                    &item,
                    passed,
                    if passed {
                        format!(
                            "Your clearance or contact covers {}.",
                            phraseology::normal_case(&volume.name)
                        )
                    } else {
                        format!(
                            "You entered {} without {}.",
                            phraseology::normal_case(&volume.name),
                            if volume.class == Some(AirspaceClass::B) {
                                "a Class Bravo clearance"
                            } else {
                                "two-way communication with the controlling facility"
                            }
                        )
                    },
                );
            }
        }
        self.comms.atc.was_airborne |= !r.on_ground && r.height_agl_m > 10.;
        let towered_touchdown = r.on_ground
            && self.comms.atc.last_ground == Some(false)
            && self.comms.atc.was_airborne
            && self
                .stations()
                .iter()
                .filter(|st| st.kind == FacilityKind::Tower)
                .any(|st| {
                    crate::regions::airport(self.sim.world(), &st.airport).is_ok_and(|a| {
                        a.runways.iter().any(|r| {
                            r.ends
                                .first()
                                .is_some_and(|e| self.on_atc_runway(&st.airport, &e.ident))
                        })
                    })
                });
        if towered_touchdown {
            let light_cleared = self.sim.failures().radio_failure
                && self
                    .comms
                    .atc
                    .light_gun
                    .as_ref()
                    .is_some_and(|g| g.signal == LightGunSignal::SteadyGreen);
            self.observation(
                None,
                "landing_clearance",
                self.comms.atc.landing || light_cleared,
                "Touchdown checked for a landing clearance or steady-green signal.".into(),
            );
        }
        self.comms.atc.last_ground = Some(r.on_ground);
        if self.sim.failures().radio_failure {
            self.auto_light_gun();
            return;
        }
        let Some(airport) = self.comms.atc.airport.clone() else {
            return;
        };
        let Some(st) = self.comms.atc.current.clone() else {
            return;
        };
        if self.comms.atc.pending.is_empty()
            && self.comms.atc.replies.is_empty()
            && !self.comms.pending.iter().any(|p| p.channel == Channel::Com)
        {
            if st.kind == FacilityKind::Tower
                && r.on_ground
                && r.ground_speed_mps < 15.
                && self.comms.atc.was_airborne
                && self.comms.atc.landing
                && let Some(runway) = self.comms.atc.active.iter().find_map(|p| match &p.instruction {
                    AtcInstruction::Land { runway }
                        if p.readback && p.airport == airport && self.on_atc_runway(&airport, runway) =>
                    {
                        Some(runway.clone())
                    }
                    _ => None,
                })
                && let Some(ground) = self.next_station(&airport, FacilityKind::Ground)
            {
                self.issue(
                    &st,
                    AtcInstruction::ExitRunway {
                        runway,
                        ground_khz: ground.channel_khz,
                    },
                );
            } else if st.kind == FacilityKind::Clearance
                && self.comms.atc.active.iter().any(|p| {
                    p.station_id == st.id
                        && matches!(
                            p.instruction,
                            AtcInstruction::IfrClearance { .. } | AtcInstruction::Squawk { .. }
                        )
                })
            {
                if let Some(next) = self.next_station(&airport, FacilityKind::Ground)
                    && self.comms.atc.last_sector.as_ref() != Some(&next.id)
                {
                    self.handoff(&st, &next);
                }
            } else if st.kind == FacilityKind::Ground
                && r.on_ground
                && self
                    .comms
                    .atc
                    .active
                    .iter()
                    .any(|p| p.station_id == st.id && matches!(p.instruction, AtcInstruction::Taxi { .. }))
            {
                if let Some(next) = self.next_station(&airport, FacilityKind::Tower)
                    && self.comms.atc.last_sector.as_ref() != Some(&next.id)
                {
                    self.handoff(&st, &next);
                }
            } else if st.kind == FacilityKind::Tower
                && self.comms.atc.takeoff
                && r.height_agl_m > 60.
                && !self.comms.atc.closed_traffic
            {
                if let Some(dep) = self.next_station(&airport, FacilityKind::Departure) {
                    self.comms.atc.takeoff = false;
                    self.handoff(&st, &dep);
                }
            } else if st.kind == FacilityKind::Tower
                && self
                    .comms
                    .atc
                    .active
                    .iter()
                    .any(|p| matches!(p.instruction, AtcInstruction::ExtendDownwind))
                && !self.runway_traffic(&airport)
            {
                if let Some(runway) = self.active_runway(&airport) {
                    self.comms
                        .atc
                        .active
                        .retain(|p| !matches!(p.instruction, AtcInstruction::ExtendDownwind));
                    self.issue(
                        &st,
                        AtcInstruction::Pattern {
                            runway: runway.clone(),
                            entry: PatternEntry::LeftBase,
                            number: 1,
                            follow: None,
                        },
                    );
                }
            } else if st.kind == FacilityKind::Approach
                && self.comms.atc.inbound
                && let Ok(a) = crate::regions::airport(self.sim.world(), &airport)
                && let Some([an, ae]) = self.radio_project(a.latitude_deg, a.longitude_deg)
                && libm::hypot(n - an, e - ae) <= 10. * 1852.
                && let Some(tower) = self.next_station(&airport, FacilityKind::Tower)
            {
                self.comms.atc.inbound = false;
                self.handoff(&st, &tower);
            } else if matches!(st.kind, FacilityKind::Approach | FacilityKind::Departure)
                && let Some(destination) = self.comms.atc.destination.clone()
                && destination != airport
                && let Ok(a) = crate::regions::airport(self.sim.world(), &destination)
                && let Some([dn, de]) = self.radio_project(a.latitude_deg, a.longitude_deg)
                && libm::hypot(n - dn, e - de) < 30. * 1852.
                && let Some(next) = self.next_station(&destination, FacilityKind::Approach)
                && self.comms.atc.last_sector.as_ref() != Some(&next.id)
            {
                self.handoff(&st, &next);
            } else if matches!(st.kind, FacilityKind::Approach | FacilityKind::Departure)
                && let Some(next) = self.sector_station(&airport, st.kind)
                && next.id != st.id
                && next.channel_khz != st.channel_khz
                && radar_facility(&next) != radar_facility(&st)
                && self.comms.atc.last_sector.as_ref() != Some(&next.id)
            {
                self.handoff(&st, &next);
            }
        }
    }
    fn check_atc_altitude(&mut self, p: &PendingInstruction, assigned: u32) {
        if self.comms.atc.active.iter().any(|new| {
            new.id > p.id
                && matches!(
                    new.instruction,
                    AtcInstruction::Altitude { .. }
                        | AtcInstruction::Vector { .. }
                        | AtcInstruction::Bravo { cleared: true, .. }
                        | AtcInstruction::Approach { .. }
                        | AtcInstruction::Land { .. }
                )
        }) {
            return;
        }
        let feet = -self.sim.state().position_m[2] / 0.3048;
        let error = f64::from(assigned) - feet;
        let attained = self
            .comms
            .atc
            .last_observed
            .get(&format!("{:?}:altitude", Some(p.id)))
            == Some(&true);
        let climbing = -self.sim.state().velocity_mps[2];
        if !attained && error.abs() > 100. && error.signum() * climbing > 0.3 {
            return;
        }
        self.observation(
            Some(p.id),
            "altitude",
            error.abs() <= 100.,
            if error.abs() <= 100. {
                format!("You are level at the assigned altitude of {assigned} feet.")
            } else {
                format!("Assigned altitude {assigned} feet; you're at {feet:.0} feet.")
            },
        );
    }
    pub(crate) fn provider_contact_for(&self, name: &str) -> bool {
        let name = words(name);
        self.stations().iter().any(|st| {
            self.comms.atc.two_way.contains(&provider(st))
                && st.metadata.as_ref().is_some_and(|m| {
                    words(&m.callsign)
                        .split_whitespace()
                        .any(|w| w.len() > 3 && name.contains(w))
                })
        })
    }
    pub(super) fn on_atc_runway(&self, airport: &str, runway: &str) -> bool {
        let Ok(a) = crate::regions::airport(self.sim.world(), airport) else {
            return false;
        };
        let Some(r) = a
            .runways
            .iter()
            .find(|r| r.ends.iter().any(|e| e.ident == runway))
        else {
            return false;
        };
        let [a, b] = &r.ends;
        let Some([an, ae]) = self.radio_project(a.latitude_deg, a.longitude_deg) else {
            return false;
        };
        let Some([bn, be]) = self.radio_project(b.latitude_deg, b.longitude_deg) else {
            return false;
        };
        let [n, e, _] = self.sim.state().position_m;
        let length = libm::hypot(bn - an, be - ae).max(1.);
        let along = ((n - an) * (bn - an) + (e - ae) * (be - ae)) / length;
        let side = (-(n - an) * (be - ae) + (e - ae) * (bn - an)) / length;
        (-30. ..=length + 30.).contains(&along) && side.abs() <= r.width_m / 2. + 30.
    }
    fn auto_light_gun(&mut self) {
        if self.comms.atc.light_override {
            return;
        }
        let Some(airport) = self.comms.atc.airport.clone() else {
            return;
        };
        let Some(runway) = self.active_runway(&airport) else {
            return;
        };
        let r = self.sim.readout();
        let near = self.comms.atc.light_gun.as_ref().is_some_and(|g| {
            let [n, e, _] = g.position_m;
            let [pn, pe, _] = self.sim.state().position_m;
            libm::hypot(n - pn, e - pe) < 8. * 1852.
        });
        if !near {
            return;
        }
        let signal = if r.on_ground {
            if self.comms.atc.was_airborne && self.on_atc_runway(&airport, &runway) {
                LightGunSignal::FlashingRed
            } else if self.runway_traffic(&airport) {
                LightGunSignal::SteadyRed
            } else {
                LightGunSignal::FlashingGreen
            }
        } else if self.runway_traffic(&airport) {
            LightGunSignal::SteadyRed
        } else if self.comms.pattern.as_ref().and_then(|p| p.pilot_leg(self)) == Some(PatternLeg::Final)
            || r.height_agl_m < 200.
        {
            LightGunSignal::SteadyGreen
        } else {
            LightGunSignal::FlashingGreen
        };
        if self
            .comms
            .atc
            .light_gun
            .as_ref()
            .is_none_or(|g| g.signal != signal)
        {
            let _ = self.set_light_gun(&airport, signal);
        }
    }
}
