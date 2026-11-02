#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use super::*;
use crate::instruments::{InstrumentInput, InstrumentPanel};
use flightcraft_control::*;
use serde_json::json;
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/radio_fixture.rs"
));

fn training_session() -> Session {
    let mut s = radio_session();
    let nav: flightcraft_world::Navdata =
        serde_json::from_str(include_str!("../../../../assets/terrain/new-york/navdata.json")).unwrap();
    s.run(CommsFacilities {
        stations: Some(nav.comms),
    })
    .unwrap();
    tune_ctaf(&mut s);
    s.execute(
        "comms.callsign",
        json!({"registration":"N1234B","spoken_type":"Sling"}),
    )
    .unwrap();
    s
}
fn tune_ctaf(s: &mut Session) {
    let channel = s
        .comms_state()
        .stations
        .iter()
        .find(|st| st.airport == "1N7" && st.kind == flightcraft_world::FacilityKind::Ctaf)
        .unwrap()
        .channel_khz;
    s.execute("comms.tune", json!({"radio":1,"channel_khz":channel}))
        .unwrap();
}
fn equipment(s: &mut Session) {
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/packs/equipment-demo");
    s.install_pack(&read_pack(&directory).unwrap()).unwrap();
    s.execute(
        "aircraft.load",
        json!({"source":{"kind":"installed","id":"equipment-demonstrator"},"reset":true}),
    )
    .unwrap();
}
#[test]
#[ignore = "writes radio UI previews and a staged pattern transcript"]
fn render_radio_previews() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/radio");
    std::fs::create_dir_all(&directory).unwrap();
    let mut s = training_session();
    equipment(&mut s);
    tune_ctaf(&mut s);
    let configuration = s.aircraft_configuration().unwrap().clone();
    let mut runtime = InstrumentPanel::default();
    let mut tune1 = String::new();
    let mut tune2 = String::new();
    let mut error = None;
    let image = crate::instruments::preview::rasterize(1600, 1000, |ui| {
        ui.label(
            RichText::new("FlightCraft · equipment demonstrator · installed audio panel")
                .size(23.0)
                .color(theme::ACCENT),
        );
        ui.allocate_ui(egui::vec2(1560.0, 910.0), |ui| {
            ui.set_max_height(910.0);
            let state = s.glass_state();
            runtime
                .show(
                    ui,
                    &configuration.panel,
                    InstrumentInput::new(s.sim().systems(), s.sim().state().time_s)
                        .with_aircraft(&configuration)
                        .with_glass(&state),
                    s.flight_generation(),
                )
                .unwrap();
        });
    });
    image.save(directory.join("equipment-audio-panel.png")).unwrap();
    crate::instruments::preview::rasterize(800, 360, |ui| {
        ui.label(
            RichText::new("Audio panel · equipment demonstrator")
                .size(22.0)
                .color(theme::ACCENT),
        );
        let state = s.comms_state();
        panel(ui, &mut s, &state, &mut tune1, &mut tune2, &mut error);
    })
    .save(directory.join("radio-controls.png"))
    .unwrap();
    let mut s = training_session();
    s.execute("traffic.set", json!({"count":2,"airport":"1N7","runway":"25"}))
        .unwrap();
    s.execute("sim.set_controls", json!({"brake":1.0})).unwrap();
    s.run(Step { steps: 16 * 120 }).unwrap();
    s.execute(
        "comms.transmit",
        json!({"text":"Blairstown traffic, {callsign}, taxiing runway 25, Blairstown"}),
    )
    .unwrap();
    s.run(Step { steps: 12 * 120 }).unwrap();
    let [park_n, park_e, park_d] = s.sim().state().position_m;
    let parked_height = -park_d - s.sim().world().ground(park_n, park_e).height_msl_m;
    assert!(!s.sim().readout().crashed, "fixture ramp must be stable");
    // A staged replay exercises all radio moments without claiming a flight-model acceptance run.
    let world = s.sim().world().clone();
    let region = world.region().unwrap();
    let projection =
        flightcraft_world::geo::LocalProjection::new(region.origin.latitude_deg, region.origin.longitude_deg)
            .unwrap();
    let airport = world.airports().unwrap().airports.first().unwrap();
    let runway = airport.runways.first().unwrap();
    let near = runway.ends.iter().find(|e| e.ident == "25").unwrap();
    let far = runway.ends.iter().find(|e| e.ident == "07").unwrap();
    let a = projection.project(near.latitude_deg, near.longitude_deg).unwrap();
    let b = projection.project(far.latitude_deg, far.longitude_deg).unwrap();
    let heading = (b[1] - a[1]).atan2(b[0] - a[0]);
    let f = [heading.cos(), heading.sin()];
    for (leg, along, side, turn) in [
        ("departing", 1600.0, 0.0, 0.0),
        ("crosswind", 4000.0, 700.0, -std::f64::consts::FRAC_PI_2),
        ("downwind", 1000.0, 1852.0, std::f64::consts::PI),
        ("base", -2700.0, 900.0, std::f64::consts::FRAC_PI_2),
        ("final", -2200.0, 0.0, 0.0),
    ] {
        let n = a[0] + along * f[0] + side * f[1];
        let e = a[1] + along * f[1] - side * f[0];
        let h = heading + turn;
        let state = flightcraft_sim::State {
            time_s: s.sim().state().time_s,
            position_m: [n, e, -near.elevation_m - 304.8],
            velocity_mps: [40.0 * h.cos(), 40.0 * h.sin(), 0.0],
            attitude: [(h / 2.0).cos(), 0.0, 0.0, (h / 2.0).sin()],
            angular_rate_rps: [0.0; 3],
        };
        *s.sim_mut() =
            flightcraft_sim::Sim::from_state_in_world(state, Default::default(), world.clone()).unwrap();
        let magnetic_heading = s.sim().heading_reference().grid_to_magnetic(h);
        s.execute(
            "autopilot.set",
            json!({"altitude_m":near.elevation_m+304.8,"heading_rad":magnetic_heading,"airspeed_mps":40.0}),
        )
        .unwrap();
        s.execute(
            "comms.transmit",
            json!({"text":format!("Blairstown traffic, {{callsign}}, {leg} runway 25, Blairstown")}),
        )
        .unwrap();
        s.run(Step { steps: 12 * 120 }).unwrap();
    }
    // Keep the ownship on a parked replay sample while the traffic continues around the circuit.
    let mut state = *s.sim().state();
    state.position_m = [
        a[0] + 200.0 * f[0] + 120.0 * f[1],
        a[1] + 200.0 * f[1] - 120.0 * f[0],
        -near.elevation_m - 1.0,
    ];
    state.position_m[2] = -world
        .ground(state.position_m[0], state.position_m[1])
        .height_msl_m
        - parked_height;
    state.velocity_mps = [0.0; 3];
    state.angular_rate_rps = [0.0; 3];
    state.attitude = [(heading / 2.0).cos(), 0.0, 0.0, (heading / 2.0).sin()];
    *s.sim_mut() = flightcraft_sim::Sim::from_state_in_world(
        state,
        flightcraft_sim::Controls {
            brake: 1.0,
            ..Default::default()
        },
        world,
    )
    .unwrap();
    s.execute(
        "comms.transmit",
        json!({"text":"Blairstown traffic, {callsign}, clear of runway 25, Blairstown"}),
    )
    .unwrap();
    s.run(Step { steps: 12 * 120 }).unwrap();
    let mut text = String::from(
        "1N7 staged radio pattern · two deterministic traffic aircraft\nReal FAA runway geometry; NASR 2026-10-29 COM frequencies and original terrain.\nOwnship samples are staged, not a flight dynamics acceptance run.\n\n",
    );
    for event in &s.flight_record().events {
        text.push_str(&format!(
            "{:6.2} {:?} {} | {} | {:?}\n",
            event.time_s,
            event.channel,
            speaker(&event.speaker),
            event.text,
            event.reception
        ));
        if let Some(grade) = &event.grade {
            text.push_str(&format!("       Grade: {} · {}\n", grade.passed, grade.feedback));
        }
    }
    std::fs::write(directory.join("full-pattern.txt"), text).unwrap();
    std::fs::write(
        directory.join("flight-record.json"),
        serde_json::to_string_pretty(s.flight_record()).unwrap(),
    )
    .unwrap();
    s.execute("map.set", json!({"range_nm":4.0,"orientation":"north_up"}))
        .unwrap();
    let mut map = crate::map::PlanningMap::default();
    crate::instruments::preview::rasterize(1600, 850, |ui| {
        ui.label(
            RichText::new("1N7 · radio pattern · two traffic aircraft")
                .size(23.0)
                .color(theme::ACCENT),
        );
        ui.colored_label(
            theme::AMBER,
            "Preview replay · NASR 2026-10-29 · CTAF 123.000 · original preview terrain · FAA runway geometry",
        );
        if let Some(event) = s
            .flight_record()
            .events
            .iter()
            .rev()
            .find(|e| e.reception == Reception::Heard)
        {
            ui.group(|ui| {
                ui.label(
                    RichText::new(format!("CAPTIONS   {} · {}", speaker(&event.speaker), event.text))
                        .color(theme::ACCENT),
                );
            });
        }
        ui.columns(2, |columns| {
            let rect = columns[0].available_rect_before_wrap();
            map.show_mfd(&mut columns[0], rect, &mut s);
            transcript(&mut columns[1], &s);
        });
    })
    .save(directory.join("pattern-captions-kneeboard.png"))
    .unwrap();
    let mut readme = String::from(
        "Generated from actual egui controls by render_radio_previews.\nThe equipment image includes the installed panel buttons; radio-controls.png shows the command-backed radio window.\nPattern replay uses FAA 1N7 runway geometry, original flat terrain, NASR 2026-10-29 COM frequencies, staged ownship positions, and live deterministic traffic.\nCOM frequencies come from the pinned official NASR extract; this simulation preview is not for navigation.\n",
    );
    readme.push_str(
        "Run through the build queue: cargo test -p flightcraft-ui render_radio_previews -- --ignored\n",
    );
    std::fs::write(directory.join("README.txt"), readme).unwrap();
    render_awos(&directory);
    if let Some(pack) = std::env::var_os("FLIGHTCRAFT_SLING_PACK") {
        render_sling(&directory, std::path::Path::new(&pack));
    }
}

fn render_awos(out: &std::path::Path) {
    let mut s = training_session();
    let nav: flightcraft_world::Navdata =
        serde_json::from_str(include_str!("../../../../assets/terrain/new-york/navdata.json")).unwrap();
    let station = nav
        .comms
        .into_iter()
        .find(|s| s.kind == flightcraft_world::FacilityKind::Awos && s.airport == "N40")
        .unwrap();
    let airport = station.airport.clone();
    // Use the installed real region and the actual weather station's airport.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain/new-york");
    let world = flightcraft_control::read_region(&root).unwrap();
    let end = world
        .airports()
        .unwrap()
        .airports
        .iter()
        .find(|a| a.ident == airport || a.icao.as_deref() == Some(&airport))
        .unwrap()
        .runways
        .first()
        .unwrap()
        .ends
        .first()
        .unwrap()
        .ident
        .clone();
    s.install_region(world).unwrap();
    s.execute(
        "sim.reset",
        json!({"world":"new-york","start":{"kind":"airport_runway","airport":airport,"end":end}}),
    )
    .unwrap();
    s.execute("sim.set_controls", json!({"brake":1.0})).unwrap();
    s.execute("weather.set", json!({"layers":[{"base_ft_msl":station.elevation_m/0.3048+600.,"top_ft_msl":station.elevation_m/0.3048+3600.,"coverage":"OVC"}],"visibility_sm":2.,"obscuration":"mist","temperature_c":9.,"dewpoint_c":8.,"sea_level_pressure_pa":100600.,"wind_from_deg":240.,"wind_speed_mps":5.14444,"shear":false,"turbulence":"off","seed":0})).unwrap();
    s.execute("comms.tune", json!({"radio":1,"channel_khz":station.channel_khz}))
        .unwrap();
    s.run(Step { steps: 45 * 120 }).unwrap();
    let event = s
        .flight_record()
        .events
        .iter()
        .find(|e| matches!(e.meaning, Meaning::Weather { .. }))
        .unwrap();
    assert_eq!(event.reception, Reception::Heard);
    std::fs::write(
        out.join("awos-ifr.txt"),
        format!(
            "{} · {:.3} MHz · NASR 2026-10-29\n{}\n{:#?}\n",
            station.airport,
            station.channel_khz as f64 / 1000.,
            event.text,
            event.meaning
        ),
    )
    .unwrap();
    crate::panel_preview::rasterize(1500, 620, |ui| {
        ui.heading(format!(
            "{} · IFR automated weather · {:.3} MHz",
            station.airport,
            station.channel_khz as f64 / 1000.
        ));
        ui.colored_label(
            theme::ACCENT,
            "NASR 2026-10-29 channel · simulation weather · 600 ft AGL overcast / 2 SM mist",
        );
        ui.label(crate::weather::note(&s));
        ui.separator();
        ui.label(RichText::new(&event.text).size(23.));
        ui.separator();
        transcript(ui, &s);
    })
    .unwrap()
    .save(out.join("awos-ifr.png"))
    .unwrap();
}
fn render_sling(out: &std::path::Path, path: &std::path::Path) {
    let mut pack = read_pack(path).unwrap();
    let panel = pack.files.get_mut("panel.json").unwrap();
    let mut value: serde_json::Value = serde_json::from_str(panel).unwrap();
    value["equipment"]["elements"].as_array_mut().unwrap().push(json!({"id":"radio-audio-preview","position":[570.,140.],"size":[190.,34.],"element":{"kind":"audio_panel"}}));
    for element in value["equipment"]["elements"].as_array_mut().unwrap() {
        if element["id"] == "registration" {
            element["element"]["text"] = json!("N1234B");
        }
    }
    *panel = serde_json::to_string(&value).unwrap();
    let mut s = training_session();
    s.install_pack(&pack).unwrap();
    s.execute(
        "aircraft.load",
        json!({"source":{"kind":"installed","id":"sling2"},"reset":true}),
    )
    .unwrap();
    tune_ctaf(&mut s);
    s.execute(
        "comms.callsign",
        json!({"registration":"N1234B","spoken_type":"Sling"}),
    )
    .unwrap();
    crate::panel_preview::render_equipment_panel(&mut s, 1728, 968)
        .unwrap()
        .save(out.join("sling-audio-panel.png"))
        .unwrap();
    std::fs::write(out.join("sling-preview.txt"),"External Sling pack loaded read-only. Audio-panel face added to its in-memory installation at [570,140], sharing the declared avionics load. Source pack unchanged. No external artwork copied into the repository.\n").unwrap();
}
