//! View preferences for the main window: the ground layout (a checklist on one side, a small
//! outside view on the other, the whole dash below) and the exterior inset that follows the
//! controls the pilot moves. `view.layout` and `view.exterior_inset` choose them.
use crate::*;
use flightcraft_aero::exterior_framing::FramingTarget;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Ground speed above which the aircraft counts as taking off (m/s).
const TAKEOFF_GROUND_SPEED_MPS: f64 = 12.;
/// Ground speed above which the aircraft is taxiing and the rudder steers the nosewheel (m/s).
const TAXI_SPEED_MPS: f64 = 0.5;
/// Seconds the inset keeps a part framed after the pilot last moved its control.
pub const DEFAULT_FRAMING_HOLD_S: f64 = 4.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LayoutMode {
    /// Ground layout while a checklist is open and the aircraft is on the ground.
    #[default]
    Auto,
    /// Ground layout now, until takeoff.
    Ground,
    /// The normal full-window outside view with the dash below.
    Normal,
}
/// When the small exterior view of the aircraft shows beside the cockpit view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InsetMode {
    Off,
    /// On the ground only (default): flaps down, canopy, brakes and the walkaround show.
    #[default]
    Ground,
    Always,
}
/// Where the inset sits over the outside view, as fractions of that view (never over the dash).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InsetPlacement {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
impl Default for InsetPlacement {
    fn default() -> Self {
        Self {
            x: 0.66,
            y: 0.04,
            w: 0.32,
            h: 0.46,
        }
    }
}
impl InsetPlacement {
    /// Fits the box inside the outside view, between 15% and 60% of it in each direction.
    #[must_use]
    pub fn clamped(self) -> Self {
        let f = |v: f32, d: f32| if v.is_finite() { v } else { d };
        let d = Self::default();
        let w = f(self.w, d.w).clamp(0.15, 0.6);
        let h = f(self.h, d.h).clamp(0.15, 0.6);
        Self {
            x: f(self.x, d.x).clamp(0., 1. - w),
            y: f(self.y, d.y).clamp(0., 1. - h),
            w,
            h,
        }
    }
}
/// The view choices that persist with the other preferences.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewPreferences {
    #[serde(default)]
    pub layout: LayoutMode,
    #[serde(default)]
    pub exterior_inset: InsetMode,
    #[serde(default)]
    pub placement: InsetPlacement,
    /// Seconds a moved part stays framed.
    #[serde(default = "default_hold")]
    pub framing_hold_s: f64,
}
fn default_hold() -> f64 {
    DEFAULT_FRAMING_HOLD_S
}
impl Default for ViewPreferences {
    fn default() -> Self {
        Self {
            layout: LayoutMode::Auto,
            exterior_inset: InsetMode::Ground,
            placement: InsetPlacement::default(),
            framing_hold_s: DEFAULT_FRAMING_HOLD_S,
        }
    }
}
impl ViewPreferences {
    pub fn validate(&self) -> Result<(), String> {
        if !self.framing_hold_s.is_finite() || !(0.5..=30.).contains(&self.framing_hold_s) {
            return Err("framing_hold_s must be between 0.5 and 30 seconds".into());
        }
        let p = self.placement;
        if [p.x, p.y, p.w, p.h].iter().any(|v| !v.is_finite()) {
            return Err("inset placement must be finite".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ControlSnapshot {
    flaps: f64,
    aileron: f64,
    elevator: f64,
    rudder: f64,
    trim: f64,
    brake: f64,
    canopy_open: f64,
    canopy_latched: bool,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct GroundLayout {
    pub prefs: ViewPreferences,
    last: Option<ControlSnapshot>,
    target: FramingTarget,
    until_s: f64,
}

impl Session {
    /// Whether the main window should show the ground layout.
    #[must_use]
    pub fn ground_layout_active(&self) -> bool {
        self.view_layout_state().ground
    }
    #[must_use]
    pub fn view_preferences(&self) -> &ViewPreferences {
        &self.ground_layout.prefs
    }
    pub fn restore_view_preferences(&mut self, prefs: ViewPreferences) -> Result<(), CommandError> {
        prefs.validate().map_err(|m| invalid("view.layout", m))?;
        self.ground_layout.prefs = ViewPreferences {
            placement: prefs.placement.clamped(),
            ..prefs
        };
        Ok(())
    }
    #[must_use]
    pub fn view_layout_state(&self) -> ViewLayoutState {
        let mode = self.ground_layout.prefs.layout;
        let (ground, reason) = if mode == LayoutMode::Normal {
            (false, "normal layout chosen")
        } else {
            let r = self.sim.readout();
            let moving = {
                let v = self.sim.state().velocity_mps;
                libm::hypot(v[0], v[1]) > TAKEOFF_GROUND_SPEED_MPS
            };
            if !r.on_ground || moving {
                (false, "airborne or taking off: normal layout")
            } else if mode == LayoutMode::Ground {
                (true, "ground layout chosen")
            } else if self.checklist.is_open() {
                (true, "a checklist is open on the ground")
            } else {
                (false, "no checklist is open")
            }
        };
        ViewLayoutState {
            mode,
            ground,
            reason: reason.into(),
            exterior_inset: self.exterior_inset_state(),
        }
    }

    /// The inset's state: whether it shows, and which part it is framing.
    #[must_use]
    pub fn exterior_inset_state(&self) -> ExteriorInsetState {
        let prefs = self.ground_layout.prefs;
        let on_ground = self.sim.readout().on_ground;
        let visible = match prefs.exterior_inset {
            InsetMode::Off => false,
            InsetMode::Ground => on_ground,
            InsetMode::Always => true,
        };
        let now = self.sim.state().time_s;
        let remaining = (self.ground_layout.until_s - now).max(0.);
        ExteriorInsetState {
            mode: prefs.exterior_inset,
            visible,
            target: if remaining > 0. {
                self.ground_layout.target
            } else {
                FramingTarget::Default
            },
            remaining_s: remaining,
            hold_s: prefs.framing_hold_s,
            placement: prefs.placement,
        }
    }

    /// Watches the controls each fixed step and frames the part the pilot just moved.
    pub(crate) fn sequence_inset(&mut self) {
        let now = self.sim.state().time_s;
        let c = self.sim.controls();
        let sys = self.sim.systems();
        let snap = ControlSnapshot {
            flaps: c.flaps,
            aileron: c.aileron,
            elevator: c.elevator,
            rudder: c.rudder,
            trim: sys.controls.trim,
            brake: c.brake,
            canopy_open: sys.canopy.open_fraction,
            canopy_latched: sys.canopy.latched,
        };
        let Some(last) = self.ground_layout.last.replace(snap) else {
            return;
        };
        let hold = self.ground_layout.prefs.framing_hold_s;
        let mut target = None;
        if (snap.flaps - last.flaps).abs() > 0.01 {
            target = Some(FramingTarget::Flaps);
        }
        if (snap.aileron - last.aileron).abs() > 0.04 {
            target = Some(FramingTarget::Ailerons);
        }
        if (snap.elevator - last.elevator).abs() > 0.04 || (snap.trim - last.trim).abs() > 0.004 {
            target = Some(FramingTarget::Tail);
        }
        if (snap.rudder - last.rudder).abs() > 0.04 {
            // On the ground, rudder pedals steer the nosewheel: look at the wheels.
            let v = self.sim.state().velocity_mps;
            let taxiing = libm::hypot(v[0], v[1]) > TAXI_SPEED_MPS && self.sim.readout().on_ground;
            target = Some(if taxiing {
                FramingTarget::Wheels
            } else {
                FramingTarget::Tail
            });
        }
        if (snap.brake - last.brake).abs() > 0.1 && self.sim.readout().on_ground {
            target = Some(FramingTarget::Wheels);
        }
        if (snap.canopy_open - last.canopy_open).abs() > 0.005 || snap.canopy_latched != last.canopy_latched {
            target = Some(FramingTarget::Cockpit);
        }
        if let Some(t) = target {
            self.ground_layout.target = t;
            self.ground_layout.until_s = now + hold;
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExteriorInsetState {
    pub mode: InsetMode,
    /// Whether the inset shows over the cockpit view right now.
    pub visible: bool,
    /// The part being framed; `default` is the three-quarter rear chase.
    pub target: FramingTarget,
    pub remaining_s: f64,
    pub hold_s: f64,
    pub placement: InsetPlacement,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ViewLayoutState {
    pub mode: LayoutMode,
    /// Whether the window shows the ground layout right now.
    pub ground: bool,
    pub reason: String,
    pub exterior_inset: ExteriorInsetState,
}

#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetViewLayout {
    /// Omitted reads the current layout.
    #[serde(default)]
    pub mode: Option<LayoutMode>,
}
impl Command for SetViewLayout {
    const NAME: &'static str = "view.layout";
    const DESCRIPTION: &'static str = "Choose the main window layout: auto (checklist beside a small outside view while a checklist is open on the ground), ground, or normal. Takeoff returns to normal. Also reports the exterior inset.";
    type Output = ViewLayoutState;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        if let Some(mode) = self.mode {
            s.ground_layout.prefs.layout = mode;
        }
        Ok(s.view_layout_state())
    }
}

#[derive(Debug, Deserialize, JsonSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetExteriorInset {
    /// Off, on the ground only (default), or always.
    #[serde(default)]
    pub mode: Option<InsetMode>,
    /// Seconds a moved part stays framed, 0.5 to 30.
    #[serde(default)]
    pub hold_s: Option<f64>,
    /// Where the inset sits over the outside view.
    #[serde(default)]
    pub placement: Option<InsetPlacement>,
}
impl Command for SetExteriorInset {
    const NAME: &'static str = "view.exterior_inset";
    const DESCRIPTION: &'static str = "Choose when the small exterior view shows beside the cockpit view (off, on the ground, always), how long a moved control surface stays framed, and where the box sits. Omitted fields are unchanged.";
    type Output = ExteriorInsetState;
    fn run(self, s: &mut Session) -> Result<Self::Output, CommandError> {
        let mut prefs = s.ground_layout.prefs;
        if let Some(mode) = self.mode {
            prefs.exterior_inset = mode;
        }
        if let Some(hold) = self.hold_s {
            prefs.framing_hold_s = hold;
        }
        if let Some(p) = self.placement {
            prefs.placement = p;
        }
        s.restore_view_preferences(prefs).map_err(|_| {
            invalid(
                Self::NAME,
                "hold_s must be between 0.5 and 30 and the placement finite",
            )
        })?;
        Ok(s.exterior_inset_state())
    }
}
