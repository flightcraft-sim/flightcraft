//! App command contracts. Platform IO and deferred work belong to the UI thread.
use crate::{Command, CommandError, Session, StateOutput, invalid, range};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CameraMode {
    #[default]
    Chase,
    /// Placeholder view from the aircraft; no cockpit panel geometry yet.
    Cockpit,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, JsonSchema)]
pub struct CameraSettings {
    pub mode: CameraMode,
    pub distance_m: f64,
    pub height_m: f64,
}

impl Default for CameraSettings {
    fn default() -> Self {
        Self {
            mode: CameraMode::Chase,
            distance_m: 19.0,
            height_m: 7.0,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetCamera {
    pub mode: CameraMode,
    #[serde(default, deserialize_with = "crate::provided")]
    #[schemars(with = "f64", range(min = 2, max = 200), transform = crate::without_default)]
    pub distance_m: Option<f64>,
    #[serde(default, deserialize_with = "crate::provided")]
    #[schemars(with = "f64", range(min = 0, max = 100), transform = crate::without_default)]
    pub height_m: Option<f64>,
}

impl Command for SetCamera {
    const NAME: &'static str = "view.set_camera";
    const DESCRIPTION: &'static str =
        "Set chase or cockpit view and chase distance/height in metres (needs the app).";
    type Output = CameraSettings;
    fn run(self, session: &mut Session) -> Result<Self::Output, CommandError> {
        if let Some(v) = self.distance_m {
            range(Self::NAME, "distance_m", v, 2.0, 200.0)?;
        }
        if let Some(v) = self.height_m {
            range(Self::NAME, "height_m", v, 0.0, 100.0)?;
        }
        let camera = session.camera.as_mut().ok_or_else(needs_app)?;
        *camera = CameraSettings {
            mode: self.mode,
            distance_m: self.distance_m.unwrap_or(camera.distance_m),
            height_m: self.height_m.unwrap_or(camera.height_m),
        };
        Ok(*camera)
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppScreenshot {
    /// Directory for a newly created PNG. Required unless base64 is true.
    #[serde(default, deserialize_with = "crate::provided")]
    #[schemars(with = "String", transform = crate::without_default)]
    pub directory: Option<String>,
    #[serde(default)]
    pub base64: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ScreenshotOutput {
    pub path: Option<String>,
    pub png_base64: Option<String>,
}

impl Command for AppScreenshot {
    const NAME: &'static str = "app.screenshot";
    const DESCRIPTION: &'static str =
        "Capture the app window over the next frames; return a PNG path and/or base64 (needs the app).";
    type Output = ScreenshotOutput;
    fn run(self, session: &mut Session) -> Result<Self::Output, CommandError> {
        if self
            .directory
            .as_ref()
            .is_some_and(|d| d.is_empty() || d.len() > 4096 || d.contains('\0'))
        {
            return Err(invalid(
                Self::NAME,
                "directory must be a nonempty path of at most 4096 bytes",
            ));
        }
        if self.directory.is_none() && !self.base64 {
            return Err(invalid(Self::NAME, "supply directory or request base64"));
        }
        session.camera.ok_or_else(needs_app)?;
        Err(CommandError::AppPending(AppRequest::Screenshot(self)))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Wait {
    #[schemars(range(min = 0, max = 60))]
    pub seconds: f64,
}

impl Command for Wait {
    const NAME: &'static str = "sim.wait";
    const DESCRIPTION: &'static str =
        "Wait for 0 to 60 simulated seconds of app real-time flight; fails while paused (needs the app).";
    type Output = StateOutput;
    fn run(self, session: &mut Session) -> Result<Self::Output, CommandError> {
        range(Self::NAME, "seconds", self.seconds, 0.0, 60.0)?;
        session.camera.ok_or_else(needs_app)?;
        if session.sim().readout().crashed {
            return Err(CommandError::Failed(
                "sim.wait cannot advance a crashed flight; reset first".into(),
            ));
        }
        if session.paused() {
            return Err(CommandError::Failed(
                "sim.wait needs real-time flight; use sim.resume first, or sim.step while paused".into(),
            ));
        }
        Err(CommandError::AppPending(AppRequest::Wait {
            seconds: self.seconds,
        }))
    }
}

/// Validated work for the UI to complete without blocking the frame loop.
#[derive(Clone, Debug, PartialEq)]
pub enum AppRequest {
    Screenshot(AppScreenshot),
    Wait { seconds: f64 },
}

fn needs_app() -> CommandError {
    CommandError::Failed("this command needs the app; connect to a desktop control port".into())
}
