//! Native entry point, including unattended screenshot capture.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use eframe::egui;
    use flightcraft_ui::{AppOptions, FlightCraftApp, Screenshot};
    use std::path::PathBuf;

    let mut path = None;
    let mut screen_set = None;
    let mut layouts_dir = None;
    let mut panels = None;
    let mut web_dir = PathBuf::from("dist");
    let mut frames = 120_u64;
    let mut at_sim_s: Option<f64> = None;
    let mut script = None;
    let mut control = None;
    let mut packs_directory = None;
    let mut regions_directory = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            #[cfg(feature = "whisper")]
            "--speech-benchmark" => {
                let request = args
                    .next()
                    .ok_or("--speech-benchmark needs a request JSON path")?;
                let report = flightcraft_ui::speech_input::benchmark::run(std::path::Path::new(&request))?;
                println!("{report}");
                return Ok(());
            }
            "--screens" => {
                screen_set = Some(PathBuf::from(
                    args.next().ok_or("--screens needs a window-set JSON path")?,
                ))
            }
            "--layouts-dir" => {
                layouts_dir = Some(PathBuf::from(
                    args.next().ok_or("--layouts-dir needs a directory")?,
                ))
            }
            "--panels" => {
                panels = Some(
                    args.next()
                        .ok_or("--panels needs a private LAN IPv4:port")?
                        .parse()?,
                )
            }
            "--web-dir" => {
                web_dir = PathBuf::from(
                    args.next()
                        .ok_or("--web-dir needs the packaged browser directory")?,
                )
            }
            "--regions-dir" => {
                regions_directory = Some(args.next().ok_or("--regions-dir needs a directory")?)
            }
            "--packs-dir" => packs_directory = Some(args.next().ok_or("--packs-dir needs a directory")?),
            "--control" => {
                let port: u16 = args
                    .next()
                    .ok_or("--control needs a port from 1 to 65535")?
                    .parse()?;
                if port == 0 {
                    return Err("--control needs a port from 1 to 65535".into());
                }
                control = Some(flightcraft_ui::control_listener(port)?);
            }
            "--screenshot" => path = Some(PathBuf::from(args.next().ok_or("--screenshot needs a PNG path")?)),
            "--frames" => {
                frames = args.next().ok_or("--frames needs a positive integer")?.parse()?;
                if frames == 0 {
                    return Err("--frames must be positive".into());
                }
            }
            "--at-time" => {
                let seconds: f64 = args.next().ok_or("--at-time needs simulated seconds")?.parse()?;
                if !seconds.is_finite() || seconds < 0.0 {
                    return Err("--at-time must be a finite number of seconds, 0 or more".into());
                }
                at_sim_s = Some(seconds);
            }
            "--script" => {
                let file = args.next().ok_or("--script needs a JSON-lines file")?;
                let text = std::fs::read_to_string(&file).map_err(|e| format!("{file}: {e}"))?;
                script =
                    Some(flightcraft_ui::script::Script::parse(&text).map_err(|e| format!("{file}: {e}"))?);
            }
            "--help" | "-h" => {
                println!(
                    "flightcraft [--screens <window-set.json>] [--layouts-dir <directory>] [--panels <private-ip:port> --web-dir <dist>] [--regions-dir <directory>] [--packs-dir <directory>] [--control <port>] [--script <flight.jsonl>] [--screenshot <path.png> [--frames <N> | --at-time <seconds>]]\nFly the Corvid. Open Input bindings for keyboard and gamepad controls.\n--regions-dir loads terrain regions (default assets/terrain).\n--packs-dir loads aircraft packs and saves the directory preference.\n--control listens only on 127.0.0.1 for shared-session commands.\n--script runs timed commands (see crates/ui/src/script.rs) and holds the controls until it ends."
                );
                return Ok(());
            }
            _ => return Err(format!("Unknown option: {arg}").into()),
        }
    }
    if let Some(path) = &path
        && path.extension().is_none_or(|ext| ext != "png")
    {
        return Err("--screenshot path must end in .png".into());
    }
    let capture = path.map(|path| {
        let shot = Screenshot::new(path, frames);
        match at_sim_s {
            Some(seconds) => shot.at_sim_time(seconds),
            None => shot,
        }
    });
    let requested_capture = capture.clone();
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title("FlightCraft · The Art of Flying")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([1000.0, 640.0]),
        multisampling: 0,
        ..Default::default()
    };
    // A downloaded build is started from anywhere (a double-click starts in `/` on macOS), so fall
    // back to the regions shipped beside the executable when the working directory has none.
    let cwd = std::env::current_dir()?;
    let executable = std::env::current_exe().ok();
    if regions_directory.is_none() {
        regions_directory = alongside_exe(&cwd, executable.as_deref(), "assets/terrain");
    }
    if packs_directory.is_none() {
        packs_directory = alongside_exe(&cwd, executable.as_deref(), "packs");
    }
    eframe::run_native(
        "FlightCraft",
        options,
        Box::new(move |cc| {
            Ok(Box::new(FlightCraftApp::new(
                cc,
                AppOptions {
                    layout: None,
                    screen_set,
                    layouts_dir,
                    panels: panels.map(|address| (address, web_dir)),
                    regions: flightcraft_ui::RegionOptions {
                        directory: regions_directory,
                        ..Default::default()
                    },
                    screenshot: capture,
                    control,
                    script,
                    packs: flightcraft_ui::PackOptions {
                        directory: packs_directory,
                        ..Default::default()
                    },
                },
            )?))
        }),
    )
    .map_err(|e| std::io::Error::other(e.to_string()))?;
    if requested_capture.is_some_and(|capture| !capture.succeeded()) {
        return Err("Screenshot was not saved; see the renderer error above".into());
    }
    Ok(())
}

// Keep every application crate buildable for the browser target.
#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn alongside_exe(cwd: &std::path::Path, exe: Option<&std::path::Path>, relative: &str) -> Option<String> {
    if cwd.join(relative).is_dir() {
        return None;
    }
    exe.and_then(std::path::Path::parent)
        .map(|p| p.join(relative))
        .filter(|p| p.is_dir())
        .and_then(|p| p.to_str().map(str::to_owned))
}
#[cfg(all(test, not(target_arch = "wasm32")))]
#[allow(clippy::unwrap_used)]
mod resource_tests {
    use super::*;
    #[test]
    fn portable_build_discovers_packs_and_terrain_without_changing_working_directory() {
        let root = std::env::temp_dir().join(format!("flightcraft-resources-{}", std::process::id()));
        let build = root.join("build");
        let elsewhere = root.join("elsewhere");
        std::fs::create_dir_all(build.join("packs/sling2")).unwrap();
        std::fs::create_dir_all(build.join("assets/terrain/new-york")).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        let exe = build.join("flightcraft");
        let before = std::env::current_dir().unwrap();
        for _ in 0..5 {
            // Each extra window uses the same loaded paths; no chdir is required.
            assert_eq!(
                alongside_exe(&elsewhere, Some(&exe), "packs"),
                Some(build.join("packs").to_str().unwrap().into())
            );
            assert_eq!(
                alongside_exe(&elsewhere, Some(&exe), "assets/terrain"),
                Some(build.join("assets/terrain").to_str().unwrap().into())
            );
        }
        assert_eq!(before, std::env::current_dir().unwrap());
        std::fs::create_dir_all(elsewhere.join("packs")).unwrap();
        assert_eq!(alongside_exe(&elsewhere, Some(&exe), "packs"), None);
        std::fs::remove_dir_all(root).unwrap();
    }
}
