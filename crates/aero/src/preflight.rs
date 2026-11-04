//! Exterior pre-flight walkaround data, carried in an aircraft pack's systems layer.
//!
//! The data is a transcription of the aircraft's own pre-flight inspection list: stations in walk
//! order, every item with its handbook reference, the physical places the figure walks and
//! reaches, removable covers and ropes, and which findings an instructor may seed on which item.
//! The engine adds no inspection steps of its own. See `docs/preflight.md`.
use crate::AircraftError;
use crate::aircraft::{error, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const PREFLIGHT_VERSION: u8 = 1;
pub const MAX_ROUTE_NODES: usize = 64;
pub const MAX_STATIONS: usize = 16;
pub const MAX_ITEMS: usize = 128;
pub const MAX_PROPS: usize = 32;
pub const MAX_KEEPOUTS: usize = 16;
/// Clearance between the walking route and any keep-out rectangle, in metres.
pub const WALK_CLEARANCE_M: f64 = 0.2;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Preflight {
    /// Document version of this section. Currently 1.
    #[schemars(range(min = 1, max = 1))]
    pub version: u8,
    /// Where the list comes from, for example the handbook revision and section.
    pub source: String,
    /// Walking pace in metres per second at real time.
    #[schemars(range(min = 0.5, max = 2.5))]
    pub pace_mps: f64,
    /// Where the pilot sits when an item is done from the cockpit: the hip point on the seat.
    pub seat_m: [f64; 3],
    /// Plan-view rectangles the figure never enters: wings, fuselage, tail, and the propeller disc.
    #[schemars(length(max = 16))]
    pub keepouts: Vec<Keepout>,
    /// A closed loop of ground points around the aircraft. The figure only walks along it.
    #[schemars(length(min = 3, max = 64))]
    pub route: Vec<RouteNode>,
    /// Stations in handbook walk order.
    #[schemars(length(min = 1, max = 16))]
    pub stations: Vec<Station>,
    #[schemars(length(min = 1, max = 128))]
    pub items: Vec<Item>,
    /// Covers, ropes, and chocks present on a parked aircraft.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 32))]
    pub props: Vec<Prop>,
    pub fuel: FuelCheck,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Keepout {
    pub id: String,
    /// Body x (forward) and y (right), metres.
    pub min_m: [f64; 2],
    pub max_m: [f64; 2],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RouteNode {
    pub id: String,
    /// Body x (forward) and y (right), metres.
    pub position_m: [f64; 2],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Station {
    pub id: String,
    pub name: String,
    /// The route node where the figure stands for this station's items.
    pub node: String,
    /// Handbook reference of the station.
    pub poh_ref: String,
    pub camera: StationCamera,
}

/// Framing for the exterior inset: look at `target_m`, from `target_m + eye_offset_m`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StationCamera {
    /// Body-frame point (x forward, y right, z down), metres.
    pub target_m: [f64; 3],
    pub eye_offset_m: [f64; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub id: String,
    pub station: String,
    /// The handbook's own wording.
    pub text: String,
    /// Handbook section and list, for example `4.4 (4) Right wing and main gear`.
    pub poh_ref: String,
    pub action: Action,
    /// Route node to stand at, when it differs from the station's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// Skipping an optional item is not graded as a miss.
    #[serde(default)]
    pub optional: bool,
    /// A handbook warning or caution that applies to the item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    /// Findings an instructor may seed on this item, with their text and cue.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 8))]
    pub defects: Vec<DefectOption>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Stance {
    Stand,
    Crouch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CabinEffect {
    /// A look or a hand check with no switch involved.
    None,
    MasterOn,
    MasterOff,
    IgnitionOff,
    /// Move the flap control to full down.
    FlapsFull,
    /// Read the fuel quantity indicator.
    FuelIndicator,
}

/// What the figure does for an item.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    /// Look at a point, optionally touching it ("condition", "secure").
    Inspect {
        target_m: [f64; 3],
        stance: Stance,
        touch: bool,
    },
    /// Move control surfaces by hand through their full travel. `parts` are exterior
    /// `moving_parts` node names; `target_m` is where the hand takes hold.
    MoveControl {
        parts: Vec<String>,
        target_m: [f64; 3],
        #[schemars(range(min = 0.1, max = 1.0))]
        amplitude: f64,
    },
    /// Open a fuel filler cap and look at the level inside.
    FuelCap { tank: String },
    /// Pull the oil tank dipstick and read it between its minimum and maximum marks.
    OilCheck { target_m: [f64; 3] },
    /// Handle the propeller blade area with the palm of the hand. Ignition must be off.
    PalmProp { target_m: [f64; 3] },
    /// Remove covers, ropes, or chocks.
    Remove { props: Vec<String> },
    /// An item done from the cockpit.
    Cabin { effect: CabinEffect, target_m: [f64; 3] },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Do not fly until fixed.
    NoGo,
    /// Fix or accept knowingly; flying with it is graded down.
    Caution,
}

/// Findings an instructor or lesson can seed. The first three are physical props left on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DefectKind {
    PitotCover,
    TieDown,
    Chock,
    LowOil,
    LowFuel,
    LooseCowlFastener,
    AileronHardwareMissing,
    FlapHardwareMissing,
    DamagedTyre,
    WheelFairingCrack,
}

impl DefectKind {
    /// Covers, ropes, and chocks are props the item removes; the rest are defects on the item.
    #[must_use]
    pub fn is_prop(self) -> bool {
        matches!(self, Self::PitotCover | Self::TieDown | Self::Chock)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DefectOption {
    pub kind: DefectKind,
    /// What the student sees and reads when the item reveals it.
    pub description: String,
    pub severity: Severity,
    /// Body-frame position of the visual cue on the model.
    pub cue_m: [f64; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PropKind {
    PitotCover,
    Chock,
    TieDownRope,
}

impl PropKind {
    #[must_use]
    pub fn defect(self) -> DefectKind {
        match self {
            Self::PitotCover => DefectKind::PitotCover,
            Self::Chock => DefectKind::Chock,
            Self::TieDownRope => DefectKind::TieDown,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Prop {
    pub id: String,
    pub kind: PropKind,
    /// Body-frame position, metres.
    pub position_m: [f64; 3],
    #[serde(default)]
    pub yaw_rad: f64,
    /// Route node where the figure stands to take it off.
    pub node: String,
    /// Present when a parked-aircraft walkaround begins.
    pub present_at_start: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FuelCheck {
    /// Fuel density used to convert tank mass to litres.
    #[schemars(range(min = 0.5, max = 1.0))]
    pub density_kg_per_l: f64,
    /// Litres at which fuel can just be seen covering the whole bottom of a tank.
    #[schemars(range(min = 1, max = 500))]
    pub covers_bottom_l: f64,
    #[schemars(length(min = 1, max = 8))]
    pub caps: Vec<FuelCap>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FuelCap {
    /// Tank id from the systems definition.
    pub tank: String,
    /// Body-frame position of the filler cap, metres.
    pub cap_m: [f64; 3],
}

fn text_ok(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}

fn point_ok(p: &[f64]) -> bool {
    p.iter().all(|v| v.is_finite() && v.abs() <= 100.)
}

fn in_rect(p: [f64; 2], k: &Keepout, margin: f64) -> bool {
    p[0] > k.min_m[0] - margin
        && p[0] < k.max_m[0] + margin
        && p[1] > k.min_m[1] - margin
        && p[1] < k.max_m[1] + margin
}

/// Whether the segment `a`-`b` passes through the rectangle grown by `margin`.
fn segment_hits(a: [f64; 2], b: [f64; 2], k: &Keepout, margin: f64) -> bool {
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    for (((a, b), lo), hi) in a.iter().zip(&b).zip(&k.min_m).zip(&k.max_m) {
        let (lo, hi) = (lo - margin, hi + margin);
        let d = b - a;
        if d.abs() < 1e-12 {
            if *a <= lo || *a >= hi {
                return false;
            }
        } else {
            let (mut u0, mut u1) = ((lo - a) / d, (hi - a) / d);
            if u0 > u1 {
                std::mem::swap(&mut u0, &mut u1);
            }
            t0 = t0.max(u0);
            t1 = t1.min(u1);
            if t0 >= t1 {
                return false;
            }
        }
    }
    true
}

impl Preflight {
    #[must_use]
    pub fn station(&self, id: &str) -> Option<&Station> {
        self.stations.iter().find(|s| s.id == id)
    }
    #[must_use]
    pub fn item(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }
    #[must_use]
    pub fn node(&self, id: &str) -> Option<&RouteNode> {
        self.route.iter().find(|n| n.id == id)
    }
    #[must_use]
    pub fn prop(&self, id: &str) -> Option<&Prop> {
        self.props.iter().find(|p| p.id == id)
    }
    /// Props an item removes.
    #[must_use]
    pub fn props_of(&self, item: &Item) -> Vec<&Prop> {
        match &item.action {
            Action::Remove { props } => props.iter().filter_map(|id| self.prop(id)).collect(),
            _ => Vec::new(),
        }
    }

    /// Structural validation; exterior and systems cross-checks happen when a pack is installed.
    pub fn validate(&self) -> Result<(), AircraftError> {
        let bad = |m: &str| error("preflight", m.to_string());
        require(
            self.version == PREFLIGHT_VERSION,
            "preflight.version",
            "unsupported version",
        )?;
        require(
            text_ok(&self.source, 512),
            "preflight.source",
            "requires 1 to 512 bytes of text",
        )?;
        require(
            self.pace_mps.is_finite() && (0.5..=2.5).contains(&self.pace_mps),
            "preflight.pace_mps",
            "must be between 0.5 and 2.5",
        )?;
        require(
            self.keepouts.len() <= MAX_KEEPOUTS
                && (3..=MAX_ROUTE_NODES).contains(&self.route.len())
                && (1..=MAX_STATIONS).contains(&self.stations.len())
                && (1..=MAX_ITEMS).contains(&self.items.len())
                && self.props.len() <= MAX_PROPS,
            "preflight",
            "section exceeds its bounds",
        )?;
        require(
            point_ok(&self.seat_m),
            "preflight.seat_m",
            "requires a finite point",
        )?;
        let mut ids = BTreeSet::new();
        let mut unique = |what: &str, id: &str| -> Result<(), AircraftError> {
            require(
                text_ok(id, 64)
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'),
                "preflight.id",
                "ids are 1 to 64 lowercase letters, digits, underscores, or hyphens",
            )?;
            require(
                ids.insert((what.to_string(), id.to_string())),
                "preflight.id",
                "duplicate id",
            )
        };
        for k in &self.keepouts {
            unique("keepout", &k.id)?;
            require(
                point_ok(&k.min_m)
                    && point_ok(&k.max_m)
                    && k.min_m[0] < k.max_m[0]
                    && k.min_m[1] < k.max_m[1],
                "preflight.keepouts",
                "requires a finite rectangle with min below max",
            )?;
        }
        for n in &self.route {
            unique("node", &n.id)?;
            require(
                point_ok(&n.position_m),
                "preflight.route",
                "requires finite positions",
            )?;
            for k in &self.keepouts {
                require(
                    !in_rect(n.position_m, k, WALK_CLEARANCE_M),
                    "preflight.route",
                    "a route node is inside or too close to a keep-out",
                )?;
            }
        }
        for (i, a) in self.route.iter().enumerate() {
            let b = self
                .route
                .get((i + 1) % self.route.len())
                .ok_or_else(|| bad("route"))?;
            for k in &self.keepouts {
                require(
                    !segment_hits(a.position_m, b.position_m, k, WALK_CLEARANCE_M),
                    "preflight.route",
                    "a route segment passes through a keep-out",
                )?;
            }
        }
        let node_ok = |id: &str| self.node(id).is_some();
        for s in &self.stations {
            unique("station", &s.id)?;
            require(
                text_ok(&s.name, 128) && text_ok(&s.poh_ref, 128) && node_ok(&s.node),
                "preflight.stations",
                "requires a name, a handbook reference, and a known route node",
            )?;
            require(
                point_ok(&s.camera.target_m) && point_ok(&s.camera.eye_offset_m),
                "preflight.stations",
                "camera requires finite points",
            )?;
            let o = s.camera.eye_offset_m;
            require(
                libm::sqrt(o[0] * o[0] + o[1] * o[1] + o[2] * o[2]) >= 0.5,
                "preflight.stations",
                "camera eye must be at least 0.5 m from its target",
            )?;
        }
        let mut order: Vec<&str> = Vec::new();
        for item in &self.items {
            unique("item", &item.id)?;
            require(
                text_ok(&item.text, 512) && text_ok(&item.poh_ref, 128),
                "preflight.items",
                "every item needs its text and a handbook reference",
            )?;
            require(
                item.warning.as_deref().is_none_or(|w| text_ok(w, 1024)),
                "preflight.items",
                "warning must be 1 to 1024 bytes of text",
            )?;
            require(
                self.station(&item.station).is_some(),
                "preflight.items",
                "unknown station",
            )?;
            // Items stay grouped in station order, as the handbook lists them.
            if order.last() != Some(&item.station.as_str()) {
                require(
                    !order.contains(&item.station.as_str()),
                    "preflight.items",
                    "items of a station must be contiguous",
                )?;
                order.push(&item.station);
            }
            require(
                item.node.as_deref().is_none_or(&node_ok),
                "preflight.items",
                "unknown route node",
            )?;
            self.validate_action(item)?;
            let mut kinds = BTreeSet::new();
            for d in &item.defects {
                require(
                    !d.kind.is_prop()
                        && text_ok(&d.description, 512)
                        && point_ok(&d.cue_m)
                        && kinds.insert(d.kind),
                    "preflight.items",
                    "defects need unique non-prop kinds, text, and a finite cue",
                )?;
            }
        }
        let station_ids: Vec<&str> = self.stations.iter().map(|s| s.id.as_str()).collect();
        require(
            order == station_ids,
            "preflight.items",
            "stations must be used in walk order, each with at least one item",
        )?;
        let mut removers: BTreeMap<&str, &str> = BTreeMap::new();
        for p in &self.props {
            unique("prop", &p.id)?;
            require(
                point_ok(&p.position_m) && p.yaw_rad.is_finite() && node_ok(&p.node),
                "preflight.props",
                "requires a finite position and a known route node",
            )?;
        }
        for item in &self.items {
            if let Action::Remove { props } = &item.action {
                for id in props {
                    require(
                        removers.insert(id, &item.id).is_none(),
                        "preflight.items",
                        "a prop can only be removed by one item",
                    )?;
                }
            }
        }
        require(
            self.props.iter().all(|p| removers.contains_key(p.id.as_str())),
            "preflight.props",
            "every prop must be removed by an item",
        )?;
        require(
            self.fuel.density_kg_per_l.is_finite()
                && (0.5..=1.0).contains(&self.fuel.density_kg_per_l)
                && self.fuel.covers_bottom_l.is_finite()
                && self.fuel.covers_bottom_l > 0.
                && (1..=8).contains(&self.fuel.caps.len())
                && self
                    .fuel
                    .caps
                    .iter()
                    .all(|c| text_ok(&c.tank, 64) && point_ok(&c.cap_m)),
            "preflight.fuel",
            "requires a density, a cover level, and filler caps",
        )?;
        Ok(())
    }

    fn validate_action(&self, item: &Item) -> Result<(), AircraftError> {
        let what = "preflight.items";
        match &item.action {
            Action::Inspect { target_m, .. }
            | Action::OilCheck { target_m }
            | Action::PalmProp { target_m } => {
                require(point_ok(target_m), what, "action target must be finite")
            }
            Action::Cabin { target_m, .. } => {
                require(point_ok(target_m), what, "action target must be finite")
            }
            Action::MoveControl {
                parts,
                target_m,
                amplitude,
            } => require(
                !parts.is_empty()
                    && parts.len() <= 4
                    && parts.iter().all(|p| text_ok(p, 64))
                    && point_ok(target_m)
                    && amplitude.is_finite()
                    && (0.1..=1.0).contains(amplitude),
                what,
                "control movement needs parts, a finite target, and an amplitude",
            ),
            Action::FuelCap { tank } => require(
                self.fuel.caps.iter().any(|c| &c.tank == tank),
                what,
                "unknown fuel tank cap",
            ),
            Action::Remove { props } => require(
                !props.is_empty() && props.iter().all(|p| self.prop(p).is_some()),
                what,
                "removal needs known props",
            ),
        }
    }

    /// Ids of every exterior moving-part node the walkaround drives.
    #[must_use]
    pub fn driven_parts(&self) -> BTreeSet<&str> {
        self.items
            .iter()
            .filter_map(|i| match &i.action {
                Action::MoveControl { parts, .. } => Some(parts.iter().map(String::as_str)),
                _ => None,
            })
            .flatten()
            .collect()
    }
}
