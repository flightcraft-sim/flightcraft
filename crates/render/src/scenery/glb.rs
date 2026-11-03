use super::{Instance, InstanceBatch, Vertex};
use flightcraft_world::{geo::LocalProjection, tiles3d};
use glam::{DMat4, DVec3, Mat4, Vec3};
use std::sync::Arc;

/// Fully validated CPU payload. Decode off the drawing thread, then submit it to a Scene.
#[derive(Clone)]
pub struct DecodedTile {
    pub(crate) vertices: Vec<Vertex>,
    pub(crate) instanced: Vec<InstanceBatch>,
    pub(crate) mips: Vec<image::RgbaImage>,
    pub(crate) byte_size: usize,
    pub(crate) expires_at_unix_ms: Option<u64>,
    pub(crate) attribution: Vec<String>,
    pub(crate) terrain_bounds: Option<[f32; 4]>,
}
impl DecodedTile {
    pub(crate) fn expired(&self) -> bool {
        self.expires_at_unix_ms
            .is_some_and(|expiry| super::unix_ms() >= expiry)
    }
    pub fn instance_count(&self) -> usize {
        self.instanced.iter().map(|b| b.instances.len()).sum()
    }
    pub fn byte_size(&self) -> usize {
        self.byte_size
    }
}
fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let b: [u8; 4] = bytes
        .get(offset..offset.saturating_add(4))
        .ok_or("truncated GLB")?
        .try_into()
        .map_err(|_| "GLB integer")?;
    Ok(u32::from_le_bytes(b))
}
/// Initial local content profile: embedded triangle geometry with one opaque or alpha-mask base-color
/// texture/material per GLB. External resources, compression, skins, and animations fail closed.
pub fn decode_glb(
    bytes: &[u8],
    tile_transform: [f64; 16],
    origin: [f64; 2],
) -> Result<Arc<DecodedTile>, String> {
    decode_glb_with_geoid(bytes, tile_transform, origin, None)
}
/// Decode standard ellipsoidal 3D Tiles content into the simulation's MSL frame.
/// Adapters should use ContentRequest::decode to carry the stream's regional geoid context.
pub fn decode_glb_with_geoid(
    bytes: &[u8],
    tile_transform: [f64; 16],
    origin: [f64; 2],
    geoid: Option<&flightcraft_world::geoid::GeoidGrid>,
) -> Result<Arc<DecodedTile>, String> {
    if let Some(grid) = geoid {
        grid.validate().map_err(|e| e.to_string())?;
    }
    if bytes.len() > tiles3d::MAX_TILE_CONTENT_BYTES
        || bytes.get(..4) != Some(b"glTF")
        || u32_at(bytes, 4)? != 2
        || u32_at(bytes, 8)? as usize != bytes.len()
        || bytes.get(16..20) != Some(b"JSON")
    {
        return Err("requires a bounded glTF 2.0 GLB".into());
    }
    if tile_transform.iter().any(|v| !v.is_finite()) {
        return Err("invalid 3D Tiles transform".into());
    }
    let json_len = u32_at(bytes, 12)? as usize;
    let end = 20_usize.checked_add(json_len).ok_or("GLB JSON overflow")?;
    if json_len > 2 * 1024 * 1024
        || !json_len.is_multiple_of(4)
        || bytes.get(end + 4..end + 8) != Some(b"BIN\0")
    {
        return Err("invalid GLB chunks".into());
    }
    let bin_len = u32_at(bytes, end)? as usize;
    let binary = bytes.get(end + 8..).ok_or("missing GLB binary")?;
    if binary.len() != bin_len || !bin_len.is_multiple_of(4) {
        return Err("GLB binary length mismatch".into());
    }
    let mut value: serde_json::Value =
        serde_json::from_slice(bytes.get(20..end).ok_or("GLB JSON bounds")?).map_err(|e| e.to_string())?;
    // gltf-rs does not yet understand EXT_texture_webp. Validate this extension ourselves,
    // normalize its source to core glTF for structural validation, then use its bounded readers.
    if let Some(extensions) = value.get("extensionsRequired") {
        let extensions = extensions.as_array().ok_or("invalid glTF extensions")?;
        if extensions
            .iter()
            .any(|v| !matches!(v.as_str(), Some("EXT_texture_webp" | "EXT_mesh_gpu_instancing")))
        {
            return Err("unsupported required glTF extension".into());
        }
    }
    if let Some(textures) = value.get_mut("textures").and_then(|v| v.as_array_mut()) {
        for texture in textures {
            let source = texture.pointer("/extensions/EXT_texture_webp/source").cloned();
            if let Some(source) = source {
                texture
                    .as_object_mut()
                    .ok_or("texture object")?
                    .insert("source".into(), source);
            }
        }
    }
    value
        .as_object_mut()
        .ok_or("glTF root")?
        .remove("extensionsRequired");
    let gltf = gltf::Gltf::from_slice(&serde_json::to_vec(&value).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if gltf.buffers().len() != 1
        || gltf
            .buffers()
            .any(|b| !matches!(b.source(), gltf::buffer::Source::Bin) || b.length() > binary.len())
        || gltf.nodes().len() > 4096
        || gltf.animations().len() != 0
        || gltf.skins().len() != 0
        || gltf.materials().len() != 1
        || gltf.images().len() != 1
        || gltf.textures().len() != 1
    {
        return Err("unsupported scenery glTF resources or budgets".into());
    }
    for view in gltf.views() {
        if view
            .offset()
            .checked_add(view.length())
            .is_none_or(|n| n > binary.len())
        {
            return Err("glTF view outside binary buffer".into());
        }
    }
    for a in gltf.accessors() {
        let view = a.view().ok_or("scenery accessor missing view")?;
        let stride = view.stride().unwrap_or(a.size());
        let used = a
            .count()
            .saturating_sub(1)
            .checked_mul(stride)
            .and_then(|n| n.checked_add(a.size()))
            .and_then(|n| n.checked_add(a.offset()))
            .ok_or("accessor overflow")?;
        if a.sparse().is_some()
            || a.count() == 0
            || a.count() > 1_000_000
            || stride < a.size()
            || used > view.length()
        {
            return Err("scenery accessor outside bounds".into());
        }
    }
    let material = gltf.materials().next().ok_or("missing material")?;
    if material.alpha_mode() == gltf::material::AlphaMode::Blend
        || material.alpha_cutoff().is_some_and(|v| (v - 0.5).abs() > 1e-6)
    {
        return Err("Scenery supports opaque or half-cutoff alpha-mask materials".into());
    }
    let pbr = material.pbr_metallic_roughness();
    let mut base_color = pbr.base_color_factor();
    if material.alpha_mode() == gltf::material::AlphaMode::Opaque {
        base_color[3] = 1.;
    }
    let texture = pbr.base_color_texture().ok_or("missing base-color texture")?;
    if texture.tex_coord() != 0 {
        return Err("only TEXCOORD_0 supported".into());
    }
    let image = texture.texture().source();
    let image_bytes = match image.source() {
        gltf::image::Source::View { view, .. } => binary
            .get(view.offset()..view.offset() + view.length())
            .ok_or("image buffer bounds")?,
        _ => return Err("scenery requires embedded images".into()),
    };
    let mut reader = image::ImageReader::new(std::io::Cursor::new(image_bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let mut image = reader.decode().map_err(|e| e.to_string())?.to_rgba8();
    if material.alpha_mode() == gltf::material::AlphaMode::Opaque {
        for pixel in image.pixels_mut() {
            pixel.0[3] = 255;
        }
    }
    let mut mips = vec![image];
    loop {
        let last = mips.last().ok_or("missing mip")?;
        if last.width() == 1 && last.height() == 1 {
            break;
        }
        let next = image::imageops::resize(
            last,
            (last.width() / 2).max(1),
            (last.height() / 2).max(1),
            image::imageops::FilterType::Triangle,
        );
        mips.push(next);
    }
    let [lat0, lon0] = origin;
    let projection = LocalProjection::new(lat0, lon0).map_err(|e| e.to_string())?;
    let ecef = DMat4::from_cols_array(&tile_transform);
    // 3D Tiles content converts glTF Y-up to the tile's Z-up before the tile transform.
    let yup = DMat4::from_rotation_x(std::f64::consts::FRAC_PI_2);
    let mut stack = Vec::new();
    let scene = gltf
        .default_scene()
        .or_else(|| gltf.scenes().next())
        .ok_or("missing glTF scene")?;
    for node in scene.nodes() {
        stack.push((node, Mat4::IDENTITY, 0_u32));
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut vertices = Vec::new();
    let mut instanced = Vec::new();
    let mut instance_total = 0_usize;
    let mut instance_geometry = 0_usize;
    let mut instance_draw_vertices = 0_usize;
    let to_sim = |p: Vec3| -> Result<Vec3, String> {
        let earth = (ecef * yup).transform_point3(DVec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z)));
        let [lat, lon, h] = tiles3d::geodetic(earth.to_array()).map_err(|e| e.to_string())?;
        let [north, east] = projection.project(lat, lon).map_err(|e| e.to_string())?;
        let height = geoid.map_or(Ok(h), |g| g.orthometric_m(lat, lon, h).map_err(|e| e.to_string()))?;
        Ok(Vec3::new(east as f32, height as f32, -north as f32))
    };
    while let Some((node, parent, depth)) = stack.pop() {
        if depth > 32 || !seen.insert(node.index()) {
            return Err("cyclic or shared glTF node tree".into());
        }
        let transform = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        if !transform.is_finite() || transform.determinant().abs() < 1e-10 {
            return Err("invalid glTF node transform".into());
        }
        let matrices = super::instances::read(&value, &gltf.document, binary, node.index())?;
        if matrices.is_some() && node.mesh().is_none() {
            return Err("Instanced node has no mesh".into());
        }
        if let Some(mesh) = node.mesh() {
            for primitive in mesh.primitives() {
                if primitive.mode() != gltf::mesh::Mode::Triangles
                    || primitive.morph_targets().len() != 0
                    || primitive.material().index() != Some(0)
                {
                    return Err("unsupported scenery primitive".into());
                }
                let reader = primitive.reader(|_| Some(binary));
                let positions: Vec<_> = reader
                    .read_positions()
                    .ok_or("missing float positions")?
                    .collect();
                let tex: Vec<_> = reader
                    .read_tex_coords(0)
                    .ok_or("missing texture coordinates")?
                    .into_f32()
                    .collect();
                if positions.len() != tex.len() || positions.len() > 200_000 {
                    return Err("invalid scenery vertex counts".into());
                }
                let indices: Vec<u32> = reader
                    .read_indices()
                    .map(|v| v.into_u32().collect())
                    .unwrap_or_else(|| (0..positions.len() as u32).collect());
                if !indices.len().is_multiple_of(3)
                    || vertices.len().saturating_add(indices.len()) > 1_000_000
                {
                    return Err("scenery triangle budget exceeded".into());
                }
                if let Some(matrices) = &matrices {
                    instance_total = instance_total
                        .checked_add(matrices.len())
                        .filter(|n| *n <= 20_000)
                        .ok_or("Tile instance budget exceeded")?;
                    instance_geometry = instance_geometry.saturating_add(indices.len());
                    instance_draw_vertices =
                        instance_draw_vertices.saturating_add(matrices.len().saturating_mul(indices.len()));
                    if instance_draw_vertices > 4_000_000 || instance_geometry > 1_000_000 {
                        return Err("Instanced scenery draw budget exceeded".into());
                    }
                    let normals: Option<Vec<_>> = reader.read_normals().map(|n| n.collect());
                    if normals.as_ref().is_some_and(|n| n.len() != positions.len()) {
                        return Err("Instanced normal count mismatch".into());
                    }
                    let mut geometry = Vec::with_capacity(indices.len());
                    for triangle in indices.as_chunks::<3>().0.iter() {
                        let mut corners = Vec::with_capacity(3);
                        for &index in triangle {
                            let p = *positions
                                .get(index as usize)
                                .ok_or("Instanced vertex index outside buffer")?;
                            let uv = *tex
                                .get(index as usize)
                                .ok_or("Instanced UV index outside buffer")?;
                            if p.iter()
                                .chain(uv.iter())
                                .any(|v| !v.is_finite() || v.abs() > 1_000_000.)
                            {
                                return Err("Nonfinite instanced vertex".into());
                            }
                            corners.push((Vec3::from_array(p), uv, index));
                        }
                        let [(a, _, _), (b, _, _), (c, _, _)] = corners.as_slice() else {
                            return Err("Instance triangle shape".into());
                        };
                        let face = (*b - *a).cross(*c - *a).normalize_or_zero();
                        for (p, uv, index) in corners {
                            let normal = if let Some(normals) = &normals {
                                Vec3::from_array(*normals.get(index as usize).ok_or("Instance normal index")?)
                            } else {
                                face
                            };
                            if !normal.is_finite() || normal.length_squared() < 0.01 {
                                return Err("Invalid instance normal".into());
                            }
                            geometry.push(Vertex {
                                position: p.to_array(),
                                normal: normal.normalize().to_array(),
                                uv,
                                color: base_color,
                            });
                        }
                    }
                    let mut instances = Vec::with_capacity(matrices.len());
                    for matrix in matrices {
                        let matrix = transform * matrix;
                        let p = to_sim(matrix.transform_point3(Vec3::ZERO))?;
                        let x = to_sim(matrix.transform_point3(Vec3::X))? - p;
                        let y = to_sim(matrix.transform_point3(Vec3::Y))? - p;
                        let z = to_sim(matrix.transform_point3(Vec3::Z))? - p;
                        let frame = Mat4::from_cols(x.extend(0.), y.extend(0.), z.extend(0.), p.extend(1.));
                        if !frame.is_finite() || frame.determinant().abs() < 1e-9 {
                            return Err("Degenerate georeferenced instance".into());
                        }
                        instances.push(Instance {
                            matrix: frame.to_cols_array_2d(),
                        });
                    }
                    instanced.push(InstanceBatch {
                        vertices: geometry,
                        instances,
                    });
                    continue;
                }
                for triangle in indices.as_chunks::<3>().0.iter() {
                    let mut corners = Vec::with_capacity(3);
                    for index in triangle {
                        let p = positions
                            .get(*index as usize)
                            .ok_or("scenery vertex index out of bounds")?;
                        let uv = *tex.get(*index as usize).ok_or("scenery UV index out of bounds")?;
                        if p.iter()
                            .chain(uv.iter())
                            .any(|v| !v.is_finite() || v.abs() > 1_000_000.)
                        {
                            return Err("non-finite scenery vertex".into());
                        }
                        let p = transform.transform_point3(Vec3::from_array(*p));
                        let earth = (ecef * yup).transform_point3(DVec3::new(
                            f64::from(p.x),
                            f64::from(p.y),
                            f64::from(p.z),
                        ));
                        let [lat, lon, height] =
                            tiles3d::geodetic(earth.to_array()).map_err(|e| e.to_string())?;
                        let [n, e] = projection.project(lat, lon).map_err(|e| e.to_string())?;
                        let height = if let Some(grid) = geoid {
                            grid.orthometric_m(lat, lon, height).map_err(|e| e.to_string())?
                        } else {
                            height
                        };
                        corners.push((Vec3::new(e as f32, height as f32, -n as f32), uv));
                    }
                    let [(a, ua), (b, ub), (c, uc)] = corners.as_slice() else {
                        return Err("triangle shape".into());
                    };
                    let normal = (*b - *a).cross(*c - *a).normalize_or_zero().to_array();
                    for (p, uv) in [(*a, *ua), (*b, *ub), (*c, *uc)] {
                        vertices.push(Vertex {
                            position: p.to_array(),
                            normal,
                            uv,
                            color: base_color,
                        });
                    }
                }
            }
        }
        for child in node.children() {
            stack.push((child, transform, depth + 1));
        }
    }
    if vertices.is_empty() && instanced.is_empty() {
        return Err("empty scenery content".into());
    }
    let terrain_bounds = if value
        .pointer("/extras/flightcraft/terrain")
        .and_then(|v| v.as_bool())
        == Some(true)
    {
        if vertices.is_empty() {
            return Err("Instanced-only content cannot claim terrain coverage".into());
        }
        let mut bounds = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
        for vertex in &vertices {
            let [x, _, z] = vertex.position;
            let [minx, minz, maxx, maxz] = &mut bounds;
            *minx = minx.min(x);
            *minz = minz.min(z);
            *maxx = maxx.max(x);
            *maxz = maxz.max(z);
        }
        Some(bounds)
    } else {
        None
    };
    let byte_size = vertices.len() * std::mem::size_of::<Vertex>()
        + instanced
            .iter()
            .map(|b| {
                b.vertices.len() * std::mem::size_of::<Vertex>()
                    + b.instances.len() * std::mem::size_of::<Instance>()
            })
            .sum::<usize>()
        + mips.iter().map(|m| m.as_raw().len()).sum::<usize>();
    Ok(Arc::new(DecodedTile {
        vertices,
        instanced,
        mips,
        byte_size,
        terrain_bounds,
        expires_at_unix_ms: None,
        attribution: Vec::new(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;
    fn fixture() -> (serde_json::Value, Vec<u8>) {
        use serde_json::json;
        let mut binary = Vec::new();
        for value in [
            0_f32, 112., 0., 1., 112., 0., 0., 112., -1., 0., 0., 1., 0., 0., 1.,
        ] {
            binary.extend(value.to_le_bytes());
        }
        for index in [0_u16, 1, 2] {
            binary.extend(index.to_le_bytes());
        }
        while !binary.len().is_multiple_of(4) {
            binary.push(0);
        }
        let offset = binary.len();
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&[60, 90, 40, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        let length = png.len();
        binary.extend(png);
        let value = json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":binary.len()}],"bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":24},{"buffer":0,"byteOffset":60,"byteLength":6},{"buffer":0,"byteOffset":offset,"byteLength":length}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,112,-1],"max":[1,112,0]},{"bufferView":1,"componentType":5126,"count":3,"type":"VEC2"},{"bufferView":2,"componentType":5123,"count":3,"type":"SCALAR"}],"images":[{"bufferView":3,"mimeType":"image/png"}],"textures":[{"source":0}],"materials":[{"pbrMetallicRoughness":{"baseColorTexture":{"index":0}}}],"meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_0":1},"indices":2,"material":0}]}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0});
        (value, binary)
    }
    fn pack(value: &serde_json::Value, binary: &[u8]) -> Vec<u8> {
        let mut json = serde_json::to_vec(value).unwrap();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut binary = binary.to_vec();
        while !binary.len().is_multiple_of(4) {
            binary.push(0);
        }
        let mut bytes = Vec::new();
        bytes.extend(b"glTF");
        bytes.extend(2_u32.to_le_bytes());
        bytes.extend((28 + json.len() as u32 + binary.len() as u32).to_le_bytes());
        bytes.extend((json.len() as u32).to_le_bytes());
        bytes.extend(b"JSON");
        bytes.extend(json);
        bytes.extend((binary.len() as u32).to_le_bytes());
        bytes.extend(b"BIN\0");
        bytes.extend(binary);
        bytes
    }
    #[test]
    fn embedded_glb_rejects_external_resources_bad_ranges_indices_and_nonfinite_vertices() {
        let (value, binary) = fixture();
        let transform = tiles3d::enu_transform(41., -75., 0.).unwrap();
        let decode = |value: &serde_json::Value, binary: &[u8]| {
            decode_glb(&pack(value, binary), transform, [41., -75.])
        };
        let valid = decode(&value, &binary).unwrap();
        assert_eq!(valid.vertices.len(), 3);
        assert_eq!(valid.mips.len(), 1);
        for (pointer, bad) in [
            ("/buffers/0/uri", serde_json::json!("file:///secret")),
            ("/accessors/0/count", serde_json::json!(4294967295_u32)),
            ("/accessors/0/byteOffset", serde_json::json!(4294967295_u32)),
            ("/nodes/0/children", serde_json::json!([0])),
            ("/nodes/0/mesh", serde_json::json!(999)),
        ] {
            let mut edited = value.clone();
            let (parent, key) = pointer.rsplit_once('/').unwrap();
            edited
                .pointer_mut(parent)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert(key.into(), bad);
            assert!(decode(&edited, &binary).is_err(), "must reject {pointer}");
        }
        let mut bad = binary.clone();
        bad.get_mut(60..62)
            .unwrap()
            .copy_from_slice(&u16::MAX.to_le_bytes());
        assert!(decode(&value, &bad).is_err());
        let mut bad = binary;
        bad.get_mut(..4).unwrap().copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(decode(&value, &bad).is_err());
    }
    #[test]
    fn ellipsoidal_vertices_convert_to_msl_with_independent_height_and_sign() {
        let (value, binary) = fixture();
        let grid = flightcraft_world::geoid::GeoidGrid {
            model: flightcraft_world::geoid::GeoidModel::NgsGeoid18,
            source_sha256: "a".repeat(64),
            south_deg: 40.99,
            west_deg: -75.01,
            step_deg: [1. / 60.; 2],
            rows: 3,
            cols: 3,
            undulation_m: vec![-31.; 9],
        };
        let transform = tiles3d::enu_transform(41., -75., 0.).unwrap();
        let bytes = pack(&value, &binary);
        let raw = decode_glb(&bytes, transform, [41., -75.]).unwrap();
        let msl = decode_glb_with_geoid(&bytes, transform, [41., -75.], Some(&grid)).unwrap();
        for (original, converted) in raw.vertices.iter().zip(&msl.vertices) {
            // The fixture has h=112 m, independent of the grid: H=h-(-31)=143 m.
            assert!((converted.position[1] - 143.).abs() < 0.002);
            assert!((converted.position[1] - original.position[1] - 31.).abs() < 0.002);
            assert_eq!(original.position[0], converted.position[0]);
            assert_eq!(original.position[2], converted.position[2]);
        }
        let mut outside = grid.clone();
        outside.south_deg = 39.;
        assert!(decode_glb_with_geoid(&bytes, transform, [41., -75.], Some(&outside)).is_err());
        let mut corrupt = grid;
        corrupt.undulation_m[0] = f32::NAN;
        assert!(decode_glb_with_geoid(&bytes, transform, [41., -75.], Some(&corrupt)).is_err());
    }
    #[test]
    fn opaque_materials_ignore_alpha_and_cutouts_preserve_it() {
        let (mut value, mut binary) = fixture();
        let offset = value["bufferViews"][3]["byteOffset"].as_u64().unwrap() as usize;
        binary.truncate(offset);
        let mut image = Vec::new();
        image::codecs::png::PngEncoder::new(&mut image)
            .write_image(&[60, 90, 40, 0], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        value["bufferViews"][3]["byteLength"] = image.len().into();
        binary.extend(image);
        value["buffers"][0]["byteLength"] = binary.len().into();
        value["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"] = serde_json::json!([1, 1, 1, 0]);
        let frame = tiles3d::enu_transform(41., -75., 0.).unwrap();
        let opaque = decode_glb(&pack(&value, &binary), frame, [41., -75.]).unwrap();
        assert_eq!(opaque.mips[0].get_pixel(0, 0).0[3], 255);
        assert_eq!(opaque.vertices[0].color[3], 1.);
        value["materials"][0]["alphaMode"] = serde_json::json!("MASK");
        let masked = decode_glb(&pack(&value, &binary), frame, [41., -75.]).unwrap();
        assert_eq!(masked.mips[0].get_pixel(0, 0).0[3], 0);
        assert_eq!(masked.vertices[0].color[3], 0.);
    }
    #[test]
    fn gpu_instances_share_geometry_and_reject_hostile_transforms_and_draw_counts() {
        let decode = |v: &serde_json::Value, b: &[u8]| {
            decode_glb(
                &pack(v, b),
                tiles3d::enu_transform(41., -75., 0.).unwrap(),
                [41., -75.],
            )
        };
        let (mut value, mut binary) = fixture();
        for offset in [4, 16, 28] {
            binary[offset..offset + 4].copy_from_slice(&0_f32.to_le_bytes());
        }
        value["accessors"][0]["min"] = serde_json::json!([0, 0, -1]);
        value["accessors"][0]["max"] = serde_json::json!([1, 0, 0]);
        while !binary.len().is_multiple_of(4) {
            binary.push(0);
        }
        let offset = binary.len();
        for v in [10_f32, 112., 20., 20., 112., 30.] {
            binary.extend(v.to_le_bytes());
        }
        value["bufferViews"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"buffer":0,"byteOffset":offset,"byteLength":24}));
        value["accessors"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"bufferView":4,"componentType":5126,"count":2,"type":"VEC3"}));
        value["buffers"][0]["byteLength"] = binary.len().into();
        value["nodes"][0]["extensions"] =
            serde_json::json!({"EXT_mesh_gpu_instancing":{"attributes":{"TRANSLATION":3}}});
        value["extensionsRequired"] = serde_json::json!(["EXT_mesh_gpu_instancing"]);
        value["materials"][0]["alphaMode"] = serde_json::json!("MASK");
        let tile = decode(&value, &binary).unwrap();
        assert!(tile.vertices.is_empty());
        assert_eq!(tile.instanced.len(), 1);
        let batch = &tile.instanced[0];
        assert_eq!(batch.vertices.len(), 3);
        assert_eq!(batch.instances.len(), 2);
        let first = Mat4::from_cols_array_2d(&batch.instances[0].matrix).transform_point3(Vec3::ZERO);
        assert!((first.y - 112.).abs() < 0.01);
        assert!((first.x - 10.).abs() < 0.01);
        for edit in [
            serde_json::json!({"TRANSLATION":999}),
            serde_json::json!({"ROTATION":3}),
        ] {
            let mut bad = value.clone();
            bad["nodes"][0]["extensions"]["EXT_mesh_gpu_instancing"]["attributes"] = edit;
            assert!(decode(&bad, &binary).is_err());
        }
        let mut scaled = value.clone();
        scaled["nodes"][0]["extensions"]["EXT_mesh_gpu_instancing"]["attributes"] =
            serde_json::json!({"SCALE":3});
        assert!(decode(&scaled, &binary).is_ok());
        let mut negative = binary.clone();
        negative[offset..offset + 4].copy_from_slice(&(-1_f32).to_le_bytes());
        assert!(decode(&scaled, &negative).is_err());
        let mut bad = value.clone();
        bad["accessors"][3]["count"] = 20_001.into();
        assert!(decode(&bad, &binary).is_err());
        let mut bad = binary.clone();
        bad[offset..offset + 4].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(decode(&value, &bad).is_err());
    }
    #[test]
    fn truncated_and_oversized_glb_is_rejected() {
        for n in 0..40 {
            assert!(decode_glb(&vec![0; n], DMat4::IDENTITY.to_cols_array(), [40., -75.]).is_err());
        }
        assert!(
            decode_glb(
                &vec![0; tiles3d::MAX_TILE_CONTENT_BYTES + 1],
                DMat4::IDENTITY.to_cols_array(),
                [40., -75.]
            )
            .is_err()
        );
    }
}
