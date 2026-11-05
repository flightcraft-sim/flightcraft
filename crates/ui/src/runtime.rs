//! The native clock owns the session. Windows consume copies and route commands to that owner.
use crate::{bridge::Bridge, clock::FixedStep, script::Script};
use flightcraft_control::{Session, Step, host::SessionHost};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub struct Runtime {
    latest: Arc<Mutex<Session>>,
    host: SessionHost,
    stop: Arc<AtomicBool>,
    external: Arc<AtomicBool>,
    script_active: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Runtime {
    pub fn start(
        mut session: Session,
        mut script: Script,
        mut bridge: Bridge,
        ctx: eframe::egui::Context,
    ) -> Result<Self, std::io::Error> {
        let (host, jobs) = SessionHost::channel();
        let latest = Arc::new(Mutex::new(session.presentation_copy()));
        let stop = Arc::new(AtomicBool::new(false));
        let external = Arc::new(AtomicBool::new(false));
        let script_active = Arc::new(AtomicBool::new(script.active()));
        let error = Arc::new(Mutex::new(None));
        let (output, shutdown, active, scripted, errors) = (
            latest.clone(),
            stop.clone(),
            external.clone(),
            script_active.clone(),
            error.clone(),
        );
        let thread = std::thread::Builder::new()
            .name("flight-session".into())
            .spawn(move || {
                let mut clock = FixedStep::default();
                let mut previous = Instant::now();
                let mut published = previous;
                let mut last_external: Option<Instant> = None;
                while !shutdown.load(Ordering::Relaxed) {
                    let now = Instant::now();
                    let elapsed = now.duration_since(previous).as_secs_f64();
                    previous = now;
                    let generation = session.flight_generation();
                    for job in jobs.try_iter().take(64) {
                        if job.external_controls {
                            last_external = Some(Instant::now());
                        }
                        (job.execute)(&mut session);
                    }
                    bridge.update(&ctx, &mut session);
                    if session.flight_generation() != generation {
                        clock.reset();
                    }
                    active.store(
                        bridge.controls_active()
                            || last_external.is_some_and(|t| t.elapsed() < Duration::from_secs(3)),
                        Ordering::Relaxed,
                    );
                    // The walkaround is a ground activity: it animates in real time while paused.
                    session.advance_walkaround_paused(elapsed);
                    let steps = clock.advance(elapsed * session.rate(), session.paused());
                    for _ in 0..steps {
                        let time = session.sim().state().time_s;
                        let result = script.run_due(&mut session, time).and_then(|()| {
                            if session.paused() {
                                return Ok(());
                            }
                            session
                                .run(Step { steps: 1 })
                                .map(|_| ())
                                .map_err(|e| e.to_string())
                        });
                        if let Err(e) = result {
                            if let Ok(mut message) = errors.lock() {
                                *message = Some(e);
                            }
                            let _ = session.execute("sim.pause", serde_json::json!({}));
                            break;
                        }
                        if session.paused() {
                            clock.reset();
                            break;
                        }
                    }
                    scripted.store(script.active(), Ordering::Relaxed);
                    session.update_efb();
                    if now.duration_since(published) >= Duration::from_secs_f64(1.0 / 60.0) {
                        if let Ok(mut snapshot) = output.lock() {
                            *snapshot = session.presentation_copy();
                        }
                        published = now;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            })?;
        Ok(Self {
            latest,
            host,
            stop,
            external,
            script_active,
            error,
            thread: Some(thread),
        })
    }
    pub fn snapshot(&self) -> Result<Session, String> {
        let mut snapshot = self
            .latest
            .lock()
            .map_err(|_| "session snapshot unavailable")?
            .presentation_copy();
        snapshot.attach_host(self.host.clone());
        Ok(snapshot)
    }
    pub fn external_active(&self) -> bool {
        self.external.load(Ordering::Relaxed)
    }
    pub fn script_active(&self) -> bool {
        self.script_active.load(Ordering::Relaxed)
    }
    pub fn error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|mut e| e.take())
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn crew_commands_and_control_ownership_live_on_the_session_thread() {
        use flightcraft_control::{ControlOwner, CrewMode};
        let mut session = Session::new().unwrap();
        session.enable_app();
        let runtime = Runtime::start(
            session,
            Script::default(),
            Bridge::default(),
            eframe::egui::Context::default(),
        )
        .unwrap();
        let mut screen = runtime.snapshot().unwrap();
        screen
            .execute(
                "crew.mode",
                serde_json::json!({"mode":"watching","director":true}),
            )
            .unwrap();
        screen
            .execute(
                "crew.intent",
                serde_json::json!({"intent":{"kind":"take_off","airport":"default","runway":"36"}}),
            )
            .unwrap();
        screen
            .execute("crew.handover", serde_json::json!({"to":"crew","actor":"crew"}))
            .unwrap();
        screen
            .execute(
                "crew.handover",
                serde_json::json!({"to":"crew","actor":"pilot","confirm":true}),
            )
            .unwrap();
        assert_eq!(screen.crew_state().controls, ControlOwner::Crew);
        // A second window reads owner state, including changes made from the first window.
        let state = runtime
            .snapshot()
            .unwrap()
            .execute("crew.state", serde_json::json!({}))
            .unwrap();
        assert_eq!(state.get("mode").unwrap(), "watching");
        assert_eq!(state.get("controls").unwrap(), "crew");
        assert!(
            screen
                .execute("sim.set_controls", serde_json::json!({"throttle":1.}))
                .is_err()
        );
        screen.execute("sim.resume", serde_json::json!({})).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        screen.execute("sim.pause", serde_json::json!({})).unwrap();
        assert!(screen.sim().state().time_s > 0.);
        assert_eq!(screen.crew_state().mode, CrewMode::Watching);
        // Browser/remote snapshots round-trip the same crew state without running a second pilot.
        let snapshot =
            serde_json::from_value(serde_json::to_value(screen.screen_snapshot()).unwrap()).unwrap();
        let mut remote = Session::new().unwrap();
        remote.apply_screen_snapshot(snapshot).unwrap();
        assert_eq!(remote.crew_state().controls, ControlOwner::Crew);
        let time = remote.sim().state().time_s;
        remote
            .execute("crew.mode", serde_json::json!({"mode":"demonstrating"}))
            .unwrap();
        assert_eq!(remote.take_screen_commands().first().unwrap().0, "crew.mode");
        assert_eq!(remote.sim().state().time_s, time);
    }
    #[test]
    fn runs_without_any_window_repaints_and_pause_keeps_fixed_steps() {
        let mut s = Session::new().unwrap();
        s.enable_app();
        s.execute("sim.resume", serde_json::json!({})).unwrap();
        let runtime = Runtime::start(
            s,
            Script::default(),
            Bridge::default(),
            eframe::egui::Context::default(),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(280));
        let mut s = runtime.snapshot().unwrap();
        s.execute("sim.pause", serde_json::json!({})).unwrap();
        let t = s.sim().state().time_s;
        assert!((0.20..0.55).contains(&t), "time without repaint: {t}");
        assert!((t * 120. - (t * 120.).round()).abs() < 1e-8);
        let before = s.sim().state_hash();
        std::thread::sleep(Duration::from_millis(40));
        s.execute("sim.state", serde_json::json!({})).unwrap();
        assert_eq!(before, s.sim().state_hash());
        let mut direct = Session::new().unwrap();
        direct
            .run(Step {
                steps: (t * 120.).round() as u32,
            })
            .unwrap();
        assert_eq!(before, direct.sim().state_hash());
    }
}

#[derive(Clone)]
pub struct Source {
    latest: Arc<Mutex<Session>>,
    host: SessionHost,
}
impl Source {
    pub fn snapshot(&self) -> Result<Session, String> {
        let mut s = self
            .latest
            .lock()
            .map_err(|_| "session snapshot unavailable")?
            .presentation_copy();
        s.attach_host(self.host.clone());
        Ok(s)
    }
}
impl Runtime {
    pub fn source(&self) -> Source {
        Source {
            latest: self.latest.clone(),
            host: self.host.for_screen(),
        }
    }
}
