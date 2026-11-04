//! What the renderer needs to draw a pre-flight walkaround: the figure, the props, hand-moved
//! control surfaces, and the fuel and oil close-ups. Plain data in body coordinates (x forward,
//! y right, z down, metres), produced by the session and consumed by the exterior renderer, so
//! neither depends on the other.
use crate::preflight::{DefectKind, PropKind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What the figure is doing right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FigureAction {
    #[default]
    Idle,
    Walking,
    /// Looking at a part from standing height.
    Looking,
    /// Crouched to see a wheel, brake, tyre, or the wing underside.
    Crouching,
    /// Hand on a surface to feel condition or security.
    Touching,
    /// Moving a control surface by hand.
    MovingControl,
    /// Taking a cover, rope, or chock off.
    Removing,
    /// Lifting a fuel filler cap and looking into the tank.
    CheckingFuel,
    /// Pulling the oil dipstick.
    CheckingOil,
    /// Palm on a propeller blade, turning it with ignition off.
    HandlingPropeller,
    /// Climbing into or out of the cockpit.
    Boarding,
    /// Seated in the cockpit working a control.
    InCockpit,
}

/// Pose parameters for the stylised figure. The renderer solves the limbs from these.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FigureState {
    /// Midpoint between the feet; z is the ground (or the seat) in body coordinates.
    pub position_m: [f32; 3],
    /// Direction the figure faces, radians from body +x towards +y.
    pub heading_rad: f32,
    /// Stride phase in radians; advances with distance walked.
    pub walk_phase: f32,
    pub walking: bool,
    /// 0 standing, 1 deep crouch.
    #[schemars(range(min = 0, max = 1))]
    pub crouch: f32,
    /// Forward lean of the torso in radians.
    pub lean_rad: f32,
    pub seated: bool,
    /// Where each hand is reaching, if it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_hand_m: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right_hand_m: Option<[f32; 3]>,
    /// Where the head is looking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gaze_m: Option<[f32; 3]>,
    /// Something carried in the hand, such as a cover or chock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carrying: Option<PropKind>,
    pub action: FigureAction,
}

impl Default for FigureState {
    fn default() -> Self {
        Self {
            position_m: [0.; 3],
            heading_rad: 0.,
            walk_phase: 0.,
            walking: false,
            crouch: 0.,
            lean_rad: 0.,
            seated: false,
            left_hand_m: None,
            right_hand_m: None,
            gaze_m: None,
            carrying: None,
            action: FigureAction::Idle,
        }
    }
}

/// A removable cover, rope, or chock as drawn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PropState {
    pub id: String,
    pub kind: PropKind,
    pub position_m: [f32; 3],
    pub yaw_rad: f32,
}

/// A control surface held at a hand-set position, in the same normalised units the exterior
/// pose uses for that surface (aileron and elevator -1 to 1, flap 0 to 1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SurfaceOverride {
    /// Exterior `moving_parts` node name.
    pub node: String,
    pub value: f32,
}

/// The inside of a tank seen through its open filler cap.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FuelView {
    pub tank: String,
    /// Filler cap position, metres.
    pub cap_m: [f32; 3],
    pub litres: f32,
    pub capacity_l: f32,
    /// Level at which the whole bottom of the tank is just covered.
    pub covers_bottom_l: f32,
    /// Fuel is visible and covers the bottom of the tank.
    pub covers_bottom: bool,
}

/// The oil tank dipstick read between its MIN and MAX marks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DipstickView {
    pub position_m: [f32; 3],
    /// 0 at the MIN mark and 1 at the MAX mark; below 0 is low.
    pub level: f32,
}

/// A visible sign of a finding, drawn on the model once found.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Cue {
    pub kind: DefectKind,
    pub position_m: [f32; 3],
}

/// Everything drawn for a walkaround at one instant.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WalkaroundScene {
    pub figure: FigureState,
    pub props: Vec<PropState>,
    pub surfaces: Vec<SurfaceOverride>,
    /// Propeller angle held by the hand, radians, overriding the engine-driven one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub propeller_phase: Option<f32>,
    /// Every tank's filler cap, open or closed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub caps: Vec<CapState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fuel_view: Option<FuelView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dipstick: Option<DipstickView>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cues: Vec<Cue>,
    /// Ground height in body coordinates (z down), metres.
    pub ground_z_m: f32,
    /// Where to look when the walkaround is the main outside view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<CameraFrame>,
}

/// A fuel filler cap.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CapState {
    pub tank: String,
    pub position_m: [f32; 3],
    pub open: bool,
}

/// A camera framing in body coordinates (x forward, y right, z down), metres.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CameraFrame {
    pub target_m: [f32; 3],
    pub eye_m: [f32; 3],
    pub fov_deg: f32,
}
