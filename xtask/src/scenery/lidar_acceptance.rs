//! Lidar acceptance, as defined in docs/scenery.md ("Lidar acceptance").
//!
//! `lidarBuilt` alone only says reconstruction ran. Acceptance additionally requires, for the
//! New Jersey survey whose returns carry no vegetation or building labels:
//! 1. the completed EPT selection decoded exactly (node and point totals match the receipts);
//! 2. classified ground agrees with the independent NAVD88 meter DEM (at least 128 distributed
//!    cells, median at most 0.5 m, 95th percentile at most 1 m) and the vertical datum passed;
//! 3. the reconstruction is geometrically sound: measured roof planes fit within 0.25 m RMS, at
//!    most 15% of roof cells are interpolated, building and tree heights are physically plausible,
//!    and records created from unclassified returns say so;
//! 4. no tree or building centre falls inside the FAA runway rectangle;
//! 5. the lidar footprints register with the independent NAIP imagery within 2 m (a building
//!    footprint search over a metre-spaced shift window must peak near zero shift).
use super::{
    alignment::{Frame, Mosaic, RunwayReference},
    buildings::Building,
    surface::Grid,
    trees::Tree,
};
use serde::Serialize;
use std::collections::BTreeMap;

pub const REGISTRATION_GATE_M: f64 = 2.0;
const MIN_REGISTRATION_CELLS: usize = 150;

#[derive(Clone, Debug, Serialize)]
pub struct Criterion {
    pub name: &'static str,
    pub passed: bool,
    pub value: serde_json::Value,
    pub requirement: &'static str,
}
#[derive(Clone, Debug, Serialize)]
pub struct Registration {
    pub buildings_used: usize,
    /// Best-matching shift of the lidar footprints over the NAIP imagery, metres east/north.
    pub best_shift_east_north_m: [f64; 2],
    pub contrast_at_best: f64,
    pub contrast_at_zero: f64,
    pub mean_contrast_over_window: f64,
}
#[derive(Clone, Debug, Serialize)]
pub struct Acceptance {
    pub passed: bool,
    pub criteria: Vec<Criterion>,
    pub registration: Option<Registration>,
}
pub struct Inputs<'a> {
    pub complete_selection: bool,
    pub ground_samples: usize,
    pub ground_median_m: Option<f64>,
    pub ground_p95_m: Option<f64>,
    pub vertical_datum_passed: bool,
    pub classes: &'a BTreeMap<u8, u64>,
    pub buildings: &'a [Building],
    pub trees: &'a [Tree],
    pub grid: &'a Grid,
    pub mosaic: Option<&'a Mosaic>,
    pub runway: Option<&'a RunwayReference>,
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

fn colour_distance(a: [f32; 3], b: [f32; 3]) -> f64 {
    f64::from(libm::sqrtf(
        (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2),
    ))
}
/// Mean colour inside the footprint versus a ring three cells outside it, at a given shift.
fn contrast(
    mosaic: &Mosaic,
    grid: &Grid,
    building: &Building,
    shift_m: [f64; 2],
    cos_lat: f64,
) -> Option<f64> {
    let [w, s, _, _] = grid.bounds;
    let (shift_x, shift_y) = (shift_m[0] / cos_lat, shift_m[1] / cos_lat);
    let sample = |col: i64, row: i64| -> Option<[f32; 3]> {
        let x = w + (col as f64 + 0.5) * grid.step + shift_x;
        let y = s + (row as f64 + 0.5) * grid.step + shift_y;
        mosaic.sample_mercator(x, y)
    };
    let (mut inside, mut outside) = ([0_f32; 3], [0_f32; 3]);
    let (mut n_in, mut n_out) = (0_f32, 0_f32);
    for &index in building.cells.keys() {
        let (col, row) = ((index % grid.width) as i64, (index / grid.width) as i64);
        let p = sample(col, row)?;
        for (acc, v) in inside.iter_mut().zip(p) {
            *acc += v;
        }
        n_in += 1.;
        for (dc, dr) in [(-3_i64, 0_i64), (3, 0), (0, -3), (0, 3)] {
            let (c, r) = (col + dc, row + dr);
            let neighbour = usize::try_from(r * grid.width as i64 + c).ok();
            let inside_footprint =
                c >= 0 && c < grid.width as i64 && neighbour.is_some_and(|i| building.cells.contains_key(&i));
            if !inside_footprint && let Some(q) = sample(c, r) {
                for (acc, v) in outside.iter_mut().zip(q) {
                    *acc += v;
                }
                n_out += 1.;
            }
        }
    }
    if n_in < 1. || n_out < 1. {
        return None;
    }
    Some(colour_distance(
        inside.map(|v| v / n_in),
        outside.map(|v| v / n_out),
    ))
}
fn registration(mosaic: &Mosaic, grid: &Grid, buildings: &[Building], lat: f64) -> Option<Registration> {
    let large: Vec<&Building> = buildings
        .iter()
        .filter(|b| b.cells.len() >= MIN_REGISTRATION_CELLS)
        .collect();
    if large.len() < 3 {
        return None;
    }
    let cos_lat = libm::cos(lat.to_radians());
    let mut best: Option<([f64; 2], f64)> = None;
    let (mut zero, mut sum, mut count) = (0., 0., 0_u32);
    for dy in -4..=4 {
        for dx in -4..=4 {
            let shift = [f64::from(dx), f64::from(dy)];
            let mut total = 0.;
            let mut used = 0_u32;
            for b in &large {
                if let Some(c) = contrast(mosaic, grid, b, shift, cos_lat) {
                    total += c;
                    used += 1;
                }
            }
            if (used as usize) < large.len() {
                continue;
            }
            let score = total / f64::from(used);
            sum += score;
            count += 1;
            if dx == 0 && dy == 0 {
                zero = score;
            }
            if best.is_none_or(|(_, b)| score > b) {
                best = Some((shift, score));
            }
        }
    }
    let (shift, score) = best?;
    Some(Registration {
        buildings_used: large.len(),
        best_shift_east_north_m: shift,
        contrast_at_best: score,
        contrast_at_zero: zero,
        mean_contrast_over_window: sum / f64::from(count.max(1)),
    })
}

pub(super) fn evaluate(input: &Inputs) -> Acceptance {
    let mut criteria = Vec::new();
    criteria.push(criterion(
        "complete_ept_selection",
        input.complete_selection,
        serde_json::json!(input.complete_selection),
        "decoded nodes and points equal the receipted hierarchy selection",
    ));
    criteria.push(criterion(
        "ground_agrees_with_navd88_meter_dem",
        input.ground_samples >= 128
            && input.ground_median_m.is_some_and(|v| v <= 0.5)
            && input.ground_p95_m.is_some_and(|v| v <= 1.),
        serde_json::json!({"samples":input.ground_samples,"median_m":input.ground_median_m,"p95_m":input.ground_p95_m}),
        "at least 128 distributed cells, median <= 0.5 m, p95 <= 1 m",
    ));
    criteria.push(criterion(
        "vertical_datum_verified",
        input.vertical_datum_passed,
        serde_json::json!(input.vertical_datum_passed),
        "NGS GEOID18 control and FAA runway elevation agree",
    ));
    let planes: Vec<f64> = input
        .buildings
        .iter()
        .flat_map(|b| b.roof_planes.iter().map(|p| p.rms_m))
        .collect();
    let worst_rms = planes.iter().copied().fold(0., f64::max);
    criteria.push(criterion(
        "roof_planes_fit_measured_points",
        !input.buildings.is_empty() && !planes.is_empty() && worst_rms <= 0.25,
        serde_json::json!({"buildings":input.buildings.len(),"planes":planes.len(),"worst_rms_m":worst_rms}),
        "at least one building; every roof plane RMS <= 0.25 m",
    ));
    let (measured, interpolated) = input.buildings.iter().fold((0_usize, 0_usize), |(m, i), b| {
        (m + b.measured_roof_cells, i + b.interpolated_roof_cells)
    });
    let fraction = interpolated as f64 / (measured + interpolated).max(1) as f64;
    criteria.push(criterion(
        "roof_cells_are_measured",
        measured > 0 && fraction <= 0.15,
        serde_json::json!({"measured":measured,"interpolated":interpolated,"interpolated_fraction":fraction}),
        "interpolated roof cells <= 15% of all roof cells",
    ));
    let tallest_building = input.buildings.iter().map(|b| b.height_m).fold(0., f64::max);
    let shortest_building = input
        .buildings
        .iter()
        .map(|b| b.height_m)
        .fold(f64::INFINITY, f64::min);
    let tallest_tree = input.trees.iter().map(|t| t.height_m).fold(0., f64::max);
    criteria.push(criterion(
        "heights_are_plausible",
        !input.trees.is_empty()
            && (input.buildings.is_empty() || shortest_building >= 2.5)
            && tallest_building <= 40.
            && tallest_tree <= 45.,
        serde_json::json!({"tallest_building_m":tallest_building,"shortest_building_m":(shortest_building.is_finite()).then_some(shortest_building),"tallest_tree_m":tallest_tree,"trees":input.trees.len()}),
        "buildings 2.5-40 m, trees at most 45 m above the DEM, at least one tree",
    ));
    let labelled = [3_u8, 4, 5, 6]
        .iter()
        .any(|class| input.classes.get(class).is_some_and(|n| *n > 0));
    let unflagged = input
        .buildings
        .iter()
        .filter(|b| !b.inferred_classification)
        .count()
        + input.trees.iter().filter(|t| !t.inferred_classification).count();
    criteria.push(criterion(
        "inferred_classification_is_declared",
        labelled || unflagged == 0,
        serde_json::json!({"survey_has_vegetation_or_building_labels":labelled,"records_not_marked_inferred":unflagged}),
        "without class 3-6 returns every building and tree record must be marked inferred",
    ));
    let (mut in_corridor, mut corridor_checked) = (0_usize, false);
    if let Some(reference) = input.runway
        && let [a, b] = &reference.ends
        && let Ok(frame) = Frame::new([a.lat, a.lon], [b.lat, b.lon])
    {
        corridor_checked = true;
        let mut inside = |lat: f64, lon: f64| {
            if let Ok([u, v]) = frame.uv(lat, lon)
                && (0. ..=frame.length).contains(&u)
                && v.abs() <= reference.width_m / 2.
            {
                in_corridor += 1;
            }
        };
        for t in input.trees {
            inside(t.lat, t.lon);
        }
        for b in input.buildings {
            inside(b.center_lat_lon[0], b.center_lat_lon[1]);
        }
    }
    criteria.push(criterion(
        "runway_corridor_is_clear",
        corridor_checked && in_corridor == 0,
        serde_json::json!({"checked":corridor_checked,"objects_inside":in_corridor}),
        "no tree or building centre inside the FAA runway rectangle",
    ));
    let lat = input.runway.map_or(0., |r| (r.ends[0].lat + r.ends[1].lat) / 2.);
    let registration = input
        .mosaic
        .and_then(|m| registration(m, input.grid, input.buildings, lat));
    criteria.push(criterion(
        "footprints_register_with_naip",
        registration.as_ref().is_some_and(|r| {
            libm::hypot(r.best_shift_east_north_m[0], r.best_shift_east_north_m[1]) <= REGISTRATION_GATE_M
                && r.contrast_at_best > 1.1 * r.mean_contrast_over_window
        }),
        serde_json::to_value(&registration).unwrap_or(serde_json::Value::Null),
        "best NAIP footprint-contrast shift within 2 m of zero and clearly above the window mean",
    ));
    Acceptance {
        passed: criteria.iter().all(|c| c.passed),
        criteria,
        registration,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenery::buildings::{Plane, RoofKind};

    fn building(cells: &[usize], rms: f64, inferred: bool, centre: [f64; 2]) -> Building {
        Building {
            roof_kind: RoofKind::Flat,
            roof_planes: vec![Plane {
                a: 0.,
                b: 0.,
                c: 10.,
                rms_m: rms,
                samples: cells.len(),
            }],
            measured_roof_cells: cells.len(),
            interpolated_roof_cells: 0,
            inferred_classification: inferred,
            height_m: 6.,
            center_lat_lon: centre,
            excluded_roof_cells: 0,
            cells: cells.iter().map(|&c| (c, 0)).collect(),
        }
    }
    fn tree(inferred: bool, lat: f64, lon: f64) -> Tree {
        Tree {
            lat,
            lon,
            ground_msl_m: 100.,
            height_m: 14.,
            crown_radii_m: [2., 2.],
            crown_cells: 12,
            crown_azimuth_rad: 0.,
            species: crate::scenery::trees::Species::Deciduous,
            inferred_classification: inferred,
        }
    }
    fn runway() -> RunwayReference {
        let airports: serde_json::Value =
            serde_json::from_str(include_str!("../../../assets/terrain/eastern-pa/airports.json")).unwrap();
        crate::scenery::alignment::reference_from_region("eastern-pa", &airports, "1N7").unwrap()
    }
    fn good<'a>(
        grid: &'a Grid,
        classes: &'a BTreeMap<u8, u64>,
        buildings: &'a [Building],
        trees: &'a [Tree],
        runway: &'a RunwayReference,
    ) -> Inputs<'a> {
        Inputs {
            complete_selection: true,
            ground_samples: 5000,
            ground_median_m: Some(0.02),
            ground_p95_m: Some(0.07),
            vertical_datum_passed: true,
            classes,
            buildings,
            trees,
            grid,
            mosaic: None,
            runway: Some(runway),
        }
    }
    fn failing(acceptance: &Acceptance) -> Vec<&'static str> {
        acceptance
            .criteria
            .iter()
            .filter(|c| !c.passed)
            .map(|c| c.name)
            .collect()
    }

    #[test]
    fn every_criterion_can_fail_independently() {
        let grid = Grid::new([0., 0., 100., 100.]).unwrap();
        let classes: BTreeMap<u8, u64> = BTreeMap::from([(1, 10), (2, 10)]);
        let runway = runway();
        let off_runway = [40.9650, -74.9990];
        let buildings = vec![building(&[1, 2, 3], 0.05, true, off_runway)];
        let trees = vec![tree(true, off_runway[0], off_runway[1])];
        let base = good(&grid, &classes, &buildings, &trees, &runway);
        // Imagery registration is exercised separately; without a mosaic it must fail closed.
        assert_eq!(failing(&evaluate(&base)), vec!["footprints_register_with_naip"]);

        let mut input = good(&grid, &classes, &buildings, &trees, &runway);
        input.complete_selection = false;
        input.ground_median_m = Some(0.6);
        input.vertical_datum_passed = false;
        let names = failing(&evaluate(&input));
        for expected in [
            "complete_ept_selection",
            "ground_agrees_with_navd88_meter_dem",
            "vertical_datum_verified",
        ] {
            assert!(names.contains(&expected), "{expected} in {names:?}");
        }

        let bad_roof = vec![building(&[1, 2, 3], 0.4, true, off_runway)];
        assert!(
            failing(&evaluate(&good(&grid, &classes, &bad_roof, &trees, &runway)))
                .contains(&"roof_planes_fit_measured_points")
        );

        let honest = vec![building(&[1, 2, 3], 0.05, false, off_runway)];
        assert!(
            failing(&evaluate(&good(&grid, &classes, &honest, &trees, &runway)))
                .contains(&"inferred_classification_is_declared")
        );
        // Real class labels make an unflagged record legitimate.
        let labelled: BTreeMap<u8, u64> = BTreeMap::from([(6, 400)]);
        assert!(
            !failing(&evaluate(&good(&grid, &labelled, &honest, &trees, &runway)))
                .contains(&"inferred_classification_is_declared")
        );

        let on_runway = vec![tree(true, 40.97114, -74.9975)];
        assert!(
            failing(&evaluate(&good(&grid, &classes, &buildings, &on_runway, &runway)))
                .contains(&"runway_corridor_is_clear")
        );

        let mut tall = tree(true, off_runway[0], off_runway[1]);
        tall.height_m = 80.;
        assert!(
            failing(&evaluate(&good(&grid, &classes, &buildings, &[tall], &runway)))
                .contains(&"heights_are_plausible")
        );
    }

    #[test]
    fn footprint_registration_finds_a_known_shift() {
        // A bright 20 x 12 cell roof on dark grass, imagery shifted 2 m east of the lidar footprint.
        let (lat, lon) = (40.971, -74.9975);
        let [cx, cy] = flightcraft_world::scenery::mercator(lat, lon).unwrap();
        let side = 1024_usize;
        let half = 0.3 * 512. / libm::cos(lat.to_radians());
        let mut mosaic = Mosaic::new([cx - half, cy - half, cx + half, cy + half], side);
        let pixel = 2. * half / side as f64;
        let cos_lat = libm::cos(lat.to_radians());
        let grid = Grid::new([cx - 40., cy - 40., cx + 40., cy + 40.]).unwrap();
        let mut footprints = Vec::new();
        // Place three roofs, each 12 x 8 grid cells, in different rows of the grid.
        for (k, (c0, r0)) in [(5_usize, 5_usize), (30, 10), (10, 40)].into_iter().enumerate() {
            let mut cells = Vec::new();
            for r in r0..r0 + 10 {
                for c in c0..c0 + 20 {
                    cells.push(r * grid.width + c);
                }
            }
            footprints.push(building(&cells, 0.05, true, [lat, lon]));
            let _ = k;
        }
        let shift_east_m = 2.0;
        let to_pixel = |x: f64, y: f64| ((x - (cx - half)) / pixel, ((cy + half) - y) / pixel);
        mosaic.rgb.fill([40, 90, 40]);
        for b in &footprints {
            for &index in b.cells.keys() {
                let (col, row) = ((index % grid.width) as f64, (index / grid.width) as f64);
                let rx = grid.bounds[0] + (col + 0.5) * grid.step + shift_east_m / cos_lat;
                let ry = grid.bounds[1] + (row + 0.5) * grid.step;
                let (px, py) = to_pixel(rx, ry);
                let reach = (grid.step / 2. / pixel) as i64;
                for dy in -reach..reach {
                    for dx in -reach..reach {
                        let (x, y) = (px as i64 + dx, py as i64 + dy);
                        if x >= 0 && y >= 0 && (x as usize) < side && (y as usize) < side {
                            mosaic.rgb[y as usize * side + x as usize] = [220, 220, 215];
                        }
                    }
                }
            }
        }
        let r = registration(&mosaic, &grid, &footprints, lat).unwrap();
        assert_eq!(r.best_shift_east_north_m, [2., 0.], "{r:?}");
        assert!(r.contrast_at_best > 1.1 * r.mean_contrast_over_window);
    }
}
