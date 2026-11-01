//! Every change the UI makes to the simulation goes through a `control` command here, the same
//! path MCP and scripts use.
use flightcraft_control::{CommandError, Pause, Reset, ResetStart, Resume, Session, SetControls, Step};
use flightcraft_sim::Controls;

pub fn set_controls(session: &mut Session, controls: Controls) -> Result<(), CommandError> {
    let c = controls.clamped();
    session.run(SetControls {
        elevator: Some(c.elevator),
        aileron: Some(c.aileron),
        rudder: Some(c.rudder),
        throttle: Some(c.throttle),
        flaps: Some(c.flaps),
        brake: Some(c.brake),
    })?;
    Ok(())
}

pub fn step(session: &mut Session, steps: u32) -> Result<(), CommandError> {
    session.run(Step { steps })?;
    Ok(())
}

/// Back to the runway threshold, running.
pub fn reset(session: &mut Session) -> Result<(), CommandError> {
    session.run(Reset {
        start: ResetStart::Runway {},
        paused: Some(false),
        weather: Some(session.sim().weather()),
    })?;
    Ok(())
}

/// Pause or resume; `paused` mirrors the session afterwards.
pub fn toggle_pause(session: &mut Session, paused: &mut bool) -> Result<(), CommandError> {
    if session.paused() {
        session.run(Resume {})?;
    } else {
        session.run(Pause {})?;
    }
    *paused = session.paused();
    Ok(())
}

/// The camera is view state, not simulation state; it moves to a `view.set_camera` command when
/// the view gets commands (ROADMAP item 12).
pub fn toggle_camera(scene: &mut flightcraft_render::Scene) {
    scene.toggle_camera();
}
