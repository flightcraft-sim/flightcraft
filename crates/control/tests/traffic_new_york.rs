#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
//! Busy-airspace traffic in the installed new-york region, which the flat ATC fixture never loads.
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/region_data.rs"));
use flightcraft_control::{FleetState, Session, Step};
use serde_json::json;

fn new_york_session(airport: &str, end: &str) -> Session {
    let mut s = Session::new().unwrap();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain/new-york");
    s.install_region(flightcraft_control::read_region(&dir).unwrap())
        .unwrap();
    s.execute(
        "sim.reset",
        json!({"world":"new-york","start":{"kind":"airport_runway","airport":airport,"end":end}}),
    )
    .unwrap();
    s.execute(
        "comms.callsign",
        json!({"registration":"N371FC","spoken_type":"Sling"}),
    )
    .unwrap();
    s
}

fn advance(s: &mut Session, seconds: f64) {
    let mut left = seconds;
    while left > 0.0 {
        let chunk = left.min(300.0);
        left -= chunk;
        s.run(Step {
            steps: (chunk * 120.).ceil() as u32,
        })
        .unwrap();
    }
}

fn state(s: &mut Session) -> FleetState {
    serde_json::from_value(s.execute("traffic.state", json!({})).unwrap()).unwrap()
}

#[test]
fn busy_kmmu_runs_in_the_real_new_york_region() {
    require_region_data!();
    let mut s = new_york_session("KMMU", "05");
    let out = s
        .execute("traffic.density", json!({"density":"busy","airport":"KMMU"}))
        .unwrap();
    let initial: FleetState = serde_json::from_value(out).unwrap();
    assert_eq!(initial.airport.as_deref(), Some("KMMU"));
    assert!(
        (10..=15).contains(&initial.aircraft.len()),
        "{}",
        initial.aircraft.len()
    );
    advance(&mut s, 600.);
    let st = state(&mut s);
    assert!((10..=15).contains(&st.aircraft.len()), "{}", st.aircraft.len());
    assert_eq!(st.stats.safety_slowdowns, 0);
    assert!(st.stats.transmissions > 20, "{:?}", st.stats);
    assert!(st.aircraft.iter().all(|a| a.intent.airport == "KMMU"));
    // Every aircraft is on a frequency that belongs to a real facility of the region.
    for a in &st.aircraft {
        assert!(
            a.frequency_khz > 118_000 && a.frequency_khz < 137_000,
            "{}",
            a.frequency_khz
        );
        assert!(!a.facility.is_empty());
    }
}

#[test]
fn busy_kmmu_and_kewr_together_in_the_real_new_york_region() {
    require_region_data!();
    let mut s = new_york_session("KMMU", "05");
    s.execute(
        "traffic.density",
        json!({"density":"moderate","airport":"KMMU","airports":["KEWR"]}),
    )
    .unwrap();
    let mut crossed = std::collections::BTreeMap::<String, std::collections::BTreeSet<String>>::new();
    for _ in 0..(2400 / 20) {
        advance(&mut s, 20.);
        let st = state(&mut s);
        let names: Vec<&str> = st.airports.iter().map(|a| a.airport.as_str()).collect();
        assert_eq!(names, ["KMMU", "KEWR"]);
        let mut ids: Vec<u32> = st.aircraft.iter().map(|a| a.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), st.aircraft.len());
        for a in st.aircraft {
            crossed.entry(a.callsign).or_default().insert(a.intent.airport);
        }
    }
    let st = state(&mut s);
    assert_eq!(st.stats.safety_slowdowns, 0);
    assert!(st.stats.transmissions > 20, "{:?}", st.stats);
    assert!(
        crossed.values().any(|v| v.len() == 2),
        "no transit flight between KMMU and KEWR in 40 minutes"
    );
}
