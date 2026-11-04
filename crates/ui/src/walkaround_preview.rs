//! Offscreen captures of the pre-flight walkaround through the real exterior PBR pipeline.
//!
//! Needs an offscreen GPU and `FLIGHTCRAFT_SLING_PACK` (the Sling pack directory). Writes PNGs to
//! `out/preflight/`.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::{test_capture::Capture, theme};
use flightcraft_aero::walkaround::FigureAction;
use flightcraft_control::{
    PreflightAdvance, PreflightBegin, PreflightPerform, PreflightSpeed, PreflightState, Session,
};
use flightcraft_render::{CameraMode, Scene};

type Frame = (&'static str, &'static str, fn(&PreflightState) -> bool);

fn advance_until(
    session: &mut Session,
    seconds: f64,
    what: &str,
    mut hit: impl FnMut(&PreflightState) -> bool,
) -> PreflightState {
    for _ in 0..4000 {
        let st = session.run(PreflightAdvance { seconds }).unwrap();
        if hit(&st) {
            return st;
        }
    }
    panic!("never reached: {what}");
}

#[test]
#[ignore = "requires an offscreen GPU and FLIGHTCRAFT_SLING_PACK"]
fn render_preflight_walkaround_frames() {
    pollster::block_on(async {
        let pack = std::env::var("FLIGHTCRAFT_SLING_PACK").unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let out = root.join("out/preflight");
        std::fs::create_dir_all(&out).unwrap();
        let mut capture = Capture::new([1280, 800]).await;
        let mut session = Session::new().unwrap();
        session
            .install_pack(&flightcraft_control::read_pack(std::path::Path::new(&pack)).unwrap())
            .unwrap();
        session
            .execute(
                "aircraft.load",
                serde_json::json!({"source":{"kind":"installed","id":"sling2"},"reset":true}),
            )
            .unwrap();
        session
            .run(PreflightBegin {
                lesson: None,
                seed: None,
                random_findings: None,
                findings: vec![],
                covers_on: Some(true),
                planned_fuel_l: Some(40.),
                speed: Some(3.),
            })
            .unwrap();
        let asset = session.aircraft_catalog().exterior("sling2").unwrap();
        let mut scene = Scene::default();
        scene.set_aircraft("sling2");
        scene.set_exterior(asset);
        scene.set_world(session.sim().world());
        scene.airspace_visible = false;
        scene.camera = CameraMode::Walkaround;
        let mut shots = Vec::new();
        let frames: [Frame; 4] = [
            ("left-aileron", "aileron_left", |st| {
                st.action == FigureAction::MovingControl
                    && st
                        .scene
                        .surfaces
                        .iter()
                        .any(|s| s.node == "aileron_left" && s.value > 0.6)
            }),
            ("right-main-wheel", "wheel_brakes_right", |st| {
                st.action == FigureAction::Crouching && st.figure.crouch > 0.8
            }),
            ("fuel-filler", "fuel_level_right", |st| {
                st.scene.fuel_view.is_some()
            }),
            ("nose-cowl", "nose_cowling", |st| {
                st.action == FigureAction::Touching || st.figure.lean_rad > 0.29
            }),
        ];
        for (name, item, hit) in frames {
            session.run(PreflightPerform { item: item.into() }).unwrap();
            // Walk quickly, then play the check at its own pace.
            let mut arrived = false;
            let st = advance_until(&mut session, 0.04, name, |st| {
                if !arrived && st.action != FigureAction::Walking && st.current_item.is_some() {
                    arrived = true;
                }
                arrived && hit(st)
            });
            // Hold the moment: slow the figure right down and let the camera settle on the part.
            session.run(PreflightSpeed { speed: 0.25 }).unwrap();
            for _ in 0..50 {
                session.run(PreflightAdvance { seconds: 0.04 }).unwrap();
            }
            session.run(PreflightSpeed { speed: 3. }).unwrap();
            let _ = st;
            scene.set_exterior_state(session.sim());
            scene.set_walkaround(session.preflight_scene());
            let state = *session.sim().state();
            capture.draw(|ui| scene.show(ui, &state, 1., theme::scene()));
            let image = capture.image();
            image.save(out.join(format!("{name}.png"))).unwrap();
            eprintln!("preflight frame {}", out.join(format!("{name}.png")).display());
            shots.push(image);
            // Finish the item so the next walk starts from a settled figure.
            advance_until(&mut session, 0.05, "item done", |st| {
                st.current_item.is_none() && st.queue.is_empty()
            });
        }
        let distinct: std::collections::BTreeSet<_> = shots
            .iter()
            .map(|i| i.as_raw().iter().map(|b| u64::from(*b)).sum::<u64>())
            .collect();
        assert_eq!(distinct.len(), 4, "the four frames must differ");
        assert!(session.preflight_state(false).items.iter().any(|i| i.done));
    });
}
