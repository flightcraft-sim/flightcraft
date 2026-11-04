//! Native session command routing. Presentations never step their copy of a flight.
use crate::{Command, CommandError, Session};
use std::sync::mpsc;
use std::time::Duration;

pub struct SessionJob {
    pub external_controls: bool,
    pub execute: Box<dyn FnOnce(&mut Session) + Send>,
}

#[derive(Clone)]
pub struct SessionHost(mpsc::SyncSender<SessionJob>, bool);
impl std::fmt::Debug for SessionHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionHost")
    }
}
impl SessionHost {
    pub fn channel() -> (Self, mpsc::Receiver<SessionJob>) {
        let (tx, rx) = mpsc::sync_channel(256);
        (Self(tx, false), rx)
    }
    pub fn for_screen(&self) -> Self {
        Self(self.0.clone(), true)
    }
    pub fn submit(&self, job: SessionJob) -> Result<(), CommandError> {
        self.0
            .try_send(job)
            .map_err(|_| CommandError::Failed("session command queue unavailable or full".into()))
    }
    pub(crate) fn run<C: Command>(
        &self,
        command: C,
    ) -> Result<(Result<C::Output, CommandError>, Session), CommandError> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.submit(SessionJob {
            external_controls: self.1 && matches!(C::NAME, "sim.set_controls" | "sim.reset"),
            execute: Box::new(move |s| {
                let result = s.run(command);
                let _ = tx.send((result, s.presentation_copy()));
            }),
        })?;
        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| CommandError::Failed("session command timed out".into()))
    }
}
impl Session {
    /// Immutable presentation data, without sockets or ownership of the simulation clock.
    /// Commands on an attached copy are routed to its owner and return an updated copy.
    pub fn presentation_copy(&self) -> Self {
        Self {
            live_weather: self.live_weather.clone(),
            host: None,
            presentation_commands: None,
            presentation_glass: self.presentation_glass.clone(),
            presentation_comms: self.presentation_comms.clone(),
            presentation_atc: self.presentation_atc.clone(),
            presentation_traffic: self.presentation_traffic.clone(),
            presentation_efb: Some(self.efb_status()),
            screens: self.screens.clone(),
            layout_undo: self.layout_undo.clone(),
            layout_redo: self.layout_redo.clone(),
            scenario: self.scenario.clone(),
            lesson: self.lesson.clone(),
            flight_archive: self.flight_archive.clone(),
            comms: self.comms.clone(),
            crew: self.crew.clone(),
            sim: self.sim.clone(),
            efb: None,
            navigation: self.navigation.clone(),
            map: self.map.clone(),
            glass: self.glass.clone(),
            touch: self.touch.clone(),
            display_prefs: self.display_prefs.clone(),
            selected_aircraft: self.selected_aircraft.clone(),
            aircraft_catalog: self.aircraft_catalog.clone(),
            selected_configuration: self.selected_configuration.clone(),
            active_configuration: self.active_configuration.clone(),
            paused: self.paused,
            rate: self.rate,
            camera: self.camera,
            airspace_visible: self.airspace_visible,
            flight_generation: self.flight_generation,
            regions: self.regions.clone(),
        }
    }
    pub fn attach_host(&mut self, host: SessionHost) {
        self.host = Some(host);
    }
}
