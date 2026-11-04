//! Bounded, immutable presentation assets. No geometry affects flight dynamics.
use crate::AircraftError;
use crate::aircraft::{error, require};
pub use gltf::material::AlphaMode as MaterialAlpha;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub const MAX_MODEL_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_TRIANGLES: usize = 120_000;
pub const MAX_VERTICES: usize = MAX_TRIANGLES * 3;
pub const MAX_NODES: usize = 128;
pub const MAX_TEXTURES: usize = 64;
pub const MAX_IMAGES: usize = 8;
pub const MAX_MATERIALS: usize = 32;
pub const MAX_TEXTURE_DIMENSION: u32 = 4096;
pub const MAX_DECODED_BYTES: usize = 5 * 4096 * 4096 * 4;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Exterior {
    pub aircraft: String,
    pub model: String,
    pub lod1: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lod2: Option<String>,
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
    /// Body-space patch for replacing registration baked into the base-color atlas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<RegistrationSurface>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RegistrationSurface {
    pub node: String,
    pub x_m: [f32; 2],
    pub z_m: [f32; 2],
    pub height_m: f32,
    pub background: [f32; 3],
    pub color: [f32; 3],
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
    /// Linear translation driven by canopy open fraction (pack format 3).
    Slide,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(transform = moving_part_schema)]
pub struct MovingPart {
    pub node: String,
    pub motion: Motion,
    pub axis: [f32; 3],
    /// Angles at normalized -1 and +1 (flaps use 0 and 1).
    #[serde(default)]
    pub travel_rad: [f32; 2],
    /// Closed-to-open translation along the unit local axis; slide only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.01, max = 3))]
    pub travel_m: Option<f32>,
    #[serde(default)]
    #[schemars(range(max = 15))]
    pub gear_index: usize,
    #[serde(default = "wheel_radius")]
    #[schemars(range(min = 0.01, max = 2))]
    pub wheel_radius_m: f32,
}
fn moving_part_schema(schema: &mut schemars::Schema) {
    schema.insert("oneOf".into(), serde_json::json!([
        {"properties":{"motion":{"const":"slide"},"travel_m":{"type":"number","minimum":0.01,"maximum":3},"travel_rad":{"const":[0,0]}},"required":["travel_m"]},
        {"properties":{"motion":{"not":{"const":"slide"}}},"required":["travel_rad"],"not":{"required":["travel_m"]}}
    ]));
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
    pub fn paths(&self) -> Vec<&str> {
        std::iter::once(self.model.as_str())
            .chain(std::iter::once(self.lod1.as_str()))
            .chain(self.lod2.as_deref())
            .collect()
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
            require(
                match (p.motion, p.travel_m) {
                    (Motion::Slide, Some(m)) => {
                        m.is_finite() && (0.01..=3.).contains(&m) && p.travel_rad == [0., 0.]
                    }
                    (Motion::Slide, None) => false,
                    (_, None) => true,
                    (_, Some(_)) => false,
                },
                "exterior.motion.travel_m",
                "slide requires 0.01 to 3 metres and zero angular travel; other motions forbid travel_m",
            )?;
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
        if let Some(surface) = &self.registration.surface {
            valid_name(&surface.node)?;
            let [left, right] = surface.x_m;
            let [top, bottom] = surface.z_m;
            require(
                [left, right, top, bottom, surface.height_m]
                    .iter()
                    .all(|v| v.is_finite() && v.abs() <= 100.)
                    && (0.01..=3.).contains(&(right - left))
                    && (0.01..=1.).contains(&(bottom - top))
                    && (0.01..=(bottom - top)).contains(&surface.height_m)
                    && surface
                        .background
                        .iter()
                        .chain(&surface.color)
                        .all(|v| v.is_finite() && (0.0..=1.).contains(v)),
                "exterior.registration.surface",
                "invalid registration paint surface",
            )?;
        }
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
    pub uv: [f32; 2],
    pub tangent: [f32; 4],
    pub material: usize,
}
#[derive(Clone, Debug)]
pub struct ModelImage {
    pub size: [u32; 2],
    pub rgba: Arc<[u8]>,
}
#[derive(Clone, Debug)]
pub struct ModelMaterial {
    pub base_color: [f32; 4],
    pub emissive: [f32; 3],
    pub metallic: f32,
    pub roughness: f32,
    pub normal_scale: f32,
    pub occlusion_strength: f32,
    pub alpha_cutoff: f32,
    pub alpha_mode: gltf::material::AlphaMode,
    pub double_sided: bool,
    pub clearcoat: f32,
    pub clearcoat_roughness: f32,
    /// Base color, metallic-roughness, normal, AO, emissive image indices.
    pub images: [Option<usize>; 5],
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
    pub materials: Vec<ModelMaterial>,
    pub images: Vec<ModelImage>,
}
#[derive(Clone, Debug)]
pub struct ExteriorAsset {
    pub definition: Exterior,
    pub lod0: Arc<ExteriorModel>,
    pub lod1: Arc<ExteriorModel>,
    pub lod2: Option<Arc<ExteriorModel>>,
}

/// Only self-contained GLB 2.0 triangle meshes, POSITION/NORMAL, and material base colors.
/// Check byte ranges before using gltf's readers (which assume valid binary storage).
pub fn load_glb(bytes: &[u8], definition: &Exterior) -> Result<ExteriorModel, AircraftError> {
    definition.validate()?;
    let fail = |s| error("exterior.glb", s);
    require(
        bytes.len() <= MAX_MODEL_BYTES && bytes.starts_with(b"glTF"),
        "exterior.glb",
        "requires a GLB of at most 64 MiB",
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
            && gltf.images().len() <= MAX_IMAGES
            && gltf.materials().len() < MAX_MATERIALS
            && gltf.textures().len() <= MAX_TEXTURES
            && gltf.animations().len() == 0
            && gltf.skins().len() == 0,
        "exterior.resources",
        "resource cap exceeded, or unsupported animations or skins",
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
    // Validate every binary view, including images, before invoking any decoder or reader.
    for view in gltf.views() {
        require(
            view.offset()
                .checked_add(view.length())
                .is_some_and(|end| end <= blob.len()),
            "exterior.view",
            "view exceeds binary storage",
        )?;
    }
    let mut decoded_bytes = 0usize;
    let mut encoded_images = Vec::new();
    for image in gltf.images() {
        let gltf::image::Source::View { view, mime_type } = image.source() else {
            return Err(fail("external image URIs are unsupported".into()));
        };
        let data = blob
            .get(view.offset()..view.offset() + view.length())
            .ok_or_else(|| fail("invalid image storage".into()))?;
        require(
            mime_type == "image/png"
                && data.starts_with(b"\x89PNG\r\n\x1a\n")
                && data.get(12..16) == Some(b"IHDR"),
            "exterior.image",
            "requires an embedded PNG",
        )?;
        let header = data
            .get(16..24)
            .ok_or_else(|| fail("truncated PNG header".into()))?;
        let dimension = |range: std::ops::Range<usize>| -> Result<u32, AircraftError> {
            let b: [u8; 4] = header
                .get(range)
                .ok_or_else(|| fail("missing PNG dimensions".into()))?
                .try_into()
                .map_err(|_| fail("invalid PNG dimensions".into()))?;
            Ok(u32::from_be_bytes(b))
        };
        let size = [dimension(0..4)?, dimension(4..8)?];
        require(
            size.iter().all(|n| (1..=MAX_TEXTURE_DIMENSION).contains(n)),
            "exterior.image",
            "PNG dimension limit exceeded",
        )?;
        let [width, height] = size;
        decoded_bytes = decoded_bytes.saturating_add(width as usize * height as usize * 4);
        require(
            decoded_bytes <= MAX_DECODED_BYTES,
            "exterior.image",
            "decoded texture budget exceeded",
        )?;
        encoded_images.push((size, data));
    }
    let mut images = Vec::new();
    for (size, data) in encoded_images {
        let [width, height] = size;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(MAX_TEXTURE_DIMENSION);
        limits.max_image_height = Some(MAX_TEXTURE_DIMENSION);
        limits.max_alloc = Some(u64::from(width) * u64::from(height) * 8 + 1024 * 1024);
        let decoder = image::codecs::png::PngDecoder::with_limits(std::io::Cursor::new(data), limits)
            .map_err(|e| fail(e.to_string()))?;
        let rgba = image::DynamicImage::from_decoder(decoder)
            .map_err(|e| fail(e.to_string()))?
            .into_rgba8();
        require(
            rgba.dimensions() == (width, height),
            "exterior.image",
            "PNG dimensions changed during decoding",
        )?;
        images.push(ModelImage {
            size,
            rgba: rgba.into_raw().into(),
        });
    }
    let mut materials = Vec::new();
    for material in gltf.materials() {
        let p = material.pbr_metallic_roughness();
        for coord in [
            p.base_color_texture().map(|t| t.tex_coord()),
            p.metallic_roughness_texture().map(|t| t.tex_coord()),
            material.normal_texture().map(|t| t.tex_coord()),
            material.occlusion_texture().map(|t| t.tex_coord()),
            material.emissive_texture().map(|t| t.tex_coord()),
        ]
        .into_iter()
        .flatten()
        {
            require(coord == 0, "exterior.texture", "only TEXCOORD_0 is supported")?;
        }
        let extension = material.extension_value("KHR_materials_clearcoat");
        let factor = |name: &str| {
            extension
                .and_then(|v| v.get(name))
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.) as f32
        };
        let strength = material
            .extension_value("KHR_materials_emissive_strength")
            .and_then(|v| v.get("emissiveStrength"))
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(1.) as f32;
        let entry = ModelMaterial {
            base_color: p.base_color_factor(),
            emissive: material.emissive_factor().map(|v| v * strength),
            metallic: p.metallic_factor(),
            roughness: p.roughness_factor(),
            normal_scale: material.normal_texture().map_or(1., |t| t.scale()),
            occlusion_strength: material.occlusion_texture().map_or(1., |t| t.strength()),
            alpha_cutoff: material.alpha_cutoff().unwrap_or(0.5),
            alpha_mode: material.alpha_mode(),
            double_sided: material.double_sided(),
            clearcoat: factor("clearcoatFactor"),
            clearcoat_roughness: factor("clearcoatRoughnessFactor"),
            images: [
                p.base_color_texture().map(|t| t.texture().source().index()),
                p.metallic_roughness_texture()
                    .map(|t| t.texture().source().index()),
                material.normal_texture().map(|t| t.texture().source().index()),
                material.occlusion_texture().map(|t| t.texture().source().index()),
                material.emissive_texture().map(|t| t.texture().source().index()),
            ],
        };
        require(
            entry
                .base_color
                .iter()
                .chain(
                    [
                        entry.metallic,
                        entry.roughness,
                        entry.occlusion_strength,
                        entry.alpha_cutoff,
                        entry.clearcoat,
                        entry.clearcoat_roughness,
                    ]
                    .iter(),
                )
                .all(|x| x.is_finite() && (0.0..=1.).contains(x))
                && entry
                    .emissive
                    .iter()
                    .all(|x| x.is_finite() && (0.0..=100.).contains(x))
                && entry.normal_scale.is_finite()
                && (0.0..=4.).contains(&entry.normal_scale),
            "exterior.material",
            "invalid PBR factors",
        )?;
        materials.push(entry);
    }
    materials.push(ModelMaterial {
        base_color: [1.; 4],
        emissive: [0.; 3],
        metallic: 1.,
        roughness: 1.,
        normal_scale: 1.,
        occlusion_strength: 1.,
        alpha_cutoff: 0.5,
        alpha_mode: gltf::material::AlphaMode::Opaque,
        double_sided: false,
        clearcoat: 0.,
        clearcoat_roughness: 0.,
        images: [None; 5],
    });
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
                let uvs: Vec<_> = reader
                    .read_tex_coords(0)
                    .map(|v| v.into_f32().collect())
                    .unwrap_or_else(|| vec![[0.; 2]; positions.len()]);
                let tangents: Option<Vec<_>> = reader.read_tangents().map(Iterator::collect);
                require(
                    uvs.len() == positions.len()
                        && tangents.as_ref().is_none_or(|v| v.len() == positions.len()),
                    "exterior.vertex",
                    "UV or tangent count differs",
                )?;
                require(
                    p.material().normal_texture().is_none()
                        || (tangents.is_some() && reader.read_tex_coords(0).is_some()),
                    "exterior.normal",
                    "normal maps require UVs and tangents",
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
                    let uv = *uvs
                        .get(i as usize)
                        .ok_or_else(|| fail("index outside UVs".into()))?;
                    let tangent = tangents
                        .as_ref()
                        .and_then(|v| v.get(i as usize))
                        .copied()
                        .unwrap_or_else(|| {
                            let [x, y, z] = normal;
                            let t = if x.abs() < 0.9 { [0., z, -y] } else { [-z, 0., x] };
                            let len = libm::sqrtf(t.iter().map(|v| v * v).sum());
                            let [tx, ty, tz] = t;
                            [tx / len, ty / len, tz / len, 1.]
                        });
                    let [_, _, _, handedness] = tangent;
                    require(
                        uv.iter().all(|v| v.is_finite() && v.abs() <= 10000.)
                            && tangent.iter().all(|v| v.is_finite() && v.abs() <= 1.001)
                            && handedness.abs() > 0.99,
                        "exterior.vertex",
                        "invalid UV or tangent",
                    )?;
                    vertices.push(ModelVertex {
                        position,
                        normal,
                        color,
                        uv,
                        tangent,
                        material: p.material().index().unwrap_or(materials.len().saturating_sub(1)),
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
        .chain(
            definition
                .registration
                .surface
                .as_ref()
                .map(|surface| &surface.node),
        )
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
            ((high - low) / stated - 1.).abs() <= 0.01,
            "exterior.bounds",
            "model dimensions differ by more than 1%",
        )?;
    }
    Ok(ExteriorModel {
        nodes,
        triangles,
        materials,
        images,
    })
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
    fn with_png(bytes: &[u8], png: &[u8], count: usize) -> Vec<u8> {
        let size = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let mut doc: serde_json::Value = serde_json::from_slice(&bytes[20..20 + size]).unwrap();
        let mut blob = bytes[28 + size..].to_vec();
        let offset = blob.len();
        blob.extend(png);
        while !blob.len().is_multiple_of(4) {
            blob.push(0);
        }
        let views = doc["bufferViews"].as_array_mut().unwrap();
        let view = views.len();
        views.push(serde_json::json!({"buffer":0,"byteOffset":offset,"byteLength":png.len()}));
        doc["images"] = serde_json::json!(
            (0..count)
                .map(|_| serde_json::json!({"bufferView":view,"mimeType":"image/png"}))
                .collect::<Vec<_>>()
        );
        doc["textures"] = serde_json::json!([{"source":0}]);
        doc["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"] = serde_json::json!({"index":0});
        doc["buffers"][0]["byteLength"] = blob.len().into();
        let mut json = serde_json::to_vec(&doc).unwrap();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut out = b"glTF".to_vec();
        out.extend(2u32.to_le_bytes());
        out.extend(((28 + json.len() + blob.len()) as u32).to_le_bytes());
        out.extend((json.len() as u32).to_le_bytes());
        out.extend(0x4e4f534au32.to_le_bytes());
        out.extend(json);
        out.extend((blob.len() as u32).to_le_bytes());
        out.extend(0x004e4942u32.to_le_bytes());
        out.extend(blob);
        out
    }
    #[test]
    fn embedded_pngs_decode_and_hostile_headers_fail_before_decompression() {
        use image::ImageEncoder;
        let (d, b) = fixture();
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&[23, 45, 67, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        let valid = with_png(&b, &png, 1);
        let model = load_glb(&valid, &d).unwrap();
        assert_eq!(model.images[0].size, [1, 1]);
        assert_eq!(&*model.images[0].rgba, &[23, 45, 67, 255]);
        let mut oversized = png.clone();
        oversized[16..20].copy_from_slice(&4097u32.to_be_bytes());
        assert!(
            load_glb(&with_png(&b, &oversized, 1), &d)
                .unwrap_err()
                .to_string()
                .contains("dimension limit")
        );
        let mut bomb = png.clone();
        bomb[16..20].copy_from_slice(&4096u32.to_be_bytes());
        bomb[20..24].copy_from_slice(&4096u32.to_be_bytes());
        assert!(
            load_glb(&with_png(&b, &bomb, 6), &d)
                .unwrap_err()
                .to_string()
                .contains("decoded texture budget")
        );
        assert!(load_glb(&with_png(&b, &png, 9), &d).is_err());
        assert!(
            load_glb(
                &edit(&valid, |v| v["materials"][0]["normalTexture"] =
                    serde_json::json!({"index":0})),
                &d
            )
            .is_err()
        );
        assert!(
            load_glb(
                &edit(&valid, |v| v["bufferViews"]
                    .as_array_mut()
                    .unwrap()
                    .last_mut()
                    .unwrap()["byteOffset"] = u32::MAX.into()),
                &d
            )
            .is_err()
        );
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
