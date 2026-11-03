use super::Vertex;
use flightcraft_world::{geo::LocalProjection, tiles3d};
use glam::{DMat4, DVec3, Mat4, Vec3};
use std::sync::Arc;

/// Fully validated CPU payload. Decode off the drawing thread, then submit it to a Scene.
#[derive(Clone)]
pub struct DecodedTile {
    pub(crate) vertices: Vec<Vertex>,
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
/// Initial local content profile: embedded triangle geometry with one opaque base-color
/// texture/material per GLB. External resources, compression, skins, and animations fail closed.
pub fn decode_glb(
    bytes: &[u8],
    tile_transform: [f64; 16],
    origin: [f64; 2],
) -> Result<Arc<DecodedTile>, String> {
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
        if extensions.iter().any(|v| v.as_str() != Some("EXT_texture_webp")) {
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
    if material.alpha_mode() != gltf::material::AlphaMode::Opaque {
        return Err("initial scenery profile requires opaque materials".into());
    }
    let pbr = material.pbr_metallic_roughness();
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
    let image = reader.decode().map_err(|e| e.to_string())?.to_rgba8();
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
    while let Some((node, parent, depth)) = stack.pop() {
        if depth > 32 || !seen.insert(node.index()) {
            return Err("cyclic or shared glTF node tree".into());
        }
        let transform = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        if !transform.is_finite() || transform.determinant().abs() < 1e-10 {
            return Err("invalid glTF node transform".into());
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
                            color: pbr.base_color_factor(),
                        });
                    }
                }
            }
        }
        for child in node.children() {
            stack.push((child, transform, depth + 1));
        }
    }
    if vertices.is_empty() {
        return Err("empty scenery content".into());
    }
    let terrain_bounds = if value
        .pointer("/extras/flightcraft/terrain")
        .and_then(|v| v.as_bool())
        == Some(true)
    {
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
    let byte_size =
        vertices.len() * std::mem::size_of::<Vertex>() + mips.iter().map(|m| m.as_raw().len()).sum::<usize>();
    Ok(Arc::new(DecodedTile {
        vertices,
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
