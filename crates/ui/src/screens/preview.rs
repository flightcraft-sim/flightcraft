//! Captures use the real page canvas and wgpu scene, with one presentation resource set per window.
#![allow(clippy::unwrap_used)]
use super::*;
use crate::test_capture::Capture;
use std::{path::Path, time::Instant};
fn new_york_session() -> Session {
    let mut s = Session::new().unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let errors = s.load_regions_directory(&root.join("assets/terrain"));
    assert!(errors.is_empty(), "{errors:?}");
    s.execute("sim.reset",json!({"world":"new-york","start":{"kind":"airport_runway","airport":"1N7","end":"07"},"paused":true})).unwrap();
    s.execute("weather.set", json!({"preset":"vfr"})).unwrap();
    s.execute("map.set", json!({"range_nm":20.,"base_map":"terrain"}))
        .unwrap();
    s
}
#[test]
#[ignore = "requires an offscreen GPU; writes out/screens previews and five-canvas timings"]
fn render_screen_previews() {
    pollster::block_on(async {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let out = root.join("out/screens");
        std::fs::create_dir_all(&out).unwrap();
        let mut s = new_york_session();
        let set: ScreenSet = serde_json::from_str(
            &std::fs::read_to_string(root.join("examples/layouts/home-cockpit-five.json")).unwrap(),
        )
        .unwrap();
        set.validate().unwrap();
        s.restore_screens(set.clone()).unwrap();
        let mut windows = Vec::new();
        for screen in &set.windows {
            windows.push((Capture::new([960, 640]).await, Canvas::default(), screen.clone()));
        }
        for _ in 0..3 {
            for (capture, canvas, screen) in &mut windows {
                capture.draw(|ui| canvas.show(ui, screen, &mut s, true));
            }
        }
        let names = [
            "five-0-outside-left.png",
            "five-1-outside-forward.png",
            "five-2-outside-right.png",
            "five-3-panel.png",
            "five-4-instructor.png",
        ];
        for ((capture, canvas, _), name) in windows.iter().zip(names) {
            assert!(canvas.error.is_none(), "{:?}", canvas.error);
            capture.image().save(out.join(name)).unwrap();
            capture.validate().await;
        }
        let mut times: Vec<Vec<f64>> = (0..5).map(|_| Vec::new()).collect();
        let mut totals = Vec::new();
        for _ in 0..30 {
            let start = Instant::now();
            for ((capture, canvas, screen), measurements) in windows.iter_mut().zip(&mut times) {
                let before = Instant::now();
                capture.draw(|ui| canvas.show(ui, screen, &mut s, true));
                measurements.push(before.elapsed().as_secs_f64() * 1000.);
            }
            totals.push(start.elapsed().as_secs_f64() * 1000.);
        }
        fn stats(mut values: Vec<f64>) -> Value {
            values.sort_by(f64::total_cmp);
            json!({"median_ms":values.get(values.len()/2),"p95_ms":values.get(values.len()*95/100)})
        }
        let adapter = windows.first().unwrap().0.state.adapter.get_info();
        let report = json!({"method":"Five independent 960×640 offscreen viewport canvases, serial CPU submit plus GPU completion, 30 warm frame sets; excludes OS compositor and swapchain presentation","backend":format!("{:?}",adapter.backend),"device_type":format!("{:?}",adapter.device_type),"per_window":names.into_iter().zip(times).map(|(name,t)|json!({"page":name,"timing":stats(t)})).collect::<Vec<_>>(),"five_window_frame_set":stats(totals)});
        std::fs::write(
            out.join("performance.json"),
            serde_json::to_string_pretty(&report).unwrap(),
        )
        .unwrap();
        for (file, prefix) in [
            ("c172-two-screens.json", "c172"),
            ("glass-touchscreen.json", "touch-glass"),
            ("single-screen.json", "single"),
            ("three-monitor-wrap.json", "wrap"),
            ("instructor-setup.json", "instructor"),
        ] {
            if prefix == "touch-glass" {
                s.execute(
                    "aircraft.load",
                    json!({"source":{"kind":"installed","id":"corvid-glass"},"reset":true}),
                )
                .unwrap();
                s.execute("sim.reset",json!({"world":"new-york","start":{"kind":"airport_runway","airport":"1N7","end":"07"},"paused":true})).unwrap();
                s.execute("sim.set_controls", json!({"throttle":0.,"brake":1.}))
                    .unwrap();
                s.execute("sim.step", json!({"steps":6000})).unwrap();
            }
            if prefix == "single" {
                s = new_york_session();
            }
            let setup: ScreenSet = serde_json::from_str(
                &std::fs::read_to_string(root.join("examples/layouts").join(file)).unwrap(),
            )
            .unwrap();
            setup.validate().unwrap();
            for screen in setup.windows {
                let [w, h] = screen.size;
                let mut capture = Capture::new([w as u32, h as u32]).await;
                let mut canvas = Canvas::default();
                for _ in 0..3 {
                    capture.draw(|ui| canvas.show(ui, &screen, &mut s, true));
                }
                assert!(canvas.error.is_none(), "{:?}", canvas.error);
                capture
                    .image()
                    .save(out.join(format!("{prefix}-{}.png", screen.id)))
                    .unwrap();
                capture.validate().await;
            }
        }
        // Load the exact pinned bundled pack copied by the packaging script into this preview fixture.
        let pack = flightcraft_control::read_pack(&out.join("packs/sling2")).unwrap();
        let mut sling = new_york_session();
        sling.install_pack(&pack).unwrap();
        sling
            .execute(
                "aircraft.load",
                json!({"source":{"kind":"installed","id":"sling2"},"reset":true}),
            )
            .unwrap();
        sling.execute("sim.reset",json!({"world":"new-york","start":{"kind":"airport_runway","airport":"1N7","end":"07"},"paused":true})).unwrap();
        assert!(matches!(
            sling.aircraft_configuration().unwrap().engine.induction,
            flightcraft_aero::Induction::Injection
        ));
        let mut controls = Layout::single(Page::new(PageKind::Control));
        controls.canvas = [950., 580.];
        controls.elements.clear();
        let mut throttle = Page::new(PageKind::Control);
        throttle.control.kind = ControlKind::Throttle;
        controls
            .elements
            .push(Element::page("throttle", throttle, [10., 10.], [200., 550.]));
        for (i, id) in ["fuel_selector", "lane_a", "lane_b", "pump1", "pump2"]
            .into_iter()
            .enumerate()
        {
            let mut page = Page::new(PageKind::PanelItem);
            page.item = Some(id.into());
            controls.elements.push(Element::page(
                id,
                page,
                [230. + (i % 3) as f32 * 235., 10. + (i / 3) as f32 * 280.],
                [220., 260.],
            ));
        }
        let screen = Screen {
            id: 8,
            title: "Sling 2 engine controls".into(),
            monitor: None,
            position: None,
            size: [950., 580.],
            fullscreen: false,
            borderless: false,
            layout: controls,
        };
        let mut capture = Capture::new([950, 580]).await;
        let mut canvas = Canvas::default();
        for _ in 0..3 {
            capture.draw(|ui| canvas.show(ui, &screen, &mut sling, true));
        }
        assert!(canvas.error.is_none(), "{:?}", canvas.error);
        capture.image().save(out.join("sling-controls.png")).unwrap();
        capture.validate().await;
        std::fs::write(out.join("README.txt"),"Actual shared egui/wgpu page canvases. FAA New York terrain, NASR navigation and COM records at 1N7 runway 07; original Corvid aircraft.\nThe five captures share one paused flight snapshot; they are offscreen viewport captures, not a photograph of five OS windows.\nperformance.json states the measurement method and its limits.\nc172-0.png and c172-1.png show the familiar six-pack / tablet split using original fictional aircraft instruments.\ntouch-glass-0.png shows touch-size bezel keys and magnified-knob entry points.\n").unwrap();
    });
}
use serde_json::Value;

#[test]
#[ignore = "starts the packaged browser build on a temporary private host and captures a paired instructor"]
fn remote_instructor_browser_preview() {
    use std::{
        io::Write,
        process::{Command, Stdio},
        time::Duration,
    };
    let chrome = std::env::var("FLIGHTCRAFT_CHROME").unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let terrain = root.join("out/screens/web/assets/terrain/new-york");
    std::fs::create_dir_all(&terrain).unwrap();
    for entry in std::fs::read_dir(root.join("assets/terrain/new-york")).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file()
            && matches!(
                entry.path().extension().and_then(|s| s.to_str()),
                Some("json" | "fcht")
            )
        {
            std::fs::copy(entry.path(), terrain.join(entry.file_name())).unwrap();
        }
    }
    let commands = [
        json!({"at_s":0,"command":"sim.reset","input":{"world":"new-york","start":{"kind":"airport_runway","airport":"1N7","end":"07"},"paused":false}}),
        json!({"at_s":0,"command":"weather.set","input":{"preset":"vfr"}}),
        json!({"at_s":0,"command":"sim.set_controls","input":{"throttle":0,"brake":1}}),
    ];
    std::fs::write(
        root.join("out/screens/web/preview.jsonl"),
        commands
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let mut s = new_york_session();
    s.enable_app();
    let runtime = crate::runtime::Runtime::start(
        s,
        crate::script::Script::default(),
        crate::bridge::Bridge::default(),
        egui::Context::default(),
    )
    .unwrap();
    let server = remote::Server::start(
        "127.0.0.1:0".parse().unwrap(),
        root.join("out/screens/web"),
        runtime.source(),
    )
    .unwrap();
    let mut child = Command::new("python3")
        .arg(root.join("scripts/screens-smoke.py"))
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    let config = json!({"chrome":chrome,"url":format!("http://{}/?panel=remote&page=instructor&gpu=webgl2",server.address()),"token":server.token(),"out":root.join("out/screens")});
    child
        .stdin
        .take()
        .unwrap()
        .write_all(config.to_string().as_bytes())
        .unwrap();
    let start = Instant::now();
    let mut paired = false;
    loop {
        for client in server.clients() {
            paired = true;
            server.grant(client.id, remote::Permission::Control);
        }
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        if start.elapsed() > Duration::from_secs(180) {
            let _ = child.kill();
            let _ = child.wait();
            assert!(
                start.elapsed() <= Duration::from_secs(180),
                "browser preview timed out"
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        paired,
        "browser never paired; inspect out/screens/remote-pairing.png and browser.log"
    );
}
