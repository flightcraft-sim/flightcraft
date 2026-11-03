//! Actual egui widgets on the released New York terrain and FAA airport data.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::{scenario::ScenarioUi, test_capture::Capture};
use flightcraft_control::*;
use serde_json::json;
#[test]
#[ignore = "requires an offscreen GPU and released region data; writes out/scenario previews"]
fn render_scenario_previews() {
    pollster::block_on(async {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let dir = root.join("out/scenario");
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Session::new().unwrap();
        s.install_region(read_region(&root.join("assets/terrain/new-york")).unwrap())
            .unwrap();
        s.execute("sim.reset",json!({"world":"new-york","start":{"kind":"airport_air","airport":"1N7","distance_m":4000.,"bearing_deg":240.,"height_m":450.,"airspeed_mps":33.,"heading_rad":1.}})).unwrap();
        let end = s
            .sim()
            .world()
            .airports()
            .unwrap()
            .airports
            .iter()
            .find(|a| a.ident == "1N7")
            .unwrap()
            .runways
            .iter()
            .flat_map(|r| &r.ends)
            .find(|e| e.ident == "07")
            .unwrap()
            .clone();
        let point = flightcraft_world::GeoPoint {
            latitude_deg: end.latitude_deg,
            longitude_deg: end.longitude_deg,
        }
        .destination((end.heading_deg + 180.).rem_euclid(360.), 2.2 * 1852.);
        s.execute("sim.reset",json!({"world":"new-york","weather":{"wind_from_deg":230.,"wind_speed_mps":4.115552,"shear":false,"turbulence":"off","seed":3407},"start":{"kind":"geographic_air","position":point,"height_agl_m":1450.*0.3048,"airspeed_mps":33.,"heading_rad":end.heading_deg.to_radians()}})).unwrap();
        let mut controls = ScenarioUi::default();
        let mut capture = Capture::new([900, 800]).await;
        let mut content = eframe::egui::Rect::NOTHING;
        for _ in 0..3 {
            capture.draw(|ui| {
                content = eframe::egui::Frame::new()
                    .fill(crate::theme::PANEL)
                    .inner_margin(16.)
                    .show(ui, |ui| {
                        controls.panel(ui, &mut s).unwrap();
                    })
                    .response
                    .rect;
            });
        }
        save_crop(&capture, content, &dir.join("scenario-picker.png"));
        capture.resize([1200, 1100]);
        s.execute(
            "scenario.instructor_failure",
            json!({"failure":{"kind":"engine"},"within_seconds":0.,"seed":3407}),
        )
        .unwrap();
        let r = s.sim().readout();
        s.execute("assist.set",json!({"heading_rad":(end.heading_deg.to_radians()-r.magnetic_declination_rad).rem_euclid(core::f64::consts::TAU),"airspeed_mps":33.,"airspeed_mode":"pitch"})).unwrap();
        s.run(Step { steps: 240 }).unwrap();
        s.run(EmergencyRecord {
            observation: EmergencyObservation::Recognized,
        })
        .unwrap();
        s.run(EmergencyRecord {
            observation: EmergencyObservation::ChooseField { id: "1N7:07".into() },
        })
        .unwrap();
        for item in ["glide", "field"] {
            s.run(EmergencyRecord {
                observation: EmergencyObservation::Checklist { item: item.into() },
            })
            .unwrap();
        }
        let mut flared = false;
        for _ in 0..1200 {
            let r = s.sim().readout();
            if r.on_ground || r.crashed {
                break;
            }
            if !flared && r.height_agl_m < 9. {
                s.execute("assist.set", json!({"airspeed_mps":null,"pitch_rad":0.12}))
                    .unwrap();
                flared = true;
            }
            s.run(Step { steps: 60 }).unwrap();
        }
        let d = s.scenario_state().emergency.clone().unwrap();
        assert!(d.track_m.len() > 10);
        assert!(d.touchdown_m.is_some(), "preview must include touchdown/impact");
        for _ in 0..3 {
            capture.draw(|ui| {
                content = eframe::egui::Frame::new()
                    .fill(crate::theme::PANEL)
                    .inner_margin(16.)
                    .show(ui, |ui| {
                        crate::scenario::debrief(ui, &mut s, &d).unwrap();
                    })
                    .response
                    .rect;
            });
        }
        save_crop(&capture, content, &dir.join("engine-out-debrief-1n7.png"));
        std::fs::write(
            dir.join("engine-out-debrief-1n7.json"),
            serde_json::to_string_pretty(&d).unwrap(),
        )
        .unwrap();
        std::fs::write(dir.join("engine-out-debrief-1n7.txt"),format!("{}\n\nReleased New York terrain, FAA airport/runway catalogue.\nWind from 230 degrees at 8 kt. Actual fixed-step Corvid engine-out flight; no Mayday or 7700.\n",d.summary())).unwrap();
        capture.validate().await;
    });
}
fn save_crop(capture: &Capture, rect: eframe::egui::Rect, path: &std::path::Path) {
    let image = capture.image();
    let width = (rect.right().ceil() as u32).min(image.width());
    let height = (rect.bottom().ceil() as u32).min(image.height());
    image::imageops::crop_imm(&image, 0, 0, width, height)
        .to_image()
        .save(path)
        .unwrap();
}
