//! Independent vertical acceptance: official NGS separation and FAA runway elevation.
use super::{InputKind, Request, Sources, dem::Dems, geoid};
use flightcraft_world::{geo::LocalProjection, geoid::GeoidGrid, scenery};
use serde::Serialize;
#[derive(Debug, Serialize)]
pub(super) struct Audit {
    pub passed: bool,
    pub dem_navd88_meters_provenance: bool,
    pub geoid_source_sha256: Option<String>,
    pub ngs_separation_m: Option<f64>,
    pub grid_separation_m: Option<f64>,
    pub ngs_residual_m: Option<f64>,
    pub ngs_stated_error_m: Option<f64>,
    pub interpolation_tolerance_m: f64,
    pub faa_field_elevation_ft: f64,
    pub runway_max_msl_m: Option<f64>,
    pub runway_max_lat_lon: Option<[f64; 2]>,
    pub faa_residual_m: Option<f64>,
    pub notes: Vec<String>,
}
fn navd88_provenance(request: &Request, sources: &Sources) -> Result<bool, String> {
    let mut verified = std::collections::BTreeSet::new();
    for input in sources
        .inputs
        .iter()
        .filter(|i| i.kind == InputKind::DemDiscovery)
    {
        let value: serde_json::Value =
            serde_json::from_slice(&super::input_bytes(request, input)?).map_err(|e| e.to_string())?;
        if let Some(items) = value.get("items").and_then(|v| v.as_array()) {
            for item in items {
                let body = item
                    .get("body")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if body.contains("navd88")
                    && body.contains("elevation values are in meters")
                    && let Some(url) = item.get("downloadURL").and_then(|v| v.as_str())
                {
                    verified.insert(url.to_owned());
                }
            }
        }
    }
    let dems: Vec<_> = sources
        .inputs
        .iter()
        .filter(|i| i.kind == InputKind::Dem)
        .collect();
    Ok(!dems.is_empty() && dems.iter().all(|i| verified.contains(&i.source.url)))
}
pub(super) fn ngs_height(bytes: &[u8], lat: f64, lon: f64) -> Result<f64, String> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if value.get("geoidModel").and_then(|v| v.as_str()) != Some("GEOID18") {
        return Err("NGS control must identify GEOID18; an error response is not a control".into());
    }
    if !matches!(
        value.get("units").and_then(|v| v.as_str()),
        Some("m" | "meters" | "metres")
    ) {
        return Err("NGS control must state meter units".into());
    }
    for (key, expected) in [("lat", lat), ("lon", lon)] {
        if let Some(got) = value.get(key)
            && got
                .as_f64()
                .is_none_or(|v| !v.is_finite() || (v - expected).abs() > 1e-7)
        {
            return Err("NGS control coordinate mismatch".into());
        }
    }
    let height = value
        .get("geoidHeight")
        .and_then(|v| v.as_f64())
        .ok_or("NGS control lacks geoidHeight")?;
    if !height.is_finite() || height.abs() > 120. {
        return Err("NGS control separation outside range".into());
    }
    ngs_error(bytes)?;
    Ok(height)
}
fn ngs_error(bytes: &[u8]) -> Result<f64, String> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    value
        .get("error")
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite() && (0.0..=0.1).contains(v))
        .ok_or_else(|| "NGS control needs a bounded numeric uncertainty in meters".into())
}
fn runway_max(dem: &mut Dems) -> Result<(f64, [f64; 2]), String> {
    let projection =
        LocalProjection::new(40.96905613156273, -75.00242278301268).map_err(|e| e.to_string())?;
    let [north, east] = projection
        .project(40.973226688232224, -74.99258941913907)
        .map_err(|e| e.to_string())?;
    let length = libm::hypot(north, east);
    let steps = libm::ceil(length) as u32;
    let mut maximum = f64::NEG_INFINITY;
    let mut location = [0.; 2];
    // Sample the paved interior, avoiding vegetation/ditches outside the runway footprint.
    for along in 0..=steps {
        for across in -9..=9 {
            let fraction = f64::from(along) / f64::from(steps);
            let cross = f64::from(across) / 9. * 21.336 * 0.4;
            let [lat, lon] = projection
                .unproject(
                    north * fraction - east / length * cross,
                    east * fraction + north / length * cross,
                )
                .map_err(|e| e.to_string())?;
            let height = dem.height(lat, lon)?;
            if height > maximum {
                maximum = height;
                location = [lat, lon];
            }
        }
    }
    Ok((maximum, location))
}
pub(super) fn evaluate(
    request: &Request,
    sources: &Sources,
    grid: Option<&GeoidGrid>,
    dem: Option<&mut Dems>,
) -> Result<Audit, String> {
    let mut audit = Audit {
        passed: false,
        dem_navd88_meters_provenance: false,
        geoid_source_sha256: grid.map(|g| g.source_sha256.clone()),
        ngs_separation_m: None,
        grid_separation_m: None,
        ngs_residual_m: None,
        ngs_stated_error_m: None,
        interpolation_tolerance_m: 0.005,
        faa_field_elevation_ft: 371.,
        runway_max_msl_m: None,
        runway_max_lat_lon: None,
        faa_residual_m: None,
        notes: Vec::new(),
    };
    if request.sample.is_some() {
        audit
            .notes
            .push("Reference-image preview is not eligible for vertical acceptance".into());
        return Ok(audit);
    }
    audit.dem_navd88_meters_provenance = navd88_provenance(request, sources)?;
    if !audit.dem_navd88_meters_provenance {
        audit
            .notes
            .push("Receipt-backed USGS NAVD88 meter provenance is incomplete".into());
    }
    if matches!(request.area.as_str(), "1n7-test" | "1n7") {
        if let Some(dem) = dem {
            let (height, location) = runway_max(dem)?;
            audit.runway_max_msl_m = Some(height);
            audit.runway_max_lat_lon = Some(location);
            audit.faa_residual_m = Some((height - 371. * 0.3048).abs());
            audit.notes.push("FAA reference: 1N7 field elevation 371 ft, supplied by flight lead; airport elevation is the highest landing surface, not the ARP".into());
        }
    } else {
        audit
            .notes
            .push("No independent FAA vertical control registered for this area".into());
    }
    let [w, s, e, n] = sources.bounds;
    let [south, west] = scenery::inverse_mercator(w, s).map_err(|e| e.to_string())?;
    let [north, east] = scenery::inverse_mercator(e, n).map_err(|e| e.to_string())?;
    let (lat, lon) = ((south + north) / 2., (west + east) / 2.);
    let expected_url = geoid::control_url(lat, lon);
    let control = sources
        .inputs
        .iter()
        .find(|i| i.kind == InputKind::GeoidControl && i.source.url == expected_url);
    if let (Some(grid), Some(control)) = (grid, control) {
        let bytes = super::input_bytes(request, control)?;
        match ngs_height(&bytes, lat, lon) {
            Ok(ngs) => {
                let modeled = grid.separation_m(lat, lon).map_err(|e| e.to_string())?;
                audit.ngs_separation_m = Some(ngs);
                audit.grid_separation_m = Some(modeled);
                audit.ngs_residual_m = Some((ngs - modeled).abs());
                audit.ngs_stated_error_m = Some(ngs_error(&bytes)?);
            }
            Err(error) => audit.notes.push(error),
        }
    } else {
        audit
            .notes
            .push("Authentic GEOID18 grid and separately fetched NGS control are both required".into());
    }
    audit.passed = audit.dem_navd88_meters_provenance
        && audit
            .ngs_residual_m
            .zip(audit.ngs_stated_error_m)
            .is_some_and(|(v, error)| v <= error + audit.interpolation_tolerance_m)
        && audit.faa_residual_m.is_some_and(|v| v <= 1.);
    Ok(audit)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn live_1n7_control_agrees_with_the_official_grid_samples() {
        let bytes = include_bytes!("fixtures/ngs-geoid18-1n7.json");
        let ngs = ngs_height(bytes, 40.971132908, -74.997504056).unwrap();
        assert_eq!(ngs, -33.197);
        let grid = GeoidGrid {
            model: flightcraft_world::geoid::GeoidModel::NgsGeoid18,
            source_sha256: "66cf840fc1e7b01e8231f3588cab499d0ae72756fa0668db38caad9a3014dc91".into(),
            south_deg: 24. + 1018. / 60.,
            west_deg: -75.,
            step_deg: [1. / 60.; 2],
            rows: 2,
            cols: 2,
            undulation_m: vec![-33.207, -33.1851, -33.1726, -33.1574],
        };
        let residual = (grid.separation_m(40.971132908, -74.997504056).unwrap() - ngs).abs();
        assert!((residual - 0.002228717).abs() < 0.00001);
        assert!(residual <= ngs_error(bytes).unwrap() + 0.005);
    }
    #[test]
    fn ngs_control_rejects_wrong_units_wrong_location_and_service_error_responses() {
        let good = br#"{"geoidModel":"GEOID18","lat":40.97,"lon":-75.0,"units":"m","geoidHeight":-31.125,"error":0.004}"#;
        assert_eq!(ngs_height(good, 40.97, -75.).unwrap(), -31.125);
        assert!(ngs_height(good, 40.98, -75.).is_err());
        for bad in [
            br#"{"error":"No suitable Geoid model found for model 18"}"#.as_slice(),
            br#"{"geoidModel":"GEOID12B","units":"m","geoidHeight":-31.125}"#,
            br#"{"units":"m","geoidHeight":-31.125}"#,
            br#"{"geoidModel":"GEOID18","units":"ft","geoidHeight":-31.125}"#.as_slice(),
            br#"{"geoidModel":"GEOID18","units":"m","error":"service unavailable"}"#,
            br#"{"geoidModel":"GEOID18","units":"m","geoidHeight":999}"#,
        ] {
            assert!(ngs_height(bad, 40.97, -75.).is_err());
        }
    }
}
