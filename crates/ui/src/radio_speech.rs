//! Presentation-only speech and original radio DSP; never called by headless simulation.
//!
//! A received call is voiced while it is on the air: its squelch break and voice start when the
//! transmission starts (while the event is still `Pending` and the user's receiver can hear it), not when
//! it ends. The [`Planner`] decides what to start and when to cut; the platform `Output` only plays.
#[cfg(any(target_arch = "wasm32", feature = "desktop-audio"))]
use flightcraft_control::Channel;
use flightcraft_control::{Meaning, OnAir, Reception, Session, Speaker, Transmission};
#[cfg(any(test, target_arch = "wasm32", feature = "desktop-audio"))]
mod dsp;
#[cfg(all(not(target_arch = "wasm32"), feature = "desktop-audio"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

/// One thing the audio output has to do now.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Action {
    /// Silence the call with this event id, or with `None` everything that is playing.
    Stop(Option<u64>),
    /// Begin a received call. `at_s` is the simulation time the audio starts; the audio must fit the
    /// carrier time `event.duration_s`.
    Say { event: Box<Transmission>, at_s: f64 },
    /// Two carriers keyed together: a heterodyne squeal for `seconds`, instead of either voice.
    Garble {
        event: Box<Transmission>,
        seconds: f64,
        at_s: f64,
    },
}

/// Decides, each frame, which calls start playing and which are cut. Pure: it reads the session and
/// returns actions, so it is tested without an audio device.
#[derive(Default)]
pub(crate) struct Planner {
    generation: Option<u64>,
    /// Highest event id already considered.
    seen: u64,
    /// Calls voiced as speech that are still on the air; a collision cuts them.
    live: Vec<u64>,
}
impl Planner {
    pub fn plan(&mut self, session: &Session, active: bool) -> Vec<Action> {
        let mut actions = Vec::new();
        let now = session.sim().state().time_s;
        let events = &session.flight_record().events;
        if self.generation != Some(session.flight_generation()) {
            self.generation = Some(session.flight_generation());
            self.seen = 0;
            self.live.clear();
            actions.push(Action::Stop(None));
        }
        if !active {
            // Nothing plays while the speech switch, volume or power is off, and nothing is voiced late.
            if !self.live.is_empty() {
                actions.push(Action::Stop(None));
                self.live.clear();
            }
            self.seen = events.last().map_or(self.seen, |e| e.id.max(self.seen));
            return actions;
        }
        // Calls already being voiced: cut one that two carriers have turned into a squeal, and play the
        // squeal for the rest of its carrier time.
        let mut cuts = Vec::new();
        let mut garbles = Vec::new();
        self.live.retain(|id| {
            let Some(event) = events.iter().rev().find(|e| e.id == *id) else {
                return false;
            };
            let end = event.time_s + event.duration_s;
            match session.on_air(*id) {
                OnAir::Audible => event.reception == Reception::Pending,
                state => {
                    if end - now > 0.05 {
                        cuts.push(*id);
                        if state == OnAir::Blocked {
                            garbles.push((event.clone(), end - now));
                        }
                    }
                    false
                }
            }
        });
        // Only the blocked call is cut; calls on other receivers keep playing.
        actions.extend(cuts.into_iter().map(|id| Action::Stop(Some(id))));
        for (event, seconds) in garbles {
            actions.push(Action::Garble {
                event: Box::new(event),
                seconds,
                at_s: now,
            });
        }
        let seen = self.seen;
        for event in events.iter().filter(|e| e.id > seen) {
            self.seen = self.seen.max(event.id);
            if !event.is_spoken()
                || (event.speaker == Speaker::Pilot
                    && event.source != flightcraft_control::EventSource::Rules)
            {
                continue;
            }
            let end = event.time_s + event.duration_s;
            // A call that ended before it was noticed is not voiced late.
            if end + 0.5 < now {
                continue;
            }
            match session.on_air(event.id) {
                OnAir::Unheard => {}
                OnAir::Blocked => {
                    if event.channel == flightcraft_control::Channel::Com
                        && event.speaker != Speaker::Pilot
                        && end - now > 0.05
                    {
                        actions.push(Action::Garble {
                            event: Box::new(event.clone()),
                            seconds: end - now,
                            at_s: now,
                        });
                    }
                }
                OnAir::Audible => {
                    if matches!(event.meaning, Meaning::Crew { critical: true }) {
                        actions.push(Action::Stop(None));
                        self.live.clear();
                    }
                    if event.reception == Reception::Pending {
                        self.live.push(event.id);
                    }
                    actions.push(Action::Say {
                        event: Box::new(event.clone()),
                        at_s: now,
                    });
                }
            }
        }
        actions
    }
}

#[derive(Default)]
pub struct Speech {
    planner: Planner,
    error: Option<String>,
    #[cfg(all(not(target_arch = "wasm32"), feature = "desktop-audio"))]
    output: Option<native::Output>,
    #[cfg(target_arch = "wasm32")]
    output: Option<web::Output>,
}
impl Speech {
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn observe(&mut self, session: &Session, master_gain: f32) {
        let state = session.comms_state();
        let active = master_gain.is_finite()
            && master_gain > 0.0
            && state.panel.speech
            && state.panel.volume > 0.0
            && state.powered;
        #[cfg(any(target_arch = "wasm32", feature = "desktop-audio"))]
        if let Some(output) = &mut self.output {
            output.poll(active, &mut self.error);
        }
        let actions = self.planner.plan(session, active);
        #[cfg(any(target_arch = "wasm32", feature = "desktop-audio"))]
        {
            let volume = state.panel.volume as f32 * master_gain.clamp(0.0, 1.0);
            for action in actions {
                if let Action::Stop(which) = action {
                    if let Some(output) = &mut self.output {
                        match which {
                            Some(id) => output.stop_call(id),
                            None => output.stop(),
                        }
                    }
                    continue;
                }
                if self.output.is_none() {
                    #[cfg(target_arch = "wasm32")]
                    let result = web::Output::new();
                    #[cfg(all(not(target_arch = "wasm32"), feature = "desktop-audio"))]
                    let result = native::Output::new();
                    match result {
                        Ok(output) => self.output = Some(output),
                        Err(error) => {
                            self.error = Some(error);
                            continue;
                        }
                    }
                }
                let Some(output) = &mut self.output else { continue };
                let result = match action {
                    Action::Say { event, .. } => {
                        let voice = flightcraft_control::speaker_voice(&event.speaker, &event.meaning)
                            .unwrap_or_else(|| flightcraft_control::pilot_voice(0, 0));
                        let distance = session.transmission_distance_fraction(&event);
                        output.say(&event, volume, voice, distance)
                    }
                    Action::Garble { event, seconds, .. } => {
                        debug_assert_eq!(event.channel, Channel::Com);
                        output.garble(&event, seconds as f32, volume)
                    }
                    Action::Stop(_) => Ok(()),
                };
                if let Err(error) = result {
                    self.error = Some(error);
                }
            }
        }
        #[cfg(not(any(target_arch = "wasm32", feature = "desktop-audio")))]
        if actions
            .iter()
            .any(|a| matches!(a, Action::Say { .. } | Action::Garble { .. }))
        {
            self.error =
                Some("Speech output needs the desktop audio feature; captions remain available.".into());
        }
    }
}
#[cfg(any(test, target_arch = "wasm32", feature = "desktop-audio"))]
fn morse_samples(code: &str, rate: u32) -> Vec<f32> {
    let mut result = Vec::new();
    for symbol in code.chars() {
        let units = match symbol {
            '.' => 1,
            '-' => 3,
            ' ' => 2,
            _ => 0,
        };
        let len = rate * units / 10;
        for i in 0..len {
            let envelope = (i as f32 / (rate as f32 * 0.004)).min(1.0)
                * ((len - i) as f32 / (rate as f32 * 0.004)).min(1.0);
            result.push(if symbol == ' ' {
                0.0
            } else {
                (libm::sin(f64::from(i) / f64::from(rate) * core::f64::consts::TAU * 1020.0) as f32)
                    * envelope
                    * 0.25
            });
        }
        if symbol != ' ' {
            result.extend(std::iter::repeat_n(0.0, (rate / 10) as usize));
        }
    }
    result
}
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::unreachable)]
    use super::*;
    use serde_json::json;
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/atc_fixture.rs"));

    #[test]
    fn morse_is_audible_tone() {
        let morse = morse_samples("... -... .---", 22050);
        assert!(morse.iter().any(|v| v.abs() > 0.2));
        assert!(morse.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
    }

    /// One frame of the UI: 1/60 s of simulation, then the planner.
    const FRAME_S: f64 = 1.0 / 60.0;
    fn frame(s: &mut Session) {
        s.run(flightcraft_control::Step { steps: 2 }).unwrap();
    }
    fn now(s: &Session) -> f64 {
        s.sim().state().time_s
    }
    /// Busy KABE traffic. With `listen`, COM1 and COM2 sit on the two approach sectors.
    fn busy_session(listen: bool) -> Session {
        let mut s = atc_session("KABE");
        let stations = s.comms_state().stations;
        let find = |sector: &str| {
            stations
                .iter()
                .find(|x| {
                    x.airport == "KABE"
                        && x.kind == flightcraft_world::FacilityKind::Approach
                        && x.metadata.as_ref().is_some_and(|m| m.sector.contains(sector))
                })
                .unwrap()
                .channel_khz
        };
        if listen {
            let (hi, lo) = (find("ABV"), find("BLW"));
            s.execute("comms.tune", json!({"radio":1,"channel_khz":hi}))
                .unwrap();
            s.execute("comms.tune", json!({"radio":2,"channel_khz":lo}))
                .unwrap();
            s.execute("comms.audio_panel", json!({"receive_com2":true}))
                .unwrap();
        }
        s.execute("traffic.density", json!({"density":"busy","airport":"KABE"}))
            .unwrap();
        s
    }
    fn fleet_say(a: &Action) -> Option<(&Transmission, f64)> {
        match a {
            Action::Say { event, at_s } if matches!(event.meaning, Meaning::Fleet { .. }) => {
                Some((event, *at_s))
            }
            _ => None,
        }
    }
    /// Frames until the planner voices a fleet call; returns it with the actions of that frame.
    fn until_say(s: &mut Session, planner: &mut Planner) -> (Transmission, f64, Vec<Action>) {
        for _ in 0..60 * 600 {
            frame(s);
            let actions = planner.plan(s, true);
            if let Some((event, at_s)) = actions.iter().find_map(fleet_say) {
                return (event.clone(), at_s, actions.clone());
            }
        }
        panic!("no fleet call was voiced in ten minutes");
    }

    #[test]
    fn a_received_calls_audio_starts_with_the_transmission() {
        let mut s = busy_session(true);
        let mut planner = Planner::default();
        let mut voiced = Vec::new();
        for _ in 0..60 * 400 {
            frame(&mut s);
            for action in planner.plan(&s, true) {
                if let Some((event, at_s)) = fleet_say(&action) {
                    // Started within one frame of the event's start, while it is still on the air.
                    assert_eq!(event.reception, Reception::Pending, "{event:?}");
                    assert!(
                        at_s - event.time_s >= 0.0 && at_s - event.time_s <= FRAME_S + 1e-9,
                        "audio at {at_s} for a call starting {}",
                        event.time_s
                    );
                    assert!(at_s < event.time_s + event.duration_s);
                    voiced.push(event.id);
                }
            }
        }
        assert!(voiced.len() > 8, "{} calls voiced", voiced.len());
        // Every call heard on the frequency was voiced as it started, once.
        let mut unique = voiced.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), voiced.len());
        for e in &s.flight_record().events {
            if e.reception == Reception::Heard && matches!(e.meaning, Meaning::Fleet { .. }) {
                assert!(
                    voiced.contains(&e.id) || e.time_s < FRAME_S * 2.0,
                    "{e:?} never voiced"
                );
            }
        }
    }

    #[test]
    fn the_audio_fits_inside_the_carrier() {
        let mut s = busy_session(true);
        let mut planner = Planner::default();
        for _ in 0..8 {
            let (event, _, _) = until_say(&mut s, &mut planner);
            let Meaning::Fleet { call } = &event.meaning else {
                unreachable!()
            };
            // The speech the voice needs is the carrier time; the renderer fits it to that slot.
            let speech_s = flightcraft_control::fleet_voice(call).duration_s(&event.text);
            assert!(
                speech_s <= event.duration_s + 1e-9,
                "{speech_s} vs {}",
                event.duration_s
            );
        }
    }

    #[test]
    fn keying_the_mic_mid_call_cuts_the_audio_and_squeals() {
        let mut s = busy_session(true);
        let mut planner = Planner::default();
        let (event, _, _) = until_say(&mut s, &mut planner);
        let Meaning::Fleet { call } = &event.meaning else {
            unreachable!()
        };
        let radio = if s.comms_state().com1.active_khz == call.frequency_khz {
            1
        } else {
            2
        };
        // A few frames into the call, the user keys the mic on the same frequency.
        for _ in 0..30 {
            frame(&mut s);
            assert!(
                planner
                    .plan(&s, true)
                    .iter()
                    .all(|a| fleet_say(a).is_none_or(|(e, _)| e.id != event.id))
            );
        }
        s.execute("comms.audio_panel", json!({"transmit":radio})).unwrap();
        s.execute("comms.transmit", json!({"text":"Approach, say again"}))
            .unwrap();
        let actions = planner.plan(&s, true);
        let stop = actions
            .iter()
            .position(|a| *a == Action::Stop(Some(event.id)))
            .expect("audio cut");
        let garble = actions
            .iter()
            .position(|a| matches!(a, Action::Garble { event: e, .. } if e.id == event.id))
            .expect("heterodyne for the overlap");
        assert!(stop < garble, "{actions:?}");
        let Action::Garble { seconds, .. } = &actions[garble] else {
            unreachable!()
        };
        let remaining = event.time_s + event.duration_s - now(&s);
        assert!(
            (seconds - remaining).abs() < 1e-6 && *seconds > 0.0,
            "{seconds} vs {remaining}"
        );
        // Nothing further is voiced for that call, and the squeal is not repeated.
        for _ in 0..60 * 20 {
            frame(&mut s);
            let more = planner.plan(&s, true);
            assert!(
                more.iter().all(|a| !matches!(a, Action::Garble { event: e, .. } | Action::Say { event: e, .. } if e.id == event.id)),
                "{more:?}"
            );
        }
    }

    #[test]
    fn blocking_one_receivers_call_leaves_the_other_receivers_call_playing() {
        let mut s = busy_session(true);
        let mut planner = Planner::default();
        let mut voiced: Vec<Transmission> = Vec::new();
        let mut pair = None;
        // Frames until a call on each receiver's frequency is on the air together.
        'search: for _ in 0..60 * 3000 {
            frame(&mut s);
            for action in planner.plan(&s, true) {
                if let Some((event, _)) = fleet_say(&action) {
                    voiced.push(event.clone());
                }
            }
            let t = now(&s);
            let (f1, f2) = (
                f64::from(s.comms_state().com1.active_khz) * 1000.0,
                f64::from(s.comms_state().com2.active_khz) * 1000.0,
            );
            let live: Vec<&Transmission> = voiced
                .iter()
                .filter(|e| planner.live.contains(&e.id) && e.time_s + e.duration_s - t > 0.5)
                .collect();
            let on = |hz: f64| {
                live.iter()
                    .find(|e| e.frequency_hz.is_some_and(|f| (f - hz).abs() < 1.0))
            };
            if let (Some(a), Some(b)) = (on(f1), on(f2)) {
                pair = Some(((**a).clone(), (**b).clone()));
                break 'search;
            }
        }
        let (com1_call, com2_call) = pair.expect("calls on both receivers overlapped");
        s.execute("comms.audio_panel", json!({"transmit":1})).unwrap();
        s.execute("comms.transmit", json!({"text":"Approach, say again"}))
            .unwrap();
        let actions = planner.plan(&s, true);
        let stops: Vec<&Action> = actions.iter().filter(|a| matches!(a, Action::Stop(_))).collect();
        assert_eq!(stops, [&Action::Stop(Some(com1_call.id))], "{actions:?}");
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, Action::Garble { event, .. } if event.id == com1_call.id)),
            "{actions:?}"
        );
        assert!(planner.live.contains(&com2_call.id), "{:?}", planner.live);
        assert!(!planner.live.contains(&com1_call.id));
    }

    #[test]
    fn a_call_the_user_cannot_hear_never_queues_audio() {
        // Not tuned to approach: the fleet calls are not even recorded for this receiver.
        let mut s = busy_session(false);
        let mut planner = Planner::default();
        for _ in 0..60 * 300 {
            frame(&mut s);
            let actions = planner.plan(&s, true);
            assert!(
                actions
                    .iter()
                    .all(|a| fleet_say(a).is_none() && !matches!(a, Action::Garble { .. })),
                "{actions:?}"
            );
        }
        // A heard call that the receiver then loses (retuned mid-call) is cut and never finishes.
        let mut s = busy_session(true);
        let mut planner = Planner::default();
        let (event, _, _) = until_say(&mut s, &mut planner);
        s.execute("comms.tune", json!({"radio":1,"channel_khz":118_000}))
            .unwrap();
        s.execute("comms.tune", json!({"radio":2,"channel_khz":118_000}))
            .unwrap();
        let actions = planner.plan(&s, true);
        assert!(actions.contains(&Action::Stop(Some(event.id))), "{actions:?}");
        assert!(
            actions
                .iter()
                .all(|a| !matches!(a, Action::Say { .. } | Action::Garble { .. }))
        );
        // Speech switched off: nothing is voiced at all.
        let mut s = busy_session(true);
        let mut planner = Planner::default();
        for _ in 0..60 * 200 {
            frame(&mut s);
            assert!(planner.plan(&s, false).iter().all(|a| *a == Action::Stop(None)));
        }
    }
}
