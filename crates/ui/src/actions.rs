//! Every change the UI makes to the simulation goes through a `control` command here, the same
//! path MCP and scripts use.
use flightcraft_control::{CommandError, Pause, Reset, ResetStart, Resume, Session, SetControls, Step};
use flightcraft_sim::Controls;

/// Select a built-in through the same load and reset commands used by scripts.
pub fn select_aircraft(session: &mut Session, id: &str) -> Result<(), CommandError> {
    session.execute(
        "aircraft.load",
        serde_json::json!({
            "source": {"kind": "built_in", "id": id}
        }),
    )?;
    reset(session)
}

pub fn aircraft_choices(session: &mut Session) -> Result<flightcraft_control::AircraftList, CommandError> {
    session.run(flightcraft_control::ListAircraft {})
}

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

/// Camera changes use the same validated command as remote clients and scripts.
pub fn toggle_camera(session: &mut Session) -> Result<(), CommandError> {
    use flightcraft_control::CameraMode;
    let current = session
        .camera()
        .ok_or_else(|| CommandError::Failed("camera needs the app".into()))?;
    let mode = match current.mode {
        CameraMode::Chase => "cockpit",
        CameraMode::Cockpit => "chase",
    };
    session.execute("view.set_camera", serde_json::json!({"mode": mode}))?;
    Ok(())
}

/// A glance is a temporary camera command, using the same angular limits as scripts.
pub fn cockpit_look(session: &mut Session, yaw: f64, pitch: f64) -> Result<(), CommandError> {
    let current = session
        .camera()
        .ok_or_else(|| CommandError::Failed("camera needs the app".into()))?;
    let yaw = yaw.clamp(-2.1, 2.1);
    let pitch = pitch.clamp(-1.4, 1.4);
    if current.look_yaw_rad != yaw || current.look_pitch_rad != pitch {
        session.execute(
            "view.set_camera",
            serde_json::json!({
                "mode": current.mode, "look_yaw_rad": yaw, "look_pitch_rad": pitch,
            }),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picker_loads_resets_and_preserves_weather_and_rate() {
        let mut session = Session::new().unwrap();
        session
            .execute("sim.set_rate", serde_json::json!({"rate":2.0}))
            .unwrap();
        let weather = session.sim().weather();
        let generation = session.flight_generation();
        select_aircraft(&mut session, "larrow").unwrap();
        assert_eq!(session.sim().aircraft().identity.id, "larrow");
        assert_eq!(session.sim().state().time_s, 0.0);
        assert_eq!(session.flight_generation(), generation + 1);
        assert_eq!(session.sim().weather(), weather);
        assert_eq!(session.rate(), 2.0);
        assert!(!session.paused());
        let before = session.sim().state_hash();
        assert!(select_aircraft(&mut session, "missing").is_err());
        assert_eq!(session.sim().state_hash(), before);
        select_aircraft(&mut session, "corvid").unwrap();
        assert_eq!(session.sim().aircraft().identity.id, "corvid");
    }
}
