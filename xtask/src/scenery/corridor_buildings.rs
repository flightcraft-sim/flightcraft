//! Lidar blocks -> measured buildings and trees -> compact building tiles for the corridor.
//!
//! The 1N7 builder meshes each building with its own texture. A city needs the opposite: the
//! corridor processes the NY_NewYorkCity point cloud in bounded 2 km blocks, and every 256 m cell
//! becomes one GLB whose single small atlas holds procedural facade strips and a measured roof
//! palette. A coarser 1 km parent tile (heights kept per 4 m cell, so towers survive) lets
//! distance selection avoid loading fine geometry. All geometry is reconstructed from the
//! classified lidar; no footprint or height from any other source is used.
use super::{
    buildings::{self, Building, Plane},
    corridor::{self, Plan},
    corridor_atlas::{self as atlas, Atlas},
    corridor_glb::{Frame, Mesh, Texture, write_glb},
    dem::Dems,
    download, ept, lidar, lidar_ground,
    surface::Grid,
    trees::{self, Tree},
};
use flightcraft_world::{
    geoid::GeoidGrid,
    scenery::{self, TileKey},
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub(super) const CELL_M: f64 = 256.;
pub(super) const PARENT_M: f64 = 1024.;
pub(super) const BLOCK_M: f64 = 2048.;
const MARGIN_M: f64 = 96.;
/// Plane clutter smaller than this (cells) is absorbed into its neighbours' plane.
const MIN_PLANE_CELLS: usize = 6;
const WALL_SINK_M: f64 = 1.2;

#[derive(Clone, Copy, Debug)]
pub(super) struct Bounds {
    pub rect: [f64; 4],
    pub lo: f64,
    pub hi: f64,
}
impl Bounds {
    fn new() -> Self {
        Self {
            rect: [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY],
            lo: f64::INFINITY,
            hi: f64::NEG_INFINITY,
        }
    }
    fn add(&mut self, x: f64, y: f64, h: f64) {
        self.rect = [
            self.rect[0].min(x),
            self.rect[1].min(y),
            self.rect[2].max(x),
            self.rect[3].max(y),
        ];
        self.lo = self.lo.min(h);
        self.hi = self.hi.max(h);
    }
    pub fn union(&mut self, other: &Bounds) {
        self.rect = [
            self.rect[0].min(other.rect[0]),
            self.rect[1].min(other.rect[1]),
            self.rect[2].max(other.rect[2]),
            self.rect[3].max(other.rect[3]),
        ];
        self.lo = self.lo.min(other.lo);
        self.hi = self.hi.max(other.hi);
    }
    pub fn valid(&self) -> bool {
        self.rect[0] <= self.rect[2] && self.lo <= self.hi
    }
}
#[derive(Clone, Debug)]
pub(super) struct TileMeta {
    pub uri: String,
    /// 1 = full-detail 256 m cell, 0 = coarse 1 km parent.
    pub lod: u8,
    pub cell: (i64, i64),
    pub bounds: Bounds,
    pub bytes: u64,
    pub vertices: usize,
    pub buildings: usize,
}

/// Decoded NAIP at the finest available level, with a small LRU, sampled by Mercator position.
pub(super) struct Imagery<'a> {
    plan: &'a Plan,
    request: &'a super::Request,
    leaves: BTreeMap<TileKey, &'a super::Input>,
    cache: Vec<(TileKey, image::RgbaImage)>,
}
impl<'a> Imagery<'a> {
    pub fn new(plan: &'a Plan, request: &'a super::Request, sources: &'a super::Sources) -> Self {
        Self {
            plan,
            request,
            leaves: sources
                .inputs
                .iter()
                .filter(|i| i.kind == super::InputKind::Imagery)
                .filter_map(|i| Some((i.key?, i)))
                .collect(),
            cache: Vec::new(),
        }
    }
    pub fn image(&mut self, key: TileKey) -> Result<&image::RgbaImage, String> {
        if let Some(i) = self.cache.iter().position(|(k, _)| *k == key) {
            let entry = self.cache.remove(i);
            self.cache.push(entry);
        } else {
            let input = self.leaves.get(&key).ok_or("imagery leaf missing")?;
            let image =
                super::decode_image(&super::input_bytes(self.request, input)?, Some(corridor::TILE_PX))?;
            if self.cache.len() >= 24 {
                self.cache.remove(0);
            }
            self.cache.push((key, image));
        }
        self.cache
            .last()
            .map(|(_, i)| i)
            .ok_or_else(|| "imagery cache".into())
    }
    /// RGB (0..255) at a Mercator position from the finest fetched leaf covering it.
    pub fn color(&mut self, x: f64, y: f64) -> Option<[f32; 3]> {
        let root = self.plan.root;
        for level in (corridor::FOREST + 1..=corridor::FINEST).rev() {
            let size = (root[2] - root[0]) / f64::from(1_u32 << level);
            let (fx, fy) = ((x - root[0]) / size, (root[3] - y) / size);
            if fx < 0. || fy < 0. {
                return None;
            }
            let key = TileKey {
                level,
                x: fx as u32,
                y: fy as u32,
            };
            if self.leaves.contains_key(&key) {
                let image = self.image(key).ok()?;
                let px = (((fx - libm::floor(fx)) * 512.) as u32).min(511);
                let py = (((fy - libm::floor(fy)) * 512.) as u32).min(511);
                let p = image.get_pixel(px, py).0;
                return Some([f32::from(p[0]), f32::from(p[1]), f32::from(p[2])]);
            }
        }
        None
    }
}

struct Ctx<'a> {
    grid: &'a Grid,
    frame: &'a Frame,
}
impl Ctx<'_> {
    /// Cell-index coordinates (cell centres at integers) to the local frame.
    fn vertex(&self, ux: f64, uy: f64, msl: f64, acc: &mut Bounds) -> Result<[f32; 3], String> {
        let mx = self.grid.bounds[0] + (ux + 0.5) * self.grid.step;
        let my = self.grid.bounds[1] + (uy + 0.5) * self.grid.step;
        let [lat, lon] = scenery::inverse_mercator(mx, my).map_err(|e| e.to_string())?;
        let (p, h) = self.frame.local_h(lat, lon, msl)?;
        acc.add(mx, my, h);
        Ok(p)
    }
}

/// A roof raster: either the 1 m measured cells or 4 m cells of per-cell maximum height.
struct Raster {
    step: i32,
    cells: BTreeMap<(i32, i32), (u32, f32)>,
    planes: Vec<Plane>,
    colors: Vec<u32>,
}
fn flat(z: f64) -> Plane {
    Plane {
        a: 0.,
        b: 0.,
        c: z,
        rms_m: 0.,
        samples: 0,
    }
}
fn plane_colors(b: &Building, grid: &Grid, imagery: &mut Imagery<'_>) -> Vec<u32> {
    let mut sums = vec![([0_f32; 3], 0_f32); b.roof_planes.len()];
    for (n, (&index, &plane)) in b.cells.iter().enumerate() {
        if n % 3 != 0 {
            continue;
        }
        let (x, y) = ((index % grid.width) as f64, (index / grid.width) as f64);
        let mx = grid.bounds[0] + (x + 0.5) * grid.step;
        let my = grid.bounds[1] + (y + 0.5) * grid.step;
        if let (Some(c), Some(s)) = (imagery.color(mx, my), sums.get_mut(plane)) {
            for (acc, v) in s.0.iter_mut().zip(c) {
                *acc += v;
            }
            s.1 += 1.;
        }
    }
    sums.iter()
        .map(|(c, n)| {
            if *n > 0. {
                atlas::palette_index(c.map(|v| v / n))
            } else {
                atlas::palette_index([118., 116., 112.])
            }
        })
        .collect()
}
/// Absorbs tiny roof-plane fragments (rooftop equipment, parapets) into the surrounding plane.
fn simplified_cells(b: &Building, grid: &Grid) -> BTreeMap<usize, usize> {
    let mut count = vec![0_usize; b.roof_planes.len()];
    for p in b.cells.values() {
        if let Some(c) = count.get_mut(*p) {
            *c += 1;
        }
    }
    let mut cells = b.cells.clone();
    for (&index, &plane) in &b.cells {
        if count.get(plane).copied().unwrap_or(0) >= MIN_PLANE_CELLS {
            continue;
        }
        let (x, y) = ((index % grid.width) as i64, (index / grid.width) as i64);
        let mut votes: BTreeMap<usize, usize> = BTreeMap::new();
        for dy in -1..=1_i64 {
            for dx in -1..=1_i64 {
                let (nx, ny) = (x + dx, y + dy);
                if (dx, dy) == (0, 0) || nx < 0 || ny < 0 || nx >= grid.width as i64 {
                    continue;
                }
                if let Some(&q) = b.cells.get(&(ny as usize * grid.width + nx as usize))
                    && q != plane
                    && count.get(q).copied().unwrap_or(0) >= MIN_PLANE_CELLS
                {
                    let z0 = b.roof_planes[plane].z(x as f64, y as f64);
                    let z1 = b.roof_planes[q].z(x as f64, y as f64);
                    if (z0 - z1).abs() <= 2.5 {
                        *votes.entry(q).or_default() += 1;
                    }
                }
            }
        }
        if let Some((&best, _)) = votes.iter().max_by_key(|(_, v)| **v) {
            cells.insert(index, best);
        }
    }
    cells
}
fn raster_fine(b: &Building, grid: &Grid, imagery: &mut Imagery<'_>) -> Raster {
    let cells = simplified_cells(b, grid);
    Raster {
        step: 1,
        cells: cells
            .iter()
            .map(|(&i, &p)| {
                (
                    ((i % grid.width) as i32, (i / grid.width) as i32),
                    (p as u32, grid.cells.get(i).map_or(0., |c| c.ground)),
                )
            })
            .collect(),
        planes: b.roof_planes.clone(),
        colors: plane_colors(b, grid, imagery),
    }
}
fn raster_coarse(b: &Building, grid: &Grid, imagery: &mut Imagery<'_>) -> Raster {
    const S: i32 = 4;
    let colors = plane_colors(b, grid, imagery);
    // Highest roof in each 4 m cell wins, so a tower on a podium survives.
    let mut top: BTreeMap<(i32, i32), (f64, f32, u32)> = BTreeMap::new();
    for (&index, &plane) in &b.cells {
        let (x, y) = ((index % grid.width) as i32, (index / grid.width) as i32);
        let z = b.roof_planes[plane].z(f64::from(x), f64::from(y));
        let ground = grid.cells.get(index).map_or(0., |c| c.ground);
        let e =
            top.entry((x.div_euclid(S), y.div_euclid(S)))
                .or_insert((f64::NEG_INFINITY, f32::INFINITY, 0));
        e.1 = e.1.min(ground);
        if z > e.0 {
            e.0 = z;
            e.2 = colors.get(plane).copied().unwrap_or(0);
        }
    }
    let mut planes: Vec<Plane> = Vec::new();
    let mut out_colors = Vec::new();
    let mut ids: BTreeMap<(i64, u32), u32> = BTreeMap::new();
    let mut cells = BTreeMap::new();
    for (key, (z, ground, color)) in top {
        let level = libm::round(z / 2.5) as i64;
        let id = *ids.entry((level, color)).or_insert_with(|| {
            planes.push(flat(level as f64 * 2.5));
            out_colors.push(color);
            planes.len() as u32 - 1
        });
        cells.insert(key, (id, ground));
    }
    Raster {
        step: S,
        cells,
        planes,
        colors: out_colors,
    }
}

/// Greedy rectangles of equal-plane raster cells (rows merged while their runs match exactly).
fn rects(r: &Raster) -> Vec<(i32, i32, i32, i32, u32)> {
    let mut rows: BTreeMap<i32, Vec<(i32, u32)>> = BTreeMap::new();
    for (&(x, y), &(p, _)) in &r.cells {
        rows.entry(y).or_default().push((x, p));
    }
    type Rect = (i32, i32, i32, i32, u32);
    let mut active: BTreeMap<(i32, i32, u32), Rect> = BTreeMap::new();
    let mut out = Vec::new();
    let mut previous: Option<i32> = None;
    for (y, mut cells) in rows {
        cells.sort();
        if previous.is_some_and(|last| last + 1 != y) {
            out.extend(std::mem::take(&mut active).into_values());
        }
        let mut runs: Vec<(i32, i32, u32)> = Vec::new();
        for (x, p) in cells {
            match runs.last_mut() {
                Some(run) if run.2 == p && run.1 + 1 == x => run.1 = x,
                _ => runs.push((x, x, p)),
            }
        }
        let mut next = BTreeMap::new();
        for (x0, x1, p) in runs {
            let key = (x0, x1, p);
            let rect = match active.remove(&key) {
                Some(mut r) => {
                    r.3 = y + 1;
                    r
                }
                None => (x0, y, x1 + 1, y + 1, p),
            };
            next.insert(key, rect);
        }
        out.extend(active.into_values());
        active = next;
        previous = Some(y);
    }
    out.extend(active.into_values());
    out
}

struct Edge {
    dir: u8,
    line: i32,
    along: i32,
    plane: u32,
    under: Option<u32>,
    ground: f32,
}
#[allow(clippy::too_many_arguments)]
fn mesh_raster(
    ctx: &Ctx<'_>,
    r: &Raster,
    atlas: &Atlas,
    style: u8,
    class: u8,
    bay_m: f64,
    mesh: &mut Mesh,
    acc: &mut Bounds,
) -> Result<(), String> {
    let s = f64::from(r.step);
    let edge = |re: i32| f64::from(re) * s - 0.5;
    // Roofs.
    for (x0, y0, x1, y1, p) in rects(r) {
        let plane = *r.planes.get(p as usize).ok_or("roof plane")?;
        let uv = atlas.palette_uv(r.colors.get(p as usize).copied().unwrap_or(0));
        let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
        let mut v = [[0.; 3]; 4];
        for (k, (cx, cy)) in corners.iter().enumerate() {
            let (ux, uy) = (edge(*cx), edge(*cy));
            v[k] = ctx.vertex(ux, uy, plane.z(ux, uy), acc)?;
        }
        mesh.quad(v, [uv; 4]);
    }
    // Walls: boundary edges against ground or a lower roof plane, merged along their run.
    let mut edges = Vec::new();
    for (&(x, y), &(p, ground)) in &r.cells {
        for (dir, (dx, dy)) in [(0, -1), (1, 0), (0, 1), (-1, 0)].into_iter().enumerate() {
            let neighbour = r.cells.get(&(x + dx, y + dy)).map(|c| c.0);
            let (line, along) = match dir {
                0 => (y, x),
                1 => (x + 1, y),
                2 => (y + 1, x),
                _ => (x, y),
            };
            match neighbour {
                Some(q) if q == p => {}
                Some(q) => {
                    // Draw the step once, from the higher roof; probe at the edge midpoint.
                    let (mx, my) = match dir {
                        0 | 2 => (f64::from(along) + 0.5, f64::from(line)),
                        _ => (f64::from(line), f64::from(along) + 0.5),
                    };
                    let (ux, uy) = (edge_f(mx, s), edge_f(my, s));
                    let zp = r.planes.get(p as usize).map_or(0., |pl| pl.z(ux, uy));
                    let zq = r.planes.get(q as usize).map_or(0., |pl| pl.z(ux, uy));
                    if zp > zq + 0.15 {
                        edges.push(Edge {
                            dir: dir as u8,
                            line,
                            along,
                            plane: p,
                            under: Some(q),
                            ground,
                        });
                    }
                }
                None => edges.push(Edge {
                    dir: dir as u8,
                    line,
                    along,
                    plane: p,
                    under: None,
                    ground,
                }),
            }
        }
    }
    edges.sort_by_key(|e| (e.dir, e.line, e.plane, e.under, e.along));
    let mut i = 0;
    while i < edges.len() {
        let first = &edges[i];
        let mut j = i;
        while j + 1 < edges.len()
            && edges[j + 1].dir == first.dir
            && edges[j + 1].line == first.line
            && edges[j + 1].plane == first.plane
            && edges[j + 1].under == first.under
            && edges[j + 1].along == edges[j].along + 1
        {
            j += 1;
        }
        let last = &edges[j];
        let (a0, a1) = (first.along, last.along + 1);
        let (pa, pb) = if first.dir % 2 == 0 {
            ((a0, first.line), (a1, first.line))
        } else {
            ((first.line, a0), (first.line, a1))
        };
        let top = *r.planes.get(first.plane as usize).ok_or("wall plane")?;
        let under = first.under.and_then(|q| r.planes.get(q as usize)).copied();
        wall(
            ctx,
            (edge(pa.0), edge(pa.1)),
            (edge(pb.0), edge(pb.1)),
            top,
            under,
            (f64::from(first.ground), f64::from(last.ground)),
            f64::from(a1 - a0) * s,
            (atlas, style, class),
            bay_m,
            mesh,
            acc,
        )?;
        i = j + 1;
    }
    Ok(())
}
fn edge_f(re: f64, s: f64) -> f64 {
    re * s - 0.5
}
#[allow(clippy::too_many_arguments)]
fn wall(
    ctx: &Ctx<'_>,
    a: (f64, f64),
    b: (f64, f64),
    top: Plane,
    under: Option<Plane>,
    ground: (f64, f64),
    length_m: f64,
    look: (&Atlas, u8, u8),
    bay_m: f64,
    mesh: &mut Mesh,
    acc: &mut Bounds,
) -> Result<(), String> {
    let (atlas, style, class) = look;
    let bays = (length_m / bay_m).ceil().clamp(1., 96.) as u32;
    for k in 0..bays {
        let (t0, t1) = (f64::from(k) / f64::from(bays), f64::from(k + 1) / f64::from(bays));
        let at = |t: f64| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
        let (p0, p1) = (at(t0), at(t1));
        let g0 = ground.0 + (ground.1 - ground.0) * t0;
        let g1 = ground.0 + (ground.1 - ground.0) * t1;
        let (zt0, zt1) = (top.z(p0.0, p0.1), top.z(p1.0, p1.1));
        let (zb0, zb1) = match under {
            Some(u) => (u.z(p0.0, p0.1).min(zt0), u.z(p1.0, p1.1).min(zt1)),
            None => (g0 - WALL_SINK_M, g1 - WALL_SINK_M),
        };
        if (zt0 - zb0) < 0.15 && (zt1 - zb1) < 0.15 {
            continue;
        }
        let h = |z: f64, g: f64| (z - g).max(0.);
        let (hb0, hb1) = if under.is_some() {
            (h(zb0, g0), h(zb1, g1))
        } else {
            (0., 0.)
        };
        let v = [
            ctx.vertex(p0.0, p0.1, zb0, acc)?,
            ctx.vertex(p1.0, p1.1, zb1, acc)?,
            ctx.vertex(p1.0, p1.1, zt1, acc)?,
            ctx.vertex(p0.0, p0.1, zt0, acc)?,
        ];
        let uv = [
            atlas.wall_uv(style, class, 0., hb0),
            atlas.wall_uv(style, class, 1., hb1),
            atlas.wall_uv(style, class, 1., h(zt1, g1)),
            atlas.wall_uv(style, class, 0., h(zt0, g0)),
        ];
        mesh.quad(v, uv);
    }
    Ok(())
}

fn seed(b: &Building) -> u64 {
    b.center_lat_lon[0].to_bits() ^ b.center_lat_lon[1].to_bits().rotate_left(21)
}

/// Aggregated evidence over every block, for the lidar-acceptance and registration gates.
#[derive(Default)]
pub(super) struct Totals {
    pub nodes_decoded: BTreeSet<ept::Key>,
    pub node_points: BTreeMap<ept::Key, u64>,
    pub classes: BTreeMap<u8, u64>,
    pub ground: lidar_ground::Check,
    pub buildings: usize,
    pub planes: usize,
    pub plane_rms: Vec<f32>,
    pub roof_cells_measured: u64,
    pub roof_cells_interpolated: u64,
    pub classified_buildings: usize,
    pub building_heights: Vec<f32>,
    pub trees: usize,
    pub tree_heights: Vec<f32>,
    pub dem_missing_cells: u64,
    pub skipped_blocks: Vec<String>,
    pub excluded_roof_cells: u64,
    pub registration: Registration,
    pub bytes_fine: u64,
    pub bytes_coarse: u64,
    pub max_tile_vertices: usize,
    /// A spread sample of lidar water-classified cells (lat, lon): an independent check of the
    /// NHD polygons' registration.
    pub water_cells: Vec<[f64; 2]>,
}
/// Footprint-versus-NAIP contrast summed over shifts (metres east, north), -5..=5.
pub(super) const REG_R: usize = 5;
pub(super) const REG_N: usize = 2 * REG_R + 1;
pub(super) struct Registration {
    pub sum: [[f64; REG_N]; REG_N],
    pub buildings: usize,
    pub per_block_best: Vec<[i32; 2]>,
}
impl Default for Registration {
    fn default() -> Self {
        Self {
            sum: [[0.; REG_N]; REG_N],
            buildings: 0,
            per_block_best: Vec::new(),
        }
    }
}

pub(super) struct BlockSpec {
    pub core: [f64; 4],
    pub grid: [f64; 4],
}
pub(super) fn blocks(plan: &Plan) -> Vec<BlockSpec> {
    let env = plan.envelope();
    let [rw, rs, ..] = plan.root;
    let (i0, i1) = (
        ((env[0] - rw) / BLOCK_M).floor() as i64,
        ((env[2] - rw) / BLOCK_M).ceil() as i64,
    );
    let (j0, j1) = (
        ((env[1] - rs) / BLOCK_M).floor() as i64,
        ((env[3] - rs) / BLOCK_M).ceil() as i64,
    );
    let mut out = Vec::new();
    for j in j0..j1 {
        for i in i0..i1 {
            let core = [
                rw + i as f64 * BLOCK_M,
                rs + j as f64 * BLOCK_M,
                rw + (i + 1) as f64 * BLOCK_M,
                rs + (j + 1) as f64 * BLOCK_M,
            ];
            if plan.lidar_wants(core) {
                let m = MARGIN_M / 0.7576;
                out.push(BlockSpec {
                    core,
                    grid: [core[0] - m, core[1] - m, core[2] + m, core[3] + m],
                });
            }
        }
    }
    out
}

pub(super) struct NodeRef {
    pub key: ept::Key,
    pub rect: [f64; 4],
    pub path: PathBuf,
}
/// Receipt-verified EPT nodes of the NY_NewYorkCity project.
pub(super) fn nodes(request: &super::Request, sources: &super::Sources) -> Result<Vec<NodeRef>, String> {
    let root = sources
        .inputs
        .iter()
        .find(|i| i.kind == super::InputKind::EptMetadata)
        .ok_or("corridor build needs the EPT metadata receipt")?;
    if root.source.url != format!("{}/ept.json", ept::NYC_ROOT) {
        return Err("corridor buildings come only from the NY_NewYorkCity EPT".into());
    }
    let metadata = ept::Metadata::parse(&super::input_bytes(request, root)?)?;
    let prefix = format!("{}/ept-data/", ept::NYC_ROOT);
    let mut out = Vec::new();
    for input in sources
        .inputs
        .iter()
        .filter(|i| i.kind == super::InputKind::EptLidar)
    {
        let name = input
            .source
            .url
            .strip_prefix(&prefix)
            .and_then(|v| v.strip_suffix(".laz"))
            .ok_or("EPT point source is outside the NYC project")?;
        let key = ept::Key::parse(name)?;
        let path = request
            .cache
            .join("objects")
            .join(download::sha256(input.source.url.as_bytes()));
        let (bytes, hash) = super::transport::digest(&path)?;
        if bytes != input.source.bytes || hash != input.source.sha256 {
            return Err(format!("EPT node {name} differs from its receipt"));
        }
        out.push(NodeRef {
            key,
            rect: metadata.node_rect(key),
            path,
        });
    }
    out.sort_by_key(|n| n.key);
    Ok(out)
}

fn intersects(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] <= b[2] && a[2] >= b[0] && a[1] <= b[3] && a[3] >= b[1]
}

pub(super) struct Output<'a> {
    pub out: &'a Path,
    pub plan: &'a Plan,
    pub frame: &'a Frame,
    pub geoid: &'a GeoidGrid,
}

/// Processes one block; returns its tiles and trees. Building GLBs are written immediately.
pub(super) fn process_block(
    spec: &BlockSpec,
    nodes: &[NodeRef],
    o: &Output<'_>,
    dem: &mut Dems,
    imagery: &mut Imagery<'_>,
    totals: &mut Totals,
) -> Result<(Vec<TileMeta>, Vec<Tree>), String> {
    let mut grid = Grid::new(spec.grid)?;
    let mut ground = lidar_ground::Check::default();
    let mut decoded = 0;
    // Developer probe: per-class point count and Z range within 40 m of the Empire State Building.
    let debug_probe =
        std::env::var_os("FLIGHTCRAFT_CORRIDOR_DEBUG").map(|_| corridor::merc(40.74844, -73.98566));
    let mut probe: BTreeMap<u8, (u64, f64, f64)> = BTreeMap::new();
    for node in nodes.iter().filter(|n| intersects(n.rect, spec.grid)) {
        let file = std::fs::File::open(&node.path).map_err(|e| e.to_string())?;
        let mut points = 0_u64;
        let audit = lidar::scan(std::io::BufReader::new(file), |p| {
            points += 1;
            if debug_probe.is_some_and(|c| libm::hypot(p.xyz[0] - c[0], p.xyz[1] - c[1]) < 40.) {
                let e = probe
                    .entry(p.classification)
                    .or_insert((0_u64, f64::NEG_INFINITY, f64::INFINITY));
                e.0 += 1;
                e.1 = e.1.max(p.xyz[2]);
                e.2 = e.2.min(p.xyz[2]);
            }
            ground.sample(p, spec.core, dem, Some(o.geoid))?;
            grid.insert(p)
        })?;
        if totals.nodes_decoded.insert(node.key) {
            totals.node_points.insert(node.key, audit.header.point_count);
            for (class, count) in &audit.classifications {
                *totals.classes.entry(*class).or_default() += count;
            }
        }
        decoded += 1;
    }
    totals.ground.absorb(ground);
    if debug_probe.is_some() {
        eprintln!("class (count, max z, min z) within 40 m of the Empire State Building: {probe:?}");
    }
    if decoded == 0 {
        totals
            .skipped_blocks
            .push(format!("{:?}: no EPT nodes", spec.core));
        return Ok((Vec::new(), Vec::new()));
    }
    // Terrain under populated cells; a cell without a valid 1 m DEM sample is dropped, not guessed.
    let mut missing = 0_u64;
    for index in 0..grid.cells.len() {
        if grid.cells.get(index).is_some_and(|c| c.count > 0) {
            let [lat, lon] = grid.coordinate(index)?;
            let ground = match dem.height_opt(lat, lon)? {
                Some(h) => h as f32,
                None => {
                    missing += 1;
                    f32::NAN
                }
            };
            if let Some(c) = grid.cells.get_mut(index) {
                c.ground = ground;
            }
        }
    }
    totals.dem_missing_cells += missing;
    {
        let water: Vec<usize> = grid
            .cells
            .iter()
            .enumerate()
            .filter(|(i, c)| {
                c.flags & 16 != 0 && {
                    let (x, y) = (i % grid.width, i / grid.width);
                    x >= 96 && y >= 96 && x + 96 < grid.width && y + 96 < grid.height
                }
            })
            .map(|(i, _)| i)
            .collect();
        let stride = (water.len() / 150).max(1);
        for index in water.into_iter().step_by(stride) {
            if let Ok(position) = grid.coordinate(index) {
                totals.water_cells.push(position);
            }
        }
    }
    if std::env::var_os("FLIGHTCRAFT_CORRIDOR_DEBUG").is_some() {
        let populated = grid.cells.iter().filter(|c| c.count > 0).count();
        let roof = grid.cells.iter().filter(|c| c.flags & 4 != 0).count();
        let veg = grid.cells.iter().filter(|c| c.flags & 2 != 0).count();
        let tall = grid
            .cells
            .iter()
            .filter(|c| c.flags & 4 != 0 && c.height() > 2.5)
            .count();
        eprintln!(
            "block grid {}x{}: populated {populated}, class-6 cells {roof}, vegetation cells {veg}, class-6 above 2.5 m {tall}, missing DEM {missing}; classes {:?}",
            grid.width, grid.height, totals.classes
        );
    }
    let found = buildings::detect(&mut grid)?;
    let mut trees = trees::detect(&grid)?;
    let owned = |lat: f64, lon: f64| {
        scenery::mercator(lat, lon).is_ok_and(|[x, y]| {
            x >= spec.core[0] && x < spec.core[2] && y >= spec.core[1] && y < spec.core[3]
        })
    };
    let found: Vec<Building> = found
        .into_iter()
        .filter(|b| owned(b.center_lat_lon[0], b.center_lat_lon[1]))
        .collect();
    trees.retain(|t| {
        let Ok([x, y]) = scenery::mercator(t.lat, t.lon) else {
            return false;
        };
        if !owned(t.lat, t.lon) {
            return false;
        }
        let col = libm::floor((x - grid.bounds[0]) / grid.step) as usize;
        let row = libm::floor((y - grid.bounds[1]) / grid.step) as usize;
        !found
            .iter()
            .any(|b| b.cells.contains_key(&(row * grid.width + col)))
    });
    for b in &found {
        totals.buildings += 1;
        totals.planes += b.roof_planes.len();
        totals
            .plane_rms
            .extend(b.roof_planes.iter().map(|p| p.rms_m as f32));
        totals.roof_cells_measured += b.measured_roof_cells as u64;
        totals.roof_cells_interpolated += b.interpolated_roof_cells as u64;
        totals.excluded_roof_cells += b.excluded_roof_cells as u64;
        totals.classified_buildings += usize::from(!b.inferred_classification);
        totals.building_heights.push(b.height_m as f32);
    }
    totals.trees += trees.len();
    totals
        .tree_heights
        .extend(trees.iter().map(|t| t.height_m as f32));
    registration(&found, &grid, imagery, &mut totals.registration);

    let ctx = Ctx {
        grid: &grid,
        frame: o.frame,
    };
    let root = o.plan.root;
    let cell_of = |b: &Building, size: f64| -> Option<(i64, i64)> {
        let [x, y] = scenery::mercator(b.center_lat_lon[0], b.center_lat_lon[1]).ok()?;
        Some((
            ((x - root[0]) / size).floor() as i64,
            ((y - root[1]) / size).floor() as i64,
        ))
    };
    let mut fine: BTreeMap<(i64, i64), Vec<&Building>> = BTreeMap::new();
    let mut coarse: BTreeMap<(i64, i64), Vec<&Building>> = BTreeMap::new();
    for b in &found {
        if let (Some(f), Some(c)) = (cell_of(b, CELL_M), cell_of(b, PARENT_M)) {
            fine.entry(f).or_default().push(b);
            coarse.entry(c).or_default().push(b);
        }
    }
    let mut tiles = Vec::new();
    for (lod, groups) in [(1_u8, &fine), (0_u8, &coarse)] {
        for (&cell, list) in groups {
            let used: BTreeSet<(u8, u8)> = list
                .iter()
                .map(|b| {
                    (
                        atlas::style_for(b.height_m, seed(b)),
                        atlas::class_for(b.height_m),
                    )
                })
                .collect();
            let atlas = Atlas::new(&used);
            let mut mesh = Mesh::default();
            let mut bounds = Bounds::new();
            let bay = if lod == 1 { 8. } else { 16. };
            for b in list {
                let raster = if lod == 1 {
                    raster_fine(b, &grid, imagery)
                } else {
                    raster_coarse(b, &grid, imagery)
                };
                mesh_raster(
                    &ctx,
                    &raster,
                    &atlas,
                    atlas::style_for(b.height_m, seed(b)),
                    atlas::class_for(b.height_m),
                    bay,
                    &mut mesh,
                    &mut bounds,
                )?;
            }
            if mesh.is_empty() {
                continue;
            }
            let bytes = write_glb(&mesh, Texture::Webp(&atlas.webp()?), None)?;
            let uri = format!("buildings/{lod}/{}_{}.glb", cell.0, cell.1);
            let path = o.out.join(&uri);
            std::fs::create_dir_all(path.parent().ok_or("tile parent")?).map_err(|e| e.to_string())?;
            std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
            if lod == 1 {
                totals.bytes_fine += bytes.len() as u64;
            } else {
                totals.bytes_coarse += bytes.len() as u64;
            }
            totals.max_tile_vertices = totals.max_tile_vertices.max(mesh.positions.len());
            tiles.push(TileMeta {
                uri,
                lod,
                cell,
                bounds,
                bytes: bytes.len() as u64,
                vertices: mesh.positions.len(),
                buildings: list.len(),
            });
        }
    }
    Ok((tiles, trees))
}

/// Edge strength of the NAIP luminance along each low-rise lidar footprint boundary, for 11 x 11
/// one-metre shifts of the footprint over the image. A footprint registered with the roof edge
/// sits on the strongest luminance step; the sum over every sampled building peaks at the true
/// horizontal offset between lidar and NAIP. Low-rise roofs only: the orthophoto is rectified to
/// bare earth, so a tall roof leans away from the camera.
fn registration(found: &[Building], grid: &Grid, imagery: &mut Imagery<'_>, acc: &mut Registration) {
    let mut sample: Vec<&Building> = found
        .iter()
        .filter(|b| (3.5..=9.).contains(&b.height_m) && (150..=4000).contains(&b.cells.len()))
        .collect();
    sample.sort_by_key(|b| b.cells.len());
    let sample: Vec<&Building> = sample.into_iter().rev().take(40).collect();
    if sample.len() < 6 {
        return;
    }
    let cos_lat = libm::cos(40.75_f64.to_radians());
    let luminance = |c: [f32; 3]| 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
    let mut block = [[0_f64; REG_N]; REG_N];
    let mut used = 0;
    for b in &sample {
        // Boundary edges: a cell with a 4-neighbour outside the footprint, and that direction.
        let mut edges: Vec<(f64, f64, f64, f64)> = Vec::new();
        for &index in b.cells.keys() {
            let (col, row) = ((index % grid.width) as i64, (index / grid.width) as i64);
            for (dc, dr) in [(1_i64, 0_i64), (-1, 0), (0, 1), (0, -1)] {
                let (c, r) = (col + dc, row + dr);
                let inside = r >= 0
                    && c >= 0
                    && c < grid.width as i64
                    && b.cells.contains_key(&(r as usize * grid.width + c as usize));
                if !inside {
                    // Mercator metres of the edge midpoint and the unit step (ground metres) outward.
                    edges.push((
                        grid.bounds[0] + (col as f64 + 0.5 + 0.5 * dc as f64) * grid.step,
                        grid.bounds[1] + (row as f64 + 0.5 + 0.5 * dr as f64) * grid.step,
                        dc as f64,
                        dr as f64,
                    ));
                }
            }
        }
        if edges.len() < 40 {
            continue;
        }
        let stride = (edges.len() / 300).max(1);
        let edges: Vec<_> = edges.into_iter().step_by(stride).collect();
        let step_m = 1.4 / cos_lat;
        let mut table = [[0_f64; REG_N]; REG_N];
        let mut ok = true;
        'shifts: for (iy, row) in table.iter_mut().enumerate() {
            for (ix, slot) in row.iter_mut().enumerate() {
                let shift = [
                    (ix as f64 - REG_R as f64) / cos_lat,
                    (iy as f64 - REG_R as f64) / cos_lat,
                ];
                let (mut total, mut n) = (0_f64, 0_f64);
                for &(x, y, dx, dy) in &edges {
                    let at = |t: f64| (x + shift[0] + dx * t * step_m, y + shift[1] + dy * t * step_m);
                    let (xi, yi) = at(-1.);
                    let (xo, yo) = at(1.);
                    if let (Some(i), Some(o)) = (imagery.color(xi, yi), imagery.color(xo, yo)) {
                        total += f64::from((luminance(i) - luminance(o)).abs());
                        n += 1.;
                    }
                }
                if n < 30. {
                    ok = false;
                    break 'shifts;
                }
                *slot = total / n;
            }
        }
        if ok {
            for (row, add) in block.iter_mut().zip(&table) {
                for (v, a) in row.iter_mut().zip(add) {
                    *v += a;
                }
            }
            used += 1;
        }
    }
    if used < 6 {
        return;
    }
    let mut best = (0, 0, f64::NEG_INFINITY);
    for (iy, (sum_row, row)) in acc.sum.iter_mut().zip(&block).enumerate() {
        for (ix, (sum, v)) in sum_row.iter_mut().zip(row).enumerate() {
            *sum += v;
            if *v > best.2 {
                best = (ix as i32 - REG_R as i32, iy as i32 - REG_R as i32, *v);
            }
        }
    }
    acc.buildings += used;
    acc.per_block_best.push([best.0, best.1]);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rectangles_merge_equal_rows_and_keep_courtyards() {
        let mut r = Raster {
            step: 1,
            cells: BTreeMap::new(),
            planes: vec![flat(10.)],
            colors: vec![0],
        };
        for y in 0..4 {
            for x in 0..6 {
                if !(x == 2 || x == 3) || !(y == 1 || y == 2) {
                    r.cells.insert((x, y), (0, 0.));
                }
            }
        }
        let found = rects(&r);
        assert!(found.len() >= 4 && found.len() <= 6, "{found:?}");
        let area: i32 = found.iter().map(|(a, b, c, d, _)| (c - a) * (d - b)).sum();
        assert_eq!(area, 24 - 4);
    }
    /// Real Midtown block from the corridor cache: `cargo test -p xtask real_midtown -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the fetched hudson-corridor cache"]
    fn real_midtown_block_reconstructs_decodes_and_fits_the_budget() {
        let cache =
            std::path::PathBuf::from("/home/ray/builds/codex-jobs/scenery/new-york/hudson-corridor/cache");
        let sources: super::super::Sources = download::json(&cache.join("sources.json")).unwrap();
        let request = super::super::Request {
            operation: super::super::Operation::Build,
            fetch_layer: super::super::FetchLayer::All,
            lidar_source: super::super::LidarSource::Ept,
            region: "new-york".into(),
            area: corridor::AREA.into(),
            cache,
            out: PathBuf::new(),
            sample: None,
            allow_http: false,
        };
        let plan = corridor::plan();
        let m = corridor::merc(40.7536, -73.9832);
        let spec = blocks(&plan)
            .into_iter()
            .find(|b| m[0] >= b.core[0] && m[0] < b.core[2] && m[1] >= b.core[1] && m[1] < b.core[3])
            .expect("Midtown block");
        let nodes = nodes(&request, &sources).unwrap();
        let mut dem = Dems::open(&request, &sources).unwrap();
        let geoid = super::super::geoid::load(&request, &sources).unwrap().unwrap();
        let origin = corridor::ROOT_CENTER;
        super::super::objects::set_origin(origin);
        let frame = Frame::new(origin, &geoid).unwrap();
        let out = std::env::temp_dir().join("fc-midtown-block");
        let _ = std::fs::remove_dir_all(&out);
        let o = Output {
            out: &out,
            plan: &plan,
            frame: &frame,
            geoid: &geoid,
        };
        let mut imagery = Imagery::new(&plan, &request, &sources);
        let mut totals = Totals::default();
        let started = std::time::Instant::now();
        let (tiles, trees) = process_block(&spec, &nodes, &o, &mut dem, &mut imagery, &mut totals).unwrap();
        eprintln!(
            "midtown block: {} tiles, {} trees, {} buildings, {} planes, {:?}; fine {} B, coarse {} B, max verts {}; ground {:?}",
            tiles.len(),
            trees.len(),
            totals.buildings,
            totals.planes,
            started.elapsed(),
            totals.bytes_fine,
            totals.bytes_coarse,
            totals.max_tile_vertices,
            totals.ground.finish().orthometric_median_absolute_error_m
        );
        let mut heights = totals.building_heights.clone();
        heights.sort_by(f32::total_cmp);
        eprintln!(
            "tallest buildings: {:?}",
            &heights[heights.len().saturating_sub(5)..]
        );
        eprintln!(
            "registration: {} buildings, per-block best {:?}, sums {:?}",
            totals.registration.buildings, totals.registration.per_block_best, totals.registration.sum
        );
        let transform = flightcraft_world::tiles3d::enu_transform(origin[0], origin[1], 0.).unwrap();
        for t in &tiles {
            let bytes = std::fs::read(out.join(&t.uri)).unwrap();
            flightcraft_render::scenery::decode_glb_with_geoid(&bytes, transform, origin, Some(&geoid))
                .unwrap();
        }
        // A buildings-only preview tileset so the renderer can be pointed at this block.
        let tree = super::super::corridor_build::building_tree(&tiles)
            .unwrap()
            .unwrap();
        let region = tree.bounding_volume.region.unwrap();
        let set = flightcraft_world::tiles3d::Tileset {
            asset: flightcraft_world::tiles3d::Asset {
                version: "1.1".into(),
                copyright: Some("Midtown block preview".into()),
            },
            geometric_error: 1_000_000.,
            root: flightcraft_world::tiles3d::Tile {
                bounding_volume: flightcraft_world::tiles3d::BoundingVolume::region(region),
                geometric_error: 1_000_000.,
                refine: Some(flightcraft_world::tiles3d::Refine::Replace),
                transform: Some(transform),
                content: None,
                children: vec![tree],
            },
            extensions_required: Vec::new(),
            extras: Some(serde_json::json!({"flightcraft":{
                "previewOnly":true,"region":"new-york","area":"hudson-corridor",
                "geoid":serde_json::to_value(&geoid).unwrap()
            }})),
        };
        download::write_json(&out.join("tileset.json"), &set).unwrap();
        eprintln!("preview tileset: {}", out.join("tileset.json").display());
        assert!(totals.buildings > 100);
    }
    fn synthetic_geoid() -> GeoidGrid {
        GeoidGrid {
            model: flightcraft_world::geoid::GeoidModel::NgsGeoid18,
            source_sha256: "a".repeat(64),
            south_deg: 40.7,
            west_deg: -74.05,
            step_deg: [1. / 60.; 2],
            rows: 12,
            cols: 12,
            undulation_m: vec![-33.; 144],
        }
    }
    /// A podium with a tower: two measured roof planes, a courtyard, walls to the DEM.
    #[test]
    fn synthetic_tower_on_a_podium_meshes_to_a_compact_decodable_tile() {
        let plan = corridor::plan();
        let m = corridor::merc(40.7536, -73.9832);
        let grid = Grid::new([m[0], m[1], m[0] + 200., m[1] + 200.]).unwrap();
        let w = grid.width;
        let mut cells = BTreeMap::new();
        for y in 20..50 {
            for x in 20..60 {
                let courtyard = (28..32).contains(&x) && (30..34).contains(&y);
                if !courtyard {
                    cells.insert(
                        y * w + x,
                        usize::from((40..46).contains(&x) && (30..38).contains(&y)),
                    );
                }
            }
        }
        let plane = |z: f64| Plane {
            a: 0.,
            b: 0.,
            c: z,
            rms_m: 0.05,
            samples: 100,
        };
        let b = Building {
            roof_kind: buildings::RoofKind::Stepped,
            roof_planes: vec![plane(40.), plane(150.)],
            measured_roof_cells: cells.len(),
            interpolated_roof_cells: 0,
            inferred_classification: false,
            height_m: 148.,
            center_lat_lon: [40.7536, -73.9832],
            excluded_roof_cells: 0,
            cells,
        };
        let mut grid = grid;
        for c in &mut grid.cells {
            c.ground = 2.;
        }
        let geoid = synthetic_geoid();
        let origin = corridor::ROOT_CENTER;
        let frame = Frame::new(origin, &geoid).unwrap();
        let ctx = Ctx {
            grid: &grid,
            frame: &frame,
        };
        let request = super::super::Request {
            operation: super::super::Operation::Build,
            fetch_layer: super::super::FetchLayer::All,
            lidar_source: super::super::LidarSource::Ept,
            region: "new-york".into(),
            area: corridor::AREA.into(),
            cache: PathBuf::new(),
            out: PathBuf::new(),
            sample: None,
            allow_http: false,
        };
        let sources = corridor::new_sources(&plan);
        let mut imagery = Imagery::new(&plan, &request, &sources);
        let used = BTreeSet::from([(4_u8, atlas::class_for(b.height_m))]);
        let atlas = Atlas::new(&used);
        let transform = flightcraft_world::tiles3d::enu_transform(origin[0], origin[1], 0.).unwrap();
        for (raster, bay) in [
            (raster_fine(&b, &grid, &mut imagery), 8.),
            (raster_coarse(&b, &grid, &mut imagery), 16.),
        ] {
            let mut mesh = Mesh::default();
            let mut bounds = Bounds::new();
            mesh_raster(
                &ctx,
                &raster,
                &atlas,
                4,
                atlas::class_for(b.height_m),
                bay,
                &mut mesh,
                &mut bounds,
            )
            .unwrap();
            assert!(!mesh.is_empty() && mesh.indices.iter().all(|i| (*i as usize) < mesh.positions.len()));
            assert!(mesh.positions.len() < 1500, "{} vertices", mesh.positions.len());
            assert!(mesh.uv.iter().flatten().all(|v| (0. ..=1.).contains(v)));
            // The tower reaches its 150 m roof; ellipsoidal bounds follow H + N.
            assert!((bounds.hi - (150. - 33.)).abs() < 1.5 && bounds.lo < 2. - 33.);
            let bytes = write_glb(&mesh, Texture::Webp(&atlas.webp().unwrap()), None).unwrap();
            let decoded =
                flightcraft_render::scenery::decode_glb_with_geoid(&bytes, transform, origin, Some(&geoid))
                    .unwrap();
            assert!(decoded.byte_size() > 0);
        }
    }
    #[test]
    fn blocks_tile_the_envelope_on_a_lattice() {
        let plan = corridor::plan();
        let blocks = blocks(&plan);
        assert!(blocks.len() > 20 && blocks.len() < 120, "{}", blocks.len());
        for b in &blocks {
            assert!(((b.core[2] - b.core[0]) - BLOCK_M).abs() < 1e-9);
            // A block's core is a whole number of 256 m cells from the root corner.
            assert!(((b.core[0] - plan.root[0]) / CELL_M).fract().abs() < 1e-9);
        }
    }
}
