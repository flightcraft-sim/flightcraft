//! Keymap actions that change the flight session, shared by the desktop window and the browser.
//!
//! The desktop main window and the browser's screen windows read keys through different loops
//! (`show_main` and `screens::web`). Everything that only needs a `Session` lives here so both
//! apply the same keymap, and a test checks that every [`Action`] is accounted for.
use crate::input::ActionFrame;
use crate::instruments::{self, DisplayOptions};
use crate::keymap::Action;
use crate::{autopilot, glass};
use flightcraft_control::Session;

/// Where each keymap action is handled. Used to prove none is dropped on any front end.
#[cfg(test)]
#[must_use]
pub fn route(action: Action) -> Route {
    use Action::*;
    match action {
        ElevatorUp | ElevatorDown | AileronLeft | AileronRight | RudderLeft | RudderRight | ThrottleUp
        | ThrottleDown | Flaps | Brakes | TrimUp | TrimDown => Route::FlightControls,
        ParkBrake => Route::ParkBrake,
        Hood | Airspace => Route::Session,
        Master | Avionics | Alternator | FuelPump | Lights | PitotHeat | CarbHeat | AlternateStatic
        | Starter | Magnetos | FuelSelector | MixtureRich | MixtureLean | Primer | AltimeterUp
        | AltimeterDown | HeadingBugLeft | HeadingBugRight | AutopilotEngage | AutopilotDisconnect
        | ControlWheelSteering | AutopilotHeading | AutopilotAltitude | AutopilotLevel | HeadingBugSync
        | StandbyCourseLeft | StandbyCourseRight | StandbyAltitudeUp | StandbyAltitudeDown
        | StandbyAttitudePower | StandbyHsiPower | StandbyReversion | StandbyDirectTo | GlassRangeIn
        | GlassRangeOut | GlassPage | GlassOrientation | GlassAltitudeUp | GlassAltitudeDown
        | GlassCourseLeft | GlassCourseRight | GlassTraffic | GlassTrafficFilter | GlassIdent => {
            Route::Session
        }
        Pause | Reset | Camera | LookLeft | LookRight | LookUp | CrewHandover | ComPtt | IntercomPtt
        | RadioPanel | Kneeboard | RadioTransmit | PlanningMap | MapZoomIn | MapZoomOut | Search => {
            Route::Window
        }
    }
}

/// How a front end handles an action.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// Folded into the flight controls (`Input::controls`).
    FlightControls,
    /// The parking brake, toggled by [`toggle_park_brake`] while the pilot has the controls.
    ParkBrake,
    /// Handled by [`apply_session_actions`].
    Session,
    /// Handled by the window itself (pause, camera, radio, map); each front end wires these.
    Window,
}

/// Toggle the parking brake (the brake intercept valve).
pub fn toggle_park_brake(session: &mut Session, error: &mut Option<String>) {
    let on = !session.sim().systems().controls.park_brake;
    if let Err(e) = session.run(flightcraft_control::SetSystems {
        setting: flightcraft_sim::systems::SystemSetting::ParkBrake { on },
    }) {
        *error = Some(e.to_string());
    }
}

/// Apply the hood, airspace, avionics, glass, systems, and autopilot shortcuts pressed this frame.
pub fn apply_session_actions(
    session: &mut Session,
    frame: &ActionFrame,
    options: DisplayOptions,
    error: &mut Option<String>,
) {
    let mut fail = |e: String| *error = Some(e);
    if frame.pressed(Action::Hood) {
        let enabled = !session.sim().hood();
        if let Err(e) = session.run(flightcraft_control::SetHood { enabled }) {
            fail(e.to_string());
        }
    }
    if frame.pressed(Action::Airspace) {
        let visible = !session.airspace_visible();
        if let Err(e) = session.run(flightcraft_control::SetAirspaceView { visible }) {
            fail(e.to_string());
        }
    }
    let glass_active = session
        .aircraft_configuration()
        .is_some_and(|c| c.panel.instruments.iter().any(|i| i.module == "glass_pfd"));
    for action in [
        Action::HeadingBugLeft,
        Action::HeadingBugRight,
        Action::HeadingBugSync,
    ] {
        if frame.pressed(action) {
            let setting = match action {
                Action::HeadingBugSync => flightcraft_control::AvionicsSetting::HeadingBugSync {},
                _ => flightcraft_control::AvionicsSetting::HeadingBugStep {
                    detents: if action == Action::HeadingBugLeft { -1 } else { 1 },
                    interval_ms: None,
                },
            };
            if let Err(e) = session.run(flightcraft_control::SetAvionics { setting }) {
                fail(e.to_string());
            }
        }
    }
    for action in Action::AVIONICS {
        if matches!(
            action,
            Action::HeadingBugLeft | Action::HeadingBugRight | Action::HeadingBugSync
        ) {
            continue;
        }
        if frame.pressed(action)
            && let Some(setting) = instruments::avionics_shortcut_with_options(
                action,
                &session.sim().systems().avionics,
                options,
            )
            && let Err(e) = session.run(flightcraft_control::SetAvionics { setting })
        {
            fail(e.to_string());
        }
    }
    if glass_active {
        let state = session.glass_state();
        for action in Action::GLASS {
            if matches!(action, Action::HeadingBugLeft | Action::HeadingBugRight) {
                continue;
            }
            if frame.pressed(action)
                && let Some(setting) = glass::shortcut(action, &state)
                && let Err(e) = session.run(flightcraft_control::SetAvionics {
                    setting: flightcraft_control::AvionicsSetting::Glass { setting },
                })
            {
                fail(e.to_string());
            }
        }
    }
    for action in Action::SYSTEMS {
        if matches!(action, Action::HeadingBugLeft | Action::HeadingBugRight)
            || (session.sim().systems().avionics.installed
                && matches!(action, Action::AltimeterUp | Action::AltimeterDown))
        {
            continue;
        }
        if frame.pressed(action)
            && let Some(setting) =
                instruments::shortcut_with_options(action, session.sim().systems(), options)
            && let Err(e) = session.run(flightcraft_control::SetSystems { setting })
        {
            fail(e.to_string());
        }
    }
    if let Some(state) = session.sim().installed_autopilot().cloned() {
        for action in Action::AUTOPILOT {
            if frame.pressed(action)
                && let Some(setting) = autopilot::shortcut(action, &state)
                && let Err(e) = session.run(flightcraft_control::SetAvionics { setting })
            {
                fail(e.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pressed(action: Action) -> ActionFrame {
        let mut frame = ActionFrame::default();
        frame.pressed.insert(action);
        frame
    }

    #[test]
    fn every_keymap_action_has_a_route() {
        // `route` is an exhaustive match, so a new action fails to compile until it is routed.
        // The routed lists must also match what the front ends iterate.
        for action in Action::ALL {
            let _ = route(action);
        }
        for action in Action::SYSTEMS
            .into_iter()
            .chain(Action::AUTOPILOT)
            .chain(Action::AVIONICS)
            .chain(Action::GLASS)
        {
            assert_eq!(route(action), Route::Session, "{action:?}");
        }
        assert_eq!(route(Action::ParkBrake), Route::ParkBrake);
    }

    #[test]
    fn park_brake_toggles_the_valve_and_holds_the_brakes_on_the_ground() {
        let mut session = Session::new().unwrap();
        let mut error = None;
        assert!(!session.sim().systems().controls.park_brake);
        toggle_park_brake(&mut session, &mut error);
        assert!(session.sim().systems().controls.park_brake);
        assert_eq!(error, None);
        toggle_park_brake(&mut session, &mut error);
        assert!(!session.sim().systems().controls.park_brake);
    }

    #[test]
    fn session_actions_reach_the_session() {
        let mut session = Session::new().unwrap();
        let mut error = None;
        let options = DisplayOptions::default();
        assert!(!session.sim().hood());
        apply_session_actions(&mut session, &pressed(Action::Hood), options, &mut error);
        assert!(session.sim().hood());
        let visible = session.airspace_visible();
        apply_session_actions(&mut session, &pressed(Action::Airspace), options, &mut error);
        assert_ne!(session.airspace_visible(), visible);
        // A system shortcut: the master switch.
        let master = session.sim().systems().controls.master;
        apply_session_actions(&mut session, &pressed(Action::Master), options, &mut error);
        assert_ne!(session.sim().systems().controls.master, master);
        assert_eq!(error, None);
        // Nothing pressed, nothing changes.
        let before = session.sim().systems().controls.clone();
        apply_session_actions(&mut session, &ActionFrame::default(), options, &mut error);
        assert_eq!(session.sim().systems().controls, before);
    }
}
