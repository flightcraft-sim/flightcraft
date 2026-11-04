//! Offscreen captures of the real lesson, procedure flight, PFD, and instructor widgets.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use crate::{
    instruments::{InstrumentInput, InstrumentPanel},
    test_capture::Capture,
    theme,
};
use eframe::egui::{self, Rect};
use flightcraft_control::*;
use flightcraft_render::{CameraMode, Scene};
fn cockpit(ui: &mut egui::Ui, s: &Session, scene: &mut Scene, panel: &mut InstrumentPanel) {
    let full = ui.max_rect();
    let split = full.top() + full.height() * 0.50;
    ui.scope_builder(
        egui::UiBuilder::new().max_rect(Rect::from_min_max(full.min, egui::pos2(full.right(), split))),
        |ui| {
            scene.set_weather(s.sim().conditions(), s.sim().hood());
            scene.show(ui, s.sim().state(), 1. / 60., theme::scene());
        },
    );
    let glass = s.glass_state();
    ui.scope_builder(
        egui::UiBuilder::new().max_rect(Rect::from_min_max(egui::pos2(full.left(), split), full.max)),
        |ui| {
            ui.painter().rect_filled(ui.max_rect(), 0., theme::PANEL);
            let config = s.aircraft_configuration().unwrap();
            let registration = s.pilot_callsign().registration;
            panel
                .show(
                    ui,
                    &config.panel,
                    InstrumentInput::new(s.sim().systems(), s.sim().state().time_s)
                        .with_glass(&glass)
                        .with_aircraft(config)
                        .with_registration(&registration)
                        .with_autopilot(s.sim().installed_autopilot()),
                    s.flight_generation(),
                )
                .unwrap();
        },
    );
}
#[test]
#[ignore = "requires an offscreen GPU and released regional terrain; writes out/ifr captures and actual KABE transcript"]
fn render_ifr_previews() {
    pollster::block_on(async {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let out = root.join("out/ifr");
        std::fs::create_dir_all(&out).unwrap();
        let mut s = Session::new().unwrap();
        for region in ["eastern-pa", "new-york"] {
            s.install_region(read_region(&root.join("assets/terrain").join(region)).unwrap())
                .unwrap();
        }
        s.enable_app();
        s.run(LessonStart {
            lesson: lesson_library()
                .unwrap()
                .into_iter()
                .find(|l| l.id == "kabe-ils-06")
                .unwrap(),
        })
        .unwrap();
        let mut capture = Capture::new([1280, 1000]).await;
        let mut instructor = crate::screens::PageUi::default();
        let page = flightcraft_control::screens::Page {
            kind: flightcraft_control::screens::PageKind::Instructor,
            ..Default::default()
        };
        for _ in 0..3 {
            capture.draw(|ui| instructor.show(ui, &page, &mut s, "ifr-instructor").unwrap());
        }
        capture.image().save(out.join("instructor.png")).unwrap();
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
                    mode: InstrumentApproach::Ils,
                    minimum_ft: 594.,
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
        let mut scene = Scene::default();
        scene.set_world(s.sim().world());
        scene.airspace_visible = false;
        scene.camera = CameraMode::Cockpit;
        let mut panel = InstrumentPanel::default();
        let mut imc = false;
        let mut breakout = false;
        for _ in 0..900 {
            s.run(Step { steps: 120 }).unwrap();
            let r = s.sim().readout();
            if !imc && r.altitude_msl_m / 0.3048 < 2100. && s.sim().radio_state()[0].vertical_dots.is_some() {
                // Remove only the training hood for this IMC capture. The actual weather still hides the horizon.
                let mut view = s.presentation_copy();
                view.run(SetHood { enabled: false }).unwrap();
                for _ in 0..3 {
                    capture.draw(|ui| cockpit(ui, &view, &mut scene, &mut panel));
                }
                capture.image().save(out.join("pfd-ils-imc.png")).unwrap();
                imc = true;
            }
            if !breakout && !s.sim().hood() && r.altitude_msl_m / 0.3048 <= 586. {
                for _ in 0..3 {
                    capture.draw(|ui| cockpit(ui, &s, &mut scene, &mut panel));
                }
                capture.image().save(out.join("minimums-breakout.png")).unwrap();
                breakout = true;
            }
            assert!(!r.crashed, "preview flight crashed");
            if r.on_ground {
                break;
            }
        }
        assert!(imc && breakout);
        assert!(
            s.sim().readout().on_ground,
            "preview must include the actual landing"
        );
        s.run(LessonFinish {}).unwrap();
        capture.resize([1280, 1100]);
        let mut debrief = crate::screens::PageUi::default();
        let page = flightcraft_control::screens::Page::new(flightcraft_control::screens::PageKind::Debrief);
        for _ in 0..3 {
            capture.draw(|ui| debrief.show(ui, &page, &mut s, "ifr-debrief").unwrap());
        }
        capture.image().save(out.join("debrief.png")).unwrap();
        std::fs::write(
            out.join("kabe-ils-06-debrief.json"),
            serde_json::to_string_pretty(s.lesson_debrief()).unwrap(),
        )
        .unwrap();
        let transcript = s
            .flight_record()
            .events
            .iter()
            .filter(|e| e.is_spoken() && !e.text.is_empty())
            .map(|e| format!("{:7.1}s  {}\n", e.time_s, e.text))
            .collect::<String>();
        std::fs::write(out.join("kabe-ils-06-transcript.txt"), transcript).unwrap();
        capture.validate().await;
    });
}
