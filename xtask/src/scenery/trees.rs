//! Original canopy segmentation: robust DSM minus DEM, local maxima, and descending crowns.
use super::surface::Grid;
use serde::Serialize;
use std::{cmp::Reverse, collections::BinaryHeap};
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(super) enum Species {
    Deciduous,
    Conifer,
}
#[derive(Clone, Debug, Serialize)]
pub(super) struct Tree {
    pub lat: f64,
    pub lon: f64,
    pub ground_msl_m: f64,
    pub height_m: f64,
    pub crown_radii_m: [f64; 2],
    pub crown_cells: usize,
    pub crown_azimuth_rad: f64,
    pub species: Species,
    pub inferred_classification: bool,
}
/// Neighborhood-median smoothing protects a single noisy return from inventing a tall tree. All heights
/// and crowns remain measured; there is no random placement or density multiplier.
pub(super) fn detect(grid: &Grid) -> Result<Vec<Tree>, String> {
    let mut chm = vec![0_f32; grid.cells.len()];
    for (i, cell) in grid.cells.iter().enumerate() {
        let h = cell.height();
        if !h.is_finite() || !(2.5..=70.).contains(&h) || cell.flags & 4 != 0 {
            continue;
        }
        let (x, y) = ((i % grid.width) as i32, (i / grid.width) as i32);
        let mut values = Vec::with_capacity(9);
        for dy in -1..=1 {
            for dx in -1..=1 {
                if let Some((_, c)) = grid.at(x + dx, y + dy) {
                    let h = c.height();
                    if (2.5..=70.).contains(&h) && c.flags & 4 == 0 {
                        values.push(h);
                    }
                }
            }
        }
        if values.len() >= 4 {
            values.sort_by(f32::total_cmp);
            *chm.get_mut(i).ok_or("CHM cell")? = *values.get(values.len() / 2).ok_or("CHM median")?;
        }
    }
    let get = |x: i32, y: i32| {
        grid.at(x, y)
            .and_then(|(i, _)| chm.get(i).copied().map(|h| (i, h)))
    };
    let mut seeds = Vec::new();
    for (i, &h) in chm.iter().enumerate() {
        if h < 3.5 {
            continue;
        }
        let (x, y) = ((i % grid.width) as i32, (i / grid.width) as i32);
        let radius = (h * 0.12).round().clamp(2., 5.) as i32;
        let mut maximum = true;
        'neighbors: for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx * dx + dy * dy > radius * radius {
                    continue;
                }
                if let Some((j, other)) = get(x + dx, y + dy)
                    && (other > h || (other == h && j < i))
                {
                    maximum = false;
                    break 'neighbors;
                }
            }
        }
        if !maximum {
            continue;
        }
        // A tree drops around its crown. Flat roofs and long gable ridges do not.
        let mut drops = 0;
        for (dx, dy) in [
            (3, 0),
            (-3, 0),
            (0, 3),
            (0, -3),
            (2, 2),
            (-2, 2),
            (2, -2),
            (-2, -2),
        ] {
            if get(x + dx, y + dy).is_some_and(|(_, v)| v < h - 0.45) {
                drops += 1;
            }
        }
        let cell = grid.cells.get(i).ok_or("CHM seed")?;
        if cell.flags & 2 == 0 && drops < 7 {
            continue;
        }
        seeds.push(i);
        if seeds.len() > 100_000 {
            return Err("Canopy tree count exceeds block budget".into());
        }
    }
    let mut labels = vec![usize::MAX; chm.len()];
    let mut queue = BinaryHeap::new();
    for (label, &index) in seeds.iter().enumerate() {
        queue.push((
            chm.get(index).ok_or("CHM seed height")?.to_bits(),
            Reverse(index),
            Reverse(label),
        ));
    }
    let mut crowns = vec![Vec::new(); seeds.len()];
    while let Some((_, Reverse(index), Reverse(label))) = queue.pop() {
        let assigned = labels.get_mut(index).ok_or("Crown cell")?;
        if *assigned != usize::MAX {
            continue;
        }
        *assigned = label;
        crowns.get_mut(label).ok_or("Crown label")?.push(index);
        let seed = *seeds.get(label).ok_or("Crown seed")?;
        let peak = *chm.get(seed).ok_or("Crown peak")?;
        let h = *chm.get(index).ok_or("Crown height")?;
        let (x, y) = ((index % grid.width) as i32, (index / grid.width) as i32);
        let (sx, sy) = ((seed % grid.width) as i32, (seed / grid.width) as i32);
        for dy in -1..=1 {
            for dx in -1..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let (nx, ny) = (x + dx, y + dy);
                if (nx - sx).pow(2) + (ny - sy).pow(2) > 100 {
                    continue;
                }
                if let Some((next, nh)) = get(nx, ny)
                    && nh >= peak * 0.45
                    && nh >= 2.5
                    && nh <= h + 0.35
                    && labels.get(next) == Some(&usize::MAX)
                {
                    queue.push((nh.to_bits(), Reverse(next), Reverse(label)));
                }
            }
        }
        if queue.len() > grid.cells.len().saturating_mul(8) {
            return Err("Canopy watershed queue exceeds block budget".into());
        }
    }
    let mut trees = Vec::new();
    for (label, crown) in crowns.into_iter().enumerate() {
        if crown.len() < 6 {
            continue;
        }
        let seed = *seeds.get(label).ok_or("Tree seed")?;
        let (sx, sy) = ((seed % grid.width) as f64, (seed / grid.width) as f64);
        let (mut xx, mut yy, mut xy) = (0., 0., 0.);
        for &index in &crown {
            let dx = (index % grid.width) as f64 - sx;
            let dy = (index / grid.width) as f64 - sy;
            xx += dx * dx;
            yy += dy * dy;
            xy += dx * dy;
        }
        let angle = 0.5 * libm::atan2(2. * xy, xx - yy);
        let (sn, cs) = (libm::sin(angle), libm::cos(angle));
        let mut rx = Vec::new();
        let mut ry = Vec::new();
        let mut top = f32::NEG_INFINITY;
        let mut classified = false;
        for &index in &crown {
            let (dx, dy) = ((index % grid.width) as f64 - sx, (index / grid.width) as f64 - sy);
            rx.push((dx * cs + dy * sn).abs() + 0.5);
            ry.push((-dx * sn + dy * cs).abs() + 0.5);
            let c = grid.cells.get(index).ok_or("Tree source cell")?;
            top = top.max(c.top);
            classified |= c.flags & 2 != 0;
        }
        rx.sort_by(f64::total_cmp);
        ry.sort_by(f64::total_cmp);
        let radius = |v: &[f64]| v.get(v.len() * 9 / 10).copied().unwrap_or(0.5).clamp(0.8, 10.);
        let radii = [radius(&rx), radius(&ry)];
        let ground = f64::from(grid.cells.get(seed).ok_or("Tree ground")?.ground);
        let height = f64::from(top) - ground;
        if !(3.5..=70.).contains(&height) {
            continue;
        }
        let [lat, lon] = grid.coordinate(seed)?;
        trees.push(Tree {
            lat,
            lon,
            ground_msl_m: ground,
            height_m: height,
            crown_radii_m: radii,
            crown_cells: crown.len(),
            crown_azimuth_rad: angle,
            species: if radii[0].max(radii[1]) / height < 0.13 {
                Species::Conifer
            } else {
                Species::Deciduous
            },
            inferred_classification: !classified,
        });
    }
    Ok(trees)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenery::surface::Cell;
    #[test]
    fn separates_measured_crowns_and_rejects_flat_roofs_and_single_spikes() {
        let mut grid = Grid::new([0., 0., 80., 50.]).unwrap();
        for y in 0..grid.height {
            for x in 0..grid.width {
                let mut h = 0_f64;
                for (cx, cy, height, radius) in [(15., 20., 18., 6.), (32., 20., 22., 3.)] {
                    let d = libm::hypot(x as f64 - cx, y as f64 - cy);
                    if d < radius {
                        h = h.max(height * (1. - 0.45 * (d / radius).powi(2)));
                    }
                }
                if x > 50 && x < 70 && y > 10 && y < 30 {
                    h = 8.;
                }
                grid.cells[y * grid.width + x] = Cell {
                    top: 100. + h as f32,
                    ground: 100.,
                    count: 4,
                    flags: 1,
                };
            }
        }
        grid.cells[2 * grid.width + 2].top = 160.;
        let trees = detect(&grid).unwrap();
        assert_eq!(trees.len(), 2);
        assert!(trees.iter().all(|t| t.inferred_classification));
        assert!(
            trees
                .iter()
                .any(|t| (t.height_m - 18.).abs() < 0.01 && t.crown_radii_m[0] > 3.)
        );
        assert!(
            trees
                .iter()
                .any(|t| (t.height_m - 22.).abs() < 0.01 && t.species == Species::Conifer)
        );
        assert!(!trees.iter().any(|t| t.height_m > 30.));
    }
}
