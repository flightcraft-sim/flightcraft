//! Original mesh/GLB writer shared by online imagery and offline city conversion.
use flightcraft_world::{World, geo::LocalProjection, scenery, tiles3d};
use glam::{DMat4, DVec3};
use serde_json::{Value, json};

/// Package standard glTF JSON and its embedded binary buffer, with bounded chunks.
pub fn pack_glb(document: &Value, binary: &[u8]) -> Result<Vec<u8>, String> {
    let mut json = serde_json::to_vec(document).map_err(|_| "Cannot encode scenery JSON")?;
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut binary = binary.to_vec();
    while !binary.len().is_multiple_of(4) {
        binary.push(0);
    }
    let size = 28_usize
        .checked_add(json.len())
        .and_then(|n| n.checked_add(binary.len()))
        .filter(|n| *n <= tiles3d::MAX_TILE_CONTENT_BYTES)
        .ok_or("Scenery GLB exceeds size limit")?;
    let mut bytes = Vec::with_capacity(size);
    bytes.extend(b"glTF");
    bytes.extend(2_u32.to_le_bytes());
    bytes.extend((size as u32).to_le_bytes());
    bytes.extend((json.len() as u32).to_le_bytes());
    bytes.extend(b"JSON");
    bytes.extend(json);
    bytes.extend((binary.len() as u32).to_le_bytes());
    bytes.extend(b"BIN\0");
    bytes.extend(binary);
    Ok(bytes)
}

/// A terrain-draped image, encoded as the same embedded-image GLB as local scenery.
/// Bounds are EPSG:3857 meters. Terrain queries are read-only; physics is unchanged.
pub fn surface_glb(world: &World, bounds: [f64; 4], image: &[u8]) -> Result<Vec<u8>, String> {
    let region = world
        .region()
        .ok_or("Streaming imagery requires an installed region")?;
    let origin = [region.origin.latitude_deg, region.origin.longitude_deg];
    let projection = LocalProjection::new(origin[0], origin[1]).map_err(|e| e.to_string())?;
    let inverse =
        DMat4::from_cols_array(&tiles3d::enu_transform(origin[0], origin[1], 0.).map_err(|e| e.to_string())?)
            .inverse();
    let [w, s, e, n] = bounds;
    if bounds.iter().any(|v| !v.is_finite()) || w >= e || s >= n || image.len() > 4 * 1024 * 1024 {
        return Err("Invalid imagery bounds or size".into());
    }
    let grid = 32_u32;
    let mut positions = Vec::new();
    let mut uv = Vec::new();
    let mut indices = Vec::new();
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    for row in 0..=grid {
        for col in 0..=grid {
            let (u, v) = (f64::from(col) / f64::from(grid), f64::from(row) / f64::from(grid));
            let [lat, lon] =
                scenery::inverse_mercator(w + u * (e - w), n - v * (n - s)).map_err(|e| e.to_string())?;
            let [north, east] = projection.project(lat, lon).map_err(|e| e.to_string())?;
            let height = world.ground(north, east).height_msl_m;
            let earth = tiles3d::ecef(lat, lon, height).map_err(|e| e.to_string())?;
            let enu = inverse.transform_point3(DVec3::from_array(earth));
            for ((min, max), value) in
                low.iter_mut()
                    .zip(&mut high)
                    .zip([enu.x as f32, enu.z as f32, -enu.y as f32])
            {
                *min = min.min(value);
                *max = max.max(value);
                positions.extend(value.to_le_bytes());
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
            for i in [a, c, b, b, c, d] {
                indices.extend((i as u16).to_le_bytes());
            }
        }
    }
    let mut binary = Vec::new();
    let mut views = Vec::new();
    for data in [&positions[..], &uv, &indices, image] {
        while !binary.len().is_multiple_of(4) {
            binary.push(0);
        }
        views.push(json!({"buffer":0,"byteOffset":binary.len(),"byteLength":data.len()}));
        binary.extend(data);
    }
    let mime = match image::guess_format(image) {
        Ok(image::ImageFormat::Png) => "image/png",
        Ok(image::ImageFormat::Jpeg) => "image/jpeg",
        _ => return Err("Unsupported imagery encoding".into()),
    };
    let document = json!({"asset":{"version":"2.0","generator":"FlightCraft imagery adapter"},
        "extras":{"flightcraft":{"terrain":true}},"buffers":[{"byteLength":binary.len()}],"bufferViews":views,
        "accessors":[{"bufferView":0,"componentType":5126,"count":(grid+1)*(grid+1),"type":"VEC3","min":low,"max":high},
        {"bufferView":1,"componentType":5126,"count":(grid+1)*(grid+1),"type":"VEC2"},
        {"bufferView":2,"componentType":5123,"count":grid*grid*6,"type":"SCALAR"}],
        "images":[{"bufferView":3,"mimeType":mime}],"textures":[{"source":0}],
        "materials":[{"pbrMetallicRoughness":{"baseColorTexture":{"index":0},"metallicFactor":0,"roughnessFactor":1}}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_0":1},"indices":2,"material":0}]}],
        "nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0});
    pack_glb(&document, &binary)
}
