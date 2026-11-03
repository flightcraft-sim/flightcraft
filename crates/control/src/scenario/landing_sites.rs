//! Geometric screening of wind-aligned landing rectangles, never a land-use clearance.
use super::emergency::{GlideField, GlideFootprint, contains};
use flightcraft_world::{Surface, World};

pub(super) fn candidates(world: &World, g: &GlideFootprint) -> Vec<GlideField> {
    let [n, e, _] = g.origin_m;
    let radius = g
        .boundary_m
        .iter()
        .map(|[pn, pe]| libm::hypot(pn - n, pe - e))
        .fold(0., f64::max);
    let spacing = (radius / 24.).max(150.);
    let [wn, we, _] = g.wind_ned_mps;
    let heading = libm::atan2(-we, -wn);
    let calm_headings = [
        0.,
        core::f64::consts::FRAC_PI_4,
        core::f64::consts::FRAC_PI_2,
        3. * core::f64::consts::FRAC_PI_4,
    ];
    let wind_heading = [heading];
    let headings: &[f64] = if libm::hypot(wn, we) < 0.5 {
        &calm_headings
    } else {
        &wind_heading
    };
    let mut found = Vec::new();
    for row in -24..=24 {
        for col in -24..=24 {
            let p = [n + f64::from(row) * spacing, e + f64::from(col) * spacing];
            if !contains(&g.boundary_m, p) {
                continue;
            }
            // Calm conditions admit four strip orientations; wind fixes the approach axis.
            for &angle in headings {
                if let Some((length, slope, elevation)) = rectangle(world, g, p, angle) {
                    found.push(GlideField {
                        id: format!("open-{row}-{col}"),
                        position_m: p,
                        terrain_msl_m: elevation,
                        airport: false,
                        length_m: length,
                        width_m: 60.,
                        slope_percent: slope,
                        heading_grid_rad: angle,
                    });
                }
            }
        }
    }
    found.sort_by(|a, b| {
        let score = |f: &GlideField| {
            let [fnorth, feast] = f.position_m;
            f.length_m - f.slope_percent * 150. - libm::hypot(fnorth - n, feast - e) * 0.03
        };
        score(b).total_cmp(&score(a)).then_with(|| a.id.cmp(&b.id))
    });
    let mut best: Vec<GlideField> = Vec::new();
    for field in found {
        let separated = best.iter().all(|f| {
            let [an, ae] = f.position_m;
            let [bn, be] = field.position_m;
            libm::hypot(an - bn, ae - be) > 900.
        });
        if separated {
            best.push(field);
            if best.len() == 6 {
                break;
            }
        }
    }
    best
}
fn rectangle(world: &World, g: &GlideFootprint, [n, e]: [f64; 2], heading: f64) -> Option<(f64, f64, f64)> {
    let (dn, de) = (libm::cos(heading), libm::sin(heading));
    let base = sample(world, n, e)?;
    let mut best = None;
    for length in [600., 750., 900., 1050., 1200.] {
        let mut max_slope: f64 = 0.;
        let mut min_h = f64::INFINITY;
        let mut max_h = f64::NEG_INFINITY;
        for step in -12..=12 {
            for cross in [-30., 0., 30.] {
                let along = f64::from(step) * length / 24.;
                let p = [n + dn * along - de * cross, e + de * along + dn * cross];
                if !contains(&g.boundary_m, p) {
                    return best;
                }
                let Some(ground) = ({
                    let [pn, pe] = p;
                    sample(world, pn, pe)
                }) else {
                    return best;
                };
                let [nn, ne, nd] = ground.normal_ned;
                max_slope = max_slope.max(100. * libm::hypot(nn, ne) / nd.abs().max(0.01));
                min_h = min_h.min(ground.height_msl_m);
                max_h = max_h.max(ground.height_msl_m);
                if max_slope > 3. || max_h - min_h > length * 0.02 {
                    return best;
                }
            }
        }
        best = Some((length, max_slope, base.height_msl_m));
    }
    best
}
fn sample(world: &World, n: f64, e: f64) -> Option<flightcraft_world::Ground> {
    let ground = if world.region().is_some() {
        world.terrain_ground(n, e)?
    } else {
        world.ground(n, e)
    };
    (ground.surface == Surface::Grass).then_some(ground)
}
