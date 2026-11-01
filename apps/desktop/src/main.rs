//! Native entry point, including unattended screenshot capture.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use eframe::egui;
    use flightcraft_ui::{AppOptions, FlightCraftApp, Screenshot};
    use std::path::PathBuf;

    let mut path = None;
    let mut frames = 120_u64;
    let mut at_sim_s: Option<f64> = None;
    let mut script = None;
    let mut control = None;
    let mut packs_directory = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
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
                    "flightcraft [--packs-dir <directory>] [--control <port>] [--script <flight.jsonl>] [--screenshot <path.png> [--frames <N> | --at-time <seconds>]]\nFly the Corvid. Open Input bindings for keyboard and gamepad controls.\n--packs-dir loads aircraft packs and saves the directory preference.\n--control listens only on 127.0.0.1 for shared-session commands.\n--script runs timed commands (see crates/ui/src/script.rs) and holds the controls until it ends."
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
            .with_title("FlightCraft · Corvid")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([1000.0, 640.0]),
        multisampling: 0,
        ..Default::default()
    };
    eframe::run_native(
        "FlightCraft",
        options,
        Box::new(move |cc| {
            Ok(Box::new(FlightCraftApp::new(
                cc,
                AppOptions {
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
