//! Offline build of `new-york/hudson-corridor`: terrain+imagery pyramid, lidar buildings and
//! trees, NHD water, the three release gates, and one 3D Tiles tileset.
use super::{
    InputKind, Request, Sources,
    corridor::{self, AREA, FINEST, Plan, TILE_PX},
    corridor_buildings::{self as cb, Bounds, Imagery, TileMeta, Totals},
    corridor_glb::{self as glb, Frame, Mesh, Texture},
    datum,
    dem::Dems,
    download, geoid, objects,
};
use flightcraft_world::{
    scenery::{self, TileKey},
    tiles3d::{self, Asset, BoundingVolume, Content, Refine, Tile, Tileset},
};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;

/// Terrain mesh cells per side. The 0.6 m tiles carry the detail; coarser levels are viewed from
/// far away and are mostly flat river and city, so they get fewer posts (and fewer bytes).
fn grid_cells(level: u8) -> u32 {
    match level {
        8 => 32,
        7 => 24,
        6 => 16,
        _ => 12,
    }
}
pub(super) const BUILDING_BUDGET_BYTES: u64 = flightcraft_render::scenery::NYC_BUILDING_BUDGET;

fn region(b: &Bounds) -> Result<[f64; 6], String> {
    let [south, west] = scenery::inverse_mercator(b.rect[0], b.rect[1]).map_err(|e| e.to_string())?;
    let [north, east] = scenery::inverse_mercator(b.rect[2], b.rect[3]).map_err(|e| e.to_string())?;
    let pad = 1. / 111_000.;
    Ok([
        (west - pad).to_radians(),
        (south - pad).to_radians(),
        (east + pad).to_radians(),
        (north + pad).to_radians(),
        b.lo - 1.,
        b.hi + 1.,
    ])
}
fn union_region(a: &mut [f64; 6], b: [f64; 6]) {
    a[0] = a[0].min(b[0]);
    a[1] = a[1].min(b[1]);
    a[2] = a[2].max(b[2]);
    a[3] = a[3].max(b[3]);
    a[4] = a[4].min(b[4]);
    a[5] = a[5].max(b[5]);
}

#[derive(Default)]
struct TerrainStats {
    tiles: usize,
    bytes_imagery: u64,
    bytes_mesh: u64,
    dem_fallback_vertices: u64,
    vertices: u64,
}
struct TerrainTile {
    heights: [f64; 2],
}

fn jpeg(image: &image::RgbaImage, quality: u8) -> Result<Vec<u8>, String> {
    let rgb = image::DynamicImage::ImageRgba8(image.clone()).to_rgb8();
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality)
        .encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}

fn terrain(
    request: &Request,
    plan: &Plan,
    sources: &Sources,
    out: &Path,
    dem: &mut Dems,
    frame: &Frame,
    stats: &mut TerrainStats,
) -> Result<BTreeMap<TileKey, TerrainTile>, String> {
    let work = out.join(".work");
    let leaves: BTreeMap<TileKey, &super::Input> = sources
        .inputs
        .iter()
        .filter(|i| i.kind == InputKind::Imagery)
        .filter_map(|i| Some((i.key?, i)))
        .collect();
    let mut tiles = plan.tiles();
    // Finest first, so every parent can be composed from its four finished children.
    tiles.sort_by_key(|(k, _)| (std::cmp::Reverse(k.level), k.y, k.x));
    let mut done = BTreeMap::new();
    let total = tiles.len();
    for (n, (key, leaf)) in tiles.into_iter().enumerate() {
        let (texture, work_bytes) = if leaf {
            let input = leaves.get(&key).ok_or("corridor imagery leaf has no receipt")?;
            let bytes = super::input_bytes(request, input)?;
            let image = super::decode_image(&bytes, Some(TILE_PX))?;
            // Embed the NAIP JPEG as delivered unless it is unusually large.
            let texture = if bytes.len() <= 140_000 {
                bytes.clone()
            } else {
                jpeg(&image, 82)?
            };
            (texture, bytes)
        } else {
            let mut mosaic = image::RgbaImage::new(2 * TILE_PX, 2 * TILE_PX);
            for (i, child) in plan.children(key).iter().enumerate() {
                // Coarse levels above the forest have children that do not exist: black, never seen.
                let Ok(bytes) = download::read(&work.join(format!("{}.jpg", child.name())), 8 * 1024 * 1024)
                else {
                    continue;
                };
                let image = super::decode_image(&bytes, Some(TILE_PX))?;
                image::imageops::replace(
                    &mut mosaic,
                    &image,
                    i64::from((i as u32 % 2) * TILE_PX),
                    i64::from((i as u32 / 2) * TILE_PX),
                );
            }
            let image =
                image::imageops::resize(&mosaic, TILE_PX, TILE_PX, image::imageops::FilterType::Triangle);
            let bytes = jpeg(&image, 86)?;
            (bytes.clone(), bytes)
        };
        let path = work.join(format!("{}.jpg", key.name()));
        std::fs::create_dir_all(path.parent().ok_or("work parent")?).map_err(|e| e.to_string())?;
        std::fs::write(&path, &work_bytes).map_err(|e| e.to_string())?;

        let [w, s, e, nn] = plan.tile_bounds(key);
        let mut mesh = Mesh::default();
        let mut heights = [f64::INFINITY, f64::NEG_INFINITY];
        let grid = grid_cells(key.level);
        for row in 0..=grid {
            for col in 0..=grid {
                let (u, v) = (f64::from(col) / f64::from(grid), f64::from(row) / f64::from(grid));
                let [lat, lon] = scenery::inverse_mercator(w + u * (e - w), nn - v * (nn - s))
                    .map_err(|e| e.to_string())?;
                let msl = match dem.height_opt(lat, lon)? {
                    Some(h) => h,
                    None => {
                        stats.dem_fallback_vertices += 1;
                        0.
                    }
                };
                stats.vertices += 1;
                let (p, h) = frame.local_h(lat, lon, msl)?;
                heights = [heights[0].min(h), heights[1].max(h)];
                mesh.positions.push(p);
                mesh.uv.push([u as f32, v as f32]);
            }
        }
        for row in 0..grid {
            for col in 0..grid {
                let a = row * (grid + 1) + col;
                let (b, c) = (a + 1, a + grid + 1);
                mesh.indices.extend([a, c, b, b, c, c + 1]);
            }
        }
        let bytes = glb::write_glb(&mesh, Texture::Jpeg(&texture), Some(glb::terrain_extras()))?;
        let path = out.join(format!("tiles/{}.glb", key.name()));
        std::fs::create_dir_all(path.parent().ok_or("tile parent")?).map_err(|e| e.to_string())?;
        std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
        stats.tiles += 1;
        stats.bytes_imagery += texture.len() as u64;
        stats.bytes_mesh += bytes.len() as u64 - texture.len() as u64;
        done.insert(key, TerrainTile { heights });
        if n % 200 == 0 {
            eprintln!("corridor terrain {n}/{total}");
        }
    }
    std::fs::remove_dir_all(&work).map_err(|e| e.to_string())?;
    Ok(done)
}

fn terrain_node(
    plan: &Plan,
    key: TileKey,
    tiles: &BTreeMap<TileKey, TerrainTile>,
    leaves: &std::collections::BTreeSet<TileKey>,
) -> Result<Option<Tile>, String> {
    let [w, s, e, n] = plan.tile_bounds(key);
    let mut children = Vec::new();
    let candidates: Vec<TileKey> = if key.level < FINEST {
        plan.children(key).to_vec()
    } else {
        Vec::new()
    };
    for child in candidates {
        if tiles.contains_key(&child)
            && let Some(node) = terrain_node(plan, child, tiles, leaves)?
        {
            children.push(node);
        }
    }
    let content = tiles.contains_key(&key);
    if !content && children.is_empty() {
        return Ok(None);
    }
    let [south, west] = scenery::inverse_mercator(w, s).map_err(|e| e.to_string())?;
    let [north, east] = scenery::inverse_mercator(e, n).map_err(|e| e.to_string())?;
    let [mut lo, mut hi] = tiles
        .get(&key)
        .map_or([f64::INFINITY, f64::NEG_INFINITY], |t| t.heights);
    for c in &children {
        if let Some([_, _, _, _, a, b]) = c.bounding_volume.region {
            lo = lo.min(a);
            hi = hi.max(b);
        }
    }
    let is_leaf = leaves.contains(&key);
    Ok(Some(Tile {
        bounding_volume: BoundingVolume::region([
            west.to_radians(),
            south.to_radians(),
            east.to_radians(),
            north.to_radians(),
            lo,
            hi,
        ]),
        geometric_error: if is_leaf {
            0.
        } else {
            (e - w) / f64::from(grid_cells(key.level))
        },
        refine: Some(Refine::Replace),
        transform: None,
        content: content.then(|| Content {
            uri: format!("tiles/{}.glb", key.name()),
        }),
        children,
    }))
}

/// Content-less spatial aggregator over group tiles (<= 8 children each). The group tiles below
/// are small and always wanted, so requiring all children resident costs little.
fn group(mut nodes: Vec<Tile>, depth: u32) -> Result<Tile, String> {
    let mut region_bounds = nodes
        .first()
        .and_then(|n| n.bounding_volume.region)
        .ok_or("empty tile group")?;
    for n in &nodes {
        union_region(
            &mut region_bounds,
            n.bounding_volume.region.ok_or("group bounds")?,
        );
    }
    let mut node = Tile {
        bounding_volume: BoundingVolume::region(region_bounds),
        geometric_error: 100_000.,
        refine: Some(Refine::Replace),
        transform: None,
        content: None,
        children: Vec::new(),
    };
    if nodes.len() <= 8 {
        node.children = nodes;
    } else {
        let axis = (depth % 2) as usize;
        nodes.sort_by(|a, b| {
            let c = |t: &Tile| {
                t.bounding_volume
                    .region
                    .map_or(0., |r| (r[axis] + r[axis + 2]) / 2.)
            };
            c(a).total_cmp(&c(b))
        });
        let half = nodes.split_off(nodes.len() / 2);
        node.children = vec![group(nodes, depth + 1)?, group(half, depth + 1)?];
    }
    Ok(node)
}

/// Fine cells hang under their 1 km coarse parent; parents are grouped into <= 32-child nodes.
pub(super) fn building_tree(metas: &[TileMeta]) -> Result<Option<Tile>, String> {
    type Parent<'a> = (Option<&'a TileMeta>, Vec<&'a TileMeta>);
    let mut parents: BTreeMap<(i64, i64), Parent<'_>> = BTreeMap::new();
    let mut groups: BTreeMap<(i64, i64), &TileMeta> = BTreeMap::new();
    for m in metas {
        if m.lod == 2 {
            groups.insert(m.cell, m);
        } else if m.lod == 0 {
            parents.entry(m.cell).or_default().0 = Some(m);
        } else {
            parents
                .entry((m.cell.0.div_euclid(4), m.cell.1.div_euclid(4)))
                .or_default()
                .1
                .push(m);
        }
    }
    let mut under_group: BTreeMap<(i64, i64), Vec<(Tile, Bounds)>> = BTreeMap::new();
    for (coarse, fine) in parents.values() {
        let mut bounds = Bounds {
            rect: [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY],
            lo: f64::INFINITY,
            hi: f64::NEG_INFINITY,
        };
        let mut children = Vec::new();
        for m in fine {
            bounds.union(&m.bounds);
            children.push(Tile {
                bounding_volume: BoundingVolume::region(region(&m.bounds)?),
                geometric_error: 0.,
                refine: Some(Refine::Replace),
                transform: None,
                content: Some(Content { uri: m.uri.clone() }),
                children: Vec::new(),
            });
        }
        if let Some(m) = coarse {
            bounds.union(&m.bounds);
        }
        if !bounds.valid() {
            continue;
        }
        let key = coarse
            .map(|m| m.cell)
            .or_else(|| {
                fine.first()
                    .map(|m| (m.cell.0.div_euclid(4), m.cell.1.div_euclid(4)))
            })
            .ok_or("parent without tiles")?;
        let tile = Tile {
            bounding_volume: BoundingVolume::region(region(&bounds)?),
            geometric_error: if children.is_empty() { 0. } else { 16. },
            refine: Some(Refine::Replace),
            transform: None,
            content: coarse.map(|m| Content { uri: m.uri.clone() }),
            children,
        };
        under_group
            .entry((key.0.div_euclid(2), key.1.div_euclid(2)))
            .or_default()
            .push((tile, bounds));
    }
    let mut nodes = Vec::new();
    for (cell, members) in under_group {
        let mut bounds = Bounds {
            rect: [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY],
            lo: f64::INFINITY,
            hi: f64::NEG_INFINITY,
        };
        for (_, b) in &members {
            bounds.union(b);
        }
        let g = groups.get(&cell);
        if let Some(m) = g {
            bounds.union(&m.bounds);
        }
        if !bounds.valid() {
            continue;
        }
        nodes.push(Tile {
            bounding_volume: BoundingVolume::region(region(&bounds)?),
            geometric_error: 64.,
            refine: Some(Refine::Replace),
            transform: None,
            content: g.map(|m| Content { uri: m.uri.clone() }),
            children: members.into_iter().map(|(t, _)| t).collect(),
        });
    }
    if nodes.is_empty() {
        return Ok(None);
    }
    Ok(Some(group(nodes, 0)?))
}

fn percentile(values: &mut [f32], q: f64) -> Option<f64> {
    values.sort_by(f32::total_cmp);
    values
        .get((q * values.len().saturating_sub(1) as f64) as usize)
        .map(|v| f64::from(*v))
}

pub(super) fn build(request: &Request) -> Result<(), String> {
    let plan = corridor::plan();
    let sources: Sources = download::json(&request.cache.join("sources.json"))?;
    let expected = corridor::new_sources(&plan);
    if sources.region != expected.region
        || sources.area != expected.area
        || sources.bounds != expected.bounds
        || sources.max_level != expected.max_level
    {
        return Err("source catalog differs from the hudson-corridor plan".into());
    }
    let have: std::collections::BTreeSet<TileKey> = sources
        .inputs
        .iter()
        .filter(|i| i.kind == InputKind::Imagery)
        .filter_map(|i| i.key)
        .collect();
    let want: std::collections::BTreeSet<TileKey> = plan.leaves().into_iter().collect();
    if have != want {
        return Err(format!(
            "imagery receipts do not match the plan ({} fetched, {} planned); finish the fetch",
            have.len(),
            want.len()
        ));
    }
    let selection = match sources.ept_selection.clone() {
        Some(selection) => selection,
        // A development run of a few blocks may use whatever nodes are already receipted; it can
        // never pass the gates (`partial`), so nothing it builds can be packaged.
        None if std::env::var_os("FLIGHTCRAFT_CORRIDOR_BLOCKS").is_some() => super::ept::SelectionSummary {
            depth: 8,
            node_count: sources
                .inputs
                .iter()
                .filter(|i| i.kind == InputKind::EptLidar)
                .count(),
            hierarchy_pages: 0,
            selected_points_before_clipping: 0,
        },
        None => {
            return Err("complete the EPT selection (scenery fetch --layer lidar) before building".into());
        }
    };
    if request.out.exists() {
        return Err("scenery build output must be a new directory".into());
    }
    let staging = request.out.with_extension("building");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| format!("{}: {e}", staging.display()))?;
    match build_inner(request, &plan, &sources, &selection, &staging) {
        Ok(()) => std::fs::rename(&staging, &request.out).map_err(|e| e.to_string()),
        Err(error) => {
            // Keep the staging directory for inspection only when asked.
            if std::env::var_os("FLIGHTCRAFT_KEEP_STAGING").is_none() {
                let _ = std::fs::remove_dir_all(&staging);
            }
            Err(error)
        }
    }
}

#[allow(clippy::too_many_lines)]
fn build_inner(
    request: &Request,
    plan: &Plan,
    sources: &Sources,
    selection: &super::ept::SelectionSummary,
    out: &Path,
) -> Result<(), String> {
    let started = std::time::Instant::now();
    let mut dem = Dems::open(request, sources)?;
    // The coarse tiles above the forest span the whole root square, so the regional geoid subset
    // (embedded in the tileset and applied by the renderer to every vertex) must cover it too.
    let mut geoid_sources = sources.clone();
    geoid_sources.bounds = plan.root;
    let grid =
        geoid::load(request, &geoid_sources)?.ok_or("corridor build needs the receipted GEOID18 grid")?;
    let vertical = datum::evaluate(request, sources, Some(&grid), Some(&mut dem))?;
    download::write_json(&out.join("datum-audit.json"), &vertical)?;
    eprintln!("vertical datum gate: passed={}", vertical.passed);
    let origin = corridor::ROOT_CENTER;
    objects::set_origin(origin);
    let frame = Frame::new(origin, &grid)?;
    let mut terrain_stats = TerrainStats::default();
    let tiles = terrain(request, plan, sources, out, &mut dem, &frame, &mut terrain_stats)?;
    eprintln!("terrain done in {:?}", started.elapsed());

    // Lidar blocks.
    let nodes = cb::nodes(request, sources)?;
    let mut imagery = Imagery::new(plan, request, sources);
    let output = cb::Output {
        out,
        plan,
        frame: &frame,
        geoid: &grid,
    };
    let mut totals = Totals::default();
    let mut metas: Vec<TileMeta> = Vec::new();
    let mut trees = Vec::new();
    let specs = cb::blocks(plan);
    let only: Option<usize> = std::env::var("FLIGHTCRAFT_CORRIDOR_BLOCKS")
        .ok()
        .and_then(|v| v.parse().ok());
    for (n, spec) in specs.iter().enumerate() {
        if only.is_some_and(|limit| n >= limit) {
            break;
        }
        // Development: only the block holding Midtown (with FLIGHTCRAFT_CORRIDOR_BLOCKS set).
        let midtown = corridor::merc(40.7536, -73.9832);
        if only.is_some()
            && std::env::var_os("FLIGHTCRAFT_CORRIDOR_MIDTOWN").is_some()
            && !(midtown[0] >= spec.core[0]
                && midtown[0] < spec.core[2]
                && midtown[1] >= spec.core[1]
                && midtown[1] < spec.core[3])
        {
            continue;
        }
        let t = std::time::Instant::now();
        let (m, t_trees) = cb::process_block(spec, &nodes, &output, &mut dem, &mut imagery, &mut totals)?;
        eprintln!(
            "block {}/{}: {} tiles, {} trees, {} buildings so far, {:?}",
            n + 1,
            specs.len(),
            m.len(),
            t_trees.len(),
            totals.buildings,
            t.elapsed()
        );
        metas.extend(m);
        trees.extend(t_trees);
    }
    let partial = only.is_some();

    // Tileset.
    let leaf_set: std::collections::BTreeSet<TileKey> = plan.leaves().into_iter().collect();
    let forest = terrain_node(plan, TileKey { level: 0, x: 0, y: 0 }, &tiles, &leaf_set)?
        .ok_or("empty corridor terrain")?;
    let mut region_bounds = forest.bounding_volume.region.ok_or("terrain bounds")?;
    let mut root_children = vec![forest];
    let building_root = building_tree(&metas)?;
    if let Some(node) = building_root {
        union_region(
            &mut region_bounds,
            node.bounding_volume.region.ok_or("building bounds")?,
        );
        root_children.push(node);
    }
    let mut root = Tile {
        bounding_volume: BoundingVolume::region(region_bounds),
        geometric_error: 1_000_000.,
        refine: Some(Refine::Replace),
        transform: Some(tiles3d::enu_transform(origin[0], origin[1], 0.).map_err(|e| e.to_string())?),
        content: None,
        children: root_children,
    };
    let _ = &mut root;
    let mut set = Tileset {
        asset: Asset {
            version: "1.1".into(),
            copyright: Some(flightcraft_render::scenery::NYC_ATTRIBUTION.into()),
        },
        geometric_error: 1_000_000.,
        root,
        extensions_required: Vec::new(),
        extras: Some(json!({"flightcraft":{
            "previewOnly":true,"verticalDatumVerified":false,"runwayAlignmentVerified":false,"lidarBuilt":false,
            "sourceResolutionVerified":false,"region":"new-york","area":AREA,"groundSampleMeters":corridor::GROUND_SAMPLE_M,
            "geoid":serde_json::to_value(&grid).map_err(|e| e.to_string())?
        }})),
    };
    // Trees as instanced crowns in a deeper quadtree than the 1N7 test needs.
    objects::write_trees_split(
        out,
        &mut set,
        &trees,
        &grid,
        objects::TreeSplit {
            max_level: 6,
            leaf: 1500,
            parent_cap: 12_000,
            variants: 2,
        },
    )?;
    // NHD water, with the data bounds narrowed to where water surfaces are drawn.
    let mut water_sources = sources.clone();
    water_sources.bounds = plan.water_bounds();
    let water = super::water_mesh::write(
        out,
        &mut set,
        request,
        &water_sources,
        &mut dem,
        &grid,
        None,
        Some(&totals.water_cells),
        true,
    )?;
    eprintln!("blocks + trees + water done in {:?}", started.elapsed());

    // Gates.
    let water_cover = water
        .as_ref()
        .and_then(|w| Some((w.lidar_water_cells?, w.lidar_water_inside_polygons?)));
    let gate = super::corridor_gates::evaluate(
        plan,
        &totals,
        &selection_check(selection, &totals),
        vertical.passed,
        partial,
        water_cover,
    )?;
    download::write_json(&out.join("lidar-acceptance.json"), &gate.lidar)?;
    download::write_json(&out.join("alignment-audit.json"), &gate.alignment)?;
    let mut heights = totals.building_heights.clone();
    heights.sort_by(f32::total_cmp);
    let bin = |lo: f32, hi: f32| heights.iter().filter(|h| **h >= lo && **h < hi).count();
    download::write_json(
        &out.join("buildings-audit.json"),
        &json!({
            "buildings": totals.buildings,
            "roof_planes": totals.planes,
            "roof_cells_measured": totals.roof_cells_measured,
            "roof_cells_interpolated": totals.roof_cells_interpolated,
            "roof_cells_excluded": totals.excluded_roof_cells,
            "inferred_from_unclassified_returns": totals.buildings - totals.classified_buildings.min(totals.buildings),
            "with_class_6": totals.classified_buildings,
            "height_m": {"under_10":bin(0.,10.),"10_to_30":bin(10.,30.),"30_to_100":bin(30.,100.),"100_to_200":bin(100.,200.),"over_200":bin(200.,1000.),"max":heights.last()},
            "trees": totals.trees,
            "lidar_classes": totals.classes,
            "source": "USGS 3DEP NY_NewYorkCity EPT only; no footprint, height or extrusion from any other source",
        }),
    )?;
    let ground = totals.ground.finish();
    download::write_json(&out.join("lidar-ground-audit.json"), &ground)?;
    let receipts: Vec<_> = sources
        .inputs
        .iter()
        .filter(|i| matches!(i.kind, InputKind::EptMetadata | InputKind::EptLidar))
        .map(|i| serde_json::to_value(&i.source))
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let accepted = !partial && vertical.passed && gate.alignment.passed && gate.lidar.passed;
    {
        let flags = set
            .extras
            .as_mut()
            .and_then(|v| v.get_mut("flightcraft"))
            .and_then(|v| v.as_object_mut())
            .ok_or("tileset flags")?;
        flags.insert("previewOnly".into(), json!(!accepted));
        flags.insert("verticalDatumVerified".into(), json!(vertical.passed));
        flags.insert("runwayAlignmentVerified".into(), json!(gate.alignment.passed));
        flags.insert("alignmentMethod".into(), json!(gate.alignment.method));
        flags.insert("lidarBuilt".into(), json!(true));
        flags.insert("lidarAcceptanceVerified".into(), json!(gate.lidar.passed));
        flags.insert("sourceKind".into(), json!("usgs-3dep-lidar"));
        flags.insert("roofPolicy".into(), json!("lidar-only"));
        flags.insert("localOnly".into(), json!(false));
        flags.insert("redistributionAllowed".into(), json!(true));
        flags.insert("buildingsBuilt".into(), json!(true));
        flags.insert("buildingCount".into(), json!(totals.buildings));
        flags.insert(
            "nycLidar".into(),
            json!({
                "selection": flightcraft_render::scenery::fly_nyc_area(),
                "reconstructionBuildings": totals.buildings,
                "reconstructionLeavesAreBuildings": true,
                "usesExternalFootprints": false,
                "receipts": receipts,
            }),
        );
    }
    set.validate().map_err(|e| e.to_string())?;
    download::write_json(&out.join("tileset.json"), &set)?;
    download::write_json(&out.join("sources.json"), sources)?;

    // Budgets, measured on disk.
    let building_bytes = dir_bytes(&out.join("buildings"))?;
    let total_bytes = dir_bytes(out)?;
    let report = json!({
        "region":"new-york","area":AREA,
        "manifest": request.out.join("tileset.json"),
        "release_ready": accepted,
        "preview_only": !accepted,
        "partial_build": partial,
        "geoid_applied": true,
        "vertical_datum_verified": vertical.passed,
        "runway_alignment_verified": gate.alignment.passed,
        "lidar_built": true,
        "lidar_accepted": gate.lidar.passed,
        "terrain_tiles": terrain_stats.tiles,
        "imagery_tiles": plan.leaves().len(),
        "source_files": sources.inputs.len(),
        "buildings": totals.buildings,
        "building_tiles": metas.len(),
        "building_tile_vertices": metas.iter().map(|m| m.vertices).sum::<usize>(),
        "building_tile_bytes": metas.iter().map(|m| m.bytes).sum::<u64>(),
        "building_tile_building_instances": metas.iter().filter(|m| m.lod == 1).map(|m| m.buildings).sum::<usize>(),
        "trees": totals.trees,
        "ept_selection": selection,
        "bytes": {
            "total": total_bytes,
            "buildings": building_bytes,
            "buildings_fine": totals.bytes_fine,
            "buildings_coarse": totals.bytes_coarse,
            "terrain_imagery": terrain_stats.bytes_imagery,
            "terrain_mesh": terrain_stats.bytes_mesh,
            "building_budget": BUILDING_BUDGET_BYTES,
        },
        "max_building_tile_vertices": totals.max_tile_vertices,
        "dem_fallback_vertices": terrain_stats.dem_fallback_vertices,
        "dem_undecodable_chunk_hits": dem.decode_errors,
        "dem_missing_lidar_cells": totals.dem_missing_cells,
        "skipped_blocks": totals.skipped_blocks,
        "water": water.as_ref().map(|w| json!({"drawn":w.drawn.len(),"skipped":w.skipped.len(),"triangles":w.triangles,"lidar_water_cells":w.lidar_water_cells,"lidar_water_inside_polygons":w.lidar_water_inside_polygons})),
        "elapsed_seconds": started.elapsed().as_secs(),
    });
    download::write_json(&out.join("build-report.json"), &report)?;
    if building_bytes > BUILDING_BUDGET_BYTES {
        return Err(format!(
            "buildings are {building_bytes} bytes, above the {BUILDING_BUDGET_BYTES} byte budget; reduce the zone or detail (nothing was dropped silently)"
        ));
    }
    let _ = (&mut totals.plane_rms, percentile(&mut [], 0.5));
    Ok(())
}

fn selection_check(
    selection: &super::ept::SelectionSummary,
    totals: &Totals,
) -> super::corridor_gates::Selection {
    super::corridor_gates::Selection {
        nodes_selected: selection.node_count,
        points_selected: selection.selected_points_before_clipping,
        nodes_decoded: totals.nodes_decoded.len(),
        points_decoded: totals.node_points.values().sum(),
    }
}

fn dir_bytes(root: &Path) -> Result<u64, String> {
    Ok(download_files(root)?
        .iter()
        .map(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
        .sum())
}
fn download_files(root: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    crate::data::files(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decodes every content of a built tileset with the renderer's loader:
    /// `FLIGHTCRAFT_DECODE_DIR=<built dir> cargo test -p xtask decode_every_tile -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs a built hudson-corridor directory"]
    fn decode_every_tile_with_the_renderer_loader() {
        let dir = std::path::PathBuf::from(std::env::var_os("FLIGHTCRAFT_DECODE_DIR").unwrap());
        let set: Tileset = serde_json::from_slice(&std::fs::read(dir.join("tileset.json")).unwrap()).unwrap();
        let geoid = set.geoid_grid().unwrap();
        let transform = set.root.transform.unwrap();
        let origin = corridor::ROOT_CENTER;
        let mut pending = vec![&set.root];
        let (mut n, mut bad) = (0, 0);
        while let Some(t) = pending.pop() {
            pending.extend(&t.children);
            if let Some(c) = &t.content {
                n += 1;
                let bytes = std::fs::read(dir.join(&c.uri)).unwrap();
                if let Err(e) = flightcraft_render::scenery::decode_glb_with_geoid(
                    &bytes,
                    transform,
                    origin,
                    geoid.as_ref(),
                ) {
                    bad += 1;
                    if bad <= 10 {
                        eprintln!("{} ({} bytes): {e}", c.uri, bytes.len());
                    }
                }
            }
        }
        eprintln!("{n} contents, {bad} failed");
        assert_eq!(bad, 0);
    }

    /// NHD water for the whole corridor from the fetched cache:
    /// `cargo test -p xtask real_water -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the fetched hudson-corridor cache"]
    fn real_water_draws_the_hudson_east_river_and_harbor() {
        let cache =
            std::path::PathBuf::from("/home/ray/builds/codex-jobs/scenery/new-york/hudson-corridor/cache");
        let sources: Sources = download::json(&cache.join("sources.json")).unwrap();
        let request = Request {
            operation: super::super::Operation::Build,
            fetch_layer: super::super::FetchLayer::All,
            lidar_source: super::super::LidarSource::Ept,
            region: "new-york".into(),
            area: AREA.into(),
            cache,
            out: std::path::PathBuf::new(),
            sample: None,
            allow_http: false,
        };
        let plan = corridor::plan();
        let mut dem = Dems::open(&request, &sources).unwrap();
        let grid = geoid::load(&request, &sources).unwrap().unwrap();
        objects::set_origin(corridor::ROOT_CENTER);
        let out = std::env::temp_dir().join("fc-water-corridor");
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).unwrap();
        let mut set = Tileset {
            asset: Asset {
                version: "1.1".into(),
                copyright: None,
            },
            geometric_error: 1_000_000.,
            root: Tile {
                bounding_volume: BoundingVolume::region([-1.29, 0.71, -1.28, 0.72, 0., 10.]),
                geometric_error: 1_000_000.,
                refine: Some(Refine::Replace),
                transform: None,
                content: None,
                children: Vec::new(),
            },
            extensions_required: Vec::new(),
            extras: Some(json!({"flightcraft":{}})),
        };
        let mut water_sources = sources.clone();
        water_sources.bounds = plan.water_bounds();
        let started = std::time::Instant::now();
        let audit = super::super::water_mesh::write(
            &out,
            &mut set,
            &request,
            &water_sources,
            &mut dem,
            &grid,
            None,
            None,
            true,
        )
        .unwrap()
        .unwrap();
        std::fs::write(out.join("water-tileset.json"), serde_json::to_vec(&set).unwrap()).unwrap();
        eprintln!(
            "water: {} polygons drawn, {} skipped, {} triangles, {} B, {:?}",
            audit.drawn.len(),
            audit.skipped.len(),
            audit.triangles,
            std::fs::metadata(out.join("water/0.glb"))
                .map(|m| m.len())
                .unwrap_or(0),
            started.elapsed()
        );
        for d in audit.drawn.iter().filter(|d| d.area_m2 > 2e5) {
            eprintln!(
                "  drawn {} {} {:.1} km2, {} vertices",
                d.name,
                d.ftype,
                d.area_m2 / 1e6,
                d.vertices
            );
        }
        for s in audit
            .skipped
            .iter()
            .filter(|s| s.ftype == "493" || s.ftype == "445")
        {
            eprintln!("  skipped tidal {} {}: {}", s.name, s.ftype, s.reason);
        }
        for s in audit
            .skipped
            .iter()
            .filter(|s| s.reason.contains("terrain"))
            .take(5)
        {
            eprintln!("  skipped {} {}: {}", s.name, s.ftype, s.reason);
        }
        assert!(!audit.drawn.is_empty());
    }
}
