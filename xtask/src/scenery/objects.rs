//! Original glTF meshes and standard EXT_mesh_gpu_instancing in the existing 3D Tiles tree.
use super::trees::{Species, Tree};
use flightcraft_world::{
    geoid::GeoidGrid,
    scenery,
    tiles3d::{self, BoundingVolume, Content, Refine, Tile, Tileset},
};
use serde_json::{Value, json};
use std::path::Path;
#[derive(Default)]
pub(super) struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uv: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}
impl Mesh {
    pub fn triangle(&mut self, p: [[f32; 3]; 3], uv: [[f32; 2]; 3]) -> Result<(), String> {
        let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        let a = sub(p[1], p[0]);
        let b = sub(p[2], p[0]);
        let n = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        let length = libm::sqrtf(n.iter().map(|v| v * v).sum());
        if !length.is_finite() || length < 1e-9 {
            return Ok(());
        }
        for (p, uv) in p.into_iter().zip(uv) {
            self.indices.push(self.positions.len() as u32);
            self.positions.push(p);
            self.normals.push(n.map(|v| v / length));
            self.uv.push(uv);
        }
        Ok(())
    }
    fn lobe(&mut self, center: [f32; 3], radius: [f32; 3], u: f32) -> Result<(), String> {
        // Smooth ellipsoidal lobes with an original cutout leaf-cluster surface.
        let point = |latitude: u32, longitude: u32| {
            let phi = std::f32::consts::PI * latitude as f32 / 5.;
            let theta = std::f32::consts::TAU * longitude as f32 / 10.;
            (
                [
                    center[0] + radius[0] * phi.sin() * theta.cos(),
                    center[1] + radius[1] * phi.cos(),
                    center[2] + radius[2] * phi.sin() * theta.sin(),
                ],
                [
                    u + 0.04 + 0.42 * longitude as f32 / 10.,
                    0.77 + 0.18 * latitude as f32 / 5.,
                ],
            )
        };
        let start = self.positions.len();
        for y in 0..5 {
            for x in 0..10 {
                let (a, ua) = point(y, x);
                let (b, ub) = point(y, x + 1);
                let (c, uc) = point(y + 1, x + 1);
                let (d, ud) = point(y + 1, x);
                self.triangle([a, b, c], [ua, ub, uc])?;
                self.triangle([a, c, d], [ua, uc, ud])?;
            }
        }
        for (p, n) in self.positions.iter().zip(&mut self.normals).skip(start) {
            let gradient = std::array::from_fn::<_, 3, _>(|i| (p[i] - center[i]) / (radius[i] * radius[i]));
            let length = libm::sqrtf(gradient.iter().map(|v| v * v).sum());
            if length > 1e-6 {
                *n = gradient.map(|v| v / length);
            }
        }
        Ok(())
    }
    fn branch(&mut self, a: [f32; 3], b: [f32; 3], ra: f32, rb: f32) -> Result<(), String> {
        let direction = std::array::from_fn::<_, 3, _>(|i| b[i] - a[i]);
        let cross = |a: [f32; 3], b: [f32; 3]| {
            [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]
        };
        let u = cross(
            direction,
            if direction[0].abs() < 0.8 {
                [1., 0., 0.]
            } else {
                [0., 1., 0.]
            },
        );
        let length = libm::sqrtf(u.iter().map(|v| v * v).sum());
        if length < 1e-6 {
            return Err("Degenerate authored branch".into());
        }
        let u = u.map(|v| v / length);
        let v = cross(direction, u);
        let length = libm::sqrtf(v.iter().map(|v| v * v).sum());
        let v = v.map(|v| v / length);
        for k in 0..8 {
            let angle = std::f32::consts::TAU * k as f32 / 8.;
            let next = std::f32::consts::TAU * (k + 1) as f32 / 8.;
            let point = |center: [f32; 3], r: f32, t: f32| {
                std::array::from_fn(|i| center[i] + r * (u[i] * libm::cosf(t) + v[i] * libm::sinf(t)))
            };
            let (p, q, r, t) = (
                point(a, ra, angle),
                point(a, ra, next),
                point(b, rb, next),
                point(b, rb, angle),
            );
            self.triangle([p, q, r], [[0.7, 0.98]; 3])?;
            self.triangle([p, r, t], [[0.7, 0.98]; 3])?;
        }
        Ok(())
    }
}
#[cfg(test)]
fn tree_mesh(species: Species, coarse: bool) -> Result<Mesh, String> {
    tree_variant(species, coarse, 0)
}
fn tree_variant(species: Species, coarse: bool, variant: u32) -> Result<Mesh, String> {
    let mut mesh = Mesh::default();
    if coarse {
        let u = if species == Species::Deciduous { 0. } else { 0.5 };
        let uv = [[u, 0.75], [u + 0.5, 0.75], [u + 0.5, 0.], [u, 0.]];
        for quad in [
            [[-1., 0., 0.], [1., 0., 0.], [1., 1., 0.], [-1., 1., 0.]],
            [[0., 0., -1.], [0., 0., 1.], [0., 1., 1.], [0., 1., -1.]],
        ] {
            mesh.triangle([quad[0], quad[1], quad[2]], [uv[0], uv[1], uv[2]])?;
            mesh.triangle([quad[0], quad[2], quad[3]], [uv[0], uv[2], uv[3]])?;
        }
    } else {
        mesh.branch([0., 0., 0.], [0., 0.68, 0.], 0.055, 0.012)?;
        let u = if species == Species::Deciduous { 0. } else { 0.5 };
        let phase = variant as f32 * 1.71;
        for k in 0..6 {
            let angle = k as f32 * 2.3999631 + phase;
            let (center, radius) = if species == Species::Deciduous {
                let y = 0.59 + 0.17 * (k as f32 * 1.83 + phase).sin();
                let spread = 0.44 + 0.12 * (k as f32 + phase).cos();
                (
                    [spread * angle.cos(), y, spread * angle.sin()],
                    [0.46, 0.22 + 0.035 * (k as f32 + phase).sin(), 0.42],
                )
            } else {
                let y = 0.36 + k as f32 * 0.10;
                let width = (1. - y) * 0.95;
                (
                    [width * 0.16 * angle.cos(), y, width * 0.16 * angle.sin()],
                    [width, 0.13, width],
                )
            };
            if k % 2 == 0 {
                mesh.branch([0., 0.40, 0.], center, 0.022, 0.006)?;
            }
            mesh.lobe(center, radius, u)?;
        }
    }
    let top = mesh.positions.iter().map(|p| p[1]).fold(0_f32, f32::max);
    if top <= 0. {
        return Err("Authored tree has no height".into());
    }
    let radii = mesh
        .positions
        .iter()
        .fold([0_f32; 2], |r, p| [r[0].max(p[0].abs()), r[1].max(p[2].abs())]);
    for p in &mut mesh.positions {
        p[0] /= radii[0].max(0.01);
        p[1] /= top;
        p[2] /= radii[1].max(0.01);
    }

    for n in &mut mesh.normals {
        n[0] *= radii[0];
        n[1] *= top;
        n[2] *= radii[1];
        let length = libm::sqrtf(n.iter().map(|v| v * v).sum());
        *n = n.map(|v| v / length);
    }
    Ok(weld(mesh))
}
/// Share identical vertices: the smooth lobes repeat each corner in about five triangles, so an
/// indexed mesh is a third of the size (and every tree tile carries several copies).
fn weld(mesh: Mesh) -> Mesh {
    let mut seen: std::collections::HashMap<[u32; 8], u32> = std::collections::HashMap::new();
    let mut out = Mesh::default();
    for &index in &mesh.indices {
        let i = index as usize;
        let (Some(p), Some(n), Some(uv)) = (mesh.positions.get(i), mesh.normals.get(i), mesh.uv.get(i))
        else {
            continue;
        };
        let key = [
            p[0].to_bits(),
            p[1].to_bits(),
            p[2].to_bits(),
            n[0].to_bits(),
            n[1].to_bits(),
            n[2].to_bits(),
            uv[0].to_bits(),
            uv[1].to_bits(),
        ];
        let next = out.positions.len() as u32;
        let id = *seen.entry(key).or_insert_with(|| {
            out.positions.push(*p);
            out.normals.push(*n);
            out.uv.push(*uv);
            next
        });
        out.indices.push(id);
    }
    out
}
fn atlas() -> Result<Vec<u8>, String> {
    use image::ImageEncoder;
    let noise = |x: u32, y: u32| {
        let mut n = x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663);
        n ^= n >> 13;
        n = n.wrapping_mul(1274126177);
        (n % 1000) as f32 / 1000.
    };
    let mut image = image::RgbaImage::new(512, 512);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let conifer = x >= 256;
        let xx = (x % 256) as f32 / 127.5 - 1.;
        let mut trunk = false;
        let leaf = if y < 384 {
            let yy = 1. - y as f32 / 384.;
            trunk = xx.abs() < 0.036 && (0.0..0.72).contains(&yy);
            let edge = (noise(x / 4, y / 4) - 0.5) * 0.15;
            let crown = if conifer {
                [(0.25, 0.72, 1.), (0.45, 0.88, 0.8), (0.65, 1., 0.55)]
                    .into_iter()
                    .any(|(lo, hi, r)| yy >= lo && yy < hi && xx.abs() < r * (hi - yy) / (hi - lo) + edge)
            } else {
                [
                    (-0.48, 0.56, 0.43, 0.23),
                    (-0.20, 0.78, 0.44, 0.21),
                    (0.26, 0.75, 0.49, 0.24),
                    (0.55, 0.49, 0.33, 0.20),
                    (0.04, 0.48, 0.55, 0.22),
                    (-0.62, 0.43, 0.24, 0.17),
                ]
                .into_iter()
                .any(|(cx, cy, rx, ry)| ((xx - cx) / rx).powi(2) + ((yy - cy) / ry).powi(2) < 1. + edge)
            };
            crown && noise(x / 2, y / 2) > 0.06
        } else if y < 498 {
            let yy = 1. - (y - 384) as f32 / 57.;
            let mut leaf = false;
            for k in 0..28 {
                let a = k as f32 * 2.3999631;
                let r = 0.87 * libm::sqrtf((k as f32 + 0.5) / 28.);
                let dx = xx - r * libm::cosf(a);
                let dy = yy - r * libm::sinf(a);
                leaf |= (dx * dx / 0.045 + dy * dy / 0.022) < 1.;
            }
            leaf
        } else {
            true
        };
        let color = if trunk || y >= 498 {
            [91, 67, 43]
        } else if conifer {
            [45, 77, 50]
        } else {
            [69, 97, 45]
        };
        let shade = 0.50 + noise(x / 8, y / 8) * 0.45 + noise(x / 2, y / 2) * 0.35;
        *pixel = image::Rgba([
            (color[0] as f32 * shade) as u8,
            (color[1] as f32 * shade) as u8,
            (color[2] as f32 * shade) as u8,
            if trunk || leaf { 255 } else { 0 },
        ]);
    }
    let mut bytes = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut bytes)
        .write_image(image.as_raw(), 512, 512, image::ExtendedColorType::Rgba8)
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}
pub(super) fn local(
    lat: f64,
    lon: f64,
    msl: f64,
    origin: [f64; 2],
    geoid: &GeoidGrid,
) -> Result<[f32; 3], String> {
    let h = geoid.ellipsoidal_m(lat, lon, msl).map_err(|e| e.to_string())?;
    let t = tiles3d::enu_transform(origin[0], origin[1], 0.).map_err(|e| e.to_string())?;
    let p = tiles3d::ecef(lat, lon, h).map_err(|e| e.to_string())?;
    let o = tiles3d::ecef(origin[0], origin[1], 0.).map_err(|e| e.to_string())?;
    let delta = std::array::from_fn::<_, 3, _>(|i| p[i] - o[i]);
    let dot = |offset: usize| -> Result<f32, String> {
        Ok(delta
            .iter()
            .enumerate()
            .map(|(i, v)| t.get(offset + i).map(|b| v * b).ok_or("Local frame axis"))
            .collect::<Result<Vec<_>, _>>()?
            .iter()
            .sum::<f64>() as f32)
    };
    Ok([dot(0)?, dot(8)?, -dot(4)?])
}
#[derive(Default)]
pub(super) struct Document {
    pub binary: Vec<u8>,
    pub views: Vec<Value>,
    pub accessors: Vec<Value>,
}
impl Document {
    pub fn view(&mut self, bytes: &[u8]) -> usize {
        while !self.binary.len().is_multiple_of(4) {
            self.binary.push(0);
        }
        let index = self.views.len();
        self.views
            .push(json!({"buffer":0,"byteOffset":self.binary.len(),"byteLength":bytes.len()}));
        self.binary.extend(bytes);
        index
    }
    pub fn floats<const N: usize>(&mut self, values: &[[f32; N]], kind: &str) -> Result<usize, String> {
        if values.is_empty()
            || values.len() > 200_000
            || values
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || v.abs() > 1_000_000.)
        {
            return Err("Object accessor exceeds bounds".into());
        }
        let bytes: Vec<u8> = values.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
        let view = self.view(&bytes);
        let low: Vec<f32> = (0..N)
            .map(|axis| {
                values
                    .iter()
                    .filter_map(|v| v.get(axis))
                    .copied()
                    .fold(f32::INFINITY, f32::min)
            })
            .collect();
        let high: Vec<f32> = (0..N)
            .map(|axis| {
                values
                    .iter()
                    .filter_map(|v| v.get(axis))
                    .copied()
                    .fold(f32::NEG_INFINITY, f32::max)
            })
            .collect();
        let index = self.accessors.len();
        self.accessors.push(json!({"bufferView":view,"componentType":5126,"count":values.len(),"type":kind,"min":low,"max":high}));
        Ok(index)
    }
    pub fn mesh(&mut self, mesh: &Mesh) -> Result<Value, String> {
        let p = self.floats(&mesh.positions, "VEC3")?;
        let n = self.floats(&mesh.normals, "VEC3")?;
        let uv = self.floats(&mesh.uv, "VEC2")?;
        let view = self.view(
            &mesh
                .indices
                .iter()
                .flat_map(|i| i.to_le_bytes())
                .collect::<Vec<_>>(),
        );
        let index = self.accessors.len();
        self.accessors
            .push(json!({"bufferView":view,"componentType":5125,"count":mesh.indices.len(),"type":"SCALAR"}));
        Ok(
            json!({"primitives":[{"attributes":{"POSITION":p,"NORMAL":n,"TEXCOORD_0":uv},"indices":index,"material":0}]}),
        )
    }
    pub fn finish(
        self,
        meshes: Vec<Value>,
        nodes: Vec<Value>,
        texture: &[u8],
        mask: bool,
        instanced: bool,
    ) -> Result<Vec<u8>, String> {
        self.finish_with_extras(meshes, nodes, texture, mask, instanced, None)
    }
    /// As `finish`, with glTF root `extras` (e.g. `{"flightcraft":{"water":true}}`).
    pub fn finish_with_extras(
        mut self,
        meshes: Vec<Value>,
        nodes: Vec<Value>,
        texture: &[u8],
        mask: bool,
        instanced: bool,
        extras: Option<Value>,
    ) -> Result<Vec<u8>, String> {
        let image = self.view(texture);
        let node_indices: Vec<usize> = (0..nodes.len()).collect();
        let extensions = if instanced {
            vec!["EXT_texture_webp", "EXT_mesh_gpu_instancing"]
        } else {
            vec!["EXT_texture_webp"]
        };
        let mut doc = json!({"asset":{"version":"2.0","generator":"FlightCraft original reconstruction"},
            "extensionsUsed":extensions,"extensionsRequired":extensions,
            "buffers":[{"byteLength":self.binary.len()}],"bufferViews":self.views,"accessors":self.accessors,
            "images":[{"bufferView":image,"mimeType":"image/webp"}],"samplers":[{"magFilter":9729,"minFilter":9987,"wrapS":33071,"wrapT":33071}],
            "textures":[{"sampler":0,"extensions":{"EXT_texture_webp":{"source":0}}}],
            "materials":[{"alphaMode":if mask {"MASK"}else{"OPAQUE"},"alphaCutoff":0.5,"doubleSided":true,"pbrMetallicRoughness":{"baseColorTexture":{"index":0},"metallicFactor":0,"roughnessFactor":0.95}}],
            "meshes":meshes,"nodes":nodes,"scenes":[{"nodes":node_indices}],"scene":0});
        if let Some(extras) = extras {
            doc["extras"] = extras;
        }
        super::tiles::glb_bytes(&doc, &self.binary)
    }
}
pub(super) const ORIGIN: [f64; 2] = [40.96905613156273, -75.00242278301268];
static AREA_ORIGIN: std::sync::Mutex<Option<[f64; 2]>> = std::sync::Mutex::new(None);
/// The tileset's local-frame origin: 1N7's unless an area (the Hudson corridor) sets its own.
pub(super) fn origin() -> [f64; 2] {
    AREA_ORIGIN.lock().ok().and_then(|o| *o).unwrap_or(ORIGIN)
}
pub(super) fn set_origin(origin: [f64; 2]) {
    if let Ok(mut o) = AREA_ORIGIN.lock() {
        *o = Some(origin);
    }
}
fn tree_content(trees: &[&Tree], coarse: bool, geoid: &GeoidGrid, variants: u32) -> Result<Vec<u8>, String> {
    let mut doc = Document::default();
    let mut meshes = Vec::new();
    let mut nodes = Vec::new();
    for (species, variant) in [Species::Deciduous, Species::Conifer]
        .into_iter()
        .flat_map(|species| (0..variants).map(move |variant| (species, variant)))
    {
        let selected: Vec<_> = trees
            .iter()
            .filter(|t| {
                let seed =
                    (t.lat.to_bits() ^ t.lon.to_bits().rotate_left(17)).wrapping_mul(0x9e3779b97f4a7c15);
                t.species == species && (seed >> 32) as u32 % variants == variant
            })
            .collect();
        if selected.is_empty() {
            continue;
        }
        if selected.len() > 20_000 {
            return Err("Tree instances exceed per-content budget; subdivide the block".into());
        }
        let translations = selected
            .iter()
            .map(|t| local(t.lat, t.lon, t.ground_msl_m, origin(), geoid))
            .collect::<Result<Vec<_>, _>>()?;
        let scales: Vec<[f32; 3]> = selected
            .iter()
            .map(|t| {
                [
                    t.crown_radii_m[0] as f32,
                    t.height_m as f32,
                    t.crown_radii_m[1] as f32,
                ]
            })
            .collect();
        let rotations: Vec<[f32; 4]> = selected
            .iter()
            .map(|t| {
                [
                    0.,
                    libm::sin(t.crown_azimuth_rad / 2.) as f32,
                    0.,
                    libm::cos(t.crown_azimuth_rad / 2.) as f32,
                ]
            })
            .collect();
        let rotation = doc.floats(&rotations, "VEC4")?;
        let translate = doc.floats(&translations, "VEC3")?;
        let scale = doc.floats(&scales, "VEC3")?;
        let mesh = doc.mesh(&tree_variant(species, coarse, variant)?)?;
        let id = meshes.len();
        meshes.push(mesh);
        nodes.push(json!({"mesh":id,"extensions":{"EXT_mesh_gpu_instancing":{"attributes":{"TRANSLATION":translate,"SCALE":scale,"ROTATION":rotation}}}}));
    }
    doc.finish(meshes, nodes, &atlas()?, true, true)
}
pub(super) fn write_trees(
    out: &Path,
    set: &mut Tileset,
    trees: &[Tree],
    geoid: &GeoidGrid,
) -> Result<(), String> {
    write_trees_split(
        out,
        set,
        trees,
        geoid,
        TreeSplit {
            max_level: 2,
            leaf: 64,
            parent_cap: usize::MAX,
            variants: 4,
        },
    )
}
/// Quadtree splitting for tree tiles: subdivide while a tile holds more than `leaf` trees.
#[derive(Clone, Copy)]
pub(super) struct TreeSplit {
    pub max_level: u8,
    pub leaf: usize,
    /// A coarse (replaced) tile shows at most this many of its trees, evenly thinned.
    pub parent_cap: usize,
    /// Crown variants per species in each tile (each is a mesh copy in every GLB).
    pub variants: u32,
}
pub(super) fn write_trees_split(
    out: &Path,
    set: &mut Tileset,
    trees: &[Tree],
    geoid: &GeoidGrid,
    split: TreeSplit,
) -> Result<(), String> {
    if trees.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(out.join("trees")).map_err(|e| e.to_string())?;
    fn node(
        out: &Path,
        trees: &[&Tree],
        geoid: &GeoidGrid,
        split: TreeSplit,
        level: u8,
        x: u32,
        y: u32,
    ) -> Result<Tile, String> {
        let mut bounds = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ];
        let mut coordinates = Vec::new();
        let mut radius = 0_f64;
        for t in trees {
            let [px, py] = scenery::mercator(t.lat, t.lon).map_err(|e| e.to_string())?;
            coordinates.push([px, py]);
            let r = t.crown_radii_m[0].max(t.crown_radii_m[1]);
            radius = radius.max(r);
            let h = geoid
                .ellipsoidal_m(t.lat, t.lon, t.ground_msl_m)
                .map_err(|e| e.to_string())?;
            bounds[0] = bounds[0].min(t.lon - r / (111_000. * libm::cos(t.lat.to_radians())));
            bounds[1] = bounds[1].min(t.lat - r / 111_000.);
            bounds[2] = bounds[2].max(t.lon + r / (111_000. * libm::cos(t.lat.to_radians())));
            bounds[3] = bounds[3].max(t.lat + r / 111_000.);
            bounds[4] = bounds[4].min(h - 0.5);
            bounds[5] = bounds[5].max(h + t.height_m + 0.5);
        }
        let mut children = Vec::new();
        if level < split.max_level && trees.len() > split.leaf {
            let mx = coordinates.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min)
                + coordinates.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max);
            let my = coordinates.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min)
                + coordinates.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max);
            for cy in 0..2 {
                for cx in 0..2 {
                    let group: Vec<_> = trees
                        .iter()
                        .zip(&coordinates)
                        .filter(|(_, p)| u32::from(p[0] >= mx / 2.) == cx && u32::from(p[1] >= my / 2.) == cy)
                        .map(|(t, _)| *t)
                        .collect();
                    if !group.is_empty() {
                        children.push(node(
                            out,
                            &group,
                            geoid,
                            split,
                            level + 1,
                            x * 2 + cx,
                            y * 2 + cy,
                        )?);
                    }
                }
            }
        }
        let coarse = !children.is_empty();
        let uri = format!("trees/{level}-{x}-{y}.glb");
        let shown: Vec<&Tree> = if coarse && trees.len() > split.parent_cap {
            trees
                .iter()
                .step_by(trees.len().div_ceil(split.parent_cap))
                .copied()
                .collect()
        } else {
            trees.to_vec()
        };
        std::fs::write(
            out.join(&uri),
            tree_content(&shown, coarse, geoid, split.variants)?,
        )
        .map_err(|e| e.to_string())?;
        let [w, s, e, n, lo, hi] = bounds;
        Ok(Tile {
            bounding_volume: BoundingVolume::region([
                w.to_radians(),
                s.to_radians(),
                e.to_radians(),
                n.to_radians(),
                lo,
                hi,
            ]),
            geometric_error: if coarse { radius } else { 0. },
            refine: Some(Refine::Replace),
            transform: None,
            content: Some(Content { uri }),
            children,
        })
    }
    let tree = node(out, &trees.iter().collect::<Vec<_>>(), geoid, split, 0, 0, 0)?;
    include_bounds(&mut set.root, &tree)?;
    set.root.children.push(tree);
    let flags = set
        .extras
        .as_mut()
        .and_then(|v| v.get_mut("flightcraft"))
        .and_then(|v| v.as_object_mut())
        .ok_or("Tree metadata")?;
    flags.insert("treesBuilt".into(), json!(true));
    flags.insert("treeCount".into(), json!(trees.len()));
    Ok(())
}
pub(super) fn include_bounds(parent: &mut Tile, child: &Tile) -> Result<(), String> {
    let bounds = parent
        .bounding_volume
        .region
        .as_mut()
        .ok_or("Object parent needs geographic bounds")?;
    let c = child
        .bounding_volume
        .region
        .ok_or("Object child needs geographic bounds")?;
    bounds[0] = bounds[0].min(c[0]);
    bounds[1] = bounds[1].min(c[1]);
    bounds[2] = bounds[2].max(c[2]);
    bounds[3] = bounds[3].max(c[3]);
    bounds[4] = bounds[4].min(c[4]);
    bounds[5] = bounds[5].max(c[5]);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_meshes_have_finite_normals_and_coarse_silhouettes_cost_less() {
        for species in [Species::Deciduous, Species::Conifer] {
            let near = tree_mesh(species, false).unwrap();
            let far = tree_mesh(species, true).unwrap();
            assert!(near.indices.len() > far.indices.len() * 5);
            assert_eq!(far.indices.len(), 12);
            assert!(near.normals.iter().flatten().all(|v| v.is_finite()));
            assert!(near.positions.iter().all(|p| p[1] >= 0. && p[1] <= 1.001));
            assert!((near.positions.iter().map(|p| p[1]).fold(0_f32, f32::max) - 1.).abs() < 1e-6);
        }
    }
}
