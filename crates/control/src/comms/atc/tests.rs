#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use super::*;
use crate as flightcraft_control;
use serde_json::json;
#[path = "flown.rs"]
mod flown;
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/atc_fixture.rs"));
fn advance(s: &mut Session, seconds: f64) {
    s.run(crate::Step {
        steps: (seconds * 120.).ceil() as u32,
    })
    .unwrap();
}
fn tune(s: &mut Session, airport: &str, kind: FacilityKind) -> ComStation {
    let st = s.next_station(airport, kind).unwrap();
    s.execute("comms.tune", json!({"radio":1,"channel_khz":st.channel_khz}))
        .unwrap();
    st
}
fn settle(s: &mut Session) {
    for _ in 0..240 {
        if s.comms.pending.iter().all(|p| p.channel != Channel::Com) && s.comms.atc.replies.is_empty() {
            return;
        }
        advance(s, 0.25);
    }
    assert!(
        s.comms.atc.replies.is_empty(),
        "queued replies {:?}",
        s.comms.atc.replies
    );
}
fn say(s: &mut Session, text: &str) {
    settle(s);
    let text = format!("{{callsign}}, {text}");
    s.execute("comms.transmit", json!({"text":text})).unwrap();
    settle(s);
}
fn instruction(s: &Session) -> PendingInstruction {
    s.atc_state()
        .pending_instructions
        .first()
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "No instruction; state {:?}; recent events {:?}",
                s.atc_state(),
                s.flight_record().events.iter().rev().take(5).collect::<Vec<_>>()
            )
        })
}
fn readback(s: &mut Session) {
    settle(s);
    let p = instruction(s);
    let text = s.atc_readback(&p.instruction);
    s.execute("comms.transmit", json!({"text":text})).unwrap();
    settle(s);
    assert!(
        !s.comms.atc.pending.iter().any(|v| v.id == p.id),
        "not accepted: {:?}",
        s.atc_state()
    );
}
fn initial(s: &mut Session, airport: &str, kind: FacilityKind, request: &str) {
    tune(s, airport, kind);
    let info = s.atis_report(airport).unwrap().information;
    say(s, &format!("with information {info}, {request}"));
}
#[test]
fn spoken_grammar_covers_callsigns_frequencies_runways_and_numbers() {
    let s = atc_session("KABE");
    let m = parse_pilot(
        "Allentown approach, Sling One Two Three Four Bravo, with information A, runway zero six, heading two seven zero, maintain three thousand five hundred, squawk zero four five six, contact ground one two one point niner zero",
        &s.pilot_callsign(),
    );
    assert!(m.callsign_present);
    assert_eq!(m.runway.as_deref(), Some("06"));
    assert_eq!(m.heading_deg, Some(270));
    assert_eq!(m.altitude_ft, Some(3500));
    assert_eq!(m.squawk.as_deref(), Some("0456"));
    assert_eq!(m.channel_khz, Some(121900));
    assert_eq!(m.information.as_deref(), Some("alpha"));
    for frequency in ["123.45", "one two three point four five", "123450"] {
        assert_eq!(
            parse_pilot(frequency, &s.pilot_callsign()).channel_khz,
            Some(123450)
        );
    }
    assert_eq!(
        parse_pilot("heading 999999999999999999999999999999", &s.pilot_callsign()).heading_deg,
        None
    );
    assert_eq!(
        parse_pilot("runway 04L", &s.pilot_callsign()).runway.as_deref(),
        Some("04L")
    );
}
#[test]
fn atis_uses_nasr_weather_runways_and_rotates_without_query_side_effects() {
    let mut s = atc_session("KABE");
    let report = s.atis_report("KABE").unwrap();
    assert_eq!(report.channel_khz, 126975);
    assert_eq!(report.runway, "06");
    assert_eq!(report.information, "alpha");
    for _ in 0..10 {
        assert_eq!(s.atc_state().atis.unwrap(), report);
    }
    s.execute(
        "weather.set",
        json!({"wind_from_deg":240.,"wind_speed_mps":8.,"shear":false,"turbulence":"off","seed":0}),
    )
    .unwrap();
    advance(&mut s, 1.);
    let newer = s.atis_report("KABE").unwrap();
    assert_eq!(newer.runway, "24");
    assert_ne!(newer.information, report.information);
    tune(&mut s, "KABE", FacilityKind::Ground);
    say(&mut s, "request taxi");
    assert!(s.comms.atc.pending.is_empty());
    assert!(
        s.flight_record()
            .events
            .iter()
            .any(|e| e.text.contains("Verify you have information"))
    );
    say(&mut s, "with information alpha, request taxi");
    assert!(s.comms.atc.pending.is_empty());
    initial(&mut s, "KABE", FacilityKind::Ground, "request taxi");
    assert!(matches!(instruction(&s).instruction, AtcInstruction::Taxi { .. }));
}
fn departure(s: &mut Session, airport: &str) {
    tune(s, airport, FacilityKind::Atis);
    advance(s, 1.);
    settle(s);
    assert!(
        s.flight_record()
            .events
            .iter()
            .any(|e| matches!(e.meaning, Meaning::Atis { .. }) && e.reception == Reception::Heard),
        "ATIS not heard at {airport}: readout {:?}, powered {}, channel {}, events {:?}",
        s.sim.readout(),
        s.comms_state().powered,
        s.comms_state().com1.active_khz,
        s.flight_record().events
    );
    initial(s, airport, FacilityKind::Clearance, "request IFR clearance");
    let AtcInstruction::IfrClearance { code, .. } = instruction(s).instruction else {
        panic!("expected CRAFT clearance")
    };
    readback(s);
    s.execute(
        "avionics.set",
        json!({"setting":{"kind":"glass","setting":{"kind":"transponder","code":code,"mode":"alt"}}}),
    )
    .unwrap();
    advance(s, 1.);
    settle(s);
    readback(s); // Clearance delivery initiates the ground handoff.
    initial(s, airport, FacilityKind::Ground, "request taxi");
    readback(s);
    advance(s, 1.);
    settle(s);
    let handoff = instruction(s);
    assert!(matches!(handoff.instruction, AtcInstruction::Handoff { .. }));
    readback(s);
    initial(s, airport, FacilityKind::Tower, "ready for departure");
    readback(s);
    assert!(s.atc_state().pending_instructions.is_empty());
    let a = crate::regions::airport(s.sim.world(), airport).unwrap().clone();
    move_airborne(
        s,
        a.latitude_deg,
        a.longitude_deg,
        if airport == "KMMU" { 1500. } else { 700. },
    );
    advance(s, 1.);
    settle(s);
    readback(s);
    tune(s, airport, FacilityKind::Departure);
    say(s, "departure, leaving 400, heading 060");
    readback(s);
    let destination = s.comms.atc.destination.clone().unwrap();
    let a = crate::regions::airport(s.sim.world(), &destination)
        .unwrap()
        .clone();
    move_airborne(s, a.latitude_deg + 0.1, a.longitude_deg - 0.2, 700.);
    advance(s, 1.);
    settle(s);
    assert!(matches!(
        instruction(s).instruction,
        AtcInstruction::Handoff { .. }
    ));
    readback(s);
}
#[test]
fn full_kabe_departure_and_arrival_via_typed_calls() {
    let mut s = atc_session("KABE");
    departure(&mut s, "KABE");
    assert!(
        s.flight_record()
            .events
            .iter()
            .any(|e| matches!(e.meaning, Meaning::RadarContact { .. }))
    );
    let mut arrival = atc_session("KABE");
    initial(
        &mut arrival,
        "KABE",
        FacilityKind::Approach,
        "inbound for landing",
    );
    readback(&mut arrival);
    initial(&mut arrival, "KABE", FacilityKind::Tower, "inbound straight in");
    readback(&mut arrival);
    say(&mut arrival, "final runway zero six");
    readback(&mut arrival);
    assert!(arrival.comms.atc.landing);
    say(&mut arrival, "clear of the runway");
    readback(&mut arrival);
    tune(&mut arrival, "KABE", FacilityKind::Ground);
    say(&mut arrival, "clear of the runway");
    assert!(
        arrival
            .flight_record()
            .events
            .iter()
            .any(|e| e.text.contains("Taxi to parking"))
    );
    for e in arrival
        .flight_record()
        .events
        .iter()
        .filter(|e| e.speaker != Speaker::Pilot)
    {
        assert_eq!(e.source, EventSource::Rules);
    }
}
#[test]
fn wrong_missing_and_blocked_readbacks_never_authorize_takeoff() {
    let mut s = atc_session("KABE");
    initial(&mut s, "KABE", FacilityKind::Tower, "ready for departure");
    say(&mut s, "runway two four, cleared for takeoff");
    assert!(!s.comms.atc.takeoff);
    assert!(!s.comms.atc.pending.is_empty());
    advance(&mut s, 30.);
    settle(&mut s);
    assert!(
        s.comms
            .atc
            .compliance
            .iter()
            .any(|g| g.item == "missing_readback" && !g.passed)
    );
    assert!(
        s.flight_record()
            .events
            .iter()
            .any(|e| e.text.contains("Negative"))
    );
    readback(&mut s);
    assert!(s.comms.atc.takeoff);
    let mut s = atc_session("KABE");
    initial(&mut s, "KABE", FacilityKind::Ground, "request taxi");
    let p = instruction(&s);
    let st = s.atc_state().current_facility.unwrap();
    s.execute(
        "comms.transmit",
        json!({"text":format!("{{callsign}}, {}",p.phrase)}),
    )
    .unwrap();
    s.start_com(
        st.channel_khz,
        Speaker::Traffic {
            callsign: "Interfering fixture".into(),
        },
        "Overlapping test transmission".into(),
        Meaning::Text,
        EventSource::Scenario,
        s.station_origin(&st).unwrap(),
        25.,
    );
    settle(&mut s);
    assert!(!s.comms.atc.pending.is_empty());
    assert!(
        !s.comms
            .atc
            .active
            .iter()
            .any(|p| matches!(p.instruction, AtcInstruction::Taxi { .. }))
    );
}
#[test]
fn bravo_requires_explicit_received_words_and_readback_cannot_invent_clearance() {
    let mut s = atc_session("KEWR");
    initial(&mut s, "KEWR", FacilityKind::Approach, "request bravo transition");
    assert!(!s.atc_state().bravo_clearance);
    assert!(matches!(
        instruction(&s).instruction,
        AtcInstruction::Bravo { cleared: false, .. }
    ));
    readback(&mut s);
    say(&mut s, "request flight following");
    let p = instruction(&s);
    let AtcInstruction::Squawk { code } = p.instruction else {
        panic!("expected squawk")
    };
    readback(&mut s);
    s.execute(
        "avionics.set",
        json!({"setting":{"kind":"glass","setting":{"kind":"transponder","code":code,"mode":"alt"}}}),
    )
    .unwrap();
    say(&mut s, "request bravo transition");
    assert!(s.atc_state().bravo_clearance);
    readback(&mut s);
    assert!(
        s.flight_record()
            .events
            .iter()
            .any(|e| e.text.contains("Cleared into the Class Bravo") && e.reception == Reception::Heard)
    );
}
fn move_airborne(s: &mut Session, lat: f64, lon: f64, altitude: f64) {
    let [n, e] = s.radio_project(lat, lon).unwrap();
    let mut state = *s.sim.state();
    state.position_m = [n, e, -altitude];
    let heading = s.sim.readout().heading_grid_rad;
    state.velocity_mps = [40. * heading.cos(), 40. * heading.sin(), 0.];
    s.sim =
        flightcraft_sim::Sim::from_state_in_world(state, s.sim.controls(), s.sim.world().clone()).unwrap();
    s.execute(
        "assist.set",
        json!({"altitude_m":altitude,"airspeed_mps":40.,"heading_rad":s.sim.readout().heading_rad}),
    )
    .unwrap();
}
#[test]
fn class_b_c_d_entry_is_graded_from_position_and_controlling_contact() {
    let mut s = atc_session("KEWR");
    move_airborne(&mut s, 40.69, -74.17, 700.);
    s.monitor_atc();
    assert!(
        s.comms
            .atc
            .compliance
            .iter()
            .any(|g| g.item.starts_with("airspace:") && !g.passed),
        "{:?}",
        s.comms.atc.compliance
    );
    s.comms.atc.two_way.insert("KABE".into());
    s.monitor_atc();
    assert!(!s.atc_state().bravo_clearance);
    for (airport, lat, lon, alt) in [("KABE", 40.65, -75.44, 400.), ("KMMU", 40.8, -74.415, 300.)] {
        let mut s = atc_session(airport);
        move_airborne(&mut s, lat, lon, alt);
        s.monitor_atc();
        assert!(
            s.comms
                .atc
                .compliance
                .iter()
                .any(|g| g.item.starts_with("airspace:") && !g.passed)
        );
        tune(&mut s, airport, FacilityKind::Tower);
        say(&mut s, "say weather");
        s.monitor_atc();
        assert!(
            s.comms
                .atc
                .compliance
                .iter()
                .any(|g| g.item.starts_with("airspace:") && g.passed),
            "{:?}",
            s.comms.atc.compliance
        );
    }
}
#[test]
fn compliance_observes_heading_altitude_hold_short_squawk_and_frequency() {
    let mut s = atc_session("KABE");
    initial(&mut s, "KABE", FacilityKind::Ground, "request taxi");
    readback(&mut s);
    advance(&mut s, 1.);
    assert!(
        s.comms
            .atc
            .compliance
            .iter()
            .any(|g| g.item == "hold_short" && !g.passed)
    );
    settle(&mut s);
    readback(&mut s); // Finish the automatic ground-to-tower transfer first.
    tune(&mut s, "KABE", FacilityKind::Departure);
    say(&mut s, "request flight following");
    readback(&mut s);
    advance(&mut s, 15.);
    assert!(
        s.comms
            .atc
            .compliance
            .iter()
            .any(|g| g.item == "squawk" && !g.passed)
    );
    say(&mut s, "departure, leaving 400");
    readback(&mut s);
    advance(&mut s, 40.);
    assert!(
        s.comms
            .atc
            .compliance
            .iter()
            .any(|g| g.item == "altitude" && !g.passed)
    );
    assert!(s.comms.atc.compliance.iter().any(|g| g.item == "heading"));
    let a = crate::regions::airport(s.sim.world(), "KMMU").unwrap().clone();
    move_airborne(&mut s, a.latitude_deg + 0.1, a.longitude_deg - 0.2, 700.);
    advance(&mut s, 1.);
    settle(&mut s);
    assert!(matches!(
        instruction(&s).instruction,
        AtcInstruction::Handoff { .. }
    ));
    readback(&mut s);
    advance(&mut s, 15.);
    assert!(
        s.comms
            .atc
            .compliance
            .iter()
            .any(|g| g.item == "frequency_change" && !g.passed)
    );
}
#[test]
fn approach_clearance_is_bound_to_loaded_cifp_identity() {
    let mut s = atc_session("KABE");
    s.execute(
        "avionics.set",
        json!({"setting":{"kind":"load_procedure","airport":"KABE","procedure":"I06"}}),
    )
    .unwrap();
    initial(
        &mut s,
        "KABE",
        FacilityKind::Approach,
        "request ILS runway zero six approach",
    );
    assert!(
        matches!(instruction(&s).instruction,AtcInstruction::Approach {ref procedure,..} if procedure=="I06")
    );
    readback(&mut s);
    // Instrument approach acceptance can now queue the tower handoff. Exercise the
    // clearance identity check directly without treating that handoff as a new readback.
    let pending = s.comms.atc.pending.clone();
    let station = s.next_station("KABE", FacilityKind::Approach).unwrap();
    let message = parse_pilot("request ILS runway two four approach", &s.pilot_callsign());
    s.clear_approach(&station, &message, "KABE", "24");
    assert_eq!(s.comms.atc.pending, pending);
    assert!(
        s.comms
            .atc
            .replies
            .back()
            .is_some_and(|r| r.text.contains("load the requested published approach"))
    );
}
#[test]
fn visual_approach_needs_airport_in_sight_and_hands_to_tower() {
    let mut s = atc_session("KABE");
    move_airborne(&mut s, 40.60, -75.50, 800.);
    initial(
        &mut s,
        "KABE",
        FacilityKind::Approach,
        "request visual approach runway zero six",
    );
    assert!(s.comms.atc.pending.is_empty());
    assert!(
        s.flight_record()
            .events
            .iter()
            .any(|e| e.text.contains("Report the airport in sight"))
    );
    say(
        &mut s,
        "airport in sight, request visual approach runway zero six",
    );
    assert!(
        matches!(instruction(&s).instruction,AtcInstruction::Approach{ref procedure,..} if procedure=="visual")
    );
    assert_eq!(instruction(&s).phrase, "Cleared visual approach runway zero six.");
    readback(&mut s);
    advance(&mut s, 1.);
    settle(&mut s);
    assert!(matches!(
        instruction(&s).instruction,
        AtcInstruction::Handoff { .. }
    ));
}

#[test]
fn radar_identification_continues_across_sectors_of_the_same_facility() {
    let mut s = atc_session("KABE");
    let first = s
        .stations()
        .into_iter()
        .find(|st| st.airport == "KABE" && st.kind == FacilityKind::Approach)
        .unwrap();
    let sector = s
        .stations()
        .into_iter()
        .find(|st| {
            st.airport == "KABE" && st.kind == FacilityKind::Approach && st.channel_khz != first.channel_khz
        })
        .unwrap();
    assert_eq!(phraseology::facility(&first), "Allentown Approach");
    assert_eq!(phraseology::facility(&sector), "Allentown Approach");
    let other = s.next_station("KMMU", FacilityKind::Departure).unwrap();
    let mut event = Transmission {
        id: 999,
        time_s: 0.,
        duration_s: 1.,
        channel: Channel::Com,
        frequency_hz: None,
        speaker: Speaker::Facility {
            id: first.id.clone(),
            role: first.kind,
        },
        speaker_name: phraseology::facility(&first),
        text: "Radar contact.".into(),
        meaning: Meaning::RadarContact {
            station_id: first.id.clone(),
        },
        source: EventSource::Rules,
        reception: Reception::Blocked,
        reason: None,
        grade: None,
    };
    s.completed_atc(&event, None);
    assert!(s.comms.atc.radar_facility.is_none());
    let mut call = event.clone();
    call.speaker = Speaker::Pilot;
    call.meaning = Meaning::Text;
    call.text = format!("{}, level two thousand four hundred", s.pilot_callsign().full);
    for (station, expect_contact) in [(first, true), (sector, false), (other, true)] {
        s.comms.atc.replies.clear();
        s.comms.atc.pending.clear();
        s.handle_pilot(&call, station.clone());
        assert_eq!(
            s.comms
                .atc
                .replies
                .iter()
                .any(|r| matches!(r.meaning, Meaning::RadarContact { .. })),
            expect_contact
        );
        if expect_contact {
            event.speaker = Speaker::Facility {
                id: station.id.clone(),
                role: station.kind,
            };
            event.meaning = Meaning::RadarContact {
                station_id: station.id,
            };
            event.reception = Reception::Heard;
            s.completed_atc(&event, None);
        }
    }
}

#[test]
fn lost_comms_failure_silences_com_but_not_intercom_and_all_signals_are_typed() {
    let mut s = atc_session("KABE");
    let before = s.sim.state_hash();
    s.execute("failure.set", json!({"failure":{"kind":"radio_failure"}}))
        .unwrap();
    assert_ne!(s.sim.state_hash(), before);
    assert!(!s.comms_state().com1_powered);
    assert!(s.execute("comms.transmit", json!({"text":"test"})).is_err());
    s.execute("comms.transmit", json!({"text":"radio failure","intercom":true}))
        .unwrap();
    for signal in [
        LightGunSignal::SteadyGreen,
        LightGunSignal::FlashingGreen,
        LightGunSignal::SteadyRed,
        LightGunSignal::FlashingRed,
        LightGunSignal::FlashingWhite,
        LightGunSignal::AlternatingRedGreen,
    ] {
        s.run(SetLightGun {
            airport: "KABE".into(),
            signal,
        })
        .unwrap();
        let g = s.atc_state().light_gun.unwrap();
        assert_eq!(g.signal, signal);
        assert!(g.position_estimated);
        assert!(!g.indication.is_empty());
    }
    s.execute("failure.clear", json!({"target":"radio_failure"}))
        .unwrap();
    assert!(!s.atc_state().radio_failed);
    assert!(s.atc_state().light_gun.is_none());
}
#[test]
fn controlled_pattern_reuses_real_traffic_for_sequence_and_go_around() {
    let mut s = atc_session("KABE");
    s.execute("traffic.set", json!({"airport":"KABE","runway":"06","count":3}))
        .unwrap();
    initial(&mut s, "KABE", FacilityKind::Tower, "inbound left downwind");
    assert!(matches!(
        instruction(&s).instruction,
        AtcInstruction::Pattern { number: 2, .. }
    ));
    readback(&mut s);
    say(&mut s, "going around runway zero six");
    assert!(matches!(
        instruction(&s).instruction,
        AtcInstruction::GoAround { .. }
    ));
    readback(&mut s);
}
#[test]
fn atc_commands_have_valid_schemas_and_hostile_inputs_fail_atomically() {
    let mut s = atc_session("KABE");
    for command in Session::commands().iter().filter(|c| c.name.starts_with("atc.")) {
        jsonschema::meta::validate(&command.input_schema).unwrap();
        jsonschema::meta::validate(&command.output_schema).unwrap();
    }
    let before = s.atc_state().airport;
    assert!(s.execute("atc.configure", json!({"airport":"NOPE"})).is_err());
    assert_eq!(before, s.atc_state().airport);
    assert!(
        s.execute("atc.light_gun", json!({"airport":"KABE","signal":"steady_green"}))
            .is_err()
    );
    let output = s.execute("atc.state", json!({})).unwrap();
    let schema = Session::commands()
        .into_iter()
        .find(|c| c.name == "atc.state")
        .unwrap();
    assert!(
        jsonschema::validator_for(&schema.output_schema)
            .unwrap()
            .is_valid(&output)
    );
}

#[test]
fn changing_airport_clears_stale_facility_without_dropping_readbacks() {
    let mut s = atc_session("KMMU");
    initial(&mut s, "KMMU", FacilityKind::Ground, "request taxi");
    let pending = instruction(&s);
    assert!(s.atc_state().current_facility.is_some());
    s.execute("atc.configure", json!({"airport":"KABE"})).unwrap();
    let state = s.atc_state();
    assert_eq!(state.airport.as_deref(), Some("KABE"));
    assert!(state.current_facility.is_none());
    assert!(state.reported_information.is_none());
    assert_eq!(state.pending_instructions.first(), Some(&pending));
}

#[test]
fn natural_craft_readbacks_and_ifr_bravo_authority() {
    let mut s = atc_session("KMMU");
    initial(&mut s, "KMMU", FacilityKind::Clearance, "request IFR clearance");
    let i = instruction(&s).instruction;
    let AtcInstruction::IfrClearance {
        code,
        altitude_ft,
        departure_khz,
        ..
    } = &i
    else {
        panic!("CRAFT");
    };
    let text = format!(
        "Cleared to Allentown as filed, climb and maintain {}, expect five thousand in ten, departure {}, squawk {}, Sling Three Four Bravo",
        phraseology::altitude(*altitude_ft),
        phraseology::frequency(*departure_khz),
        phraseology::digits(code.parse().unwrap(), 4)
    );
    let m = parse_pilot(&text, &s.pilot_callsign());
    assert!(m.callsign_present);
    assert!(rules::readback_matches(&i, &m), "{m:?}");
    let full = parse_pilot(&s.instruction_phrase(&i), &s.pilot_callsign());
    assert!(rules::readback_matches(&i, &full));
    assert!(!rules::readback_matches(
        &i,
        &parse_pilot(
            &text.replace("five thousand", "six thousand"),
            &s.pilot_callsign()
        )
    ));
    assert!(s.atc_state().ifr_clearance);
    move_airborne(&mut s, 40.69, -74.17, 700.);
    s.monitor_atc();
    assert!(
        s.atc_state()
            .compliance
            .iter()
            .filter(|g| g.item.starts_with("airspace:"))
            .all(|g| g.passed)
    );
    s.comms.atc.ifr_clearance = false;
    s.comms.atc.last_observed.clear();
    s.monitor_atc();
    assert!(
        s.atc_state()
            .compliance
            .iter()
            .any(|g| g.item.starts_with("airspace:") && !g.passed)
    );
}
#[test]
fn feedback_is_plain_throttled_and_safety_first() {
    let mut s = atc_session("KABE");
    s.atc_grade(None, "squawk", false, "Squawk is still 1200 — set 4003.".into());
    s.atc_grade(
        None,
        "heading",
        false,
        "Assigned heading 048, you're on 063.".into(),
    );
    s.atc_grade(
        None,
        "hold_short",
        false,
        "You crossed the hold-short line at runway 5.".into(),
    );
    advance(&mut s, 20.);
    let cues: Vec<_> = s
        .flight_record()
        .events
        .iter()
        .filter(|e| matches!(e.meaning, Meaning::InstructorFeedback { .. }))
        .collect();
    assert!(cues.first().unwrap().text.contains("hold-short"));
    assert!(cues.windows(2).all(|p| p[1].time_s - p[0].time_s >= 8.));
    assert!(
        cues.iter()
            .all(|e| !e.text.contains("Some(") && !e.text.contains("squawk:"))
    );
}

fn save_preview(s: &mut Session, name: &str, title: &str, note: &str) {
    for _ in 0..240 {
        if s.comms.pending.is_empty() && s.comms.atc.replies.is_empty() {
            break;
        }
        advance(s, 0.25);
    }
    assert!(
        s.comms.pending.is_empty(),
        "preview must finish its transmissions"
    );
    assert!(
        s.comms.atc.replies.is_empty(),
        "preview must drain controller replies"
    );
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/atc");
    std::fs::create_dir_all(&dir).unwrap();
    let events = &s.flight_record().events;
    std::fs::write(
        dir.join(format!("{name}.json")),
        serde_json::to_string_pretty(s.flight_record()).unwrap(),
    )
    .unwrap();
    let mut md = format!(
        "# {title}\n\n{note}\n\n| Flight seconds | Speaker | Channel MHz | Reception | Transmission |\n|---:|---|---:|---|---|\n"
    );
    let mut rows = String::new();
    fn escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }
    for e in events {
        if matches!(e.meaning, Meaning::Compliance { .. }) {
            continue;
        }
        let speaker = e.speaker_name.clone();
        let freq = e
            .frequency_hz
            .map_or("intercom".into(), |f| format!("{:.3}", f / 1e6));
        md += &format!(
            "| {:.2} | {} | {} | {} | {} |\n",
            e.time_s,
            speaker,
            freq,
            phraseology::reception(e.reception),
            e.text.replace('|', "/")
        );
        rows += &format!(
            "<tr><td>{:.2}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            e.time_s,
            escape(&speaker),
            freq,
            phraseology::reception(e.reception),
            escape(&e.text)
        );
    }
    std::fs::write(dir.join(format!("{name}.md")), md).unwrap();
    let html = format!(
        "<!doctype html><html lang=en><meta charset=utf-8><meta name=viewport content='width=device-width'><title>{}</title><style>body{{font:16px system-ui;background:#14202b;color:#e8edf2;margin:32px}}a{{color:#7cddc5}}table{{border-collapse:collapse;width:100%}}td,th{{padding:10px;text-align:left;border-bottom:1px solid #364653}}td:last-child{{width:55%}}header{{max-width:900px}}input{{font:inherit;padding:8px;margin:16px 0}}.lamp{{display:inline-block;border-radius:50%;width:24px;height:24px;background:#8aff9c;box-shadow:0 0 18px #8aff9c;animation:flash 1s steps(1) infinite}}@keyframes flash{{50%{{opacity:0}}}}</style><header><a href=index.html>ATC previews</a><h1>{}</h1><p>{}</p><p>Original deterministic rules; FAA NASR/CIFP cycle 2611. Flight time only. For simulation training.</p></header><input placeholder='Filter transcript' aria-label='Filter transcript' oninput=\"for(const r of document.querySelectorAll('tbody tr'))r.hidden=!r.textContent.toLowerCase().includes(this.value.toLowerCase())\"><table><thead><tr><th>Seconds</th><th>Speaker</th><th>MHz</th><th>Reception</th><th>Transmission</th></tr></thead><tbody>{rows}</tbody></table></html>",
        escape(title),
        escape(title),
        escape(note)
    );
    std::fs::write(dir.join(format!("{name}.html")), html).unwrap();
    std::fs::write(
        dir.join(format!("{name}-state.json")),
        serde_json::to_string_pretty(&s.atc_state()).unwrap(),
    )
    .unwrap();
}
#[test]
#[ignore = "writes ATC transcripts and interactive HTML previews under out/atc"]
fn write_atc_previews() {
    flown::route_preview();
    flown::other_previews();
    write_preview_index();
}
#[test]
#[ignore = "writes the continuously flown pattern, Hudson, and lost-comms previews"]
fn write_other_atc_previews() {
    flown::other_previews();
    write_preview_index();
}
fn write_preview_index() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/atc");
    std::fs::write(dir.join("index.html"),"<!doctype html><html lang=en><meta charset=utf-8><title>ATC previews</title><style>body{font:18px system-ui;background:#14202b;color:#e8edf2;margin:40px;max-width:960px}a{color:#7cddc5}li{margin:16px}</style><h1>Tower and approach training</h1><p>Continuously flown radio exercises on fetched New York/eastern Pennsylvania terrain and FAA NASR/CIFP records. Each exercise includes a sampled track; see the ATC guide for training limits.</p><ul><li><a href=kmmu-kabe.html>KMMU departure to KABE arrival</a><li><a href=kabe-pattern.html>KABE tower pattern with moving traffic</a><li><a href=hudson-bravo.html>Hudson Class Bravo transition</a><li><a href=lost-comms.html>Lost communications and all six light-gun signals</a><li><a href=atc-panel.png>Actual egui ATC panel</a><li><a href=light-gun-world.png>World-projected tower light gun</a></ul><p>Each transcript has matching .md and typed .json flight-event records, plus -state.json. See docs/atc.md for operating limits.</p></html>").unwrap();
}

#[test]
fn unidentified_calls_do_not_establish_two_way_contact_and_unknown_requests_get_say_again() {
    let mut s = atc_session("KABE");
    tune(&mut s, "KABE", FacilityKind::Approach);
    s.execute(
        "comms.transmit",
        json!({"text":"Allentown approach, request flight following"}),
    )
    .unwrap();
    settle(&mut s);
    assert!(s.atc_state().two_way_providers.is_empty());
    assert!(
        s.flight_record()
            .events
            .iter()
            .any(|e| e.text == "Aircraft calling, say again callsign.")
    );
    say(&mut s, "request something not in the grammar");
    assert!(
        s.flight_record()
            .events
            .iter()
            .any(|e| e.text.contains("Say again request"))
    );
    assert!(s.atc_state().two_way_providers.contains(&"ABE".to_string()));
}
#[test]
fn vector_tolerances_and_default_radio_hash_are_independent() {
    let mut s = atc_session("KABE");
    let mut pose = *s.sim.state();
    pose.time_s = 100.;
    pose.position_m[2] = -(3000. + 99.9) * 0.3048;
    let heading = 9.99_f64.to_radians();
    pose.attitude = [(heading / 2.).cos(), 0., 0., (heading / 2.).sin()];
    pose.velocity_mps = [40., 0., 0.];
    s.sim = flightcraft_sim::Sim::from_state_in_world(pose, s.sim.controls(), s.sim.world().clone()).unwrap();
    let heading_deg = (s.sim.readout().heading_rad.to_degrees() - 9.0)
        .round()
        .rem_euclid(360.) as u16;
    s.comms.atc.active.push(PendingInstruction {
        id: 999,
        station_id: "tolerance-fixture".into(),
        airport: "KABE".into(),
        instruction: AtcInstruction::Vector {
            heading_deg,
            altitude_ft: 3000,
        },
        phrase: "Tolerance fixture".into(),
        heard: true,
        readback: true,
        issued_s: 0.,
        reminder_s: f64::MAX,
    });
    let hash = s.sim.state_hash();
    s.monitor_atc();
    assert_eq!(s.sim.state_hash(), hash);
    assert!(
        s.comms
            .atc
            .compliance
            .iter()
            .any(|g| g.instruction_id == Some(999) && g.item == "altitude" && g.passed)
    );
    assert!(
        s.comms
            .atc
            .compliance
            .iter()
            .any(|g| g.instruction_id == Some(999) && g.item == "heading" && g.passed)
    );
    if let Some(p) = s.comms.atc.active.first_mut() {
        p.instruction = AtcInstruction::Vector {
            heading_deg: (heading_deg + 30) % 360,
            altitude_ft: 4000,
        };
    }
    s.monitor_atc();
    assert!(
        s.comms
            .atc
            .compliance
            .iter()
            .any(|g| g.instruction_id == Some(999) && g.item == "heading" && !g.passed)
    );
    assert!(
        s.comms
            .atc
            .compliance
            .iter()
            .any(|g| g.instruction_id == Some(999) && g.item == "altitude" && !g.passed)
    );
}
#[test]
fn blocked_controller_clearance_can_be_repeated_but_not_invented_by_pilot() {
    let mut s = atc_session("KABE");
    tune(&mut s, "KABE", FacilityKind::Tower);
    let station = s.next_station("KABE", FacilityKind::Tower).unwrap();
    let p = PendingInstruction {
        id: 100,
        station_id: station.id,
        airport: "KABE".into(),
        instruction: AtcInstruction::Takeoff {
            runway: "06".into(),
            closed_traffic: false,
        },
        phrase: "Runway 06, cleared for takeoff.".into(),
        heard: false,
        readback: false,
        issued_s: 0.,
        reminder_s: f64::MAX,
    };
    s.comms.atc.pending.push(p);
    say(&mut s, "runway zero six, cleared for takeoff");
    assert!(!s.comms.atc.takeoff);
    say(&mut s, "say again");
    assert!(instruction(&s).heard);
    readback(&mut s);
    assert!(s.comms.atc.takeoff);
}

#[test]
fn addressed_radar_provider_wins_when_two_facilities_share_a_carrier() {
    let s = atc_session("KABE");
    let st = s
        .stations()
        .into_iter()
        .find(|s| s.airport == "KABE" && s.kind == FacilityKind::Approach && s.channel_khz == 124450)
        .unwrap();
    let text = phraseology::pilot_request(
        &st,
        &s.pilot_callsign(),
        "request flight following",
        Some("alpha"),
    );
    let message = parse_pilot(&text, &s.pilot_callsign());
    assert_eq!(
        s.tuned_facility(&message, st.channel_khz).unwrap().airport,
        "KABE"
    );
}

#[test]
fn automatic_handoff_waits_until_the_pilot_finishes_transmitting() {
    let mut s = atc_session("KABE");
    move_airborne(&mut s, 40.65, -75.44, 700.);
    let st = tune(&mut s, "KABE", FacilityKind::Tower);
    s.comms.atc.current = Some(st);
    s.comms.atc.takeoff = true;
    s.execute(
        "comms.transmit",
        json!({"text":"Allentown Tower, Sling One Two Three Four Bravo, departing southwest."}),
    )
    .unwrap();
    s.monitor_atc();
    assert!(
        s.comms.atc.pending.is_empty(),
        "handoff must not pre-empt an in-progress pilot call"
    );
    s.comms.pending.clear();
    s.monitor_atc();
    assert!(
        s.comms
            .atc
            .pending
            .iter()
            .any(|p| matches!(p.instruction, AtcInstruction::Handoff { .. }))
    );
}

#[test]
fn accepted_landing_clearance_is_not_reissued_for_position_reports() {
    let mut s = atc_session("KABE");
    initial(&mut s, "KABE", FacilityKind::Tower, "inbound straight in");
    readback(&mut s);
    say(&mut s, "final runway zero six");
    readback(&mut s);
    assert!(s.comms.atc.landing);
    let issued = s.flight_record().events.len();
    for report in [
        "left downwind runway zero six",
        "left base runway zero six",
        "final runway zero six",
    ] {
        say(&mut s, report);
        assert!(s.comms.atc.pending.is_empty());
    }
    assert!(!s.flight_record().events.iter().skip(issued).any(|e| matches!(
        e.meaning,
        Meaning::Atc {
            instruction: AtcInstruction::Land { .. }
        }
    )));
    // Also protect clients that issue the same clearance directly rather than reporting position.
    let st = s.comms.atc.current.clone().unwrap();
    s.issue(&st, AtcInstruction::Land { runway: "06".into() });
    assert!(s.comms.atc.pending.is_empty());
}
#[test]
fn ground_decimal_readback_and_atis_spacing_use_shared_phraseology() {
    let s = atc_session("KABE");
    let instruction = AtcInstruction::ExitRunway {
        runway: "06".into(),
        ground_khz: 121900,
    };
    let text = s.atc_readback(&instruction);
    assert!(text.starts_with("Contact ground point niner, "), "{text}");
    assert!(rules::readback_matches(
        &instruction,
        &parse_pilot(&text, &s.pilot_callsign())
    ));
    assert_eq!(
        parse_pilot("contact ground point seven five", &s.pilot_callsign()).channel_khz,
        Some(121750)
    );
    assert_eq!(parse_pilot("point niner", &s.pilot_callsign()).channel_khz, None);
    assert!(!s.atis_report("KABE").unwrap().text.contains("  "));
}

#[test]
fn hold_readback_requires_the_fix_course_turn_legs_altitude_and_efc() {
    let s = atc_session("KABE");
    let instruction = AtcInstruction::Hold {
        fix: "JISTO".into(),
        inbound_deg: 63,
        right_turns: true,
        leg_time_s: Some(60),
        leg_distance_nm: None,
        altitude_ft: 3000,
        efc_utc: 1530,
    };
    let readback = s.atc_readback(&instruction);
    assert!(rules::readback_matches(
        &instruction,
        &parse_pilot(&readback, &s.pilot_callsign())
    ));
    for (correct, wrong) in [
        ("JISTO", "MUDRE"),
        ("zero six three", "zero six four"),
        ("right turns", "left turns"),
        ("one minute legs", "two minute legs"),
        ("three thousand", "four thousand"),
        ("one five three zero", "one six three zero"),
    ] {
        let bad = readback.replace(correct, wrong);
        assert_ne!(bad, readback);
        assert!(
            !rules::readback_matches(&instruction, &parse_pilot(&bad, &s.pilot_callsign())),
            "{bad}"
        );
    }
}

#[test]
fn ifr_intercept_readback_requires_heading_restriction_and_approach() {
    let s = atc_session("KABE");
    let instruction = AtcInstruction::Approach {
        airport: "KABE".into(),
        procedure: "I06".into(),
        name: "ILS runway 06".into(),
        runway: "06".into(),
        intercept: Some(ApproachIntercept {
            heading_deg: 40,
            altitude_ft: 2300,
            turn_left: true,
        }),
    };
    let good = s.atc_readback(&instruction);
    assert!(rules::readback_matches(
        &instruction,
        &parse_pilot(&good, &s.pilot_callsign())
    ));
    for bad in [
        good.replace("heading zero four zero, ", ""),
        good.replace("two thousand three hundred", "three thousand"),
        good.replace("until established on the localizer, ", ""),
        good.replace("cleared ILS runway zero six approach", "ILS runway zero six"),
        good.replace("zero four zero", "zero five zero"),
        good.replace("localizer", "final approach course"),
    ] {
        assert!(
            !rules::readback_matches(&instruction, &parse_pilot(&bad, &s.pilot_callsign())),
            "{bad}"
        );
    }
    let station = s.next_station("KABE", FacilityKind::Approach).unwrap();
    let call = phraseology::pilot_ifr_checkin(&station, &s.pilot_callsign(), 4300, Some("alpha"));
    assert_eq!(
        parse_pilot(&call, &s.pilot_callsign()).request,
        Some(PilotRequest::Position)
    );
    assert!(!call.to_lowercase().contains("flight following"));
}
