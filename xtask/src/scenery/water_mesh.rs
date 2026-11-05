//! NHD water polygons as a 3D Tiles water tile (original geometry; public-domain source).
//!
//! Polygons are clipped to the scenery area, simplified, tessellated with the repository's
//! even-odd strip triangulator, and lifted to the independent DEM: a standing pond or reservoir
//! takes the median DEM height along its rim; river areas follow the DEM at every vertex. The
//! GLB root carries `extras.flightcraft.water`, which the renderer draws with its water shader.
use super::{
    Request, Sources, alignment, city_mesh,
    dem::Dems,
    objects, shapefile, transport,
    water::{NHD_PREFIX, archives, geographic_bounds},
};
use crate::download;
use flightcraft_world::{
    geo::LocalProjection,
    geoid::GeoidGrid,
    tiles3d::{BoundingVolume, Content, Refine, Tile, Tileset},
};
use glam::DVec3;
use serde::Serialize;
use std::path::Path;

/// NHD feature classes drawn as water surfaces. Swamp/marsh, playa, ice, estuary and
/// flowline-only streams are not reflective open water and are recorded but not drawn.
const WATERBODY_FTYPES: [&str; 2] = ["390", "436"]; // LakePond, Reservoir
const AREA_FTYPES: [&str; 2] = ["460", "336"]; // StreamRiver, CanalDitch
/// Tidal water (the Hudson, the East River and the harbor are NHD estuary and bay polygons):
/// Estuary 493 (NHDWaterbody), Bay/Inlet 312 and SeaOcean 445 (NHDArea).
const TIDAL_TYPES: [&str; 3] = ["493", "312", "445"];
const WATER_LIFT_M: f64 = 0.03;
const NEARBY_NAMED_M: f64 = 5_000.;
/// Rivers are drawn this far beyond the imagery area so the stream corridor continues past its
/// edge (the Paulins Kill polygon starts 33 m from the 1N7 test area). Standing water gets only
/// a short margin: beyond the imagery the ground is coarse fallback terrain, and a pond lifted
/// to the 1 m DEM could float above or hide below it.
pub(super) const RIVER_MARGIN_M: f64 = 1_500.;
pub(super) const POND_MARGIN_M: f64 = 100.;
const MAX_TESSELLATION_VERTICES: usize = 20_000;
/// 3 vertices per triangle, 200,000 vertices per content.
const MAX_TILE_TRIANGLES: usize = 60_000;

#[derive(Clone, Debug, Serialize)]
pub(super) struct PolygonAudit {
    pub layer: &'static str,
    pub name: String,
    pub ftype: String,
    pub fcode: String,
    pub permanent_id: String,
    pub area_m2: f64,
    pub vertices: usize,
    pub triangles: usize,
    pub level_msl_m: f64,
}
#[derive(Clone, Debug, Serialize)]
pub(super) struct Skipped {
    pub layer: &'static str,
    pub name: String,
    pub ftype: String,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize)]
pub(super) struct Nearby {
    pub layer: &'static str,
    pub name: String,
    pub ftype: String,
    /// Distance from the area rectangle to the feature's bounding box, metres.
    pub bbox_distance_m: f64,
}
#[derive(Clone, Debug, Default, Serialize)]
pub(super) struct Audit {
    pub receipts: Vec<download::Source>,
    pub drawn: Vec<PolygonAudit>,
    pub skipped: Vec<Skipped>,
    pub named_features_beyond_drawn_extent_within_5_km: Vec<Nearby>,
    pub flowlines_not_drawn: &'static str,
    pub triangles: usize,
    pub water_albedo_srgb: [u8; 3],
    /// Lidar water-classified 1 m cells and the share that lie inside a drawn NHD polygon.
    pub lidar_water_cells: Option<usize>,
    pub lidar_water_inside_polygons: Option<f64>,
}

pub(super) struct Tessellated {
    pub mesh: objects::Mesh,
    pub audit: Audit,
    /// lon/lat extents of drawn geometry: west, south, east, north (degrees).
    pub extent: Option<[f64; 4]>,
    pub msl_range: Option<[f64; 2]>,
    /// Drawn polygons as local rings (x east, y north), for coverage checks.
    pub footprints: Vec<Vec<Vec<[f64; 2]>>>,
}

#[derive(Default)]
struct Staged {
    triangles: Vec<[[f32; 3]; 3]>,
    level_sum: f64,
    count: u32,
    extent: Option<[f64; 4]>,
    msl: Option<[f64; 2]>,
}

fn rect_clip(ring: &[[f64; 2]], rect: [f64; 4]) -> Vec<[f64; 2]> {
    // Sutherland-Hodgman against the four edges of an axis-aligned rectangle.
    let mut points: Vec<[f64; 2]> = ring[..ring.len().saturating_sub(1)].to_vec();
    for edge in 0..4 {
        let inside = |p: [f64; 2]| match edge {
            0 => p[0] >= rect[0],
            1 => p[0] <= rect[2],
            2 => p[1] >= rect[1],
            _ => p[1] <= rect[3],
        };
        let cross = |a: [f64; 2], b: [f64; 2]| -> [f64; 2] {
            if edge < 2 {
                let x = if edge == 0 { rect[0] } else { rect[2] };
                let t = (x - a[0]) / (b[0] - a[0]);
                [x, a[1] + t * (b[1] - a[1])]
            } else {
                let y = if edge == 2 { rect[1] } else { rect[3] };
                let t = (y - a[1]) / (b[1] - a[1]);
                [a[0] + t * (b[0] - a[0]), y]
            }
        };
        let mut next = Vec::new();
        for i in 0..points.len() {
            let (a, b) = (points[(i + points.len() - 1) % points.len()], points[i]);
            match (inside(a), inside(b)) {
                (true, true) => next.push(b),
                (true, false) => next.push(cross(a, b)),
                (false, true) => {
                    next.push(cross(a, b));
                    next.push(b);
                }
                (false, false) => {}
            }
        }
        points = next;
        if points.is_empty() {
            break;
        }
    }
    if let Some(first) = points.first().copied() {
        points.push(first);
    }
    points
}
/// Douglas-Peucker on an open polyline; endpoints are kept.
fn simplify(points: &[[f64; 2]], epsilon: f64) -> Vec<[f64; 2]> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let (a, b) = (points[0], points[points.len() - 1]);
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let norm = libm::hypot(dx, dy);
    let (mut worst, mut index) = (0., 0);
    for (i, p) in points.iter().enumerate().take(points.len() - 1).skip(1) {
        let d = if norm < 1e-9 {
            libm::hypot(p[0] - a[0], p[1] - a[1])
        } else {
            ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / norm
        };
        if d > worst {
            (worst, index) = (d, i);
        }
    }
    if worst <= epsilon {
        return vec![a, b];
    }
    let mut left = simplify(&points[..=index], epsilon);
    let right = simplify(&points[index..], epsilon);
    left.pop();
    left.extend(right);
    left
}
fn ring_area(ring: &[[f64; 2]]) -> f64 {
    ring.windows(2)
        .map(|w| w[0][0] * w[1][1] - w[1][0] * w[0][1])
        .sum::<f64>()
        / 2.
}
fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.total_cmp(b));
    values.get(values.len() / 2).copied().unwrap_or(f64::NAN)
}
fn attribute(p: &shapefile::Polygon, key: &str) -> String {
    p.attributes.get(key).cloned().unwrap_or_default()
}
fn point_in(ring: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut inside = false;
    for e in ring.windows(2) {
        let (a, b) = (e[0], e[1]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
    }
    inside
}

/// Everything the polygons need from the outside world, so tessellation is testable without
/// real DEMs: the DEM height (MSL, metres) and the glTF-frame position of a lat/lon/MSL point.
pub(super) struct Areas {
    /// Clip rectangles (west, south, east, north degrees) for standing water and for rivers.
    pub ponds: [f64; 4],
    pub rivers: [f64; 4],
    /// Also draw tidal estuary, bay and ocean polygons, with per-vertex water levels from the
    /// hydro-flattened DEM (a coastline has no single rim level) and a coarser tolerance.
    pub tidal: bool,
}
impl Areas {
    #[cfg(test)]
    pub(super) fn same(area: [f64; 4]) -> Self {
        Self {
            ponds: area,
            rivers: area,
            tidal: false,
        }
    }
    pub(super) fn tidal(mut self) -> Self {
        self.tidal = true;
        self
    }
    pub(super) fn around(imagery: [f64; 4]) -> Self {
        let mid = (imagery[1] + imagery[3]) / 2.;
        let grow = |m: f64| {
            let (dlat, dlon) = (m / 110_574., m / (111_320. * libm::cos(mid.to_radians())));
            [
                imagery[0] - dlon,
                imagery[1] - dlat,
                imagery[2] + dlon,
                imagery[3] + dlat,
            ]
        };
        Self {
            ponds: grow(POND_MARGIN_M),
            rivers: grow(RIVER_MARGIN_M),
            tidal: false,
        }
    }
}
pub(super) struct World<'a> {
    pub origin: [f64; 2],
    pub height: &'a mut dyn FnMut(f64, f64) -> Result<f64, String>,
    pub local: &'a dyn Fn(f64, f64, f64) -> Result<[f32; 3], String>,
}

/// Tessellates NHD polygons clipped to `area` (west, south, east, north degrees).
pub(super) fn tessellate(
    layers: &[(&'static str, Vec<shapefile::Polygon>)],
    areas: Areas,
    world: &mut World,
) -> Result<Tessellated, String> {
    let projection = LocalProjection::new(world.origin[0], world.origin[1]).map_err(|e| e.to_string())?;
    let local = |area: [f64; 4]| -> Result<[f64; 4], String> {
        let [w, s, e, n] = area;
        let a = projection.project(s, w).map_err(|e| e.to_string())?;
        let b = projection.project(n, e).map_err(|e| e.to_string())?;
        // x = east, y = north
        Ok([a[1].min(b[1]), a[0].min(b[0]), a[1].max(b[1]), a[0].max(b[0])])
    };
    let (pond_rect, river_rect) = (local(areas.ponds)?, local(areas.rivers)?);
    let tidal = areas.tidal;
    let mut out = Tessellated {
        mesh: objects::Mesh::default(),
        audit: Audit {
            flowlines_not_drawn: "NHD flowlines carry no measured width; narrow streams are not drawn as invented ribbons",
            water_albedo_srgb: [18, 42, 46],
            ..Audit::default()
        },
        extent: None,
        msl_range: None,
        footprints: Vec::new(),
    };
    let mut seen = std::collections::BTreeSet::new();
    for (layer, polygons) in layers {
        let drawn_types: &[&str] = match (*layer == "NHDWaterbody", tidal) {
            (true, false) => &WATERBODY_FTYPES,
            (true, true) => &["390", "436", "493", "312", "445"],
            (false, false) => &AREA_FTYPES,
            (false, true) => &["460", "336", "312", "445"],
        };
        for polygon in polygons {
            let (name, ftype, fcode) = (
                attribute(polygon, "GNIS_NAME"),
                attribute(polygon, "FTYPE"),
                attribute(polygon, "FCODE"),
            );
            let id = attribute(polygon, "PERMANENT_");
            // Standing water has one level and a short margin; rivers and tidal water follow the
            // DEM at each vertex and continue past the imagery edge.
            let is_tidal = tidal && TIDAL_TYPES.contains(&ftype.as_str());
            let standing = *layer == "NHDWaterbody" && !is_tidal;
            let local_rect = if standing { pond_rect } else { river_rect };
            let local_rings: Vec<Vec<[f64; 2]>> = polygon
                .rings
                .iter()
                .filter_map(|ring| {
                    ring.iter()
                        .map(|p| projection.project(p[1], p[0]).ok().map(|[n, e]| [e, n]))
                        .collect::<Option<Vec<_>>>()
                })
                .collect();
            if local_rings.len() != polygon.rings.len() {
                out.audit.skipped.push(Skipped {
                    layer,
                    name,
                    ftype,
                    reason: "coordinates outside the regional projection".into(),
                });
                continue;
            }
            let (min, max) = local_rings.iter().flatten().fold(
                ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
                |(lo, hi), p| {
                    (
                        [lo[0].min(p[0]), lo[1].min(p[1])],
                        [hi[0].max(p[0]), hi[1].max(p[1])],
                    )
                },
            );
            let gap = |lo: f64, hi: f64, rlo: f64, rhi: f64| (rlo - hi).max(lo - rhi).max(0.);
            let distance = libm::hypot(
                gap(min[0], max[0], local_rect[0], local_rect[2]),
                gap(min[1], max[1], local_rect[1], local_rect[3]),
            );
            if distance > 0. {
                if !name.is_empty() && distance <= NEARBY_NAMED_M {
                    out.audit
                        .named_features_beyond_drawn_extent_within_5_km
                        .push(Nearby {
                            layer,
                            name,
                            ftype,
                            bbox_distance_m: distance,
                        });
                }
                continue;
            }
            if !drawn_types.contains(&ftype.as_str()) {
                out.audit.skipped.push(Skipped {
                    layer,
                    name,
                    ftype,
                    reason: "not open reflective water (swamp, marsh, playa, ice, estuary or other)".into(),
                });
                continue;
            }
            if !id.is_empty() && !seen.insert(id.clone()) {
                continue; // adjacent HU-8 subbasins repeat features along their shared edge
            }
            let mut rings = Vec::new();
            for (index, ring) in local_rings.iter().enumerate() {
                let clipped = rect_clip(ring, local_rect);
                let mut simple = simplify(&clipped[..clipped.len().saturating_sub(1)], 0.4);
                if let Some(first) = simple.first().copied() {
                    simple.push(first);
                }
                // The exterior must survive; a hole reduced below a triangle simply vanishes.
                // Tidal holes are piers and tiny islands: below this the water simply runs under them.
                let minimum = if tidal && index > 0 { 1_500. } else { 1. };
                if simple.len() >= 4 && ring_area(&simple).abs() > minimum {
                    rings.push(simple);
                } else if index == 0 {
                    rings.clear();
                    break;
                }
            }
            if rings.is_empty() {
                out.audit.skipped.push(Skipped {
                    layer,
                    name,
                    ftype,
                    reason: "no area remains after clipping to the scenery area".into(),
                });
                continue;
            }
            // Very detailed river banks are simplified further until they fit the tessellator.
            for epsilon in [1.5, 3.0, 6.0, 12.0, 24.0] {
                if rings.iter().map(Vec::len).sum::<usize>() <= MAX_TESSELLATION_VERTICES {
                    break;
                }
                for ring in &mut rings {
                    let mut simple = simplify(&ring[..ring.len() - 1], epsilon);
                    if let Some(first) = simple.first().copied() {
                        simple.push(first);
                    }
                    if simple.len() >= 4 {
                        *ring = simple;
                    }
                }
            }
            let vertices: usize = rings.iter().map(Vec::len).sum();
            let mut rings3: Vec<Vec<DVec3>> = rings
                .iter()
                .map(|r| r.iter().map(|p| DVec3::new(p[0], p[1], 0.)).collect())
                .collect();
            let mut triangulated = city_mesh::triangulate(&rings3);
            // Simplifying a coastline and its islands independently can make them touch. The
            // water then runs under the islands (the DEM there is above the water surface, so
            // the terrain hides it), and the polygon is still drawn.
            if tidal && rings3.len() > 1 && triangulated.as_ref().is_err_and(|e| e.contains("intersecting")) {
                rings3.truncate(1);
                rings.truncate(1);
                triangulated = city_mesh::triangulate(&rings3);
            }
            let triangles = match triangulated {
                Ok(t) => t,
                Err(reason) => {
                    out.audit.skipped.push(Skipped {
                        layer,
                        name,
                        ftype,
                        reason,
                    });
                    continue;
                }
            };
            let to_geodetic = |p: DVec3| projection.unproject(p.y, p.x).map_err(|e| e.to_string());
            // Stage every vertex first so a terrain gap (no DEM coverage this far out) skips only
            // this polygon, with the reason on record, instead of aborting the whole build.
            let staged = (|| -> Result<Staged, String> {
                // A standing pond or reservoir has one level: the median of the DEM along its rim,
                // where terrain meets the water. Rivers follow the DEM at each vertex.
                let mut level = None;
                if standing {
                    let mut rim = Vec::new();
                    for p in rings3.iter().flatten().step_by(3) {
                        let [lat, lon] = to_geodetic(*p)?;
                        rim.push((world.height)(lat, lon)?);
                    }
                    level = Some(median(&mut rim));
                }
                let mut staged = Staged::default();
                for t in &triangles {
                    let mut corners = [[0_f32; 3]; 3];
                    for (corner, vertex) in corners.iter_mut().zip(t) {
                        let [lat, lon] = to_geodetic(*vertex)?;
                        let msl = match level {
                            Some(l) => l,
                            None => {
                                // The polygon edge sits on the bank; the lowest nearby DEM sample
                                // is the (hydro-flattened) water surface rather than the bank top.
                                let dlat = 6. / 110_574.;
                                let dlon = 6. / (111_320. * libm::cos(lat.to_radians()));
                                let mut lowest = (world.height)(lat, lon)?;
                                for (a, b) in [(dlat, 0.), (-dlat, 0.), (0., dlon), (0., -dlon)] {
                                    if let Ok(h) = (world.height)(lat + a, lon + b) {
                                        lowest = lowest.min(h);
                                    }
                                }
                                lowest
                            }
                        } + WATER_LIFT_M;
                        staged.level_sum += msl;
                        staged.count += 1;
                        *corner = (world.local)(lat, lon, msl)?;
                        staged.extent = Some(match staged.extent {
                            None => [lon, lat, lon, lat],
                            Some([a, b, c, d]) => [a.min(lon), b.min(lat), c.max(lon), d.max(lat)],
                        });
                        staged.msl = Some(match staged.msl {
                            None => [msl, msl],
                            Some([a, b]) => [a.min(msl), b.max(msl)],
                        });
                    }
                    staged.triangles.push(corners);
                }
                Ok(staged)
            })();
            let staged = match staged {
                Ok(staged) => staged,
                Err(reason) => {
                    out.audit.skipped.push(Skipped {
                        layer,
                        name,
                        ftype,
                        reason: format!("terrain or projection unavailable: {reason}"),
                    });
                    continue;
                }
            };
            let before = out.mesh.indices.len();
            for corners in &staged.triangles {
                out.mesh.triangle(*corners, [[0.5, 0.5]; 3])?;
            }
            if let Some([a, b, c, d]) = staged.extent {
                out.extent = Some(match out.extent {
                    None => [a, b, c, d],
                    Some([w, s, e, n]) => [w.min(a), s.min(b), e.max(c), n.max(d)],
                });
            }
            if let Some([lo, hi]) = staged.msl {
                out.msl_range = Some(match out.msl_range {
                    None => [lo, hi],
                    Some([a, b]) => [a.min(lo), b.max(hi)],
                });
            }
            let (mean_level, count) = (staged.level_sum, staged.count);
            let area_m2 = rings.first().map_or(0., |r| ring_area(r).abs())
                - rings.iter().skip(1).map(|r| ring_area(r).abs()).sum::<f64>();
            out.audit.drawn.push(PolygonAudit {
                layer,
                name,
                ftype,
                fcode,
                permanent_id: id,
                area_m2,
                vertices,
                triangles: (out.mesh.indices.len() - before) / 3,
                level_msl_m: mean_level / f64::from(count.max(1)),
            });
            out.footprints.push(rings);
        }
    }
    out.audit.triangles = out.mesh.indices.len() / 3;
    Ok(out)
}

fn read_layer(
    archive: &mut zip::ZipArchive<std::fs::File>,
    layer: &str,
    bbox: [f64; 4],
) -> Result<Vec<shapefile::Polygon>, String> {
    let wanted = layer.to_ascii_lowercase();
    let find = |archive: &zip::ZipArchive<std::fs::File>, extension: &str| -> Option<String> {
        archive.file_names().filter_map(Result::ok).find_map(|n| {
            let lower = n.to_ascii_lowercase();
            (lower.ends_with(&format!("/{wanted}.{extension}")) || lower == format!("{wanted}.{extension}"))
                .then(|| n.into_owned())
        })
    };
    let (Some(shp), Some(dbf)) = (find(archive, "shp"), find(archive, "dbf")) else {
        return Ok(Vec::new());
    };
    let mut read = |name: &str| -> Result<Vec<u8>, String> {
        let entry = archive.by_name(name).map_err(|e| e.to_string())?;
        if entry.size() > 256 * 1024 * 1024 {
            return Err("NHD layer exceeds its bound".into());
        }
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut std::io::Read::take(entry, 256 * 1024 * 1024 + 1), &mut bytes)
            .map_err(|e| e.to_string())?;
        Ok(bytes)
    };
    let (shp, dbf) = (read(&shp)?, read(&dbf)?);
    shapefile::read_polygons(&shp, &dbf, bbox)
}

/// Builds the water tile and adds it beside trees and buildings. Returns `None` when the cache
/// has no NHD receipts (acquisition not yet run for this area).
#[allow(clippy::too_many_arguments)]
pub(super) fn write(
    out: &Path,
    set: &mut Tileset,
    request: &Request,
    sources: &Sources,
    dem: &mut Dems,
    geoid: &GeoidGrid,
    mosaic: Option<&alignment::Mosaic>,
    lidar_water_cells: Option<&[[f64; 2]]>,
    tidal: bool,
) -> Result<Option<Audit>, String> {
    let inputs: Vec<_> = archives(sources).collect();
    if inputs.is_empty() {
        return Ok(None);
    }
    let areas = Areas::around(geographic_bounds(sources.bounds)?);
    let areas = if tidal { areas.tidal() } else { areas };
    // Search a wider margin so named neighbours (a river just outside) are reported.
    let search = [
        areas.rivers[0] - 0.06,
        areas.rivers[1] - 0.06,
        areas.rivers[2] + 0.06,
        areas.rivers[3] + 0.06,
    ];
    let mut layers: Vec<(&'static str, Vec<shapefile::Polygon>)> =
        vec![("NHDWaterbody", Vec::new()), ("NHDArea", Vec::new())];
    let mut receipts = Vec::new();
    for input in inputs {
        if !input.source.url.starts_with(NHD_PREFIX) {
            return Err("NHD receipt outside the approved prefix".into());
        }
        let path = request
            .cache
            .join("objects")
            .join(download::sha256(input.source.url.as_bytes()));
        let (bytes, hash) = transport::digest(&path)?;
        if bytes != input.source.bytes || hash != input.source.sha256 {
            return Err("NHD archive differs from its receipt".into());
        }
        let mut archive = zip::ZipArchive::new(std::fs::File::open(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        for (layer, all) in &mut layers {
            all.extend(read_layer(&mut archive, layer, search)?);
        }
        receipts.push(input.source.clone());
    }
    let origin = objects::origin();
    let mut height = |lat: f64, lon: f64| {
        if tidal {
            // Open water beyond a DEM project edge is at sea level.
            Ok(dem.height_opt(lat, lon)?.unwrap_or(0.))
        } else {
            dem.height(lat, lon)
        }
    };
    let local = |lat: f64, lon: f64, msl: f64| objects::local(lat, lon, msl, origin, geoid);
    let mut world = World {
        origin,
        height: &mut height,
        local: &local,
    };
    let mut result = tessellate(&layers, areas, &mut world)?;
    result.audit.receipts = receipts;
    // Mean imagery colour over the drawn triangles' footprints becomes the water albedo.
    if let Some(mosaic) = mosaic {
        let projection = LocalProjection::new(origin[0], origin[1]).map_err(|e| e.to_string())?;
        let (mut sum, mut count) = ([0_f64; 3], 0_f64);
        for rings in &result.footprints {
            let Some(outer) = rings.first() else { continue };
            let (min, max) =
                outer
                    .iter()
                    .fold(([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]), |(lo, hi), p| {
                        (
                            [lo[0].min(p[0]), lo[1].min(p[1])],
                            [hi[0].max(p[0]), hi[1].max(p[1])],
                        )
                    });
            let mut y = min[1];
            while y < max[1] {
                let mut x = min[0];
                while x < max[0] {
                    if point_in(outer, [x, y])
                        && !rings.iter().skip(1).any(|h| point_in(h, [x, y]))
                        && let Ok([lat, lon]) = projection.unproject(y, x)
                        && let Some(c) = mosaic.sample(lat, lon)
                    {
                        for (a, v) in sum.iter_mut().zip(c) {
                            *a += f64::from(v);
                        }
                        count += 1.;
                    }
                    x += 3.;
                }
                y += 3.;
            }
        }
        if count > 0. {
            result.audit.water_albedo_srgb = sum.map(|v| (v / count).round().clamp(0., 255.) as u8);
        }
    }
    if let Some(cells) = lidar_water_cells {
        result.audit.lidar_water_cells = Some(cells.len());
        let projection = LocalProjection::new(origin[0], origin[1]).map_err(|e| e.to_string())?;
        let inside = cells
            .iter()
            .filter(|c| {
                projection.project(c[0], c[1]).is_ok_and(|[n, e]| {
                    result.footprints.iter().any(|rings| {
                        rings.first().is_some_and(|outer| point_in(outer, [e, n]))
                            && !rings.iter().skip(1).any(|hole| point_in(hole, [e, n]))
                    })
                })
            })
            .count();
        result.audit.lidar_water_inside_polygons = Some(inside as f64 / cells.len().max(1) as f64);
    }
    let (Some(extent), Some(msl)) = (result.extent, result.msl_range) else {
        download::write_json(&out.join("water-audit.json"), &result.audit)?;
        return Ok(Some(result.audit));
    };
    // One 1x1 lossless WebP carries the measured mean imagery colour of the water; the shader
    // multiplies it into a dark body colour and adds Fresnel sky reflection.
    let [r, g, b] = result.audit.water_albedo_srgb;
    let mut texture = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut texture)
        .encode(&[r, g, b, 255], 1, 1, image::ExtendedColorType::Rgba8)
        .map_err(|e| e.to_string())?;
    let (lo, hi) = (
        geoid
            .ellipsoidal_m(extent[1], extent[0], msl[0])
            .map_err(|e| e.to_string())?,
        geoid
            .ellipsoidal_m(extent[3], extent[2], msl[1])
            .map_err(|e| e.to_string())?,
    );
    std::fs::create_dir_all(out.join("water")).map_err(|e| e.to_string())?;
    // A tile may hold at most 200,000 vertices. Large tidal polygons are cut into west-to-east
    // runs of triangles, each its own content with its own bounds.
    let projection = LocalProjection::new(origin[0], origin[1]).map_err(|e| e.to_string())?;
    let triangles = result.mesh.indices.len() / 3;
    let mut order: Vec<usize> = (0..triangles).collect();
    if triangles > MAX_TILE_TRIANGLES {
        let east = |t: usize| -> f32 {
            (0..3)
                .filter_map(|k| result.mesh.indices.get(t * 3 + k))
                .filter_map(|i| result.mesh.positions.get(*i as usize))
                .map(|p| p[0])
                .sum()
        };
        order.sort_by(|a, b| east(*a).total_cmp(&east(*b)));
    }
    for (n, chunk) in order.chunks(MAX_TILE_TRIANGLES).enumerate() {
        let mut mesh = objects::Mesh::default();
        let (mut west_m, mut south_m) = (f64::INFINITY, f64::INFINITY);
        let (mut east_m, mut north_m) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for &t in chunk {
            let corners: Vec<usize> = (0..3)
                .filter_map(|k| result.mesh.indices.get(t * 3 + k).map(|i| *i as usize))
                .collect();
            let [a, b, c] = corners.as_slice() else {
                return Err("Water triangle".into());
            };
            let p = |i: &usize| result.mesh.positions.get(*i).copied().ok_or("Water vertex");
            let (pa, pb, pc) = (p(a)?, p(b)?, p(c)?);
            for q in [pa, pb, pc] {
                west_m = west_m.min(f64::from(q[0]));
                east_m = east_m.max(f64::from(q[0]));
                // glTF z is minus north.
                south_m = south_m.min(f64::from(-q[2]));
                north_m = north_m.max(f64::from(-q[2]));
            }
            mesh.triangle([pa, pb, pc], [[0.5, 0.5]; 3])?;
        }
        let mut doc = objects::Document::default();
        let mesh = doc.mesh(&mesh)?;
        let bytes = doc.finish_with_extras(
            vec![mesh],
            vec![serde_json::json!({"mesh":0})],
            &texture,
            false,
            false,
            Some(serde_json::json!({"flightcraft":{"water":true}})),
        )?;
        let uri = format!("water/{n}.glb");
        std::fs::write(out.join(&uri), bytes).map_err(|e| e.to_string())?;
        let [lat0, lon0] = projection.unproject(south_m, west_m).map_err(|e| e.to_string())?;
        let [lat1, lon1] = projection.unproject(north_m, east_m).map_err(|e| e.to_string())?;
        let pad = 5. / 111_000.;
        let tile = Tile {
            bounding_volume: BoundingVolume::region([
                (lon0.min(lon1) - pad).to_radians(),
                (lat0.min(lat1) - pad).to_radians(),
                (lon0.max(lon1) + pad).to_radians(),
                (lat0.max(lat1) + pad).to_radians(),
                lo.min(hi) - 1.,
                lo.max(hi) + 1.,
            ]),
            geometric_error: 0.,
            refine: Some(Refine::Replace),
            transform: None,
            content: Some(Content { uri }),
            children: Vec::new(),
        };
        objects::include_bounds(&mut set.root, &tile)?;
        set.root.children.push(tile);
    }
    let flags = set
        .extras
        .as_mut()
        .and_then(|v| v.get_mut("flightcraft"))
        .and_then(|v| v.as_object_mut())
        .ok_or("Water metadata")?;
    flags.insert("waterBuilt".into(), serde_json::json!(true));
    flags.insert(
        "waterPolygons".into(),
        serde_json::json!(result.audit.drawn.len()),
    );
    download::write_json(&out.join("water-audit.json"), &result.audit)?;
    Ok(Some(result.audit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenery::shapefile::{Polygon, fixtures};

    fn square(lon: f64, lat: f64, size: f64, clockwise: bool) -> Vec<[f64; 2]> {
        let mut ring = vec![
            [lon, lat],
            [lon + size, lat],
            [lon + size, lat + size],
            [lon, lat + size],
            [lon, lat],
        ];
        if clockwise {
            ring.reverse();
        }
        ring
    }
    fn polygon(rings: Vec<Vec<[f64; 2]>>, name: &str, ftype: &str, id: &str) -> Polygon {
        Polygon {
            rings,
            attributes: [
                ("GNIS_NAME", name),
                ("FTYPE", ftype),
                ("FCODE", "39004"),
                ("PERMANENT_", id),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
        }
    }
    fn flat_geoid() -> GeoidGrid {
        GeoidGrid {
            model: flightcraft_world::geoid::GeoidModel::NgsGeoid18,
            source_sha256: "a".repeat(64),
            south_deg: 40.9,
            west_deg: -75.1,
            step_deg: [1. / 60.; 2],
            rows: 8,
            cols: 8,
            undulation_m: vec![-33.; 64],
        }
    }
    fn run(layers: &[(&'static str, Vec<Polygon>)], area: [f64; 4]) -> Tessellated {
        run_areas(layers, Areas::same(area))
    }
    fn run_areas(layers: &[(&'static str, Vec<Polygon>)], areas: Areas) -> Tessellated {
        let geoid = flat_geoid();
        let origin = objects::ORIGIN;
        let mut height = |lat: f64, _lon: f64| Ok(100. + (lat - 40.96) * 100.);
        let local = |lat: f64, lon: f64, msl: f64| objects::local(lat, lon, msl, origin, &geoid);
        tessellate(
            layers,
            areas,
            &mut World {
                origin,
                height: &mut height,
                local: &local,
            },
        )
        .unwrap()
    }
    const AREA: [f64; 4] = [-75.005, 40.9655, -74.990, 40.9768];

    #[test]
    fn ponds_get_one_level_with_holes_and_rivers_follow_the_dem() {
        let pond = polygon(
            vec![
                square(-75.0, 40.967, 0.003, true),
                square(-74.9985, 40.968, 0.0005, false),
            ],
            "Test Pond",
            "390",
            "pond-1",
        );
        // A river crossing the area diagonally, a swamp (not drawn), and a far named river.
        let river = polygon(
            vec![
                vec![
                    [-75.01, 40.9655],
                    [-74.99, 40.9765],
                    [-74.99, 40.9758],
                    [-75.01, 40.9648],
                    [-75.01, 40.9655],
                ]
                .into_iter()
                .rev()
                .collect(),
            ],
            "Test Kill",
            "460",
            "river-1",
        );
        let swamp = polygon(vec![square(-74.995, 40.970, 0.002, true)], "", "466", "swamp-1");
        let far = polygon(
            vec![square(-74.95, 40.97, 0.003, true)],
            "Paulins Test",
            "460",
            "far-1",
        );
        let duplicate = pond.clone();
        let result = run(
            &[
                ("NHDWaterbody", vec![pond, swamp, duplicate]),
                ("NHDArea", vec![river, far]),
            ],
            AREA,
        );
        let names: Vec<_> = result.audit.drawn.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["Test Pond", "Test Kill"], "{:#?}", result.audit);
        assert_eq!(result.audit.skipped.len(), 1);
        assert_eq!(
            result.audit.named_features_beyond_drawn_extent_within_5_km[0].name,
            "Paulins Test"
        );
        // The pond is one level; the hole removes area; the river's vertices differ in height.
        let pond_audit = &result.audit.drawn[0];
        let full = 0.003 * 111_000. * 0.003 * 111_000. * libm::cos(40.968_f64.to_radians());
        assert!(
            pond_audit.area_m2 < full && pond_audit.area_m2 > 0.9 * full,
            "{} {}",
            pond_audit.area_m2,
            full
        );
        let ys: Vec<f32> = result.mesh.positions.iter().map(|p| p[1]).collect();
        assert!(ys.iter().all(|y| y.is_finite()));
        let river_audit = &result.audit.drawn[1];
        assert!(river_audit.triangles >= 2);
        assert!(result.extent.is_some());
        // Everything drawn lies inside the requested area (the river was clipped).
        let [w, s, e, n] = result.extent.unwrap();
        assert!(w >= AREA[0] - 1e-6 && s >= AREA[1] - 1e-6 && e <= AREA[2] + 1e-6 && n <= AREA[3] + 1e-6);
    }
    #[test]
    fn rivers_continue_past_the_imagery_but_ponds_do_not() {
        let near_pond = polygon(
            vec![square(-74.9895, 40.970, 0.0006, true)],
            "Near Pond",
            "390",
            "near",
        );
        let far_pond = polygon(
            vec![square(-74.985, 40.970, 0.0006, true)],
            "Far Pond",
            "390",
            "far",
        );
        // A river just east of the area: 40 m wide, 300 m long, starting 80 m outside.
        let river = polygon(vec![square(-74.9892, 40.971, 0.0004, true)], "", "460", "river");
        let areas = Areas::around(AREA);
        let result = run_areas(
            &[
                ("NHDWaterbody", vec![near_pond, far_pond]),
                ("NHDArea", vec![river]),
            ],
            areas,
        );
        let names: Vec<_> = result
            .audit
            .drawn
            .iter()
            .map(|d| (d.layer, d.permanent_id.as_str()))
            .collect();
        assert_eq!(
            names,
            [("NHDWaterbody", "near"), ("NHDArea", "river")],
            "{:#?}",
            result.audit
        );
        assert!(
            result
                .audit
                .named_features_beyond_drawn_extent_within_5_km
                .iter()
                .any(|n| n.name == "Far Pond")
        );
    }
    #[test]
    fn terrain_gaps_skip_one_polygon_and_leave_the_rest() {
        let a = polygon(
            vec![square(-75.0, 40.967, 0.003, true)],
            "Has Terrain",
            "390",
            "a",
        );
        let b = polygon(
            vec![square(-74.995, 40.9715, 0.001, true)],
            "No Terrain",
            "390",
            "b",
        );
        let geoid = flat_geoid();
        let origin = objects::ORIGIN;
        let mut height = |_lat: f64, lon: f64| {
            if lon > -74.996 {
                Err("DEM does not cover this position".to_string())
            } else {
                Ok(100.)
            }
        };
        let local = |lat: f64, lon: f64, msl: f64| objects::local(lat, lon, msl, origin, &geoid);
        let result = tessellate(
            &[("NHDWaterbody", vec![a, b])],
            Areas::same(AREA),
            &mut World {
                origin,
                height: &mut height,
                local: &local,
            },
        )
        .unwrap();
        assert_eq!(result.audit.drawn.len(), 1);
        assert_eq!(result.audit.skipped.len(), 1);
        assert!(result.audit.skipped[0].reason.contains("terrain"));
    }
    #[test]
    fn real_shapefile_bytes_flow_through_the_reader_and_a_valid_water_glb() {
        let shp = fixtures::polygon_shapefile(&[vec![square(-75.0, 40.967, 0.003, true)]]);
        let dbf = fixtures::dbf(&[("Test Pond", 390, 39004)]);
        let polygons = shapefile::read_polygons(&shp, &dbf, AREA).unwrap();
        assert_eq!(polygons[0].attributes["FTYPE"], "390");
        let result = run(&[("NHDWaterbody", polygons)], AREA);
        let mut doc = objects::Document::default();
        let mesh = doc.mesh(&result.mesh).unwrap();
        let bytes = doc
            .finish_with_extras(
                vec![mesh],
                vec![serde_json::json!({"mesh":0})],
                &{
                    let mut t = Vec::new();
                    image::codecs::webp::WebPEncoder::new_lossless(&mut t)
                        .encode(&[18, 42, 46, 255], 1, 1, image::ExtendedColorType::Rgba8)
                        .unwrap();
                    t
                },
                false,
                false,
                Some(serde_json::json!({"flightcraft":{"water":true}})),
            )
            .unwrap();
        let tile = flightcraft_render::scenery::decode_glb(
            &bytes,
            flightcraft_world::tiles3d::enu_transform(40.96905613156273, -75.00242278301268, 0.).unwrap(),
            objects::ORIGIN,
        );
        assert!(tile.unwrap().is_water());
    }
    #[test]
    fn archive_with_nothing_in_the_area_draws_nothing_and_a_clipped_sliver_is_dropped() {
        let outside = polygon(vec![square(-74.9, 40.9, 0.003, true)], "Elsewhere", "390", "x");
        let result = run(&[("NHDWaterbody", vec![outside])], AREA);
        assert!(result.audit.drawn.is_empty() && result.extent.is_none());
        let sliver = polygon(vec![square(-74.995, 40.97, 0.00001, true)], "", "390", "sliver");
        let result = run(&[("NHDWaterbody", vec![sliver])], AREA);
        assert!(result.audit.drawn.is_empty());
    }
}
