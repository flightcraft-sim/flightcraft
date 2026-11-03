//! Bounded public airport catalogues and projected, sloped runway surfaces.
use crate::{Ground, Region, Surface, TerrainError, geo::LocalProjection};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
pub const MAX_AIRPORT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_RUNWAYS: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunwaySurface {
    Asphalt,
    Concrete,
    Turf,
    Gravel,
    Water,
    Other,
}
impl RunwaySurface {
    pub fn ground_surface(self) -> Surface {
        match self {
            Self::Asphalt | Self::Concrete => Surface::Pavement,
            Self::Water => Surface::Water,
            _ => Surface::Grass,
        }
    }
}
/// Published fixture metadata when supplied by the airport source. None means unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApproachLights {
    None,
    Alsf2,
    Malsr,
    Odals,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VisualGuidance {
    Papi,
    Vasi,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunwayLighting {
    pub approach: Option<ApproachLights>,
    pub visual_guidance: Option<VisualGuidance>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunwayEnd {
    pub ident: String,
    pub latitude_deg: f64,
    pub longitude_deg: f64,
    pub elevation_m: f64,
    pub heading_deg: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lighting: Option<RunwayLighting>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AirportRunway {
    pub designator: String,
    pub length_m: f64,
    pub width_m: f64,
    pub surface: RunwaySurface,
    pub ends: [RunwayEnd; 2],
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Airport {
    pub ident: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icao: Option<String>,
    pub name: String,
    pub elevation_m: f64,
    /// Published traffic-pattern altitude, feet MSL; absence uses the crew fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traffic_pattern_altitude_ft: Option<u32>,
    pub latitude_deg: f64,
    pub longitude_deg: f64,
    pub runways: Vec<AirportRunway>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Airports {
    pub attribution: String,
    pub airports: Vec<Airport>,
}
fn text(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}
fn ident(s: &str) -> bool {
    text(s, 16) && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
}
fn elevation(v: f64) -> bool {
    v.is_finite() && (-500.0..=9000.0).contains(&v)
}
impl Airports {
    pub fn from_json(bytes: &[u8], region: &Region) -> Result<Self, TerrainError> {
        if bytes.len() > MAX_AIRPORT_BYTES {
            return Err(TerrainError("airport catalogue too large"));
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| TerrainError("invalid airports JSON"))?;
        value.validate(region)?;
        Ok(value)
    }
    pub fn validate(&self, region: &Region) -> Result<(), TerrainError> {
        region.validate()?;
        if !text(&self.attribution, 2048) || self.airports.len() > MAX_RUNWAYS {
            return Err(TerrainError("airport catalogue bounds"));
        }
        let projection = LocalProjection::new(region.origin.latitude_deg, region.origin.longitude_deg)?;
        let mut count = 0;
        let mut names = std::collections::BTreeSet::new();
        for airport in &self.airports {
            if !ident(&airport.ident)
                || !names.insert(&airport.ident)
                || airport.icao.as_deref().is_some_and(|s| !ident(s))
                || !text(&airport.name, 256)
                || !elevation(airport.elevation_m)
                || airport.traffic_pattern_altitude_ft.is_some_and(|a| a > 30000)
            {
                return Err(TerrainError("invalid or duplicate airport"));
            }
            inside(
                region,
                projection.project(airport.latitude_deg, airport.longitude_deg)?,
            )?;
            let mut designators = std::collections::BTreeSet::new();
            for runway in &airport.runways {
                count += 1;
                let [a, b] = &runway.ends;
                if a.ident == b.ident
                    || count > MAX_RUNWAYS
                    || !text(&runway.designator, 35)
                    || runway.designator != format!("{}/{}", a.ident, b.ident)
                    || !designators.insert(&runway.designator)
                    || !runway.length_m.is_finite()
                    || !(50.0..=10000.0).contains(&runway.length_m)
                    || !runway.width_m.is_finite()
                    || !(1.0..=200.0).contains(&runway.width_m)
                {
                    return Err(TerrainError("invalid runway dimensions or designator"));
                }
                for end in &runway.ends {
                    if !end_ident(&end.ident)
                        || !elevation(end.elevation_m)
                        || !end.heading_deg.is_finite()
                        || !(0.0..360.0).contains(&end.heading_deg)
                    {
                        return Err(TerrainError("invalid runway end"));
                    }
                    inside(region, projection.project(end.latitude_deg, end.longitude_deg)?)?;
                }
                let number = |s: &str| s.get(..2).and_then(|s| s.parse::<u8>().ok());
                if number(&a.ident).map(|n| (n + 17) % 36 + 1) != number(&b.ident) {
                    return Err(TerrainError("runway end numbers must be reciprocal"));
                }
                for (start, finish) in [(a, b), (b, a)] {
                    let phi = start.latitude_deg.to_radians();
                    let phi1 = finish.latitude_deg.to_radians();
                    let delta = (finish.longitude_deg - start.longitude_deg).to_radians();
                    let bearing = libm::atan2(
                        libm::sin(delta) * libm::cos(phi1),
                        libm::cos(phi) * libm::sin(phi1)
                            - libm::sin(phi) * libm::cos(phi1) * libm::cos(delta),
                    )
                    .to_degrees();
                    if ((bearing - start.heading_deg + 180.0).rem_euclid(360.0) - 180.0).abs() > 2.0 {
                        return Err(TerrainError("runway heading disagrees with centreline"));
                    }
                }
                let [an, ae] = projection.project(a.latitude_deg, a.longitude_deg)?;
                let [bn, be] = projection.project(b.latitude_deg, b.longitude_deg)?;
                let length = libm::hypot(bn - an, be - ae);
                let (dn, de) = (
                    -(be - ae) / length * runway.width_m * 0.5,
                    (bn - an) / length * runway.width_m * 0.5,
                );
                for [n, e] in [
                    [an + dn, ae + de],
                    [an - dn, ae - de],
                    [bn + dn, be + de],
                    [bn - dn, be - de],
                ] {
                    inside(region, [n, e])?;
                }
                if (length / runway.length_m - 1.0).abs() > 0.05
                    || (b.elevation_m - a.elevation_m).abs() / length > 0.1
                {
                    return Err(TerrainError(
                        "runway centreline length or slope disagrees with dimensions",
                    ));
                }
            }
        }
        Ok(())
    }
}
pub fn end_ident(s: &str) -> bool {
    let bytes = s.as_bytes();
    if !(2..=3).contains(&bytes.len()) {
        return false;
    }
    let Some(number) = s.get(..2).and_then(|s| s.parse::<u8>().ok()) else {
        return false;
    };
    (1..=36).contains(&number)
        && (bytes.len() == 2 || matches!(bytes.get(2), Some(b'L' | b'R' | b'C' | b'G')))
}
pub fn inside(region: &Region, [n, e]: [f64; 2]) -> Result<(), TerrainError> {
    let [sn, se] = region.southwest_m;
    let span = f64::from(region.tile_size - 1) * f64::from(region.spacing_m);
    if n < sn
        || e < se
        || n > sn + span * f64::from(region.tiles_north)
        || e > se + span * f64::from(region.tiles_east)
    {
        return Err(TerrainError("airport or runway outside region grid"));
    }
    Ok(())
}
/// Projected surface. North/east endpoints, MSL heights, and a linear longitudinal grade.
#[derive(Clone, Debug)]
pub struct RunwayStrip {
    pub ends_m: [[f64; 2]; 2],
    pub elevations_m: [f64; 2],
    pub width_m: f64,
    pub surface: RunwaySurface,
    pub end_idents: [String; 2],
    pub lighting: [Option<RunwayLighting>; 2],
}
impl RunwayStrip {
    pub fn project(runway: &AirportRunway, p: &LocalProjection) -> Result<Self, TerrainError> {
        let [a, b] = &runway.ends;
        Ok(Self {
            ends_m: [
                p.project(a.latitude_deg, a.longitude_deg)?,
                p.project(b.latitude_deg, b.longitude_deg)?,
            ],
            elevations_m: [a.elevation_m, b.elevation_m],
            width_m: runway.width_m,
            surface: runway.surface,
            end_idents: [a.ident.clone(), b.ident.clone()],
            lighting: [a.lighting, b.lighting],
        })
    }
    pub fn near(&self, north: f64, east: f64, margin: f64) -> bool {
        let [[an, ae], [bn, be]] = self.ends_m;
        let pad = self.width_m * 0.5 + margin;
        north >= an.min(bn) - pad
            && north <= an.max(bn) + pad
            && east >= ae.min(be) - pad
            && east <= ae.max(be) + pad
    }
    pub fn length_m(&self) -> f64 {
        let [[an, ae], [bn, be]] = self.ends_m;
        libm::hypot(bn - an, be - ae)
    }
    /// Signed rectangle distance and the runway plane at the query position.
    pub fn plane(&self, north: f64, east: f64) -> (f64, Ground) {
        let [[an, ae], [bn, be]] = self.ends_m;
        let [ha, hb] = self.elevations_m;
        let length = self.length_m();
        let (c, s) = ((bn - an) / length, (be - ae) / length);
        let (dn, de) = (north - an, east - ae);
        let along = dn * c + de * s;
        let across = -dn * s + de * c;
        let slope = (hb - ha) / length;
        let norm = libm::sqrt(1.0 + slope * slope);
        (
            (-along)
                .max(along - length)
                .max(across.abs() - self.width_m * 0.5),
            Ground {
                height_msl_m: ha + along * slope,
                normal_ned: [-slope * c / norm, -slope * s / norm, -1.0 / norm],
                surface: self.surface.ground_surface(),
            },
        )
    }
}
