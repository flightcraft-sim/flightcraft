//! Bounded CIFP procedure catalogue. Courses are magnetic unless explicitly named true.
use crate::{
    GeoPoint, TerrainError,
    navdata::{iso_date, text_ok},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_PROCEDURES_BYTES: usize = 16 * 1024 * 1024;
pub const PROCEDURES_ATTRIBUTION: &str = "Instrument procedures: FAA CIFP and d-TPP, public domain; for simulation only, not for real-world navigation";
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum PathTerminator {
    IF,
    TF,
    CF,
    DF,
    FA,
    FC,
    FD,
    FM,
    CA,
    CD,
    CI,
    CR,
    RF,
    AF,
    VA,
    VD,
    VI,
    VM,
    VR,
    PI,
    HA,
    HF,
    HM,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Turn {
    L,
    R,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProcedureKind {
    Departure,
    Arrival,
    Approach,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProcedurePoint {
    pub id: String,
    pub ident: String,
    pub airport: Option<String>,
    pub section: String,
    pub position: GeoPoint,
    pub elevation_m: Option<f64>,
    pub magnetic_variation_deg: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProcedureLeg {
    pub sequence: u16,
    pub fix: Option<String>,
    pub waypoint_description: String,
    pub path: PathTerminator,
    pub turn: Option<Turn>,
    pub recommended_navaid: Option<String>,
    pub theta_deg: Option<f64>,
    pub rho_nm: Option<f64>,
    pub course_deg: Option<f64>,
    pub course_true: bool,
    pub distance_nm: Option<f64>,
    pub time_s: Option<f64>,
    pub altitude_descriptor: String,
    pub altitude1_m: Option<f64>,
    pub altitude2_m: Option<f64>,
    pub speed_limit_kt: Option<f64>,
    pub vertical_angle_deg: Option<f64>,
    pub arc_radius_nm: Option<f64>,
    pub center_fix: Option<String>,
    pub missed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProcedureRoute {
    pub route_type: String,
    pub transition: String,
    #[schemars(length(min = 1, max = 256))]
    pub legs: Vec<ProcedureLeg>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlateLink {
    pub name: String,
    pub pdf: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ApproachCapability {
    LNAV,
    #[serde(rename = "LNAV/VNAV")]
    LnavVnav,
    LPV,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Procedure {
    pub airport: String,
    pub ident: String,
    pub kind: ProcedureKind,
    pub runway: Option<String>,
    pub plate: Option<PlateLink>,
    #[schemars(length(max = 3))]
    pub capabilities: Vec<ApproachCapability>,
    #[schemars(length(min = 1, max = 128))]
    pub routes: Vec<ProcedureRoute>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Localizer {
    pub airport: String,
    pub ident: String,
    pub runway: String,
    pub frequency_hz: f64,
    pub position: GeoPoint,
    pub course_magnetic_deg: f64,
    pub magnetic_variation_deg: f64,
    /// Full angular width, degrees.
    pub course_width_deg: Option<f64>,
    pub gs_position: Option<GeoPoint>,
    pub gs_angle_deg: Option<f64>,
    pub gs_elevation_m: Option<f64>,
    pub threshold_crossing_height_m: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SbasPathPoint {
    pub airport: String,
    pub approach: String,
    pub runway: String,
    pub threshold: GeoPoint,
    pub flight_path_alignment: GeoPoint,
    pub angle_deg: f64,
    /// Original PP payload, retained for service and integrity fields not used by this receiver.
    pub payload: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Procedures {
    #[schemars(range(min = 1, max = 1))]
    pub version: u8,
    #[schemars(length(min = 4, max = 4))]
    pub cycle: String,
    pub effective_date: String,
    pub attribution: String,
    #[schemars(length(max = 30000))]
    pub points: Vec<ProcedurePoint>,
    #[schemars(length(max = 4096))]
    pub localizers: Vec<Localizer>,
    #[schemars(length(max = 4096))]
    pub sbas: Vec<SbasPathPoint>,
    #[schemars(length(max = 10000))]
    pub procedures: Vec<Procedure>,
}
fn number(v: Option<f64>, lo: f64, hi: f64) -> bool {
    v.is_none_or(|n| n.is_finite() && (lo..=hi).contains(&n))
}
impl Procedures {
    pub fn from_json(text: &str) -> Result<Self, TerrainError> {
        if text.len() > MAX_PROCEDURES_BYTES {
            return Err(TerrainError("procedures exceed 16 MiB"));
        }
        let value: Self = serde_json::from_str(text).map_err(|_| TerrainError("invalid procedures JSON"))?;
        value.validate()?;
        Ok(value)
    }
    pub fn point(&self, id: &str) -> Option<&ProcedurePoint> {
        self.points.iter().find(|p| p.id == id)
    }
    pub fn validate(&self) -> Result<(), TerrainError> {
        let bad = || TerrainError("invalid procedure catalogue or reference");
        if self.version != 1
            || self.cycle.len() != 4
            || !self.cycle.bytes().all(|b| b.is_ascii_digit())
            || !iso_date(&self.effective_date)
            || !text_ok(&self.attribution, 2048)
            || self.points.len() > 30000
            || self.localizers.len() > 4096
            || self.sbas.len() > 4096
            || self.procedures.len() > 10000
        {
            return Err(bad());
        }
        let mut ids = BTreeSet::new();
        for p in &self.points {
            p.position.validate()?;
            if !text_ok(&p.id, 64)
                || !ids.insert(&p.id)
                || !text_ok(&p.ident, 16)
                || !text_ok(&p.section, 2)
                || p.airport.as_deref().is_some_and(|a| !text_ok(a, 8))
                || !number(p.elevation_m, -1000., 20000.)
                || !number(p.magnetic_variation_deg, -180., 180.)
            {
                return Err(TerrainError("invalid procedure point"));
            }
        }
        let mut keys = BTreeSet::new();
        let mut total = 0usize;
        for p in &self.procedures {
            if !text_ok(&p.airport, 8)
                || !text_ok(&p.ident, 16)
                || !keys.insert((&p.airport, &p.ident))
                || p.runway.as_deref().is_some_and(|r| !text_ok(r, 8))
                || p.capabilities.len() > 3
                || p.routes.is_empty()
                || p.routes.len() > 128
            {
                return Err(TerrainError("invalid procedure route or leg"));
            }
            if let Some(plate) = &p.plate
                && (!text_ok(&plate.name, 256)
                    || !text_ok(&plate.pdf, 64)
                    || !plate
                        .pdf
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
                    || !plate.pdf.to_ascii_uppercase().ends_with(".PDF"))
            {
                return Err(TerrainError("invalid procedure route or leg"));
            }
            let mut routes = BTreeSet::new();
            for r in &p.routes {
                if r.route_type.len() != 1
                    || !r.route_type.bytes().all(|b| b.is_ascii_alphanumeric())
                    || r.transition.len() > 8
                    || r.transition.chars().any(char::is_control)
                    || !routes.insert((&r.route_type, &r.transition))
                    || r.legs.is_empty()
                    || r.legs.len() > 256
                {
                    return Err(TerrainError("invalid procedure route or leg"));
                }
                let mut previous = None;
                let mut missed = false;
                total = total.saturating_add(r.legs.len());
                for l in &r.legs {
                    if previous.is_some_and(|s| s >= l.sequence)
                        || (missed && !l.missed)
                        || [&l.fix, &l.recommended_navaid, &l.center_fix]
                            .iter()
                            .any(|id| id.as_ref().is_some_and(|id| !ids.contains(id)))
                        || l.waypoint_description.len() > 4
                        || !l
                            .waypoint_description
                            .bytes()
                            .all(|b| b == b' ' || b.is_ascii_alphanumeric())
                        || !matches!(
                            l.altitude_descriptor.as_str(),
                            "" | "+" | "-" | "B" | "C" | "D" | "G" | "H" | "I" | "J" | "V" | "X" | "Y"
                        )
                        || !number(l.theta_deg, 0., 360.)
                        || !number(l.rho_nm, 0., 1000.)
                        || !number(l.course_deg, 0., 360.)
                        || !number(l.distance_nm, 0., 1000.)
                        || !number(l.time_s, 0., 3600.)
                        || (l.distance_nm.is_some() && l.time_s.is_some())
                        || !number(l.altitude1_m, -1000., 20000.)
                        || !number(l.altitude2_m, -1000., 20000.)
                        || !number(l.speed_limit_kt, 0., 600.)
                        || !number(l.vertical_angle_deg, -10., 10.)
                        || !number(l.arc_radius_nm, 0., 1000.)
                    {
                        return Err(TerrainError("invalid procedure route or leg"));
                    }
                    previous = Some(l.sequence);
                    missed = l.missed;
                }
            }
        }
        if total > 100000 {
            return Err(bad());
        }
        for l in &self.localizers {
            l.position.validate()?;
            if let Some(p) = l.gs_position {
                p.validate()?;
            }
            if !text_ok(&l.airport, 8)
                || !text_ok(&l.ident, 8)
                || !text_ok(&l.runway, 8)
                || !number(Some(l.frequency_hz), 108e6, 112e6)
                || !number(Some(l.course_magnetic_deg), 0., 360.)
                || !number(Some(l.magnetic_variation_deg), -180., 180.)
                || !number(l.course_width_deg, 0.5, 10.)
                || !number(l.gs_angle_deg, 1., 10.)
                || !number(l.gs_elevation_m, -1000., 10000.)
                || !number(l.threshold_crossing_height_m, 0., 100.)
            {
                return Err(TerrainError("invalid localizer fields"));
            }
        }
        for s in &self.sbas {
            s.threshold.validate()?;
            s.flight_path_alignment.validate()?;
            if !text_ok(&s.airport, 8)
                || !text_ok(&s.approach, 16)
                || !text_ok(&s.runway, 8)
                || !number(Some(s.angle_deg), 0., 10.)
                || s.payload.len() != 132
                || !s.payload.is_ascii()
            {
                return Err(TerrainError("invalid SBAS path point"));
            }
        }
        Ok(())
    }
}
