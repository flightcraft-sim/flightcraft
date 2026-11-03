use super::*;
fn position(s: &Session) -> Option<GeoPoint> {
    let r = s.sim.readout();
    Some(GeoPoint {
        latitude_deg: r.latitude_deg?,
        longitude_deg: r.longitude_deg?,
    })
}
fn airspaces(s: &Session) -> Vec<String> {
    let [n, e, d] = s.sim.state().position_m;
    s.sim
        .world()
        .airspace_at(n, e, -d)
        .map(|q| q.containing.into_iter().map(|v| v.id).collect())
        .unwrap_or_default()
}
fn phase(s: &Session) -> FlightPhase {
    let r = s.sim.readout();
    if r.on_ground {
        if r.indicated_airspeed_mps < 2. {
            FlightPhase::Landed
        } else if r.indicated_airspeed_mps < 15. {
            FlightPhase::Taxi
        } else {
            FlightPhase::Takeoff
        }
    } else if r.height_agl_m < 150. && r.vertical_speed_mps > 0. {
        FlightPhase::Takeoff
    } else if r.vertical_speed_mps > 1. {
        FlightPhase::Climb
    } else if r.vertical_speed_mps < -1. {
        if r.height_agl_m < 600. {
            FlightPhase::Approach
        } else {
            FlightPhase::Descent
        }
    } else {
        FlightPhase::Cruise
    }
}
impl Session {
    fn scenario_condition(&self, t: &Trigger, index: usize) -> bool {
        let rt = &self.scenario;
        let r = self.sim.readout();
        let elapsed = r.time_s - rt.started_s;
        let calls: Vec<_> = self
            .flight_record()
            .events
            .iter()
            .filter(|e| {
                e.id > rt.event_cursor
                    && e.time_s >= rt.previous_s
                    && e.speaker == Speaker::Pilot
                    && !matches!(e.meaning, Meaning::Recognition { .. })
                    && e.channel == Channel::Com
                    && e.time_s + e.duration_s <= r.time_s
                    && !matches!(e.reception, Reception::Blocked | Reception::NotReceived)
            })
            .collect();
        match t {
            Trigger::Distance {
                reference: id,
                within_nm,
            } => position(self)
                .zip(reference(self, id))
                .is_some_and(|(a, b)| a.inverse(b).0 <= *within_nm * 1852.),
            Trigger::DepartingRunway {
                airport,
                runway: end,
                within_nm,
            } => {
                let Some(runway) = runway(self, airport, end) else {
                    return false;
                };
                let p = GeoPoint {
                    latitude_deg: runway.latitude_deg,
                    longitude_deg: runway.longitude_deg,
                };
                position(self).is_some_and(|own| {
                    let (distance, bearing) = p.inverse(own);
                    let delta = wrap((bearing - runway.heading_deg).to_radians());
                    !r.on_ground
                        && distance <= within_nm * 1852.
                        && libm::cos(delta) > 0.
                        && (libm::sin(delta) * distance).abs() < 0.25 * 1852.
                        && wrap(r.heading_true_rad - runway.heading_deg.to_radians()).abs()
                            < 20_f64.to_radians()
                })
            }
            Trigger::OnFinal {
                airport,
                runway: end,
                within_nm,
            } => {
                let Some(runway) = runway(self, airport, end) else {
                    return false;
                };
                let p = GeoPoint {
                    latitude_deg: runway.latitude_deg,
                    longitude_deg: runway.longitude_deg,
                };
                let Some(own) = position(self) else {
                    return false;
                };
                let (distance, bearing) = p.inverse(own);
                let delta = wrap((bearing - runway.heading_deg - 180.).to_radians());
                let heading = wrap(r.heading_true_rad - runway.heading_deg.to_radians());
                distance <= *within_nm * 1852.
                    && distance > 50.
                    && (libm::sin(delta) * distance).abs() < 0.25 * 1852.
                    && libm::cos(delta) > 0.
                    && heading.abs() < 20_f64.to_radians()
                    && !r.on_ground
            }
            Trigger::CrossingRadial {
                reference: id,
                radial_deg,
            } => {
                let Some(origin) = reference(self, id) else {
                    return false;
                };
                rt.previous_position.zip(position(self)).is_some_and(|(a, b)| {
                    let old = wrap((origin.inverse(a).1 - *radial_deg).to_radians());
                    let new = wrap((origin.inverse(b).1 - *radial_deg).to_radians());
                    old * new <= 0. && (old - new).abs() < core::f64::consts::PI && a != b
                })
            }
            Trigger::EnteringAirspace { id } => {
                !rt.previous_airspaces.contains(id) && airspaces(self).contains(id)
            }
            Trigger::HeightAgl { min_m, max_m } => (*min_m..=*max_m).contains(&r.height_agl_m),
            Trigger::Airspeed { min_mps, max_mps } => {
                (*min_mps..=*max_mps).contains(&r.indicated_airspeed_mps)
            }
            Trigger::Phase { phase: expected } => phase(self) == *expected,
            Trigger::Configuration { min_flaps, max_flaps } => {
                (*min_flaps..=*max_flaps).contains(&r.effective_controls.flaps)
            }
            Trigger::TunedFrequency { channel_khz } => {
                let c = self.comms_state();
                [c.com1, c.com2].iter().any(|r| r.active_khz == *channel_khz)
            }
            Trigger::PilotCall { contains } => calls.iter().any(|c| contains_phrase(&c.text, contains)),
            Trigger::Readback {
                contains,
                result,
                timeout_s,
            } => {
                let correct = calls.iter().any(|c| contains_phrase(&c.text, contains));
                match result {
                    ReadbackResult::Correct => correct,
                    ReadbackResult::Incorrect => !calls.is_empty() && !correct,
                    ReadbackResult::Missing => calls.is_empty() && r.time_s - rt.previous_s >= *timeout_s,
                }
            }
            Trigger::Checklist { item } => rt.checklist.contains(item),
            Trigger::Time {
                seconds,
                since_previous,
            } => {
                r.time_s
                    - if *since_previous {
                        rt.previous_s
                    } else {
                        rt.started_s
                    }
                    >= *seconds
            }
            Trigger::RandomWindow { earliest_s, latest_s } => {
                let seed = rt.document.as_ref().map_or(0, |d| d.seed);
                let mixed = seed.wrapping_add((index as u64).wrapping_mul(0x9e3779b97f4a7c15));
                let mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
                let mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d049bb133111eb);
                let unit = ((mixed ^ (mixed >> 31)) >> 11) as f64 / 9007199254740992.;
                elapsed >= earliest_s + (latest_s - earliest_s) * unit
            }
            Trigger::Event { name } => rt.signals.contains(name),
        }
    }
    pub(crate) fn sequence_scenario(&mut self) {
        if self.scenario.generation != self.flight_generation {
            self.scenario = ScenarioRuntime {
                generation: self.flight_generation,
                ..Default::default()
            };
        }
        if self.scenario.document.is_none()
            && self.scenario.scheduled.is_empty()
            && self.scenario.state.emergency.is_none()
        {
            return;
        }
        let now = self.sim.state().time_s;
        let due: Vec<_> = self
            .scenario
            .scheduled
            .iter()
            .filter(|(time, _)| *time <= now)
            .map(|(_, f)| *f)
            .collect();
        self.scenario.scheduled.retain(|(time, _)| *time > now);
        for failure in due {
            let action = ScenarioAction::FailureSet { failure };
            let mut next = self.scenario.state.step;
            let result = self.scenario_action(&action, EventSource::Rules, &mut next);
            self.record_scenario_progress(ScenarioEvent {
                time_s: now,
                step: "instructor".into(),
                source: EventSource::Rules,
                action,
                error: result.err().map(|e| e.to_string()),
            });
        }
        // One step per fixed tick; forward-only branching and bounded documents prevent loops.
        let index = self.scenario.state.step;
        let step = self
            .scenario
            .document
            .as_ref()
            .and_then(|d| d.steps.get(index))
            .cloned();
        if let Some(step) = step {
            let wait_met = self
                .scenario
                .wait
                .as_ref()
                .is_none_or(|t| self.scenario_condition(t, index));
            self.scenario.state.waiting = !wait_met;
            if wait_met && step.when.iter().all(|t| self.scenario_condition(t, index)) {
                self.scenario.wait = None;
                let mut next = index + 1;
                let source = if self.scenario.document.as_ref().is_some_and(|d| d.ai_written) {
                    EventSource::ScenarioAi
                } else {
                    EventSource::Scenario
                };
                for action in step.actions {
                    let result = self.scenario_action(&action, source, &mut next);
                    self.record_scenario_progress(ScenarioEvent {
                        time_s: self.sim.state().time_s,
                        step: step.id.clone(),
                        source,
                        action,
                        error: result.err().map(|e| e.to_string()),
                    });
                }
                self.scenario.state.step = next;
                self.scenario.previous_s = self.sim.state().time_s;
                self.scenario.event_cursor = self.flight_record().events.last().map_or(0, |e| e.id);
                self.scenario.state.complete = self
                    .scenario
                    .document
                    .as_ref()
                    .is_some_and(|d| next >= d.steps.len())
                    && self.scenario.wait.is_none();
            }
        } else if self
            .scenario
            .wait
            .as_ref()
            .is_some_and(|t| self.scenario_condition(t, index))
        {
            self.scenario.wait = None;
            self.scenario.state.waiting = false;
            self.scenario.state.complete = true;
        }
        self.scenario.previous_position = position(self);
        self.scenario.previous_airspaces = airspaces(self);
        self.update_emergency();
    }
    fn record_scenario_progress(&mut self, event: ScenarioEvent) {
        if self.scenario.state.events.len() < crate::comms::MAX_EVENTS {
            self.scenario.state.events.push(event);
        } else {
            self.scenario.state.dropped_events += 1;
        }
    }
    fn scenario_action(
        &mut self,
        a: &ScenarioAction,
        source: EventSource,
        next: &mut usize,
    ) -> Result<(), CommandError> {
        if !matches!(
            a,
            ScenarioAction::Transmission { .. } | ScenarioAction::Cue { .. }
        ) {
            self.record_scenario_action(a.clone(), source);
        }
        match a {
            ScenarioAction::Transmission {
                facility,
                channel_khz,
                role,
                instruction,
            } => {
                self.scenario_transmission(facility, *channel_khz, *role, instruction.clone(), source)?;
            }
            ScenarioAction::FailureSet { failure } => {
                SetFailure { failure: *failure }.run(self)?;
                self.begin_emergency(source);
            }
            ScenarioAction::FailureClear { target } => {
                ClearFailure { target: *target }.run(self)?;
            }
            ScenarioAction::Weather { conditions } => self.sim.set_conditions(conditions.clone())?,
            ScenarioAction::TrafficSpawn {
                airport,
                runway,
                count,
            } => {
                SetTraffic {
                    airport: airport.clone(),
                    runway: Some(runway.clone()),
                    count: *count,
                    right_pattern: false,
                }
                .run(self)?;
            }
            ScenarioAction::Cue { text } => {
                self.start_local(
                    Channel::Intercom,
                    Speaker::Instructor,
                    text.clone(),
                    Meaning::Scenario {
                        instruction: Instruction::Advisory { text: text.clone() },
                    },
                    source,
                );
            }
            ScenarioAction::Wait { condition } => {
                self.scenario.wait = Some(condition.clone());
                self.scenario.state.waiting = true;
            }
            ScenarioAction::Branch {
                condition,
                then_step,
                else_step,
            } => {
                let target = if self.scenario_condition(condition, self.scenario.state.step) {
                    then_step
                } else {
                    else_step
                };
                *next = self
                    .scenario
                    .document
                    .as_ref()
                    .and_then(|d| d.steps.iter().position(|st| st.id == *target))
                    .ok_or_else(|| invalid("scenario", "missing branch target"))?;
            }
            ScenarioAction::EmitEvent { name } => {
                if self.scenario.signals.len() >= 512 {
                    return Err(invalid("scenario", "at most 512 named events"));
                }
                self.scenario.signals.insert(name.clone());
            }
        }
        Ok(())
    }
    pub(crate) fn scenario_transmission(
        &mut self,
        facility: &str,
        khz: u32,
        role: FacilityKind,
        instruction: Instruction,
        source: EventSource,
    ) -> Result<Transmission, CommandError> {
        if self.flight_record().events.len() >= crate::comms::MAX_EVENTS {
            return Err(invalid("comms.atc", "flight event record is full"));
        }
        let station = self
            .stations()
            .into_iter()
            .find(|st| st.id == facility)
            .ok_or_else(|| invalid("scenario", "missing facility"))?;
        let [n, e] = self
            .radio_project(station.position.latitude_deg, station.position.longitude_deg)
            .ok_or_else(|| invalid("scenario", "missing facility projection"))?;
        if let Instruction::AirspaceClearance { id } = &instruction {
            self.scenario.granted_clearances.insert(id.clone());
        }
        if let Instruction::Controller { instruction } = instruction {
            return self.controller_transmission(&station, instruction, source);
        }
        let text = crate::comms::phraseology::scripted_transmission(
            &station,
            &self.pilot_callsign(),
            &instruction,
            self.pilot_weather(),
            self,
        );
        let id = self.start_com(
            khz,
            Speaker::Facility {
                id: facility.into(),
                role,
            },
            text,
            Meaning::Scenario { instruction },
            source,
            [n, e, -station.elevation_m - station.antenna_agl_m],
            station.range_nm,
        );
        self.flight_record()
            .events
            .iter()
            .find(|e| e.id == id)
            .cloned()
            .ok_or_else(|| invalid("comms.atc", "transmission was not recorded"))
    }
}
fn wrap(v: f64) -> f64 {
    (v + core::f64::consts::PI).rem_euclid(core::f64::consts::TAU) - core::f64::consts::PI
}
