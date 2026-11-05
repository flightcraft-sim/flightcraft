//! Compact indexed GLB writer for corridor tiles: positions, UVs and u16/u32 indices only. The
//! renderer derives flat normals per triangle itself, so normals are not stored.
use super::{corridor::AREA, tiles::glb_bytes};
use flightcraft_world::{geoid::GeoidGrid, tiles3d};
use serde_json::{Value, json};

/// Precomputed east/north/up frame at a fixed origin: geodetic -> glTF Y-up local metres.
pub(super) struct Frame {
    t: [f64; 16],
    o: [f64; 3],
    geoid: GeoidGrid,
}
impl Frame {
    pub fn new(origin: [f64; 2], geoid: &GeoidGrid) -> Result<Self, String> {
        Ok(Self {
            t: tiles3d::enu_transform(origin[0], origin[1], 0.).map_err(|e| e.to_string())?,
            o: tiles3d::ecef(origin[0], origin[1], 0.).map_err(|e| e.to_string())?,
            geoid: geoid.clone(),
        })
    }
    /// Orthometric (NAVD88-like MSL) height to the local frame the tileset root transform
    /// expects, also returning the ellipsoidal height used.
    pub fn local_h(&self, lat: f64, lon: f64, msl: f64) -> Result<([f32; 3], f64), String> {
        let h = self
            .geoid
            .ellipsoidal_m(lat, lon, msl)
            .map_err(|e| e.to_string())?;
        Ok((self.local_ellipsoidal(lat, lon, h)?, h))
    }
    pub fn local_ellipsoidal(&self, lat: f64, lon: f64, h: f64) -> Result<[f32; 3], String> {
        let p = tiles3d::ecef(lat, lon, h).map_err(|e| e.to_string())?;
        let d = [p[0] - self.o[0], p[1] - self.o[1], p[2] - self.o[2]];
        let dot = |k: usize| d[0] * self.t[k] + d[1] * self.t[k + 1] + d[2] * self.t[k + 2];
        Ok([dot(0) as f32, dot(8) as f32, -dot(4) as f32])
    }
}

#[derive(Default)]
pub(super) struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub uv: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}
impl Mesh {
    pub fn quad(&mut self, p: [[f32; 3]; 4], uv: [[f32; 2]; 4]) {
        let base = self.positions.len() as u32;
        self.positions.extend(p);
        self.uv.extend(uv);
        self.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

#[derive(Clone, Copy)]
pub(super) enum Texture<'a> {
    Jpeg(&'a [u8]),
    Webp(&'a [u8]),
}

pub(super) fn write_glb(mesh: &Mesh, texture: Texture<'_>, extras: Option<Value>) -> Result<Vec<u8>, String> {
    if mesh.is_empty() || mesh.positions.len() > 200_000 || mesh.positions.len() != mesh.uv.len() {
        return Err("corridor mesh is empty or exceeds the per-tile vertex budget".into());
    }
    let mut binary = Vec::new();
    let mut views = Vec::new();
    let mut view = |bytes: &[u8], binary: &mut Vec<u8>| {
        while !binary.len().is_multiple_of(4) {
            binary.push(0);
        }
        views.push(json!({"buffer":0,"byteOffset":binary.len(),"byteLength":bytes.len()}));
        binary.extend(bytes);
        views.len() - 1
    };
    let (mut low, mut high) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    let mut position_bytes = Vec::with_capacity(mesh.positions.len() * 12);
    for p in &mesh.positions {
        for (k, v) in p.iter().enumerate() {
            if !v.is_finite() || v.abs() > 1_000_000. {
                return Err("non-finite corridor vertex".into());
            }
            low[k] = low[k].min(*v);
            high[k] = high[k].max(*v);
            position_bytes.extend(v.to_le_bytes());
        }
    }
    let uv_bytes: Vec<u8> = mesh.uv.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
    let small = mesh.positions.len() <= 65_535;
    let index_bytes: Vec<u8> = if small {
        mesh.indices
            .iter()
            .flat_map(|i| (*i as u16).to_le_bytes())
            .collect()
    } else {
        mesh.indices.iter().flat_map(|i| i.to_le_bytes()).collect()
    };
    let p = view(&position_bytes, &mut binary);
    let t = view(&uv_bytes, &mut binary);
    let i = view(&index_bytes, &mut binary);
    let (bytes, mime, webp) = match texture {
        Texture::Jpeg(b) => (b, "image/jpeg", false),
        Texture::Webp(b) => (b, "image/webp", true),
    };
    let img = view(bytes, &mut binary);
    let mut doc = json!({
        "asset":{"version":"2.0","generator":"FlightCraft original corridor builder"},
        "buffers":[{"byteLength":binary.len()}],"bufferViews":views,
        "accessors":[
            {"bufferView":p,"componentType":5126,"count":mesh.positions.len(),"type":"VEC3","min":low,"max":high},
            {"bufferView":t,"componentType":5126,"count":mesh.uv.len(),"type":"VEC2"},
            {"bufferView":i,"componentType":if small {5123} else {5125},"count":mesh.indices.len(),"type":"SCALAR"}],
        "images":[{"bufferView":img,"mimeType":mime}],
        "samplers":[{"magFilter":9729,"minFilter":9987,"wrapS":33071,"wrapT":33071}],
        "textures":[{"sampler":0}],
        "materials":[{"doubleSided":true,"pbrMetallicRoughness":{"baseColorTexture":{"index":0},"metallicFactor":0,"roughnessFactor":0.95}}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_0":1},"indices":2,"material":0}]}],
        "nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0
    });
    if webp {
        doc["extensionsUsed"] = json!(["EXT_texture_webp"]);
        doc["extensionsRequired"] = json!(["EXT_texture_webp"]);
        doc["textures"] = json!([{"sampler":0,"extensions":{"EXT_texture_webp":{"source":0}}}]);
    }
    if let Some(extras) = extras {
        doc["extras"] = extras;
    }
    glb_bytes(&doc, &binary)
}

/// Terrain tiles mark themselves so the renderer suppresses fallback ground inside them.
pub(super) fn terrain_extras() -> Value {
    json!({"flightcraft":{"terrain":true,"area":AREA}})
}
