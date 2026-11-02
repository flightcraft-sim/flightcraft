//! Bounded geographic airspace documents and deterministic local-volume queries.
use crate::{TerrainError, World, srtm::LocalProjection};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const MAX_AIRSPACE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_AIRSPACE_VERTICES: usize = 50_000;
pub const FAA_AIRSPACE_ATTRIBUTION: &str = "FAA Aeronautical Data Delivery Service, public domain";
pub const FOOT_M: f64 = 0.3048;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
pub enum AirspaceClass {
    A,
    B,
    C,
    D,
    E,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AirspaceKind {
    Class,
    ModeC,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum AltitudeReference {
    MSL,
    SFC,
    STD,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AirspaceAltitude {
    pub value_ft: f64,
    pub reference: AltitudeReference,
}
impl AirspaceAltitude {
    /// STD is drawn and queried as nominal MSL; pressure correction is not yet modelled.
    pub fn msl_m(self, terrain_m: f64) -> f64 {
        self.value_ft * FOOT_M
            + if self.reference == AltitudeReference::SFC {
                terrain_m
            } else {
                0.0
            }
    }
}
/// Polygon rings contain latitude/longitude degrees, with the exterior first.
pub type GeographicPolygon = Vec<Vec<[f64; 2]>>;
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AirspaceVolume {
    pub id: String,
    pub name: String,
    /// Mode C veils have no airspace class.
    pub class: Option<AirspaceClass>,
    pub kind: AirspaceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sector: Option<String>,
    pub floor: AirspaceAltitude,
    pub ceiling: AirspaceAltitude,
    pub polygons: Vec<GeographicPolygon>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AirspaceFile {
    pub attribution: String,
    pub volumes: Vec<AirspaceVolume>,
}
impl AirspaceFile {
    pub fn from_json(text: &str) -> Result<Self, TerrainError> {
        if text.len() > MAX_AIRSPACE_BYTES {
            return Err(TerrainError("airspace file too large"));
        }
        let mut file: Self = serde_json::from_str(text).map_err(|_| TerrainError("invalid airspace JSON"))?;
        file.validate()?;
        Ok(file)
    }
    /// Close closable rings, and reject ambiguous or unbounded input before projection.
    pub fn validate(&mut self) -> Result<(), TerrainError> {
        if self.attribution.trim().is_empty() || self.attribution.len() > 2048 || self.volumes.len() > 4096 {
            return Err(TerrainError("invalid airspace attribution or volume count"));
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut vertices = 0_usize;
        for v in &mut self.volumes {
            if v.id.is_empty()
                || v.id.len() > 128
                || !ids.insert(v.id.clone())
                || v.name.trim().is_empty()
                || v.name.len() > 256
                || v.sector.as_ref().is_some_and(|s| s.is_empty() || s.len() > 128)
                || (v.kind == AirspaceKind::Class) != v.class.is_some()
                || v.polygons.is_empty()
            {
                return Err(TerrainError("invalid airspace identity or kind"));
            }
            for a in [v.floor, v.ceiling] {
                if !a.value_ft.is_finite()
                    || !(-2000.0..=100000.0).contains(&a.value_ft)
                    || (a.reference == AltitudeReference::SFC && a.value_ft < 0.0)
                {
                    return Err(TerrainError("invalid airspace altitude"));
                }
            }
            if v.floor.value_ft >= v.ceiling.value_ft {
                return Err(TerrainError("airspace floor must be below ceiling"));
            }
            for polygon in &mut v.polygons {
                if polygon.is_empty() {
                    return Err(TerrainError("empty airspace polygon"));
                }
                for ring in polygon {
                    if ring.len() < 3
                        || ring.iter().any(|[lat, lon]| {
                            !lat.is_finite() || !lon.is_finite() || lat.abs() > 90.0 || lon.abs() > 180.0
                        })
                    {
                        return Err(TerrainError("invalid airspace ring"));
                    }
                    if ring.first() != ring.last()
                        && let Some(first) = ring.first().copied()
                    {
                        ring.push(first);
                    }
                    vertices = vertices.saturating_add(ring.len());
                    if vertices > MAX_AIRSPACE_VERTICES
                        || ring.len() < 4
                        || ring.windows(2).any(|w| matches!(w, [a,b] if a == b))
                        || ring
                            .windows(2)
                            .map(|w| match w {
                                [a, b] => a[0] * b[1] - b[0] * a[1],
                                _ => 0.0,
                            })
                            .sum::<f64>()
                            .abs()
                            < 1e-12
                    {
                        return Err(TerrainError("degenerate ring or airspace vertex budget exceeded"));
                    }
                }
            }
        }
        Ok(())
    }
    pub fn project(mut self, projection: LocalProjection) -> Result<Airspace, TerrainError> {
        self.validate()?;
        let mut volumes = Vec::new();
        for info in self.volumes {
            let polygons = info
                .polygons
                .iter()
                .map(|polygon| {
                    polygon
                        .iter()
                        .map(|ring| {
                            ring.iter()
                                .map(|[lat, lon]| projection.project(*lat, *lon))
                                .collect()
                        })
                        .collect()
                })
                .collect::<Result<Vec<Vec<Vec<_>>>, _>>()?;
            volumes.push(ProjectedAirspaceVolume { info, polygons });
        }
        Ok(Airspace {
            attribution: self.attribution,
            volumes,
        })
    }
}
#[derive(Clone, Debug)]
pub struct ProjectedAirspaceVolume {
    pub info: AirspaceVolume,
    /// Closed local north/east rings, exterior first, holes after.
    pub polygons: Vec<Vec<Vec<[f64; 2]>>>,
}
impl ProjectedAirspaceVolume {
    pub fn contains_lateral(&self, point: [f64; 2]) -> bool {
        self.polygons.iter().any(|p| {
            p.first().is_some_and(|r| in_ring(point, r))
                && p.iter()
                    .skip(1)
                    .filter(|r| in_ring(point, r))
                    .count()
                    .is_multiple_of(2)
        })
    }
    pub fn boundary_distance_m(&self, point: [f64; 2]) -> f64 {
        self.polygons
            .iter()
            .flatten()
            .flat_map(|r| r.windows(2))
            .filter_map(|w| match w {
                [a, b] => Some(segment_distance(point, *a, *b)),
                _ => None,
            })
            .fold(f64::INFINITY, f64::min)
    }
}
#[derive(Clone, Debug)]
pub struct Airspace {
    pub attribution: String,
    pub volumes: Vec<ProjectedAirspaceVolume>,
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct AirspaceMembership {
    pub id: String,
    pub class: Option<AirspaceClass>,
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct AirspaceBoundary {
    pub id: String,
    pub class: AirspaceClass,
    /// Zero underneath a shelf; otherwise distance to its lateral footprint.
    pub lateral_distance_m: f64,
    pub floor_msl_m: f64,
    /// Nonnegative height to the floor; zero alongside a shelf.
    pub floor_above_m: f64,
    pub warning: bool,
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct AirspaceQuery {
    pub containing: Vec<AirspaceVolume>,
    pub nearest: Option<AirspaceBoundary>,
}
impl Airspace {
    pub fn query(
        &self,
        north: f64,
        east: f64,
        altitude: f64,
        terrain: f64,
    ) -> Result<AirspaceQuery, TerrainError> {
        if [north, east, altitude, terrain]
            .iter()
            .any(|v| !v.is_finite() || v.abs() > 10_000_000.0)
        {
            return Err(TerrainError("invalid airspace query coordinate"));
        }
        let point = [north, east];
        let mut containing = Vec::new();
        let mut nearest: Option<(f64, AirspaceBoundary)> = None;
        for v in &self.volumes {
            let floor = v.info.floor.msl_m(terrain);
            let ceiling = v.info.ceiling.msl_m(terrain);
            let inside = v.contains_lateral(point);
            if inside && altitude >= floor && altitude < ceiling {
                containing.push(v.info.clone());
            }
            let Some(class @ (AirspaceClass::B | AirspaceClass::C)) = v.info.class else {
                continue;
            };
            if altitude >= ceiling || (inside && altitude >= floor) {
                continue;
            }
            let lateral = if inside { 0.0 } else { v.boundary_distance_m(point) };
            let vertical = (floor - altitude).max(0.0);
            let distance = libm::sqrt(lateral * lateral + vertical * vertical);
            let warning = lateral <= 1852.0 && vertical <= 300.0 * FOOT_M;
            if nearest
                .as_ref()
                .is_none_or(|(d, b)| distance < *d || (distance == *d && floor < b.floor_msl_m))
            {
                nearest = Some((
                    distance,
                    AirspaceBoundary {
                        id: v.info.id.clone(),
                        class,
                        lateral_distance_m: lateral,
                        floor_msl_m: floor,
                        floor_above_m: vertical,
                        warning,
                    },
                ));
            }
        }
        containing.sort_by_key(|v| (v.kind == AirspaceKind::ModeC, v.class, v.id.clone()));
        Ok(AirspaceQuery {
            containing,
            nearest: nearest.map(|(_, b)| b),
        })
    }
}
fn in_ring(p: [f64; 2], ring: &[[f64; 2]]) -> bool {
    let [n, e] = p;
    let mut inside = false;
    for w in ring.windows(2) {
        let [a, b] = w else {
            continue;
        };
        if segment_distance(p, *a, *b) <= 1e-7 {
            return true;
        }
        if (a[1] > e) != (b[1] > e) && n < (b[0] - a[0]) * (e - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
    }
    inside
}
pub fn segment_distance(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let [dn, de] = [b[0] - a[0], b[1] - a[1]];
    let length2 = dn * dn + de * de;
    let t = if length2 > 0.0 {
        ((p[0] - a[0]) * dn + (p[1] - a[1]) * de) / length2
    } else {
        0.0
    }
    .clamp(0.0, 1.0);
    libm::hypot(p[0] - a[0] - t * dn, p[1] - a[1] - t * de)
}
impl World {
    pub fn airspace_at(&self, north: f64, east: f64, altitude: f64) -> Result<AirspaceQuery, TerrainError> {
        if [north, east, altitude]
            .iter()
            .any(|v| !v.is_finite() || v.abs() > 10_000_000.0)
        {
            return Err(TerrainError("invalid airspace query coordinate"));
        }
        if let Some(airspace) = self.airspace() {
            airspace.query(north, east, altitude, self.ground(north, east).height_msl_m)
        } else {
            Ok(AirspaceQuery {
                containing: Vec::new(),
                nearest: None,
            })
        }
    }
}
