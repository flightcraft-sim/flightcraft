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
    geoid: Option<&flightcraft_world::geoid::GeoidGrid>,
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
            // Standard 3D Tiles/ECEF uses ellipsoidal height h = H + N.
            // With no authentic geoid input this remains an explicitly unaccepted preview.
            let height = if let Some(grid) = geoid {
                grid.ellipsoidal_m(lat, lon, height).map_err(|e| e.to_string())?
            } else {
                height
            };
            let [min_height, max_height] = &mut heights;
            *min_height = min_height.min(height);
            *max_height = max_height.max(height);
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
    let glb = glb_bytes(&document, &binary)?;
    let path = out.join(format!("tiles/{}.glb", key.name()));
    std::fs::create_dir_all(path.parent().ok_or("GLB parent")?).map_err(|e| e.to_string())?;
    std::fs::write(path, glb).map_err(|e| e.to_string())?;
    Ok(heights)
}
pub(super) fn glb_bytes(document: &serde_json::Value, binary: &[u8]) -> Result<Vec<u8>, String> {
    let mut json = serde_json::to_vec(document).map_err(|e| e.to_string())?;
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut binary = binary.to_vec();
    while !binary.len().is_multiple_of(4) {
        binary.push(0);
    }
    let length = 28_usize
        .checked_add(json.len())
        .and_then(|n| n.checked_add(binary.len()))
        .filter(|n| *n <= flightcraft_world::tiles3d::MAX_TILE_CONTENT_BYTES)
        .ok_or("GLB content exceeds tile budget")?;
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
    Ok(glb)
}
pub(super) fn tileset(
    layout: &ImageryGrid,
    origin: [f64; 2],
    geoid: Option<&flightcraft_world::geoid::GeoidGrid>,
) -> Result<Tileset, String> {
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
    let mut set = Tileset {
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
    if let Some(grid) = geoid {
        grid.validate().map_err(|e| e.to_string())?;
        let context = set
            .extras
            .as_mut()
            .and_then(|v| v.get_mut("flightcraft"))
            .and_then(|v| v.as_object_mut())
            .ok_or("missing scenery metadata")?;
        context.insert(
            "geoid".into(),
            serde_json::to_value(grid).map_err(|e| e.to_string())?,
        );
    }
    set.validate().map_err(|e| e.to_string())?;
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn geoid_changes_ecef_geometry_and_region_heights_without_claiming_acceptance() {
        let out = std::env::temp_dir().join(format!("fc-ecef-datum-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let mut layout = manifest(&plan("eastern-pa", "1n7-test").unwrap(), true);
        let key = TileKey { level: 0, x: 0, y: 0 };
        let grid = flightcraft_world::geoid::GeoidGrid {
            model: flightcraft_world::geoid::GeoidModel::NgsGeoid18,
            source_sha256: "a".repeat(64),
            south_deg: 40.95,
            west_deg: -75.02,
            step_deg: [1. / 60.; 2],
            rows: 3,
            cols: 3,
            undulation_m: vec![-31.; 9],
        };
        let origin = [40.96905613156273, -75.00242278301268];
        let mut tile = write_tile(
            &out,
            key,
            image::RgbaImage::from_pixel(512, 512, image::Rgba([80, 120, 60, 255])),
        )
        .unwrap();
        tile.height_bounds = write_glb(&out, &layout, key, origin, None, Some(&grid)).unwrap();
        assert_eq!(tile.height_bounds, [81., 81.]); // H=112, N=-31, h=81.
        layout.tiles.push(tile);
        layout.max_level = 0;
        let set = tileset(&layout, origin, Some(&grid)).unwrap();
        assert_eq!(set.root.bounding_volume.region.unwrap()[4..], [81., 81.]);
        assert_eq!(
            set.extras.as_ref().unwrap()["flightcraft"]["verticalDatumVerified"],
            false
        );
        let bytes = std::fs::read(out.join("tiles/0/0/0.glb")).unwrap();
        let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let binary = &bytes[28 + json_len..];
        let offset = (16 * 33 + 16) * 12;
        let p: Vec<f64> = binary[offset..offset + 12]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f64::from(f32::from_le_bytes(*b)))
            .collect();
        let t = set.root.transform.unwrap();
        let earth = std::array::from_fn(|axis| {
            t[12 + axis] + t[axis] * p[0] - t[4 + axis] * p[2] + t[8 + axis] * p[1]
        });
        let [lat, lon, height] = tiles3d::geodetic(earth).unwrap();
        assert!((height - 81.).abs() < 0.002);
        assert!(
            (set.geoid_grid()
                .unwrap()
                .unwrap()
                .orthometric_m(lat, lon, height)
                .unwrap()
                - 112.)
                .abs()
                < 0.002
        );
        std::fs::remove_dir_all(out).unwrap();
    }
}
