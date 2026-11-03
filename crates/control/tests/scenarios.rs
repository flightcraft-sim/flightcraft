#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_control::{scenario::*, *};
use serde_json::json;
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/scenario_fixture.rs"
));
fn timed() -> Scenario {
    let mut d = library().unwrap().remove(0);
    d.steps = vec![ScenarioStep {
        id: "fire".into(),
        when: vec![Trigger::RandomWindow {
            earliest_s: 1.,
            latest_s: 4.,
        }],
        actions: vec![
            ScenarioAction::FailureSet {
                failure: Failure::Engine {},
            },
            ScenarioAction::Cue {
                text: "Maintain control".into(),
            },
        ],
    }];
    d
}
#[test]
fn all_library_documents_validate_against_faa_catalogues_and_schema() {
    let s = scenario_session();
    let schema = schemars::schema_for!(Scenario).to_value();
    let validator = jsonschema::validator_for(&schema).unwrap();
    for doc in library().unwrap() {
        assert!(validator.is_valid(&serde_json::to_value(&doc).unwrap()));
        let r = validate(&doc, &s);
        assert!(r.valid, "{}: {:?}", doc.id, r.errors);
    }
}
#[test]
fn seeded_determinism_step_batching_and_read_only_commands() {
    let mut a = scenario_session();
    let mut b = scenario_session();
    let d = timed();
    a.run(ScenarioStart { scenario: d.clone() }).unwrap();
    b.run(ScenarioStart { scenario: d }).unwrap();
    a.run(Step { steps: 1200 }).unwrap();
    for _ in 0..1200 {
        b.run(Step { steps: 1 }).unwrap();
        b.run(GetScenarioState {}).unwrap();
    }
    assert_eq!(a.sim().state_hash(), b.sim().state_hash());
    assert_eq!(
        serde_json::to_value(a.scenario_state()).unwrap(),
        serde_json::to_value(b.scenario_state()).unwrap()
    );
    assert!(a.scenario_state().complete);
    assert!(a.scenario_state().emergency.is_some());
    assert_eq!(
        serde_json::to_value(a.flight_record()).unwrap(),
        serde_json::to_value(b.flight_record()).unwrap()
    );
}
#[test]
fn validation_reports_references_terrain_airspace_frequencies_and_branches() {
    let mut s = scenario_session();
    let mut d = timed();
    d.route[0].reference = "invented".into();
    d.route[1].altitude_msl_m = 10.;
    d.steps[0].when.push(Trigger::OnFinal {
        airport: "KABE".into(),
        runway: "99".into(),
        within_nm: 3.,
    });
    d.steps[0].actions.push(ScenarioAction::Transmission {
        facility: "invented".into(),
        channel_khz: 123450,
        role: flightcraft_world::FacilityKind::Tower,
        instruction: Instruction::Land {
            airport: "KABE".into(),
            runway: "99".into(),
        },
    });
    d.steps[0].actions.push(ScenarioAction::Branch {
        condition: Trigger::Time {
            seconds: 0.,
            since_previous: false,
        },
        then_step: "fire".into(),
        else_step: "absent".into(),
    });
    let r = s.run(ScenarioValidate { scenario: d.clone() }).unwrap();
    assert!(!r.valid && r.errors.len() >= 4);
    assert!(s.run(ScenarioStart { scenario: d }).is_err());
    let mut d = timed();
    d.route[0].reference = "KABE".into();
    d.route[1].reference = "KABE".into();
    d.route[0].altitude_msl_m = 600.;
    d.route[1].altitude_msl_m = 600.;
    assert!(
        validate(&d, &s)
            .errors
            .iter()
            .any(|e| e.contains("clearance required"))
    );
    d.route[0].altitude_msl_m = 100.;
    d.route[1].altitude_msl_m = 100.;
    assert!(
        validate(&d, &s)
            .errors
            .iter()
            .any(|e| e.contains("terrain clearance"))
    );
    d = timed();
    d.steps[0].when = vec![Trigger::Distance {
        reference: "KMMU".into(),
        within_nm: 0.01,
    }];
    assert!(!validate(&d, &s).valid);
    assert!(s.execute("comms.atc",json!({"facility":"nasr-2026-10-29-4611-Tower","channel_khz":120500,"role":"tower","instruction":{"kind":"altitude","altitude_msl_m":1.,"at":"KABE"}})).is_err());
}
#[test]
fn amendments_are_atomic_and_cannot_rewrite_executed_history() {
    let mut s = scenario_session();
    let d = timed();
    s.run(ScenarioStart { scenario: d.clone() }).unwrap();
    let mut amended = d.clone();
    amended.steps[0].when = vec![Trigger::Time {
        seconds: 0.,
        since_previous: false,
    }];
    s.run(ScenarioAmend {
        scenario: amended.clone(),
    })
    .unwrap();
    s.run(Step { steps: 1 }).unwrap();
    let before = serde_json::to_value(s.scenario_state()).unwrap();
    assert!(s.run(ScenarioAmend { scenario: d }).is_err());
    assert_eq!(before, serde_json::to_value(s.scenario_state()).unwrap());
    amended.route[0].reference = "BAD".into();
    assert!(s.run(ScenarioAmend { scenario: amended }).is_err());
}
#[test]
fn provenance_live_atc_and_both_radio_failure() {
    let mut s = scenario_session();
    let mut d = timed();
    d.ai_written = true;
    d.steps[0].when.clear();
    s.run(ScenarioStart { scenario: d }).unwrap();
    assert!(
        s.scenario_state()
            .events
            .iter()
            .all(|e| e.source == EventSource::ScenarioAi)
    );
    assert!(
        s.flight_record()
            .events
            .iter()
            .all(|e| e.source == EventSource::ScenarioAi)
    );
    assert!(s.execute("comms.atc",json!({"facility":"nasr-2026-10-29-4611-Tower","channel_khz":120500,"role":"tower","instruction":{"kind":"land","airport":"KABE","runway":"24"}})).is_ok());
    let e = s.flight_record().events.last().unwrap();
    assert_eq!(e.source, EventSource::Ai);
    assert!(!e.text.contains("Simulated ATC"));
    assert!(e.is_simulated_atc());
    assert!(e.text.contains("Allentown Tower, runway two four"));
    s.execute("failure.set", json!({"failure":{"kind":"radio_failure"}}))
        .unwrap();
    let c = s.comms_state();
    assert!(!c.com1_powered && !c.com2_powered);
    assert!(s.execute("comms.transmit", json!({"text":"Mayday"})).is_err());
    s.execute("failure.clear", json!({"target":"radio_failure"}))
        .unwrap();
    assert!(s.comms_state().com1_powered);
}
#[test]
fn glide_geometry_wind_terrain_and_hostile_inputs() {
    let flat = flightcraft_world::World::default();
    let a = glide_footprint(&flat, [0., 0., -1000.], [0.; 3], 9., 33.).unwrap();
    let b = glide_footprint(&flat, [0., 0., -1000.], [10., 0., 0.], 9., 33.).unwrap();
    assert_eq!(a.boundary_m.len(), 72);
    for [n, e] in &a.boundary_m {
        assert!((n.hypot(*e) - 8800.).abs() < 101.);
    }
    assert!(b.boundary_m[0][0] > a.boundary_m[0][0]);
    assert!(b.boundary_m[36][0].abs() < a.boundary_m[36][0].abs());
    assert!(glide_footprint(&flat, [0., 0., f64::NAN], [0.; 3], 9., 33.).is_err());
    let region = scenario_fixture();
    let g = glide_footprint(&region, [0., 0., -1200.], [0.; 3], 9., 33.).unwrap();
    assert!(g.fields.iter().any(|f| f.id == "1N7:07"));
    let valley = flightcraft_world::World::load(flightcraft_world::WorldChoice::TrainingValley).unwrap();
    let h = glide_footprint(&valley, [0., 0., -1000.], [0.; 3], 9., 33.).unwrap();
    assert_ne!(a.boundary_m, h.boundary_m);
}
#[test]
fn instructor_window_and_checklist_order_grading_are_deterministic() {
    let mut s = scenario_session();
    s.execute(
        "scenario.instructor_failure",
        json!({"failure":{"kind":"engine"},"within_seconds":0.,"seed":9}),
    )
    .unwrap();
    s.run(Step { steps: 120 }).unwrap();
    s.run(EmergencyRecord {
        observation: EmergencyObservation::Recognized,
    })
    .unwrap();
    s.run(EmergencyRecord {
        observation: EmergencyObservation::Checklist { item: "field".into() },
    })
    .unwrap();
    s.run(EmergencyRecord {
        observation: EmergencyObservation::Checklist { item: "glide".into() },
    })
    .unwrap();
    s.run(EmergencyRecord {
        observation: EmergencyObservation::ChooseField { id: "1N7:07".into() },
    })
    .unwrap();
    let d = s.scenario_state().emergency.as_ref().unwrap();
    assert!(d.recognition_s.unwrap() >= 0.99);
    assert!(!d.steps[0].in_order);
    assert!(d.steps[1].in_order);
    assert!(d.field_reachable);
    assert!(d.speed_in_tolerance > 0);
    assert!(
        s.run(EmergencyRecord {
            observation: EmergencyObservation::Checklist {
                item: "imaginary".into()
            }
        })
        .is_err()
    );
    assert!(
        s.run(EmergencyRecord {
            observation: EmergencyObservation::Checklist { item: "glide".into() }
        })
        .is_err()
    );
}
#[test]
fn scripted_engine_out_landing_grades_radio_flow_and_missed_field() {
    let mut s = Session::new().unwrap();
    s.execute(
        "sim.reset",
        json!({"start":{"kind":"air","height_m":100.,"airspeed_mps":33.,"heading_rad":0.}}),
    )
    .unwrap();
    s.execute("failure.set", json!({"failure":{"kind":"engine"}}))
        .unwrap();
    s.execute("emergency.record", json!({"observation":{"kind":"recognized"}}))
        .unwrap();
    for item in ["glide", "field", "fuel", "ignition", "radio", "secure"] {
        s.execute(
            "emergency.record",
            json!({"observation":{"kind":"checklist","item":item}}),
        )
        .unwrap();
    }
    // A reachable patch behind the aircraft is chosen, but the pilot continues straight ahead.
    // The debrief must distinguish a successful touchdown from following that field decision.
    let field = s
        .scenario_state()
        .emergency
        .as_ref()
        .unwrap()
        .glide
        .fields
        .iter()
        .filter(|f| !f.airport)
        .min_by(|a, b| a.position_m[0].total_cmp(&b.position_m[0]))
        .unwrap()
        .id
        .clone();
    s.execute(
        "emergency.record",
        json!({"observation":{"kind":"choose_field","id":field}}),
    )
    .unwrap();
    s.execute(
        "avionics.set",
        json!({"setting":{"kind":"glass","setting":{"kind":"transponder","code":"7700","mode":"alt"}}}),
    )
    .unwrap();
    s.execute(
        "comms.transmit",
        json!({"text":"Mayday Mayday Mayday engine failure, landing ahead"}),
    )
    .unwrap();
    s.execute(
        "autopilot.set",
        json!({"airspeed_mps":33.,"airspeed_mode":"pitch","wings_level":true}),
    )
    .unwrap();
    let mut flared = false;
    for _ in 0..2000 {
        let r = s.sim().readout();
        if !flared && r.height_agl_m < 9. {
            s.execute("autopilot.set", json!({"airspeed_mps":null,"pitch_rad":0.12}))
                .unwrap();
            flared = true;
        }
        if r.on_ground || r.crashed {
            break;
        }
        s.run(Step { steps: 12 }).unwrap();
    }
    let d = s.scenario_state().emergency.as_ref().unwrap();
    assert_eq!(d.outcome, "landed", "{:?}", s.sim().readout());
    assert!(d.emergency_call && d.squawk_7700);
    assert!(d.field_reachable);
    assert!(!d.landed_at_chosen_field);
    assert_eq!(d.steps.len(), 6);
    assert!(d.steps.iter().all(|i| i.in_order));
    assert!(d.speed_in_tolerance > 0);
}
#[test]
fn waits_use_new_completed_calls_and_checklist_signals() {
    let mut s = scenario_session();
    let mut d = timed();
    d.steps = vec![
        ScenarioStep {
            id: "failure".into(),
            when: vec![],
            actions: vec![
                ScenarioAction::FailureSet {
                    failure: Failure::Engine {},
                },
                ScenarioAction::Wait {
                    condition: Trigger::Readback {
                        contains: "roger".into(),
                        result: ReadbackResult::Correct,
                        timeout_s: 10.,
                    },
                },
            ],
        },
        ScenarioStep {
            id: "acknowledged".into(),
            when: vec![Trigger::Checklist { item: "glide".into() }],
            actions: vec![ScenarioAction::EmitEvent {
                name: "emergency-flow-complete".into(),
            }],
        },
    ];
    s.run(ScenarioStart { scenario: d }).unwrap();
    s.run(Step { steps: 120 }).unwrap();
    assert!(s.scenario_state().waiting);
    s.execute("comms.transmit", json!({"text":"Roger"})).unwrap();
    s.run(Step { steps: 1200 }).unwrap();
    assert_eq!(s.scenario_state().step, 1);
    s.run(EmergencyRecord {
        observation: EmergencyObservation::Checklist { item: "glide".into() },
    })
    .unwrap();
    assert!(s.scenario_state().complete);
    assert!(
        s.scenario_state()
            .events
            .iter()
            .any(|e| matches!(&e.action,ScenarioAction::EmitEvent {name} if name=="emergency-flow-complete"))
    );
}
#[test]
fn debrief_survives_record_export_import_and_rejects_hostile_geometry() {
    let mut s = scenario_session();
    s.execute("failure.set", json!({"failure":{"kind":"engine"}}))
        .unwrap();
    s.run(Step { steps: 120 }).unwrap();
    s.run(EmergencyRecord {
        observation: EmergencyObservation::Recognized,
    })
    .unwrap();
    let text = serde_json::to_string(s.flight_record()).unwrap();
    let loaded = FlightRecord::from_json(&text).unwrap();
    let d = loaded.emergency.as_ref().unwrap();
    assert!(d.recognition_s.is_some());
    assert_eq!(d.glide.boundary_m.len(), 72);
    assert_eq!(d.procedure.id, "engine_failure");
    let mut value = serde_json::to_value(&loaded).unwrap();
    value["emergency"]["glide"]["boundary_m"] = json!([]);
    assert!(FlightRecord::from_json(&value.to_string()).is_err());
}
#[test]
fn live_altitude_requires_clearance_and_advisory_cannot_hide_instructions() {
    let mut s = scenario_session();
    let station = "nasr-2026-10-29-4611-Tower";
    let altitude = json!({"facility":station,"channel_khz":120500,"role":"tower","instruction":{"kind":"altitude","at":"KABE","altitude_msl_m":600.}});
    assert!(s.execute("comms.atc", altitude.clone()).is_err());
    assert!(s.execute("comms.atc",json!({"facility":station,"channel_khz":120500,"role":"tower","instruction":{"kind":"advisory","text":"Descend to one hundred feet"}})).is_err());
    let world = s.sim().world();
    let region = world.region().unwrap();
    let projection =
        flightcraft_world::geo::LocalProjection::new(region.origin.latitude_deg, region.origin.longitude_deg)
            .unwrap();
    let airport = world
        .airports()
        .unwrap()
        .airports
        .iter()
        .find(|a| a.icao.as_deref() == Some("KABE"))
        .unwrap();
    let [n, e] = projection
        .project(airport.latitude_deg, airport.longitude_deg)
        .unwrap();
    let ids: Vec<_> = world
        .airspace_at(n, e, 600.)
        .unwrap()
        .containing
        .into_iter()
        .filter(|v| {
            matches!(
                v.class,
                Some(
                    flightcraft_world::AirspaceClass::B
                        | flightcraft_world::AirspaceClass::C
                        | flightcraft_world::AirspaceClass::D
                )
            )
        })
        .map(|v| v.id)
        .collect();
    assert!(!ids.is_empty());
    for id in ids {
        s.execute("comms.atc",json!({"facility":station,"channel_khz":120500,"role":"tower","instruction":{"kind":"airspace_clearance","id":id}})).unwrap();
    }
    assert!(s.execute("comms.atc", altitude).is_ok());
}

#[test]
fn terrain_candidates_are_screened_rectangles_and_machine_events_are_hidden() {
    let flat = flightcraft_world::World::default();
    let g = glide_footprint(&flat, [0., 0., -1000.], [3., -4., 0.], 9., 33.).unwrap();
    let fields: Vec<_> = g.fields.iter().filter(|f| !f.airport).collect();
    assert!(!fields.is_empty() && fields.len() <= 6);
    for f in fields {
        assert!(f.length_m >= 600. && f.width_m >= 60. && f.slope_percent <= 3.);
        assert!((f.heading_grid_rad - (-(-4_f64)).atan2(-3.)).abs() < 1e-8);
        assert!(f.description(g.origin_m).contains("unverified surface"));
    }
    let mut s = scenario_session();
    let mut doc = timed();
    doc.steps[0].when.clear();
    s.run(ScenarioStart { scenario: doc }).unwrap();
    let stations = s.comms_state().stations;
    assert!(s.flight_record().events.iter().any(|e| !e.is_spoken()));
    assert!(
        s.flight_record()
            .events
            .iter()
            .filter(|e| !e.is_spoken())
            .all(|e| e.transcript_line(&stations).is_none())
    );
    let text = s
        .flight_record()
        .events
        .iter()
        .filter_map(|e| e.transcript_line(&stations))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!text.contains("Scenario state event"));
    assert!(!text.contains("Facility {"));
}

#[test]
fn terrain_screen_rejects_water_and_steep_surfaces() {
    use flightcraft_world::*;
    for (surface, slope) in [(Surface::Water, 0.), (Surface::Grass, 0.1)] {
        let mut region: Region =
            serde_json::from_str(include_str!("../../../assets/terrain/eastern-pa/region.json")).unwrap();
        region.southwest_m = [-8000., -8000.];
        region.tiles_north = 1;
        region.tiles_east = 1;
        region.spacing_m = 100.;
        region.tile_size = 257;
        region.tiles = vec!["000.fcht".into()];
        let heights = (0..257)
            .flat_map(|row| (0..257).map(move |_| row as f32 * 100. * slope))
            .collect();
        let tile = HeightTile::new(region.southwest_m, 100., 257, heights, vec![surface; 256 * 256]).unwrap();
        let world = World::from_region(region, vec![tile]).unwrap();
        let g = glide_footprint(&world, [0., 0., -3000.], [0., 0., 0.], 9., 33.).unwrap();
        assert!(g.fields.is_empty());
    }
}
