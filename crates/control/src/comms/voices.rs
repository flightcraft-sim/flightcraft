//! Radio voices: which synthetic voice speaks a transmission, and how long it takes to say.
//!
//! Presentation data only. The simulation uses `fleet_call_duration` so a transmission occupies its
//! frequency for as long as the voice takes to say it; the audio layer uses `speaker_voice` to pick
//! the engine voice, pitch, rate, and radio character. Nothing here enters the physics state.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gender {
    Male,
    Female,
}

/// Character of one radio: its band edges, how hard its audio stage clips, and its hiss floor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadioCharacter {
    pub low_cut_hz: f32,
    pub high_cut_hz: f32,
    pub drive: f32,
    pub hiss: f32,
}

/// A synthetic voice. `variant` is a timbre number (1-7) the engine maps to an installed voice.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoiceProfile {
    pub gender: Gender,
    pub variant: u8,
    /// espeak scale: 0-99, 50 neutral.
    pub pitch: u8,
    /// Words per minute.
    pub wpm: u16,
    pub radio: RadioCharacter,
}
impl VoiceProfile {
    /// Seconds the voice takes to say `text`.
    pub fn duration_s(&self, text: &str) -> f64 {
        speech_duration_s(text, self.wpm)
    }
}

// (gender, variant, pitch, wpm)
const CONTROLLERS: [(Gender, u8, u8, u16); 8] = [
    (Gender::Male, 2, 38, 184),
    (Gender::Female, 1, 60, 188),
    (Gender::Male, 4, 46, 178),
    (Gender::Female, 3, 68, 192),
    (Gender::Male, 6, 30, 180),
    (Gender::Female, 4, 54, 194),
    (Gender::Male, 5, 52, 182),
    (Gender::Female, 2, 74, 178),
];
const PILOTS: [(Gender, u8, u8, u16); 8] = [
    (Gender::Male, 1, 50, 166),
    (Gender::Male, 3, 42, 172),
    (Gender::Female, 5, 58, 168),
    (Gender::Male, 7, 34, 160),
    (Gender::Female, 1, 66, 176),
    (Gender::Male, 4, 56, 158),
    (Gender::Female, 2, 48, 170),
    (Gender::Male, 2, 62, 164),
];
/// Per-aircraft offsets applied to the shared palette so two aircraft on one base voice still differ.
/// `aircraft % 6` picks the pitch step and `aircraft / 6 % 5` the rate step: injective below 30.
const PITCH_STEP: [i16; 6] = [-9, -5, -2, 2, 5, 9];
const RATE_STEP: [i16; 5] = [-10, -5, 0, 5, 10];

fn radio_for(palette: u8, index: usize) -> RadioCharacter {
    let i = index % 8;
    RadioCharacter {
        low_cut_hz: 270.0 + 14.0 * i as f32 + 6.0 * f32::from(palette),
        high_cut_hz: 2750.0 + 85.0 * ((i * 5) % 8) as f32,
        drive: 1.8 + 0.18 * ((i * 3) % 8) as f32,
        hiss: 0.004 + 0.0007 * ((i * 7) % 8) as f32,
    }
}

/// A controller's voice. `voice` is `FleetCall::voice` for a facility (0-7).
pub fn controller_voice(voice: u8) -> VoiceProfile {
    let i = usize::from(voice % 8);
    let (gender, variant, pitch, wpm) = CONTROLLERS.get(i).copied().unwrap_or(CONTROLLERS[0]);
    VoiceProfile {
        gender,
        variant,
        pitch,
        wpm,
        radio: radio_for(0, i),
    }
}
/// An AI pilot's voice: the shared palette entry plus a stable per-aircraft offset.
pub fn pilot_voice(voice: u8, aircraft: u32) -> VoiceProfile {
    let i = usize::from(voice % 8);
    let (gender, variant, pitch, wpm) = PILOTS.get(i).copied().unwrap_or(PILOTS[0]);
    let dp = PITCH_STEP.get((aircraft % 6) as usize).copied().unwrap_or(0);
    let dw = RATE_STEP.get(((aircraft / 6) % 5) as usize).copied().unwrap_or(0);
    VoiceProfile {
        gender,
        variant,
        pitch: (i16::from(pitch) + dp).clamp(5, 95) as u8,
        wpm: (i32::from(wpm) + i32::from(dw)).clamp(110, 240) as u16,
        radio: radio_for(1, i + (aircraft % 3) as usize),
    }
}
/// The voice of one simulated-traffic transmission.
pub fn fleet_voice(call: &FleetCall) -> VoiceProfile {
    match call.role {
        FleetRole::Controller => controller_voice(call.voice),
        FleetRole::Pilot => pilot_voice(call.voice, call.aircraft),
    }
}
/// Seconds a simulated-traffic transmission occupies its frequency. The fleet scheduler and the
/// recorded event both use this, so the audio and the carrier agree.
pub fn fleet_call_duration(text: &str, call: &FleetCall) -> f64 {
    fleet_voice(call).duration_s(text)
}

fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}
/// Base controller slot by facility kind, so Tower, Ground, Approach, and Departure always differ.
pub fn facility_slot(kind: FacilityKind) -> u8 {
    match kind {
        FacilityKind::Tower => 0,
        FacilityKind::Ground => 1,
        FacilityKind::Departure => 2,
        FacilityKind::Approach => 3,
        FacilityKind::Clearance => 4,
        FacilityKind::Atis => 5,
        FacilityKind::Awos | FacilityKind::Asos => 6,
        FacilityKind::Ctaf | FacilityKind::Unicom => 7,
    }
}
/// The stable voice of `stations[index]`: its kind's slot, shifted by three for each other station of
/// the same kind with a smaller id (so approach high and low differ).
pub fn facility_voice(stations: &[ComStation], index: usize) -> u8 {
    let Some(st) = stations.get(index) else { return 0 };
    let mut peers: Vec<&str> = stations
        .iter()
        .filter(|o| o.kind == st.kind)
        .map(|o| o.id.as_str())
        .collect();
    peers.sort_unstable();
    peers.dedup();
    let rank = peers.iter().position(|id| *id == st.id).unwrap_or(0) as u8;
    (facility_slot(st.kind) + 3 * rank) % 8
}

/// The voice for any speaker. `None` for the user's own transmissions, which are never re-voiced.
pub fn speaker_voice(speaker: &Speaker, meaning: &Meaning) -> Option<VoiceProfile> {
    if let Meaning::Fleet { call } = meaning {
        return Some(fleet_voice(call));
    }
    Some(match speaker {
        Speaker::Pilot => return None,
        Speaker::Instructor => VoiceProfile {
            gender: Gender::Female,
            variant: 3,
            pitch: 56,
            wpm: 158,
            radio: radio_for(0, 2),
        },
        Speaker::Crew => VoiceProfile {
            gender: Gender::Male,
            variant: 2,
            pitch: 44,
            wpm: 165,
            radio: radio_for(0, 5),
        },
        Speaker::Facility { role, .. } => controller_voice(facility_slot(*role)),
        Speaker::Traffic { callsign } => {
            let h = fnv(callsign);
            pilot_voice((h % 8) as u8, ((h >> 8) % 30) as u32)
        }
        Speaker::Nav { .. } => controller_voice(7),
    })
}

/// Deterministic speaking time: a fixed word rate, a short pause at commas, a longer one at the end of
/// a sentence, and a fifth of a second for the squelch break. The audio layer synthesizes at the same
/// rate and speeds a voice up if the engine runs long.
pub fn speech_duration_s(text: &str, wpm: u16) -> f64 {
    let words = text.split_whitespace().count() as f64;
    let pauses: f64 = text
        .chars()
        .map(|c| match c {
            ',' => 0.08,
            '.' | '?' | '!' | ':' | ';' => 0.2,
            _ => 0.0,
        })
        .sum();
    (0.2 + words * 60.0 / f64::from(wpm.max(60)) + pauses).clamp(1.0, 45.0)
}

/// Service radius used for simulated pilots' transmissions.
pub(crate) const FLEET_PILOT_RANGE_NM: f64 = 40.0;

/// What the user's receiver is getting from a transmission right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnAir {
    /// The user's receiver can hear it and nothing else is keyed on the carrier.
    Audible,
    /// Two carriers are keyed together: the user hears a heterodyne squeal, not either voice.
    Blocked,
    /// Wrong frequency, out of range, receiver off, or otherwise not received.
    Unheard,
}

impl Session {
    /// The live reception state of transmission `id`, valid while it is still on the air (`Pending`)
    /// and, afterwards, derived from its final reception. The audio layer starts a call's audio the
    /// moment it is `Audible` and cuts it if it turns `Blocked`.
    pub fn on_air(&self, id: u64) -> OnAir {
        if let Some(p) = self.comms.pending.iter().find(|p| p.id == id) {
            return if !p.eligible {
                OnAir::Unheard
            } else if p.collided {
                OnAir::Blocked
            } else {
                OnAir::Audible
            };
        }
        match self
            .comms
            .record
            .events
            .iter()
            .rev()
            .find(|e| e.id == id)
            .map(|e| e.reception)
        {
            Some(Reception::Heard) => OnAir::Audible,
            Some(Reception::Blocked) => OnAir::Blocked,
            _ => OnAir::Unheard,
        }
    }
    /// How far a transmitter is, as a fraction of its service range (0 at the antenna, 1 at the edge).
    /// The audio layer raises the radio noise floor with it. Zero when the origin is unknown.
    pub fn transmission_distance_fraction(&self, event: &Transmission) -> f32 {
        let [pn, pe, _] = self.sim.state().position_m;
        let fraction = match &event.speaker {
            Speaker::Facility { id, .. } => {
                self.stations()
                    .into_iter()
                    .find(|st| &st.id == id)
                    .and_then(|st| {
                        let o = self.station_origin(&st)?;
                        Some(libm::hypot(o[0] - pn, o[1] - pe) / 1852.0 / st.range_nm.max(1.0))
                    })
            }
            Speaker::Traffic { callsign } => self
                .fleets()
                .flat_map(|f| f.craft.iter())
                .find(|a| &a.ident.full == callsign)
                .map(|a| libm::hypot(a.body.pos[0] - pn, a.body.pos[1] - pe) / 1852.0 / FLEET_PILOT_RANGE_NM),
            _ => None,
        };
        fraction.map_or(0.0, |v| v.clamp(0.0, 1.0) as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(v: &VoiceProfile) -> (bool, u8, u8, u16) {
        (v.gender == Gender::Male, v.variant, v.pitch, v.wpm)
    }
    #[test]
    fn controller_voices_are_distinct_and_stable() {
        let all: Vec<_> = (0..8).map(controller_voice).collect();
        for (i, a) in all.iter().enumerate() {
            assert_eq!(*a, controller_voice(i as u8));
            for b in &all[i + 1..] {
                assert_ne!(key(a), key(b));
            }
        }
        assert!(all.iter().any(|v| v.gender == Gender::Female));
        assert!(all.iter().any(|v| v.gender == Gender::Male));
    }
    #[test]
    fn every_aircraft_gets_a_distinct_stable_voice() {
        // Even thirty aircraft on one base voice differ from one another.
        for base in 0..8_u8 {
            let all: Vec<_> = (0..30).map(|a| pilot_voice(base, a)).collect();
            for (i, a) in all.iter().enumerate() {
                assert_eq!(*a, pilot_voice(base, i as u32));
                for b in &all[i + 1..] {
                    assert_ne!(key(a), key(b), "base {base}");
                }
            }
        }
        for p in 0..8 {
            for c in 0..8 {
                assert_ne!(key(&pilot_voice(p, 0)), key(&controller_voice(c)));
            }
        }
    }
    fn station(id: &str, kind: FacilityKind) -> ComStation {
        ComStation {
            id: id.into(),
            airport: "KABE".into(),
            name: id.into(),
            kind,
            channel_khz: 120_000,
            position: flightcraft_world::navdata::GeoPoint {
                latitude_deg: 40.0,
                longitude_deg: -75.0,
            },
            elevation_m: 100.0,
            antenna_agl_m: 10.0,
            range_nm: 20.0,
            source: "test".into(),
            metadata: None,
        }
    }
    #[test]
    fn facilities_at_one_airport_differ() {
        let stations = vec![
            station("KABE_TWR", FacilityKind::Tower),
            station("KABE_GND", FacilityKind::Ground),
            station("KABE_APP_HI", FacilityKind::Approach),
            station("KABE_APP_LO", FacilityKind::Approach),
            station("KABE_DEP", FacilityKind::Departure),
        ];
        let voices: Vec<u8> = (0..stations.len())
            .map(|i| facility_voice(&stations, i))
            .collect();
        let mut unique = voices.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), stations.len(), "{voices:?}");
        let again: Vec<u8> = (0..stations.len())
            .map(|i| facility_voice(&stations, i))
            .collect();
        assert_eq!(voices, again);
    }
    #[test]
    fn duration_estimate_is_deterministic_and_tracks_rate() {
        let text = "Cessna four two seven kilo papa, descend and maintain three thousand.";
        assert_eq!(speech_duration_s(text, 160), speech_duration_s(text, 160));
        assert!(speech_duration_s(text, 120) > speech_duration_s(text, 200));
        let d = speech_duration_s(text, 160);
        assert!((d - (0.2 + 11.0 * 60.0 / 160.0 + 0.08 + 0.2)).abs() < 1e-9, "{d}");
        assert_eq!(speech_duration_s("", 160), 1.0);
        assert_eq!(speech_duration_s(&"word ".repeat(500), 160), 45.0);
    }
    #[test]
    fn users_own_transmissions_are_not_revoiced() {
        assert!(speaker_voice(&Speaker::Pilot, &Meaning::Text).is_none());
        assert!(speaker_voice(&Speaker::Instructor, &Meaning::Text).is_some());
    }
}
