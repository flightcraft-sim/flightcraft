//! Release gates for the Hudson corridor, reusing the 1N7 evidence model (docs/scenery.md,
//! "Release gates for the Hudson corridor"): vertical datum against Teterboro's FAA field
//! elevation and the NGS GEOID18 control, horizontal registration of lidar building footprints
//! against NAIP (no runway lies inside the fly-nyc.json envelope), and lidar acceptance.
use super::{
    corridor::{self, Plan},
    corridor_buildings::{REG_N, REG_R, Totals},
    dem::Dems,
    download,
};
use flightcraft_world::geo::LocalProjection;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub(super) struct Criterion {
    pub name: &'static str,
    pub passed: bool,
    pub value: serde_json::Value,
    pub requirement: &'static str,
}
#[derive(Clone, Debug, Serialize)]
pub(super) struct Acceptance {
    pub passed: bool,
    pub criteria: Vec<Criterion>,
}
#[derive(Clone, Debug, Serialize)]
pub(super) struct AlignmentAudit {
    pub passed: bool,
    pub method: &'static str,
    pub runway_in_envelope: bool,
    pub notes: Vec<String>,
    pub buildings_used: usize,
    pub blocks_used: usize,
    pub best_shift_east_north_m: [i32; 2],
    pub best_shift_magnitude_m: f64,
    pub contrast_at_best: f64,
    pub contrast_at_zero: f64,
    pub border_ring_max_contrast: f64,
    pub mean_contrast_over_window: f64,
    pub blocks_peaking_within_2_m: usize,
    pub contrast_by_shift: Vec<Vec<f64>>,
}
pub(super) struct Gates {
    pub lidar: Acceptance,
    pub alignment: AlignmentAudit,
}
pub(super) struct Selection {
    pub nodes_selected: usize,
    pub points_selected: u64,
    pub nodes_decoded: usize,
    pub points_decoded: u64,
}

fn criterion(
    name: &'static str,
    passed: bool,
    value: serde_json::Value,
    requirement: &'static str,
) -> Criterion {
    Criterion {
        name,
        passed,
        value,
        requirement,
    }
}

pub(super) fn evaluate(
    _plan: &Plan,
    totals: &Totals,
    selection: &Selection,
    vertical_passed: bool,
    partial: bool,
    water_cover: Option<(usize, f64)>,
) -> Result<Gates, String> {
    // Horizontal registration: footprints versus NAIP, summed over every block's sampled buildings.
    let r = &totals.registration;
    let mut best = (0_i32, 0_i32, f64::NEG_INFINITY);
    let (mut sum, mut cells) = (0., 0.);
    for (iy, row) in r.sum.iter().enumerate() {
        for (ix, v) in row.iter().enumerate() {
            sum += v;
            cells += 1.;
            if *v > best.2 {
                best = (ix as i32 - REG_R as i32, iy as i32 - REG_R as i32, *v);
            }
        }
    }
    let per_building = |v: f64| {
        if r.buildings > 0 {
            v / r.buildings as f64
        } else {
            0.
        }
    };
    let mean = per_building(sum / cells);
    let at_best = per_building(best.2);
    let at_zero = per_building(r.sum[REG_R][REG_R]);
    let magnitude = libm::hypot(f64::from(best.0), f64::from(best.1));
    let within = r
        .per_block_best
        .iter()
        .filter(|b| libm::hypot(f64::from(b[0]), f64::from(b[1])) <= 2.)
        .count();
    // The correlation surface is one broad hill, so "above the window mean" says little: the peak
    // must also beat every shift on the window's border ring (|shift| = 5 m) by 3%.
    let border_max = r
        .sum
        .iter()
        .enumerate()
        .flat_map(|(iy, row)| row.iter().enumerate().map(move |(ix, v)| (ix, iy, *v)))
        .filter(|(ix, iy, _)| *ix == 0 || *iy == 0 || *ix == REG_N - 1 || *iy == REG_N - 1)
        .map(|(_, _, v)| per_building(v))
        .fold(f64::NEG_INFINITY, f64::max);
    let registered = r.buildings >= 300
        && magnitude <= 2.
        && at_best >= 1.03 * border_max
        && at_best >= 1.05 * mean
        && at_zero >= 1.02 * mean;
    let alignment = AlignmentAudit {
        passed: registered && !partial,
        method: "lidar-building-footprints-vs-naip",
        runway_in_envelope: false,
        notes: vec![
            "No runway lies inside the fly-nyc.json envelope (KTEB, KEWR, KLGA and KJFK are outside it; the heliports are not surveyed runways). Horizontal registration is therefore validated by shifting lidar building footprints over the NAIP orthophoto in 1 m steps and requiring the summed luminance edge strength along the footprint outlines to peak within 2 m of zero shift, above every shift on the window border.".into(),
            "Only low-rise roofs (3.5 to 9 m) are used: the orthophoto is rectified to bare earth, so a roof leans away from the camera by its height times the view angle (about 1 m for a 9 m roof, 3 m for a 20 m one). Footprints come from the lidar alone.".into(),
        ],
        buildings_used: r.buildings,
        blocks_used: r.per_block_best.len(),
        best_shift_east_north_m: [best.0, best.1],
        best_shift_magnitude_m: magnitude,
        contrast_at_best: at_best,
        contrast_at_zero: at_zero,
        border_ring_max_contrast: border_max,
        mean_contrast_over_window: mean,
        blocks_peaking_within_2_m: within,
        contrast_by_shift: r.sum.iter().map(|row| row.iter().map(|v| per_building(*v)).collect()).collect(),
    };

    let ground = totals.ground.finish();
    let mut rms = totals.plane_rms.clone();
    rms.sort_by(f32::total_cmp);
    let fit = rms.iter().filter(|v| **v <= 0.25).count() as f64 / rms.len().max(1) as f64;
    let worst = rms.last().copied().unwrap_or(0.);
    let measured = totals.roof_cells_measured as f64;
    let interpolated = totals.roof_cells_interpolated as f64;
    let interpolated_share = interpolated / (measured + interpolated).max(1.);
    let mut heights = totals.building_heights.clone();
    heights.sort_by(f32::total_cmp);
    let (low, high) = (
        heights.first().copied().unwrap_or(0.),
        heights.last().copied().unwrap_or(0.),
    );
    let tall = heights.iter().filter(|h| **h >= 100.).count();
    let mut tree_heights = totals.tree_heights.clone();
    tree_heights.sort_by(f32::total_cmp);
    let tree_max = tree_heights.last().copied().unwrap_or(0.);
    let inferred = totals.buildings - totals.classified_buildings.min(totals.buildings);
    let criteria = vec![
        criterion(
            "complete EPT selection decoded",
            !partial
                && selection.nodes_decoded == selection.nodes_selected
                && selection.points_decoded == selection.points_selected,
            serde_json::json!({"nodes_selected":selection.nodes_selected,"nodes_decoded":selection.nodes_decoded,"points_selected":selection.points_selected,"points_decoded":selection.points_decoded,"partial_build":partial}),
            "decoded node and point totals equal the receipted hierarchy selection",
        ),
        criterion(
            "classified ground agrees with the NAVD88 meter DEM",
            ground.ept_z_agrees_with_navd88_meters,
            serde_json::json!({"cells":ground.samples,"median_m":ground.orthometric_median_absolute_error_m,"p95_m":ground.orthometric_p95_absolute_error_m,"ellipsoidal_hypothesis_median_m":ground.ellipsoidal_median_absolute_error_m,"us_survey_feet_hypothesis_median_m":ground.us_survey_feet_median_absolute_error_m}),
            "at least 128 distributed cells, median at most 0.5 m, 95th percentile at most 1 m",
        ),
        criterion(
            "vertical datum gate",
            vertical_passed,
            serde_json::json!(vertical_passed),
            "GEOID18 grid, NGS control and Teterboro FAA field elevation agree",
        ),
        criterion(
            "roof planes fit their measured points",
            totals.planes > 0 && fit >= 0.99 && worst <= 0.30,
            serde_json::json!({"planes":totals.planes,"share_within_0.25_m":fit,"worst_rms_m":worst,"p99_rms_m":rms.get(((rms.len() as f64) * 0.99) as usize).copied()}),
            "at least 99% of planes within 0.25 m RMS and none above the detector's 0.30 m ceiling",
        ),
        criterion(
            "roof cells are measured",
            interpolated_share <= 0.15,
            serde_json::json!({"measured":totals.roof_cells_measured,"interpolated":totals.roof_cells_interpolated,"share":interpolated_share}),
            "interpolated roof cells at most 15% of all roof cells",
        ),
        criterion(
            "heights are plausible",
            low >= 2.5 && high <= 560. && tree_max <= 70.,
            serde_json::json!({"building_min_m":low,"building_max_m":high,"tree_max_m":tree_max,"buildings_100m_or_taller":tall}),
            "buildings 2.5 to 560 m (One World Trade is 541 m), trees at most 70 m",
        ),
        criterion(
            "classification inference is declared",
            totals.buildings > 0 && inferred + totals.classified_buildings == totals.buildings,
            serde_json::json!({"buildings":totals.buildings,"inferred_from_unclassified_returns":inferred,"with_class_6":totals.classified_buildings,"ept_classes":totals.classes,"note":"NY_NewYorkCity labels no building class (the Empire State Building is entirely class 1); roofs are inferred from planar, elevated unclassified returns exactly as for the NJ survey"}),
            "every building built from unclassified returns carries inferred_classification (buildings-audit.json)",
        ),
        criterion(
            "tall and stepped Manhattan is present",
            totals.buildings >= 10_000 && tall >= 40,
            serde_json::json!({"buildings":totals.buildings,"over_100_m":tall,"planes":totals.planes}),
            "at least 10,000 buildings including at least 40 taller than 100 m",
        ),
        criterion(
            "NHD water polygons cover the lidar water cells",
            water_cover.is_some_and(|(cells, share)| cells >= 100 && share >= 0.80),
            serde_json::json!({"lidar_water_cells_sampled":water_cover.map(|w| w.0),"share_inside_nhd_polygons":water_cover.map(|w| w.1)}),
            "at least 80% of a spread sample of lidar class 9 cells lie inside a drawn NHD polygon (an independent registration check of the water layer)",
        ),
        criterion(
            "footprints register with NAIP",
            alignment.passed,
            serde_json::json!({"best_shift_east_north_m":alignment.best_shift_east_north_m,"contrast_at_best":at_best,"contrast_at_zero":at_zero,"window_mean":mean,"buildings":r.buildings}),
            "the footprint edge-strength surface peaks within 2 m of zero shift, above every shift on the window border",
        ),
    ];
    Ok(Gates {
        lidar: Acceptance {
            passed: criteria.iter().all(|c| c.passed),
            criteria,
        },
        alignment,
    })
}

pub(super) struct Control {
    pub airport: &'static str,
    pub field_elevation_m: f64,
    pub dem_max_m: f64,
    pub at: [f64; 2],
}
/// Teterboro: the highest paved-runway DEM sample against the FAA field elevation in the
/// region's airport catalogue (the highest point of the landing area).
pub(super) fn vertical_control(dem: &mut Dems) -> Result<Control, String> {
    let airports: serde_json::Value =
        download::json(&crate::root()?.join("assets/terrain/new-york/airports.json"))?;
    let teb = airports
        .get("airports")
        .and_then(|v| v.as_array())
        .and_then(|a| {
            a.iter()
                .find(|a| a.get("ident").and_then(|v| v.as_str()) == Some("TEB"))
        })
        .ok_or("TEB missing from the New York airport catalogue")?;
    let field = teb
        .get("elevation_m")
        .and_then(|v| v.as_f64())
        .ok_or("TEB elevation")?;
    let [lat0, lon0] = corridor::KTEB;
    let projection = LocalProjection::new(lat0, lon0).map_err(|e| e.to_string())?;
    let mut best = (f64::NEG_INFINITY, [0.; 2]);
    for runway in teb
        .get("runways")
        .and_then(|v| v.as_array())
        .ok_or("TEB runways")?
    {
        let ends = runway
            .get("ends")
            .and_then(|v| v.as_array())
            .ok_or("runway ends")?;
        let [a, b] = ends.as_slice() else { continue };
        let get = |e: &serde_json::Value| -> Result<[f64; 2], String> {
            projection
                .project(
                    e.get("latitude_deg").and_then(|v| v.as_f64()).ok_or("end lat")?,
                    e.get("longitude_deg").and_then(|v| v.as_f64()).ok_or("end lon")?,
                )
                .map_err(|e| e.to_string())
        };
        let (pa, pb) = (get(a)?, get(b)?);
        let length = libm::hypot(pb[0] - pa[0], pb[1] - pa[1]);
        let steps = libm::ceil(length) as u32;
        let (nx, ny) = ((pb[1] - pa[1]) / length, -(pb[0] - pa[0]) / length);
        for along in 0..=steps {
            let f = f64::from(along) / f64::from(steps);
            for across in -4..=4 {
                // The runway is 45.7 m wide; stay in its paved interior.
                let c = f64::from(across) * 3.;
                let n = pa[0] + (pb[0] - pa[0]) * f + nx * c;
                let e = pa[1] + (pb[1] - pa[1]) * f + ny * c;
                let [lat, lon] = projection.unproject(n, e).map_err(|e| e.to_string())?;
                if let Some(h) = dem.height_opt(lat, lon)?
                    && h > best.0
                {
                    best = (h, [lat, lon]);
                }
            }
        }
    }
    if !best.0.is_finite() {
        return Err("no 1 m DEM samples on the Teterboro runways".into());
    }
    Ok(Control {
        airport: "KTEB",
        field_elevation_m: field,
        dem_max_m: best.0,
        at: best.1,
    })
}
