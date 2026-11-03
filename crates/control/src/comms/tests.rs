#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use super::*;
use crate as flightcraft_control;
use serde_json::json;
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/radio_fixture.rs"
));

fn session() -> Session {
    Session::new().unwrap()
}
fn advance(s: &mut Session, seconds: u32) {
    s.run(crate::Step { steps: seconds * 120 }).unwrap();
}
fn call(s: &mut Session, khz: u32, text: &str, origin: [f64; 3], range: f64) {
    s.start_com(
        khz,
        Speaker::Traffic {
            callsign: "Fixture".into(),
        },
        text.into(),
        Meaning::Text,
        EventSource::Scenario,
        origin,
        range,
    );
}
#[test]
fn channels_spacing_and_atomic_invalid_tunes() {
    let mut s = session();
    for khz in [118005, 118010, 118015, 136990] {
        s.execute(
            "comms.tune",
            json!({"radio":1,"spacing":"khz833","channel_khz":khz}),
        )
        .unwrap();
    }
    assert_eq!(carrier_thirds_hz(118000), carrier_thirds_hz(118005));
    assert_eq!(carrier_thirds_hz(118010), Some(354025000));
    for khz in [118020, 118045, 118070, 118095, 117975, 137000] {
        assert!(
            s.execute("comms.tune", json!({"radio":1,"channel_khz":khz}))
                .is_err()
        );
    }
    assert!(s.execute("comms.tune", json!({"radio":0})).is_err());
    assert!(s.execute("comms.tune", json!({"radio":3})).is_err());
    assert!(
        s.execute("comms.tune", json!({"radio":1,"spacing":"khz25"}))
            .is_err()
    );
    assert_eq!(s.comms_state().com1.spacing, ComSpacing::Khz833);
}
#[test]
fn reception_requires_tuning_range_power_and_whole_transmission() {
    let mut s = session();
    call(&mut s, 122800, "Audible fixture", [100.0, 0.0, -10.0], 10.0);
    advance(&mut s, 2);
    assert_eq!(
        s.flight_record().events.last().unwrap().reception,
        Reception::Heard
    );
    call(&mut s, 123000, "Wrong channel", [100.0, 0.0, -10.0], 10.0);
    call(&mut s, 122800, "Too far away", [100000.0, 0.0, -10.0], 10.0);
    advance(&mut s, 2);
    assert!(
        s.flight_record()
            .events
            .iter()
            .skip(1)
            .all(|e| e.reception == Reception::NotReceived)
    );
    call(
        &mut s,
        122800,
        "Retune during this long transmission",
        [100.0, 0.0, -10.0],
        10.0,
    );
    s.execute("comms.tune", json!({"radio":1,"channel_khz":123000}))
        .unwrap();
    advance(&mut s, 3);
    assert_eq!(
        s.flight_record().events.last().unwrap().reception,
        Reception::NotReceived
    );
    s.execute(
        "systems.set",
        json!({"setting":{"kind":"switch","switch":"avionics","on":false}}),
    )
    .unwrap();
    assert!(!s.comms_state().powered);
    assert!(s.execute("comms.transmit", json!({"text":"test"})).is_err());
}
#[test]
fn simultaneous_speakers_block_both_but_separate_carriers_do_not() {
    let mut s = session();
    call(&mut s, 122800, "First call", [100.0, 0.0, -10.0], 10.0);
    call(&mut s, 122800, "Second call", [110.0, 0.0, -10.0], 10.0);
    advance(&mut s, 2);
    assert!(
        s.flight_record()
            .events
            .iter()
            .all(|e| e.reception == Reception::Blocked)
    );
    assert!(s.run(CommsListen {}).unwrap().transmission.is_none());
    call(&mut s, 122800, "Third call", [100.0, 0.0, -10.0], 10.0);
    call(&mut s, 123000, "Other carrier", [110.0, 0.0, -10.0], 10.0);
    advance(&mut s, 2);
    assert_eq!(
        s.run(CommsListen {}).unwrap().transmission.unwrap().text,
        "Third call"
    );
}
#[test]
fn terrain_and_curvature_block_reception() {
    let world = flightcraft_world::World::load(flightcraft_world::WorldChoice::Flat).unwrap();
    assert!(reception(
        &world,
        [0.0, 0.0, -2.0],
        [100.0, 0.0, -10.0],
        100.0,
        0.0
    ));
    assert!(!reception(
        &world,
        [0.0, 0.0, -2.0],
        [80000.0, 0.0, -10.0],
        100.0,
        0.0
    ));
    assert!(!reception(
        &world,
        [0.0, 0.0, -2.0],
        [1000.0, 0.0, 10.0],
        100.0,
        0.0
    ));
    assert!(!reception(
        &world,
        [f64::NAN, 0.0, -2.0],
        [100.0, 0.0, -10.0],
        100.0,
        0.0
    ));
}
#[test]
fn event_record_round_trip_and_controls_share_timeline_without_hash_change() {
    let mut a = session();
    let mut b = session();
    a.execute(
        "comms.transmit",
        json!({"text":"Fixture traffic, taxiing runway 36"}),
    )
    .unwrap();
    for s in [&mut a, &mut b] {
        s.execute("sim.set_controls", json!({"throttle":0.6})).unwrap();
        advance(s, 10);
    }
    assert_eq!(a.sim().state_hash(), b.sim().state_hash());
    let text = serde_json::to_string(a.flight_record()).unwrap();
    let record = FlightRecord::from_json(&text).unwrap();
    assert_eq!(record.events, a.flight_record().events);
    assert_eq!(record.inputs.len(), 2);
    assert!(record.inputs.iter().any(|i| i.controls.throttle == 0.6));
    assert!(FlightRecord::from_json(&text.replace("\"time_s\":0.0", "\"time_s\":-1.0")).is_err());
    a.execute("sim.reset", json!({"start":{"kind":"runway"}}))
        .unwrap();
    assert!(a.flight_record().events.is_empty());
    assert_eq!(a.flight_record().inputs.len(), 1);
}
#[test]
fn command_schemas_match_outputs_and_hostile_input_is_rejected() {
    let mut s = session();
    for (name, input) in [
        ("comms.tune", json!({"radio":1,"channel_khz":122800})),
        ("comms.audio_panel", json!({"receive_com2":true})),
        (
            "comms.transmit",
            json!({"text":"Fixture traffic, taxiing runway 36"}),
        ),
        ("comms.listen", json!({})),
        ("comms.facilities", json!({})),
        ("traffic.set", json!({"count":0,"airport":"1N7"})),
        ("flight.events", json!({})),
    ] {
        let info = Session::commands().into_iter().find(|c| c.name == name).unwrap();
        assert!(
            jsonschema::validator_for(&info.input_schema)
                .unwrap()
                .is_valid(&input),
            "{name}"
        );
        let output = s.execute(name, input).unwrap();
        assert!(
            jsonschema::validator_for(&info.output_schema)
                .unwrap()
                .is_valid(&output),
            "{name}: {output}"
        );
    }
    for (name, input) in [
        ("comms.audio_panel", json!({"volume":2})),
        ("comms.audio_panel", json!({"transmit":0})),
        ("comms.transmit", json!({"text":""})),
        ("flight.events", json!({"limit":1001})),
        ("traffic.set", json!({"count":4,"airport":"1N7"})),
    ] {
        assert!(s.execute(name, input).is_err());
    }
}
#[test]
fn morse_identifier_encodes_letters_digits_and_gaps() {
    assert_eq!(morse("SBJ"), "... -... .---");
    assert_eq!(morse("I-1"), ".. .----");
    assert!(morse_duration(&morse("SBJ")) > 3.0);
}

#[test]
fn one_n_seven_ramp_cannot_hear_distant_approach_over_a_ridge() {
    let w = radio_fixture(true);
    let altitude = w.ground(0.0, 0.0).height_msl_m;
    assert!(!reception(
        &w,
        [0.0, 0.0, -altitude - 2.0],
        [10000.0, 0.0, -altitude - 20.0],
        80.0,
        0.0
    ));
    assert!(reception(
        &w,
        [0.0, 0.0, -altitude - 1500.0],
        [10000.0, 0.0, -altitude - 1500.0],
        80.0,
        0.0
    ));
}
#[test]
fn taxi_calls_are_graded_for_name_runway_moment_and_frequency() {
    for (text, channel, correct) in [
        (
            "Blairstown traffic, Trainer pilot taxiing runway 25, Blairstown",
            122800,
            true,
        ),
        ("Wrongtown traffic, taxiing runway 25", 122800, false),
        ("Blairstown traffic, taxiing runway 07", 122800, false),
        ("Blairstown traffic, final runway 25", 122800, false),
        ("Blairstown traffic, taxiing runway 25", 123000, false),
    ] {
        let mut s = radio_session();
        s.run(SetTraffic {
            count: 1,
            airport: "1N7".into(),
            runway: Some("25".into()),
            right_pattern: false,
        })
        .unwrap();
        // Let the traffic's initial announcement finish before the pilot speaks.
        advance(&mut s, 12);
        s.execute("comms.tune", json!({"radio":1,"channel_khz":channel}))
            .unwrap();
        let event = s.execute("comms.transmit", json!({"text":text})).unwrap();
        let id = event["id"].as_u64().unwrap();
        advance(&mut s, 10);
        let grade = s
            .flight_record()
            .events
            .iter()
            .find(|e| e.id == id)
            .unwrap()
            .grade
            .as_ref()
            .unwrap();
        assert_eq!(grade.passed, correct, "{text} on {channel}: {grade:?}");
    }
}
#[test]
fn deterministic_traffic_has_separation_leg_calls_and_batch_invariance() {
    let mut a = radio_session();
    let mut b = radio_session();
    for s in [&mut a, &mut b] {
        s.execute("traffic.set", json!({"count":3,"airport":"1N7","runway":"25"}))
            .unwrap();
        s.execute("sim.set_controls", json!({"brake":1.0})).unwrap();
    }
    for _ in 0..60 {
        advance(&mut a, 5);
        for _ in 0..5 {
            advance(&mut b, 1);
        }
        let traffic = a.traffic_state();
        assert_eq!(traffic.aircraft.len(), 3);
        for (i, x) in traffic.aircraft.iter().enumerate() {
            for y in traffic.aircraft.iter().skip(i + 1) {
                assert!(
                    libm::hypot(
                        x.position_m[0] - y.position_m[0],
                        x.position_m[1] - y.position_m[1]
                    ) > 500.0
                );
            }
        }
    }
    assert_eq!(
        serde_json::to_value(a.traffic_state()).unwrap(),
        serde_json::to_value(b.traffic_state()).unwrap()
    );
    assert_eq!(a.flight_record().events, b.flight_record().events);
    assert!(
        a.flight_record()
            .events
            .iter()
            .filter(|e| matches!(e.speaker, Speaker::Traffic { .. }))
            .count()
            > 12
    );
    assert!(
        a.flight_record()
            .events
            .iter()
            .any(|e| matches!(e.meaning, Meaning::Cue { .. }))
    );
}
#[test]
fn automated_weather_uses_tuned_imported_station_and_sim_wind() {
    let mut s = radio_session();
    s.execute(
        "weather.set",
        json!({"wind_from_deg":250.0,"wind_speed_mps":5.0,"shear":false,"turbulence":"off","seed":0}),
    )
    .unwrap();
    s.execute("comms.tune", json!({"radio":1,"channel_khz":123000}))
        .unwrap();
    advance(&mut s, 45);
    let weather = s
        .flight_record()
        .events
        .iter()
        .find(|e| matches!(e.meaning, Meaning::Weather { .. }))
        .unwrap();
    assert_eq!(weather.frequency_hz, Some(123e6));
    assert_eq!(weather.reception, Reception::Heard);
    if let Meaning::Weather {
        wind_from_deg,
        wind_speed_mps,
        ..
    } = weather.meaning
    {
        assert!((wind_from_deg - 250.0).abs() < 0.01);
        assert_eq!(wind_speed_mps, 5.0);
    }
}

fn stage_aircraft(s: &mut Session, position_m: [f64; 3], heading: f64, speed: f64) {
    let state = flightcraft_sim::State {
        time_s: s.sim.state().time_s,
        position_m,
        velocity_mps: [speed * heading.cos(), speed * heading.sin(), 0.0],
        attitude: [(heading / 2.0).cos(), 0.0, 0.0, (heading / 2.0).sin()],
        angular_rate_rps: [0.0; 3],
    };
    s.sim =
        flightcraft_sim::Sim::from_state_in_world(state, Default::default(), s.sim.world().clone()).unwrap();
}
#[test]
fn grading_uses_measured_pattern_legs_and_boundary_words() {
    assert_eq!(PatternLeg::parse("database finalized"), None);
    for (leg, along, side, relative_heading) in [
        (PatternLeg::Departing, 1500.0, 0.0, 0.0),
        (
            PatternLeg::Crosswind,
            4000.0,
            600.0,
            -core::f64::consts::FRAC_PI_2,
        ),
        (PatternLeg::Downwind, 1000.0, 1852.0, core::f64::consts::PI),
        (PatternLeg::Base, -2200.0, 600.0, core::f64::consts::FRAC_PI_2),
        (PatternLeg::Final, -2200.0, 0.0, 0.0),
    ] {
        let mut s = radio_session();
        s.execute("traffic.set", json!({"count":1,"airport":"1N7","runway":"25"}))
            .unwrap();
        advance(&mut s, 12);
        let pattern = s.comms.pattern.as_ref().unwrap();
        let [f, g] = pattern.forward;
        let position = [
            pattern.origin[0] + along * f + side * g,
            pattern.origin[1] + along * g - side * f,
            -pattern.elevation_m - 304.8,
        ];
        stage_aircraft(&mut s, position, g.atan2(f) + relative_heading, 40.0);
        assert_eq!(s.comms_state().expected_call, Some(leg));
        let out = s
            .execute(
                "comms.transmit",
                json!({"text":format!("Blairstown traffic, {} runway two five, Blairstown",leg.phrase())}),
            )
            .unwrap();
        assert_eq!(out["grade"]["timely"], true);
        assert_eq!(out["grade"]["runway_correct"], true);
    }
    let mut s = radio_session();
    s.execute("traffic.set", json!({"count":1,"airport":"1N7","runway":"25"}))
        .unwrap();
    advance(&mut s, 12);
    let out = s
        .execute(
            "comms.transmit",
            json!({"text":"NotBlairstown traffic, taxiing runway 250"}),
        )
        .unwrap();
    assert_eq!(out["grade"]["airport_correct"], false);
    assert_eq!(out["grade"]["runway_correct"], false);
}
#[test]
fn traffic_sequences_behind_an_ownship_on_the_same_leg() {
    let mut s = radio_session();
    s.execute("traffic.set", json!({"count":2,"airport":"1N7","runway":"25"}))
        .unwrap();
    let aircraft = s.traffic_state().aircraft.first().unwrap().clone();
    let h = aircraft.heading_grid_rad;
    stage_aircraft(
        &mut s,
        [
            aircraft.position_m[0] + 800.0 * h.cos(),
            aircraft.position_m[1] + 800.0 * h.sin(),
            aircraft.position_m[2],
        ],
        h,
        32.0,
    );
    s.update_pattern(0.25);
    let updated = s.traffic_state().aircraft.first().unwrap().clone();
    assert!(updated.following_pilot);
    assert!(updated.speed_mps <= 32.1);
    assert!(
        s.flight_record()
            .events
            .iter()
            .any(|e| e.text.contains("following you"))
            || s.comms
                .traffic_calls
                .iter()
                .any(|e| e.text.contains("following you"))
    );
}

#[test]
fn ndb_tuning_produces_a_typed_morse_event_from_navigation_data() {
    use flightcraft_world::{
        Navdata,
        navdata::{Navaid, NavaidComponent, NavaidKind},
    };
    let station = radio_station(FacilityKind::Ctaf, 122800);
    let mut data = Navdata {
        version: 1,
        attribution: "Original radio fixture".into(),
        snapshot_date: "2026-01-01".into(),
        ..Default::default()
    };
    data.navaids.push(Navaid {
        id: "fixture-ndb".into(),
        ident: "FX".into(),
        name: "Fixture NDB".into(),
        kind: NavaidKind::Ndb,
        position: station.position,
        components: vec![NavaidComponent {
            id: "fixture-ndb-component".into(),
            source_key: "Original fixture".into(),
            subtype_code: 1,
            position: station.position,
            frequency_hz: Some(335000.0),
            channel: None,
            magnetic_variation_deg: None,
            effective_unix_ms: None,
        }],
    });
    let mut s = session();
    s.install_region(radio_fixture(false).with_navdata(data).unwrap())
        .unwrap();
    s.execute(
        "sim.reset",
        json!({"world":"radio-fixture","start":{"kind":"airport_runway","airport":"1N7","end":"25"}}),
    )
    .unwrap();
    s.execute("comms.tune", json!({"radio":1,"adf_khz":335})).unwrap();
    s.execute("comms.audio_panel", json!({"nav_ident":true})).unwrap();
    advance(&mut s, 8);
    let event = s
        .flight_record()
        .events
        .iter()
        .find(|e| matches!(e.meaning, Meaning::Ident { .. }))
        .unwrap();
    assert_eq!(event.frequency_hz, Some(335000.0));
    assert_eq!(event.reception, Reception::Heard);
    assert_eq!(
        event.meaning,
        Meaning::Ident {
            ident: "FX".into(),
            morse: "..-. -..-".into()
        }
    );
}

#[test]
fn installed_one_n_seven_terrain_blocks_a_distant_low_antenna() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain/new-york");
    if !root.join("000.fcht").exists() {
        eprintln!("Installed terrain unavailable; the synthetic ridge test still runs.");
        return;
    }
    let world = crate::read_region(&root).unwrap();
    let region = world.region().unwrap();
    let projection =
        flightcraft_world::geo::LocalProjection::new(region.origin.latitude_deg, region.origin.longitude_deg)
            .unwrap();
    let near = crate::regions::airport(&world, "1N7").unwrap();
    let far = crate::regions::airport(&world, "KEWR").unwrap();
    let [n, e] = projection.project(near.latitude_deg, near.longitude_deg).unwrap();
    let [tn, te] = projection.project(far.latitude_deg, far.longitude_deg).unwrap();
    let receiver = [n, e, -world.ground(n, e).height_msl_m - 2.0];
    let transmitter = [tn, te, -world.ground(tn, te).height_msl_m - 10.0];
    // Site is a synthetic low antenna at a real FAA airport, not an invented approach facility.
    assert!(!reception(&world, receiver, transmitter, 80.0, 0.0));
}

#[test]
fn equipment_declares_independent_audio_and_com_breakers() {
    let mut s = session();
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/packs/equipment-demo");
    s.install_pack(&crate::read_pack(&directory).unwrap()).unwrap();
    s.execute(
        "aircraft.load",
        json!({"source":{"kind":"installed","id":"equipment-demonstrator"},"reset":true}),
    )
    .unwrap();
    assert!(s.comms_state().powered && s.comms_state().com1_powered && s.comms_state().com2_powered);
    s.execute(
        "systems.set",
        json!({"setting":{"kind":"breaker","load":"com1","closed":false}}),
    )
    .unwrap();
    assert!(s.comms_state().powered && !s.comms_state().com1_powered && s.comms_state().com2_powered);
    assert!(
        s.execute("comms.transmit", json!({"text":"Fixture call"}))
            .is_err()
    );
    s.execute("comms.audio_panel", json!({"transmit":2})).unwrap();
    assert!(
        s.execute("comms.transmit", json!({"text":"Fixture call"}))
            .is_ok()
    );
    s.execute(
        "systems.set",
        json!({"setting":{"kind":"breaker","load":"audio_panel","closed":false}}),
    )
    .unwrap();
    assert!(!s.comms_state().powered);
}

#[test]
fn pack_callsigns_override_first_contact_abbreviation_and_channel_change() {
    let mut s = radio_session();
    let hash = s.sim().state_hash();
    let identity = s
        .execute(
            "comms.callsign",
            json!({"registration":"N371FC","spoken_type":"Sling"}),
        )
        .unwrap();
    assert_eq!(identity["current"], "Sling Three Seven One Foxtrot Charlie");
    s.execute(
        "comms.transmit",
        json!({"text":"Blairstown traffic, {callsign} taxiing runway 25, Blairstown"}),
    )
    .unwrap();
    advance(&mut s, 12);
    assert_eq!(
        s.pilot_callsign().current,
        "Sling Three Seven One Foxtrot Charlie"
    ); // CTAF never initiates an ATC abbreviation.
    s.execute("comms.tune", json!({"radio":1,"channel_khz":123500}))
        .unwrap();
    s.execute("comms.tune", json!({"radio":1,"channel_khz":122800}))
        .unwrap();
    assert_eq!(
        s.pilot_callsign().current,
        "Sling Three Seven One Foxtrot Charlie"
    );
    let before = s.pilot_callsign().registration;
    assert!(
        s.execute("comms.callsign", json!({"registration":"bad/../name"}))
            .is_err()
    );
    assert_eq!(s.pilot_callsign().registration, before);
    assert_ne!(s.sim().state_hash(), hash); // Time alone advances physics; identity changes below do not.
    let hash = s.sim().state_hash();
    s.execute(
        "comms.callsign",
        json!({"registration":"N52R","spoken_type":"Skyhawk"}),
    )
    .unwrap();
    assert_eq!(s.pilot_callsign().full, "Skyhawk Five Two Romeo");
    assert_eq!(s.sim().state_hash(), hash);
}

#[test]
fn abbreviation_waits_for_a_heard_controller_not_traffic_or_weather() {
    let mut s = radio_session();
    s.execute(
        "comms.callsign",
        json!({"registration":"N371FC","spoken_type":"Sling"}),
    )
    .unwrap();
    let full = s.pilot_callsign().full;
    let short = s.pilot_callsign().abbreviated;
    let origin = s.sim().state().position_m;
    for (speaker, text, expected_short) in [
        (
            Speaker::Traffic {
                callsign: "Other".into(),
            },
            short.clone(),
            false,
        ),
        (
            Speaker::Facility {
                id: "weather".into(),
                role: FacilityKind::Awos,
            },
            short.clone(),
            false,
        ),
        (
            Speaker::Facility {
                id: "tower".into(),
                role: FacilityKind::Tower,
            },
            full.clone(),
            false,
        ),
        (
            Speaker::Facility {
                id: "tower".into(),
                role: FacilityKind::Tower,
            },
            short.clone(),
            true,
        ),
    ] {
        s.start_com(
            122800,
            speaker,
            format!("{text}, roger."),
            Meaning::Text,
            EventSource::Rules,
            origin,
            100_000.,
        );
        assert_eq!(
            s.pilot_callsign().current,
            full,
            "pending controller transmission"
        );
        advance(&mut s, 12);
        assert_eq!(
            s.pilot_callsign().current,
            if expected_short {
                short.clone()
            } else {
                full.clone()
            }
        );
    }
    s.execute("comms.tune", json!({"radio":1,"channel_khz":123500}))
        .unwrap();
    assert_eq!(s.pilot_callsign().current, full);
    s.execute("comms.tune", json!({"radio":1,"channel_khz":122800}))
        .unwrap();
    s.start_com(
        122800,
        Speaker::Facility {
            id: "tower".into(),
            role: FacilityKind::Tower,
        },
        short.clone(),
        Meaning::Text,
        EventSource::Rules,
        origin,
        100_000.,
    );
    call(&mut s, 122800, "Interfering traffic", origin, 100_000.);
    advance(&mut s, 12);
    assert_eq!(
        s.pilot_callsign().current,
        full,
        "a blocked abbreviation gives no permission"
    );
}

#[test]
fn full_ifr_report_layers_station_agl_temperature_dewpoint_and_qnh() {
    use flightcraft_world::{CloudLayer, Conditions, Coverage, Obscuration};
    let station = radio_station(FacilityKind::Awos, 123000);
    let c = Conditions {
        layers: vec![
            CloudLayer {
                base_ft_msl: station.elevation_m / 0.3048 + 2000.,
                top_ft_msl: 4000.,
                coverage: Coverage::Sct,
            },
            CloudLayer {
                base_ft_msl: station.elevation_m / 0.3048 + 600.,
                top_ft_msl: 3500.,
                coverage: Coverage::Ovc,
            },
        ],
        visibility_sm: 0.5,
        obscuration: Obscuration::Fog,
        temperature_c: -2.,
        dewpoint_c: -3.,
        sea_level_pressure_pa: 100600.,
        ..Default::default()
    };
    let (text, meaning) = weather::weather_report(
        &station,
        &c,
        flightcraft_sim::Weather {
            wind_from_deg: 250.,
            wind_speed_mps: 5.14444,
            ..Default::default()
        },
        0.,
    );
    assert!(text.contains("Wind two five zero at one zero knots"), "{text}");
    assert!(text.contains("Visibility one half statute miles"), "{text}");
    assert!(text.contains("Ceiling overcast six hundred"), "{text}");
    assert!(text.contains("Temperature minus"));
    assert!(text.contains("Altimeter two niner seven one"), "{text}");
    match meaning {
        Meaning::Weather {
            ceiling_agl_m,
            visibility_m,
            temperature_c,
            dewpoint_c,
            layers,
            altimeter_pa,
            ..
        } => {
            assert!((ceiling_agl_m.unwrap() - 182.88).abs() < 0.001);
            assert_eq!(visibility_m, Some(804.672));
            assert!(dewpoint_c <= temperature_c);
            assert_eq!(layers.len(), 2);
            assert_eq!(altimeter_pa, 100600.);
        }
        _ => panic!("expected weather"),
    }
    let clear = weather::weather_report(&station, &Conditions::default(), Default::default(), 0.).0;
    assert!(clear.contains("Wind calm") && clear.contains("Sky clear") && clear.contains("one zero or more"));
}

#[test]
fn bundled_nasr_frequencies_have_all_facility_kinds_and_no_phones() {
    let text = include_str!("../../../../assets/terrain/new-york/navdata.json");
    let nav = flightcraft_world::Navdata::from_json(text).unwrap();
    for kind in [
        FacilityKind::Ctaf,
        FacilityKind::Unicom,
        FacilityKind::Awos,
        FacilityKind::Asos,
        FacilityKind::Atis,
        FacilityKind::Tower,
        FacilityKind::Ground,
        FacilityKind::Clearance,
        FacilityKind::Approach,
        FacilityKind::Departure,
    ] {
        assert!(nav.comms.iter().any(|s| s.kind == kind), "{kind:?}");
    }
    assert!(
        nav.comms
            .iter()
            .any(|s| s.airport == "1N7" && s.kind == FacilityKind::Ctaf && s.channel_khz == 123000)
    );
    assert!(nav.comms.iter().any(|s| s.airport == "1N7"
        && s.kind == FacilityKind::Approach
        && s.channel_khz == 124450
        && s.name == "NEWARK"));
    assert!(nav.comms.iter().all(|s| {
        s.metadata
            .as_ref()
            .is_some_and(|m| m.effective_date == "2026-10-29")
    }));
    assert!(!text.contains("PHONE_NO"));
}

#[test]
fn instructor_waits_for_a_timely_call_to_finish_before_declaring_it_missing() {
    let mut s = radio_session();
    s.execute("sim.set_controls", json!({"brake":1.0})).unwrap();
    s.execute("traffic.set", json!({"count":1,"airport":"1N7","runway":"25"}))
        .unwrap();
    advance(&mut s, 16);
    s.execute(
        "comms.transmit",
        json!({"text":"Blairstown traffic, Sling Three Seven One Foxtrot Charlie, taxiing runway 25, Blairstown"}),
    )
    .unwrap();
    // Cross the twenty-second phase deadline while the spoken call is still on the carrier.
    s.run(crate::Step { steps: 4 * 120 + 40 }).unwrap();
    assert!(
        s.flight_record()
            .events
            .iter()
            .any(|e| e.speaker == Speaker::Pilot && e.reception == Reception::Pending)
    );
    assert!(!s.flight_record().events.iter().any(|e| matches!(
        e.meaning,
        Meaning::Cue {
            leg: PatternLeg::Taxi,
            ..
        }
    )));
    advance(&mut s, 10);
    assert!(
        s.flight_record()
            .events
            .iter()
            .any(|e| e.speaker == Speaker::Pilot && e.grade.as_ref().is_some_and(|g| g.passed))
    );
    assert!(!s.flight_record().events.iter().any(|e| matches!(
        e.meaning,
        Meaning::Cue {
            leg: PatternLeg::Taxi,
            ..
        }
    )));
}
