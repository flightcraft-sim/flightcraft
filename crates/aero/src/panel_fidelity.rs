//! Optional pack artwork, expressed in millimetres and sRGB. No behaviour lives here.
use crate::{AircraftError, aircraft::require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Six-digit opaque sRGB, including the leading `#`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct SrgbHex(#[schemars(regex(pattern = "^#[0-9a-fA-F]{6}$"))] pub String);
impl SrgbHex {
    pub fn rgb(&self) -> Option<[u8; 3]> {
        let value = self.0.strip_prefix('#')?;
        if value.len() != 6 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let n = u32::from_str_radix(value, 16).ok()?;
        Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
    }
    pub fn validate(&self) -> Result<(), AircraftError> {
        require(self.rgb().is_some(), "panel.color", "requires #RRGGBB")
    }
    pub fn contrast(&self, other: &Self) -> Option<f64> {
        let luminance = |c: [u8; 3]| {
            let [r, g, b] = c.map(|n| {
                let v = f64::from(n) / 255.;
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    libm::pow((v + 0.055) / 1.055, 2.4)
                }
            });
            0.2126 * r + 0.7152 * g + 0.0722 * b
        };
        let (a, b) = (luminance(self.rgb()?), luminance(other.rgb()?));
        Some((a.max(b) + 0.05) / (a.min(b) + 0.05))
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PanelPalette {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub face: Option<SrgbHex>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glare_shield: Option<SrgbHex>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bezel: Option<SrgbHex>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placard_text: Option<SrgbHex>,
}
impl PanelPalette {
    pub fn validate(&self) -> Result<(), AircraftError> {
        for c in [&self.face, &self.glare_shield, &self.bezel, &self.placard_text]
            .into_iter()
            .flatten()
        {
            c.validate()?;
        }
        let ink = self.placard_text.clone().unwrap_or(SrgbHex("#e4edf2".into()));
        for bg in self.face.as_ref().into_iter().chain(self.bezel.as_ref()) {
            require(
                ink.contrast(bg).is_some_and(|v| v >= 4.5),
                "panel.palette",
                "labels require contrast of at least 4.5:1",
            )?;
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ControlStyle {
    BlackRocker,
    PushPullBreaker,
    RoundedPullHandle,
    SilverKey,
    LouvredEyeball,
    EltPlate,
    RedTHandle,
    ArcDimmer,
    CabinHeat,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ControlBinding {
    LaneA,
    LaneB,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NavigatorLayout {
    #[default]
    Classic,
    HomeColumns,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DisplayBezel {
    /// A display model supplies the bezel geometry and control set; `margins`, soft keys, and
    /// corner knobs are then ignored and the installation must have the model's bezel aspect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<crate::display_model::DisplayModel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<SrgbHex>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_trim: Option<SrgbHex>,
    /// Left, top, right, bottom margins, in mm.
    #[serde(default)]
    pub margins: [f64; 4],
    #[serde(default)]
    pub softkeys: bool,
    #[serde(default = "one_key")]
    #[schemars(range(min = 1, max = 12))]
    pub softkey_count: u8,
    /// Knob centres in local installation mm; zero, one, or two knobs.
    #[serde(default)]
    #[schemars(length(max = 2))]
    pub corner_knobs: Vec<[f64; 2]>,
    #[serde(default)]
    pub card_slot: bool,
}
fn one_key() -> u8 {
    1
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ControllerAction {
    Ap,
    Fd,
    Yd,
    Lvl,
    Hdg,
    Nav,
    Apr,
    Vs,
    Alt,
    Ias,
    Vnav,
    HeadingKnob,
    AltitudeKnob,
    PitchWheel,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ControllerControl {
    pub action: ControllerAction,
    pub rect: [f64; 4],
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lit_color: Option<SrgbHex>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ControllerSection {
    pub label: String,
    pub rect: [f64; 4],
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ControllerLayout {
    #[schemars(length(max = 8))]
    pub sections: Vec<ControllerSection>,
    #[schemars(length(min = 1, max = 32))]
    pub controls: Vec<ControllerControl>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct EquipmentAppearance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<ControlStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding: Option<ControlBinding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bezel: Option<DisplayBezel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub controller: Option<ControllerLayout>,
    pub navigator_layout: NavigatorLayout,
    /// Host registry artwork id; omission selects original artwork.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skin: Option<String>,
    /// A soft reference to a skin pack id. It is used when that pack is installed and the pilot has
    /// not chosen another; otherwise the built-in public skin is shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommended_skin: Option<String>,
    pub synthetic_vision: bool,
}
pub(crate) fn rectangle([x, y, w, h]: [f64; 4], [width, height]: [f64; 2]) -> bool {
    [x, y, w, h].iter().all(|v| v.is_finite())
        && x >= 0.
        && y >= 0.
        && w > 0.
        && h > 0.
        && x + w <= width
        && y + h <= height
}
impl EquipmentAppearance {
    pub fn validate(&self, size: [f64; 2]) -> Result<(), AircraftError> {
        if let Some(id) = &self.skin {
            crate::layers::id(id, "equipment.skin")?;
        }
        if let Some(id) = &self.recommended_skin {
            crate::layers::id(id, "equipment.recommended_skin")?;
        }
        if let Some(model) = self.bezel.as_ref().and_then(|b| b.model) {
            let [w, h] = size;
            require(
                (w / h / model.bezel_aspect() - 1.).abs() <= 0.03,
                "display.bezel.model",
                "installation aspect must match the display model bezel within 3%",
            )?;
        }
        if let Some(b) = self.bezel.as_ref().filter(|b| b.model.is_none()) {
            for c in [&b.color, &b.edge_trim].into_iter().flatten() {
                c.validate()?;
            }
            let [l, t, r, bottom] = b.margins;
            let [w, h] = size;
            require(
                b.margins.iter().all(|v| v.is_finite() && *v >= 0.)
                    && l + r < w
                    && t + bottom < h
                    && (1..=12).contains(&b.softkey_count)
                    && b.corner_knobs.len() <= 2
                    && b.corner_knobs.iter().all(|[x, y]| {
                        x.is_finite() && y.is_finite() && (0. ..=w).contains(x) && (0. ..=h).contains(y)
                    }),
                "display.bezel",
                "invalid margins, key count, or knob position",
            )?;
        }
        if let Some(c) = &self.controller {
            require(
                c.sections.len() <= 8 && (1..=32).contains(&c.controls.len()),
                "controller",
                "invalid section or control count",
            )?;
            let mut actions = std::collections::BTreeSet::new();
            for control in &c.controls {
                require(
                    actions.insert(control.action) && rectangle(control.rect, size),
                    "controller.control",
                    "duplicate action or rectangle outside installation",
                )?;
                if let Some(color) = &control.lit_color {
                    color.validate()?;
                }
            }
            for (i, a) in c.controls.iter().enumerate() {
                for b in c.controls.iter().skip(i + 1) {
                    let [ax, ay, aw, ah] = a.rect;
                    let [bx, by, bw, bh] = b.rect;
                    require(
                        ax + aw <= bx || bx + bw <= ax || ay + ah <= by || by + bh <= ay,
                        "controller.controls",
                        "control hit rectangles must not overlap",
                    )?;
                }
            }
            for section in &c.sections {
                require(
                    !section.label.is_empty()
                        && section.label.len() <= 32
                        && !section.label.chars().any(char::is_control)
                        && rectangle(section.rect, size),
                    "controller.section",
                    "invalid label or rectangle",
                )?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BackupCircuit {
    Ems,
    Efis,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BackupBus {
    pub circuit: BackupCircuit,
    pub capacity_ah: f64,
    pub voltage: f64,
    #[schemars(length(min = 1, max = 64))]
    pub loads: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Wigwag {
    pub left_load: String,
    pub right_load: String,
    pub half_period_s: f64,
}
