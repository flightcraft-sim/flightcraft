#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/region_data.rs"));
use flightcraft_control::*;
use serde_json::json;
fn session() -> Session {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain");
    let mut s = Session::new().unwrap();
    for region in ["eastern-pa", "new-york"] {
        s.install_region(read_region(&root.join(region)).unwrap())
            .unwrap();
    }
    s
}
fn start(s: &mut Session, id: &str) {
    let lesson = lesson_library()
        .unwrap()
        .into_iter()
        .find(|l| l.id == id)
        .unwrap();
    s.run(LessonStart { lesson }).unwrap();
}
#[test]
fn lessons_validate_start_and_read_without_advancing_time() {
    require_region_data!();
    let mut s = session();
    for lesson in lesson_library().unwrap() {
        s.run(LessonStart { lesson }).unwrap();
        assert!(s.sim().hood());
        let hash = s.sim().state_hash();
        let a = s.execute("lesson.debrief", json!({})).unwrap();
        assert_eq!(a, s.execute("lesson.debrief", json!({})).unwrap());
        assert_eq!(hash, s.sim().state_hash());
        assert!(a["geometry"].as_array().unwrap().len() > 2);
    }
}
#[test]
fn bad_lesson_is_atomic_and_incomplete_never_passes() {
    require_region_data!();
    let mut s = session();
    start(&mut s, "kabe-ils-06");
    let hash = s.sim().state_hash();
    let mut lesson = lesson_library().unwrap().remove(0);
    lesson.tasks[0].tolerances.cdi_scale = 1.;
    assert!(s.run(LessonStart { lesson }).is_err());
    assert_eq!(hash, s.sim().state_hash());
    let d = s.run(LessonFinish {}).unwrap();
    assert!(d.tasks.iter().all(|t| t.result == TaskResult::Fail));
    assert!(d.tasks[0].reasons.iter().any(|r| r.contains("Incomplete")));
}
fn grade(kind: InstrumentTaskKind) -> TaskGrade {
    TaskGrade {
        title: "Test".into(),
        kind,
        acs: kind.acs().into(),
        tolerances: Default::default(),
        result: TaskResult::Pending,
        reasons: vec![],
        observed_s: 0.,
        completed: false,
    }
}
#[test]
fn synthetic_bad_pilots_fail_for_specific_measured_reasons() {
    let good = InstrumentSample {
        grading: true,
        signal_valid: true,
        interval_s: 0.5,
        altitude_ft: 1000.,
        vertical_scale: Some(0.),
        ..Default::default()
    };
    for (sample, reason) in [
        (
            InstrumentSample {
                cdi_scale: 0.751,
                ..good.clone()
            },
            "Lateral",
        ),
        (
            InstrumentSample {
                vertical_scale: Some(-0.751),
                ..good.clone()
            },
            "Vertical",
        ),
        (
            InstrumentSample {
                airspeed_error_kt: 10.01,
                ..good.clone()
            },
            "Airspeed",
        ),
        (
            InstrumentSample {
                signal_valid: false,
                ..good.clone()
            },
            "Navigation",
        ),
        (
            InstrumentSample {
                at_minimum: true,
                ..good.clone()
            },
            "minimums",
        ),
    ] {
        let mut g = grade(InstrumentTaskKind::Precision);
        g.observe(&sample, 594., true);
        assert_eq!(g.result, TaskResult::Fail);
        assert!(g.reasons.iter().any(|r| r.contains(reason)), "{:?}", g.reasons);
    }
    let mut g = grade(InstrumentTaskKind::Nonprecision);
    g.observe(
        &InstrumentSample {
            altitude_ft: 839.,
            ..good.clone()
        },
        840.,
        false,
    );
    assert!(g.reasons.iter().any(|r| r.contains("MDA")));
    let mut g = grade(InstrumentTaskKind::Precision);
    g.observe(
        &InstrumentSample {
            cdi_scale: 0.75,
            vertical_scale: Some(-0.75),
            airspeed_error_kt: 10.,
            ..good
        },
        594.,
        true,
    );
    assert_eq!(g.result, TaskResult::Pending);
}
#[test]
fn hold_sector_classification_covers_every_heading_and_turn_direction() {
    use flightcraft_sim::avionics::procedures::{HoldEntry, hold_entry};
    use flightcraft_world::procedures::Turn;
    for inbound in 0..360 {
        for arrival in 0..360 {
            let right = hold_entry(f64::from(inbound), f64::from(arrival), Turn::R);
            let left = hold_entry(-f64::from(inbound), -f64::from(arrival), Turn::L);
            assert_eq!(right, left);
            let relative = wrap_deg(f64::from(arrival - inbound));
            let expected = if (-70. ..=110.).contains(&relative) {
                HoldEntry::Direct
            } else if relative > 110. {
                HoldEntry::Teardrop
            } else {
                HoldEntry::Parallel
            };
            assert_eq!(right, expected);
            let (outbound, turn) = match expected {
                HoldEntry::Direct => (f64::from(inbound) + 180., (180. - relative).rem_euclid(360.)),
                // Near an outbound arrival, classification waits for the opposite-direction reversal.
                HoldEntry::Parallel => (f64::from(inbound) + 180., wrap_deg(180. - relative).min(-30.)),
                HoldEntry::Teardrop => (f64::from(inbound) + 150., wrap_deg(150. - relative)),
            };
            assert_eq!(
                actual_hold_entry(f64::from(inbound), outbound, turn, Turn::R),
                expected
            );
        }
    }
    assert_eq!(actual_hold_entry(63., 243., 180., Turn::R), HoldEntry::Direct);
    assert_eq!(actual_hold_entry(63., 213., 150., Turn::R), HoldEntry::Teardrop);
    assert_eq!(actual_hold_entry(63., 243., -150., Turn::R), HoldEntry::Parallel);
}
#[test]
fn instructor_failure_and_unusual_attitude_preserve_flight_and_time() {
    require_region_data!();
    let mut s = session();
    start(&mut s, "unusual-attitude");
    let state = *s.sim().state();
    s.run(InstructorAttitude {
        pitch_deg: 20.,
        bank_deg: -45.,
    })
    .unwrap();
    assert_eq!(state.time_s, s.sim().state().time_s);
    assert_eq!(state.position_m, s.sim().state().position_m);
    assert!(s.glass_state().air_data.attitude_valid);
    assert!((s.glass_state().air_data.roll_rad.to_degrees() + 45.).abs() < 0.01);
    assert!((s.glass_state().air_data.pitch_rad.to_degrees() - 20.).abs() < 0.01);
    assert!(s.sim().hood());
    assert!(s.paused());
    s.run(InstructorInstrumentFailure {
        failure: InstrumentFailure::Vacuum,
        earliest_s: 1.,
        latest_s: 1.,
        seed: 1,
    })
    .unwrap();
    assert!(!s.sim().failures().vacuum_pump);
    s.run(Step { steps: 120 }).unwrap();
    assert!(s.sim().failures().vacuum_pump);
}
#[test]
fn crew_kabe_ils_smoke() {
    require_region_data!();
    let mut s = session();
    start(&mut s, "kabe-ils-06");
    s.run(SetCrewMode {
        mode: CrewMode::Demonstrating,
        director: false,
    })
    .unwrap();
    let accepted = s
        .run(CrewIntent {
            intent: PilotIntent::Approach {
                airport: "KABE".into(),
                procedure: "I06".into(),
                mode: InstrumentApproach::Ils,
                minimum_ft: 594.,
                coupled: false,
            },
        })
        .unwrap();
    assert!(accepted.accepted, "{:?}", accepted.refusal);
    s.run(CrewHandover {
        to: ControlOwner::Crew,
        actor: ControlOwner::Pilot,
        confirm: false,
    })
    .unwrap();
    s.run(CrewHandover {
        to: ControlOwner::Crew,
        actor: ControlOwner::Crew,
        confirm: true,
    })
    .unwrap();
    let data = s.sim().world().procedures().unwrap();
    let procedure = data
        .procedures
        .iter()
        .find(|p| p.airport == "KABE" && p.ident == "I06")
        .unwrap();
    let final_route = procedure.routes.iter().find(|r| r.transition.is_empty()).unwrap();
    let faf = final_route
        .legs
        .iter()
        .find(|l| l.waypoint_description.as_bytes().get(3) == Some(&b'F'))
        .unwrap();
    let faf = data.point(faf.fix.as_deref().unwrap()).unwrap().position;
    let endpoint = s
        .lesson_debrief()
        .geometry
        .iter()
        .find(|p| p.name == "RW06")
        .unwrap()
        .position;
    let faf_distance_nm = endpoint.inverse(faf).0 / 1852.;
    let mut handoff_distance = None;
    for second in 0..900 {
        s.run(Step { steps: 120 }).unwrap();
        if handoff_distance.is_none()
            && s.atc_state()
                .pending_instructions
                .iter()
                .any(|p| matches!(p.instruction, AtcInstruction::Handoff { .. }))
        {
            let r = s.sim().readout();
            let position = flightcraft_world::GeoPoint {
                latitude_deg: r.latitude_deg.unwrap(),
                longitude_deg: r.longitude_deg.unwrap(),
            };
            handoff_distance = Some(endpoint.inverse(position).0 / 1852.);
        }
        if second % 30 == 0 {
            let r = s.sim().readout();
            eprintln!(
                "{second}: {:?} {:.0}ft {:.1}kt {:.2}cdi {:?} {}",
                s.crew_state().phase,
                r.altitude_msl_m / 0.3048,
                r.indicated_airspeed_mps / 0.514444,
                s.sim().radio_state()[0].lateral_dots.unwrap_or(9.),
                s.sim().radio_state()[0].vertical_dots,
                s.crew_state().reason
            );
        }
        assert!(!s.sim().readout().crashed);
        if s.sim().readout().on_ground {
            break;
        }
    }
    let d = s.lesson_debrief();
    eprintln!("grades {:?}", d.tasks);
    for e in &s.flight_record().events {
        if !e.text.is_empty() {
            eprintln!("{:.1} {:?} {}", e.time_s, e.speaker, e.text);
        }
    }
    assert!(s.crew_state().touchdown.is_some(), "{:?}", s.crew_state());
    let handoff_distance = handoff_distance.expect("tower handoff observed");
    assert!(
        (handoff_distance - faf_distance_nm).abs() < 0.4,
        "handoff {handoff_distance} NM; FAF {faf_distance_nm} NM"
    );
    let events: Vec<_> = s
        .flight_record()
        .events
        .iter()
        .filter(|e| e.is_spoken())
        .collect();
    assert!(
        !events
            .iter()
            .any(|e| e.text.to_lowercase().contains("flight following"))
    );
    assert!(
        !events
            .iter()
            .any(|e| e.text.contains("Final. Tracking the centerline."))
    );
    let mut previous = None;
    for text in [
        "I'll fly the published approach",
        "information alpha. Wind",
        "four thousand three hundred, information alpha",
        "Radar contact",
        "until established on the localizer, cleared ILS",
        "Localizer alive",
        "Glideslope captured",
        "Contact Allentown Tower",
        "Minimums, runway in sight, landing",
    ] {
        let i = events
            .iter()
            .position(|e| e.text.contains(text))
            .unwrap_or_else(|| panic!("missing {text}"));
        assert!(previous.is_none_or(|p| i > p), "callout out of order: {text}");
        previous = Some(i);
    }
    assert!(
        s.lesson_debrief()
            .tasks
            .iter()
            .all(|t| t.result != TaskResult::Fail)
    );
}
#[test]
fn lesson_documents_have_schemas_and_fixed_acs_references() {
    let schema = schemars::schema_for!(Lesson).to_value();
    let validator = jsonschema::validator_for(&schema).unwrap();
    for lesson in lesson_library().unwrap() {
        assert!(validator.is_valid(&serde_json::to_value(lesson).unwrap()));
    }
    let commands = Session::commands();
    for name in [
        "lesson.list",
        "lesson.start",
        "lesson.debrief",
        "lesson.finish",
        "lesson.instructor_failure",
        "lesson.instructor_attitude",
        "atc.ifr",
    ] {
        let c = commands.iter().find(|c| c.name == name).unwrap();
        assert!(c.input_schema.is_object());
        assert!(c.output_schema.is_object());
    }
}
#[test]
fn ifr_vectors_reject_bad_angles_altitude_and_efc() {
    require_region_data!();
    let mut s = session();
    start(&mut s, "kabe-ils-06");
    for instruction in [
        json!({"kind":"vector_to_final","procedure":"I06","heading_deg":150,"altitude_ft":3000}),
        json!({"kind":"vector_to_final","procedure":"I06","heading_deg":63,"altitude_ft":5000}),
        json!({"kind":"hold","procedure":"I06","fix":"JISTO","efc_utc":2460}),
    ] {
        assert!(
            s.execute("atc.ifr", json!({"airport":"KABE","instruction":instruction}))
                .is_err()
        );
    }
    assert!(s.atc_state().pending_instructions.is_empty());
    s.execute("atc.ifr",json!({"airport":"KABE","instruction":{"kind":"hold","procedure":"I06","fix":"JISTO","efc_utc":1530}})).unwrap();
    let pending = s.atc_state().pending_instructions.remove(0);
    assert!(
        pending
            .phrase
            .contains("Expect further clearance at one five three zero")
    );
    assert!(pending.phrase.contains("right turns"));
    assert!(pending.phrase.contains("JISTO"));
}
#[test]
fn lnav_lesson_does_not_supply_a_glidepath() {
    require_region_data!();
    let mut s = session();
    start(&mut s, "kabe-lnav-06");
    s.run(Step { steps: 1 }).unwrap();
    assert_eq!(
        s.sim().procedure_receiver().mode,
        flightcraft_sim::avionics::procedures::ApproachMode::LNAV
    );
    assert!(s.sim().procedure_receiver().vertical_deviation_m.is_none());
    assert_eq!(s.sim().procedure_receiver().cdi_full_scale_nm, 0.3);
}
#[test]
fn crew_approach_matrix_calm_and_fifteen_knot_crosswind_within_acs() {
    require_region_data!();
    let mut s = session();
    for id in [
        "kabe-ils-06",
        "kabe-lpv-06",
        "kabe-lnav-06",
        "kmmu-lpv-05",
        "kmmu-lnav-05",
        "kabe-localizer-06",
    ] {
        if std::env::var("FLIGHTCRAFT_IFR_CASE").is_ok_and(|case| case != id) {
            continue;
        }
        for wind in [0., 15.] {
            for coupled in if id == "kabe-ils-06" {
                vec![false, true]
            } else {
                vec![false]
            } {
                start(&mut s, id);
                let setup = lesson_library()
                    .unwrap()
                    .into_iter()
                    .find(|l| l.id == id)
                    .unwrap()
                    .setup;
                let from = (s.sim().readout().heading_rad.to_degrees() + 90.).rem_euclid(360.);
                s.execute("weather.set",json!({"wind_from_deg":from,"wind_speed_mps":wind*0.514444,"shear":false,"turbulence":"off","seed":3407})).unwrap();
                s.run(SetCrewMode {
                    mode: CrewMode::Demonstrating,
                    director: false,
                })
                .unwrap();
                let output = s
                    .run(CrewIntent {
                        intent: PilotIntent::Approach {
                            airport: setup.airport,
                            procedure: setup.procedure,
                            mode: setup.mode,
                            minimum_ft: setup.minimum_ft,
                            coupled,
                        },
                    })
                    .unwrap();
                assert!(output.accepted, "{:?}", output.refusal);
                s.run(CrewHandover {
                    to: ControlOwner::Crew,
                    actor: ControlOwner::Pilot,
                    confirm: false,
                })
                .unwrap();
                s.run(CrewHandover {
                    to: ControlOwner::Crew,
                    actor: ControlOwner::Crew,
                    confirm: true,
                })
                .unwrap();
                let mut coupled_seen = false;
                for second in 0..900 {
                    s.run(Step { steps: 120 }).unwrap();
                    if std::env::var_os("FLIGHTCRAFT_IFR_TRACE").is_some() && second % 30 == 0 {
                        let r = s.sim().readout();
                        eprintln!(
                            "{id} {wind} {second}: {:?} {:.0} ft {:.1}kt GPS {:?} CDI {:.3} V {:?} {}",
                            s.crew_state().phase,
                            r.altitude_msl_m / 0.3048,
                            r.indicated_airspeed_mps / 0.514444,
                            s.sim().procedure_receiver().mode,
                            s.sim().procedure_receiver().cross_track_nm,
                            s.sim().procedure_receiver().vertical_deviation_m,
                            s.crew_state().reason
                        );
                    }

                    coupled_seen |= s.sim().installed_autopilot().is_some_and(|ap| {
                        matches!(
                            ap.active_vertical,
                            Some(
                                flightcraft_sim::equipment::VerticalMode::Gs
                                    | flightcraft_sim::equipment::VerticalMode::Gp
                            )
                        )
                    });
                    if s.lesson_debrief().tasks.iter().all(|t| t.completed)
                        && (!coupled || s.sim().readout().on_ground)
                        || s.sim().readout().crashed
                    {
                        break;
                    }
                }
                eprintln!(
                    "{id} wind {wind} coupled {coupled}: {:?}",
                    s.lesson_debrief().tasks
                );
                assert!(!s.sim().readout().crashed, "{id} {wind}");
                assert!(
                    s.lesson_debrief()
                        .tasks
                        .iter()
                        .all(|t| t.result == TaskResult::Pass),
                    "{id}, crosswind {wind}, coupled {coupled}: {:?}",
                    s.lesson_debrief().tasks
                );
                assert!(
                    !coupled || s.crew_state().touchdown.is_some(),
                    "coupled ILS did not land on the runway"
                );
                assert!(
                    !coupled || coupled_seen,
                    "coupled demonstration never captured the installed autopilot"
                );
                assert!(s.lesson_debrief().samples.iter().filter(|p| p.grading).count() > 60);
            }
        }
    }
}
#[test]
fn crew_published_hold_records_actual_entry_and_a_complete_lap() {
    require_region_data!();
    let mut s = session();
    start(&mut s, "kabe-hold");
    s.run(SetCrewMode {
        mode: CrewMode::Demonstrating,
        director: false,
    })
    .unwrap();
    assert!(
        s.run(CrewIntent {
            intent: PilotIntent::HoldAtFix {
                airport: "KABE".into(),
                procedure: "I06".into(),
                fix: "JISTO".into()
            }
        })
        .unwrap()
        .accepted
    );
    s.run(CrewHandover {
        to: ControlOwner::Crew,
        actor: ControlOwner::Pilot,
        confirm: false,
    })
    .unwrap();
    s.run(CrewHandover {
        to: ControlOwner::Crew,
        actor: ControlOwner::Crew,
        confirm: true,
    })
    .unwrap();
    for second in 0..800 {
        s.run(Step { steps: 120 }).unwrap();
        if second % 60 == 0 {
            eprintln!(
                "hold {second}: {:?} {:?}",
                s.lesson_debrief().hold,
                s.crew_state().reason
            );
        }
        assert!(!s.sim().readout().crashed);
        if s.lesson_debrief().hold.as_ref().is_some_and(|h| h.laps > 0) {
            break;
        }
    }
    assert!(
        s.lesson_debrief()
            .hold
            .as_ref()
            .is_some_and(|h| h.actual_entry.is_some() && h.laps > 0),
        "{:?}",
        s.lesson_debrief().hold
    );
    assert!(
        s.lesson_debrief()
            .tasks
            .iter()
            .all(|g| g.result == TaskResult::Pass),
        "{:?}",
        s.lesson_debrief().tasks
    );
}

#[test]
fn synthetic_bad_flight_failure_is_sticky_after_recovery() {
    for (bad, reason) in [(0, "Lateral"), (1, "Vertical"), (2, "Airspeed"), (3, "Altitude")] {
        let mut task = grade(InstrumentTaskKind::Precision);
        for second in 0..180 {
            let mut sample = InstrumentSample {
                time_s: f64::from(second),
                interval_s: 1.,
                altitude_ft: 2400.,
                signal_valid: true,
                grading: true,
                vertical_scale: Some(0.),
                ..Default::default()
            };
            if second == 60 {
                match bad {
                    0 => sample.cdi_scale = 0.9,
                    1 => sample.vertical_scale = Some(-0.9),
                    2 => sample.airspeed_error_kt = 12.,
                    _ => {
                        sample.level_segment = true;
                        sample.altitude_error_ft = 110.;
                    }
                }
            }
            task.observe(&sample, 594., true);
        }
        assert_eq!(task.result, TaskResult::Fail);
        assert!(task.reasons.iter().any(|r| r.contains(reason)));
        assert_eq!(task.observed_s, 180.);
    }
}
#[test]
fn crew_missed_at_da_reaches_the_published_missed_hold() {
    require_region_data!();
    let mut s = session();
    for (id, mode, minimum_ft) in [
        ("kabe-missed", InstrumentApproach::Ils, 594.),
        ("kabe-localizer-missed", InstrumentApproach::Localizer, 920.),
    ] {
        start(&mut s, id);
        s.run(SetCrewMode {
            mode: CrewMode::Demonstrating,
            director: false,
        })
        .unwrap();
        assert!(
            s.run(CrewIntent {
                intent: PilotIntent::Approach {
                    airport: "KABE".into(),
                    procedure: "I06".into(),
                    mode,
                    minimum_ft,
                    coupled: false
                }
            })
            .unwrap()
            .accepted
        );
        s.run(CrewHandover {
            to: ControlOwner::Crew,
            actor: ControlOwner::Pilot,
            confirm: false,
        })
        .unwrap();
        s.run(CrewHandover {
            to: ControlOwner::Crew,
            actor: ControlOwner::Crew,
            confirm: true,
        })
        .unwrap();
        for second in 0..1500 {
            s.run(Step { steps: 120 }).unwrap();
            assert!(!s.sim().readout().crashed);
            if second % 120 == 0 {
                eprintln!(
                    "missed {second}: {:?} {:.0}ft leg {} {:?}",
                    s.crew_state().phase,
                    s.sim().readout().altitude_msl_m / 0.3048,
                    s.sim().procedure_receiver().active_leg,
                    s.lesson_debrief().tasks
                );
            }
            if s.lesson_debrief().tasks.iter().all(|t| t.completed) {
                break;
            }
        }
        assert!(s.sim().procedure_receiver().missed_approach);
        assert!(
            s.lesson_debrief()
                .tasks
                .iter()
                .all(|t| t.result == TaskResult::Pass),
            "{:?}",
            s.lesson_debrief().tasks
        );
    }
}
#[test]
fn exported_flight_retains_measured_lesson_without_starting_it_on_import() {
    require_region_data!();
    let mut s = session();
    start(&mut s, "kabe-ils-06");
    s.run(Step { steps: 120 }).unwrap();
    s.run(LessonFinish {}).unwrap();
    let hash = s.sim().state_hash();
    let exported = s.execute("flight.export", json!({})).unwrap();
    let record = FlightRecord::from_json(exported["json"].as_str().unwrap()).unwrap();
    assert!(
        record
            .lesson
            .as_ref()
            .is_some_and(|l| l.complete && !l.samples.is_empty())
    );
    assert_eq!(hash, s.sim().state_hash());
}

#[test]
fn lesson_start_applies_declared_wind() {
    require_region_data!();
    let mut s = session();
    let mut lesson = lesson_library()
        .unwrap()
        .into_iter()
        .find(|l| l.id == "kabe-ils-06")
        .unwrap();
    lesson.setup.wind = flightcraft_world::Weather {
        wind_from_deg: 153.3,
        wind_speed_mps: 15. * 0.514444,
        ..Default::default()
    };
    s.run(LessonStart { lesson }).unwrap();
    assert!((s.sim().readout().weather.wind_from_deg - 153.3).abs() < 0.01);
    assert_eq!(s.sim().readout().weather.wind_speed_mps, 15. * 0.514444);
    assert_eq!(s.sim().state().time_s, 0.);
}
