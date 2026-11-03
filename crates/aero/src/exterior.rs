//! Bounded, immutable presentation assets. No geometry affects flight dynamics.
use crate::AircraftError;
use crate::aircraft::{error, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub const MAX_MODEL_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_TRIANGLES: usize = 30_000;
pub const MAX_VERTICES: usize = 90_000;
pub const MAX_NODES: usize = 128;
/// The initial exterior profile uses material colors and geometry lettering; textures are rejected.
pub const MAX_TEXTURES: usize = 0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Exterior {
    pub aircraft: String,
    pub model: String,
    pub lod1: String,
    #[schemars(range(min = 0.01, max = 100))]
    pub span_m: f32,
    #[schemars(range(min = 0.01, max = 100))]
    pub length_m: f32,
    #[schemars(range(min = 0.01, max = 100))]
    pub height_m: f32,
    #[schemars(range(min = 0.01, max = 100))]
    pub wing_area_m2: f32,
    #[schemars(range(min = 0.01, max = 100))]
    pub prop_diameter_m: f32,
    #[schemars(length(max = 64))]
    pub moving_parts: Vec<MovingPart>,
    #[schemars(length(max = 16))]
    pub lights: Vec<ExteriorLight>,
    pub registration: Registration,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    pub node: String,
    #[schemars(length(min = 1, max = 16), regex(pattern = "^[A-Z0-9-]+$"))]
    pub text: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Motion {
    Aileron,
    Elevator,
    Rudder,
    Flaps,
    Propeller,
    PropBlur,
    Steering,
    Wheel,
    Oleo,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MovingPart {
    pub node: String,
    pub motion: Motion,
    pub axis: [f32; 3],
    /// Angles at normalized -1 and +1 (flaps use 0 and 1).
    pub travel_rad: [f32; 2],
    #[serde(default)]
    #[schemars(range(max = 15))]
    pub gear_index: usize,
    #[serde(default = "wheel_radius")]
    #[schemars(range(min = 0.01, max = 2))]
    pub wheel_radius_m: f32,
}
fn wheel_radius() -> f32 {
    0.19
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LightKind {
    NavRed,
    NavGreen,
    NavWhite,
    Strobe,
    Beacon,
    Landing,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExteriorLight {
    pub node: String,
    pub kind: LightKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power_load: Option<String>,
}

impl Exterior {
    pub fn paths(&self) -> [&str; 2] {
        [&self.model, &self.lod1]
    }
    pub fn validate(&self) -> Result<(), AircraftError> {
        crate::layers::id(&self.aircraft, "exterior.aircraft")?;
        for path in self.paths() {
            super::validate_asset_path(path)?;
        }
        for value in [
            self.span_m,
            self.length_m,
            self.height_m,
            self.wing_area_m2,
            self.prop_diameter_m,
        ] {
            require(
                value.is_finite() && (0.01..=100.).contains(&value),
                "exterior.dimensions",
                "requires finite dimensions between 0.01 and 100",
            )?;
        }
        require(
            self.moving_parts.len() <= 64 && self.lights.len() <= 16,
            "exterior.nodes",
            "too many bindings",
        )?;
        let mut names = BTreeSet::new();
        for p in &self.moving_parts {
            valid_name(&p.node)?;
            require(names.insert(&p.node), "exterior.node", "duplicate moving node")?;
            let norm: f32 = p.axis.iter().map(|x| x * x).sum();
            require(
                norm.is_finite()
                    && (0.999..=1.001).contains(&norm)
                    && p.travel_rad
                        .iter()
                        .all(|x| x.is_finite() && x.abs() <= std::f32::consts::TAU)
                    && p.gear_index < 16
                    && p.wheel_radius_m.is_finite()
                    && (0.01..=2.).contains(&p.wheel_radius_m),
                "exterior.motion",
                "invalid axis, travel, or wheel",
            )?;
        }
        for l in &self.lights {
            valid_name(&l.node)?;
            if let Some(load) = &l.power_load {
                crate::layers::id(load, "exterior.power_load")?;
            }
            require(names.insert(&l.node), "exterior.node", "duplicate light binding")?;
        }
        valid_name(&self.registration.node)?;
        require(
            !self.registration.text.is_empty()
                && self.registration.text.len() <= 16
                && self
                    .registration
                    .text
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-'),
            "exterior.registration",
            "requires 1 to 16 uppercase registration characters",
        )
    }
}
fn valid_name(s: &str) -> Result<(), AircraftError> {
    require(
        !s.is_empty()
            && s.len() <= 128
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
        "exterior.node",
        "invalid node name",
    )
}
#[derive(Clone, Debug)]
pub struct ModelVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 4],
}
#[derive(Clone, Debug)]
pub struct ModelNode {
    pub name: String,
    pub parent: Option<usize>,
    pub transform: [[f32; 4]; 4],
    pub vertices: Vec<ModelVertex>,
}
#[derive(Clone, Debug)]
pub struct ExteriorModel {
    pub nodes: Vec<ModelNode>,
    pub triangles: usize,
}
#[derive(Clone, Debug)]
pub struct ExteriorAsset {
    pub definition: Exterior,
    pub lod0: Arc<ExteriorModel>,
    pub lod1: Arc<ExteriorModel>,
}

/// Only self-contained GLB 2.0 triangle meshes, POSITION/NORMAL, and material base colors.
/// Check byte ranges before using gltf's readers (which assume valid binary storage).
pub fn load_glb(bytes: &[u8], definition: &Exterior) -> Result<ExteriorModel, AircraftError> {
    definition.validate()?;
    let fail = |s| error("exterior.glb", s);
    require(
        bytes.len() <= MAX_MODEL_BYTES && bytes.starts_with(b"glTF"),
        "exterior.glb",
        "requires a GLB of at most 4 MiB",
    )?;
    let gltf = gltf::Gltf::from_slice(bytes).map_err(|e| fail(e.to_string()))?;
    let blob = gltf
        .blob
        .as_deref()
        .ok_or_else(|| fail("missing binary chunk".into()))?;
    require(
        gltf.buffers().len() == 1
            && gltf
                .buffers()
                .all(|b| matches!(b.source(), gltf::buffer::Source::Bin) && b.length() <= blob.len()),
        "exterior.buffers",
        "requires one embedded binary buffer",
    )?;
    require(
        gltf.nodes().len() <= MAX_NODES
            && gltf.images().len() == 0
            && gltf.textures().len() == MAX_TEXTURES
            && gltf.animations().len() == 0
            && gltf.skins().len() == 0,
        "exterior.resources",
        "node cap exceeded, or unsupported textures, animations, or skins",
    )?;
    for a in gltf.accessors() {
        require(
            a.sparse().is_none() && a.count() <= MAX_VERTICES,
            "exterior.accessor",
            "sparse or oversized accessor",
        )?;
        let view = a.view().ok_or_else(|| fail("accessor has no view".into()))?;
        let stride = view.stride().unwrap_or(a.size());
        let used = a
            .count()
            .saturating_sub(1)
            .saturating_mul(stride)
            .saturating_add(a.size())
            .saturating_add(a.offset());
        require(
            stride >= a.size()
                && used <= view.length()
                && view.offset().saturating_add(view.length()) <= blob.len(),
            "exterior.accessor",
            "accessor exceeds binary bounds",
        )?;
    }
    let mut parents = BTreeMap::new();
    for n in gltf.nodes() {
        for c in n.children() {
            require(
                parents.insert(c.index(), n.index()).is_none(),
                "exterior.hierarchy",
                "multiple parents",
            )?;
        }
    }
    for n in gltf.nodes() {
        let mut seen = BTreeSet::new();
        let mut current = Some(n.index());
        while let Some(i) = current {
            require(seen.insert(i), "exterior.hierarchy", "cyclic nodes")?;
            current = parents.get(&i).copied();
        }
    }
    let mut nodes = Vec::new();
    let mut triangles = 0usize;
    let mut vertices_total = 0usize;
    let mut unique = BTreeSet::new();
    for n in gltf.nodes() {
        let name = n.name().unwrap_or("").to_owned();
        valid_name(&name)?;
        require(
            unique.insert(name.clone()),
            "exterior.node",
            "duplicate node name",
        )?;
        let transform = n.transform().matrix();
        require(
            transform
                .iter()
                .flatten()
                .all(|x| x.is_finite() && x.abs() <= 1000.),
            "exterior.transform",
            "invalid node transform",
        )?;
        let [a, b, c, d] = transform;
        let [a0, a1, a2, a3] = a;
        let [b0, b1, b2, b3] = b;
        let [c0, c1, c2, c3] = c;
        let [_, _, _, d3] = d;
        let determinant = a0 * (b1 * c2 - b2 * c1) - b0 * (a1 * c2 - a2 * c1) + c0 * (a1 * b2 - a2 * b1);
        require(
            determinant.is_finite()
                && determinant.abs() > 1e-12
                && a3 == 0.
                && b3 == 0.
                && c3 == 0.
                && d3 == 1.,
            "exterior.transform",
            "requires a nonsingular affine transform",
        )?;
        let mut vertices = Vec::new();
        if let Some(mesh) = n.mesh() {
            for p in mesh.primitives() {
                require(
                    p.mode() == gltf::mesh::Mode::Triangles && p.morph_targets().len() == 0,
                    "exterior.primitive",
                    "requires triangles without morph targets",
                )?;
                let reader = p.reader(|_| Some(blob));
                let positions: Vec<_> = reader
                    .read_positions()
                    .ok_or_else(|| fail("missing positions".into()))?
                    .collect();
                let normals: Vec<_> = reader
                    .read_normals()
                    .ok_or_else(|| fail("missing normals".into()))?
                    .collect();
                require(
                    positions.len() == normals.len(),
                    "exterior.normals",
                    "normal count differs",
                )?;
                let indices: Vec<u32> = reader
                    .read_indices()
                    .map(|x| x.into_u32().collect())
                    .unwrap_or_else(|| (0..positions.len() as u32).collect());
                triangles = triangles.saturating_add(indices.len() / 3);
                vertices_total = vertices_total.saturating_add(indices.len());
                require(
                    indices.len().is_multiple_of(3)
                        && triangles <= MAX_TRIANGLES
                        && vertices_total <= MAX_VERTICES,
                    "exterior.triangles",
                    "triangle budget exceeded",
                )?;
                let color = p.material().pbr_metallic_roughness().base_color_factor();
                require(
                    color.iter().all(|x| x.is_finite() && (0.0..=1.).contains(x)),
                    "exterior.material",
                    "invalid material color",
                )?;
                for i in indices {
                    let position = *positions
                        .get(i as usize)
                        .ok_or_else(|| fail("index outside positions".into()))?;
                    let normal = *normals
                        .get(i as usize)
                        .ok_or_else(|| fail("index outside normals".into()))?;
                    require(
                        position.iter().all(|x| x.is_finite() && x.abs() <= 100.)
                            && normal.iter().all(|x| x.is_finite() && x.abs() <= 1.001)
                            && normal.iter().map(|x| x * x).sum::<f32>() > 0.5,
                        "exterior.vertex",
                        "invalid vertex or normal",
                    )?;
                    vertices.push(ModelVertex {
                        position,
                        normal,
                        color,
                    });
                }
            }
        }
        nodes.push(ModelNode {
            name,
            parent: parents.get(&n.index()).copied(),
            transform,
            vertices,
        });
    }
    for name in definition
        .moving_parts
        .iter()
        .map(|p| &p.node)
        .chain(definition.lights.iter().map(|p| &p.node))
        .chain(std::iter::once(&definition.registration.node))
    {
        require(
            nodes.iter().any(|n| &n.name == name),
            "exterior.node",
            &format!("missing mesh node {name}"),
        )?;
    }
    require(triangles > 0, "exterior.triangles", "empty model")?;
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    for node in &nodes {
        let mut matrix = node.transform;
        let mut parent = node.parent;
        while let Some(p) = parent {
            let ancestor = nodes.get(p).ok_or_else(|| fail("invalid parent".into()))?;
            matrix = multiply(ancestor.transform, matrix);
            parent = ancestor.parent;
        }
        for vertex in &node.vertices {
            let [x, y, z] = vertex.position;
            for (row, (low, high)) in low.iter_mut().zip(&mut high).enumerate() {
                let value: f32 = matrix
                    .iter()
                    .zip([x, y, z, 1.])
                    .map(|(column, v)| column.get(row).copied().unwrap_or(0.) * v)
                    .sum();
                require(
                    value.is_finite() && value.abs() <= 1000.,
                    "exterior.bounds",
                    "transformed vertex exceeds limits",
                )?;
                *low = low.min(value);
                *high = high.max(value);
            }
        }
    }
    for ((low, high), stated) in
        low.iter()
            .zip(high)
            .zip([definition.length_m, definition.span_m, definition.height_m])
    {
        require(
            ((high - low) / stated - 1.).abs() <= 0.02,
            "exterior.bounds",
            "model dimensions differ by more than 2%",
        )?;
    }
    Ok(ExteriorModel { nodes, triangles })
}

fn multiply(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    std::array::from_fn(|column| {
        std::array::from_fn(|row| {
            a.iter()
                .enumerate()
                .map(|(k, ac)| {
                    ac.get(row).copied().unwrap_or(0.)
                        * b.get(column).and_then(|bc| bc.get(k)).copied().unwrap_or(0.)
                })
                .sum()
        })
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    fn fixture() -> (Exterior, Vec<u8>) {
        (
            serde_json::from_str(include_str!("../../../aircraft/corvid/exterior.json")).unwrap(),
            include_bytes!("../../../aircraft/corvid/exterior.glb").to_vec(),
        )
    }
    fn edit(bytes: &[u8], change: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
        let size = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let mut doc: serde_json::Value = serde_json::from_slice(&bytes[20..20 + size]).unwrap();
        change(&mut doc);
        let mut json = serde_json::to_vec(&doc).unwrap();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let binary = &bytes[20 + size..];
        let mut out = b"glTF".to_vec();
        out.extend(2u32.to_le_bytes());
        out.extend(((20 + json.len() + binary.len()) as u32).to_le_bytes());
        out.extend((json.len() as u32).to_le_bytes());
        out.extend(0x4e4f534au32.to_le_bytes());
        out.extend(json);
        out.extend(binary);
        out
    }
    #[test]
    fn bounded_original_glbs_and_missing_nodes() {
        let (mut d, b) = fixture();
        let m = load_glb(&b, &d).unwrap();
        assert!(m.triangles <= MAX_TRIANGLES);
        d.moving_parts[0].node = "absent".into();
        assert!(load_glb(&b, &d).is_err());
    }
    #[test]
    fn hostile_binary_storage_and_resource_limits() {
        let (d, b) = fixture();
        for size in [0, 1, 12, 20, 100, b.len() - 1] {
            assert!(load_glb(&b[..size], &d).is_err());
        }
        assert!(load_glb(&vec![0; MAX_MODEL_BYTES + 1], &d).is_err());
        for bad in [
            edit(&b, |v| v["accessors"][0]["count"] = (MAX_VERTICES + 1).into()),
            edit(&b, |v| v["accessors"][0]["byteOffset"] = u32::MAX.into()),
            edit(&b, |v| v["bufferViews"][0]["byteLength"] = u32::MAX.into()),
            edit(&b, |v| v["buffers"][0]["uri"] = "file:///secret".into()),
            edit(&b, |v| v["nodes"][0]["children"] = serde_json::json!([0])),
            edit(&b, |v| v["nodes"][1]["name"] = v["nodes"][0]["name"].clone()),
            edit(&b, |v| {
                v["images"] = serde_json::json!([{"uri":"https://example.invalid/art.png"}])
            }),
        ] {
            assert!(load_glb(&bad, &d).is_err());
        }
    }
    #[test]
    fn binding_numbers_paths_and_unknown_fields_are_hostile() {
        let (mut d, _) = fixture();
        d.model = "../plane.glb".into();
        assert!(d.validate().is_err());
        d.model = "plane.glb".into();
        d.moving_parts[0].axis = [0., 0., 0.];
        assert!(d.validate().is_err());
        d.moving_parts[0].axis = [0., 1., 0.];
        d.moving_parts[0].wheel_radius_m = f32::NAN;
        assert!(d.validate().is_err());
        let mut json = serde_json::to_value(fixture().0).unwrap();
        json["script"] = "evil".into();
        assert!(serde_json::from_value::<Exterior>(json).is_err());
    }
}
