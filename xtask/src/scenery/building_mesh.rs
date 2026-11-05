//! NAIP-textured measured roofs and DEM-grounded walls, including courtyard and step walls.
use super::{
    InputKind, Sources,
    buildings::{Building, Plane, Rect},
    dem::Dems,
    download,
    grid::ImageryGrid,
    objects::{self, Document, Mesh},
    surface::Grid,
};
use flightcraft_world::{
    geoid::GeoidGrid,
    scenery::{self, TileKey},
    tiles3d::{BoundingVolume, Content, Refine, Tile, Tileset},
};
use serde_json::json;
use std::{collections::BTreeMap, path::Path};
const ORIGIN: [f64; 2] = [40.96905613156273, -75.00242278301268];
struct Texture {
    bounds: [f64; 4],
    height: u32,
    facade_height_m: f64,
    bytes: Vec<u8>,
}
impl Texture {
    fn make(out: &Path, layout: &ImageryGrid, grid: &Grid, b: &Building) -> Result<Self, String> {
        use image::ImageEncoder;
        let mut bounds = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for &index in b.cells.keys() {
            let (x, y) = ((index % grid.width) as f64, (index / grid.width) as f64);
            bounds[0] = bounds[0].min(x - 1.);
            bounds[1] = bounds[1].min(y - 1.);
            bounds[2] = bounds[2].max(x + 1.);
            bounds[3] = bounds[3].max(y + 1.);
        }
        let width = libm::ceil((bounds[2] - bounds[0]) / layout.ground_sample_m).clamp(256., 512.) as u32;
        let height = libm::ceil((bounds[3] - bounds[1]) / layout.ground_sample_m).clamp(16., 508.) as u32;
        let mut image = image::RgbaImage::new(width, height + 128);
        let mut cache = BTreeMap::new();
        let side = 1_u32 << layout.max_level;
        for (col, row, pixel) in image.enumerate_pixels_mut() {
            if row >= height {
                let x = f64::from(col) / f64::from(width) * 8.;
                let h = f64::from(height + 127 - row) / 127. * b.height_m;
                let hangar = b.height_m < 12. && b.measured_roof_cells > 300;
                *pixel = facade_pixel(x, h, b.height_m, hangar);
                continue;
            }
            let gx = bounds[0] + (f64::from(col) + 0.5) / f64::from(width) * (bounds[2] - bounds[0]);
            let gy = bounds[3] - (f64::from(row) + 0.5) / f64::from(height) * (bounds[3] - bounds[1]);
            let mx = grid.bounds[0] + (gx + 0.5) * grid.step;
            let my = grid.bounds[1] + (gy + 0.5) * grid.step;
            let fx = ((mx - layout.bounds[0]) / (layout.bounds[2] - layout.bounds[0]) * f64::from(side))
                .clamp(0., f64::from(side) - 1e-9);
            let fy = ((layout.bounds[3] - my) / (layout.bounds[3] - layout.bounds[1]) * f64::from(side))
                .clamp(0., f64::from(side) - 1e-9);
            let key = TileKey {
                level: layout.max_level,
                x: libm::floor(fx) as u32,
                y: libm::floor(fy) as u32,
            };
            if !cache.contains_key(&key) {
                if cache.len() >= 16 {
                    cache.pop_first();
                }
                let img = super::decode_image(
                    &download::read(
                        &out.join(format!("imagery/{}/m0.webp", key.name())),
                        8 * 1024 * 1024,
                    )?,
                    Some(512),
                )?;
                cache.insert(key, img);
            }
            let image = cache.get(&key).ok_or("Roof imagery tile")?;
            *pixel = *image
                .get_pixel_checked(
                    ((fx - frac_floor(fx)) * 512.) as u32,
                    ((fy - frac_floor(fy)) * 512.) as u32,
                )
                .ok_or("Roof imagery sample")?;
        }
        let mut bytes = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut bytes)
            .write_image(
                image.as_raw(),
                width,
                height + 128,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|e| e.to_string())?;
        Ok(Self {
            bounds,
            height: height + 128,
            facade_height_m: b.height_m,
            bytes,
        })
    }
    fn uv(&self, x: f64, y: f64) -> [f32; 2] {
        [
            ((x - self.bounds[0]) / (self.bounds[2] - self.bounds[0])) as f32,
            ((self.bounds[3] - y) / (self.bounds[3] - self.bounds[1])
                * (f64::from(self.height - 128) / f64::from(self.height))) as f32,
        ]
    }
    fn wall_uv(&self, u: f64, h: f64) -> [f32; 2] {
        [
            u.clamp(0.002, 0.998) as f32,
            ((f64::from(self.height - 128)
                + 1.
                + 126. * (1. - h / self.facade_height_m.max(1.)).clamp(0., 1.))
                / f64::from(self.height)) as f32,
        ]
    }
}
// Original plausible facade detail, not a measured window/door inventory. Units are meters.
fn facade_pixel(x: f64, h: f64, height: f64, hangar: bool) -> image::Rgba<u8> {
    let mut color = if hangar {
        [0.62, 0.63, 0.59]
    } else if height > 18. {
        [0.56, 0.59, 0.60]
    } else {
        [0.66, 0.62, 0.55]
    };
    let rib = if hangar {
        ((x / 0.28).fract() - 0.5).abs() < 0.09
    } else {
        (h / 0.18).fract() < 0.12
    };
    if rib {
        color = color.map(|v| v * 0.74);
    }
    if hangar && x > 0.65 && x < 6.8 && h > 0.12 && h < height * 0.77 {
        let seam = (x - 0.65) % 1.45 < 0.05 || h % 0.38 < 0.025;
        color = if seam {
            [0.35, 0.37, 0.37]
        } else {
            [0.55, 0.57, 0.55]
        };
    }
    let window = if hangar {
        x > 7.05 && x < 7.7 && h > 2.2 && h < 3.0
    } else {
        let floor = h % 3.4;
        let col = x % 2.7;
        col > 0.65 && col < 1.85 && floor > 1.05 && floor < 2.45
    };
    if window {
        color = if (x % 0.5) < 0.035 || (h % 1.2) < 0.04 {
            [0.26, 0.28, 0.28]
        } else {
            [
                0.16,
                0.24 + 0.08 * h / height.max(1.),
                0.29 + 0.12 * h / height.max(1.),
            ]
        };
    }
    if !hangar && x > 6.6 && x < 7.5 && h < 2.1 && h > 0.05 {
        color = [0.28, 0.27, 0.23];
    }
    if h < 0.12 {
        color = [0.38, 0.39, 0.38];
    }
    image::Rgba([
        (color[0] * 255.) as u8,
        (color[1] * 255.) as u8,
        (color[2] * 255.) as u8,
        255,
    ])
}
fn frac_floor(v: f64) -> f64 {
    libm::floor(v)
}
fn geographic(grid: &Grid, x: f64, y: f64) -> Result<[f64; 2], String> {
    scenery::inverse_mercator(
        grid.bounds[0] + (x + 0.5) * grid.step,
        grid.bounds[1] + (y + 0.5) * grid.step,
    )
    .map_err(|e| e.to_string())
}
fn vertex(grid: &Grid, x: f64, y: f64, z: f64, geoid: &GeoidGrid) -> Result<[f32; 3], String> {
    let [lat, lon] = geographic(grid, x, y)?;
    objects::local(lat, lon, z, ORIGIN, geoid)
}
fn roof_polygon(
    mesh: &mut Mesh,
    grid: &Grid,
    polygon: &[[f64; 2]],
    plane: Plane,
    texture: &Texture,
    geoid: &GeoidGrid,
) -> Result<(), String> {
    if polygon.len() < 3 {
        return Ok(());
    }
    let a = *polygon.first().ok_or("Roof polygon")?;
    for pair in polygon.get(1..).ok_or("Roof polygon tail")?.windows(2) {
        let [b, c] = pair else {
            return Err("Roof polygon edge".into());
        };
        let points = [a, *b, *c];
        let mut p = Vec::new();
        let mut uv = Vec::new();
        for [x, y] in points {
            p.push(vertex(grid, x, y, plane.z(x, y), geoid)?);
            uv.push(texture.uv(x, y));
        }
        mesh.triangle(
            p.try_into().map_err(|_| "Roof triangle")?,
            uv.try_into().map_err(|_| "Roof UV triangle")?,
        )?;
    }
    Ok(())
}
/// Clip to the lower envelope of measured gable/hip planes. Never used for stepped roofs.
fn clip(poly: &[[f64; 2]], a: f64, b: f64, c: f64) -> Vec<[f64; 2]> {
    let mut result = Vec::new();
    let Some(mut previous) = poly.last().copied() else {
        return result;
    };
    for &p in poly {
        let dp = a * previous[0] + b * previous[1] + c;
        let dq = a * p[0] + b * p[1] + c;
        if (dp <= 0.) != (dq <= 0.) {
            let t = dp / (dp - dq);
            result.push([
                previous[0] + t * (p[0] - previous[0]),
                previous[1] + t * (p[1] - previous[1]),
            ]);
        }
        if dq <= 0. {
            result.push(p);
        }
        previous = p;
    }
    result
}
fn smooth_outline(grid: &Grid, b: &Building) -> Option<Vec<[f64; 2]>> {
    if b.roof_planes.len() > 5 {
        return None;
    }
    let n = b.cells.len() as f64;
    let mx = b.cells.keys().map(|i| (i % grid.width) as f64).sum::<f64>() / n;
    let my = b.cells.keys().map(|i| (i / grid.width) as f64).sum::<f64>() / n;
    let (mut xx, mut yy, mut xy) = (0., 0., 0.);
    for &i in b.cells.keys() {
        let x = (i % grid.width) as f64 - mx;
        let y = (i / grid.width) as f64 - my;
        xx += x * x;
        yy += y * y;
        xy += x * y;
    }
    let angle = 0.5 * libm::atan2(2. * xy, xx - yy);
    let (sn, cs) = (libm::sin(angle), libm::cos(angle));
    let (mut u0, mut v0, mut u1, mut v1) =
        (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for &i in b.cells.keys() {
        let x = (i % grid.width) as f64 - mx;
        let y = (i / grid.width) as f64 - my;
        let u = x * cs + y * sn;
        let v = -x * sn + y * cs;
        u0 = u0.min(u);
        u1 = u1.max(u);
        v0 = v0.min(v);
        v1 = v1.max(v);
    }
    u0 -= 0.5;
    v0 -= 0.5;
    u1 += 0.5;
    v1 += 0.5;
    if n / ((u1 - u0) * (v1 - v0)) < 0.87 {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    let mut error = 0.;
    for &i in b.cells.keys() {
        x0 = x0.min(i % grid.width);
        x1 = x1.max(i % grid.width);
        y0 = y0.min(i / grid.width);
        y1 = y1.max(i / grid.width);
        let p = grid.cells.get(i)?;
        let x = (i % grid.width) as f64;
        let y = (i / grid.width) as f64;
        let roof = b
            .roof_planes
            .iter()
            .map(|p| p.z(x, y))
            .fold(f64::INFINITY, f64::min);
        error += (roof - f64::from(p.top)).powi(2);
    }
    if libm::sqrt(error / n) > 0.35 {
        return None;
    }
    // A courtyard (or an unresolved interior data gap) disables rectangle fitting completely.
    for y in y0..=y1 {
        for x in x0..=x1 {
            if b.cells.contains_key(&(y * grid.width + x)) {
                continue;
            }
            let u = (x as f64 - mx) * cs + (y as f64 - my) * sn;
            let v = -(x as f64 - mx) * sn + (y as f64 - my) * cs;
            if u > u0 + 1.2 && u < u1 - 1.2 && v > v0 + 1.2 && v < v1 - 1.2 {
                return None;
            }
        }
    }
    Some(
        [[u0, v0], [u1, v0], [u1, v1], [u0, v1]]
            .map(|[u, v]| [mx + u * cs - v * sn, my + u * sn + v * cs])
            .to_vec(),
    )
}
#[allow(clippy::too_many_arguments)]
fn wall(
    mesh: &mut Mesh,
    grid: &Grid,
    a: [f64; 2],
    b: [f64; 2],
    top: Plane,
    bottom: Option<Plane>,
    texture: &Texture,
    dem: &mut Dems,
    geoid: &GeoidGrid,
) -> Result<(), String> {
    let height = |[x, y]: [f64; 2], dem: &mut Dems| -> Result<f64, String> {
        if let Some(p) = bottom {
            Ok(p.z(x, y))
        } else {
            let [lat, lon] = geographic(grid, x, y)?;
            dem.height(lat, lon)
        }
    };
    let za = top.z(a[0], a[1]);
    let zb = top.z(b[0], b[1]);
    let ba = height(a, dem)?;
    let bb = height(b, dem)?;
    if za <= ba + 0.05 && zb <= bb + 0.05 {
        return Ok(());
    }
    let length = libm::hypot(b[0] - a[0], b[1] - a[1]);
    if length < 0.001 {
        return Ok(());
    }
    let axis = [(b[0] - a[0]) / length, (b[1] - a[1]) / length];
    let start = a[0] * axis[0] + a[1] * axis[1];
    let mut knots = vec![0., 1.];
    let mut next = (libm::floor(start / 8.) + 1.) * 8.;
    while next < start + length - 1e-6 {
        knots.push((next - start) / length);
        next += 8.;
    }
    knots.sort_by(f64::total_cmp);
    for pair in knots.windows(2) {
        let [t0, t1] = pair else {
            continue;
        };
        let q0 = [a[0] + (b[0] - a[0]) * t0, a[1] + (b[1] - a[1]) * t0];
        let q1 = [a[0] + (b[0] - a[0]) * t1, a[1] + (b[1] - a[1]) * t1];
        let z0 = ba + (bb - ba) * t0;
        let z1 = ba + (bb - ba) * t1;
        let r0 = za + (zb - za) * t0;
        let r1 = za + (zb - za) * t1;
        let p = [
            vertex(grid, q0[0], q0[1], z0.min(r0), geoid)?,
            vertex(grid, q1[0], q1[1], z1.min(r1), geoid)?,
            vertex(grid, q1[0], q1[1], r1, geoid)?,
            vertex(grid, q0[0], q0[1], r0, geoid)?,
        ];
        let u0 = (start + length * t0).rem_euclid(8.) / 8.;
        let u1 = (u0 + length * (t1 - t0) / 8.).min(1.);
        let uv = [
            texture.wall_uv(u0, 0.),
            texture.wall_uv(u1, 0.),
            texture.wall_uv(u1, r1 - z1),
            texture.wall_uv(u0, r0 - z0),
        ];
        mesh.triangle([p[0], p[1], p[2]], [uv[0], uv[1], uv[2]])?;
        mesh.triangle([p[0], p[2], p[3]], [uv[0], uv[2], uv[3]])?;
    }
    Ok(())
}
fn geometry(
    grid: &Grid,
    b: &Building,
    texture: &Texture,
    dem: &mut Dems,
    geoid: &GeoidGrid,
) -> Result<Mesh, String> {
    let mut mesh = Mesh::default();
    if let Some(outline) = smooth_outline(grid, b) {
        for plane in &b.roof_planes {
            let mut region = outline.clone();
            for other in &b.roof_planes {
                region = clip(&region, plane.a - other.a, plane.b - other.b, plane.c - other.c);
            }
            roof_polygon(&mut mesh, grid, &region, *plane, texture, geoid)?;
        }
        for (&a, &bpoint) in outline
            .iter()
            .zip(outline.iter().cycle().skip(1))
            .take(outline.len())
        {
            let mut cuts = vec![0., 1.];
            for (i, p) in b.roof_planes.iter().enumerate() {
                for q in b.roof_planes.iter().skip(i + 1) {
                    let start = p.z(a[0], a[1]) - q.z(a[0], a[1]);
                    let end = p.z(bpoint[0], bpoint[1]) - q.z(bpoint[0], bpoint[1]);
                    if (end - start).abs() > 1e-8 {
                        let t = -start / (end - start);
                        if (0.0..1.).contains(&t) {
                            cuts.push(t);
                        }
                    }
                }
            }
            cuts.sort_by(f64::total_cmp);
            cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-7);
            for pair in cuts.windows(2) {
                let [t0, t1] = pair else {
                    return Err("Wall cut".into());
                };
                let interpolate = |t: f64| [a[0] + t * (bpoint[0] - a[0]), a[1] + t * (bpoint[1] - a[1])];
                let mid = interpolate((t0 + t1) / 2.);
                let top = *b
                    .roof_planes
                    .iter()
                    .min_by(|p, q| p.z(mid[0], mid[1]).total_cmp(&q.z(mid[0], mid[1])))
                    .ok_or("Wall roof plane")?;
                wall(
                    &mut mesh,
                    grid,
                    interpolate(*t0),
                    interpolate(*t1),
                    top,
                    None,
                    texture,
                    dem,
                    geoid,
                )?;
            }
        }
    } else {
        for Rect {
            x0,
            y0,
            x1,
            y1,
            plane,
        } in super::buildings::rectangles(b, grid.width)
        {
            roof_polygon(
                &mut mesh,
                grid,
                &[[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
                *b.roof_planes.get(plane).ok_or("Roof rectangle plane")?,
                texture,
                geoid,
            )?;
        }
        for (&index, &plane) in &b.cells {
            let (x, y) = ((index % grid.width) as f64, (index / grid.width) as f64);
            let top = *b.roof_planes.get(plane).ok_or("Roof wall plane")?;
            for (dx, dy, a, c) in [
                (0, -1, [x - 0.5, y - 0.5], [x + 0.5, y - 0.5]),
                (1, 0, [x + 0.5, y - 0.5], [x + 0.5, y + 0.5]),
                (0, 1, [x + 0.5, y + 0.5], [x - 0.5, y + 0.5]),
                (-1, 0, [x - 0.5, y + 0.5], [x - 0.5, y - 0.5]),
            ] {
                let neighbor = grid
                    .at(x as i32 + dx, y as i32 + dy)
                    .and_then(|(i, _)| b.cells.get(&i))
                    .copied();
                if neighbor == Some(plane) {
                    continue;
                }
                wall(
                    &mut mesh,
                    grid,
                    a,
                    c,
                    top,
                    neighbor.and_then(|id| b.roof_planes.get(id).copied()),
                    texture,
                    dem,
                    geoid,
                )?;
            }
        }
    }
    if mesh.positions.len() > 200_000 {
        return Err("Building mesh exceeds tile vertex budget".into());
    }
    Ok(mesh)
}
#[allow(clippy::too_many_arguments)]
pub(super) fn write(
    out: &Path,
    set: &mut Tileset,
    buildings: &[Building],
    grid: &Grid,
    layout: &ImageryGrid,
    dem: &mut Dems,
    geoid: &GeoidGrid,
    sources: &Sources,
) -> Result<(), String> {
    if buildings.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(out.join("buildings")).map_err(|e| e.to_string())?;
    let mut leaves = Vec::new();
    let mut payloads = Vec::new();
    for (index, b) in buildings.iter().enumerate() {
        let texture = Texture::make(out, layout, grid, b)?;
        let mesh = geometry(grid, b, &texture, dem, geoid)?;
        let mut doc = Document::default();
        let m = doc.mesh(&mesh)?;
        let bytes = doc.finish(vec![m], vec![json!({"mesh":0})], &texture.bytes, false, false)?;
        let uri = format!("building-{index:05}.glb");
        std::fs::write(out.join("buildings").join(&uri), bytes).map_err(|e| e.to_string())?;
        let [mut w, mut s, mut e, mut n, mut lo, mut hi] = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ];
        for &i in b.cells.keys() {
            let [lat, lon] = grid.coordinate(i)?;
            w = w.min(lon);
            e = e.max(lon);
            s = s.min(lat);
            n = n.max(lat);
            let c = grid.cells.get(i).ok_or("Building tile bounds")?;
            lo = lo.min(
                geoid
                    .ellipsoidal_m(lat, lon, f64::from(c.ground))
                    .map_err(|e| e.to_string())?,
            );
            hi = hi.max(
                geoid
                    .ellipsoidal_m(lat, lon, f64::from(c.top))
                    .map_err(|e| e.to_string())?,
            );
        }
        let pad = 2. / 111_000.;
        leaves.push(Tile {
            bounding_volume: BoundingVolume::region([
                (w - pad * 1.5).to_radians(),
                (s - pad).to_radians(),
                (e + pad * 1.5).to_radians(),
                (n + pad).to_radians(),
                lo - 1.,
                hi + 1.,
            ]),
            geometric_error: 0.,
            refine: Some(Refine::Add),
            transform: None,
            content: Some(Content { uri }),
            children: Vec::new(),
        });
        payloads.push((mesh, texture));
    }
    // One complete building per handoff leaf. Group spatially to stay within the 64-child cap.
    fn group(mut nodes: Vec<Tile>) -> Result<Tile, String> {
        if nodes.is_empty() {
            return Err("Empty building group".into());
        }
        let mut root = Tile {
            bounding_volume: nodes
                .first()
                .ok_or("Building group bounds")?
                .bounding_volume
                .clone(),
            geometric_error: 0.,
            refine: Some(Refine::Add),
            transform: None,
            content: None,
            children: Vec::new(),
        };
        for node in &nodes {
            objects::include_bounds(&mut root, node)?;
        }
        if nodes.len() <= 32 {
            root.children = nodes;
        } else {
            nodes.sort_by(|a, b| {
                a.bounding_volume
                    .region
                    .map(|r| r[0])
                    .unwrap_or(0.)
                    .total_cmp(&b.bounding_volume.region.map(|r| r[0]).unwrap_or(0.))
            });
            let half = nodes.split_off(nodes.len() / 2);
            root.children = vec![group(nodes)?, group(half)?];
        }
        Ok(root)
    }
    let mut root = group(leaves.clone())?;
    root.transform = set.root.transform;
    let accepted = set
        .extras
        .as_ref()
        .and_then(|v| v.pointer("/flightcraft/verticalDatumVerified"))
        .and_then(|v| v.as_bool())
        == Some(true);
    let preview = set
        .extras
        .as_ref()
        .and_then(|v| v.pointer("/flightcraft/previewOnly"))
        .and_then(|v| v.as_bool())
        != Some(false);
    let runway = set
        .extras
        .as_ref()
        .and_then(|v| v.pointer("/flightcraft/runwayAlignmentVerified"))
        .and_then(|v| v.as_bool())
        == Some(true);
    // The 1N7 pipeline records an explicit acceptance result; flows without one (the NYC handoff)
    // keep the earlier meaning of lidarBuilt.
    let lidar_built = set
        .extras
        .as_ref()
        .and_then(|v| v.pointer("/flightcraft/lidarAcceptanceVerified"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let mut flags = json!({"region":sources.region,"area":sources.area,"sourceKind":"usgs-3dep-lidar","roofPolicy":"lidar-only","usesExternalFootprints":false,"lidarBuilt":lidar_built,"buildingCount":buildings.len(),"previewOnly":preview,"verticalDatumVerified":accepted,"runwayAlignmentVerified":runway,"localOnly":false,"redistributionAllowed":true});
    let receipts: Vec<_> = sources
        .inputs
        .iter()
        .filter(|i| {
            matches!(
                i.kind,
                InputKind::EptMetadata | InputKind::EptHierarchy | InputKind::EptLidar
            )
        })
        .map(|i| &i.source)
        .collect();
    flags["lidarReceipts"] = serde_json::to_value(receipts).map_err(|e| e.to_string())?;
    if sources.area == "nyc-lidar" {
        if sources.region != "new-york"
            || buildings.iter().any(|b| b.inferred_classification)
            || !sources.inputs.iter().any(|i| {
                i.kind == InputKind::EptMetadata
                    && i.source.url == format!("{}/ept.json", super::ept::NYC_ROOT)
            })
        {
            return Err("Fly NYC handoff requires classified NY_NewYorkCity building returns; inferred NJ buildings cannot be relabeled as NYC".into());
        }
        // Consume the merged city lane's area contract at runtime: one definition, no second
        // source schema or hardcoded copy of its selection/budget in this lane.
        let selection: serde_json::Value = serde_json::from_slice(&download::read(
            &crate::root()?.join("assets/scenery/fly-nyc.json"),
            64 * 1024,
        )?)
        .map_err(|e| e.to_string())?;
        if selection.get("region").and_then(|v| v.as_str()) != Some("new-york")
            || selection.get("area").and_then(|v| v.as_str()) != Some("nyc-lidar")
            || selection.get("ept").and_then(|v| v.as_str())
                != Some(&format!("{}/ept.json", super::ept::NYC_ROOT))
            || selection.get("classification").and_then(|v| v.as_u64()) != Some(6)
        {
            return Err("Fly NYC area contract does not match the classified public survey".into());
        }
        flags["nycLidar"] = json!({"selection":selection,"reconstructionBuildings":buildings.len(),"reconstructionLeavesAreBuildings":true,"usesExternalFootprints":false,"receipts":flags.get("lidarReceipts").ok_or("NYC source receipts")?});
    }
    if let Some(geoid) = set.extras.as_ref().and_then(|v| v.pointer("/flightcraft/geoid")) {
        flags["geoid"] = geoid.clone();
    }
    let handoff = Tileset {
        asset: set.asset.clone(),
        geometric_error: 0.,
        root: root.clone(),
        extensions_required: Vec::new(),
        extras: Some(json!({"flightcraft":flags})),
    };
    handoff.validate().map_err(|e| e.to_string())?;
    download::write_json(&out.join("buildings/tileset.json"), &handoff)?;
    download::write_json(
        &out.join("reconstruction-provider.json"),
        &json!({"location":{"kind":"local-path","value":"buildings/tileset.json"},"enabled":true}),
    )?;
    // Runtime content has its own spatial replacement LODs. The handoff above stays
    // one complete building per leaf so the city lane can build its own showcase LODs.
    fn runtime(
        out: &Path,
        ids: &[usize],
        leaves: &[Tile],
        payloads: &[(Mesh, Texture)],
        level: u8,
        code: u32,
        error: f64,
    ) -> Result<Tile, String> {
        let first = leaves
            .get(*ids.first().ok_or("Empty runtime building group")?)
            .ok_or("Runtime building leaf")?;
        let mut node = Tile {
            bounding_volume: first.bounding_volume.clone(),
            geometric_error: error,
            refine: Some(Refine::Replace),
            transform: None,
            content: None,
            children: Vec::new(),
        };
        for &id in ids {
            objects::include_bounds(&mut node, leaves.get(id).ok_or("Runtime building bounds")?)?;
        }
        if ids.len() <= 8 {
            node.children = ids
                .iter()
                .map(|i| leaves.get(*i).cloned().ok_or("Runtime building content"))
                .collect::<Result<_, _>>()?;
        } else {
            let mut sorted = ids.to_vec();
            sorted.sort_by(|a, b| {
                let axis = if level.is_multiple_of(2) { 0 } else { 1 };
                let center = |id: usize| {
                    leaves
                        .get(id)
                        .and_then(|t| t.bounding_volume.region)
                        .map(|r| (r[axis] + r[axis + 2]) / 2.)
                        .unwrap_or(0.)
                };
                center(*a).total_cmp(&center(*b))
            });
            for (index, part) in sorted.chunks(sorted.len().div_ceil(4)).enumerate() {
                node.children.push(runtime(
                    out,
                    part,
                    leaves,
                    payloads,
                    level + 1,
                    code * 4 + index as u32,
                    error / 2.,
                )?);
            }
        }
        let count: usize = ids
            .iter()
            .filter_map(|i| payloads.get(*i))
            .map(|(m, _)| m.positions.len())
            .sum();
        let side = libm::ceil(libm::sqrt(ids.len() as f64)) as u32;
        if count <= 200_000 && side <= 32 {
            use image::ImageEncoder;
            let slot = 64;
            let dimension = side * slot;
            let mut atlas =
                image::RgbaImage::from_pixel(dimension, dimension, image::Rgba([152, 153, 143, 255]));
            let mut mesh = Mesh::default();
            for (index, &id) in ids.iter().enumerate() {
                let (source, texture) = payloads.get(id).ok_or("Runtime building payload")?;
                let image = super::decode_image(&texture.bytes, None)?;
                let small = image::imageops::resize(&image, 60, 60, image::imageops::FilterType::Triangle);
                let (x, y) = ((index as u32 % side) * slot, (index as u32 / side) * slot);
                image::imageops::replace(&mut atlas, &small, i64::from(x + 2), i64::from(y + 2));
                let offset = mesh.positions.len() as u32;
                mesh.positions.extend(&source.positions);
                mesh.normals.extend(&source.normals);
                mesh.uv.extend(source.uv.iter().map(|uv| {
                    [
                        (x as f32 + 2. + uv[0] * 60.) / dimension as f32,
                        (y as f32 + 2. + uv[1] * 60.) / dimension as f32,
                    ]
                }));
                mesh.indices.extend(source.indices.iter().map(|i| i + offset));
            }
            let mut texture = Vec::new();
            image::codecs::webp::WebPEncoder::new_lossless(&mut texture)
                .write_image(
                    atlas.as_raw(),
                    dimension,
                    dimension,
                    image::ExtendedColorType::Rgba8,
                )
                .map_err(|e| e.to_string())?;
            let mut doc = Document::default();
            let m = doc.mesh(&mesh)?;
            let bytes = doc.finish(vec![m], vec![json!({"mesh":0})], &texture, false, false)?;
            let uri = format!("lod-{level}-{code}.glb");
            std::fs::write(out.join("buildings").join(&uri), bytes).map_err(|e| e.to_string())?;
            node.content = Some(Content { uri });
        }
        Ok(node)
    }
    root = runtime(
        out,
        &(0..leaves.len()).collect::<Vec<_>>(),
        &leaves,
        &payloads,
        0,
        0,
        set.root.geometric_error.min(16.),
    )?;
    fn prefix(tile: &mut Tile) {
        if let Some(content) = tile.content.as_mut() {
            content.uri = format!("buildings/{}", content.uri);
        }
        for child in &mut tile.children {
            prefix(child);
        }
    }
    prefix(&mut root);
    objects::include_bounds(&mut set.root, &root)?;
    set.root.children.push(root);
    if let Some(flags) = set.extras.as_mut().and_then(|v| v.get_mut("flightcraft")) {
        flags["buildingsBuilt"] = json!(true);
        flags["buildingCount"] = json!(buildings.len());
        flags["lidarBuilt"] = json!(lidar_built);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn facade_material_separates_metal_ribs_windows_and_the_foundation() {
        let rib = super::facade_pixel(0.14, 1.5, 6., true);
        let sheet = super::facade_pixel(0.05, 1.5, 6., true);
        assert!(rib[0] < sheet[0]);
        let glass = super::facade_pixel(1.1, 1.6, 12., false);
        assert!(glass[2] > glass[0]);
        assert_ne!(super::facade_pixel(1.1, 0.05, 12., false), glass);
        let roof = super::Texture {
            bounds: [0., 0., 10., 10.],
            height: 256,
            facade_height_m: 6.,
            bytes: Vec::new(),
        };
        assert_eq!(roof.uv(5., 5.), [0.5, 0.25]);
        assert!(roof.wall_uv(0.5, 3.)[1] > 0.5);
    }

    use super::*;
    #[test]
    fn lower_envelope_clips_gables_and_keeps_shared_ridge() {
        let box_ = [[-2., -3.], [2., -3.], [2., 3.], [-2., 3.]];
        let a = clip(&box_, 1., 0., 0.);
        let b = clip(&box_, -1., 0., 0.);
        assert_eq!(a.len(), 4);
        assert_eq!(b.len(), 4);
        assert!(a.iter().all(|p| p[0] <= 0.));
        assert!(b.iter().all(|p| p[0] >= 0.));
    }
}
