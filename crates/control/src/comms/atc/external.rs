//! Live MCP controllers share the rules engine's typed readback and compliance path.
use super::*;
impl Session {
    pub(crate) fn validate_controller(
        &self,
        facility: &str,
        khz: u32,
        role: FacilityKind,
        i: &AtcInstruction,
    ) -> Vec<String> {
        let mut errors = Vec::new();
        let station = self.sim.world().navdata().and_then(|n| {
            n.comms
                .iter()
                .find(|st| st.id == facility && st.channel_khz == khz && st.kind == role)
        });
        let Some(station) = station else {
            return vec!["facility, role, and frequency must match an FAA station".into()];
        };
        let airport = crate::regions::airport(self.sim.world(), &station.airport).ok();
        let runway_ok = |r: &str| {
            airport
                .as_ref()
                .is_some_and(|a| a.runways.iter().flat_map(|r| &r.ends).any(|e| e.ident == r))
        };
        let altitude_ok = |ft: u32| {
            (1000..=60000).contains(&ft)
                && f64::from(ft) * 0.3048
                    >= self
                        .sim
                        .world()
                        .ground(self.sim.state().position_m[0], self.sim.state().position_m[1])
                        .height_msl_m
                        + 304.8
        };
        let radar = matches!(role, FacilityKind::Approach | FacilityKind::Departure);
        let ground = self.sim.world().navdata().is_some_and(|n| {
            n.comms.iter().any(|st| {
                st.airport == station.airport
                    && st.kind == FacilityKind::Ground
                    && match i {
                        AtcInstruction::ExitRunway { ground_khz, .. } => st.channel_khz == *ground_khz,
                        _ => true,
                    }
            })
        });
        let valid = match i {
            AtcInstruction::Taxi {
                runway,
                route,
                hold_short,
            } => role == FacilityKind::Ground && runway_ok(runway) && route.is_empty() && *hold_short,
            AtcInstruction::Parking => role == FacilityKind::Ground,
            AtcInstruction::Takeoff { runway, .. }
            | AtcInstruction::Land { runway }
            | AtcInstruction::TouchAndGo { runway }
            | AtcInstruction::GoAround { runway } => role == FacilityKind::Tower && runway_ok(runway),
            AtcInstruction::Pattern {
                runway,
                number,
                follow,
                ..
            } => {
                role == FacilityKind::Tower
                    && runway_ok(runway)
                    && (1..=4).contains(number)
                    && (*number == 1 || follow.is_some())
                    && follow
                        .as_ref()
                        .is_none_or(|call| self.traffic_state().aircraft.iter().any(|a| a.callsign == *call))
            }
            AtcInstruction::ExtendDownwind => role == FacilityKind::Tower,
            AtcInstruction::Vector {
                heading_deg,
                altitude_ft,
            } => radar && *heading_deg <= 360 && altitude_ok(*altitude_ft),
            AtcInstruction::Altitude { altitude_ft } => radar && altitude_ok(*altitude_ft),
            AtcInstruction::Ident => radar,
            AtcInstruction::Squawk { code, .. } => {
                (radar || role == FacilityKind::Clearance)
                    && code.len() == 4
                    && code.bytes().all(|b| (b'0'..=b'7').contains(&b))
            }
            AtcInstruction::Handoff {
                station_id,
                channel_khz,
            } => self.sim.world().navdata().is_some_and(|n| {
                n.comms.iter().any(|st| {
                    st.id == *station_id
                        && st.channel_khz == *channel_khz
                        && matches!(
                            st.kind,
                            FacilityKind::Tower
                                | FacilityKind::Ground
                                | FacilityKind::Approach
                                | FacilityKind::Departure
                                | FacilityKind::Clearance
                        )
                })
            }),
            AtcInstruction::ExitRunway { runway, .. } => {
                role == FacilityKind::Tower && runway_ok(runway) && ground
            }
            AtcInstruction::Speed { .. } => radar,
            // These require additional route, separation, or procedure authority. Use rules mode.
            AtcInstruction::Hold { .. }
            | AtcInstruction::MissedApproach { .. }
            | AtcInstruction::Bravo { .. }
            | AtcInstruction::Approach { .. }
            | AtcInstruction::IfrClearance { .. } => false,
        };
        if !valid {
            errors.push("instruction must match the FAA facility role, runway, channel, numeric limits, and supported authority; named taxiways, Bravo, IFR, and approach clearances require rules mode".into());
        }
        if !self.comms.atc.external_controller {
            errors.push(
                "select external_controller in atc.configure before issuing controller instructions".into(),
            );
        }
        if !self.comms.atc.pending.is_empty() {
            errors.push("wait for the outstanding readback before issuing another instruction".into());
        }
        if self.sim.failures().radio_failure {
            errors.push("COM radio failure".into());
        }
        if self.flight_record().events.len() >= MAX_EVENTS {
            errors.push("flight event record is full".into());
        }
        errors
    }
    pub(crate) fn controller_transmission(
        &mut self,
        station: &ComStation,
        instruction: AtcInstruction,
        source: EventSource,
    ) -> Result<Transmission, CommandError> {
        let origin = self
            .station_origin(station)
            .ok_or_else(|| invalid("comms.atc", "facility has no regional position"))?;
        let phrase = self.instruction_phrase(&instruction);
        let text = phraseology::scripted_transmission(
            station,
            &self.pilot_callsign(),
            &crate::Instruction::Controller {
                instruction: instruction.clone(),
            },
            self.pilot_weather(),
            self,
        );
        let id = self.start_com(
            station.channel_khz,
            Speaker::Facility {
                id: station.id.clone(),
                role: station.kind,
            },
            text,
            Meaning::Atc {
                instruction: instruction.clone(),
            },
            source,
            origin,
            station.range_nm,
        );
        self.comms.atc.pending.push(PendingInstruction {
            id,
            station_id: station.id.clone(),
            airport: station.airport.clone(),
            instruction,
            phrase,
            heard: false,
            readback: false,
            issued_s: self.sim.state().time_s,
            reminder_s: f64::MAX,
        });
        self.flight_record()
            .events
            .iter()
            .find(|e| e.id == id)
            .cloned()
            .ok_or_else(|| invalid("comms.atc", "transmission was not recorded"))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate as flightcraft_control;
    use serde_json::json;
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/atc_fixture.rs"));
    #[test]
    fn live_controller_rejects_invented_data_and_role_mismatches() {
        let mut s = atc_session("KABE");
        s.execute(
            "atc.configure",
            json!({"airport":"KABE","external_controller":true}),
        )
        .unwrap();
        let tower = s
            .comms_state()
            .stations
            .into_iter()
            .find(|v| v.airport == "KABE" && v.kind == FacilityKind::Tower)
            .unwrap();
        for (id, channel, role, instruction) in [
            (
                "invented",
                tower.channel_khz,
                tower.kind,
                AtcInstruction::Land { runway: "06".into() },
            ),
            (
                tower.id.as_str(),
                123450,
                tower.kind,
                AtcInstruction::Land { runway: "06".into() },
            ),
            (
                tower.id.as_str(),
                tower.channel_khz,
                FacilityKind::Ground,
                AtcInstruction::Land { runway: "06".into() },
            ),
            (
                tower.id.as_str(),
                tower.channel_khz,
                tower.kind,
                AtcInstruction::Land { runway: "37".into() },
            ),
            (
                tower.id.as_str(),
                tower.channel_khz,
                tower.kind,
                AtcInstruction::Taxi {
                    runway: "06".into(),
                    route: vec!["Invented taxiway".into()],
                    hold_short: true,
                },
            ),
            (
                tower.id.as_str(),
                tower.channel_khz,
                tower.kind,
                AtcInstruction::Squawk {
                    code: "8899".into(),
                    ident: false,
                },
            ),
            (
                tower.id.as_str(),
                tower.channel_khz,
                tower.kind,
                AtcInstruction::Handoff {
                    station_id: "invented".into(),
                    channel_khz: 123450,
                },
            ),
        ] {
            let before = s.flight_record().events.len();
            assert!(!s.validate_controller(id, channel, role, &instruction).is_empty());
            assert_eq!(s.flight_record().events.len(), before);
        }
        assert!(
            s.validate_controller(
                &tower.id,
                tower.channel_khz,
                tower.kind,
                &AtcInstruction::Land { runway: "06".into() }
            )
            .is_empty()
        );
        // Ident is a radar instruction: a tower may not ask for it, a departure controller may.
        let departure = s
            .comms_state()
            .stations
            .into_iter()
            .find(|v| v.airport == "KABE" && v.kind == FacilityKind::Departure)
            .unwrap();
        for ident in [
            AtcInstruction::Ident,
            AtcInstruction::Squawk {
                code: "4321".into(),
                ident: true,
            },
        ] {
            assert!(
                !s.validate_controller(&tower.id, tower.channel_khz, tower.kind, &ident)
                    .is_empty()
            );
            assert!(
                s.validate_controller(&departure.id, departure.channel_khz, departure.kind, &ident)
                    .is_empty(),
                "{ident:?}"
            );
        }
    }
}
