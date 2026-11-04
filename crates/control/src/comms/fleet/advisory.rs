//! Traffic advisories to the user (FAA JO 7110.65 2-1-21, AIM 4-1-15) and radio-discipline grading.
//!
//! The controller of the facility the user is working watches the busy-airspace traffic. When an
//! aircraft is within about 5 NM and 1,000 ft, or converging, it calls it: clock position from the
//! user's ground track, whole miles, direction of flight, type, altitude. The numbers are computed
//! from the same `FleetAircraft` values `traffic.state` reports at the instant the call starts, so a
//! display can highlight `target_aircraft` and show the same clock and distance the pilot heard.
//! The call takes the carrier through the fleet's own frequency queue, so it never overlaps traffic.
//!
//! The pilot answers `looking`, `traffic in sight`, or `negative contact`; the controller follows up
//! (`maintain visual separation`, `follow that traffic`, an update, or `traffic no factor`). A call
//! with no answer is repeated once, then graded. Answering a call made to another aircraft, or
//! keying over a transmission, is graded too. Faults are `ComplianceGrade` entries with items
//! `radio_discipline:*`; answered advisories are `traffic_advisory`.
use super::control::{Ac, Fleet};
use super::*;
use std::collections::{BTreeMap, VecDeque};

const NM: f64 = 1852.0;
/// Seconds the pilot has after a call ends before the controller repeats it.
pub(crate) const RESPONSE_S: f64 = 12.0;
/// Minimum seconds between two advisory calls from one controller.
const MIN_CALL_GAP_S: f64 = 20.0;
/// Seconds after an advisory before the controller updates it for a pilot still looking.
const UPDATE_AFTER_S: f64 = 20.0;
const MAX_UPDATES: u8 = 2;
const HEARD_KEEP: usize = 12;
/// A pilot reading back or acknowledging within this long of a call to someone else is answering it.
const OTHER_CALL_WINDOW_S: f64 = 25.0;
const ACK_WINDOW_S: f64 = 10.0;
/// Seconds between two `stepped on` faults.
const STEP_FAULT_GAP_S: f64 = 8.0;

/// Radio-discipline tally for the debrief: calls to the user that were answered, and the faults.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RadioDiscipline {
    /// Initial traffic advisories spoken to the user.
    pub advisories_issued: u32,
    /// Advisories the user answered (looking, in sight, negative contact, or an acknowledgment).
    pub advisories_answered: u32,
    /// Unanswered advisories the controller repeated.
    pub repeats: u32,
    /// Answered or acknowledged a call that used another aircraft's callsign.
    pub wrong_callsign: u32,
    /// Read back or acknowledged an instruction given to another aircraft.
    pub acknowledged_other: u32,
    /// Keyed the microphone while a traffic or controller transmission was on the frequency.
    pub stepped_on: u32,
    /// Advisories still unanswered after the repeat.
    pub no_response: u32,
}
impl RadioDiscipline {
    pub fn faults(&self) -> u32 {
        self.wrong_callsign + self.acknowledged_other + self.stepped_on + self.no_response
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Spoken, or answered with `looking`; the controller may update it.
    Looking,
    /// Sighted, no factor, or unanswered: never called again.
    Closed,
}
#[derive(Clone, Debug)]
struct Seen {
    last_s: f64,
    updates: u8,
    clock: u8,
    miles: u32,
    phase: Phase,
}
#[derive(Clone, Debug)]
struct Open {
    aircraft: u32,
    event_id: u64,
    station: ComStation,
    issued_s: f64,
    spoken_end_s: Option<f64>,
    repeats: u8,
    first: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FollowKind {
    /// A fresh advisory for the same aircraft (`Traffic now ...`).
    Update,
    NoFactor,
    VisualSeparation,
    Follow,
    Roger,
}
#[derive(Clone, Debug)]
struct Follow {
    kind: FollowKind,
    aircraft: u32,
    station: ComStation,
    due_s: f64,
}
/// A transmission to another aircraft that the user heard.
#[derive(Clone, Debug)]
struct HeardCall {
    end_s: f64,
    aircraft: u32,
    instructions: Vec<AtcInstruction>,
    controller: bool,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct AdvisoryBook {
    seen: BTreeMap<u32, Seen>,
    open: Option<Open>,
    follow: Option<Follow>,
    last_call_s: Option<f64>,
    next_scan_s: f64,
    heard: VecDeque<HeardCall>,
    last_stepped_s: Option<f64>,
    pub(crate) tally: RadioDiscipline,
}

/// The user's motion, from which the clock position and closure are measured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct UserMotion {
    /// Ground track, true degrees (JO 7110.65 2-1-21 measures from the track, not the heading).
    pub track_true_deg: f64,
    pub speed_kt: f64,
    /// False when too slow for a track; the heading stands in and directions are given as compass words.
    pub has_track: bool,
    pub vs_fpm: f64,
}
/// One traffic aircraft as the controller sees it from the user's position.
#[derive(Clone, Debug)]
pub(crate) struct Assessment {
    pub id: u32,
    pub callsign: String,
    pub spoken_type: String,
    pub clock: u8,
    pub range_nm: f64,
    pub miles: u32,
    pub relative_ft: f64,
    pub altitude_ft: u32,
    pub direction: String,
    pub altitude: phraseology::AdvisoryAltitude,
    pub cpa_nm: f64,
    pub factor: bool,
    pub score: f64,
}

/// Clock position 1-12 of a true bearing, relative to a true ground track.
pub(crate) fn clock_of(bearing_true_deg: f64, track_true_deg: f64) -> u8 {
    let relative = (bearing_true_deg - track_true_deg).rem_euclid(360.);
    match (relative / 30.).round() as u32 % 12 {
        0 => 12,
        n => n as u8,
    }
}

/// The geometry of one aircraft against the user. Pure: the same `FleetAircraft` values that
/// `traffic.state` reports, and the user's ground track.
pub(crate) fn assess(a: &FleetAircraft, user: &UserMotion) -> Assessment {
    let bearing = a.bearing_true_deg.to_radians();
    let p = [a.range_nm * libm::cos(bearing), a.range_nm * libm::sin(bearing)];
    let velocity = |kt: f64, track_deg: f64| {
        let t = track_deg.to_radians();
        [kt / 3600. * libm::cos(t), kt / 3600. * libm::sin(t)]
    };
    let (vt, vu) = (
        velocity(a.ground_speed_kt, a.track_true_deg),
        velocity(user.speed_kt, user.track_true_deg),
    );
    let v = [vt[0] - vu[0], vt[1] - vu[1]];
    let v2 = v[0] * v[0] + v[1] * v[1];
    let tcpa = if v2 > 1e-12 {
        (-(p[0] * v[0] + p[1] * v[1]) / v2).max(0.)
    } else {
        0.
    };
    let closing = tcpa > 0.;
    let cpa = libm::hypot(p[0] + v[0] * tcpa, p[1] + v[1] * tcpa);
    let dz = a.relative_altitude_ft;
    let dz_cpa = dz + (a.vertical_rate_fpm - user.vs_fpm) / 60. * tcpa.min(120.);
    let near = a.range_nm <= 5. && dz.abs() <= 1000. && (closing || a.range_nm <= 3.);
    let converging = closing && tcpa <= 120. && cpa <= 3. && a.range_nm <= 10. && dz_cpa.abs() <= 1000.;
    let altitude_ft = a.altitude_msl_ft.max(0.).round() as u32;
    Assessment {
        id: a.id,
        callsign: a.callsign.clone(),
        spoken_type: a.aircraft_type.clone(),
        clock: clock_of(a.bearing_true_deg, user.track_true_deg),
        range_nm: a.range_nm,
        miles: a.range_nm.round().max(1.) as u32,
        relative_ft: dz,
        altitude_ft,
        direction: phraseology::traffic_direction(
            a.track_true_deg - user.track_true_deg,
            a.track_magnetic_deg,
            user.has_track,
        ),
        // Circuit traffic is not under radar: its altitude is an unverified readout.
        altitude: if a.intent.kind == FleetFlight::Pattern {
            phraseology::AdvisoryAltitude::Indicates(altitude_ft)
        } else {
            phraseology::AdvisoryAltitude::Known(altitude_ft)
        },
        cpa_nm: cpa,
        factor: near || converging,
        score: cpa * 2. + a.range_nm,
    }
}

impl Session {
    pub(crate) fn user_motion(&self) -> UserMotion {
        let [vn, ve, vd] = self.sim.state().velocity_mps;
        let ground = libm::hypot(vn, ve);
        let has_track = ground > 10.;
        let grid = if has_track {
            libm::atan2(ve, vn)
        } else {
            self.sim.readout().heading_grid_rad
        };
        UserMotion {
            track_true_deg: (grid + self.sim.heading_reference().grid_north_true_rad)
                .to_degrees()
                .rem_euclid(360.),
            speed_kt: ground / 0.514_444,
            has_track,
            vs_fpm: -vd / 0.3048 * 60.,
        }
    }

    pub fn radio_discipline(&self) -> RadioDiscipline {
        self.comms.advisories.tally.clone()
    }

    fn controller_address(&self) -> String {
        if self.comms.abbreviate {
            self.pilot_callsign().abbreviated
        } else {
            self.pilot_callsign().full
        }
    }

    fn carrier_in_use(&self, khz: u32) -> bool {
        let now = self.sim.state().time_s;
        let carrier = carrier_thirds_hz(khz);
        self.comms
            .pending
            .iter()
            .any(|p| p.carrier == carrier && p.end_s + 0.5 > now)
            || self
                .comms
                .atc
                .replies
                .iter()
                .any(|r| carrier_thirds_hz(r.station.channel_khz) == carrier)
            || self
                .comms
                .traffic_calls
                .iter()
                .any(|c| carrier_thirds_hz(c.khz) == carrier)
    }

    fn user_transmitting(&self) -> bool {
        let now = self.sim.state().time_s;
        self.comms
            .pending
            .iter()
            .any(|p| p.channel == Channel::Com && p.radio.is_some() && p.end_s > now)
    }

    /// The facility whose advisories the user is entitled to: the controller on the selected transmit
    /// frequency, if it is one of the busy-airspace facilities, the user can hear it, and the service
    /// rules allow it. Radar approach and departure advise aircraft in radar contact (IFR or VFR
    /// flight following); the tower advises aircraft it is working in the circuit.
    fn advisory_station(&self, fleet: &Fleet) -> Option<ComStation> {
        let r = self.sim.readout();
        if r.on_ground || r.height_agl_m < 50. {
            return None;
        }
        let transmit = self
            .comms
            .radios
            .get(usize::from(self.comms.panel.transmit.saturating_sub(1)))?;
        let carrier = carrier_thirds_hz(transmit.active_khz)?;
        let st = fleet.field.stations.iter().find(|st| {
            carrier_thirds_hz(st.channel_khz) == Some(carrier)
                && matches!(
                    st.kind,
                    FacilityKind::Tower | FacilityKind::Approach | FacilityKind::Departure
                )
        })?;
        let origin = self.station_origin(st)?;
        if !self.listening(carrier, origin, st.range_nm) {
            return None;
        }
        let [pn, pe, _] = self.sim.state().position_m;
        let allowed = match st.kind {
            FacilityKind::Tower => {
                self.comms.atc.current.as_ref().is_some_and(|c| c.id == st.id)
                    && libm::hypot(fleet.field.center[0] - pn, fleet.field.center[1] - pe) <= 6. * NM
            }
            _ => {
                self.comms.atc.radar_facility.as_deref()
                    == Some(super::super::atc::radar_facility(st).as_str())
            }
        };
        allowed.then(|| st.clone())
    }

    /// Callsigns of traffic the user has already been sequenced to follow: no advisory beyond that call.
    fn followed_callsigns(&self) -> Vec<String> {
        self.comms
            .atc
            .pending
            .iter()
            .chain(self.comms.atc.active.iter())
            .filter_map(|p| match &p.instruction {
                AtcInstruction::Pattern { follow, .. } => follow.clone(),
                _ => None,
            })
            .collect()
    }

    /// The facility's voice, from the same palette the fleet uses for its controller calls.
    fn advisory_voice(&self, st: &ComStation) -> u8 {
        self.comms
            .fleet
            .as_ref()
            .and_then(|f| {
                let i = f
                    .field
                    .stations
                    .iter()
                    .position(|s| s.id == st.id && s.channel_khz == st.channel_khz)?;
                Some(facility_voice(&f.field.stations, i))
            })
            .unwrap_or(0)
    }

    /// Speak a call to the user through the fleet frequency queue. `None` when the carrier is busy,
    /// the user cannot hear the facility, or the facility has no position.
    fn deliver_user_call(&mut self, st: &ComStation, text: String, meaning: Meaning) -> Option<u64> {
        let origin = self.station_origin(st)?;
        let carrier = carrier_thirds_hz(st.channel_khz)?;
        if !self.listening(carrier, origin, st.range_nm) {
            return None;
        }
        let now = self.sim.state().time_s;
        let in_use = self.carrier_in_use(st.channel_khz);
        let voice = self.advisory_voice(st);
        let claimed = self
            .comms
            .fleet
            .as_mut()?
            .claim_channel(st.channel_khz, now, in_use, &text, voice);
        if !claimed {
            return None;
        }
        Some(self.start_com(
            st.channel_khz,
            Speaker::Facility {
                id: st.id.clone(),
                role: st.kind,
            },
            text,
            meaning,
            EventSource::Rules,
            origin,
            st.range_nm,
        ))
    }

    fn channel_is_open(&self, st: &ComStation) -> bool {
        let now = self.sim.state().time_s;
        self.comms
            .fleet
            .as_ref()
            .is_some_and(|f| f.channel_open(st.channel_khz, now, self.carrier_in_use(st.channel_khz)))
    }

    fn advisory_meaning(
        &self,
        st: &ComStation,
        a: &Assessment,
        user: &UserMotion,
        updated: bool,
        repeat: bool,
    ) -> Meaning {
        Meaning::TrafficAdvisory {
            callsign: a.callsign.clone(),
            clock: a.clock,
            miles: a.range_nm,
            altitude_ft: a.altitude_ft,
            target_aircraft: vec![a.id],
            miles_spoken: a.miles,
            relative_altitude_ft: a.relative_ft.round() as i32,
            direction: a.direction.clone(),
            user_track_true_deg: user.track_true_deg,
            updated,
            repeat,
            voice: self.advisory_voice(st),
        }
    }

    /// Issue an advisory (or update, or repeat) for one aircraft now. Geometry is from the present
    /// `traffic.state`; `None` when the carrier could not be taken.
    fn speak_advisory(
        &mut self,
        st: &ComStation,
        a: &Assessment,
        user: &UserMotion,
        updated: bool,
        repeat: bool,
    ) -> Option<u64> {
        let body = phraseology::traffic_advisory(
            a.clock,
            a.miles,
            &a.direction,
            Some(&a.spoken_type),
            a.altitude,
            updated,
        );
        let text = format!("{}, {body}", self.controller_address());
        let meaning = self.advisory_meaning(st, a, user, updated, repeat);
        self.deliver_user_call(st, text, meaning)
    }

    pub(crate) fn sequence_advisories(&mut self) {
        if self.comms.fleet.is_none() {
            if self.comms.advisories.open.is_some() || self.comms.advisories.follow.is_some() {
                self.comms.advisories = AdvisoryBook::default();
            }
            return;
        }
        let now = self.sim.state().time_s;
        self.advisory_deadline(now);
        if self.advisory_follow(now) {
            return;
        }
        self.advisory_scan(now);
    }

    /// The answer window of an open advisory: repeat once, then grade the silence.
    fn advisory_deadline(&mut self, now: f64) {
        let Some(open) = self.comms.advisories.open.clone() else {
            return;
        };
        let Some(end) = open.spoken_end_s else {
            // Never finished (blocked, or the session reset): do not hold the controller forever.
            if now > open.issued_s + 90. {
                self.comms.advisories.open = None;
            }
            return;
        };
        if now < end + RESPONSE_S || self.user_transmitting() {
            return;
        }
        if open.repeats == 0 {
            let user = self.user_motion();
            let fresh = self
                .fleet_state()
                .aircraft
                .iter()
                .find(|a| a.id == open.aircraft)
                .map(|a| assess(a, &user))
                .filter(|a| a.factor);
            if let Some(a) = fresh {
                if !self.channel_is_open(&open.station) {
                    return;
                }
                let changed = self
                    .comms
                    .advisories
                    .seen
                    .get(&a.id)
                    .is_none_or(|s| s.clock != a.clock || s.miles != a.miles);
                if let Some(id) = self.speak_advisory(&open.station, &a, &user, changed, true) {
                    self.comms.advisories.tally.repeats += 1;
                    self.comms.advisories.open = Some(Open {
                        event_id: id,
                        issued_s: now,
                        spoken_end_s: None,
                        repeats: 1,
                        ..open
                    });
                    if let Some(seen) = self.comms.advisories.seen.get_mut(&a.id) {
                        seen.clock = a.clock;
                        seen.miles = a.miles;
                    }
                }
                return;
            }
        }
        self.comms.advisories.open = None;
        self.comms.advisories.tally.no_response += 1;
        if let Some(seen) = self.comms.advisories.seen.get_mut(&open.aircraft) {
            seen.phase = Phase::Closed;
        }
        let who = phraseology::facility(&open.station);
        self.atc_grade(
            Some(open.event_id),
            "radio_discipline:no_response",
            false,
            format!("{who} called you with a traffic advisory and you did not answer. Reply with your callsign: looking, traffic in sight, or negative contact."),
        );
    }

    /// Send a queued controller follow-up. True while one is waiting, so no new advisory starts.
    fn advisory_follow(&mut self, now: f64) -> bool {
        let Some(follow) = self.comms.advisories.follow.clone() else {
            return false;
        };
        if now < follow.due_s {
            return true;
        }
        if now > follow.due_s + 45. {
            self.comms.advisories.follow = None;
            return false;
        }
        if !self.channel_is_open(&follow.station) {
            return true;
        }
        let user = self.user_motion();
        let st = follow.station.clone();
        let voice = self.advisory_voice(&st);
        let mut kind = follow.kind;
        if kind == FollowKind::Update {
            let fresh = self
                .fleet_state()
                .aircraft
                .iter()
                .find(|a| a.id == follow.aircraft)
                .map(|a| assess(a, &user))
                .filter(|a| a.factor);
            match fresh {
                Some(a) => {
                    if let Some(id) = self.speak_advisory(&st, &a, &user, true, false) {
                        self.comms.advisories.follow = None;
                        self.comms.advisories.last_call_s = Some(now);
                        self.comms.advisories.open = Some(Open {
                            aircraft: a.id,
                            event_id: id,
                            station: st,
                            issued_s: now,
                            spoken_end_s: None,
                            repeats: 0,
                            first: false,
                        });
                        if let Some(seen) = self.comms.advisories.seen.get_mut(&a.id) {
                            seen.updates += 1;
                            seen.last_s = now;
                            seen.clock = a.clock;
                            seen.miles = a.miles;
                            seen.phase = Phase::Looking;
                        }
                    }
                    return true;
                }
                None => kind = FollowKind::NoFactor,
            }
        }
        let (phrase, tag) = match kind {
            FollowKind::NoFactor | FollowKind::Update => {
                (phraseology::traffic_no_factor(), TrafficFollowUpKind::NoFactor)
            }
            FollowKind::VisualSeparation => (
                phraseology::maintain_visual_separation(),
                TrafficFollowUpKind::VisualSeparation,
            ),
            FollowKind::Follow => (phraseology::follow_that_traffic(), TrafficFollowUpKind::Follow),
            FollowKind::Roger => ("Roger.", TrafficFollowUpKind::Roger),
        };
        let text = format!("{}, {phrase}", self.controller_address());
        let meaning = Meaning::TrafficFollowUp {
            kind: tag,
            target_aircraft: vec![follow.aircraft],
            voice,
        };
        if self.deliver_user_call(&st, text, meaning).is_some() {
            self.comms.advisories.follow = None;
            self.comms.advisories.last_call_s = Some(now);
            if kind == FollowKind::NoFactor
                && let Some(seen) = self.comms.advisories.seen.get_mut(&follow.aircraft)
            {
                seen.phase = Phase::Closed;
            }
        }
        true
    }

    /// Find traffic worth a call: new aircraft, or updates for aircraft the pilot is still looking for.
    fn advisory_scan(&mut self, now: f64) {
        if self.comms.advisories.open.is_some()
            || self.comms.advisories.follow.is_some()
            || now < self.comms.advisories.next_scan_s
            || self
                .comms
                .advisories
                .last_call_s
                .is_some_and(|t| now - t < MIN_CALL_GAP_S)
        {
            return;
        }
        let Some(fleet) = self.comms.fleet.as_ref() else {
            return;
        };
        let Some(st) = self.advisory_station(fleet) else {
            self.comms.advisories.next_scan_s = now + 1.;
            return;
        };
        // Only compute the picture when the call could start this instant.
        if !self.channel_is_open(&st) {
            return;
        }
        self.comms.advisories.next_scan_s = now + 1.;
        let state = self.fleet_state();
        let user = self.user_motion();
        let followed = self.followed_callsigns();
        let alive: Vec<u32> = state.aircraft.iter().map(|a| a.id).collect();
        self.comms.advisories.seen.retain(|id, _| alive.contains(id));
        let book = &self.comms.advisories;
        let mut best: Option<(Assessment, bool)> = None;
        let mut clear: Option<u32> = None;
        for a in state.aircraft.iter().filter(|a| !a.on_ground) {
            if followed.contains(&a.callsign) {
                continue;
            }
            let r = assess(a, &user);
            match book.seen.get(&a.id) {
                None if r.factor => {}
                Some(s)
                    if s.phase == Phase::Looking
                        && now - s.last_s >= UPDATE_AFTER_S
                        && r.factor
                        && (s.clock != r.clock || s.miles != r.miles)
                        && s.updates < MAX_UPDATES => {}
                Some(s) if s.phase == Phase::Looking && !r.factor && now - s.last_s >= 8. => {
                    clear = clear.or(Some(a.id));
                    continue;
                }
                _ => continue,
            }
            let updated = book.seen.contains_key(&a.id);
            if best.as_ref().is_none_or(|(b, _)| r.score < b.score) {
                best = Some((r, updated));
            }
        }
        let Some((a, updated)) = best else {
            if let Some(id) = clear {
                self.comms.advisories.follow = Some(Follow {
                    kind: FollowKind::NoFactor,
                    aircraft: id,
                    station: st,
                    due_s: now,
                });
            }
            return;
        };
        let Some(event_id) = self.speak_advisory(&st, &a, &user, updated, false) else {
            return;
        };
        let book = &mut self.comms.advisories;
        book.last_call_s = Some(now);
        if !updated {
            book.tally.advisories_issued += 1;
        }
        let entry = book.seen.entry(a.id).or_insert(Seen {
            last_s: now,
            updates: 0,
            clock: a.clock,
            miles: a.miles,
            phase: Phase::Looking,
        });
        entry.last_s = now;
        entry.clock = a.clock;
        entry.miles = a.miles;
        entry.phase = Phase::Looking;
        if updated {
            entry.updates += 1;
        }
        book.open = Some(Open {
            aircraft: a.id,
            event_id,
            station: st,
            issued_s: now,
            spoken_end_s: None,
            repeats: 0,
            first: !updated,
        });
    }

    // -------------------------------------------------------------------- what the user heard and said

    /// An advisory event finished. Heard: the answer window opens. Otherwise it never reached the pilot.
    pub(in crate::comms) fn advisory_event_done(&mut self, event: &Transmission) {
        let now = self.sim.state().time_s;
        let book = &mut self.comms.advisories;
        let Some(open) = book.open.as_mut().filter(|o| o.event_id == event.id) else {
            return;
        };
        if event.reception == Reception::Heard {
            open.spoken_end_s = Some(now);
        } else {
            if open.first {
                book.seen.remove(&open.aircraft);
                book.tally.advisories_issued = book.tally.advisories_issued.saturating_sub(1);
            }
            book.open = None;
        }
    }

    /// A fleet transmission the user heard: remembered for 25 s so answering it can be recognised.
    pub(in crate::comms) fn note_heard_fleet(&mut self, event: &Transmission, call: &FleetCall) {
        if event.reception != Reception::Heard {
            return;
        }
        let book = &mut self.comms.advisories;
        if book.heard.len() >= HEARD_KEEP {
            book.heard.pop_front();
        }
        book.heard.push_back(HeardCall {
            end_s: event.time_s + event.duration_s,
            aircraft: call.aircraft,
            instructions: call.instructions.clone(),
            controller: call.role == FleetRole::Controller,
        });
    }

    /// Called when the user keys the microphone. A fleet or controller transmission on the same
    /// carrier that has not finished means the user transmitted over it.
    pub(crate) fn note_user_keyed(&mut self, own_id: u64, khz: u32) {
        if self.comms.fleet.is_none() {
            return;
        }
        let now = self.sim.state().time_s;
        let carrier = carrier_thirds_hz(khz);
        let over = self
            .comms
            .pending
            .iter()
            .filter(|p| p.id != own_id && p.carrier == carrier && p.end_s > now && p.eligible)
            .find_map(|p| {
                self.comms
                    .record
                    .events
                    .iter()
                    .rev()
                    .find(|e| e.id == p.id)
                    .filter(|e| {
                        matches!(
                            e.meaning,
                            Meaning::Fleet { .. }
                                | Meaning::TrafficAdvisory { .. }
                                | Meaning::TrafficFollowUp { .. }
                        )
                    })
                    .map(|e| e.speaker_name.clone())
            });
        let Some(who) = over else { return };
        if self
            .comms
            .advisories
            .last_stepped_s
            .is_some_and(|t| now - t < STEP_FAULT_GAP_S)
        {
            return;
        }
        self.comms.advisories.last_stepped_s = Some(now);
        self.comms.advisories.tally.stepped_on += 1;
        self.atc_grade(
            Some(own_id),
            "radio_discipline:stepped_on",
            false,
            format!("You transmitted while {who} was talking. Listen first, then key the microphone."),
        );
    }

    /// The fleet aircraft whose callsign the text uses, if any.
    fn other_callsign(&self, text: &str) -> Option<&Ac> {
        let heard = words(text);
        let padded = format!(" {heard} ");
        self.comms
            .fleet
            .as_ref()?
            .craft
            .iter()
            .filter(|a| !a.gone)
            .find(|a| {
                [&a.ident.full, &a.ident.short]
                    .iter()
                    .any(|c| padded.contains(&format!(" {} ", words(c))))
            })
    }

    /// Grade a heard user transmission for radio discipline, then apply it to an open advisory.
    /// Returns true when it answered the advisory and nothing else needs to handle it.
    pub(in crate::comms) fn user_call_heard(&mut self, event: &Transmission) -> bool {
        if self.comms.fleet.is_none() {
            return false;
        }
        let me = self.pilot_callsign();
        let m = parse_pilot(&event.text, &me);
        let now = self.sim.state().time_s;
        // Answering a call that was made to somebody else.
        if !m.callsign_present
            && let Some(other) = self.other_callsign(&event.text)
        {
            let spoken = other.ident.spec().spoken.to_string();
            let callsign = other.ident.full.clone();
            self.comms.advisories.tally.wrong_callsign += 1;
            self.atc_grade(
                Some(event.id),
                "radio_discipline:wrong_callsign",
                false,
                format!("That call was for the {spoken} ({callsign}), not for you. Answer only calls that use your callsign."),
            );
            return false;
        }
        let waiting_for_me = self.comms.advisories.open.is_some()
            || self.comms.advisories.follow.is_some()
            || !self.comms.atc.pending.is_empty();
        if !waiting_for_me {
            let recent: Vec<HeardCall> = self
                .comms
                .advisories
                .heard
                .iter()
                .filter(|h| {
                    h.controller
                        && event.time_s >= h.end_s - 1.
                        && event.time_s - h.end_s <= OTHER_CALL_WINDOW_S
                })
                .cloned()
                .collect();
            let acknowledges = m
                .normalized
                .split_whitespace()
                .any(|w| matches!(w, "roger" | "wilco" | "affirmative" | "copy"));
            // Reading back any part of another aircraft's clearance is the fault, not only all of it.
            let read_back = recent
                .iter()
                .rev()
                .find(|h| h.instructions.iter().any(|i| readback_matches(i, &m)));
            let acked = recent
                .iter()
                .rev()
                .find(|h| acknowledges && event.time_s - h.end_s <= ACK_WINDOW_S);
            if let Some(h) = read_back.or(acked) {
                let who = self
                    .comms
                    .fleet
                    .as_ref()
                    .and_then(|f| f.craft.iter().find(|a| a.id == h.aircraft))
                    .map(|a| (a.ident.spec().spoken.to_string(), a.ident.full.clone()));
                if let Some((spoken, callsign)) = who {
                    self.comms.advisories.tally.acknowledged_other += 1;
                    self.atc_grade(
                        Some(event.id),
                        "radio_discipline:acknowledged_other",
                        false,
                        format!("That call was for the {spoken} ({callsign}), not for you. Do not read back or acknowledge other aircraft's instructions."),
                    );
                    return false;
                }
            }
        }
        self.advisory_answer(event, &m, now)
    }

    /// The user's reply to an open advisory.
    fn advisory_answer(&mut self, event: &Transmission, m: &PilotMessage, now: f64) -> bool {
        let Some(open) = self.comms.advisories.open.clone() else {
            return false;
        };
        let Some(end) = open.spoken_end_s else {
            return false;
        };
        let carrier = carrier_thirds_hz(open.station.channel_khz);
        let on_frequency = event
            .frequency_hz
            .map(|hz| (hz * 3.).round() as u32)
            .is_some_and(|c| Some(c) == carrier);
        if !m.callsign_present || !on_frequency || event.time_s + event.duration_s < end {
            return false;
        }
        let delay = (event.time_s - end).max(0.);
        let reply = m.request;
        let spoken_reply = matches!(
            reply,
            Some(PilotRequest::TrafficInSight | PilotRequest::NegativeContact | PilotRequest::TrafficLooking)
        );
        let acknowledged = m
            .normalized
            .split_whitespace()
            .any(|w| matches!(w, "roger" | "wilco" | "copy"));
        if !spoken_reply && !acknowledged {
            return false;
        }
        let user = self.user_motion();
        let assessed = self
            .fleet_state()
            .aircraft
            .iter()
            .find(|a| a.id == open.aircraft)
            .map(|a| (assess(a, &user), a.position_m));
        let book = &mut self.comms.advisories;
        book.open = None;
        book.tally.advisories_answered += 1;
        let (note, follow) = match reply {
            Some(PilotRequest::TrafficInSight) => {
                if let Some(seen) = book.seen.get_mut(&open.aircraft) {
                    seen.phase = Phase::Closed;
                }
                let ahead = assessed.as_ref().is_some_and(|(a, pos)| {
                    open.station.kind == FacilityKind::Tower
                        && self.landing_ahead(open.aircraft, *pos)
                        && a.range_nm < 8.
                });
                let factor = assessed
                    .as_ref()
                    .is_some_and(|(a, _)| a.factor && (a.range_nm <= 3. || a.cpa_nm < 2.));
                let kind = if ahead {
                    FollowKind::Follow
                } else if factor {
                    FollowKind::VisualSeparation
                } else {
                    FollowKind::Roger
                };
                (format!("Traffic in sight, reported {delay:.0} s after the call."), Some(kind))
            }
            Some(PilotRequest::NegativeContact) => {
                let (updates, factor) = (
                    book.seen.get(&open.aircraft).map_or(0, |s| s.updates),
                    assessed.as_ref().is_some_and(|(a, _)| a.factor),
                );
                let kind = if !factor {
                    Some(FollowKind::NoFactor)
                } else if updates < MAX_UPDATES {
                    Some(FollowKind::Update)
                } else {
                    None
                };
                (format!("Negative contact reported {delay:.0} s after the call."), kind)
            }
            Some(PilotRequest::TrafficLooking) => (format!("Looking for the traffic, reported {delay:.0} s after the call."), None),
            _ => (
                "Acknowledged the traffic call. Next time say looking, traffic in sight, or negative contact.".to_string(),
                None,
            ),
        };
        let book = &mut self.comms.advisories;
        if let Some(kind) = follow {
            book.follow = Some(Follow {
                kind,
                aircraft: open.aircraft,
                station: open.station.clone(),
                due_s: now + 1.5,
            });
        }
        self.atc_grade(Some(open.event_id), "traffic_advisory", true, note);
        true
    }

    /// The traffic is a lander on base or final that will reach the runway before the user.
    fn landing_ahead(&self, id: u32, position_m: [f64; 3]) -> bool {
        let Some(fleet) = &self.comms.fleet else {
            return false;
        };
        let leg = self
            .fleet_targets()
            .into_iter()
            .find(|t| t.address == 0xf20000 + id)
            .map(|t| t.leg);
        if !matches!(leg, Some(PatternLeg::Base | PatternLeg::Final)) {
            return false;
        }
        let [pn, pe, _] = self.sim.state().position_m;
        let thr = fleet.field.thr;
        libm::hypot(position_m[0] - thr[0], position_m[1] - thr[1]) + 0.3 * NM
            < libm::hypot(pn - thr[0], pe - thr[1])
    }
}
