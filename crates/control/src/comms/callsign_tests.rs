#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use super::*;
use serde_json::json;

#[test]
fn registration_validation_table() {
    use CallsignError::*;
    let cases = [
        ("N371FC", false, Ok("N371FC")),
        ("N12345", false, Ok("N12345")),
        ("n371fc", false, Ok("N371FC")),
        ("N1", false, Ok("N1")),
        ("N9", false, Ok("N9")),
        ("N10", false, Ok("N10")),
        ("N99999", false, Ok("N99999")),
        ("N1A", false, Ok("N1A")),
        ("N1AZ", false, Ok("N1AZ")),
        ("N1234Z", false, Ok("N1234Z")),
        ("N123ZZ", false, Ok("N123ZZ")),
        ("N10000", false, Ok("N10000")),
        ("N0FC", false, Err(LeadingZero)),
        ("N0", false, Err(LeadingZero)),
        ("N01234", false, Err(LeadingZero)),
        ("N12IO", false, Err(ExcludedLetters)),
        ("N1I", false, Err(ExcludedLetters)),
        ("N1O", false, Err(ExcludedLetters)),
        ("NFC1", false, Err(LetterPosition)),
        ("N1A2", false, Err(LetterPosition)),
        ("NAB", false, Err(LetterPosition)),
        ("N1ABC", false, Err(TooManyLetters)),
        ("N", false, Err(UsLength)),
        ("N123456", false, Err(UsLength)),
        ("", false, Err(UsPrefix)),
        ("12345", false, Err(UsPrefix)),
        ("G-ABCD", false, Err(UsPrefix)),
        ("N12-3", false, Err(UsCharacters)),
        ("N1 23", false, Err(UsCharacters)),
        ("N1é", false, Err(UsCharacters)),
        ("N1/2", false, Err(UsCharacters)),
        ("N1\n", false, Err(UsCharacters)),
        ("C-GABC", true, Ok("C-GABC")),
        ("g-abcd", true, Ok("G-ABCD")),
        ("D-IOAB", true, Ok("D-IOAB")),
        ("9M-ABC", true, Ok("9M-ABC")),
        ("A-1234567890", true, Ok("A-1234567890")),
        ("A-12345678901", true, Err(ForeignFormat)),
        ("-GABC", true, Err(ForeignFormat)),
        ("G-", true, Err(ForeignFormat)),
        ("G--ABCD", true, Err(ForeignFormat)),
        ("G-AB CD", true, Err(ForeignFormat)),
        ("G-ABC/", true, Err(ForeignFormat)),
        ("N371FC", true, Err(ForeignFormat)),
        ("1-ABC", true, Err(ForeignFormat)),
        ("Ｇ-ABC", true, Err(ForeignFormat)),
    ];
    for (input, foreign, expected) in cases {
        assert_eq!(
            validate_registration(input, foreign).as_deref(),
            expected.as_ref().copied(),
            "{input}"
        );
    }
    assert!(LeadingZero.to_string().contains("can't start with zero"));
    assert!(
        ExcludedLetters
            .to_string()
            .contains("letters I and O aren't used")
    );
    assert!(LetterPosition.to_string().contains("letters only at the end"));
}

#[test]
fn typed_errors_are_atomic_and_callsigns_do_not_change_physics() {
    let mut s = Session::new().unwrap();
    let hash = s.sim().state_hash();
    s.execute(
        "comms.callsign",
        json!({"registration":"n371fc","spoken_type":"Sling"}),
    )
    .unwrap();
    let before = serde_json::to_value(s.callsign_preferences()).unwrap();
    assert_eq!(
        s.execute(
            "comms.callsign",
            json!({"registration":"N0FC","abbreviate_after_contact":false})
        )
        .unwrap_err(),
        CommandError::Callsign(CallsignError::LeadingZero)
    );
    assert_eq!(serde_json::to_value(s.callsign_preferences()).unwrap(), before);
    assert_eq!(s.pilot_callsign().registration, "N371FC");
    assert!(s.comms.abbreviate);
    assert_eq!(s.sim().state_hash(), hash);
}

fn load(s: &mut Session, id: &str) {
    s.execute(
        "aircraft.load",
        json!({"source":{"kind":"installed","id":id},"reset":true}),
    )
    .unwrap();
}

#[test]
fn preferences_round_trip_variants_transient_reset_and_worker() {
    let mut s = Session::new().unwrap();
    s.execute(
        "comms.callsign",
        json!({"registration":"N371FC","spoken_type":"Sling"}),
    )
    .unwrap();
    load(&mut s, "larrow");
    assert!(!s.pilot_callsign().overridden);
    s.execute(
        "comms.callsign",
        json!({"registration":"g-abio","spoken_type":"Larrow","not_us":true}),
    )
    .unwrap();
    load(&mut s, "corvid");
    assert_eq!(s.pilot_callsign().registration, "N371FC");
    s.execute("comms.callsign", json!({"registration":"N52R","persist":false}))
        .unwrap();
    assert_eq!(s.pilot_callsign().registration, "N52R");
    let json = serde_json::to_string(s.callsign_preferences()).unwrap();
    let mut fresh = Session::new().unwrap();
    fresh
        .restore_callsign_preferences(serde_json::from_str(&json).unwrap())
        .unwrap();
    assert_eq!(fresh.pilot_callsign().registration, "N371FC");
    load(&mut fresh, "larrow");
    assert_eq!(fresh.pilot_callsign().registration, "G-ABIO");
    assert_eq!(fresh.pilot_callsign().full, "Larrow Golf Alfa Bravo India Oscar");
    let seed = serde_json::from_str(&serde_json::to_string(&fresh.worker_seed()).unwrap()).unwrap();
    let worker = Session::from_worker_seed(seed).unwrap();
    assert_eq!(worker.pilot_callsign().registration, "G-ABIO");
    let mut display = Session::new().unwrap();
    display.apply_screen_snapshot(worker.screen_snapshot()).unwrap();
    assert_eq!(display.pilot_callsign().registration, "G-ABIO");
    assert_eq!(display.callsign_preferences().len(), 2);
    fresh
        .execute("comms.callsign", json!({"reset_to_pack":true}))
        .unwrap();
    assert!(!fresh.pilot_callsign().overridden);
    assert_eq!(fresh.callsign_preferences().len(), 1);
    load(&mut fresh, "corvid");
    assert_eq!(fresh.pilot_callsign().registration, "N371FC");
    fresh
        .execute("sim.reset", json!({"start":{"kind":"runway"}}))
        .unwrap();
    assert_eq!(fresh.pilot_callsign().registration, "N371FC");
}

#[test]
fn staging_and_switching_variants_preserves_each_choice() {
    let mut s = Session::new().unwrap();
    s.execute("comms.callsign", json!({"registration":"N371FC"}))
        .unwrap();
    s.execute(
        "aircraft.load",
        json!({"source":{"kind":"installed","id":"corvid-standby"},"reset":false}),
    )
    .unwrap();
    assert_eq!(
        s.pilot_callsign().registration,
        "N371FC",
        "staging leaves the flying aircraft's identity alone"
    );
    s.execute("sim.reset", json!({"start":{"kind":"runway"}}))
        .unwrap();
    assert!(!s.pilot_callsign().overridden);
    s.execute("comms.callsign", json!({"registration":"N73FC"}))
        .unwrap();
    load(&mut s, "corvid");
    assert_eq!(s.pilot_callsign().registration, "N371FC");
    load(&mut s, "corvid-standby");
    assert_eq!(s.pilot_callsign().registration, "N73FC");
}

#[test]
fn browser_display_commands_return_the_new_identity_and_preserve_transient_values() {
    let mut worker = Session::new().unwrap();
    worker
        .execute("comms.callsign", json!({"registration":"N52R","persist":false}))
        .unwrap();
    let mut display = Session::new().unwrap();
    display.apply_screen_snapshot(worker.screen_snapshot()).unwrap();
    assert_eq!(display.pilot_callsign().registration, "N52R");
    assert!(display.callsign_preferences().is_empty());
    let output = display
        .execute("comms.callsign", json!({"spoken_type":"Sling","persist":false}))
        .unwrap();
    assert_eq!(output["registration"], "N52R");
    assert_eq!(output["spoken_type"], "Sling");
    assert_eq!(display.pilot_callsign().full, "Sling Five Two Romeo");
    let output = display
        .execute("comms.callsign", json!({"reset_to_pack":true}))
        .unwrap();
    assert_eq!(output["overridden"], false);
}

#[test]
fn aim_type_replaces_november_and_contact_is_channel_local() {
    assert_eq!(
        phraseology::callsign_forms("N1-ABC", "Sling").0,
        "Sling November One Alfa Bravo Charlie"
    );
    assert_eq!(
        phraseology::callsign_forms("N371FC", "Sling"),
        (
            "Sling Three Seven One Foxtrot Charlie".into(),
            "Sling One Foxtrot Charlie".into()
        )
    );
    assert_eq!(
        phraseology::callsign_forms("N12345", "Corvid").1,
        "Corvid Three Four Five"
    );
    assert_eq!(phraseology::callsign_forms("N1", "Corvid").1, "Corvid One");
    assert_eq!(
        phraseology::callsign_forms("N371FC", "").0,
        "November Three Seven One Foxtrot Charlie"
    );
    let mut s = Session::new().unwrap();
    let full = s.pilot_callsign().full;
    assert_eq!(s.pilot_callsign().current, full);
    s.comms.contacts.insert(carrier_thirds_hz(122800).unwrap());
    assert_eq!(s.pilot_callsign().current, s.pilot_callsign().abbreviated);
    s.execute("comms.tune", json!({"radio":1,"channel_khz":123000}))
        .unwrap();
    assert_eq!(s.pilot_callsign().current, full);
    s.execute("comms.tune", json!({"radio":1,"channel_khz":122800}))
        .unwrap();
    assert_eq!(s.pilot_callsign().current, full);
    s.execute("comms.tune", json!({"radio":2,"channel_khz":123000}))
        .unwrap();
    s.comms.contacts.insert(carrier_thirds_hz(123000).unwrap());
    s.execute("comms.audio_panel", json!({"transmit":2})).unwrap();
    assert_eq!(
        s.pilot_callsign().current,
        full,
        "switching COM transmit to a different carrier starts a new contact"
    );
    s.execute("comms.audio_panel", json!({"transmit":1})).unwrap();
    s.execute("comms.callsign", json!({"abbreviate_after_contact":false}))
        .unwrap();
    assert!(!s.pilot_callsign().overridden);
    s.comms.contacts.insert(carrier_thirds_hz(122800).unwrap());
    assert_eq!(s.pilot_callsign().current, full);
}
