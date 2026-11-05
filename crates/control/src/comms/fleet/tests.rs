#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::unreachable)]
use super::*;
use crate as flightcraft_control;
use serde_json::json;
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/atc_fixture.rs"));

fn advance(s: &mut Session, seconds: f64) {
    let mut left = seconds;
    while left > 0.0 {
        let chunk = left.min(300.0);
        left -= chunk;
        advance_chunk(s, chunk);
    }
}
fn advance_chunk(s: &mut Session, seconds: f64) {
    s.run(crate::Step {
        steps: (seconds * 120.).ceil() as u32,
    })
    .unwrap();
}
/// COM1 on approach high, COM2 on approach low: the two sectors a Busy arrival flow uses.
fn listen_to_approach(s: &mut Session) {
    let st = s.stations();
    let find = |sector: &str| {
        st.iter()
            .find(|x| {
                x.airport == "KABE"
                    && x.kind == FacilityKind::Approach
                    && x.metadata.as_ref().is_some_and(|m| m.sector.contains(sector))
            })
            .unwrap()
            .channel_khz
    };
    let (hi, lo) = (find("ABV"), find("BLW"));
    s.execute("comms.tune", json!({"radio":1,"channel_khz":hi}))
        .unwrap();
    s.execute("comms.tune", json!({"radio":2,"channel_khz":lo}))
        .unwrap();
    s.execute("comms.audio_panel", json!({"receive_com2":true}))
        .unwrap();
}
fn busy_session() -> Session {
    let mut s = atc_session("KABE");
    listen_to_approach(&mut s);
    s.execute("traffic.density", json!({"density":"busy","airport":"KABE"}))
        .unwrap();
    s
}
fn fleet_events(s: &Session) -> Vec<(Transmission, FleetCall)> {
    s.flight_record()
        .events
        .iter()
        .filter_map(|e| match &e.meaning {
            Meaning::Fleet { call } if e.reception == Reception::Heard => Some((e.clone(), call.clone())),
            _ => None,
        })
        .collect()
}
fn transcript(s: &Session) -> Vec<String> {
    fleet_events(s)
        .iter()
        .map(|(e, c)| {
            format!(
                "{:.0}s {:.3} {} | {}",
                e.time_s,
                f64::from(c.frequency_khz) / 1000.,
                e.speaker_name,
                e.text
            )
        })
        .collect()
}

#[test]
fn off_adds_nothing_and_state_is_empty() {
    let mut s = atc_session("KABE");
    advance(&mut s, 120.);
    assert!(fleet_events(&s).is_empty());
    let state = s.fleet_state();
    assert_eq!(state.density, TrafficDensity::Off);
    assert!(state.aircraft.is_empty());
    assert!(s.traffic_state().aircraft.is_empty());
}

#[test]
fn density_ranges_hold_through_a_session() {
    for (density, lo, hi) in [("light", 3, 5), ("moderate", 6, 9), ("busy", 10, 15)] {
        let mut s = atc_session("KABE");
        s.execute("traffic.density", json!({"density":density,"airport":"KABE"}))
            .unwrap();
        for _ in 0..12 {
            let n = s.fleet_state().aircraft.len();
            assert!((lo..=hi).contains(&n), "{density}: {n} aircraft");
            advance(&mut s, 60.);
        }
    }
}

#[test]
fn busy_kabe_gets_ten_aircraft_talking_in_fifteen_minutes() {
    let mut s = busy_session();
    advance(&mut s, 900.);
    let events = fleet_events(&s);
    let mut heard: Vec<u32> = events.iter().map(|(_, c)| c.aircraft).collect();
    heard.sort();
    heard.dedup();
    let lines = transcript(&s);
    assert!(
        heard.len() >= 10,
        "only {} aircraft heard\n{}",
        heard.len(),
        lines.join("\n")
    );
    // Different voices respond: many distinct pilot voices on the frequency.
    let mut voices: Vec<u8> = events
        .iter()
        .filter(|(_, c)| c.role == FleetRole::Pilot)
        .map(|(_, c)| c.voice)
        .collect();
    voices.sort();
    voices.dedup();
    assert!(voices.len() >= 4, "voices {voices:?}");
    // The sequence remark is tied to real traffic.
    let state = s.fleet_state();
    assert_eq!(
        state.stats.safety_slowdowns,
        0,
        "physics guard: {}",
        s.comms.fleet.as_ref().unwrap().guard_note
    );
    println!(
        "{}",
        lines.iter().take(60).cloned().collect::<Vec<_>>().join("\n")
    );
    println!("stats {:?}", state.stats);
}

#[test]
fn every_readback_is_accepted_or_corrected_then_accepted() {
    let mut s = busy_session();
    advance(&mut s, 900.);
    let events = fleet_events(&s);
    let mut exchanges: std::collections::BTreeMap<u32, Vec<&(Transmission, FleetCall)>> = Default::default();
    for e in &events {
        exchanges.entry(e.1.exchange).or_default().push(e);
    }
    let (mut ok, mut corrected) = (0, 0);
    for (id, calls) in &exchanges {
        let kinds: Vec<FleetCallKind> = calls.iter().map(|c| c.1.kind).collect();
        // The window can open mid-exchange; judge only exchanges that start with the controller.
        if kinds.first() != Some(&FleetCallKind::Instruction) {
            continue;
        }
        match kinds.as_slice() {
            [FleetCallKind::Instruction, FleetCallKind::Readback] => ok += 1,
            [
                FleetCallKind::Instruction,
                FleetCallKind::Readback,
                FleetCallKind::Correction,
                FleetCallKind::Readback,
            ] => corrected += 1,
            // The last exchange may still be in progress when the run ends.
            other if calls.last().is_some_and(|c| c.0.time_s > 880.) => {
                let _ = other;
            }
            other => panic!("exchange {id}: {other:?}\n{}", transcript(&s).join("\n")),
        }
    }
    assert!(ok > 10, "{ok} clean readbacks");
    assert!(corrected >= 1, "no corrected readback in 15 minutes");
    let st = s.fleet_state().stats;
    assert!(st.readbacks_accepted >= st.readbacks_corrected);
}

#[test]
fn aircraft_never_lose_separation_under_control() {
    let mut s = busy_session();
    let mut closest = f64::MAX;
    for _ in 0..(1800 / 2) {
        advance(&mut s, 2.);
        let ac: Vec<_> = s
            .fleet_state()
            .aircraft
            .into_iter()
            .filter(|a| !a.on_ground)
            .collect();
        for (i, a) in ac.iter().enumerate() {
            for b in &ac[i + 1..] {
                let lateral = libm::hypot(
                    a.position_m[0] - b.position_m[0],
                    a.position_m[1] - b.position_m[1],
                ) / 1852.;
                let vertical = (a.altitude_msl_ft - b.altitude_msl_ft).abs();
                if vertical < 500. {
                    closest = closest.min(lateral);
                }
                assert!(
                    !(lateral < 1.0 && vertical < 500.),
                    "{} and {} within {lateral:.2} NM, {vertical:.0} ft at {:.0}s ({} / {})",
                    a.callsign,
                    b.callsign,
                    s.sim().state().time_s,
                    a.intent.phase,
                    b.intent.phase
                );
            }
        }
    }
    println!("closest lateral with under 500 ft vertical: {closest:.2} NM");
    assert_eq!(s.fleet_state().stats.safety_slowdowns, 0);
}

#[test]
fn same_seed_same_transcript_and_other_seed_differs() {
    let run = |seed: u64| {
        let mut s = atc_session("KABE");
        listen_to_approach(&mut s);
        s.execute(
            "weather.set",
            json!({"wind_from_deg":0.,"wind_speed_mps":0.,"shear":false,"turbulence":"off","seed":seed}),
        )
        .unwrap();
        s.execute("traffic.density", json!({"density":"busy","airport":"KABE"}))
            .unwrap();
        advance(&mut s, 400.);
        transcript(&s)
    };
    let a = run(7);
    assert!(a.len() > 20);
    assert_eq!(a, run(7));
    assert_ne!(a, run(8));
}

#[test]
fn callsigns_are_fictional_and_follow_faa_format() {
    let s = busy_session();
    let mut seen = std::collections::BTreeSet::new();
    let mut rng = craft::Rng::new(99);
    for _ in 0..300 {
        let id = craft::identity(&mut rng, &[], 0.3);
        if id.company {
            assert!(
                ["Tamarack", "Brindle", "Quillon", "Harrow", "Fenwick"]
                    .iter()
                    .any(|n| id.full.starts_with(n)),
                "{}",
                id.full
            );
        } else {
            assert_eq!(
                validate_registration(&id.registration, false).unwrap(),
                id.registration
            );
            assert!(id.full.contains(id.spec().spoken), "{}", id.full);
        }
        seen.insert(id.full);
    }
    assert!(seen.len() > 200);
    for a in s.fleet_state().aircraft {
        assert!(a.callsign.chars().next().is_some_and(char::is_uppercase));
    }
}

#[test]
fn fleet_traffic_uses_real_procedures_and_facilities() {
    let s = busy_session();
    let state = s.fleet_state();
    assert_eq!(state.airport.as_deref(), Some("KABE"));
    let fleet = s.comms.fleet.as_ref().unwrap();
    assert_eq!(fleet.field.procedure, "I06");
    assert_eq!(fleet.field.family, "ILS");
    assert!(
        fleet.field.entries.iter().any(|e| !e.name.starts_with("FEED")),
        "no CIFP entry fix"
    );
    for a in &state.aircraft {
        assert!(
            s.stations()
                .iter()
                .any(|st| st.id == a.facility_id && st.channel_khz == a.frequency_khz)
        );
    }
    let kinds: std::collections::BTreeSet<_> = state
        .aircraft
        .iter()
        .map(|a| format!("{:?}", a.intent.kind))
        .collect();
    assert!(kinds.len() >= 3, "{kinds:?}");
}

#[test]
fn the_user_still_gets_service_with_traffic_on() {
    let mut s = atc_session("KABE");
    s.execute("traffic.density", json!({"density":"busy","airport":"KABE"}))
        .unwrap();
    let ground = s
        .stations()
        .into_iter()
        .find(|x| x.airport == "KABE" && x.kind == FacilityKind::Ground)
        .unwrap();
    s.execute("comms.tune", json!({"radio":1,"channel_khz":ground.channel_khz}))
        .unwrap();
    advance(&mut s, 5.);
    let info = s.atis_report("KABE").unwrap().information;
    s.execute(
        "comms.transmit",
        json!({"text":format!("{{callsign}}, with information {info}, request taxi")}),
    )
    .unwrap();
    advance(&mut s, 20.);
    assert!(
        s.atc_state()
            .pending_instructions
            .iter()
            .any(|p| matches!(p.instruction, AtcInstruction::Taxi { .. })),
        "{:?}",
        s.atc_state()
    );
}

/// Seconds of speech the voice needs for a recorded fleet call: the audio layer renders to the carrier's
/// slot, so this is what it plays.
fn fleet_voice_audio_s(e: &Transmission) -> f64 {
    let Meaning::Fleet { call } = &e.meaning else {
        unreachable!("fleet events only")
    };
    fleet_voice(call).duration_s(&e.text)
}

#[test]
fn transmissions_last_as_long_as_their_voice_and_audio_is_bounded_by_the_carrier() {
    let mut s = busy_session();
    advance(&mut s, 900.);
    let events = fleet_events(&s);
    assert!(events.len() > 20, "{} events", events.len());
    for (e, c) in &events {
        let expected = fleet_call_duration(&e.text, c);
        assert!(
            (e.duration_s - expected).abs() < 1e-9,
            "{} vs {expected}",
            e.duration_s
        );
    }
    let mut khz: Vec<u32> = events.iter().map(|(_, c)| c.frequency_khz).collect();
    khz.sort();
    khz.dedup();
    for k in khz {
        let on: Vec<&Transmission> = events
            .iter()
            .filter(|(_, c)| c.frequency_khz == k)
            .map(|(e, _)| e)
            .collect();
        for pair in on.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            // No carrier overlap and the engine's gap. Audio plays while the call is on the air, so its
            // length is bounded by the carrier: the next call never starts before this one's audio ends.
            assert!(b.time_s >= a.time_s + a.duration_s + 1.4, "{k}: {a:?} {b:?}");
            let audio_s = fleet_voice_audio_s(a);
            assert!(
                audio_s <= a.duration_s + 1e-9,
                "audio outlasts carrier on {k}: {a:?}"
            );
        }
    }
}

#[test]
fn a_call_the_user_keys_over_is_blocked_and_sent_again() {
    let mut s = busy_session();
    // Run until a fleet call is on the air on a frequency the user is listening to.
    let mut stepped = None;
    for _ in 0..4000 {
        advance_chunk(&mut s, 0.25);
        stepped = s
            .flight_record()
            .events
            .iter()
            .find(|e| e.reception == Reception::Pending && matches!(e.meaning, Meaning::Fleet { .. }))
            .cloned();
        if stepped.is_some() {
            break;
        }
    }
    let target = stepped.expect("a fleet call is on the air");
    let Meaning::Fleet { call } = &target.meaning else {
        panic!("a fleet call")
    };
    let khz = call.frequency_khz;
    let radio = if s.comms_state().com1.active_khz == khz {
        1
    } else {
        2
    };
    s.execute("comms.audio_panel", json!({"transmit":radio})).unwrap();
    s.execute("comms.transmit", json!({"text":"Approach, say again"}))
        .unwrap();
    advance(&mut s, 60.);
    let record = s.flight_record();
    let first = record.events.iter().find(|e| e.id == target.id).unwrap();
    assert_eq!(first.reception, Reception::Blocked, "{first:?}");
    assert_eq!(s.fleet_state().stats.stepped_on, 1);
    // The same call comes round again, introduced as a repeat, and is heard.
    let again = record.events.iter().find(|e| {
        e.id > target.id
            && e.reception == Reception::Heard
            && matches!(&e.meaning, Meaning::Fleet { call: c } if c.exchange == call.exchange && c.kind == call.kind)
            && e.text.starts_with("Blocked, ")
            && e.text.ends_with(&target.text)
    });
    assert!(
        again.is_some(),
        "no repeat of {:?}: {:#?}",
        target.text,
        record
            .events
            .iter()
            .filter(|e| e.id >= target.id)
            .take(6)
            .collect::<Vec<_>>()
    );
}

// ------------------------------------------------------------------------------------------------
// Traffic advisories and radio discipline
// ------------------------------------------------------------------------------------------------

fn wind() -> serde_json::Value {
    json!({"wind_from_deg":300.,"wind_speed_mps":6.,"shear":false,"turbulence":"off","seed":11})
}
fn step(s: &mut Session) {
    s.run(crate::Step { steps: 1 }).unwrap();
}
/// Step one physics frame at a time until `done`, up to `limit_s` simulated seconds.
fn step_until(s: &mut Session, limit_s: f64, mut done: impl FnMut(&Session) -> bool) -> bool {
    let end = s.sim().state().time_s + limit_s;
    while s.sim().state().time_s < end {
        step(s);
        if done(s) {
            return true;
        }
    }
    false
}
fn advisory_events(s: &Session) -> Vec<Transmission> {
    s.flight_record()
        .events
        .iter()
        .filter(|e| matches!(e.meaning, Meaning::TrafficAdvisory { .. }))
        .cloned()
        .collect()
}
fn follow_ups(s: &Session) -> Vec<(Transmission, TrafficFollowUpKind)> {
    s.flight_record()
        .events
        .iter()
        .filter_map(|e| match e.meaning {
            Meaning::TrafficFollowUp { kind, .. } => Some((e.clone(), kind)),
            _ => None,
        })
        .collect()
}
fn grades(s: &Session, item: &str) -> Vec<ComplianceGrade> {
    s.atc_state()
        .compliance
        .into_iter()
        .filter(|g| g.item == item)
        .collect()
}
fn approach_high(s: &Session) -> ComStation {
    s.stations()
        .into_iter()
        .find(|x| {
            x.airport == "KABE"
                && x.kind == FacilityKind::Approach
                && x.metadata.as_ref().is_some_and(|m| m.sector.contains("ABV"))
        })
        .unwrap()
}
/// Fly the user in the open air 3.5 NM behind a cruising aircraft and 300 ft below it, closing, in
/// radar contact with approach high. A 6 m/s wind makes the ground track differ from the heading.
fn user_behind_traffic(s: &mut Session) -> u32 {
    let state = s.fleet_state();
    let target = state
        .aircraft
        .iter()
        .filter(|a| !a.on_ground && a.ground_speed_kt >= 90. && a.altitude_msl_ft > 1500.)
        .min_by(|a, b| a.range_nm.total_cmp(&b.range_nm))
        .unwrap()
        .clone();
    let gnt = s.sim().heading_reference().grid_north_true_rad;
    let track_grid = (target.track_true_deg.to_radians() - gnt).rem_euclid(core::f64::consts::TAU);
    let back = track_grid + core::f64::consts::PI;
    let [tn, te, _] = target.position_m;
    let altitude_m = target.altitude_msl_ft * 0.3048 - 90.;
    let mut pose = *s.sim().state();
    pose.position_m = [
        tn + 3.5 * 1852. * back.cos(),
        te + 3.5 * 1852. * back.sin(),
        -altitude_m,
    ];
    pose.attitude = [(track_grid / 2.).cos(), 0., 0., (track_grid / 2.).sin()];
    let speed = (target.ground_speed_kt + 25.) * 0.514_444;
    pose.velocity_mps = [speed * track_grid.cos(), speed * track_grid.sin(), 0.];
    *s.sim_mut() =
        flightcraft_sim::Sim::from_state_in_world(pose, s.sim().controls(), s.sim().world().clone()).unwrap();
    s.execute("weather.set", wind()).unwrap();
    let heading = s.sim().readout().heading_rad;
    s.execute(
        "assist.set",
        json!({"altitude_m":altitude_m,"airspeed_mps":speed,"heading_rad":heading}),
    )
    .unwrap();
    let hi = approach_high(s);
    s.comms.atc.radar_facility = Some(crate::comms::atc::radar_facility(&hi));
    target.id
}
fn advisory_session() -> Session {
    let mut s = atc_session("KABE");
    listen_to_approach(&mut s);
    s.execute("weather.set", wind()).unwrap();
    s.execute("traffic.density", json!({"density":"busy","airport":"KABE"}))
        .unwrap();
    advance(&mut s, 2.);
    user_behind_traffic(&mut s);
    s
}
/// A session stepped until the first advisory has been heard to its end.
fn heard_advisory() -> (Session, Transmission) {
    let mut s = advisory_session();
    assert!(
        step_until(&mut s, 240., |s| {
            advisory_events(s)
                .first()
                .is_some_and(|e| e.reception == Reception::Heard)
        }),
        "no advisory\n{}",
        transcript(&s).join("\n")
    );
    let event = advisory_events(&s).remove(0);
    (s, event)
}
fn say_as_user(s: &mut Session, text: &str) {
    s.execute("comms.transmit", json!({"text":text})).unwrap();
}

#[test]
fn the_spoken_advisory_matches_traffic_state_at_that_instant() {
    let mut s = advisory_session();
    let mut checked = 0;
    let mut seen = 0;
    let end = s.sim().state().time_s + 240.;
    while s.sim().state().time_s < end && checked < 3 {
        step(&mut s);
        let events = advisory_events(&s);
        if events.len() == seen {
            continue;
        }
        seen = events.len();
        let event = events.last().unwrap();
        // Recompute from the literal `traffic.state` output and the user's own motion.
        let state: FleetState =
            serde_json::from_value(s.execute("traffic.state", json!({})).unwrap()).unwrap();
        let Meaning::TrafficAdvisory {
            clock,
            miles,
            miles_spoken,
            altitude_ft,
            relative_altitude_ft,
            target_aircraft,
            user_track_true_deg,
            direction,
            ..
        } = &event.meaning
        else {
            unreachable!()
        };
        assert_eq!(event.time_s, s.sim().state().time_s, "event is from this instant");
        let a = state
            .aircraft
            .iter()
            .find(|a| Some(&a.id) == target_aircraft.first())
            .unwrap();
        let [vn, ve, _] = s.sim().state().velocity_mps;
        let gnt = s.sim().heading_reference().grid_north_true_rad;
        let track = (libm::atan2(ve, vn) + gnt).to_degrees().rem_euclid(360.);
        let heading_true = (s.sim().readout().heading_grid_rad + gnt)
            .to_degrees()
            .rem_euclid(360.);
        assert!(
            (track - user_track_true_deg).abs() < 1e-6,
            "{track} {user_track_true_deg}"
        );
        assert!(
            (track - heading_true).abs() > 1.,
            "the wind should make track ({track:.1}) differ from heading ({heading_true:.1})"
        );
        // Bearing and range from the reported positions alone.
        let [pn, pe, pd] = s.sim().state().position_m;
        let [tn, te, _] = a.position_m;
        let bearing = (libm::atan2(te - pe, tn - pn) + gnt)
            .to_degrees()
            .rem_euclid(360.);
        let range = libm::hypot(tn - pn, te - pe) / 1852.;
        assert!((range - miles).abs() < 1e-9 && (range - a.range_nm).abs() < 1e-9);
        let mut expect = ((bearing - track).rem_euclid(360.) / 30.).round() as u8 % 12;
        if expect == 0 {
            expect = 12;
        }
        assert_eq!(
            *clock, expect,
            "clock from ground track, bearing {bearing:.1} track {track:.1}"
        );
        assert_eq!(*miles_spoken, range.round().max(1.) as u32);
        assert_eq!(*altitude_ft, a.altitude_msl_ft.round() as u32);
        assert_eq!(
            *relative_altitude_ft,
            (a.altitude_msl_ft + pd / 0.3048).round() as i32
        );
        // The words say the same numbers.
        let say_clock = phraseology::clock_position(expect);
        let say_miles = phraseology::advisory_miles(*miles_spoken);
        let say_alt = phraseology::altitude((a.altitude_msl_ft / 100.).round() as u32 * 100);
        assert!(
            event.text.contains("Traffic")
                && event
                    .text
                    .contains(&format!("{say_clock}, {say_miles}, {direction}")),
            "{}",
            event.text
        );
        assert!(event.text.contains(&say_alt), "{} / {say_alt}", event.text);
        assert!(event.text.contains(&a.aircraft_type), "{}", event.text);
        assert!(
            a.range_nm <= 10.5 && a.relative_altitude_ft.abs() <= 1000.,
            "{a:?}"
        );
        println!("{:.0}s {} | {}", event.time_s, event.speaker_name, event.text);
        checked += 1;
    }
    assert!(
        checked >= 1,
        "no advisory in four minutes\n{}",
        transcript(&s).join("\n")
    );
}

#[test]
fn advisories_never_overlap_the_traffic_on_the_frequency() {
    let (mut s, _) = heard_advisory();
    advance(&mut s, 120.);
    let mut spans: Vec<(f64, f64, u32)> = s
        .flight_record()
        .events
        .iter()
        .filter(|e| e.channel == Channel::Com && e.reception == Reception::Heard)
        .filter_map(|e| {
            Some((
                e.time_s,
                e.time_s + e.duration_s,
                (e.frequency_hz? * 3.).round() as u32,
            ))
        })
        .collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (i, a) in spans.iter().enumerate() {
        for b in spans.iter().skip(i + 1).filter(|b| b.2 == a.2) {
            assert!(b.0 >= a.1 - 1e-6, "overlap {a:?} {b:?}");
        }
    }
}

#[test]
fn traffic_in_sight_is_answered_with_a_visual_separation_or_sequencing_call() {
    let (mut s, event) = heard_advisory();
    let Meaning::TrafficAdvisory { callsign, .. } = &event.meaning else {
        unreachable!()
    };
    let before = follow_ups(&s).len();
    say_as_user(&mut s, "{callsign}, traffic in sight");
    assert!(
        step_until(&mut s, 40., |s| follow_ups(s).len() > before),
        "no follow-up\n{}",
        transcript(&s).join("\n")
    );
    let (reply, kind) = follow_ups(&s).remove(0);
    assert!(
        matches!(
            kind,
            TrafficFollowUpKind::VisualSeparation | TrafficFollowUpKind::Follow | TrafficFollowUpKind::Roger
        ),
        "{kind:?}"
    );
    assert!(
        reply.text.contains("visual separation")
            || reply.text.contains("Follow that traffic")
            || reply.text.contains("Roger"),
        "{}",
        reply.text
    );
    let ok = grades(&s, "traffic_advisory");
    assert!(ok.len() == 1 && ok[0].passed, "{ok:?} for {callsign}");
    let tally = s.radio_discipline();
    assert_eq!((tally.advisories_answered, tally.faults()), (1, 0), "{tally:?}");
    // The aircraft is sighted: never called again.
    advance(&mut s, 60.);
    assert!(advisory_events(&s).iter().all(|e| {
        !matches!(&e.meaning, Meaning::TrafficAdvisory { callsign: c, updated: true, .. } if c == callsign)
    }));
}

#[test]
fn negative_contact_brings_an_update_or_traffic_no_factor() {
    let (mut s, event) = heard_advisory();
    let Meaning::TrafficAdvisory { target_aircraft, .. } = event.meaning.clone() else {
        unreachable!()
    };
    let (n_adv, n_fu) = (advisory_events(&s).len(), follow_ups(&s).len());
    say_as_user(&mut s, "{callsign}, negative contact");
    assert!(
        step_until(&mut s, 40., |s| advisory_events(s).len() > n_adv
            || follow_ups(s).len() > n_fu),
        "no answer\n{}",
        transcript(&s).join("\n")
    );
    if advisory_events(&s).len() > n_adv {
        let update = advisory_events(&s).pop().unwrap();
        let Meaning::TrafficAdvisory {
            updated,
            target_aircraft: t,
            ..
        } = &update.meaning
        else {
            unreachable!()
        };
        assert!(*updated && *t == target_aircraft);
        assert!(update.text.contains("Traffic now"), "{}", update.text);
    } else {
        let (reply, kind) = follow_ups(&s).pop().unwrap();
        assert_eq!(kind, TrafficFollowUpKind::NoFactor);
        assert!(reply.text.contains("Traffic no factor"), "{}", reply.text);
    }
    let ok = grades(&s, "traffic_advisory");
    assert!(ok.len() == 1 && ok[0].passed, "{ok:?}");
    assert_eq!(s.radio_discipline().faults(), 0);
}

#[test]
fn looking_is_accepted_without_a_follow_up_call() {
    let (mut s, _) = heard_advisory();
    let before = follow_ups(&s).len();
    say_as_user(&mut s, "{callsign}, looking for the traffic");
    advance(&mut s, 12.);
    assert_eq!(follow_ups(&s).len(), before);
    let ok = grades(&s, "traffic_advisory");
    assert!(ok.len() == 1 && ok[0].passed, "{ok:?}");
    assert_eq!(s.radio_discipline().advisories_answered, 1);
}

#[test]
fn a_missed_call_is_repeated_then_graded() {
    let (mut s, first) = heard_advisory();
    let Meaning::TrafficAdvisory { target_aircraft, .. } = first.meaning.clone() else {
        unreachable!()
    };
    // Say nothing. After the answer window the same aircraft is called again.
    assert!(
        step_until(&mut s, 60., |s| advisory_events(s).len() >= 2),
        "no repeat\n{}",
        transcript(&s).join("\n")
    );
    let repeat = advisory_events(&s).remove(1);
    let Meaning::TrafficAdvisory {
        repeat: is_repeat,
        target_aircraft: t,
        ..
    } = &repeat.meaning
    else {
        unreachable!()
    };
    assert!(*is_repeat && *t == target_aircraft);
    assert!(
        repeat.time_s - (first.time_s + first.duration_s) >= crate::comms::fleet::advisory::RESPONSE_S,
        "repeated too soon"
    );
    assert!(
        grades(&s, "radio_discipline:no_response").is_empty(),
        "graded before the repeat"
    );
    // Still silent: now it is a fault, and the instructor says so.
    assert!(
        step_until(&mut s, 60., |s| !grades(s, "radio_discipline:no_response")
            .is_empty()),
        "never graded"
    );
    let fault = &grades(&s, "radio_discipline:no_response")[0];
    assert!(
        !fault.passed && fault.observed.contains("did not answer"),
        "{fault:?}"
    );
    let tally = s.radio_discipline();
    assert_eq!(
        (tally.repeats, tally.no_response, tally.advisories_answered),
        (1, 1, 0),
        "{tally:?}"
    );
    // The aircraft is closed: no endless nagging.
    advance(&mut s, 60.);
    assert!(advisory_events(&s).iter().all(|e| {
        e.time_s <= repeat.time_s + 1.
            || !matches!(&e.meaning, Meaning::TrafficAdvisory { target_aircraft: t, .. } if *t == target_aircraft)
    }));
}

#[test]
fn a_late_reply_inside_the_window_answers_the_call_without_a_fault() {
    let (mut s, first) = heard_advisory();
    let end = first.time_s + first.duration_s;
    let hi = carrier_thirds_hz(approach_high(&s).channel_khz);
    // Wait out most of the window, then key the microphone on a quiet carrier.
    assert!(step_until(&mut s, 30., |s| {
        let now = s.sim().state().time_s;
        now >= end + 6. && !s.comms.pending.iter().any(|p| p.carrier == hi && p.end_s > now)
    }));
    assert!(s.sim().state().time_s < end + 10.);
    say_as_user(&mut s, "{callsign}, traffic in sight");
    advance(&mut s, 30.);
    let Meaning::TrafficAdvisory { target_aircraft, .. } = first.meaning.clone() else {
        unreachable!()
    };
    assert!(
        advisory_events(&s).iter().all(|e| !matches!(
            &e.meaning,
            Meaning::TrafficAdvisory { repeat: true, target_aircraft: t, .. } if *t == target_aircraft
        )),
        "answered in time, so nothing is repeated"
    );
    assert_eq!(s.radio_discipline().faults(), 0);
    assert_eq!(s.radio_discipline().advisories_answered, 1);
}

#[test]
fn answering_with_another_aircrafts_callsign_is_a_fault() {
    let (mut s, first) = heard_advisory();
    let Meaning::TrafficAdvisory { target_aircraft, .. } = first.meaning.clone() else {
        unreachable!()
    };
    let other = s
        .fleet_state()
        .aircraft
        .into_iter()
        .find(|a| Some(&a.id) != target_aircraft.first())
        .unwrap();
    say_as_user(&mut s, &format!("{}, traffic in sight", other.callsign));
    advance(&mut s, 5.);
    let faults = grades(&s, "radio_discipline:wrong_callsign");
    assert_eq!(faults.len(), 1, "{:?}", s.atc_state().compliance);
    assert!(!faults[0].passed);
    assert!(
        faults[0]
            .observed
            .contains(&format!("for the {}", other.aircraft_type)),
        "{}",
        faults[0].observed
    );
    assert_eq!(s.radio_discipline().wrong_callsign, 1);
    // The wrong-callsign answer did not satisfy the advisory.
    assert_eq!(s.radio_discipline().advisories_answered, 0);
}

#[test]
fn reading_back_another_aircrafts_instruction_is_a_fault() {
    let mut s = busy_session();
    advance(&mut s, 5.);
    // Find an instruction to somebody else that the user heard.
    let hi = approach_high(&s);
    let mut spoken: Option<FleetCall> = None;
    assert!(step_until(&mut s, 600., |s| {
        spoken = fleet_events(s).into_iter().map(|e| e.1).rev().find(|c| {
            c.role == FleetRole::Controller
                && c.kind == FleetCallKind::Instruction
                && c.frequency_khz == hi.channel_khz
                && c.instructions
                    .iter()
                    .any(|i| matches!(i, AtcInstruction::Altitude { .. }))
        });
        spoken.is_some()
    }));
    let call = spoken.unwrap();
    let altitude = call
        .instructions
        .iter()
        .find(|i| matches!(i, AtcInstruction::Altitude { .. }))
        .unwrap();
    let text = s.atc_readback(altitude);
    say_as_user(&mut s, &text);
    advance(&mut s, 5.);
    let faults = grades(&s, "radio_discipline:acknowledged_other");
    assert_eq!(faults.len(), 1, "{text} -> {:?}", s.atc_state().compliance);
    assert!(
        faults[0].observed.contains("not for you"),
        "{}",
        faults[0].observed
    );
    assert_eq!(s.radio_discipline().acknowledged_other, 1);
}

#[test]
fn keying_over_traffic_is_a_fault() {
    let mut s = busy_session();
    advance(&mut s, 5.);
    let hi = carrier_thirds_hz(approach_high(&s).channel_khz);
    assert!(step_until(&mut s, 600., |s| {
        let now = s.sim().state().time_s;
        s.comms.pending.iter().any(|p| {
            p.carrier == hi
                && p.end_s > now + 1.
                && s.flight_record()
                    .events
                    .iter()
                    .any(|e| e.id == p.id && matches!(e.meaning, Meaning::Fleet { .. }))
        })
    }));
    say_as_user(&mut s, "{callsign}, request flight following");
    let faults = grades(&s, "radio_discipline:stepped_on");
    assert_eq!(faults.len(), 1, "{:?}", s.atc_state().compliance);
    assert!(
        faults[0].observed.contains("was talking"),
        "{}",
        faults[0].observed
    );
    assert_eq!(s.radio_discipline().stepped_on, 1);
}

#[test]
fn a_clean_listener_collects_no_faults_in_a_busy_quarter_hour() {
    let mut s = busy_session();
    advance(&mut s, 900.);
    assert_eq!(s.radio_discipline().faults(), 0);
    assert!(
        s.atc_state()
            .compliance
            .iter()
            .all(|g| !g.item.starts_with("radio_discipline:"))
    );
}

#[test]
fn traffic_the_user_already_follows_is_not_advised_again() {
    let mut s = advisory_session();
    // Every aircraft is "followed": the sequencing call is all the controller says.
    for a in s.fleet_state().aircraft {
        s.comms.atc.active.push(PendingInstruction {
            id: 9000 + u64::from(a.id),
            station_id: "KABE-TWR".into(),
            airport: "KABE".into(),
            instruction: AtcInstruction::Pattern {
                runway: "06".into(),
                entry: PatternEntry::StraightIn,
                number: 2,
                follow: Some(a.callsign.clone()),
            },
            phrase: "follow".into(),
            heard: true,
            readback: true,
            issued_s: 0.,
            reminder_s: f64::MAX,
        });
    }
    advance(&mut s, 90.);
    assert!(advisory_events(&s).is_empty());
}

#[test]
fn off_the_frequency_or_out_of_radar_contact_gets_no_advisory() {
    let mut s = advisory_session();
    s.comms.atc.radar_facility = None;
    advance(&mut s, 90.);
    assert!(advisory_events(&s).is_empty());
    s.comms.atc.radar_facility = Some(crate::comms::atc::radar_facility(&approach_high(&s)));
    // Transmitting on a different frequency than the approach controller's.
    s.execute("comms.tune", json!({"radio":1,"channel_khz":122800}))
        .unwrap();
    advance(&mut s, 90.);
    assert!(advisory_events(&s).is_empty());
}

#[test]
fn clock_position_is_measured_from_the_ground_track() {
    use super::advisory::{UserMotion, assess, clock_of};
    // A target due east of a user whose heading is 090 but whose track is 120 (a left-to-right wind).
    assert_eq!(clock_of(90., 120.), 11);
    assert_eq!(clock_of(90., 90.), 12);
    assert_eq!(clock_of(120., 90.), 1);
    assert_eq!(clock_of(180., 90.), 3);
    assert_eq!(clock_of(0., 90.), 9);
    assert_eq!(clock_of(270., 90.), 6);
    assert_eq!(clock_of(5., 355.), 12);
    let user = UserMotion {
        track_true_deg: 90.,
        speed_kt: 100.,
        has_track: true,
        vs_fpm: 0.,
    };
    let mut a = FleetAircraft {
        id: 7,
        callsign: "Cherokee Four Two Seven Kilo Papa".into(),
        registration: "N427KP".into(),
        aircraft_type: "Cherokee".into(),
        type_designator: "PA28".into(),
        latitude_deg: None,
        longitude_deg: None,
        altitude_msl_ft: 3480.,
        position_m: [0.; 3],
        track_true_deg: 270.,
        track_magnetic_deg: 275.,
        ground_speed_kt: 110.,
        vertical_rate_fpm: 0.,
        on_ground: false,
        squawk: "1200".into(),
        frequency_khz: 0,
        facility_id: String::new(),
        facility: String::new(),
        intent: FleetIntent {
            kind: FleetFlight::FlightFollowing,
            phase: String::new(),
            airport: "KABE".into(),
            runway: "06".into(),
        },
        assigned_altitude_ft: None,
        assigned_heading_deg: None,
        assigned_speed_kt: None,
        cleared_to_land_or_approach: false,
        airspace_class: None,
        voice: 0,
        range_nm: 3.4,
        bearing_true_deg: 60.,
        relative_altitude_ft: 480.,
        on_user_frequency: true,
    };
    let r = assess(&a, &user);
    assert_eq!(
        (r.clock, r.miles, r.direction.as_str()),
        (11, 3, "opposite direction")
    );
    assert_eq!(r.altitude_ft, 3480);
    assert!(r.factor, "inside 5 NM and 1,000 ft");
    let text = phraseology::traffic_advisory(
        r.clock,
        r.miles,
        &r.direction,
        Some(&r.spoken_type),
        r.altitude,
        false,
    );
    assert_eq!(
        text,
        "Traffic, eleven o'clock, three miles, opposite direction, Cherokee, three thousand five hundred."
    );
    // Altitude rounds to the nearest hundred, the distance to whole miles (never less than one).
    a.altitude_msl_ft = 3449.6;
    a.range_nm = 0.4;
    let r = assess(&a, &user);
    assert_eq!((r.miles, r.altitude_ft), (1, 3450));
    assert_eq!(
        phraseology::traffic_advisory(
            r.clock,
            r.miles,
            &r.direction,
            None,
            phraseology::AdvisoryAltitude::Indicates(3450),
            true
        ),
        "Traffic now eleven o'clock, one mile, opposite direction, altitude indicates three thousand five hundred."
    );
    // 1,500 ft above and 4 NM away is not a factor; 800 ft above and closing is.
    a.range_nm = 4.0;
    a.relative_altitude_ft = 1500.;
    assert!(!assess(&a, &user).factor);
    a.relative_altitude_ft = 800.;
    assert!(assess(&a, &user).factor);
    // Same direction, crossing either way.
    for (track, expect) in [
        (100., "same direction"),
        (180., "crossing left to right"),
        (0., "crossing right to left"),
        (90. + 170., "opposite direction"),
    ] {
        a.track_true_deg = track;
        assert_eq!(assess(&a, &user).direction, expect, "track {track}");
    }
    // Without a track the compass word stands in.
    assert_eq!(phraseology::traffic_direction(10., 95., false), "eastbound");
    assert_eq!(phraseology::traffic_direction(10., 225., false), "southwestbound");
    // Converging: 6 NM ahead on a reciprocal track, closing at 220 kt (98 s).
    a.range_nm = 6.;
    a.bearing_true_deg = 90.;
    a.track_true_deg = 270.;
    a.ground_speed_kt = 120.;
    a.relative_altitude_ft = 600.;
    let r = assess(&a, &user);
    assert!(r.factor && r.cpa_nm < 0.1, "{r:?}");
    // Climbing away through 1,000 ft above the user by the closest point: not converging on the user.
    a.vertical_rate_fpm = 4000.;
    assert!(!assess(&a, &user).factor);
    assert_eq!(
        phraseology::traffic_advisory(
            12,
            2,
            "same direction",
            Some("Mooney"),
            phraseology::AdvisoryAltitude::Unknown,
            false
        ),
        "Traffic, twelve o'clock, two miles, same direction, Mooney, altitude unknown."
    );
}

#[test]
fn pilot_replies_to_traffic_calls_parse() {
    let me = PilotCallsign {
        overridden: false,
        registration: "N371FC".into(),
        spoken_type: "Sling".into(),
        not_us: false,
        full: "Sling Three Seven One Foxtrot Charlie".into(),
        abbreviated: "Sling One Foxtrot Charlie".into(),
        current: "Sling One Foxtrot Charlie".into(),
    };
    for (text, expect) in [
        (
            "Sling one foxtrot charlie, traffic in sight",
            PilotRequest::TrafficInSight,
        ),
        (
            "Traffic in sight, Sling one foxtrot charlie",
            PilotRequest::TrafficInSight,
        ),
        (
            "sling one foxtrot charlie has the traffic in sight",
            PilotRequest::TrafficInSight,
        ),
        ("Sling one foxtrot charlie, looking", PilotRequest::TrafficLooking),
        (
            "Looking for traffic, Sling one foxtrot charlie",
            PilotRequest::TrafficLooking,
        ),
        (
            "Sling one foxtrot charlie, negative contact",
            PilotRequest::NegativeContact,
        ),
        (
            "Negative contact, Sling one foxtrot charlie",
            PilotRequest::NegativeContact,
        ),
        (
            "Sling one foxtrot charlie, traffic not in sight",
            PilotRequest::NegativeContact,
        ),
    ] {
        let m = parse_pilot(text, &me);
        assert!(m.callsign_present, "{text}");
        assert_eq!(m.request, Some(expect), "{text}");
    }
    assert_eq!(
        parse_pilot("Sling one foxtrot charlie, request flight following", &me).request,
        Some(PilotRequest::FlightFollowing)
    );
    // Words that sound alike but belong to other requests stay with them.
    assert_eq!(
        parse_pilot(
            "Sling one foxtrot charlie, airport in sight, request visual approach runway zero six",
            &me
        )
        .request,
        Some(PilotRequest::Approach)
    );
}

// ------------------------------------------------------------------------------------------------
// The "Busy frequency" lesson
// ------------------------------------------------------------------------------------------------

/// The lesson on the original flat-terrain fixture region (same airports, frequencies, procedures).
fn lesson_session() -> Session {
    let mut lesson = crate::lesson_library()
        .unwrap()
        .into_iter()
        .find(|l| l.id == "busy-frequency")
        .unwrap();
    lesson.setup.region = "atc-fixture".into();
    let mut s = Session::new().unwrap();
    s.install_region(atc_world()).unwrap();
    s.run(crate::LessonStart { lesson }).unwrap();
    let heading = s.sim().readout().heading_rad;
    let altitude_m = s.sim().readout().altitude_msl_m;
    s.execute(
        "assist.set",
        json!({"altitude_m":altitude_m,"airspeed_mps":90. * 0.514_444,"heading_rad":heading}),
    )
    .unwrap();
    s
}
/// Fly the lesson; the pilot answers each advisory with `reply` the moment it has been heard.
fn fly_lesson(reply: &dyn Fn(&Transmission) -> Option<String>, seconds: f64) -> (Session, Vec<String>) {
    let mut s = lesson_session();
    let mut answered = std::collections::BTreeSet::new();
    let end = s.sim().state().time_s + seconds;
    while s.sim().state().time_s < end && !s.lesson_debrief().tasks.iter().all(|t| t.completed) {
        step(&mut s);
        let heard: Vec<Transmission> = advisory_events(&s)
            .into_iter()
            .filter(|e| e.reception == Reception::Heard && !answered.contains(&e.id))
            .collect();
        for e in heard {
            answered.insert(e.id);
            if let Some(text) = reply(&e) {
                s.execute("comms.transmit", json!({"text":text})).unwrap();
            }
        }
    }
    let lines = s
        .flight_record()
        .events
        .iter()
        .filter(|e| e.channel == Channel::Com || matches!(e.meaning, Meaning::Compliance { .. }))
        .map(|e| format!("{:.2} {} | {}", e.time_s, e.speaker_name, e.text))
        .collect();
    (s, lines)
}

#[test]
fn the_busy_frequency_lesson_is_in_the_library_and_loads() {
    let lesson = crate::lesson_library()
        .unwrap()
        .into_iter()
        .find(|l| l.id == "busy-frequency")
        .unwrap();
    assert_eq!(lesson.setup.airport, "KABE");
    assert_eq!(lesson.setup.traffic.unwrap().density, TrafficDensity::Busy);
    assert_eq!(lesson.tasks.len(), 1);
    assert_eq!(lesson.tasks[0].kind, crate::InstrumentTaskKind::Radio);
    let s = lesson_session();
    assert_eq!(s.fleet_state().density, TrafficDensity::Busy);
    assert_eq!(s.fleet_state().airport.as_deref(), Some("KABE"));
    assert!(s.fleet_state().aircraft.len() >= 10);
    // COM1 is on the approach controller the user is in radar contact with.
    assert_eq!(
        s.comms_state().com1.active_khz,
        s.comms
            .fleet
            .as_ref()
            .unwrap()
            .field
            .st(s.comms.fleet.as_ref().unwrap().field.lo)
            .channel_khz
    );
    assert!(s.comms.atc.radar_facility.is_some());
    assert_eq!(s.lesson_debrief().tasks[0].result, crate::TaskResult::Pending);
}

#[test]
fn a_pilot_who_answers_every_call_passes_the_lesson_deterministically() {
    let reply = |e: &Transmission| {
        let Meaning::TrafficAdvisory { clock, .. } = &e.meaning else {
            return None;
        };
        Some(if *clock >= 9 || *clock <= 1 {
            "{callsign}, traffic in sight".to_string()
        } else {
            "{callsign}, negative contact".to_string()
        })
    };
    let (s, first) = fly_lesson(&reply, 900.);
    let debrief = s.lesson_debrief();
    assert!(
        debrief.tasks[0].completed && debrief.tasks[0].result == crate::TaskResult::Pass,
        "{:?}\n{}",
        debrief.tasks[0],
        first.join("\n")
    );
    assert!(s.radio_discipline().advisories_answered >= 2);
    assert_eq!(s.radio_discipline().faults(), 0);
    println!(
        "{}",
        first
            .iter()
            .filter(|l| l.contains("raffic"))
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    let (again, second) = fly_lesson(&reply, 900.);
    assert_eq!(first, second, "the lesson must replay identically");
    assert_eq!(
        serde_json::to_string(again.lesson_debrief()).unwrap(),
        serde_json::to_string(s.lesson_debrief()).unwrap()
    );
}

#[test]
fn a_pilot_who_answers_the_wrong_aircraft_fails_the_lesson() {
    let reply = |e: &Transmission| {
        let Meaning::TrafficAdvisory { callsign, .. } = &e.meaning else {
            return None;
        };
        // Answer as the aircraft that was called, not as ourselves.
        Some(format!("{callsign}, traffic in sight"))
    };
    let (s, lines) = fly_lesson(&reply, 400.);
    let task = &s.lesson_debrief().tasks[0];
    assert_eq!(task.result, crate::TaskResult::Fail, "{lines:?}");
    assert!(
        task.reasons.iter().any(|r| r.contains("not for you")),
        "{:?}",
        task.reasons
    );
    assert!(s.radio_discipline().wrong_callsign >= 1);
}

#[test]
fn a_pilot_who_stays_silent_fails_the_lesson_for_unanswered_calls() {
    let (s, lines) = fly_lesson(&|_| None, 400.);
    let task = &s.lesson_debrief().tasks[0];
    assert_eq!(task.result, crate::TaskResult::Fail, "{lines:?}");
    assert!(
        task.reasons.iter().any(|r| r.contains("did not answer")),
        "{:?}",
        task.reasons
    );
    assert!(s.radio_discipline().no_response >= 1);
}

// ---------------------------------------------------------------- several airports at once

// Seed 3 is one whose single-airport traffic already keeps the physics guard at zero for the whole
// run; some other weather seeds trip the guard at KABE alone, which is not what these tests probe.
fn two_airport_session(seed: u64) -> Session {
    let mut s = atc_session("KABE");
    listen_to_approach(&mut s);
    s.execute(
        "weather.set",
        json!({"wind_from_deg":0.,"wind_speed_mps":0.,"shear":false,"turbulence":"off","seed":seed}),
    )
    .unwrap();
    s.execute(
        "traffic.density",
        json!({"density":"busy","airport":"KABE","airports":["KMMU"]}),
    )
    .unwrap();
    s
}

#[test]
fn two_towered_airports_each_get_their_own_traffic_and_frequencies() {
    let s = two_airport_session(3);
    let state = s.fleet_state();
    let names: Vec<&str> = state.airports.iter().map(|a| a.airport.as_str()).collect();
    assert_eq!(names, ["KABE", "KMMU"]);
    assert_eq!(state.airport.as_deref(), Some("KABE"));
    assert!(
        state.airports.iter().all(|a| (8..=16).contains(&a.aircraft)),
        "{:?}",
        state.airports
    );
    assert_eq!(
        state.aircraft.len() as u32,
        state.airports.iter().map(|a| a.aircraft).sum::<u32>()
    );
    let mut ids: Vec<u32> = state.aircraft.iter().map(|a| a.id).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(
        ids.len(),
        state.aircraft.len(),
        "aircraft ids must be unique across airports"
    );
    for airport in ["KABE", "KMMU"] {
        let mine: Vec<_> = state
            .aircraft
            .iter()
            .filter(|a| a.intent.airport == airport)
            .collect();
        assert!(!mine.is_empty());
        for a in mine {
            // The controlling facility is one of this airport's own stations.
            let st = s
                .stations()
                .into_iter()
                .find(|st| st.id == a.facility_id && st.channel_khz == a.frequency_khz)
                .unwrap();
            let fleet = s.fleet_of(&st).unwrap();
            assert_eq!(fleet.field.airport, airport, "{} {}", a.callsign, a.facility);
        }
    }
    let freqs: std::collections::BTreeSet<u32> = state.frequencies.iter().map(|f| f.khz).collect();
    let kabe = s
        .fleets()
        .next()
        .unwrap()
        .field
        .stations
        .iter()
        .map(|x| x.channel_khz)
        .collect::<Vec<_>>();
    let kmmu = s
        .fleets()
        .nth(1)
        .unwrap()
        .field
        .stations
        .iter()
        .map(|x| x.channel_khz)
        .collect::<Vec<_>>();
    assert!(freqs.iter().any(|f| kabe.contains(f)) && freqs.iter().any(|f| kmmu.contains(f)));
    // The two airports have different towers.
    assert_ne!(
        s.fleets()
            .next()
            .unwrap()
            .field
            .st(s.fleets().next().unwrap().field.tower)
            .channel_khz,
        s.fleets()
            .nth(1)
            .unwrap()
            .field
            .st(s.fleets().nth(1).unwrap().field.tower)
            .channel_khz
    );
}

#[test]
fn transit_traffic_flies_between_the_airports() {
    let mut s = two_airport_session(3);
    let mut seen: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> = Default::default();
    for _ in 0..(3600 / 10) {
        advance(&mut s, 10.);
        for a in s.fleet_state().aircraft {
            seen.entry(a.callsign).or_default().insert(a.intent.airport);
        }
    }
    let crossed: Vec<_> = seen.iter().filter(|(_, v)| v.len() >= 2).collect();
    assert!(
        !crossed.is_empty(),
        "no transit flight changed airports in an hour"
    );
    let st = s.fleet_state();
    let notes: Vec<_> = s
        .fleets()
        .map(|f| {
            format!(
                "{}: {} ({} spawned)",
                f.field.airport, f.guard_note, f.stats.spawned
            )
        })
        .collect();
    assert_eq!(st.stats.safety_slowdowns, 0, "{notes:?}");
}

#[test]
fn several_airports_keep_separation_and_unique_callsigns() {
    let mut s = two_airport_session(3);
    for _ in 0..(1200 / 4) {
        advance(&mut s, 4.);
        let ac: Vec<_> = s.fleet_state().aircraft;
        let mut names: Vec<&str> = ac.iter().map(|a| a.callsign.as_str()).collect();
        names.sort();
        let n = names.len();
        names.dedup();
        assert_eq!(names.len(), n, "two aircraft share a callsign");
        let air: Vec<_> = ac.iter().filter(|a| !a.on_ground).collect();
        for (i, a) in air.iter().enumerate() {
            for b in &air[i + 1..] {
                let lateral = libm::hypot(
                    a.position_m[0] - b.position_m[0],
                    a.position_m[1] - b.position_m[1],
                ) / 1852.;
                let vertical = (a.altitude_msl_ft - b.altitude_msl_ft).abs();
                assert!(
                    !(lateral < 1.0 && vertical < 500.),
                    "{} ({}) and {} ({}) within {lateral:.2} NM, {vertical:.0} ft at {:.0}s",
                    a.callsign,
                    a.intent.airport,
                    b.callsign,
                    b.intent.airport,
                    s.sim().state().time_s
                );
            }
        }
    }
    let notes: Vec<_> = s
        .fleets()
        .map(|f| format!("{}: {}", f.field.airport, f.guard_note))
        .collect();
    assert_eq!(s.fleet_state().stats.safety_slowdowns, 0, "{notes:?}");
}

#[test]
fn several_airports_are_deterministic_per_seed() {
    let run = |seed: u64| {
        let mut s = two_airport_session(seed);
        advance(&mut s, 600.);
        let positions: Vec<(String, u64, u64)> = s
            .fleet_state()
            .aircraft
            .iter()
            .map(|a| {
                (
                    a.callsign.clone(),
                    a.position_m[0].to_bits(),
                    a.position_m[1].to_bits(),
                )
            })
            .collect();
        (transcript(&s), positions)
    };
    let a = run(7);
    assert!(a.0.len() > 10);
    assert_eq!(a, run(7));
    assert_ne!(a.1, run(8).1);
}

#[test]
fn a_single_airport_is_unchanged_and_off_clears_every_airport() {
    // One airport behaves exactly as before: no transit flights, one entry in the list.
    let mut s = atc_session("KABE");
    s.execute("traffic.density", json!({"density":"busy","airport":"KABE"}))
        .unwrap();
    advance(&mut s, 300.);
    let st = s.fleet_state();
    assert_eq!(st.airports.len(), 1);
    assert!(s.fleets().all(|f| f.craft.iter().all(|a| a.transit.is_none())));
    let mut s = two_airport_session(1);
    assert_eq!(s.fleet_state().airports.len(), 2);
    s.execute("traffic.density", json!({"density":"off"})).unwrap();
    let st = s.fleet_state();
    assert!(st.aircraft.is_empty() && st.airports.is_empty());
    assert_eq!(s.fleets().count(), 0);
    // Too many airports are refused.
    let err = s.execute(
        "traffic.density",
        json!({"density":"light","airport":"KABE","airports":["KMMU","KEWR","KABE","KMMU"]}),
    );
    assert!(err.is_err());
}

#[test]
fn the_user_hears_only_the_airport_in_range_but_both_keep_talking() {
    let mut s = two_airport_session(2);
    advance(&mut s, 900.);
    let kabe: Vec<u32> = s
        .fleets()
        .next()
        .unwrap()
        .field
        .stations
        .iter()
        .map(|x| x.channel_khz)
        .collect();
    let events = fleet_events(&s);
    assert!(!events.is_empty());
    // KMMU is 45 NM from the parked user: its calls happen but fall outside the radios' reach.
    assert!(
        events.iter().all(|(_, c)| kabe.contains(&c.frequency_khz)),
        "{}",
        transcript(&s).join("\n")
    );
    let state = s.fleet_state();
    let heard = events.len() as u64;
    assert!(
        state.stats.transmissions > heard * 3 / 2,
        "{} vs {heard}",
        state.stats.transmissions
    );
}

fn seeded_session(seed: u64, airports: &[&str]) -> Session {
    let mut s = atc_session("KABE");
    listen_to_approach(&mut s);
    s.execute(
        "weather.set",
        json!({"wind_from_deg":0.,"wind_speed_mps":0.,"shear":false,"turbulence":"off","seed":seed}),
    )
    .unwrap();
    s.execute(
        "traffic.density",
        json!({"density":"busy","airport":"KABE","airports":airports}),
    )
    .unwrap();
    s
}

/// Half an hour of Busy traffic under one seed. Returns the closest approach (NM lateral between
/// aircraft with under 500 ft of vertical separation, with who and when) and the physics-guard
/// report ("" when the guard was never used).
fn separation_run(seed: u64, airports: &[&str]) -> (f64, String, String) {
    let mut s = seeded_session(seed, airports);
    let mut closest = (f64::MAX, String::new());
    for _ in 0..(1800 / 2) {
        advance(&mut s, 2.);
        let ac: Vec<_> = s
            .fleet_state()
            .aircraft
            .into_iter()
            .filter(|a| !a.on_ground)
            .collect();
        for (i, a) in ac.iter().enumerate() {
            for b in &ac[i + 1..] {
                let vertical = (a.altitude_msl_ft - b.altitude_msl_ft).abs();
                let lateral = libm::hypot(
                    a.position_m[0] - b.position_m[0],
                    a.position_m[1] - b.position_m[1],
                ) / 1852.;
                if vertical < 500. && lateral < closest.0 {
                    closest = (
                        lateral,
                        format!(
                            "{} ({}) / {} ({}) at {:.0}s, {vertical:.0} ft",
                            a.callsign,
                            a.intent.phase,
                            b.callsign,
                            b.intent.phase,
                            s.sim().state().time_s
                        ),
                    );
                }
            }
        }
    }
    let report: Vec<String> = s
        .fleets()
        .filter(|f| !f.guard_pairs.is_empty())
        .map(|f| format!("{}: {:?} first: {}", f.field.airport, f.guard_pairs, f.guard_note))
        .collect();
    (closest.0, closest.1, report.join(" | "))
}

/// Sequencing keeps the aircraft apart on its own: for every weather seed the physics guard never
/// acts, and no two aircraft come within 1 NM with under 500 ft of vertical separation.
fn sweep(airports: &[&str], seeds: std::ops::RangeInclusive<u64>) {
    let mut failures = Vec::new();
    let mut closest = f64::MAX;
    for seed in seeds {
        let (c, who, report) = separation_run(seed, airports);
        closest = closest.min(c);
        if !report.is_empty() {
            failures.push(format!("seed {seed}: physics guard used: {report}"));
        }
        if c < 1.0 {
            failures.push(format!("seed {seed}: {c:.2} NM: {who}"));
        }
    }
    println!("{airports:?}: closest approach {closest:.2} NM");
    assert!(failures.is_empty(), "{airports:?}\n{}", failures.join("\n"));
}

/// Two seeds that tripped the guard before the sequencing fix, kept in the default suite.
#[test]
fn busy_kabe_seeds_that_once_needed_the_guard_no_longer_do() {
    sweep(&[], 1..=1);
    sweep(&[], 4..=4);
}

#[test]
#[ignore = "full seed sweep: run with --ignored (about 3.5 min each in release)"]
fn busy_kabe_never_need_the_physics_guard_across_seeds_0_to_10() {
    sweep(&[], 0..=10);
}

#[test]
#[ignore = "full seed sweep: run with --ignored (about 3.5 min each in release)"]
fn busy_kabe_never_need_the_physics_guard_across_seeds_11_to_20() {
    sweep(&[], 11..=20);
}

#[test]
#[ignore = "full seed sweep: run with --ignored (about 3.5 min each in release)"]
fn busy_kabe_and_kmmu_never_need_the_physics_guard_across_seeds_0_to_10() {
    sweep(&["KMMU"], 0..=10);
}

#[test]
#[ignore = "full seed sweep: run with --ignored (about 3.5 min each in release)"]
fn busy_kabe_and_kmmu_never_need_the_physics_guard_across_seeds_11_to_20() {
    sweep(&["KMMU"], 11..=20);
}
