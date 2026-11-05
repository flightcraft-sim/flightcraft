//! Bounded offline construction. Scratch triangles are deleted; retained content is only GLB.
use super::city::{self, Triangle};
use flightcraft_world::{
    Region,
    tiles3d::{self, Asset, Content, Refine, Tile, Tileset},
};
use glam::{DMat4, DVec3};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{BufReader, Read, Write},
    path::{Path, PathBuf},
};

pub(super) const WEB_BUILDING_BUDGET: u64 = flightcraft_render::scenery::NYC_BUILDING_BUDGET;
pub(super) fn frame() -> Result<([f64; 16], DMat4), String> {
    let region = Region::from_json(include_str!("../../../assets/terrain/new-york/region.json"))
        .map_err(|e| e.to_string())?;
    let t = tiles3d::enu_transform(region.origin.latitude_deg, region.origin.longitude_deg, 0.)
        .map_err(|e| e.to_string())?;
    Ok((t, DMat4::from_cols_array(&t).inverse()))
}
pub(super) fn local(inverse: DMat4, lat: f64, lon: f64, height: f64) -> Result<DVec3, String> {
    if !(40.4..=41.).contains(&lat) || !(-74.35..=-73.6).contains(&lon) || !(-500.0..=1500.).contains(&height)
    {
        return Err(
            "Building coordinate is outside NYC; check CRS, axis order, units, and height reference".into(),
        );
    }
    let p = inverse.transform_point3(DVec3::from_array(
        tiles3d::ecef(lat, lon, height).map_err(|e| e.to_string())?,
    ));
    Ok(DVec3::new(p.x, p.z, -p.y))
}
/// Even-odd planar strips preserve concavity, courtyards, and the input roof plane.
/// Every strip endpoint lies on an original polygon edge. No triangle fan fills holes.
pub(super) fn triangulate(rings: &[Vec<DVec3>]) -> Result<Vec<Triangle>, String> {
    let outer = rings.first().ok_or("Polygon has no exterior")?;
    let vertices: usize = rings.iter().map(Vec::len).sum();
    if vertices > 30_000 || rings.len() > 4096 {
        return Err("Polygon exceeds tessellation budget".into());
    }
    for ring in rings {
        if ring.len() < 4 || ring.first() != ring.last() || ring.iter().any(|p| !p.is_finite()) {
            return Err("Polygon requires closed finite rings".into());
        }
    }
    let origin = *outer.first().ok_or("Polygon origin")?;
    let normal = outer
        .windows(2)
        .fold(DVec3::ZERO, |n, p| n + (p[0] - origin).cross(p[1] - origin))
        .normalize_or_zero();
    if normal.length_squared() < 0.5 {
        return Err("Degenerate polygon plane".into());
    }
    if rings
        .iter()
        .flatten()
        .any(|p| (*p - origin).dot(normal).abs() > 0.10)
    {
        return Err("Polygon is not planar within 10 cm".into());
    }
    let abs = normal.abs();
    let project = |p: DVec3| {
        if abs.x >= abs.y && abs.x >= abs.z {
            [p.y, p.z]
        } else if abs.y >= abs.z {
            [p.x, p.z]
        } else {
            [p.x, p.y]
        }
    };
    let projected: Vec<Vec<_>> = rings
        .iter()
        .map(|r| r.iter().copied().map(project).collect())
        .collect();
    let orientation =
        |a: [f64; 2], b: [f64; 2], c: [f64; 2]| (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    let on_segment = |a: [f64; 2], b: [f64; 2], p: [f64; 2]| {
        orientation(a, b, p).abs() < 1e-8
            && p[0] >= a[0].min(b[0]) - 1e-8
            && p[0] <= a[0].max(b[0]) + 1e-8
            && p[1] >= a[1].min(b[1]) - 1e-8
            && p[1] <= a[1].max(b[1]) + 1e-8
    };
    let edges: Vec<_> = projected
        .iter()
        .enumerate()
        .flat_map(|(r, ring)| {
            ring.windows(2)
                .enumerate()
                .map(move |(i, e)| (r, i, ring.len() - 1, e[0], e[1]))
        })
        .collect();
    for (index, a) in edges.iter().enumerate() {
        for b in edges.iter().skip(index + 1) {
            if a.0 == b.0 && ((a.1 + 1) % a.2 == b.1 || (b.1 + 1) % b.2 == a.1) {
                continue;
            }
            let [ab, ac, cd, ce] = [
                orientation(a.3, a.4, b.3),
                orientation(a.3, a.4, b.4),
                orientation(b.3, b.4, a.3),
                orientation(b.3, b.4, a.4),
            ];
            if (ab * ac < 0. && cd * ce < 0.)
                || on_segment(a.3, a.4, b.3)
                || on_segment(a.3, a.4, b.4)
                || on_segment(b.3, b.4, a.3)
                || on_segment(b.3, b.4, a.4)
            {
                return Err("Polygon has intersecting or touching rings".into());
            }
        }
    }
    let contains = |ring: &[[f64; 2]], p: [f64; 2]| {
        let mut inside = false;
        for e in ring.windows(2) {
            let [a, b] = [e[0], e[1]];
            if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
                inside = !inside;
            }
        }
        inside
    };
    let exterior = projected.first().ok_or("Polygon exterior")?;
    for (i, hole) in projected.iter().enumerate().skip(1) {
        let p = *hole.first().ok_or("Polygon hole")?;
        if !contains(exterior, p)
            || projected
                .iter()
                .enumerate()
                .skip(1)
                .any(|(j, other)| i != j && contains(other, p))
        {
            return Err("Polygon has an exterior or nested courtyard".into());
        }
    }
    let mut ys: Vec<_> = rings.iter().flatten().map(|p| project(*p)[1]).collect();
    ys.sort_by(f64::total_cmp);
    ys.dedup_by(|a, b| (*a - *b).abs() < 1e-8);
    let mut triangles = Vec::new();
    for span in ys.windows(2) {
        let [lo, hi] = [span[0], span[1]];
        let mid = (lo + hi) * 0.5;
        let mut edges = Vec::new();
        for edge in rings.iter().flat_map(|r| r.windows(2)) {
            let [a, b] = [project(edge[0]), project(edge[1])];
            if (a[1] <= mid && b[1] > mid) || (b[1] <= mid && a[1] > mid) {
                let x = a[0] + (b[0] - a[0]) * (mid - a[1]) / (b[1] - a[1]);
                edges.push((x, edge[0], edge[1], a[1], b[1]));
            }
        }
        edges.sort_by(|a, b| a.0.total_cmp(&b.0));
        if edges.len() % 2 != 0 {
            return Err("Polygon has unmatched boundaries".into());
        }
        for pair in edges.as_chunks::<2>().0 {
            let point = |edge: &(f64, DVec3, DVec3, f64, f64), y: f64| {
                edge.1 + (edge.2 - edge.1) * ((y - edge.3) / (edge.4 - edge.3))
            };
            let [a, b, c, d] = [
                point(&pair[0], lo),
                point(&pair[1], lo),
                point(&pair[1], hi),
                point(&pair[0], hi),
            ];
            for mut t in [[a, b, c], [a, c, d]] {
                let n = (t[1] - t[0]).cross(t[2] - t[0]);
                if n.length_squared() < 1e-12 {
                    continue;
                }
                if n.dot(normal) < 0. {
                    t.swap(1, 2);
                }
                triangles.push(t);
                if triangles.len() > 100_000 {
                    return Err("Polygon exceeds triangle budget".into());
                }
            }
        }
    }
    if triangles.is_empty() {
        return Err("Empty polygon tessellation".into());
    }
    Ok(triangles)
}
#[derive(Default)]
struct Cell {
    proxies: Vec<[DVec3; 2]>,
    triangles: u64,
}
pub(super) struct Writer {
    out: PathBuf,
    cells: BTreeMap<(i32, i32), Cell>,
    scratch_bytes: u64,
    bytes: u64,
    limit: u64,
}
struct Group {
    tile: Tile,
    proxies: Vec<[DVec3; 2]>,
}
fn representatives(mut proxies: Vec<[DVec3; 2]>) -> Vec<[DVec3; 2]> {
    proxies.sort_by(|a, b| {
        (b[1].y - b[0].y)
            .total_cmp(&(a[1].y - a[0].y))
            .then_with(|| a[0].x.total_cmp(&b[0].x))
            .then_with(|| a[0].z.total_cmp(&b[0].z))
    });
    proxies.truncate(256);
    proxies
}
fn boxes(proxies: &[[DVec3; 2]]) -> Vec<Triangle> {
    let mut out = Vec::new();
    for [lo, hi] in proxies {
        let [a, b, c, d, e, f, g, h] = [
            DVec3::new(lo.x, lo.y, lo.z),
            DVec3::new(hi.x, lo.y, lo.z),
            DVec3::new(hi.x, lo.y, hi.z),
            DVec3::new(lo.x, lo.y, hi.z),
            DVec3::new(lo.x, hi.y, lo.z),
            DVec3::new(hi.x, hi.y, lo.z),
            DVec3::new(hi.x, hi.y, hi.z),
            DVec3::new(lo.x, hi.y, hi.z),
        ];
        out.extend([
            [a, b, f],
            [a, f, e],
            [b, c, g],
            [b, g, f],
            [c, d, h],
            [c, h, g],
            [d, a, e],
            [d, e, h],
            [e, f, g],
            [e, g, h],
        ]);
    }
    out
}
impl Writer {
    pub(super) fn new(out: &Path, limit: u64) -> Result<Self, String> {
        std::fs::create_dir_all(out.join("tiles")).map_err(|e| e.to_string())?;
        std::fs::create_dir(out.join("scratch")).map_err(|e| e.to_string())?;
        Ok(Self {
            out: out.into(),
            cells: BTreeMap::new(),
            scratch_bytes: 0,
            bytes: 0,
            limit,
        })
    }
    pub(super) fn push(&mut self, triangles: &[Triangle]) -> Result<(), String> {
        if triangles.is_empty() || triangles.len() > 200_000 {
            return Err("Building triangle budget exceeded".into());
        }
        let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
        for p in triangles.iter().flatten() {
            if !p.is_finite() {
                return Err("Nonfinite building vertex".into());
            }
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        let center = (lo + hi) * 0.5;
        let key = ((center.x / 250.).floor() as i32, (center.z / 250.).floor() as i32);
        if self.cells.len() > 20_000
            || self.scratch_bytes.saturating_add(triangles.len() as u64 * 72) > 64_000_000_000
        {
            return Err("City scratch budget exceeded".into());
        }
        // One append handle per building; no whole-city map of open files or vertex buffers.
        let mut file = std::io::BufWriter::new(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.path(key))
                .map_err(|e| e.to_string())?,
        );
        for p in triangles.iter().flatten() {
            for value in p.to_array() {
                file.write_all(&value.to_le_bytes()).map_err(|e| e.to_string())?;
            }
        }
        file.flush().map_err(|e| e.to_string())?;
        let cell = self.cells.entry(key).or_default();
        cell.triangles += triangles.len() as u64;
        cell.proxies.push([lo, hi]);
        if cell.proxies.len() >= 512 {
            cell.proxies = representatives(std::mem::take(&mut cell.proxies));
        }
        self.scratch_bytes += triangles.len() as u64 * 72;
        Ok(())
    }
    fn path(&self, (x, y): (i32, i32)) -> PathBuf {
        self.out.join(format!("scratch/{x}-{y}"))
    }
    fn content(&mut self, triangles: &[Triangle], error: f64) -> Result<Content, String> {
        let bytes = city::encode(triangles, error).or_else(|_| city::encode(triangles, 0.))?;
        if bytes.len() > tiles3d::MAX_TILE_CONTENT_BYTES
            || self.bytes.saturating_add(bytes.len() as u64) > self.limit
        {
            return Err("NYC building budget exceeded; reduce the showcase bounds or geometry detail before packaging".into());
        }
        let uri = format!("tiles/{}.glb", self.bytes);
        self.bytes += bytes.len() as u64;
        std::fs::write(self.out.join(&uri), bytes).map_err(|e| e.to_string())?;
        Ok(Content { uri })
    }
    pub(super) fn finish(
        mut self,
        credit: &str,
        extras: serde_json::Value,
    ) -> Result<(Tileset, u64, usize), String> {
        if self.cells.is_empty() {
            return Err("No buildings were converted".into());
        }
        let (transform, _) = frame()?;
        let mut groups = Vec::new();
        let mut tile_count = 0;
        for (key, cell) in std::mem::take(&mut self.cells) {
            let mut reader = BufReader::new(File::open(self.path(key)).map_err(|e| e.to_string())?);
            let mut leaves = Vec::new();
            let mut remaining = cell.triangles;
            while remaining > 0 {
                let count = remaining.min(20_000) as usize;
                let mut triangles = Vec::with_capacity(count);
                for _ in 0..count {
                    let mut points = [DVec3::ZERO; 3];
                    for p in &mut points {
                        let mut xyz = [0.; 3];
                        for v in &mut xyz {
                            let mut bytes = [0; 8];
                            reader.read_exact(&mut bytes).map_err(|e| e.to_string())?;
                            *v = f64::from_le_bytes(bytes);
                        }
                        *p = DVec3::from_array(xyz);
                    }
                    triangles.push(points);
                }
                remaining -= count as u64;
                let bounds = city::cell_volume(&triangles, transform)?;
                let mut child = None;
                for error in [0., 2.] {
                    let content = self.content(&triangles, error)?;
                    child = Some(Tile {
                        bounding_volume: bounds.clone(),
                        geometric_error: error * 2.,
                        refine: Some(Refine::Replace),
                        transform: None,
                        content: Some(content),
                        children: child.into_iter().collect(),
                    });
                    tile_count += 1;
                }
                if let Some(tile) = child {
                    leaves.push(tile);
                }
            }
            std::fs::remove_file(self.path(key)).map_err(|e| e.to_string())?;
            while leaves.len() > 64 {
                leaves = leaves.chunks(64).map(|c| city::parent(c.to_vec())).collect();
            }
            let proxies = representatives(cell.proxies);
            let proxy = boxes(&proxies);
            let mut tile = city::parent(leaves);
            tile.content = Some(self.content(&proxy, 0.)?);
            tile.refine = Some(Refine::Replace);
            // Omitted small buildings and rectangular distant roofs are bounded by this error.
            tile.geometric_error = 1024.;
            groups.push(Group { tile, proxies });
            tile_count += 1;
            if tile_count > 70_000 {
                return Err("City tileset node budget exceeded".into());
            }
        }
        while groups.len() > 1 {
            let mut next = Vec::new();
            for chunk in groups.chunks(4) {
                let proxies = representatives(chunk.iter().flat_map(|g| g.proxies.iter().copied()).collect());
                let mut tile = city::parent(chunk.iter().map(|g| g.tile.clone()).collect());
                tile.geometric_error = chunk.iter().map(|g| g.tile.geometric_error).fold(0., f64::max) * 2.;
                tile.content = Some(self.content(&boxes(&proxies), 0.)?);
                tile.refine = Some(Refine::Replace);
                next.push(Group { tile, proxies });
                tile_count += 1;
            }
            groups = next;
        }
        let mut root = groups.pop().ok_or("Missing city root")?.tile;
        root.transform = Some(transform);
        let set = Tileset {
            asset: Asset {
                version: "1.1".into(),
                copyright: Some(credit.into()),
            },
            geometric_error: root.geometric_error,
            root,
            extensions_required: Vec::new(),
            extras: Some(extras),
        };
        let encoded = serde_json::to_vec(&set).map_err(|e| e.to_string())?;
        Tileset::from_json(&encoded).map_err(|e| e.to_string())?;
        self.bytes += encoded.len() as u64;
        if self.bytes > self.limit {
            return Err("City manifest exceeds building budget".into());
        }
        std::fs::write(self.out.join("tileset.json"), encoded).map_err(|e| e.to_string())?;
        std::fs::remove_dir(self.out.join("scratch")).map_err(|e| e.to_string())?;
        Ok((set, self.bytes, tile_count))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tessellation_rejects_crossing_boundaries_and_outside_courtyards() {
        let ring = |p: &[[f64; 2]]| p.iter().map(|[x, z]| DVec3::new(*x, 0., *z)).collect::<Vec<_>>();
        let outer = ring(&[[0., 0.], [10., 0.], [10., 10.], [0., 10.], [0., 0.]]);
        let outside = ring(&[[20., 20.], [21., 20.], [21., 21.], [20., 21.], [20., 20.]]);
        assert!(triangulate(&[outer.clone(), outside]).is_err());
        let crossing = ring(&[[0., 0.], [10., 0.], [0., 10.], [10., 10.], [5., -2.], [0., 0.]]);
        assert!(triangulate(&[crossing]).is_err());
        let touching = ring(&[[0., 3.], [3., 3.], [3., 7.], [0., 7.], [0., 3.]]);
        assert!(triangulate(&[outer, touching]).is_err());
    }
    #[test]
    fn tessellation_preserves_courtyards_concavity_and_sloping_roofs() {
        let ring = |pairs: &[[f64; 2]]| {
            pairs
                .iter()
                .map(|[x, z]| DVec3::new(*x, 10. + x * 0.2, *z))
                .collect::<Vec<_>>()
        };
        let outer = ring(&[[0., 0.], [10., 0.], [10., 10.], [0., 10.], [0., 0.]]);
        let hole = ring(&[[3., 3.], [7., 3.], [7., 7.], [3., 7.], [3., 3.]]);
        let triangles = triangulate(&[outer, hole]).unwrap();
        let area: f64 = triangles
            .iter()
            .map(|[a, b, c]| (b - a).cross(*c - *a).y.abs() * 0.5)
            .sum();
        assert!((area - 84.).abs() < 1e-6);
        for p in triangles.iter().flatten() {
            assert!((p.y - 10. - p.x * 0.2).abs() < 1e-7);
        }
        let concave = ring(&[
            [0., 0.],
            [4., 0.],
            [4., 2.],
            [2., 2.],
            [2., 4.],
            [0., 4.],
            [0., 0.],
        ]);
        let area: f64 = triangulate(&[concave])
            .unwrap()
            .iter()
            .map(|[a, b, c]| (b - a).cross(*c - *a).y.abs() * 0.5)
            .sum();
        assert!((area - 12.).abs() < 1e-6);
    }
}
