use super::*;
const NM: f64 = 1852.0;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PatternLeg {
    Taxi,
    Departing,
    Crosswind,
    Downwind,
    Base,
    Final,
    Clear,
}
impl PatternLeg {
    pub fn phrase(self) -> &'static str {
        match self {
            Self::Taxi => "taxiing",
            Self::Departing => "departing",
            Self::Crosswind => "crosswind",
            Self::Downwind => "downwind",
            Self::Base => "base",
            Self::Final => "final",
            Self::Clear => "clear of runway",
        }
    }
    pub fn parse(text: &str) -> Option<Self> {
        let t = format!(" {} ", words(text));
        [
            ("clear", Self::Clear),
            ("crosswind", Self::Crosswind),
            ("downwind", Self::Downwind),
            ("base", Self::Base),
            ("final", Self::Final),
            ("depart", Self::Departing),
            ("departing", Self::Departing),
            ("departure", Self::Departing),
            ("taking off", Self::Departing),
            ("takeoff", Self::Departing),
            ("taxi", Self::Taxi),
            ("taxiing", Self::Taxi),
        ]
        .iter()
        .find_map(|(word, leg)| t.contains(&format!(" {word} ")).then_some(*leg))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TrafficAircraft {
    pub address: u32,
    pub callsign: String,
    pub registration: String,
    pub aircraft_type: String,
    pub position_m: [f64; 3],
    pub latitude_deg: Option<f64>,
    pub longitude_deg: Option<f64>,
    pub altitude_msl_m: f64,
    pub heading_grid_rad: f64,
    pub speed_mps: f64,
    pub vertical_speed_mps: f64,
    pub leg: PatternLeg,
    pub sequence: u8,
    pub following_pilot: bool,
}
#[derive(Clone, Debug)]
pub(super) struct Pattern {
    pub airport: String,
    pub name: String,
    pub runway: String,
    pub frequency_khz: u32,
    pub origin: [f64; 2],
    pub forward: [f64; 2],
    pub right_pattern: bool,
    pub elevation_m: f64,
    pub length_m: f64,
    pub progress: Vec<f64>,
    pub aircraft: Vec<TrafficAircraft>,
    pub expected: Option<PatternLeg>,
    pub phase_since: f64,
    pub phase_called: bool,
    pub missing_reported: bool,
}
impl Pattern {
    fn points(&self) -> [[f64; 2]; 6] {
        [
            [0.0, 0.0],
            [self.length_m + 1.5 * NM, 0.0],
            [self.length_m + 1.5 * NM, NM],
            [-1.5 * NM, NM],
            [-1.5 * NM, 0.0],
            [0.0, 0.0],
        ]
    }
    fn perimeter(&self) -> f64 {
        2.0 * (self.length_m + 3.0 * NM) + 2.0 * NM
    }
    fn location(&self, progress: f64) -> ([f64; 2], f64, PatternLeg) {
        let mut distance = progress.rem_euclid(self.perimeter());
        let legs = [
            PatternLeg::Departing,
            PatternLeg::Crosswind,
            PatternLeg::Downwind,
            PatternLeg::Base,
            PatternLeg::Final,
        ];
        for (pair, leg) in self.points().windows(2).zip(legs) {
            let [a, b] = pair else { continue };
            let length = libm::hypot(b[0] - a[0], b[1] - a[1]);
            if distance <= length {
                let fraction = distance / length;
                let xy = [a[0] + (b[0] - a[0]) * fraction, a[1] + (b[1] - a[1]) * fraction];
                let direction = libm::atan2(b[1] - a[1], b[0] - a[0]);
                return (xy, direction, leg);
            }
            distance -= length;
        }
        ([0.0, 0.0], 0.0, PatternLeg::Departing)
    }
    fn grid(&self, [along, side]: [f64; 2]) -> [f64; 2] {
        let [fnorth, feast] = self.forward;
        let sign = if self.right_pattern { 1.0 } else { -1.0 };
        [
            self.origin[0] + along * fnorth - side * feast * sign,
            self.origin[1] + along * feast + side * fnorth * sign,
        ]
    }
    pub fn pilot_leg(&self, s: &Session) -> Option<PatternLeg> {
        let [n, e, _] = s.sim.state().position_m;
        let dn = n - self.origin[0];
        let de = e - self.origin[1];
        let [f, g] = self.forward;
        let along = dn * f + de * g;
        let side = (-dn * g + de * f) * if self.right_pattern { 1.0 } else { -1.0 };
        let r = s.sim.readout();
        if libm::hypot(dn, de) > 8.0 * NM {
            return None;
        }
        if r.on_ground {
            return Some(
                if side.abs() > 65.0
                    && self
                        .expected
                        .is_some_and(|leg| matches!(leg, PatternLeg::Final | PatternLeg::Clear))
                {
                    PatternLeg::Clear
                } else if r.ground_speed_mps > 20.0 {
                    PatternLeg::Departing
                } else {
                    PatternLeg::Taxi
                },
            );
        }
        let heading = r.heading_grid_rad - libm::atan2(g, f);
        let h_along = libm::cos(heading);
        let h_side = libm::sin(heading) * if self.right_pattern { 1.0 } else { -1.0 };
        if h_along < -0.55 && side > 300.0 {
            Some(PatternLeg::Downwind)
        } else if h_side > 0.55 && along > self.length_m {
            Some(PatternLeg::Crosswind)
        } else if h_side < -0.55 && along < 0.0 {
            Some(PatternLeg::Base)
        } else if h_along > 0.55 && along < 0.0 && side.abs() < 600.0 {
            Some(PatternLeg::Final)
        } else if h_along > 0.55 && along >= 0.0 && side.abs() < 600.0 {
            Some(PatternLeg::Departing)
        } else {
            None
        }
    }
}
#[derive(Clone, Debug, Serialize, JsonSchema, serde::Deserialize)]
pub struct TrafficState {
    pub airport: Option<String>,
    pub runway: Option<String>,
    pub aircraft: Vec<TrafficAircraft>,
}
#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetTraffic {
    #[schemars(range(min = 0, max = 3))]
    pub count: u8,
    pub airport: String,
    #[serde(default)]
    pub runway: Option<String>,
    /// Conventional left traffic by default; select right only from published local information.
    #[serde(default)]
    pub right_pattern: bool,
}
impl Command for SetTraffic {
    const NAME: &'static str = "traffic.set";
    const DESCRIPTION: &'static str =
        "Set zero to three deterministic circuit aircraft at an FAA airport with a published CTAF.";
    type Output = TrafficState;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        if self.count > 3 {
            return Err(invalid(Self::NAME, "count is 0..3"));
        }
        if self.count == 0 {
            s.comms.pattern = None;
            s.comms.traffic_calls.clear();
            return Ok(s.traffic_state());
        }
        let airport = crate::regions::airport(s.sim.world(), &self.airport)?.clone();
        let station = s
            .stations()
            .into_iter()
            .find(|st| {
                (st.airport == airport.ident || airport.icao.as_deref() == Some(&st.airport))
                    && matches!(st.kind, FacilityKind::Ctaf | FacilityKind::Tower)
            })
            .ok_or_else(|| {
                invalid(
                    Self::NAME,
                    "airport has no imported FAA CTAF; import frequency data first",
                )
            })?;
        let (runway, end) = airport
            .runways
            .iter()
            .find_map(|r| {
                r.ends
                    .iter()
                    .find(|e| self.runway.as_ref().is_none_or(|id| &e.ident == id))
                    .map(|e| (r, e))
            })
            .ok_or_else(|| invalid(Self::NAME, "runway end does not exist"))?;
        let origin = s
            .radio_project(end.latitude_deg, end.longitude_deg)
            .ok_or_else(|| invalid(Self::NAME, "traffic requires a geographic world"))?;
        let other = runway
            .ends
            .iter()
            .find(|e| e.ident != end.ident)
            .ok_or_else(|| invalid(Self::NAME, "missing opposite runway end"))?;
        let far = s
            .radio_project(other.latitude_deg, other.longitude_deg)
            .ok_or_else(|| invalid(Self::NAME, "cannot project runway"))?;
        let heading = libm::atan2(far[1] - origin[1], far[0] - origin[0]);
        let mut pattern = Pattern {
            airport: station.airport,
            name: airport.name,
            runway: end.ident.clone(),
            frequency_khz: station.channel_khz,
            origin,
            forward: [libm::cos(heading), libm::sin(heading)],
            right_pattern: self.right_pattern,
            elevation_m: end.elevation_m,
            length_m: runway.length_m,
            progress: Vec::new(),
            aircraft: Vec::new(),
            expected: None,
            phase_since: s.sim.state().time_s,
            phase_called: false,
            missing_reported: false,
        };
        for i in 0..self.count {
            pattern
                .progress
                .push(pattern.perimeter() * f64::from(i + 1) / f64::from(self.count + 1));
        }
        s.comms.traffic_calls.clear();
        s.comms.pattern = Some(pattern);
        s.update_pattern(0.0);
        s.sequence_traffic_calls();
        Ok(s.traffic_state())
    }
}
impl Session {
    pub fn traffic_state(&self) -> TrafficState {
        TrafficState {
            airport: self.comms.pattern.as_ref().map(|p| p.airport.clone()),
            runway: self.comms.pattern.as_ref().map(|p| p.runway.clone()),
            aircraft: self
                .comms
                .pattern
                .as_ref()
                .map(|p| p.aircraft.clone())
                .unwrap_or_default(),
        }
    }
    pub(super) fn update_pattern(&mut self, dt: f64) {
        let Some(mut pattern) = self.comms.pattern.take() else {
            return;
        };
        let controlled = self
            .stations()
            .iter()
            .any(|st| st.airport == pattern.airport && st.kind == FacilityKind::Tower);
        let time = self.sim.state().time_s;
        let pilot_leg = pattern.pilot_leg(self);
        let [pn, pe, pd] = self.sim.state().position_m;
        let mut calls = Vec::new();
        let previous = pattern.aircraft.clone();
        pattern.aircraft.clear();
        // Every aircraft reacts to the same previous snapshot. No iteration-order priority.
        let mut next_progress = pattern.progress.clone();
        for (i, progress) in pattern.progress.iter().copied().enumerate() {
            let (xy, direction, old_leg) = pattern.location(progress);
            let [n, e] = pattern.grid(xy);
            let grid_heading = libm::atan2(pattern.forward[1], pattern.forward[0])
                + direction * if pattern.right_pattern { 1.0 } else { -1.0 };
            let ahead = (pn - n) * libm::cos(grid_heading) + (pe - e) * libm::sin(grid_heading);
            let lateral = (-(pn - n) * libm::sin(grid_heading) + (pe - e) * libm::cos(grid_heading)).abs();
            let following_pilot = pilot_leg == Some(old_leg)
                && ahead > 0.0
                && ahead < 1.6 * NM
                && lateral < 300.0
                && (-pd - (pattern.elevation_m + 304.8)).abs() < 200.0;
            let following_traffic = pattern
                .progress
                .iter()
                .enumerate()
                .any(|(j, p)| j != i && (*p - progress).rem_euclid(pattern.perimeter()) < 1.1 * NM);
            let speed = if following_traffic {
                25.0
            } else if following_pilot {
                self.sim.readout().ground_speed_mps.clamp(25.0, 40.0)
            } else {
                45.0
            };
            let next = (progress + dt * speed).rem_euclid(pattern.perimeter());
            if let Some(value) = next_progress.get_mut(i) {
                *value = next;
            }
            let (xy, direction, leg) = pattern.location(next);
            let [n, e] = pattern.grid(xy);
            let altitude = pattern.elevation_m
                + if leg == PatternLeg::Final {
                    (-xy[0] / (1.5 * NM) * 304.8).clamp(15.0, 304.8)
                } else if leg == PatternLeg::Departing {
                    (xy[0] / (pattern.length_m + 1.5 * NM) * 304.8).clamp(15.0, 304.8)
                } else {
                    304.8
                };
            let geo = self.sim.world().geodetic(n, e);
            let (kind, registration) = [
                ("Cherokee", "N7381F"),
                ("Cessna", "N236EL"),
                ("Skyhawk", "N9852R"),
            ]
            .get(i)
            .copied()
            .unwrap_or(("Cessna", "N236EL"));
            let suffix = registration.strip_prefix('N').unwrap_or(registration);
            let short = suffix
                .chars()
                .rev()
                .take(3)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<String>();
            let full_callsign = format!("{kind} {}", spoken_registration(suffix));
            let callsign = format!(
                "{kind} {}",
                spoken_registration(if previous.get(i).is_none() { suffix } else { &short })
            );
            let old = previous.get(i);
            let vertical_speed_mps = old
                .filter(|_| dt > 0.0)
                .map_or(0.0, |a| (altitude - a.altitude_msl_m) / dt);
            pattern.aircraft.push(TrafficAircraft {
                address: 0xf10001 + i as u32,
                callsign: callsign.clone(),
                registration: registration.into(),
                aircraft_type: kind.into(),
                position_m: [n, e, -altitude],
                latitude_deg: geo.map(|g| g[0]),
                longitude_deg: geo.map(|g| g[1]),
                altitude_msl_m: altitude,
                heading_grid_rad: (libm::atan2(pattern.forward[1], pattern.forward[0])
                    + direction * if pattern.right_pattern { 1.0 } else { -1.0 })
                .rem_euclid(core::f64::consts::TAU),
                speed_mps: speed,
                vertical_speed_mps,
                leg,
                sequence: i as u8 + 1,
                following_pilot,
            });
            if old.is_none_or(|a| a.leg != leg || a.following_pilot != following_pilot) {
                let text = format!(
                    "{}, {}, {}{} runway {}{}.",
                    if controlled {
                        self.stations()
                            .iter()
                            .find(|s| s.airport == pattern.airport && s.kind == FacilityKind::Tower)
                            .map_or_else(
                                || format!("{} Tower", phraseology::normal_case(&pattern.name)),
                                phraseology::facility,
                            )
                    } else {
                        format!("{} traffic", pattern.name)
                    },
                    callsign,
                    leg.phrase(),
                    if following_pilot { ", following you," } else { "" },
                    phraseology::runway(&pattern.runway),
                    if controlled {
                        String::new()
                    } else {
                        format!(", {}", phraseology::normal_case(&pattern.name))
                    }
                );
                calls.push((
                    Speaker::Traffic {
                        callsign: full_callsign,
                    },
                    text,
                    Meaning::Position {
                        airport: pattern.airport.clone(),
                        runway: pattern.runway.clone(),
                        leg,
                    },
                    [n, e, -altitude],
                ));
            }
        }
        pattern.progress = next_progress;
        let call_in_progress = self.pending_phase_call(pattern.expected);
        if controlled {
            pattern.phase_called = true;
        }
        if pilot_leg != pattern.expected {
            if let Some(leg) = pattern.expected
                && !pattern.phase_called
                && !call_in_progress
                && time - pattern.phase_since > 8.0
            {
                self.missing_call(&pattern, leg);
            }
            pattern.expected = pilot_leg;
            pattern.phase_since = time;
            pattern.phase_called = controlled;
            pattern.missing_reported = false;
        }
        if let Some(leg) = pilot_leg
            && !pattern.phase_called
            && !self.pending_phase_call(Some(leg))
            && !pattern.missing_reported
            && time - pattern.phase_since >= 20.0
        {
            self.missing_call(&pattern, leg);
            pattern.missing_reported = true;
        }
        let frequency = pattern.frequency_khz;
        self.comms.pattern = Some(pattern);
        for (speaker, text, meaning, origin) in calls {
            // Traffic listens before speaking. A new position replaces a stale queued call.
            self.comms.traffic_calls.retain(|c| c.speaker != speaker);
            if self.comms.traffic_calls.len() < 8 {
                self.comms.traffic_calls.push_back(TrafficCall {
                    khz: frequency,
                    speaker,
                    text,
                    meaning,
                    origin,
                });
            }
        }
    }
    fn pending_phase_call(&self, leg: Option<PatternLeg>) -> bool {
        leg.is_some()
            && self.comms.pending.iter().any(|pending| {
                self.comms
                    .record
                    .events
                    .iter()
                    .find(|event| event.id == pending.id)
                    .is_some_and(|event| {
                        event.speaker == Speaker::Pilot
                            && event.grade.as_ref().is_some_and(|grade| {
                                grade.expected_leg == leg
                                    && grade.airport_correct
                                    && grade.runway_correct
                                    && grade.frequency_correct
                                    && grade.timely
                            })
                    })
            })
    }
    fn missing_call(&mut self, pattern: &Pattern, leg: PatternLeg) {
        let text = format!(
            "No {} CTAF call recorded for {} runway {}.",
            leg.phrase(),
            pattern.name,
            pattern.runway
        );
        self.start_local(
            Channel::Intercom,
            Speaker::Instructor,
            text.clone(),
            Meaning::Cue {
                airport: pattern.airport.clone(),
                leg,
                message: text,
            },
            EventSource::Rules,
        );
    }
}
