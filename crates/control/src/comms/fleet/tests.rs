#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
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

#[test]
fn transmissions_last_as_long_as_their_voice_and_audio_never_overlaps() {
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
            // No carrier overlap, the engine's gap, and the next call ends after the previous audio does.
            assert!(b.time_s >= a.time_s + a.duration_s + 1.4, "{k}: {a:?} {b:?}");
            assert!(
                b.time_s + b.duration_s + 1e-6 >= a.time_s + 2.0 * a.duration_s,
                "audio overlap on {k}: {:?} then {:?}",
                a.text,
                b.text
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
    assert!(again.is_some(), "no repeat of {:?}", target.text);
}
