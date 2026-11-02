//! Published communications facilities; antenna and service limits are simulator estimates.
use crate::{GeoPoint, TerrainError, navdata::text_ok};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FacilityKind {
    Ctaf,
    Unicom,
    Awos,
    Asos,
    Atis,
    Tower,
    Ground,
    Approach,
    Departure,
    Clearance,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComMetadata {
    pub provider: String,
    pub callsign: String,
    pub hours: String,
    pub sector: String,
    pub services: Vec<String>,
    pub effective_date: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub published_use: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weather_sensor: Option<String>,
    /// NASR gives a facility/airport reference point, not necessarily a remote antenna site.
    pub position_estimated: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComStation {
    pub id: String,
    pub airport: String,
    pub name: String,
    pub kind: FacilityKind,
    /// Channel designator in kHz, not the rounded 8.33 kHz carrier.
    pub channel_khz: u32,
    pub position: GeoPoint,
    pub elevation_m: f64,
    /// Engineering estimate, not an FAA-published antenna height.
    pub antenna_agl_m: f64,
    pub range_nm: f64,
    /// FAA source URL or file/record identifier, retained by the importer.
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ComMetadata>,
}
impl ComStation {
    pub fn validate(&self) -> Result<(), TerrainError> {
        self.position.validate()?;
        if !text_ok(&self.id, 128)
            || !text_ok(&self.airport, 16)
            || !text_ok(&self.name, 256)
            || !text_ok(&self.source, 2048)
            || !valid_channel(self.channel_khz, true)
            || !self.elevation_m.is_finite()
            || !(-500.0..=9000.0).contains(&self.elevation_m)
            || !self.antenna_agl_m.is_finite()
            || !(1.0..=300.0).contains(&self.antenna_agl_m)
            || !self.range_nm.is_finite()
            || !(1.0..=300.0).contains(&self.range_nm)
        {
            return Err(TerrainError("invalid communications station"));
        }
        if self.metadata.as_ref().is_some_and(|m| {
            !text_ok(&m.provider, 16)
                || m.callsign.len() > 256
                || m.hours.len() > 2048
                || m.sector.len() > 4096
                || m.services.len() > 64
                || m.services.iter().any(|s| !text_ok(s, 256))
                || !crate::navdata::iso_date(&m.effective_date)
                || m.published_use.len() > 4096
                || m.weather_sensor.as_ref().is_some_and(|v| !text_ok(v, 64))
        }) {
            return Err(TerrainError("invalid communications metadata"));
        }
        Ok(())
    }
}
/// ICAO 8.33 channel labels repeat at 100 kHz; .020/.045/.070/.095 are not channels.
/// .000/.025/.050/.075 select the corresponding 25 kHz carrier.
pub fn valid_channel(khz: u32, eight_third: bool) -> bool {
    (118000..=136990).contains(&khz)
        && if eight_third {
            khz.is_multiple_of(5) && !matches!(khz % 100, 20 | 45 | 70 | 95)
        } else {
            khz.is_multiple_of(25)
        }
}
/// Integer carrier in thirds of a hertz avoids floating point frequency comparisons.
pub fn carrier_thirds_hz(khz: u32) -> Option<u32> {
    if !valid_channel(khz, true) {
        return None;
    }
    let base = khz / 25 * 25;
    let offset = match khz % 25 {
        0 | 5 => 0,
        10 => 25000,
        15 => 50000,
        _ => return None,
    };
    Some(base * 3000 + offset)
}
