//! Deterministic tower and radar rules, driven only by completed, received pilot calls and flight time.
use super::*;
mod monitor;
mod parser;
mod phrase;
mod rules;
mod types;
use flightcraft_world::AirspaceClass;
use parser::letter;
pub use parser::{PilotMessage, PilotRequest, parse_pilot};
pub use types::*;

#[derive(Debug, Default, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct GetAtcState {}
impl Command for GetAtcState {
    const NAME: &'static str = "atc.state";
    const DESCRIPTION: &'static str =
        "Read current facility, ATIS, expected call, readbacks, compliance, and lost-communications signals.";
    type Output = AtcState;
    fn run(self, s: &mut Session) -> Result<AtcState, CommandError> {
        Ok(s.atc_state())
    }
}
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigureAtc {
    pub airport: String,
    #[serde(default)]
    pub destination: Option<String>,
}
impl Command for ConfigureAtc {
    const NAME: &'static str = "atc.configure";
    const DESCRIPTION: &'static str = "Select a towered training airport and optional clearance destination; preserves outstanding instructions.";
    type Output = AtcState;
    fn run(self, s: &mut Session) -> Result<AtcState, CommandError> {
        let a = crate::regions::airport(s.sim.world(), &self.airport)?;
        let id = a.icao.clone().unwrap_or_else(|| a.ident.clone());
        if !s
            .stations()
            .iter()
            .any(|st| st.airport == id && st.kind == FacilityKind::Tower)
        {
            return Err(invalid(Self::NAME, "airport has no imported tower"));
        }
        if let Some(d) = &self.destination {
            crate::regions::airport(s.sim.world(), d)?;
        }
        if s.comms.atc.airport.as_ref() != Some(&id) {
            s.comms.atc.reported = None;
            s.comms.atc.current = None;
            s.comms.atc.takeoff = false;
            s.comms.atc.landing = false;
            s.comms.atc.closed_traffic = false;
            s.comms.atc.last_sector = None;
        }
        s.comms.atc.airport = Some(id);
        s.comms.atc.destination = self.destination;
        s.refresh_atis();
        if s.sim.failures().radio_failure && s.comms.atc.light_gun.is_none() {
            s.set_light_gun(&self.airport, LightGunSignal::FlashingGreen)?;
        }
        Ok(s.atc_state())
    }
}
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetLightGun {
    pub airport: String,
    pub signal: LightGunSignal,
}
impl Command for SetLightGun {
    const NAME: &'static str = "atc.light_gun";
    const DESCRIPTION: &'static str = "Select a tower light-gun training indication during radio failure; position is NASR's estimated tower reference.";
    type Output = AtcState;
    fn run(self, s: &mut Session) -> Result<AtcState, CommandError> {
        if !s.sim.failures().radio_failure {
            return Err(invalid(Self::NAME, "requires radio failure"));
        }
        s.set_light_gun(&self.airport, self.signal)?;
        s.comms.atc.light_override = true;
        Ok(s.atc_state())
    }
}
impl Session {
    pub fn atc_state(&self) -> AtcState {
        if let Some(state) = &self.presentation_atc {
            return state.clone();
        }
        let a = &self.comms.atc;
        AtcState {
            airport: a.airport.clone(),
            current_facility: a.current.clone(),
            expected_next_call: if self.sim.failures().radio_failure {
                "Observe tower light-gun signal".into()
            } else if let Some(p) = a.pending.first() {
                format!("Read back: {}", p.phrase)
            } else if a.reported.is_none() {
                "Obtain ATIS; report with information letter".into()
            } else if a.landing {
                "Report clear of the runway".into()
            } else if a.takeoff {
                "Contact departure when airborne".into()
            } else {
                "Call the tuned facility with callsign and request".into()
            },
            atis: a.airport.as_ref().and_then(|id| self.atis_report(id)),
            reported_information: a.reported.as_ref().map(|(_, l)| l.clone()),
            pending_instructions: a.pending.clone(),
            compliance: a.compliance.clone(),
            bravo_clearance: a.bravo.is_some(),
            ifr_clearance: a.ifr_clearance,
            two_way_providers: a.two_way.iter().cloned().collect(),
            radio_failed: self.sim.failures().radio_failure,
            light_gun: a.light_gun.clone(),
            squawk: self.glass.transponder_code.clone(),
        }
    }
    pub(super) fn active_runway(&self, airport: &str) -> Option<String> {
        let a = crate::regions::airport(self.sim.world(), airport).ok()?;
        let wind = self.sim.weather();
        a.runways
            .iter()
            .flat_map(|r| &r.ends)
            .max_by(|a, b| {
                let score = |end: &flightcraft_world::RunwayEnd| {
                    // Imported runway headings are true; wind is grid, converted at the aircraft.
                    let wind_true = (wind.wind_from_deg.to_radians()
                        + self.sim.heading_reference().grid_north_true_rad)
                        .to_degrees();
                    if wind.wind_speed_mps < 0.5 {
                        0.0
                    } else {
                        libm::cos((end.heading_deg - wind_true).to_radians())
                    }
                };
                score(a).total_cmp(&score(b)).then_with(|| b.ident.cmp(&a.ident))
            })
            .map(|e| e.ident.clone())
    }
    pub fn atis_report(&self, airport: &str) -> Option<AtisReport> {
        let station = self
            .stations()
            .into_iter()
            .find(|st| st.airport == airport && st.kind == FacilityKind::Atis)?;
        let runway = self.active_runway(airport)?;
        let info = letter(
            self.comms.atc.info_epochs.get(airport).map_or(0, |(_, n)| *n)
                + (self.sim.state().time_s / 1800.).floor() as u64,
        );
        let (wx, _) = weather::weather_report(
            &station,
            self.sim.conditions(),
            self.pilot_weather(),
            self.sim.state().time_s,
        );
        let wx = wx
            .strip_prefix(&format!(
                "{} automated weather observation. ",
                phraseology::facility(&station)
            ))
            .unwrap_or(&wx);
        let remarks = vec!["Read back all runway hold-short instructions".into()];
        let text = format!(
            "{} information {info}. {wx} Landing and departing runway {}. Remarks: {}. Advise on initial contact you have information {info}.",
            phraseology::facility(&station).trim_end_matches(" ATIS"),
            runway_words(&runway),
            remarks.join(". ")
        );
        Some(AtisReport {
            airport: airport.into(),
            station_id: station.id,
            channel_khz: station.channel_khz,
            information: info,
            runway,
            remarks,
            text,
        })
    }
    pub(super) fn refresh_atis(&mut self) {
        for st in self.stations().iter().filter(|st| st.kind == FacilityKind::Atis) {
            let signature = format!(
                "{:?}|{:?}|{:?}",
                self.sim.conditions(),
                self.sim.weather(),
                self.active_runway(&st.airport)
            );
            let entry = self
                .comms
                .atc
                .info_epochs
                .entry(st.airport.clone())
                .or_insert((signature.clone(), 0));
            if entry.0 != signature {
                entry.0 = signature;
                entry.1 = entry.1.wrapping_add(1);
                self.comms.next_weather = self.sim.state().time_s;
            }
        }
    }
    pub(crate) fn sync_radio_failure(&mut self) {
        let failed = self.sim.failures().radio_failure;
        if self.comms.atc.radio_failure_observed == failed {
            return;
        }
        self.comms.atc.radio_failure_observed = failed;
        self.comms.atc.light_override = false;
        if failed {
            if let Some(a) = self.comms.atc.airport.clone() {
                let _ = self.set_light_gun(&a, LightGunSignal::FlashingGreen);
            }
        } else {
            self.comms.atc.light_gun = None;
        }
        self.atc_grade(
            None,
            "radio_failure",
            !failed,
            if failed {
                "COM transmit and reception failed"
            } else {
                "COM restored"
            }
            .into(),
        );
    }
    fn set_light_gun(&mut self, airport: &str, signal: LightGunSignal) -> Result<(), CommandError> {
        let st = self
            .stations()
            .into_iter()
            .find(|st| st.airport == airport && st.kind == FacilityKind::Tower)
            .ok_or_else(|| invalid("atc.light_gun", "no imported tower"))?;
        let position_m = self
            .station_origin(&st)
            .ok_or_else(|| invalid("atc.light_gun", "cannot project tower reference"))?;
        let state = LightGunState {
            airport: airport.into(),
            signal,
            position_m,
            position_estimated: true,
            since_s: self.sim.state().time_s,
            indication: signal.indication(self.sim.readout().on_ground).into(),
        };
        let text = format!("Tower light gun: {}.", state.indication);
        self.start_local(
            Channel::Intercom,
            Speaker::Instructor,
            text,
            Meaning::LightGun {
                signal: state.clone(),
            },
            EventSource::Rules,
        );
        self.comms.atc.light_gun = Some(state);
        Ok(())
    }
    pub(super) fn station_origin(&self, st: &ComStation) -> Option<[f64; 3]> {
        // Imported provider-airport coordinates do not locate remote approach/departure antennas.
        // For explicitly estimated positions, use the served field as a coverage proxy.
        if matches!(st.kind, FacilityKind::Approach | FacilityKind::Departure)
            && st.metadata.as_ref().is_some_and(|m| m.position_estimated)
            && let Ok(a) = crate::regions::airport(self.sim.world(), &st.airport)
            && let Some([an, ae]) = self.radio_project(a.latitude_deg, a.longitude_deg)
            && let Some([sn, se]) = self.radio_project(st.position.latitude_deg, st.position.longitude_deg)
            && libm::hypot(an - sn, ae - se) > 10. * 1852.
        {
            return Some([an, ae, -(a.elevation_m + st.antenna_agl_m)]);
        }
        let [n, e] = self.radio_project(st.position.latitude_deg, st.position.longitude_deg)?;
        Some([n, e, -(st.elevation_m + st.antenna_agl_m)])
    }

    pub(super) fn atc_reply(&mut self, st: &ComStation, text: String, meaning: Meaning) {
        if self.comms.atc.replies.len() < 64 {
            self.comms.atc.replies.push_back(Reply {
                station: st.clone(),
                text: format!("{}, {text}", self.pilot_callsign().current),
                meaning,
            });
        }
    }
    pub(super) fn issue(&mut self, st: &ComStation, instruction: AtcInstruction) {
        let phrase = self.instruction_phrase(&instruction);
        if self.comms.atc.pending.len() >= 16 {
            return;
        }
        self.atc_reply(
            st,
            phrase.clone(),
            Meaning::Atc {
                instruction: instruction.clone(),
            },
        );
        self.comms.atc.pending.push(PendingInstruction {
            id: 0,
            station_id: st.id.clone(),
            airport: self
                .comms
                .atc
                .airport
                .clone()
                .unwrap_or_else(|| st.airport.clone()),
            instruction,
            phrase,
            heard: false,
            readback: false,
            issued_s: self.sim.state().time_s,
            reminder_s: f64::MAX,
        });
    }
    pub(super) fn sequence_atc(&mut self) {
        let now = self.sim.state().time_s;
        if now >= self.comms.atc.next_check {
            self.comms.atc.next_check = now + 1.;
            self.refresh_atis();
            self.monitor_atc();
            self.flush_atc_feedback();
        }
        if self.sim.failures().radio_failure {
            return;
        }
        if let Some(reply) = self.comms.atc.replies.front() {
            let carrier = carrier_thirds_hz(reply.station.channel_khz);
            if self
                .comms
                .pending
                .iter()
                .any(|p| p.carrier == carrier && p.end_s + 0.5 > now)
            {
                return;
            }
        }
        if let Some(reply) = self.comms.atc.replies.pop_front()
            && let Some(origin) = self.station_origin(&reply.station)
        {
            let meaning = reply.meaning.clone();
            let id = self.start_com(
                reply.station.channel_khz,
                Speaker::Facility {
                    id: reply.station.id.clone(),
                    role: reply.station.kind,
                },
                reply.text,
                reply.meaning,
                EventSource::Rules,
                origin,
                reply.station.range_nm,
            );
            if let Meaning::Atc { instruction } = meaning
                && let Some(p) = self
                    .comms
                    .atc
                    .pending
                    .iter_mut()
                    .find(|p| p.id == 0 && p.instruction == instruction)
            {
                p.id = id;
            }
        }
    }
    pub(super) fn completed_atc(&mut self, event: &Transmission, receiver: Option<ComStation>) {
        if event.channel != Channel::Com {
            return;
        }
        if let Speaker::Facility { id, .. } = &event.speaker {
            if event.reception == Reception::Heard {
                let now = self.sim.state().time_s;
                if matches!(
                    event.meaning,
                    Meaning::Atc {
                        instruction: AtcInstruction::GoAround { .. }
                    }
                ) {
                    self.comms.atc.landing = false;
                }
                if matches!(
                    event.meaning,
                    Meaning::Atc {
                        instruction: AtcInstruction::Bravo { cleared: false, .. }
                    }
                ) {
                    self.comms.atc.bravo = None;
                }
                if matches!(
                    event.meaning,
                    Meaning::Atc {
                        instruction: AtcInstruction::IfrClearance { .. }
                    }
                ) {
                    self.comms.atc.ifr_clearance = true;
                }
                if let Some(p) = self.comms.atc.pending.iter_mut().find(|p| p.id == event.id) {
                    p.heard = true;
                    p.reminder_s = now + 10.;
                }
                if let Some(st) = self.stations().into_iter().find(|st| st.id == *id)
                    && !matches!(
                        st.kind,
                        FacilityKind::Atis | FacilityKind::Asos | FacilityKind::Awos
                    )
                {
                    if matches!(event.meaning, Meaning::RadarContact { .. }) {
                        self.comms.atc.radar_facility = Some(radar_facility(&st));
                    }
                    if parse_pilot(&event.text, &self.pilot_callsign()).callsign_present {
                        self.comms.atc.two_way.insert(provider(&st));
                    }
                    if let Meaning::Atc {
                        instruction: AtcInstruction::Bravo { cleared: true, .. },
                    } = &event.meaning
                        && new_york_controller(&st)
                    {
                        self.comms.atc.bravo = Some("NEW YORK CLASS B".into());
                    }
                }
            }
        } else if event.speaker == Speaker::Pilot
            && event.reception == Reception::Heard
            && !self.sim.failures().radio_failure
            && let Some(st) = receiver
        {
            self.handle_pilot(event, st);
        }
    }
    pub(super) fn atc_receiver(&self, message: &PilotMessage, khz: u32) -> Option<ComStation> {
        self.comms
            .atc
            .pending
            .iter()
            .find_map(|p| {
                self.stations().into_iter().find(|st| {
                    st.id == p.station_id
                        && carrier_thirds_hz(st.channel_khz) == carrier_thirds_hz(khz)
                        && self.station_origin(st).is_some_and(|o| {
                            reception(self.sim.world(), self.sim.state().position_m, o, st.range_nm, 0.)
                        })
                })
            })
            .or_else(|| self.tuned_facility(message, khz))
    }
    pub(super) fn tuned_facility(&self, message: &PilotMessage, khz: u32) -> Option<ComStation> {
        let [pn, pe, _] = self.sim.state().position_m;
        self.stations()
            .into_iter()
            .filter(|st| {
                is_controller(st.kind) && carrier_thirds_hz(st.channel_khz) == carrier_thirds_hz(khz)
            })
            .filter(|st| {
                self.station_origin(st).is_some_and(|o| {
                    reception(self.sim.world(), o, self.sim.state().position_m, st.range_nm, 0.)
                })
            })
            .min_by(|a, b| {
                let score = |st: &ComStation| {
                    self.station_origin(st)
                        .map_or(f64::MAX, |[n, e, _]| libm::hypot(n - pn, e - pe))
                        + if self.comms.atc.airport.as_ref() == Some(&st.airport) {
                            0.
                        } else {
                            1000.
                        }
                        + if message.facility.is_some_and(|f| f != st.kind) {
                            1_000_000.
                        } else {
                            0.
                        }
                };
                score(a).total_cmp(&score(b)).then_with(|| a.id.cmp(&b.id))
            })
    }
}
fn is_controller(f: FacilityKind) -> bool {
    matches!(
        f,
        FacilityKind::Ground
            | FacilityKind::Tower
            | FacilityKind::Clearance
            | FacilityKind::Approach
            | FacilityKind::Departure
    )
}
pub(super) fn provider(st: &ComStation) -> String {
    st.metadata
        .as_ref()
        .map_or_else(|| st.airport.clone(), |m| m.provider.clone())
}

#[cfg(test)]
mod tests;

fn new_york_controller(st: &ComStation) -> bool {
    let call = st
        .metadata
        .as_ref()
        .map_or(st.name.as_str(), |m| m.callsign.as_str())
        .to_uppercase();
    matches!(st.kind, FacilityKind::Approach | FacilityKind::Departure)
        && (call.contains("NEWARK") || call.contains("NEW YORK"))
}

// NASR radio calls identify a radar facility across its sector frequencies and roles.
fn radar_facility(station: &ComStation) -> String {
    let name = station
        .metadata
        .as_ref()
        .map(|m| m.callsign.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&station.name);
    format!("{}:{}", provider(station), phraseology::normal_case(name))
}
