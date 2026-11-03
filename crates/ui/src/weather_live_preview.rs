//! Opt-in GPU captures: synthetic fixture and independently fetched live station data.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::{test_capture::Capture, weather_live::LiveUi};
use flightcraft_control::{Session, SourceStatus, WeatherSource};
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/radio_fixture.rs"
));
fn preview_session() -> Session {
    let mut session = Session::new().unwrap();
    session.install_region(radio_fixture(false)).unwrap();
    session.execute("sim.reset",serde_json::json!({"world":"radio-fixture","start":{"kind":"airport_runway","airport":"1N7","end":"25"}})).unwrap();
    session
}
fn status(observation: flightcraft_control::LiveObservation) -> SourceStatus {
    let now = flightcraft_control::live_weather::runtime::now();
    SourceStatus {
        source: observation.source.clone(),
        age_seconds: Some(observation.age_seconds(now)),
        stale: observation.stale(now),
        last_good: Some(observation),
        error: None,
        fetching: false,
        next_attempt_at: now + 60,
        failures: 0,
    }
}
fn capture(mut session: Session, title: &str, filename: &str) {
    pollster::block_on(async {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let out = root.join("out/live-weather");
        std::fs::create_dir_all(&out).unwrap();
        let mut gpu = Capture::new([1408, 1056]).await;
        let mut panel = LiveUi::default();
        for _ in 0..3 {
            gpu.draw(|ui| {
                ui.heading(title);
                ui.separator();
                panel.panel(ui, &mut session).unwrap();
            });
        }
        gpu.image().save(out.join(filename)).unwrap();
        gpu.validate().await;
    });
}
#[test]
#[ignore = "requires an offscreen GPU; original invented fixture, no external requests"]
fn render_live_weather_fixture_preview() {
    let mut session = preview_session();
    let source =
        WeatherSource::from_json(include_str!("../../../examples/weather-sources/1n7-digiwx.json")).unwrap();
    let at = 1_793_715_300; // 2026-11-03 14:15 UTC, the fixture's invented observation time.
    let observation = flightcraft_control::LiveObservation::from_body(
        source,
        include_bytes!("../../control/tests/fixtures/local-weather.html"),
        at,
    )
    .unwrap();
    let nearby = flightcraft_control::live_weather::nearby::parse_reports(
        include_bytes!("../../control/tests/fixtures/nearby-metar.json"),
        [40.97, -75.0],
        "1N7",
        at,
    )
    .unwrap();
    let mut snapshot = session.screen_snapshot();
    snapshot
        .live_weather
        .sources
        .insert(observation.source.id.clone(), status(observation));
    for station in &nearby {
        snapshot.live_weather.sources.insert(
            station.observation.source.id.clone(),
            status(station.observation.clone()),
        );
    }
    snapshot.live_weather.nearby.insert(
        "1N7".into(),
        flightcraft_control::live_weather::nearby::NearbyStatus {
            stations: nearby,
            ..Default::default()
        },
    );
    session.apply_screen_snapshot(snapshot).unwrap();
    capture(
        session,
        "Fixture preview · invented values, not current weather",
        "1n7-fixture.png",
    );
}
#[test]
#[ignore = "requires external weather network access and an offscreen GPU; never run in CI"]
fn render_live_weather_network_preview() {
    let mut session = preview_session();
    let source =
        WeatherSource::from_json(include_str!("../../../examples/weather-sources/1n7-digiwx.json")).unwrap();
    let body = flightcraft_control::live_weather::fetch::fetch(&source).unwrap();
    let now = flightcraft_control::live_weather::runtime::now();
    let observation = flightcraft_control::LiveObservation::from_body(source, &body, now).unwrap();
    let mut snapshot = session.screen_snapshot();
    snapshot
        .live_weather
        .sources
        .insert(observation.source.id.clone(), status(observation));
    let discovery =
        flightcraft_control::live_weather::nearby::discovery_source("1N7", [40.97, -75.0]).unwrap();
    if let Ok(body) = flightcraft_control::live_weather::fetch::fetch(&discovery)
        && let Ok(stations) =
            flightcraft_control::live_weather::nearby::parse_reports(&body, [40.97, -75.0], "1N7", now)
    {
        for station in &stations {
            snapshot.live_weather.sources.insert(
                station.observation.source.id.clone(),
                status(station.observation.clone()),
            );
        }
        snapshot.live_weather.nearby.insert(
            "1N7".into(),
            flightcraft_control::live_weather::nearby::NearbyStatus {
                stations,
                ..Default::default()
            },
        );
    }
    session.apply_screen_snapshot(snapshot).unwrap();
    capture(session, "Live weather · 1N7 DigiWx Advisor", "1n7-live.png");
}
