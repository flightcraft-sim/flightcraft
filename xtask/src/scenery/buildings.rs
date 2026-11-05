//! Roof planes and cell-cluster footprints. Concavities, courtyards, and measured steps survive.
//! A roof without enough planar evidence is excluded, never replaced by an invented flat roof.
use super::surface::Grid;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
#[derive(Clone, Copy, Debug, Serialize)]
pub(super) struct Plane {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub rms_m: f64,
    pub samples: usize,
}
impl Plane {
    pub fn z(self, x: f64, y: f64) -> f64 {
        self.a * x + self.b * y + self.c
    }
    fn fit(points: &[[f64; 3]]) -> Option<Self> {
        if points.len() < 6 {
            return None;
        }
        let n = points.len() as f64;
        let mean = std::array::from_fn::<_, 3, _>(|axis| points.iter().map(|p| p[axis]).sum::<f64>() / n);
        let (mut xx, mut yy, mut xy, mut xz, mut yz) = (0., 0., 0., 0., 0.);
        for p in points {
            let (x, y, z) = (p[0] - mean[0], p[1] - mean[1], p[2] - mean[2]);
            xx += x * x;
            yy += y * y;
            xy += x * y;
            xz += x * z;
            yz += y * z;
        }
        let determinant = xx * yy - xy * xy;
        if determinant < 1e-8 {
            return None;
        }
        let a = (xz * yy - yz * xy) / determinant;
        let b = (yz * xx - xz * xy) / determinant;
        let c = mean[2] - a * mean[0] - b * mean[1];
        let rms = libm::sqrt(
            points
                .iter()
                .map(|p| (p[2] - a * p[0] - b * p[1] - c).powi(2))
                .sum::<f64>()
                / n,
        );
        if [a, b, c, rms].iter().any(|v| !v.is_finite()) || libm::hypot(a, b) > 1.2 {
            return None;
        }
        Some(Self {
            a,
            b,
            c,
            rms_m: rms,
            samples: points.len(),
        })
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum RoofKind {
    Flat,
    Gable,
    Hip,
    Stepped,
}
#[derive(Debug, Serialize)]
pub(super) struct Building {
    pub roof_kind: RoofKind,
    pub roof_planes: Vec<Plane>,
    pub measured_roof_cells: usize,
    pub interpolated_roof_cells: usize,
    pub inferred_classification: bool,
    pub height_m: f64,
    pub center_lat_lon: [f64; 2],
    pub excluded_roof_cells: usize,
    #[serde(skip)]
    pub cells: BTreeMap<usize, usize>,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub plane: usize,
}
/// Coalesce coplanar cells into rectangles, retaining every unoccupied courtyard cell.
/// Complexity follows the occupied raster, rather than a full-resolution triangle mesh.
pub(super) fn rectangles(building: &Building, width: usize) -> Vec<Rect> {
    let mut rows = BTreeMap::<usize, Vec<(usize, usize)>>::new();
    for (&index, &plane) in &building.cells {
        rows.entry(index / width)
            .or_default()
            .push((index % width, plane));
    }
    let mut active = BTreeMap::<(usize, usize, usize), Rect>::new();
    let mut result = Vec::new();
    let mut previous = None;
    for (y, cells) in rows {
        if previous.is_some_and(|last| last + 1 != y) {
            result.extend(std::mem::take(&mut active).into_values());
        }
        let mut runs = Vec::new();
        let mut start = 0;
        let mut end = 0;
        let mut old = None;
        for (x, plane) in cells {
            if old == Some(plane) && x == end + 1 {
                end = x;
            } else {
                if let Some(p) = old {
                    runs.push((start, end, p));
                }
                start = x;
                end = x;
                old = Some(plane);
            }
        }
        if let Some(p) = old {
            runs.push((start, end, p));
        }
        let mut next = BTreeMap::new();
        for (start, end, plane) in runs {
            let key = (start, end, plane);
            let rect = if let Some(mut r) = active.remove(&key) {
                r.y1 = y as f64 + 0.5;
                r
            } else {
                Rect {
                    x0: start as f64 - 0.5,
                    x1: end as f64 + 0.5,
                    y0: y as f64 - 0.5,
                    y1: y as f64 + 0.5,
                    plane,
                }
            };
            next.insert(key, rect);
        }
        result.extend(active.into_values());
        active = next;
        previous = Some(y);
    }
    result.extend(active.into_values());
    result
}
fn point(grid: &Grid, index: usize) -> Option<[f64; 3]> {
    let c = grid.cells.get(index)?;
    if (c.count == 0 && c.flags & 32 == 0) || !c.top.is_finite() {
        return None;
    }
    Some([
        (index % grid.width) as f64,
        (index / grid.width) as f64,
        f64::from(c.top),
    ])
}
fn neighbors(grid: &Grid, index: usize) -> impl Iterator<Item = usize> + '_ {
    let (x, y) = ((index % grid.width) as i32, (index / grid.width) as i32);
    [(1, 0), (-1, 0), (0, 1), (0, -1)]
        .into_iter()
        .filter_map(move |(dx, dy)| grid.at(x + dx, y + dy).map(|(i, _)| i))
}
/// A roof cluster larger than this (square metres of roof) is skipped rather than meshed.
const MAX_COMPONENT_CELLS: usize = 250_000;
pub(super) fn detect(grid: &mut Grid) -> Result<Vec<Building>, String> {
    let mut mask = vec![false; grid.cells.len()];
    let mut local = vec![None; grid.cells.len()];
    for (index, cell) in grid.cells.iter().enumerate() {
        let h = cell.height();
        if !(2.5..=700.).contains(&h) || cell.count == 0 || (cell.flags & 2 != 0 && cell.flags & 4 == 0) {
            continue;
        }
        let (x, y) = ((index % grid.width) as i32, (index / grid.width) as i32);
        let mut points = Vec::with_capacity(25);
        for dy in -2..=2 {
            for dx in -2..=2 {
                if let Some((i, c)) = grid.at(x + dx, y + dy)
                    && (2.0..=700.).contains(&c.height())
                    && (c.flags & 2 == 0 || c.flags & 4 != 0)
                    && let Some(p) = point(grid, i)
                {
                    points.push(p);
                }
            }
        }
        let plane = if points.len() >= 12 {
            Plane::fit(&points).filter(|p| p.rms_m <= 0.18)
        } else {
            None
        };
        *local.get_mut(index).ok_or("Roof plane cell")? = plane;
        *mask.get_mut(index).ok_or("Roof mask cell")? = cell.flags & 4 != 0 || plane.is_some();
    }
    // Fill an isolated no-return cell only when all eight neighbors are measured roof
    // candidates and predict a consistent plane. Ground/water evidence vetoes filling.
    let original_mask = mask.clone();
    let mut fills = Vec::new();
    for (index, cell) in grid.cells.iter().enumerate() {
        if cell.count > 0 || cell.flags & 24 != 0 {
            continue;
        }
        let (x, y) = ((index % grid.width) as i32, (index / grid.width) as i32);
        let mut source = Vec::new();
        let mut ground = 0.;
        for dy in -1..=1 {
            for dx in -1..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                if let Some((i, c)) = grid.at(x + dx, y + dy)
                    && original_mask.get(i) == Some(&true)
                    && let Some(p) = point(grid, i)
                {
                    source.push(p);
                    ground += f64::from(c.ground);
                }
            }
        }
        if source.len() == 8
            && let Some(p) = Plane::fit(&source).filter(|p| p.rms_m <= 0.15)
        {
            fills.push((index, p, ground / 8.));
        }
    }
    for (index, plane, ground) in fills {
        let cell = grid.cells.get_mut(index).ok_or("Interpolated roof cell")?;
        cell.top = plane.z((index % grid.width) as f64, (index / grid.width) as f64) as f32;
        cell.ground = ground as f32;
        cell.flags |= 32;
        *mask.get_mut(index).ok_or("Interpolated roof mask")? = true;
        *local.get_mut(index).ok_or("Interpolated roof plane")? = Some(plane);
    }
    // Bridge one-cell gaps at a ridge only when there is an actual elevated return.
    // Ground, water, absent data, and courtyards are never filled by this operation.
    let original = mask.clone();
    for (index, cell) in grid.cells.iter().enumerate() {
        if original.get(index) == Some(&true)
            || !(2.5..=700.).contains(&cell.height())
            || (cell.flags & 2 != 0 && cell.flags & 4 == 0)
        {
            continue;
        }
        let (x, y) = ((index % grid.width) as i32, (index / grid.width) as i32);
        let mut count = 0;
        for dy in -1..=1 {
            for dx in -1..=1 {
                if let Some((i, _)) = grid.at(x + dx, y + dy)
                    && original.get(i) == Some(&true)
                {
                    count += 1;
                }
            }
        }
        if count >= 5 {
            *mask.get_mut(index).ok_or("Roof closing cell")? = true;
        }
    }
    let mut visited = vec![false; mask.len()];
    let mut buildings = Vec::new();
    let mut oversized = false;
    for seed in 0..mask.len() {
        if mask.get(seed) != Some(&true) || visited.get(seed) == Some(&true) {
            continue;
        }
        let mut queue = VecDeque::from([seed]);
        *visited.get_mut(seed).ok_or("Building seed")? = true;
        let mut component = BTreeSet::new();
        while let Some(i) = queue.pop_front() {
            component.insert(i);
            if component.len() > MAX_COMPONENT_CELLS {
                // Keep consuming the cluster so its cells never seed new ones, but skip it below:
                // one pathological connected roof must not abort a whole block.
                oversized = true;
            }
            for j in neighbors(grid, i) {
                if mask.get(j) == Some(&true) && visited.get(j) == Some(&false) {
                    *visited.get_mut(j).ok_or("Building neighbor")? = true;
                    queue.push_back(j);
                }
            }
        }
        if component.len() < 20 || std::mem::take(&mut oversized) {
            continue;
        }
        let mut cells = BTreeMap::new();
        let mut rejected = BTreeSet::new();
        let mut planes = Vec::new();
        for &start in &component {
            if cells.contains_key(&start) || rejected.contains(&start) {
                continue;
            }
            let Some(candidate) = local.get(start).copied().flatten() else {
                continue;
            };
            let mut group = BTreeSet::from([start]);
            let mut queue = VecDeque::from([start]);
            while let Some(i) = queue.pop_front() {
                for j in neighbors(grid, i) {
                    if !component.contains(&j) || cells.contains_key(&j) || group.contains(&j) {
                        continue;
                    }
                    if let Some(p) = point(grid, j)
                        && (p[2] - candidate.z(p[0], p[1])).abs() <= 0.30
                    {
                        group.insert(j);
                        queue.push_back(j);
                    }
                }
            }
            if group.len() < 6 {
                rejected.extend(group);
                continue;
            }
            let points: Vec<_> = group
                .iter()
                .filter(|i| grid.cells.get(**i).is_some_and(|c| c.count > 0))
                .filter_map(|i| point(grid, *i))
                .collect();
            let Some(plane) = Plane::fit(&points).filter(|p| p.rms_m <= 0.30) else {
                rejected.extend(group);
                continue;
            };
            if planes.len() >= 128 {
                // Cluttered roofs: the remaining unplaced cells are snapped to measured planes below.
                break;
            }
            let id = planes.len();
            planes.push(plane);
            for i in group {
                cells.insert(i, id);
            }
        }
        if planes.is_empty() {
            continue;
        }
        // Narrow ridge/parapet cells can join only a measured plane close to their actual Z.
        for &index in &component {
            if cells.contains_key(&index) {
                continue;
            }
            let Some(p) = point(grid, index) else {
                continue;
            };
            let nearest = planes
                .iter()
                .enumerate()
                .map(|(id, plane)| (id, (p[2] - plane.z(p[0], p[1])).abs()))
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((id, error)) = nearest
                && error <= 0.55
            {
                cells.insert(index, id);
            }
        }
        if cells.len() < 20 || cells.len() * 10 < component.len() * 8 {
            continue;
        }
        // Merge independently grown patches only if their measured returns support the
        // same global plane. This removes scan-strip fragmentation without flattening steps.
        for i in 0..planes.len() {
            if !cells.values().any(|p| *p == i) {
                continue;
            }
            for j in 0..i {
                if !cells.values().any(|p| *p == j) {
                    continue;
                }
                let p = *planes.get(i).ok_or("Roof merge plane")?;
                let q = *planes.get(j).ok_or("Roof merge target")?;
                let center = *cells.keys().nth(cells.len() / 2).ok_or("Roof merge center")?;
                let x = (center % grid.width) as f64;
                let y = (center / grid.width) as f64;
                if libm::hypot(p.a - q.a, p.b - q.b) > 0.06 || (p.z(x, y) - q.z(x, y)).abs() > 0.20 {
                    continue;
                }
                let points: Vec<_> = cells
                    .iter()
                    .filter(|(id, plane)| {
                        (**plane == i || **plane == j) && grid.cells.get(**id).is_some_and(|c| c.count > 0)
                    })
                    .filter_map(|(id, _)| point(grid, *id))
                    .collect();
                if let Some(merged) = Plane::fit(&points).filter(|p| p.rms_m <= 0.20) {
                    *planes.get_mut(j).ok_or("Merged roof plane")? = merged;
                    for plane in cells.values_mut() {
                        if *plane == i {
                            *plane = j;
                        }
                    }
                    break;
                }
            }
        }
        let mut compact = Vec::new();
        let mut ids = BTreeMap::new();
        for id in cells.values_mut() {
            let next = if let Some(next) = ids.get(id) {
                *next
            } else {
                let next = compact.len();
                compact.push(*planes.get(*id).ok_or("Roof plane remap")?);
                ids.insert(*id, next);
                next
            };
            *id = next;
        }
        planes = compact;
        let measured = cells
            .keys()
            .filter(|i| grid.cells.get(**i).is_some_and(|c| c.count > 0))
            .count();
        let interpolated = cells.len() - measured;
        let classified = cells
            .keys()
            .filter(|i| {
                grid.cells
                    .get(**i)
                    .is_some_and(|c| c.count > 0 && c.flags & 4 != 0)
            })
            .count();
        let center = *cells.keys().nth(cells.len() / 2).ok_or("Building center")?;
        let height = cells
            .keys()
            .filter_map(|i| grid.cells.get(*i))
            .map(|c| f64::from(c.height()))
            .fold(0., f64::max);
        let mut counts = vec![0_usize; planes.len()];
        for &id in cells.values() {
            *counts.get_mut(id).ok_or("Roof plane count")? += 1;
        }
        let significant: Vec<_> = planes
            .iter()
            .zip(counts)
            .filter(|(_, count)| *count * 20 >= cells.len())
            .map(|(p, _)| *p)
            .collect();
        let steep: Vec<_> = significant
            .iter()
            .filter(|p| libm::hypot(p.a, p.b) >= 0.03)
            .copied()
            .collect();
        let gable = steep.first().is_some_and(|p| {
            steep.len() >= 2
                && steep.iter().all(|q| {
                    (p.a * q.b - p.b * q.a).abs() < 0.15 * libm::hypot(p.a, p.b) * libm::hypot(q.a, q.b)
                })
                && steep.iter().any(|q| p.a * q.a + p.b * q.b < 0.)
        });
        let flat_level = significant.first().is_some_and(|p| {
            significant.iter().all(|q| {
                (p.z((center % grid.width) as f64, (center / grid.width) as f64)
                    - q.z((center % grid.width) as f64, (center / grid.width) as f64))
                .abs()
                    < 0.30
            })
        });
        let roof_kind = if steep.is_empty() && flat_level {
            RoofKind::Flat
        } else if gable {
            RoofKind::Gable
        } else if steep.len() >= 3 {
            RoofKind::Hip
        } else {
            RoofKind::Stepped
        };
        buildings.push(Building {
            roof_kind,
            roof_planes: planes,
            measured_roof_cells: measured,
            interpolated_roof_cells: interpolated,
            inferred_classification: classified * 10 < measured * 9,
            height_m: height,
            center_lat_lon: grid.coordinate(center)?,
            excluded_roof_cells: component.len() - cells.len(),
            cells,
        });
        if buildings.len() > 100_000 {
            return Err("Building count exceeds regional budget".into());
        }
    }
    Ok(buildings)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenery::surface::Cell;
    #[test]
    fn long_low_pitch_hangars_are_gables_not_stepped_towers() {
        let mut grid = Grid::new([0., 0., 100., 50.]).unwrap();
        for y in 10..30 {
            for x in 10..90 {
                grid.cells[y * grid.width + x] = Cell {
                    top: (106. - 0.07 * (y as f32 - 20.).abs()),
                    ground: 100.,
                    count: 5,
                    flags: 1,
                };
            }
        }
        let buildings = detect(&mut grid).unwrap();
        assert_eq!(buildings.len(), 1);
        assert!(matches!(buildings[0].roof_kind, RoofKind::Gable));
        assert!(buildings[0].height_m < 7.);
    }
    #[test]
    fn fills_only_an_isolated_supported_gap_and_preserves_ground_evidence() {
        let mut grid = Grid::new([0., 0., 60., 50.]).unwrap();
        for y in 10..40 {
            for x in 10..50 {
                grid.cells[y * grid.width + x] = Cell {
                    top: 110.,
                    ground: 100.,
                    count: 6,
                    flags: 4,
                };
            }
        }
        let missing = 20 * grid.width + 20;
        let ground = 20 * grid.width + 30;
        grid.cells[missing] = Cell::default();
        grid.cells[ground] = Cell {
            flags: 8,
            ..Cell::default()
        };
        let buildings = detect(&mut grid).unwrap();
        assert_eq!(buildings.len(), 1);
        let building = &buildings[0];
        assert!(building.cells.contains_key(&missing));
        assert!(!building.cells.contains_key(&ground));
        assert_eq!(building.interpolated_roof_cells, 1);
        assert_eq!(building.measured_roof_cells, 1198);
    }
    #[test]
    fn tall_stepped_roofs_keep_the_courtyard_and_fitted_heights() {
        let mut grid = Grid::new([0., 0., 80., 70.]).unwrap();
        for y in 10..50 {
            for x in 10..60 {
                if (25..45).contains(&x) && (22..38).contains(&y) {
                    continue;
                }
                let h = if x < 30 { 150. } else { 210. };
                grid.cells[y * grid.width + x] = Cell {
                    top: 100. + h,
                    ground: 100.,
                    count: 8,
                    flags: 4,
                };
            }
        }
        let buildings = detect(&mut grid).unwrap();
        assert_eq!(buildings.len(), 1);
        let b = &buildings[0];
        assert!(!b.inferred_classification);
        assert_eq!(b.roof_planes.len(), 2);
        assert_eq!(b.height_m, 210.);
        let rects = rectangles(b, grid.width);
        assert_eq!(
            rects.iter().map(|r| (r.x1 - r.x0) * (r.y1 - r.y0)).sum::<f64>(),
            1680.
        );
        assert!(
            rects
                .iter()
                .all(|r| !(r.x0 < 35. && r.x1 > 35. && r.y0 < 30. && r.y1 > 30.))
        );
        assert!(b.roof_planes.iter().all(|p| p.rms_m < 1e-8));
        assert!(rects.len() < 20);
    }
    #[test]
    fn original_gable_and_hip_surfaces_fit_without_flat_substitution() {
        for hip in [false, true] {
            let mut grid = Grid::new([0., 0., 70., 60.]).unwrap();
            for y in 10..45 {
                for x in 10..60 {
                    let across = (y as f64 - 27.).abs();
                    let along = (x as f64 - 35.).abs();
                    let drop = if hip {
                        (across * 0.25).max((along - 10.).max(0.) * 0.25)
                    } else {
                        across * 0.25
                    };
                    grid.cells[y * grid.width + x] = Cell {
                        top: (115. - drop) as f32,
                        ground: 100.,
                        count: 4,
                        flags: 4,
                    };
                }
            }
            let buildings = detect(&mut grid).unwrap();
            assert_eq!(buildings.len(), 1);
            let b = &buildings[0];
            assert!(b.roof_planes.len() >= if hip { 4 } else { 2 });
            assert!(b.roof_planes.iter().all(|p| p.rms_m < 0.15));
            assert!(b.height_m > 14.9);
        }
    }
}
