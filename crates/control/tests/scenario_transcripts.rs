//! Reproducible library transcripts on the released terrain and FAA catalogues.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use flightcraft_control::*;
use serde_json::json;
fn preview_session() -> Session {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut s = Session::new().unwrap();
    s.install_region(read_region(&root.join("assets/terrain/eastern-pa")).unwrap())
        .unwrap();
    s.execute("sim.reset",json!({"world":"eastern-pa","start":{"kind":"air","height_m":1200.,"airspeed_mps":33.,"heading_rad":0.}})).unwrap();
    s
}
#[test]
#[ignore = "writes out/scenario library transcripts"]
fn write_scenario_transcripts() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/scenario");
    std::fs::create_dir_all(&dir)?;
    let mut s = preview_session();
    s.execute("sim.reset",json!({"world":"eastern-pa","start":{"kind":"airport_air","airport":"KMPO","distance_m":12000.,"bearing_deg":180.,"height_m":1200.,"airspeed_mps":33.,"heading_rad":0.}}))?;
    s.execute("comms.tune", json!({"radio":1,"channel_khz":122700}))?;
    let doc = library()?
        .into_iter()
        .find(|d| d.id == "engine-cruise")
        .ok_or("missing library scenario")?;
    s.run(ScenarioStart { scenario: doc })?;
    s.run(Step { steps: 120 })?;
    s.run(EmergencyRecord {
        observation: EmergencyObservation::Recognized,
    })?;
    let callsign = s.pilot_callsign().full;
    s.execute("comms.transmit",json!({"text":format!("Mayday Mayday Mayday, {callsign}, engine failure, six miles south of Pocono Mountains, selecting a landing field.")}))?;
    s.run(Step { steps: 1800 })?;
    assert!(
        s.flight_record()
            .events
            .iter()
            .any(|e| e.speaker == Speaker::Pilot && e.reception == Reception::Heard)
    );
    save(&s, &dir, "engine-cruise")?;
    let mut s = preview_session();
    let airport = s
        .sim()
        .world()
        .airports()
        .ok_or("missing airports")?
        .airports
        .iter()
        .find(|a| a.icao.as_deref() == Some("KABE"))
        .ok_or("KABE missing")?;
    let end = airport
        .runways
        .iter()
        .flat_map(|r| &r.ends)
        .find(|e| e.ident == "24")
        .ok_or("runway missing")?;
    let point = flightcraft_world::GeoPoint {
        latitude_deg: end.latitude_deg,
        longitude_deg: end.longitude_deg,
    }
    .destination((end.heading_deg + 180.).rem_euclid(360.), 2.9 * 1852.);
    let heading = end.heading_deg.to_radians();
    s.execute("sim.reset",json!({"world":"eastern-pa","start":{"kind":"geographic_air","position":point,"height_agl_m":300.,"airspeed_mps":33.,"heading_rad":heading}}))?;
    s.execute(
        "comms.callsign",
        json!({"registration":"N34B","spoken_type":"Sling","abbreviate_after_contact":false}),
    )?;
    // Convert true runway alignment to the pilot's magnetic heading through a feedback hold.
    let read = s.sim().readout();
    let magnetic = (heading - read.magnetic_declination_rad).rem_euclid(core::f64::consts::TAU);
    s.execute(
        "assist.set",
        json!({"heading_rad":magnetic,"altitude_m":s.sim().readout().altitude_msl_m,"airspeed_mps":33.}),
    )?;
    s.execute("comms.tune", json!({"radio":1,"channel_khz":120500}))?;
    let doc = library()?
        .into_iter()
        .find(|d| d.id == "three-mile-final")
        .ok_or("missing scenario")?;
    s.run(ScenarioStart { scenario: doc })?;
    s.run(Step { steps: 1200 })?;
    s.run(Step { steps: 600 })?;
    assert!(
        s.flight_record()
            .events
            .iter()
            .any(|e| e.is_simulated_atc() && e.reception == Reception::Heard)
    );
    assert!(
        s.flight_record()
            .events
            .iter()
            .any(|e| e.text == "No readback yet. Read back the landing clearance.")
    );
    assert!(
        s.flight_record().events.iter().any(|e| e.text
            == "Sling Three Four Bravo, Allentown Tower, runway two four, cleared to land, wind calm.")
    );
    save(&s, &dir, "three-mile-final")?;
    Ok(())
}
fn save(s: &Session, dir: &std::path::Path, id: &str) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::write(
        dir.join(format!("{id}-transcript.json")),
        serde_json::to_string_pretty(s.flight_record())?,
    )?;
    let mut text = String::from(
        "Reproducible scenario transcript. Released Eastern Pennsylvania terrain; published FAA catalogue.
Simulated ATC is a transcript tag, never spoken. State changes are retained only in the JSON event log.\n\n",
    );
    let stations = s.comms_state().stations;
    for line in s
        .flight_record()
        .events
        .iter()
        .filter_map(|e| e.transcript_line(&stations))
    {
        text.push_str(&line);
        text.push('\n');
    }
    std::fs::write(dir.join(format!("{id}-transcript.txt")), text)?;
    Ok(())
}
