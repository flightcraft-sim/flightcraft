#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_control::*;
use serde_json::{Value, json};
use std::path::Path;
fn call(s: &mut Session, name: &str, input: Value) -> Value {
    s.execute(name, input).unwrap()
}
fn activate(s: &mut Session, intent: Value) {
    call(s, "crew.mode", json!({"mode":"watching","director":true}));
    let result = call(s, "crew.intent", json!({"intent":intent}));
    assert_eq!(result["accepted"], true, "{result}");
    call(s, "crew.handover", json!({"to":"crew","actor":"crew"}));
    assert!(!s.crew_owns_controls());
    call(
        s,
        "crew.handover",
        json!({"to":"crew","actor":"pilot","confirm":true}),
    );
    assert!(s.crew_owns_controls());
}
fn finish(s: &mut Session, seconds: u32) {
    try_finish(s, seconds).unwrap();
}
fn try_finish(s: &mut Session, seconds: u32) -> Result<(), String> {
    let mut last = PilotPhase::Idle;
    for _ in 0..seconds {
        call(s, "sim.step", json!({"steps":120}));
        let state = s.crew_state();
        let r = s.sim().readout();
        if state.phase != last {
            println!(
                "t={:.1} {:?} pos={:?} h={:.1} v={:.1} heading={:.2} reason={}",
                r.time_s,
                state.phase,
                s.sim().state().position_m,
                r.height_agl_m,
                r.true_airspeed_mps,
                r.heading_grid_rad,
                state.reason
            );
            last = state.phase;
        }
        if r.crashed || state.phase == PilotPhase::Refused {
            return Err(format!(
                "{:?}: {} at {:?}; readout={:?}; controls={:?}",
                state.phase,
                state.reason,
                s.sim().state().position_m,
                r,
                s.sim().controls()
            ));
        }
        let towered = state
            .plan
            .as_ref()
            .and_then(|p| p.arrival.as_ref())
            .is_some_and(|a| {
                s.comms_state()
                    .stations
                    .iter()
                    .any(|st| st.airport == a.airport && st.kind == flightcraft_world::FacilityKind::Tower)
            });
        if state.phase == PilotPhase::Complete && r.ground_speed_mps < 0.2 && (!towered || state.parked) {
            if towered {
                audit_towered(s)?;
            }
            return Ok(());
        }
    }
    Err(format!(
        "Flight did not finish: {:?} {:?}",
        s.crew_state(),
        s.sim().readout()
    ))
}
#[test]
fn crew_flies_complete_pattern_using_pilot_controls() {
    for aircraft in ["corvid", "larrow"] {
        let mut s = Session::new().unwrap();
        call(
            &mut s,
            "aircraft.load",
            json!({"source":{"kind":"installed","id":aircraft}}),
        );
        call(&mut s, "sim.reset", json!({"start":{"kind":"runway"}}));
        activate(
            &mut s,
            json!({"kind":"fly_to_airport","airport":"default","runway":"36","altitude_m":315.}),
        );
        finish(&mut s, 1800);
        let touchdown = s.crew_state().touchdown.unwrap();
        assert!(touchdown.within_limits, "{aircraft}: {touchdown:?}");
        assert!(touchdown.sink_mps < 2.);
        assert!(s.sim().readout().on_ground);
        let recording = call(&mut s, "flight.events", json!({"limit":1000}));
        let inputs = recording["inputs"].as_array().unwrap();
        assert!(
            inputs.windows(2).any(|pair| {
                let dt = pair[1]["time_s"].as_f64().unwrap() - pair[0]["time_s"].as_f64().unwrap();
                dt > 0. && dt < 0.2
            }),
            "crew guidance must be recorded within a batched sim.step"
        );
        assert_eq!(recording["dropped_inputs"], 0);
    }
}
#[test]
fn handover_requires_two_participants_and_reset_removes_crew() {
    let mut s = Session::new().unwrap();
    call(&mut s, "crew.mode", json!({"mode":"watching"}));
    assert!(
        s.execute(
            "crew.handover",
            json!({"to":"crew","actor":"pilot","confirm":true})
        )
        .is_err()
    );
    call(&mut s, "crew.handover", json!({"to":"crew","actor":"pilot"}));
    assert!(
        s.execute(
            "crew.handover",
            json!({"to":"crew","actor":"pilot","confirm":true})
        )
        .is_err()
    );
    assert!(!s.crew_owns_controls());
    activate(
        &mut s,
        json!({"kind":"take_off","airport":"default","runway":"36"}),
    );
    assert!(s.execute("sim.set_controls", json!({"throttle":0})).is_err());
    assert!(s.execute("assist.set", json!({"heading_rad":0})).is_err());
    assert!(s.execute("crew.mode", json!({"mode":"solo"})).is_err());
    call(&mut s, "crew.handover", json!({"to":"pilot","actor":"crew"}));
    assert!(s.crew_owns_controls());
    call(
        &mut s,
        "crew.handover",
        json!({"to":"pilot","actor":"pilot","confirm":true}),
    );
    assert!(!s.crew_owns_controls());
    call(&mut s, "sim.reset", json!({"start":{"kind":"runway"}}));
    assert_eq!(s.crew_state().mode, CrewMode::Solo);
    assert!(s.crew_state().plan.is_none());
}
#[test]
fn schemas_refusals_and_observation_do_not_move_aircraft() {
    let mut s = Session::new().unwrap();
    let before = *s.sim().state();
    for (name, input) in [
        ("crew.mode", json!({"mode":"watching"})),
        (
            "crew.intent",
            json!({"intent":{"kind":"hold","altitude_m":300,"heading_rad":0}}),
        ),
        ("crew.state", json!({})),
    ] {
        let info = Session::commands().into_iter().find(|c| c.name == name).unwrap();
        assert!(
            jsonschema::validator_for(&info.input_schema)
                .unwrap()
                .is_valid(&input)
        );
        let out = call(&mut s, name, input);
        assert!(
            jsonschema::validator_for(&info.output_schema)
                .unwrap()
                .is_valid(&out),
            "{out}"
        );
    }
    assert_eq!(*s.sim().state(), before);
    for intent in [
        json!({"kind":"fly_to_coordinate","position":{"latitude_deg":91,"longitude_deg":0},"altitude_m":1000}),
        json!({"kind":"orbit","position":{"latitude_deg":41,"longitude_deg":-75},"altitude_m":1000,"radius_m":0}),
        json!({"kind":"scenic","preset":"hudson_corridor","airport":"1N7","runway":"07"}),
    ] {
        let result = call(&mut s, "crew.intent", json!({"intent":intent}));
        assert_eq!(result["accepted"], false);
    }
    assert_eq!(*s.sim().state(), before);
}
#[test]
fn crew_off_preserves_physics_and_batching_is_deterministic() {
    let mut a = Session::new().unwrap();
    let mut b = Session::new().unwrap();
    assert_eq!(a.crew_state().mode, CrewMode::Solo);
    call(&mut a, "sim.set_controls", json!({"throttle":1}));
    call(&mut b, "sim.set_controls", json!({"throttle":1}));
    a.run(Step { steps: 1200 }).unwrap();
    b.sim_mut().step(1200);
    assert_eq!(a.sim().state(), b.sim().state());
    for s in [&mut a, &mut b] {
        call(
            s,
            "sim.reset",
            json!({"start":{"kind":"air","height_m":500,"airspeed_mps":40,"heading_rad":0}}),
        );
        activate(s, json!({"kind":"hold","altitude_m":500,"heading_rad":1}));
    }
    a.run(Step { steps: 1200 }).unwrap();
    for _ in 0..100 {
        b.run(Step { steps: 12 }).unwrap();
    }
    assert_eq!(a.sim().state(), b.sim().state());
}
fn region_session(root: &Path) -> Session {
    let mut s = Session::new().unwrap();
    if root.join(".cache/packs").is_dir() {
        assert!(s.load_packs_directory(&root.join(".cache/packs")).is_empty());
    }
    s.install_region(read_region(&root.join("assets/terrain/eastern-pa")).unwrap())
        .unwrap();
    s
}
#[test]
#[ignore = "requires installed released terrain; flies every runway in both aircraft and winds"]
fn regional_runway_acceptance_matrix() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut s = region_session(&root);
    let catalogue = s
        .execute(
            "sim.reset",
            json!({"world":"eastern-pa","start":{"kind":"runway"}}),
        )
        .unwrap();
    assert!(!catalogue.is_null());
    let airports = s.sim().world().airports().unwrap().airports.clone();
    let mut reports = Vec::new();
    let filter = std::env::var("PILOT_CREW_MATRIX_FILTER").unwrap_or_default();
    for aircraft in ["corvid", "larrow", "sling2"] {
        call(
            &mut s,
            "aircraft.load",
            json!({"source":{"kind":"installed","id":aircraft}}),
        );
        for ident in ["1N7", "MPO", "ABE", "AVP", "MMU"] {
            let airport = airports.iter().find(|a| a.ident == ident).unwrap();
            for runway in &airport.runways {
                for end in &runway.ends {
                    for wind in [0., 4.] {
                        let case = format!("{aircraft} {ident} {} {wind}", end.ident);
                        if !filter.is_empty() && !case.contains(&filter) {
                            continue;
                        }
                        call(
                            &mut s,
                            "sim.reset",
                            json!({"world":"eastern-pa","start":{"runway":{"airport":ident,"end":end.ident}}}),
                        );
                        let reference = s.sim().heading_reference();
                        let from = reference
                            .grid_to_magnetic((end.heading_deg + 70.).to_radians())
                            .to_degrees()
                            .rem_euclid(360.);
                        call(
                            &mut s,
                            "weather.set",
                            json!({"wind_from_deg":from,"wind_speed_mps":wind,"shear":false,"turbulence":if wind==0.{"off"}else{"light"},"seed":41}),
                        );
                        println!("Acceptance {aircraft} {ident} {} wind {wind}", end.ident);
                        call(&mut s, "crew.mode", json!({"mode":"watching"}));
                        let planned = call(
                            &mut s,
                            "crew.intent",
                            json!({"intent":{"kind":"fly_to_airport","airport":ident,"runway":end.ident,"altitude_m":airport.elevation_m+315.}}),
                        );
                        let result = if planned["accepted"] == true {
                            call(&mut s, "crew.handover", json!({"to":"crew","actor":"crew"}));
                            call(
                                &mut s,
                                "crew.handover",
                                json!({"to":"crew","actor":"pilot","confirm":true}),
                            );
                            try_finish(&mut s, 2400)
                        } else {
                            Err(planned["refusal"].to_string())
                        };
                        let state = s.crew_state();
                        let passed =
                            result.is_ok() && state.touchdown.as_ref().is_some_and(|t| t.within_limits);
                        println!(
                            "Result {case}: {} {:?}",
                            if passed { "PASS" } else { "FAIL" },
                            result
                        );
                        if !passed {
                            let debug = root.join("out/crew");
                            std::fs::create_dir_all(&debug).unwrap();
                            std::fs::write(
                                debug.join(format!("failure-{}.json", case.replace(' ', "-"))),
                                serde_json::to_vec_pretty(
                                    &json!({"crew":state,"atc":s.atc_state(),"record":s.flight_record()}),
                                )
                                .unwrap(),
                            )
                            .unwrap();
                        }
                        reports.push(json!({"case":case,"passed":passed,"error":result.err(),"touchdown":state.touchdown,"taxi_clear":state.taxi_clear,"parked":state.parked,"stop_reason":state.reason,"violations":s.atc_state().compliance.iter().filter(|g|!g.passed).count(),"unanswered_instructions":s.atc_state().pending_instructions.len(),"arrival":state.plan.as_ref().and_then(|p|p.arrival.as_ref())}));
                        let out = root.join("out/crew");
                        std::fs::create_dir_all(&out).unwrap();
                        std::fs::write(
                            out.join(if filter.is_empty() {
                                "acceptance-matrix.json".into()
                            } else {
                                format!(
                                    "acceptance-{}.json",
                                    filter
                                        .chars()
                                        .filter(char::is_ascii_alphanumeric)
                                        .collect::<String>()
                                )
                            }),
                            serde_json::to_string_pretty(&reports).unwrap(),
                        )
                        .unwrap();
                    }
                }
            }
        }
    }
    let failures: Vec<_> = reports.iter().filter(|r| r["passed"] != true).collect();
    assert!(!reports.is_empty());
    assert!(
        failures.is_empty(),
        "{} of {} flights failed; see out/crew/acceptance-matrix.json",
        failures.len(),
        reports.len()
    );
}
#[test]
#[ignore = "requires released region data; produces flight transcripts and caption previews"]
fn write_crew_flight_previews() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join("out/crew");
    std::fs::create_dir_all(&out).unwrap();
    for (name, intent) in [
        (
            "water-gap",
            json!({"kind":"scenic","preset":"delaware_water_gap","airport":"1N7","runway":"07"}),
        ),
        (
            "1n7-kabe",
            json!({"kind":"fly_to_airport","airport":"KABE","runway":"24","altitude_m":1250.}),
        ),
    ] {
        let mut s = region_session(&root);
        call(
            &mut s,
            "aircraft.load",
            json!({"source":{"kind":"installed","id":"sling2"}}),
        );
        s.enable_app();
        call(
            &mut s,
            "sim.reset",
            json!({"world":"eastern-pa","start":{"runway":{"airport":"1N7","end":"07"}}}),
        );
        activate(&mut s, intent);
        let mut frames = Vec::new();
        let mut previous = PilotPhase::Idle;
        let mut cruise_start = None;
        let mut captured_flyby = false;
        let mut captured_final = false;
        for _ in 0..3600 {
            s.run(Step { steps: 120 }).unwrap();
            let state = s.crew_state();
            let r = s.sim().readout();
            assert!(!r.crashed, "{state:?}");
            if state.phase != previous {
                println!(
                    "{name} {:.0}s {:?}: {:?}",
                    r.time_s,
                    state.phase,
                    s.sim().state().position_m
                );
                frames.push(json!({"crew":state,"state":s.sim().state(),"camera":s.camera()}));
                previous = state.phase;
            }
            if state.phase == PilotPhase::Cruise
                && r.turn_rate_rps.abs() < 0.003
                && r.vertical_speed_mps.abs() < 0.5
            {
                if cruise_start.is_none() {
                    frames.push(json!({"shot":"flyby_start","crew":state,"state":s.sim().state(),"camera":s.camera()}));
                }
                let start = *cruise_start.get_or_insert(r.time_s);
                if !captured_flyby && r.time_s - start >= 8.5 {
                    frames.push(
                        json!({"shot":"flyby_pass","crew":state,"state":s.sim().state(),"camera":s.camera()}),
                    );
                    captured_flyby = true;
                }
            } else if !captured_flyby {
                cruise_start = None;
            }
            if !captured_final
                && state.phase == PilotPhase::Final
                && let Some(arrival) = state.plan.as_ref().and_then(|p| p.arrival.as_ref())
            {
                let [n, e, _] = s.sim().state().position_m;
                let [rn, re, _] = arrival.threshold_m;
                let along =
                    (n - rn) * arrival.heading_grid_rad.cos() + (e - re) * arrival.heading_grid_rad.sin();
                if along > -140. {
                    frames.push(json!({"shot":"short_final","crew":state,"state":s.sim().state(),"camera":s.camera()}));
                    captured_final = true;
                }
            }
            let towered = state
                .plan
                .as_ref()
                .and_then(|p| p.arrival.as_ref())
                .is_some_and(|a| {
                    s.comms_state().stations.iter().any(|st| {
                        st.airport == a.airport && st.kind == flightcraft_world::FacilityKind::Tower
                    })
                });
            if state.phase == PilotPhase::Complete && r.ground_speed_mps < 0.2 && (!towered || state.parked) {
                if towered {
                    audit_towered(&s).unwrap();
                }
                break;
            }
        }
        let state = s.crew_state();
        assert_eq!(state.phase, PilotPhase::Complete, "{state:?}");
        assert!(state.touchdown.as_ref().unwrap().within_limits, "{state:?}");
        s.run(Step { steps: 1200 }).unwrap();
        let record = s.flight_record();
        std::fs::write(
            out.join(format!("{name}-events.json")),
            serde_json::to_string_pretty(record).unwrap(),
        )
        .unwrap();
        let stations = s.comms_state().stations;
        let transcript = record
            .events
            .iter()
            .filter_map(|e| e.transcript_line(&stations))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(out.join(format!("{name}-transcript.txt")), transcript).unwrap();
        let mut captions = "WEBVTT\n\n".to_owned();
        for e in &record.events {
            if e.reception == Reception::Heard && e.is_spoken() {
                captions.push_str(&format!(
                    "{}\n{} --> {}\n{}\n\n",
                    e.id,
                    vtt_time(e.time_s),
                    vtt_time(e.time_s + e.duration_s),
                    e.text
                ));
            }
        }
        std::fs::write(out.join(format!("{name}-captions.vtt")), captions).unwrap();
        std::fs::write(
            out.join(format!("{name}-shots.json")),
            serde_json::to_string_pretty(&frames).unwrap(),
        )
        .unwrap();
        std::fs::write(
            out.join(format!("{name}-report.json")),
            serde_json::to_string_pretty(&state).unwrap(),
        )
        .unwrap();
    }
}
fn vtt_time(time: f64) -> String {
    let ms = (time * 1000.).round() as u64;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        ms / 3600000,
        ms / 60000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

#[test]
fn class_b_route_is_refused_without_replacing_the_active_plan() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if !root.join("assets/terrain/new-york/000.fcht").is_file() {
        return;
    }
    let mut s = Session::new().unwrap();
    if root.join(".cache/packs").is_dir() {
        assert!(s.load_packs_directory(&root.join(".cache/packs")).is_empty());
    }
    s.install_region(read_region(&root.join("assets/terrain/new-york")).unwrap())
        .unwrap();
    call(
        &mut s,
        "sim.reset",
        json!({"world":"new-york","start":{"kind":"air","height_m":1000,"airspeed_mps":40,"heading_rad":0}}),
    );
    activate(&mut s, json!({"kind":"hold","altitude_m":1100,"heading_rad":0}));
    let old = serde_json::to_value(s.crew_state()).unwrap();
    let result = call(
        &mut s,
        "crew.intent",
        json!({"intent":{"kind":"fly_to_coordinate","position":{"latitude_deg":40.6895,"longitude_deg":-74.1745},"altitude_m":700}}),
    );
    assert_eq!(result["accepted"], false, "{result}");
    assert!(
        result["refusal"].as_str().unwrap().contains("Class B"),
        "{result}"
    );
    assert_eq!(serde_json::to_value(s.crew_state()).unwrap(), old);
}

#[test]
fn crew_radio_renderer_uses_nasr_names_and_spoken_runway_digits() {
    let nav = flightcraft_world::Navdata::from_json(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/terrain/eastern-pa/navdata.json"
    )))
    .unwrap();
    let station = nav
        .comms
        .iter()
        .find(|s| s.airport == "1N7" && s.kind == flightcraft_world::FacilityKind::Ctaf)
        .unwrap();
    let mut s = Session::new().unwrap();
    let pilot = s
        .run(CommsCallsign {
            registration: Some("N81F".into()),
            spoken_type: Some("Corvid".into()),
            ..Default::default()
        })
        .unwrap();
    let text =
        flightcraft_control::comms::phraseology::pilot_position(station, &pilot, "07", "left downwind");
    assert_eq!(
        text,
        "Blairstown Traffic, Corvid Eight One Foxtrot, left downwind runway zero seven, Blairstown."
    );
    let mut pilot = pilot;
    pilot.current = pilot.abbreviated.clone();
    assert_eq!(
        flightcraft_control::comms::phraseology::pilot_inbound(station, &pilot, "07", 10, "north", None),
        "Blairstown Traffic, Corvid Eight One Foxtrot, one zero miles north, inbound for landing, runway zero seven, Blairstown."
    );
    assert!(
        flightcraft_control::comms::phraseology::pilot_position(station, &pilot, "07", "final")
            .contains(&pilot.full)
    );
}

#[test]
fn holds_capture_heading_and_altitude_and_go_around_uses_real_controls() {
    let mut s = Session::new().unwrap();
    call(
        &mut s,
        "sim.reset",
        json!({"start":{"kind":"air","height_m":500,"airspeed_mps":40,"heading_rad":0}}),
    );
    activate(
        &mut s,
        json!({"kind":"hold","altitude_m":700,"heading_rad":std::f64::consts::FRAC_PI_2}),
    );
    s.run(Step { steps: 14400 }).unwrap();
    let r = s.sim().readout();
    assert!(!r.crashed);
    assert!((r.altitude_msl_m - 700.).abs() < 10., "{r:?}");
    assert!(
        flightcraft_sim::autopilot::wrap(r.heading_rad - std::f64::consts::FRAC_PI_2).abs() < 0.05,
        "{r:?}"
    );
    call(
        &mut s,
        "sim.reset",
        json!({"start":{"kind":"air","height_m":100,"airspeed_mps":40,"heading_rad":0}}),
    );
    activate(
        &mut s,
        json!({"kind":"land_runway","airport":"default","runway":"36"}),
    );
    let before = *s.sim().state();
    let intent = s
        .run(CrewIntent {
            intent: PilotIntent::GoAround {},
        })
        .unwrap();
    assert!(intent.accepted);
    assert_eq!(*s.sim().state(), before);
    s.run(Step { steps: 600 }).unwrap();
    let r = s.sim().readout();
    assert!(!r.crashed);
    assert!(r.altitude_msl_m > 105. && r.vertical_speed_mps > 1., "{r:?}");
    assert_eq!(r.controls.throttle, 1.);
}

#[test]
fn tailwheel_named_sloping_runway_start_keeps_aft_gear_on_pavement() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut s = region_session(&root);
    call(
        &mut s,
        "aircraft.load",
        json!({"source":{"kind":"installed","id":"larrow"}}),
    );
    call(
        &mut s,
        "sim.reset",
        json!({"world":"eastern-pa","start":{"runway":{"airport":"MPO","end":"23"}}}),
    );
    call(&mut s, "sim.step", json!({"steps":120}));
    assert!(!s.sim().readout().crashed);
    assert!(s.sim().readout().on_ground);
    assert!(!s.crew_owns_controls());
}

fn audit_towered(s: &Session) -> Result<(), String> {
    let atc = s.atc_state();
    let violations: Vec<_> = atc.compliance.iter().filter(|g| !g.passed).collect();
    if !violations.is_empty() {
        return Err(format!("ATC violations: {violations:?}"));
    }
    if !atc.pending_instructions.is_empty() {
        return Err(format!(
            "Unanswered ATC instructions: {:?}",
            atc.pending_instructions
        ));
    }
    if s.flight_record()
        .events
        .iter()
        .any(|e| e.text.to_lowercase().contains("say again"))
    {
        return Err("ATC had to request a repeat or corrected readback".into());
    }
    for event in &s.flight_record().events {
        if event.reception == Reception::Heard
            && matches!(event.meaning, Meaning::Atc { .. })
            && !atc
                .compliance
                .iter()
                .any(|g| g.item == "readback" && g.instruction_id == Some(event.id) && g.passed)
        {
            return Err(format!(
                "Instruction {} has no accepted readback: {}",
                event.id, event.text
            ));
        }
    }
    if !atc
        .compliance
        .iter()
        .any(|g| g.item == "landing_clearance" && g.passed)
    {
        return Err("No observed landing with clearance".into());
    }
    let events = &s.flight_record().events;
    let clearances: Vec<_> = events
        .iter()
        .filter(|e| {
            e.reception == Reception::Heard
                && matches!(
                    e.meaning,
                    Meaning::Atc {
                        instruction: AtcInstruction::Land { .. }
                    }
                )
        })
        .collect();
    if clearances.len() != 1 {
        return Err(format!(
            "Expected one landing clearance, got {}",
            clearances.len()
        ));
    }
    if events.iter().any(|e| {
        e.speaker == Speaker::Pilot
            && e.time_s > clearances.first().unwrap().time_s
            && (e.text.contains(", final runway")
                || e.text.contains(", left base runway")
                || e.text.contains(", right base runway"))
    }) {
        return Err("Unrequested base/final report after landing clearance".into());
    }
    let exit = events
        .iter()
        .find(|e| {
            e.reception == Reception::Heard
                && matches!(
                    e.meaning,
                    Meaning::Atc {
                        instruction: AtcInstruction::ExitRunway { .. }
                    }
                )
        })
        .ok_or("No tower exit and ground instruction")?;
    let clear = events
        .iter()
        .find(|e| e.speaker == Speaker::Pilot && e.text.to_lowercase().contains("clear of runway"))
        .ok_or("No pilot report clear of runway")?;
    if exit.time_s >= clear.time_s {
        return Err("Tower gave exit/ground after the pilot reported clear".into());
    }
    if !events
        .iter()
        .any(|e| e.speaker == Speaker::Pilot && e.text.starts_with("Contact ground point niner,"))
    {
        return Err("Missing compact ground-frequency readback".into());
    }
    let mut last_landmark = None;
    for event in events.iter().filter(|e| e.text.starts_with("That's ")) {
        if last_landmark.is_some_and(|last| event.time_s - last < 120.) || event.text.contains("Pond") {
            return Err("Landmark narration is too frequent or includes a minor pond".into());
        }
        last_landmark = Some(event.time_s);
    }
    Ok(())
}
#[test]
fn crew_towered_flight_obeys_every_instruction_and_clearance() {
    crew_towered_case("corvid");
}
#[test]
#[ignore = "requires the bundled Sling pack; the standard crew regression also flies this route"]
fn crew_towered_sling_flight_obeys_every_instruction_and_clearance() {
    crew_towered_case("sling2");
}
fn crew_towered_case(aircraft: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut s = region_session(&root);
    call(
        &mut s,
        "aircraft.load",
        json!({"source":{"kind":"installed","id":aircraft}}),
    );
    call(
        &mut s,
        "sim.reset",
        json!({"world":"eastern-pa","start":{"runway":{"airport":"1N7","end":"07"}}}),
    );
    activate(
        &mut s,
        json!({"kind":"fly_to_airport","airport":"KABE","runway":"24","altitude_m":1250.}),
    );
    let result = try_finish(&mut s, 3600);
    std::fs::create_dir_all(root.join("out/crew")).unwrap();
    std::fs::write(
        root.join("out/crew/kabe-atc-debug.json"),
        serde_json::to_string_pretty(
            &json!({"crew":s.crew_state(),"atc":s.atc_state(),"record":s.flight_record()}),
        )
        .unwrap(),
    )
    .unwrap();
    result.unwrap();
    assert_eq!(s.crew_state().plan.unwrap().arrival.unwrap().runway, "06");
    assert!(s.crew_state().parked);
    audit_towered(&s).unwrap();
}

#[test]
#[ignore = "requires completed regional matrix logs; writes measured taxi terrain diagnostics"]
fn write_taxi_hold_diagnostics() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join("out/crew");
    let world = read_region(&root.join("assets/terrain/eastern-pa")).unwrap();
    let mut result = Vec::new();
    for aircraft in ["corvid", "larrow", "sling2"] {
        let rows: Vec<Value> =
            serde_json::from_slice(&std::fs::read(out.join(format!("acceptance-{aircraft}.json"))).unwrap())
                .unwrap();
        let log = std::fs::read_to_string(out.join(format!("check-matrix-{aircraft}.log"))).unwrap();
        for row in rows.iter().filter(|r| r["taxi_clear"] == false) {
            let case = row["case"].as_str().unwrap();
            let (head, wind) = case.rsplit_once(' ').unwrap();
            let section = log
                .split(&format!("Acceptance {head} wind {wind}"))
                .nth(1)
                .unwrap();
            let stop = section.lines().find(|l| l.contains("Complete pos=")).unwrap();
            let pose = stop
                .split("pos=")
                .nth(1)
                .unwrap()
                .split("] ")
                .next()
                .unwrap()
                .to_owned()
                + "]";
            let position: [f64; 3] = serde_json::from_str(&pose).unwrap();
            let r = &row["arrival"];
            let [rn, re, _]: [f64; 3] = serde_json::from_value(r["threshold_m"].clone()).unwrap();
            let heading = r["heading_grid_rad"].as_f64().unwrap();
            let (sin, cos) = heading.sin_cos();
            let along = (position[0] - rn) * cos + (position[1] - re) * sin;
            let width = r["width_m"].as_f64().unwrap();
            let runway_length = r["length_m"].as_f64().unwrap();
            let remaining = (runway_length - along - 45.).clamp(0., 1800.);
            let mut probes = Vec::new();
            for advance in (0..=(remaining / 2.).floor() as u32).map(|i| f64::from(i) * 2.) {
                if along + advance + 35. > runway_length - 10. {
                    continue;
                }
                for side in [-1., 1.] {
                    let cross = side * (width * 0.5 + 9.);
                    let diagonal = 25_f64.hypot(cross);
                    let mut previous: Option<f64> = None;
                    let mut worst_step = 0_f64;
                    let mut worst_normal = -1_f64;
                    let mut water = false;
                    for step in 0..=((advance + diagonal) * 2.).ceil() as u32 {
                        let travelled = f64::from(step) * 0.5;
                        let f = ((travelled - advance) / diagonal).clamp(0., 1.);
                        let a = along + travelled.min(advance) + 25. * f;
                        let c = cross * f;
                        let g = world.ground(rn + a * cos - c * sin, re + a * sin + c * cos);
                        if let Some(h) = previous {
                            worst_step = worst_step.max((g.height_msl_m - h).abs());
                        }
                        previous = Some(g.height_msl_m);
                        worst_normal = worst_normal.max(g.normal_ned[2]);
                        water |= g.surface == flightcraft_world::Surface::Water;
                    }
                    probes.push(json!({"advance_m":advance,"side":side,"max_step_m_per_half_m":worst_step,"worst_normal_down":worst_normal,"water":water}));
                }
            }
            probes.sort_by(|a, b| {
                a["max_step_m_per_half_m"]
                    .as_f64()
                    .unwrap()
                    .total_cmp(&b["max_step_m_per_half_m"].as_f64().unwrap())
            });
            result.push(json!({"case":case,"stop_position_m":position,"along_m":along,"runway_length_m":runway_length,"candidate_count":probes.len(),"best_candidates":probes.into_iter().take(12).collect::<Vec<_>>() }));
        }
    }
    std::fs::write(
        out.join("taxi-hold-diagnostics.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
}

#[test]
fn before_takeoff_holds_for_aligning_and_failed_attitude_sources() {
    for (id, setting, repair) in [
        (
            "corvid-glass",
            json!({"kind":"glass","setting":{"kind":"failure","target":"ahrs","failed":true}}),
            json!({"kind":"glass","setting":{"kind":"failure","target":"ahrs","failed":false}}),
        ),
        (
            "corvid-standby",
            json!({"kind":"ahrs_failure","unit":"electronic_attitude","failed":true}),
            json!({"kind":"ahrs_failure","unit":"electronic_attitude","failed":false}),
        ),
        (
            "corvid-standby",
            json!({"kind":"ahrs_failure","unit":"electronic_hsi","failed":true}),
            json!({"kind":"ahrs_failure","unit":"electronic_hsi","failed":false}),
        ),
    ] {
        let mut s = Session::new().unwrap();
        call(
            &mut s,
            "aircraft.load",
            json!({"source":{"kind":"installed","id":id},"reset":true}),
        );
        call(&mut s, "sim.reset", json!({"start":{"kind":"runway"}}));
        call(&mut s, "avionics.set", json!({"setting":setting}));
        activate(
            &mut s,
            json!({"kind":"take_off","airport":"default","runway":"36"}),
        );
        call(&mut s, "sim.step", json!({"steps":6000}));
        assert_eq!(s.crew_state().phase, PilotPhase::BeforeTakeoff, "{id}");
        assert!(s.sim().readout().on_ground);
        assert!(s.sim().controls().brake >= 0.5);
        assert_eq!(s.sim().controls().throttle, 0.);
        let message = "Waiting for the attitude display to align.";
        assert_eq!(s.crew_state().reason, message);
        assert_eq!(
            s.flight_record()
                .events
                .iter()
                .filter(|e| e.text == message)
                .count(),
            1
        );
        call(&mut s, "avionics.set", json!({"setting":repair}));
        call(&mut s, "sim.step", json!({"steps":4800}));
        assert_eq!(
            s.crew_state().phase,
            PilotPhase::BeforeTakeoff,
            "{id}: took off before alignment"
        );
        assert!(s.sim().controls().brake >= 0.5);
        call(&mut s, "sim.step", json!({"steps":720}));
        assert_eq!(
            s.crew_state().phase,
            PilotPhase::Takeoff,
            "{id}: did not resume after alignment"
        );
    }
}

#[test]
fn before_takeoff_waits_for_independent_glass_standby_alignment() {
    let mut s = Session::new().unwrap();
    call(
        &mut s,
        "aircraft.load",
        json!({"source":{"kind":"installed","id":"corvid-glass"},"reset":true}),
    );
    call(&mut s, "sim.reset", json!({"start":{"kind":"runway"}}));
    // Model a freshly replaced standby computer while the primary remains ready.
    s.sim_mut().glass_computers_mut().standby = Default::default();
    s.sim_mut().refresh_glass_computers();
    assert!(s.glass_state().air_data.attitude_valid);
    assert!(!s.glass_state().standby.attitude_valid);
    activate(
        &mut s,
        json!({"kind":"take_off","airport":"default","runway":"36"}),
    );
    call(&mut s, "sim.step", json!({"steps":4800}));
    assert_eq!(s.crew_state().phase, PilotPhase::BeforeTakeoff);
    assert_eq!(
        s.crew_state().reason,
        "Waiting for the attitude display to align."
    );
    assert!(s.sim().readout().on_ground && s.sim().controls().brake >= 0.5);
    call(&mut s, "sim.step", json!({"steps":720}));
    assert_eq!(s.crew_state().phase, PilotPhase::Takeoff);
}

#[test]
fn before_takeoff_aborts_ground_roll_for_a_failed_attitude_display() {
    let mut s = Session::new().unwrap();
    call(
        &mut s,
        "aircraft.load",
        json!({"source":{"kind":"installed","id":"corvid-glass"},"reset":true}),
    );
    call(&mut s, "sim.reset", json!({"start":{"kind":"runway"}}));
    activate(
        &mut s,
        json!({"kind":"take_off","airport":"default","runway":"36"}),
    );
    call(&mut s, "sim.step", json!({"steps":1080}));
    assert_eq!(s.crew_state().phase, PilotPhase::Takeoff);
    assert!(s.sim().readout().on_ground);
    call(
        &mut s,
        "avionics.set",
        json!({"setting":{"kind":"glass","setting":{"kind":"failure","target":"pfd","failed":true}}}),
    );
    call(&mut s, "sim.step", json!({"steps":60}));
    assert_eq!(s.crew_state().phase, PilotPhase::BeforeTakeoff);
    assert_eq!(s.sim().controls().throttle, 0.);
    assert!(s.sim().controls().brake >= 0.5);
    assert!(
        s.flight_record()
            .events
            .iter()
            .any(|e| e.text == "Waiting for the attitude display to align.")
    );
}
