//! UI-thread execution and completion of requests received by the native line transport.
use base64::Engine;
use eframe::egui;
use flightcraft_control::{
    AppRequest, AppScreenshot, CommandError, ScreenshotOutput, Session,
    bridge::{self, Listener},
};
use serde_json::{Value, json};
use std::{
    io::{Cursor, Write},
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};

struct Pending {
    id: Value,
    reply: mpsc::Sender<Value>,
    deadline: Instant,
    work: Work,
}
enum Work {
    Navigation(mpsc::Receiver<Result<Value, String>>),
    Wait {
        start: f64,
        target: f64,
        generation: u64,
    },
    Screenshot {
        options: AppScreenshot,
        token: u64,
    },
}

#[derive(Default)]
pub struct Bridge {
    pub listener: Option<Listener>,
    pending: Vec<Pending>,
    last_controls: Option<Instant>,
    next_token: u64,
}
impl Bridge {
    pub fn new(listener: Option<Listener>) -> Self {
        Self {
            listener,
            ..Self::default()
        }
    }
    pub fn controls_active(&self) -> bool {
        self.last_controls
            .is_some_and(|t| t.elapsed() < Duration::from_secs(3))
    }
    /// Called at the start of every frame. Neither waits nor captures block the frame loop.
    pub fn update(&mut self, ctx: &egui::Context, session: &mut Session) {
        self.complete(ctx, session);
        for _ in 0..16 {
            let Some(incoming) = self.listener.as_ref().and_then(|l| l.try_recv().ok()) else {
                break;
            };
            let request = match incoming.line {
                Ok(line) => bridge::parse(&line),
                Err(error) => Err(bridge::failure(Value::Null, error)),
            };
            let request = match request {
                Ok(request) => request,
                Err(response) => {
                    let _ = incoming.reply.send(response);
                    continue;
                }
            };
            let result = session.execute(&request.command, request.input);
            let interrupt_waits =
                result.is_ok() && matches!(request.command.as_str(), "sim.reset" | "sim.pause");
            if result.is_ok()
                && matches!(
                    request.command.as_str(),
                    "sim.set_controls"
                        | "autopilot.set"
                        | "autopilot.off"
                        | "assist.set"
                        | "assist.off"
                        | "avionics.set"
                        | "sim.reset"
                )
            {
                self.last_controls = Some(Instant::now());
            }
            match result {
                Ok(output) => {
                    let _ = incoming.reply.send(bridge::success(request.id, output));
                }
                Err(CommandError::AppPending(work)) => {
                    if self.pending.len() >= 16 {
                        let _ = incoming.reply.send(bridge::failure(
                            request.id,
                            "app is busy; too many pending requests",
                        ));
                        continue;
                    }
                    let work = match work {
                        AppRequest::Wait { seconds } => {
                            let start = session.sim().state().time_s;
                            Work::Wait {
                                start,
                                target: start + seconds,
                                generation: session.flight_generation(),
                            }
                        }
                        request @ (AppRequest::Location | AppRequest::Geocode(_)) => {
                            let (tx, rx) = mpsc::channel();
                            let context = ctx.clone();
                            std::thread::spawn(move || {
                                let _ = tx.send(
                                    flightcraft_control::run_navigation_request(request)
                                        .map_err(|e| e.to_string()),
                                );
                                context.request_repaint();
                            });
                            Work::Navigation(rx)
                        }
                        AppRequest::Screenshot(options) => {
                            self.next_token = self.next_token.wrapping_add(1);
                            let token = self.next_token;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                                token,
                            )));
                            Work::Screenshot { options, token }
                        }
                    };
                    self.pending.push(Pending {
                        id: request.id,
                        reply: incoming.reply,
                        deadline: Instant::now() + Duration::from_secs(120),
                        work,
                    });
                }
                Err(error) => {
                    let _ = incoming
                        .reply
                        .send(bridge::failure(request.id, error.to_string()));
                }
            }
            // A reset invalidates outstanding waits even if time was reset to the same value.
            if interrupt_waits {
                self.cancel_waits("sim.wait interrupted by reset or pause");
            }
        }
    }
    fn cancel_waits(&mut self, message: &str) {
        self.pending.retain(|p| {
            if matches!(p.work, Work::Wait { .. }) {
                let _ = p.reply.send(bridge::failure(p.id.clone(), message));
                false
            } else {
                true
            }
        });
    }
    fn complete(&mut self, ctx: &egui::Context, session: &mut Session) {
        self.pending.retain(|pending| {
            let time = session.sim().state().time_s;
            let result = if Instant::now() >= pending.deadline {
                Some(Err("app request timed out after 120 seconds".into()))
            } else {
                match &pending.work {
                    Work::Wait { .. } if session.sim().readout().crashed => {
                        Some(Err("sim.wait interrupted by a crash".into()))
                    }
                    Work::Wait {
                        start, generation, ..
                    } if time < *start || *generation != session.flight_generation() || session.paused() => {
                        Some(Err("sim.wait interrupted by reset or pause".into()))
                    }
                    Work::Wait { target, .. } if time >= *target => {
                        Some(session.execute("sim.state", json!({})).map_err(|e| e.to_string()))
                    }
                    Work::Wait { .. } => None,
                    Work::Navigation(rx) => match rx.try_recv() {
                        Ok(result) => Some(result),
                        Err(mpsc::TryRecvError::Empty) => None,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            Some(Err("navigation request interrupted".into()))
                        }
                    },
                    Work::Screenshot { options, token } => {
                        let image = ctx.input(|i| {
                            i.events.iter().find_map(|event| {
                                if let egui::Event::Screenshot { user_data, image, .. } = event
                                    && user_data.data.as_ref().and_then(|d| d.downcast_ref::<u64>())
                                        == Some(token)
                                {
                                    Some(image.clone())
                                } else {
                                    None
                                }
                            })
                        });
                        image.map(|image| {
                            save_png(&image, options)
                                .and_then(|out| serde_json::to_value(out).map_err(|e| e.to_string()))
                        })
                    }
                }
            };
            if let Some(result) = result {
                let response = match result {
                    Ok(output) => bridge::success(pending.id.clone(), output),
                    Err(error) => bridge::failure(pending.id.clone(), error),
                };
                let _ = pending.reply.send(response);
                false
            } else {
                true
            }
        });
    }
}

fn save_png(image: &egui::ColorImage, options: &AppScreenshot) -> Result<ScreenshotOutput, String> {
    let pixels: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
    let width = u32::try_from(image.width()).map_err(|e| e.to_string())?;
    let height = u32::try_from(image.height()).map_err(|e| e.to_string())?;
    let mut png = Cursor::new(Vec::new());
    image::write_buffer_with_format(
        &mut png,
        &pixels,
        width,
        height,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .map_err(|e| e.to_string())?;
    let bytes = png.into_inner();
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("screenshot exceeds 16 MiB".into());
    }
    let path = options
        .directory
        .as_ref()
        .map(|directory| write_new_png(directory, &bytes))
        .transpose()?;
    Ok(ScreenshotOutput {
        path,
        png_base64: options
            .base64
            .then(|| base64::engine::general_purpose::STANDARD.encode(&bytes)),
    })
}
fn write_new_png(directory: &str, bytes: &[u8]) -> Result<String, String> {
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let directory: PathBuf = std::fs::canonicalize(directory).map_err(|e| e.to_string())?;
    // Exclusive creation never overwrites a caller's existing file.
    for sequence in 0..10_000 {
        let path = directory.join(format!("flightcraft-{}-{sequence}.png", std::process::id()));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                if let Err(error) = file.write_all(bytes) {
                    let _ = std::fs::remove_file(&path);
                    return Err(error.to_string());
                }
                return Ok(path.to_string_lossy().into_owned());
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("screenshot directory has too many captures".into())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    #[test]
    fn waits_complete_only_after_simulated_time_advances_and_pause_interrupts() {
        let mut session = Session::new().unwrap();
        session.enable_app();
        session.execute("sim.resume", json!({})).unwrap();
        let mut bridge = Bridge::default();
        let ctx = egui::Context::default();
        let (reply, receive) = mpsc::channel();
        bridge.pending.push(Pending {
            id: json!(1),
            reply,
            deadline: Instant::now() + Duration::from_secs(10),
            work: Work::Wait {
                start: 0.0,
                target: 1.0,
                generation: session.flight_generation(),
            },
        });
        bridge.complete(&ctx, &mut session);
        assert!(receive.try_recv().is_err());
        session.run(flightcraft_control::Step { steps: 120 }).unwrap();
        bridge.complete(&ctx, &mut session);
        assert_eq!(receive.try_recv().unwrap()["ok"]["state"]["time_s"], 1.0);
        let (reply, receive) = mpsc::channel();
        bridge.pending.push(Pending {
            id: json!(2),
            reply,
            deadline: Instant::now() + Duration::from_secs(10),
            work: Work::Wait {
                start: 1.0,
                target: 2.0,
                generation: session.flight_generation(),
            },
        });
        session.execute("sim.pause", json!({})).unwrap();
        bridge.complete(&ctx, &mut session);
        assert!(
            receive.try_recv().unwrap()["error"]
                .as_str()
                .unwrap()
                .contains("pause")
        );
    }

    #[test]
    fn reset_to_the_same_time_interrupts_waits() {
        let mut session = Session::new().unwrap();
        session.enable_app();
        session.execute("sim.resume", json!({})).unwrap();
        let mut bridge = Bridge::default();
        let (reply, receive) = mpsc::channel();
        bridge.pending.push(Pending {
            id: json!(1),
            reply,
            deadline: Instant::now() + Duration::from_secs(10),
            work: Work::Wait {
                start: 0.0,
                target: 1.0,
                generation: session.flight_generation(),
            },
        });
        session
            .execute("sim.reset", json!({"start":{"kind":"runway"},"paused":false}))
            .unwrap();
        bridge.complete(&egui::Context::default(), &mut session);
        assert!(
            receive.try_recv().unwrap()["error"]
                .as_str()
                .unwrap()
                .contains("reset")
        );
    }

    #[test]
    fn screenshot_uses_matching_renderer_event_and_saves_png_without_overwriting() {
        let image = egui::ColorImage::filled([2, 2], egui::Color32::from_rgb(32, 64, 128));
        let directory = std::env::temp_dir().join(format!("flightcraft-capture-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let options = AppScreenshot {
            directory: Some(directory.to_string_lossy().into_owned()),
            base64: true,
        };
        let mut bridge = Bridge::default();
        let (reply, receive) = mpsc::channel();
        bridge.pending.push(Pending {
            id: json!(7),
            reply,
            deadline: Instant::now() + Duration::from_secs(10),
            work: Work::Screenshot {
                options: options.clone(),
                token: 42,
            },
        });
        let ctx = egui::Context::default();
        let mut session = Session::new().unwrap();
        let raw = egui::RawInput {
            events: vec![egui::Event::Screenshot {
                viewport_id: egui::ViewportId::ROOT,
                user_data: egui::UserData::new(41_u64),
                image: std::sync::Arc::new(image.clone()),
            }],
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| bridge.complete(ctx, &mut session));
        assert!(receive.try_recv().is_err());
        let raw = egui::RawInput {
            events: vec![egui::Event::Screenshot {
                viewport_id: egui::ViewportId::ROOT,
                user_data: egui::UserData::new(42_u64),
                image: std::sync::Arc::new(image.clone()),
            }],
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| bridge.complete(ctx, &mut session));
        let result = receive.try_recv().unwrap();
        let path = result["ok"]["path"].as_str().unwrap();
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(result["ok"]["png_base64"].as_str().unwrap())
                .unwrap(),
            bytes
        );
        assert_eq!(image::load_from_memory(&bytes).unwrap().width(), 2);
        let second = save_png(&image, &options).unwrap();
        assert_ne!(Some(path), second.path.as_deref());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn external_controls_expire_after_three_seconds() {
        let mut bridge = Bridge::default();
        assert!(!bridge.controls_active());
        bridge.last_controls = Some(Instant::now());
        assert!(bridge.controls_active());
        bridge.last_controls = Some(Instant::now() - Duration::from_secs(4));
        assert!(!bridge.controls_active());
    }
}
