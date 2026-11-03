#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use super::*;
use crate as flightcraft_control;
use serde_json::json;
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/atc_fixture.rs"));

#[test]
fn table_of_student_radio_phrasings() {
    let s = atc_session("KABE");
    let table: Vec<serde_json::Value> = serde_json::from_str(include_str!("phrases.json")).unwrap();
    assert!(table.len() >= 60);
    for row in table {
        let raw = row["text"].as_str().unwrap();
        let (text, accepted) = normalize_speech(raw, row["intercom"].as_bool().unwrap_or(false), &s);
        assert_eq!(accepted, row["valid"].as_bool().unwrap(), "{raw}: {text}");
        let parsed = serde_json::to_value(parse_pilot(&text, &s.pilot_callsign())).unwrap();
        if let Some(slots) = row["slots"].as_object() {
            for (key, value) in slots {
                assert_eq!(&parsed[key], value, "{raw}: {key}");
            }
        }
    }
}
#[test]
fn preview_is_ungraded_retry_cannot_transmit_and_changed_com_is_refused() {
    let mut s = atc_session("KABE");
    let input = |confidence| CommsRecognize {
        request_id: None,
        text: "Allentown tower Sling three seven one foxtrot charlie ready runway zero six".into(),
        confidence,
        intercom: false,
    };
    let before = s.atc_state().compliance.len();
    let draft = s.run(input(0.2)).unwrap();
    assert!(draft.retry);
    assert_eq!(s.atc_state().compliance.len(), before);
    assert!(s.flight_record().events.last().unwrap().grade.is_none());
    assert!(s.run(CommsConfirmSpeech { id: draft.id }).is_err());
    let draft = s.run(input(0.9)).unwrap();
    s.execute("comms.audio_panel", json!({"transmit":2})).unwrap();
    assert!(s.run(CommsConfirmSpeech { id: draft.id }).is_err());
    s.execute("comms.audio_panel", json!({"transmit":1})).unwrap();
    let event = s.run(CommsConfirmSpeech { id: draft.id }).unwrap();
    assert!(matches!(event.meaning, Meaning::PilotCall { .. }));
    assert_eq!(event.channel, Channel::Com);
    assert!(s.run(CommsConfirmSpeech { id: draft.id }).is_err());
}
#[test]
fn expected_readback_never_repairs_wrong_numbers_or_negations() {
    let mut s = atc_session("KABE");
    s.comms.atc.pending.push(PendingInstruction {
        id: 42,
        station_id: "test".into(),
        airport: "KABE".into(),
        instruction: AtcInstruction::Vector {
            heading_deg: 270,
            altitude_ft: 4500,
        },
        phrase: String::new(),
        heard: true,
        readback: false,
        issued_s: 0.,
        reminder_s: 0.,
    });
    let (text, ok) = normalize_speech(
        "Sling three seven one foxtrot charlie heading two five zero maintain four thousand",
        false,
        &s,
    );
    assert!(ok);
    let m = parse_pilot(&text, &s.pilot_callsign());
    assert_eq!(m.heading_deg, Some(250));
    assert_eq!(m.altitude_ft, Some(4000));
    let (text, _) = normalize_speech(
        "Sling three seven one foxtrot charlie negative cleared to land runway zero six",
        false,
        &s,
    );
    assert!(text.contains("negative"));
}
#[test]
fn intercom_voice_preserves_positive_control_exchange() {
    let mut s = atc_session("KABE");
    s.execute("crew.mode", json!({"mode":"watching"})).unwrap();
    let draft = s
        .run(CommsRecognize {
            request_id: None,
            text: "your controls".into(),
            confidence: 0.95,
            intercom: true,
        })
        .unwrap();
    s.run(CommsConfirmSpeech { id: draft.id }).unwrap();
    assert_eq!(s.crew_state().controls, crate::ControlOwner::Pilot);
    assert_eq!(s.crew_state().pending_handover, Some(crate::ControlOwner::Crew));
    assert!(
        s.flight_record()
            .events
            .iter()
            .any(|e| e.text == "your controls" && e.channel == Channel::Intercom)
    );
}

#[test]
fn fuzzy_readback_uses_heard_state_without_guessing_numeric_slots() {
    let mut s = atc_session("KABE");
    let raw = "Sling three seven one foxtrot charlie cleered for takeoff runway zero six";
    assert!(!normalize_speech(raw, false, &s).1);
    s.comms.atc.pending.push(PendingInstruction {
        id: 42,
        station_id: "test".into(),
        airport: "KABE".into(),
        instruction: AtcInstruction::Takeoff {
            runway: "06".into(),
            closed_traffic: false,
        },
        phrase: String::new(),
        heard: true,
        readback: false,
        issued_s: 0.,
        reminder_s: 0.,
    });
    let (normalized, valid) = normalize_speech(raw, false, &s);
    assert!(valid && normalized.contains("cleared for takeoff"));
    assert!(
        !normalize_speech(
            "Sling threex seven one foxtrot charlie runway zero six",
            false,
            &s
        )
        .1
    );
}

#[test]
fn retry_discards_the_draft_in_the_command_owner() {
    let mut s = atc_session("KABE");
    let draft = s
        .run(CommsRecognize {
            request_id: None,
            text: "Allentown tower Sling three seven one foxtrot charlie ready runway zero six".into(),
            confidence: 0.95,
            intercom: false,
        })
        .unwrap();
    assert!(s.run(CommsDiscardSpeech { id: draft.id }).unwrap());
    assert!(s.run(CommsConfirmSpeech { id: draft.id }).is_err());
    assert!(!s.run(CommsDiscardSpeech { id: draft.id }).unwrap());
}
#[test]
fn ai_instructor_cues_remain_intercom_and_do_not_transfer_controls() {
    let mut s = atc_session("KABE");
    let event = s
        .run(crate::CrewSay {
            text: "Check the assigned runway before reading it back.".into(),
            critical: false,
        })
        .unwrap();
    assert_eq!(event.channel, Channel::Intercom);
    assert_eq!(event.source, EventSource::Ai);
    assert_eq!(event.speaker, Speaker::Instructor);
    assert_eq!(s.crew_state().controls, crate::ControlOwner::Pilot);
    assert!(s.atc_state().pending_instructions.is_empty());
}

#[test]
fn browser_command_queue_keeps_speech_correlation_across_owner_event_ids() {
    let mut owner = atc_session("KABE");
    let mut display = atc_session("KABE");
    owner.execute("comms.audio_panel", json!({"transmit":2})).unwrap();
    display.apply_screen_snapshot(owner.screen_snapshot()).unwrap();
    let draft = display
        .run(CommsRecognize {
            request_id: Some(7001),
            text: "Allentown tower Sling three seven one foxtrot charlie ready runway zero six".into(),
            confidence: 0.95,
            intercom: false,
        })
        .unwrap();
    assert_eq!(draft.radio, 2);
    for (name, input) in display.take_screen_commands() {
        owner.execute(&name, input).unwrap();
    }
    display.apply_screen_snapshot(owner.screen_snapshot()).unwrap();
    display.run(CommsConfirmSpeech { id: draft.id }).unwrap();
    for (name, input) in display.take_screen_commands() {
        owner.execute(&name, input).unwrap();
    }
    assert!(
        owner
            .flight_record()
            .events
            .iter()
            .any(|e| e.speaker == Speaker::Pilot && matches!(e.meaning, Meaning::PilotCall { .. }))
    );
}

#[test]
fn pattern_position_fuzzy_match_requires_an_accepted_closed_traffic_clearance() {
    let mut s = atc_session("KABE");
    let raw = "Allentown Tower Sling three seven one foxtrot charlie left townwind runway zero six";
    assert!(!normalize_speech(raw, false, &s).1);
    s.comms.atc.closed_traffic = true;
    let (normalized, valid) = normalize_speech(raw, false, &s);
    assert!(valid && normalized.contains("left downwind runway zero six"));
    let (normalized, valid) = normalize_speech(&raw.replace("zero six", "two four"), false, &s);
    assert!(valid);
    assert_eq!(
        parse_pilot(&normalized, &s.pilot_callsign()).runway.as_deref(),
        Some("24")
    );
}

#[test]
fn browser_snapshot_carries_the_accepted_pattern_call_set() {
    let mut owner = atc_session("KABE");
    let mut display = atc_session("KABE");
    owner.comms.atc.closed_traffic = true;
    let snapshot = owner.screen_snapshot();
    assert!(snapshot.atc.closed_traffic);
    display.apply_screen_snapshot(snapshot).unwrap();
    let raw = "Allentown Tower Sling three seven one foxtrot charlie left townwind runway zero six";
    assert_eq!(
        normalize_speech(raw, false, &owner),
        normalize_speech(raw, false, &display)
    );
    assert!(normalize_speech(raw, false, &display).1);
}

#[test]
fn pattern_word_boundaries_preserve_spoken_direction_numbers_and_negation() {
    let mut s = atc_session("KABE");
    let raw = "Allentown Tower Sling three seven one foxtrot charlie leftown wind runway two four";
    assert!(!normalize_speech(raw, false, &s).1);
    s.comms.atc.closed_traffic = true;
    let (normalized, valid) = normalize_speech(raw, false, &s);
    assert!(
        valid && normalized.contains("left downwind runway two four"),
        "{normalized}"
    );
    assert_eq!(
        parse_pilot(&normalized, &s.pilot_callsign()).runway.as_deref(),
        Some("24")
    );
    let (right, valid) = normalize_speech(&raw.replace("leftown", "rightown"), false, &s);
    assert!(valid && right.contains("right downwind"));
    assert!(!right.contains("left downwind"));
    let (negated, _) = normalize_speech(&raw.replace("leftown", "negative leftown"), false, &s);
    assert!(negated.contains("negative left downwind"));
    assert!(!normalize_speech(&raw.replace("leftown", "ightown"), false, &s).1);
    assert!(!normalize_speech(&raw.replace("leftown", "leftnotdown"), false, &s).1);
}
