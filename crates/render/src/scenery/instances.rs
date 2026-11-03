//! Bounded EXT_mesh_gpu_instancing attributes. No CPU expansion per tree.
use glam::{Mat4, Quat, Vec3};
use serde_json::Value;
fn accessor<'a>(value: &Value, doc: &'a gltf::Document) -> Result<gltf::Accessor<'a>, String> {
    let index = value
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or("Invalid instance accessor")?;
    let a = doc.accessors().nth(index).ok_or("Instance accessor is missing")?;
    if a.data_type() != gltf::accessor::DataType::F32
        || a.normalized()
        || a.sparse().is_some()
        || a.count() > 20_000
    {
        return Err("Instance accessor exceeds supported type/count".into());
    }
    Ok(a)
}
fn vec3(value: &Value, doc: &gltf::Document, bytes: &[u8], count: usize) -> Result<Vec<[f32; 3]>, String> {
    let a = accessor(value, doc)?;
    if a.dimensions() != gltf::accessor::Dimensions::Vec3 || a.count() != count {
        return Err("Instance vector shape/count mismatch".into());
    }
    gltf::accessor::Iter::<[f32; 3]>::new(a, |_| Some(bytes))
        .map(|i| i.collect())
        .ok_or_else(|| "Invalid instance buffer".into())
}
pub(super) fn read(
    value: &Value,
    doc: &gltf::Document,
    bytes: &[u8],
    node: usize,
) -> Result<Option<Vec<Mat4>>, String> {
    let Some(ext) = value
        .get("nodes")
        .and_then(|v| v.get(node))
        .and_then(|v| v.pointer("/extensions/EXT_mesh_gpu_instancing"))
    else {
        return Ok(None);
    };
    let attrs = ext
        .get("attributes")
        .and_then(|v| v.as_object())
        .filter(|v| !v.is_empty())
        .ok_or("Empty instance attributes")?;
    if attrs
        .keys()
        .any(|k| !matches!(k.as_str(), "TRANSLATION" | "ROTATION" | "SCALE") && !k.starts_with('_'))
    {
        return Err("Unsupported instance attribute".into());
    }
    let count = accessor(attrs.values().next().ok_or("Instance attributes missing")?, doc)?.count();
    if count == 0 {
        return Err("Empty instance collection".into());
    }
    for v in attrs.values() {
        if accessor(v, doc)?.count() != count {
            return Err("Instance attribute count mismatch".into());
        }
    }
    let translations = attrs
        .get("TRANSLATION")
        .map(|v| vec3(v, doc, bytes, count))
        .transpose()?
        .unwrap_or_else(|| vec![[0.; 3]; count]);
    let scales = attrs
        .get("SCALE")
        .map(|v| vec3(v, doc, bytes, count))
        .transpose()?
        .unwrap_or_else(|| vec![[1.; 3]; count]);
    let rotations = if let Some(v) = attrs.get("ROTATION") {
        let a = accessor(v, doc)?;
        if a.dimensions() != gltf::accessor::Dimensions::Vec4 {
            return Err("Instance quaternion shape mismatch".into());
        }
        gltf::accessor::Iter::<[f32; 4]>::new(a, |_| Some(bytes))
            .ok_or("Invalid quaternion buffer")?
            .collect()
    } else {
        vec![[0., 0., 0., 1.]; count]
    };
    let mut result = Vec::with_capacity(count);
    for ((t, s), r) in translations.into_iter().zip(scales).zip(rotations) {
        if t.iter().any(|v| !v.is_finite() || v.abs() > 1_000_000.)
            || s.iter().any(|v| !v.is_finite() || !(0.001..=2000.).contains(v))
            || r.iter().any(|v| !v.is_finite())
        {
            return Err("Invalid instance transform".into());
        }
        let rotation = Quat::from_array(r);
        if (rotation.length_squared() - 1.).abs() > 0.001 {
            return Err("Instance quaternion must be normalized".into());
        }
        result.push(Mat4::from_scale_rotation_translation(
            Vec3::from_array(s),
            rotation,
            Vec3::from_array(t),
        ));
    }
    Ok(Some(result))
}
