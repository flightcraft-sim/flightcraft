//! Original 3D Tiles 1.1 / embedded-WebP glTF 2.0 writer. Geometry uses WGS84 ECEF.
use super::*;
use flightcraft_world::tiles3d::{self, Asset, BoundingVolume, Content, Refine, Tile, Tileset};
use serde_json::json;

pub(super) fn write_glb(
    out: &Path,
    layout: &ImageryGrid,
    key: TileKey,
    origin: [f64; 2],
    mut dem: Option<&mut super::dem::Dems>,
) -> Result<[f64; 2], String> {
    let [lat0, lon0] = origin;
    let transform = tiles3d::enu_transform(lat0, lon0, 0.).map_err(|e| e.to_string())?;
    let [ox, oy, oz] = tiles3d::ecef(lat0, lon0, 0.).map_err(|e| e.to_string())?;
    let [w, s, e, n] = layout.tile_bounds(key).map_err(|e| e.to_string())?;
    let grid = 32_u32;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uv = Vec::new();
    let mut indices = Vec::new();
    let mut heights = [f64::INFINITY, f64::NEG_INFINITY];
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    for row in 0..=grid {
        for col in 0..=grid {
            let (u, v) = (f64::from(col) / f64::from(grid), f64::from(row) / f64::from(grid));
            let [lat, lon] =
                scenery::inverse_mercator(w + u * (e - w), n - v * (n - s)).map_err(|e| e.to_string())?;
            let height = if let Some(dem) = dem.as_deref_mut() {
                dem.height(lat, lon)?
            } else {
                112.
            };
            let [min_height, max_height] = &mut heights;
            *min_height = min_height.min(height);
            *max_height = max_height.max(height);
            // Preview: existing MSL elevations treated as ellipsoidal. Release acceptance requires
            // a reviewed vertical-datum transform; extras explicitly records this unresolved gate.
            let [x, y, z] = tiles3d::ecef(lat, lon, height).map_err(|e| e.to_string())?;
            let (dx, dy, dz) = (x - ox, y - oy, z - oz);
            let dot = |offset: usize| -> Result<f32, String> {
                Ok((dx * transform.get(offset).ok_or("ENU x")?
                    + dy * transform.get(offset + 1).ok_or("ENU y")?
                    + dz * transform.get(offset + 2).ok_or("ENU z")?) as f32)
            };
            let position = [dot(0)?, dot(8)?, -dot(4)?];
            for ((min, max), value) in low.iter_mut().zip(&mut high).zip(position) {
                *min = min.min(value);
                *max = max.max(value);
                positions.extend(value.to_le_bytes());
            }
            for value in [0_f32, 1., 0.] {
                normals.extend(value.to_le_bytes());
            }
            for value in [u as f32, v as f32] {
                uv.extend(value.to_le_bytes());
            }
        }
    }
    for row in 0..grid {
        for col in 0..grid {
            let a = row * (grid + 1) + col;
            let b = a + 1;
            let c = a + grid + 1;
            let d = c + 1;
            for index in [a, c, b, b, c, d] {
                indices.extend((index as u16).to_le_bytes());
            }
        }
    }
    let texture = download::read(
        &out.join(format!("imagery/{}/m0.webp", key.name())),
        8 * 1024 * 1024,
    )?;
    let mut binary = Vec::new();
    let mut views = Vec::new();
    for bytes in [&positions, &normals, &uv, &indices, &texture] {
        while !binary.len().is_multiple_of(4) {
            binary.push(0);
        }
        views.push(json!({"buffer":0,"byteOffset":binary.len(),"byteLength":bytes.len()}));
        binary.extend(bytes);
    }
    let count = (grid + 1) * (grid + 1);
    let document = json!({
        "asset":{"version":"2.0","generator":"FlightCraft original scenery builder"},
        "extras":{"flightcraft":{"terrain":true}},
        "extensionsUsed":["EXT_texture_webp"],"extensionsRequired":["EXT_texture_webp"],
        "buffers":[{"byteLength":binary.len()}],"bufferViews":views,
        "accessors":[
            {"bufferView":0,"componentType":5126,"count":count,"type":"VEC3","min":low,"max":high},
            {"bufferView":1,"componentType":5126,"count":count,"type":"VEC3"},
            {"bufferView":2,"componentType":5126,"count":count,"type":"VEC2"},
            {"bufferView":3,"componentType":5123,"count":grid*grid*6,"type":"SCALAR"}],
        "images":[{"bufferView":4,"mimeType":"image/webp"}],
        "samplers":[{"magFilter":9729,"minFilter":9987,"wrapS":33071,"wrapT":33071}],
        "textures":[{"sampler":0,"extensions":{"EXT_texture_webp":{"source":0}}}],
        "materials":[{"pbrMetallicRoughness":{"baseColorTexture":{"index":0},"metallicFactor":0,"roughnessFactor":1}}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0,"NORMAL":1,"TEXCOORD_0":2},"indices":3,"material":0}]}],
        "nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0
    });
    let mut json = serde_json::to_vec(&document).map_err(|e| e.to_string())?;
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    while !binary.len().is_multiple_of(4) {
        binary.push(0);
    }
    let length = 28 + json.len() + binary.len();
    let mut glb = Vec::with_capacity(length);
    glb.extend(b"glTF");
    glb.extend(2_u32.to_le_bytes());
    glb.extend((length as u32).to_le_bytes());
    glb.extend((json.len() as u32).to_le_bytes());
    glb.extend(b"JSON");
    glb.extend(json);
    glb.extend((binary.len() as u32).to_le_bytes());
    glb.extend(b"BIN\0");
    glb.extend(binary);
    let path = out.join(format!("tiles/{}.glb", key.name()));
    std::fs::create_dir_all(path.parent().ok_or("GLB parent")?).map_err(|e| e.to_string())?;
    std::fs::write(path, glb).map_err(|e| e.to_string())?;
    Ok(heights)
}
pub(super) fn tileset(layout: &ImageryGrid, origin: [f64; 2]) -> Result<Tileset, String> {
    fn node(
        layout: &ImageryGrid,
        key: TileKey,
        heights: &BTreeMap<TileKey, [f64; 2]>,
    ) -> Result<Tile, String> {
        let [w, s, e, n] = layout.tile_bounds(key).map_err(|e| e.to_string())?;
        let [south, west] = scenery::inverse_mercator(w, s).map_err(|e| e.to_string())?;
        let [north, east] = scenery::inverse_mercator(e, n).map_err(|e| e.to_string())?;
        let mut children = Vec::new();
        if key.level < layout.max_level {
            for y in 0..2 {
                for x in 0..2 {
                    children.push(node(
                        layout,
                        TileKey {
                            level: key.level + 1,
                            x: key.x * 2 + x,
                            y: key.y * 2 + y,
                        },
                        heights,
                    )?);
                }
            }
        }
        let [mut lo, mut hi] = heights.get(&key).copied().unwrap_or([112., 112.]);
        for child in &children {
            if let Some([_, _, _, _, low, high]) = child.bounding_volume.region {
                lo = lo.min(low);
                hi = hi.max(high);
            }
        }
        // Initial LOD estimates use the mesh sampling scale. Accepted nonflat terrain
        // still needs measured parent/child residuals; leaves hold terminal content.
        let error = if children.is_empty() { 0. } else { (e - w) / 32. };
        Ok(Tile {
            bounding_volume: BoundingVolume::region([
                west.to_radians(),
                south.to_radians(),
                east.to_radians(),
                north.to_radians(),
                lo,
                hi,
            ]),
            geometric_error: error,
            refine: Some(Refine::Replace),
            transform: None,
            content: Some(Content {
                uri: format!("tiles/{}.glb", key.name()),
            }),
            children,
        })
    }
    let mut root = node(
        layout,
        TileKey { level: 0, x: 0, y: 0 },
        &layout
            .tiles
            .iter()
            .map(|tile| (tile.key, tile.height_bounds))
            .collect(),
    )?;
    root.transform = Some(tiles3d::enu_transform(origin[0], origin[1], 0.).map_err(|e| e.to_string())?);
    let set = Tileset {
        asset: Asset {
            version: "1.1".into(),
            copyright: Some(layout.attribution.clone()),
        },
        geometric_error: root.geometric_error,
        root,
        extensions_required: Vec::new(),
        extras: Some(
            json!({"flightcraft":{"previewOnly":layout.preview_only,"verticalDatumVerified":false,"runwayAlignmentVerified":false,"lidarBuilt":false,"sourceResolutionVerified":false,"region":layout.region,"area":layout.area,"groundSampleMeters":layout.ground_sample_m}}),
        ),
    };
    set.validate().map_err(|e| e.to_string())?;
    Ok(set)
}
