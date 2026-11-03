//! Reviewed source metadata is not a scenery format. Output is exclusively 3D Tiles/GLB.
use super::{Operation, Report, Request, download, transport};
use flightcraft_world::{
    Region,
    tiles3d::{self, Asset, BoundingVolume, Content, Refine, Tile, Tileset},
};
use glam::{DMat4, DVec3};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::BTreeMap, path::Path};

/// Filled from authoritative TUM documentation on the networked machine. No guessed defaults.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Review {
    pub documentation_url: String,
    pub approved_host: String,
    pub licence: String,
    pub licence_evidence: String,
    pub attribution: String,
    pub redistribution_allowed: bool,
    /// Exact source CRS, axis order, units, and vertical datum from the workflow documentation.
    pub source_crs: String,
    pub models: Vec<Model>,
}
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub url: String,
    /// Source glTF Y-up positions to WGS84 ECEF, in double precision, column-major.
    /// Must include any workflow offset and source vertical-datum correction.
    pub gltf_to_ecef: [f64; 16],
}
fn review(path: &Path) -> Result<Review, String> {
    let bytes=download::read(path, 2*1024*1024).map_err(|_| "NYC LoD2 needs cache/source-review.json from authoritative TUM workflow documentation. This checkout has no verified download host, license, file URLs, or source CRS; see docs/scenery.md.")?;
    let review: Review = serde_json::from_slice(&bytes).map_err(|_| "Invalid NYC source review")?;
    if review.documentation_url != "https://github.com/tum-gis/3d-model-new-york-city"
        || [
            &review.licence,
            &review.licence_evidence,
            &review.attribution,
            &review.source_crs,
        ]
        .iter()
        .any(|s| s.trim().is_empty() || s.len() > 8192)
        || review.models.is_empty()
        || review.models.len() > 4096
        || review.models.iter().any(|m| {
            !transport::exact_host(&m.url, &review.approved_host)
                || !m.url.ends_with(".glb")
                || !affine(m.gltf_to_ecef)
        })
    {
        return Err("NYC review needs an exact HTTPS dataset host, license evidence, attribution, CRS, and bounded georeferenced GLB URLs. Archives and external glTF resources need a reviewed importer first.".into());
    }
    Ok(review)
}
fn affine(t: [f64; 16]) -> bool {
    t.iter().all(|v| v.is_finite() && v.abs() < 100_000_000.)
        && t.get(3) == Some(&0.)
        && t.get(7) == Some(&0.)
        && t.get(11) == Some(&0.)
        && t.get(15) == Some(&1.)
        && DMat4::from_cols_array(&t).determinant().abs() > 1e-12
}
pub(super) fn run(request: &Request) -> Result<(), String> {
    if request.sample.is_some() {
        return Err("NYC LoD2 does not accept reference imagery".into());
    }
    let review = review(&request.cache.join("source-review.json"))?;
    match request.operation {
        Operation::Fetch => {
            let mut remote = transport::Http::for_host(&review.approved_host);
            let mut receipts = Vec::new();
            for model in &review.models {
                let (_, source) = transport::fetch(
                    &mut remote,
                    &model.url,
                    &request.cache.join("objects"),
                    tiles3d::MAX_TILE_CONTENT_BYTES as u64,
                )?;
                receipts.push(source);
                download::write_json(&request.cache.join("city-sources.json"), &receipts)?;
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&receipts).map_err(|e| e.to_string())?
            );
        }
        Operation::Build => {
            if request.out.exists() {
                return Err("City output already exists".into());
            }
            let stage = request.out.with_extension("building");
            if stage.exists() {
                return Err("City build staging directory already exists".into());
            }
            std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
            let result = build(request, &review, &stage);
            match result {
                Ok(report) => {
                    std::fs::rename(&stage, &request.out).map_err(|e| e.to_string())?;
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
                    );
                }
                Err(error) => {
                    let _ = std::fs::remove_dir_all(&stage);
                    return Err(error);
                }
            }
        }
    }
    Ok(())
}
struct Offline;
impl transport::Remote for Offline {
    fn allows(&self, _: &str) -> bool {
        true
    }
    fn open(&mut self, _: &str, _: u64, _: Option<&str>) -> Result<transport::Response, String> {
        Err("City build requires completed, receipt-verified fetch objects; no network is used".into())
    }
}
pub(super) type Triangle = [DVec3; 3];
pub(super) fn triangles(bytes: &[u8], model: &Model, inverse: DMat4) -> Result<Vec<Triangle>, String> {
    if bytes.len() > tiles3d::MAX_TILE_CONTENT_BYTES {
        return Err("City model exceeds input budget".into());
    }
    let gltf = gltf::Gltf::from_slice(bytes).map_err(|_| "Unsupported city GLB")?;
    let binary = gltf.blob.as_deref().ok_or("City model must be an embedded GLB")?;
    if gltf.buffers().len() != 1
        || gltf
            .buffers()
            .any(|b| !matches!(b.source(), gltf::buffer::Source::Bin) || b.length() > binary.len())
        || gltf.nodes().len() > 4096
        || gltf.animations().len() != 0
        || gltf.skins().len() != 0
        || gltf.extensions_required().next().is_some()
    {
        return Err("Unsupported city resources, extensions, or budgets".into());
    }
    for view in gltf.views() {
        if view
            .offset()
            .checked_add(view.length())
            .is_none_or(|end| end > binary.len())
        {
            return Err("City buffer view is outside GLB".into());
        }
    }
    for accessor in gltf.accessors() {
        let view = accessor.view().ok_or("City accessor needs a buffer view")?;
        let stride = view.stride().unwrap_or(accessor.size());
        let size = accessor
            .count()
            .saturating_sub(1)
            .checked_mul(stride)
            .and_then(|n| n.checked_add(accessor.offset()))
            .and_then(|n| n.checked_add(accessor.size()));
        if accessor.sparse().is_some()
            || accessor.count() == 0
            || accessor.count() > 1_000_000
            || stride < accessor.size()
            || size.is_none_or(|n| n > view.length())
        {
            return Err("City accessor exceeds bounds".into());
        }
    }
    let scene = gltf
        .default_scene()
        .or_else(|| gltf.scenes().next())
        .ok_or("City model needs a scene")?;
    let transform = DMat4::from_cols_array(&model.gltf_to_ecef);
    let mut stack: Vec<_> = scene.nodes().map(|node| (node, DMat4::IDENTITY, 0)).collect();
    let mut seen = std::collections::BTreeSet::new();
    let mut result = Vec::new();
    while let Some((node, parent, depth)) = stack.pop() {
        if depth > 32 || !seen.insert(node.index()) {
            return Err("City node tree is cyclic or shared".into());
        }
        let local =
            parent * DMat4::from_cols_array_2d(&node.transform().matrix().map(|row| row.map(f64::from)));
        if !affine(local.to_cols_array()) {
            return Err("Invalid city node transform".into());
        }
        if let Some(mesh) = node.mesh() {
            for primitive in mesh.primitives() {
                if primitive.mode() != gltf::mesh::Mode::Triangles || primitive.morph_targets().len() != 0 {
                    return Err("City models require static triangles".into());
                }
                let reader = primitive.reader(|_| Some(binary));
                let positions: Vec<_> = reader
                    .read_positions()
                    .ok_or("City model needs positions")?
                    .collect();
                let indices: Vec<_> = reader
                    .read_indices()
                    .map(|v| v.into_u32().collect())
                    .unwrap_or_else(|| (0..positions.len() as u32).collect());
                if !indices.len().is_multiple_of(3)
                    || result.len().saturating_add(indices.len() / 3) > 300_000
                {
                    return Err("City triangle budget exceeded; split upstream GLB".into());
                }
                for triangle in indices.chunks_exact(3) {
                    let mut corners = Vec::new();
                    for index in triangle {
                        let p = positions
                            .get(*index as usize)
                            .ok_or("City triangle index exceeds positions")?;
                        if p.iter().any(|v| !v.is_finite() || v.abs() > 10_000_000.) {
                            return Err("Invalid city vertex".into());
                        }
                        let earth = (transform * local).transform_point3({
                            let [x, y, z] = *p;
                            DVec3::new(f64::from(x), f64::from(y), f64::from(z))
                        });
                        let [lat, lon, height] =
                            tiles3d::geodetic(earth.to_array()).map_err(|e| e.to_string())?;
                        if !(40.45..=40.95).contains(&lat)
                            || !(-74.3..=-73.65).contains(&lon)
                            || !(-500.0..=1500.).contains(&height)
                        {
                            return Err("City vertex is outside NYC or has an unreviewed CRS/datum".into());
                        }
                        let enu = inverse.transform_point3(earth);
                        corners.push(DVec3::new(enu.x, enu.z, -enu.y));
                    }
                    let triangle: Triangle = corners.try_into().map_err(|_| "City triangle shape")?;
                    result.push(triangle);
                }
            }
        }
        for child in node.children() {
            stack.push((child, local, depth + 1));
        }
    }
    if result.is_empty() {
        return Err("Empty city model".into());
    }
    Ok(result)
}
pub(super) fn encode(triangles: &[Triangle], quantization: f64) -> Result<Vec<u8>, String> {
    use image::ImageEncoder;
    let mut positions = Vec::new();
    let mut uv = Vec::new();
    let mut indices = Vec::new();
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    let mut count = 0_u32;
    let mut vertices = BTreeMap::new();
    for triangle in triangles {
        let [a, b, c] = triangle.map(|p| {
            if quantization > 0. {
                (p / quantization).round() * quantization
            } else {
                p
            }
        });
        let normal = (b - a).cross(c - a).normalize_or_zero();
        if normal.length_squared() < 0.5 {
            continue;
        }
        // Original two-swatch procedural material: pale masonry walls and gray roof surfaces.
        let tex = if normal.y.abs() > 0.5 {
            [0.75_f32, 0.5]
        } else {
            [0.25, 0.5]
        };
        for point in [a, b, c] {
            let xyz = point.to_array().map(|v| v as f32);
            // Keep roof and wall UVs distinct. Shared positions never smooth the flat normals:
            // the common runtime decoder computes each triangle's own normal.
            let key = (xyz.map(f32::to_bits), normal.y.abs() > 0.5);
            let index = if let Some(index) = vertices.get(&key) {
                *index
            } else {
                for ((min, max), value) in low.iter_mut().zip(&mut high).zip(xyz) {
                    *min = min.min(value);
                    *max = max.max(value);
                    positions.extend(value.to_le_bytes());
                }
                for value in tex {
                    uv.extend(value.to_le_bytes());
                }
                let index = count;
                count = count.checked_add(1).ok_or("City vertex index overflow")?;
                vertices.insert(key, index);
                index
            };
            indices.push(index);
        }
    }
    if count == 0 {
        return Err("City LOD has no surviving triangles".into());
    }
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            &[183, 174, 157, 255, 92, 99, 104, 255],
            2,
            1,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| e.to_string())?;
    let index_count = indices.len();
    let component = if count <= u32::from(u16::MAX) + 1 {
        5123
    } else {
        5125
    };
    let mut index_bytes = Vec::new();
    for index in indices {
        if component == 5123 {
            index_bytes.extend(
                u16::try_from(index)
                    .map_err(|_| "City short index overflow")?
                    .to_le_bytes(),
            );
        } else {
            index_bytes.extend(index.to_le_bytes());
        }
    }
    let mut binary = Vec::new();
    let mut views = Vec::new();
    for data in [&positions, &uv, &index_bytes, &png] {
        while !binary.len().is_multiple_of(4) {
            binary.push(0);
        }
        views.push(json!({"buffer":0,"byteOffset":binary.len(),"byteLength":data.len()}));
        binary.extend(data);
    }
    let document = json!({"asset":{"version":"2.0","generator":"FlightCraft city converter; original procedural materials"},
        "buffers":[{"byteLength":binary.len()}],"bufferViews":views,
        "accessors":[{"bufferView":0,"componentType":5126,"count":count,"type":"VEC3","min":low,"max":high},{"bufferView":1,"componentType":5126,"count":count,"type":"VEC2"},{"bufferView":2,"componentType":component,"count":index_count,"type":"SCALAR"}],
        "images":[{"bufferView":3,"mimeType":"image/png"}],"samplers":[{"magFilter":9728,"minFilter":9728}],"textures":[{"source":0,"sampler":0}],
        "materials":[{"pbrMetallicRoughness":{"baseColorTexture":{"index":0},"metallicFactor":0,"roughnessFactor":0.9}}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_0":1},"indices":2,"material":0}]}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0});
    flightcraft_render::scenery::pack_glb(&document, &binary)
}
fn volume() -> BoundingVolume {
    BoundingVolume::region([
        -74.3_f64.to_radians(),
        40.45_f64.to_radians(),
        -73.65_f64.to_radians(),
        40.95_f64.to_radians(),
        -500.,
        1500.,
    ])
}
pub(super) fn cell_volume(triangles: &[Triangle], transform: [f64; 16]) -> Result<BoundingVolume, String> {
    let matrix = DMat4::from_cols_array(&transform) * DMat4::from_rotation_x(std::f64::consts::FRAC_PI_2);
    let mut region = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ];
    for point in triangles.iter().flatten() {
        let [lat, lon, height] =
            tiles3d::geodetic(matrix.transform_point3(*point).to_array()).map_err(|e| e.to_string())?;
        let [w, s, e, n, low, high] = &mut region;
        *w = w.min(lon.to_radians());
        *s = s.min(lat.to_radians());
        *e = e.max(lon.to_radians());
        *n = n.max(lat.to_radians());
        *low = low.min(height);
        *high = high.max(height);
    }
    let [w, s, e, n, low, high] = region;
    Ok(BoundingVolume::region([
        w - 0.000004,
        s - 0.000004,
        e + 0.000004,
        n + 0.000004,
        low - 16.,
        high + 16.,
    ]))
}
pub(super) fn parent(children: Vec<Tile>) -> Tile {
    let mut bounds = None;
    for child in &children {
        if let Some([a, b, c, d, lo, hi]) = child.bounding_volume.region {
            let [w, s, e, n, low, high] = bounds.get_or_insert([a, b, c, d, lo, hi]);
            *w = w.min(a);
            *s = s.min(b);
            *e = e.max(c);
            *n = n.max(d);
            *low = low.min(lo);
            *high = high.max(hi);
        }
    }
    Tile {
        bounding_volume: bounds.map(BoundingVolume::region).unwrap_or_else(volume),
        geometric_error: 128.,
        refine: Some(Refine::Add),
        transform: None,
        content: None,
        children,
    }
}
fn build(request: &Request, review: &Review, out: &Path) -> Result<Report, String> {
    let region = Region::from_json(include_str!("../../../assets/terrain/new-york/region.json"))
        .map_err(|e| e.to_string())?;
    let transform = tiles3d::enu_transform(region.origin.latitude_deg, region.origin.longitude_deg, 0.)
        .map_err(|e| e.to_string())?;
    let inverse = DMat4::from_cols_array(&transform).inverse();
    let mut nodes = Vec::new();
    let mut receipts = Vec::new();
    std::fs::create_dir_all(out.join("tiles")).map_err(|e| e.to_string())?;
    for (source_id, model) in review.models.iter().enumerate() {
        let (path, source) = transport::fetch(
            &mut Offline,
            &model.url,
            &request.cache.join("objects"),
            tiles3d::MAX_TILE_CONTENT_BYTES as u64,
        )?;
        receipts.push(source);
        let triangles = triangles(
            &download::read(&path, tiles3d::MAX_TILE_CONTENT_BYTES)?,
            model,
            inverse,
        )?;
        let mut cells: BTreeMap<(i32, i32), Vec<Triangle>> = BTreeMap::new();
        for triangle in triangles {
            let center = triangle.iter().copied().sum::<DVec3>() / 3.;
            cells
                .entry((
                    (center.x / 1000.).floor() as i32,
                    (center.z / 1000.).floor() as i32,
                ))
                .or_default()
                .push(triangle);
        }
        for ((x, y), triangles) in cells {
            if nodes.len() > 20_000 {
                return Err("City tile budget exceeded".into());
            }
            let bounds = cell_volume(&triangles, transform)?;
            let mut child = None;
            for (level, error) in [(2, 0.), (1, 2.), (0, 8.)] {
                let uri = format!("tiles/{source_id}-{x}-{y}-{level}.glb");
                let bytes = encode(&triangles, error).or_else(|_| encode(&triangles, 0.))?;
                // Validate the same loader used by the runtime, including decoded memory budgets.
                flightcraft_render::scenery::decode_glb(
                    &bytes,
                    transform,
                    [region.origin.latitude_deg, region.origin.longitude_deg],
                )?;
                std::fs::write(out.join(&uri), bytes).map_err(|e| e.to_string())?;
                child = Some(Tile {
                    bounding_volume: bounds.clone(),
                    geometric_error: error * 2.,
                    refine: Some(Refine::Replace),
                    transform: None,
                    content: Some(Content { uri }),
                    children: child.into_iter().collect(),
                });
            }
            if let Some(node) = child {
                nodes.push(node);
            }
        }
    }
    while nodes.len() > 64 {
        nodes = nodes.chunks(64).map(|chunk| parent(chunk.to_vec())).collect();
    }
    let mut root = parent(nodes);
    root.transform = Some(transform);
    let set = Tileset {
        asset: Asset {
            version: "1.1".into(),
            copyright: Some(review.attribution.clone()),
        },
        geometric_error: 128.,
        root,
        extensions_required: Vec::new(),
        extras: Some(
            json!({"flightcraft":{"region":"new-york","area":"nyc-lod2","previewOnly":true,"localOnly":true,"redistributionAllowed":review.redistribution_allowed,"licence":review.licence,"verticalDatumVerified":false,"runwayAlignmentVerified":false,"lidarBuilt":false}}),
        ),
    };
    set.validate().map_err(|e| e.to_string())?;
    let encoded = serde_json::to_vec(&set).map_err(|e| e.to_string())?;
    Tileset::from_json(&encoded).map_err(|e| e.to_string())?;
    download::write_json(&out.join("tileset.json"), &set)?;
    download::write_json(&out.join("city-sources.json"), &receipts)?;
    download::write_json(&out.join("source-review.json"), review)?;
    let report = Report {
        region: request.region.clone(),
        area: request.area.clone(),
        manifest: request.out.join("tileset.json"),
        imagery_tiles: 0,
        source_files: receipts.len(),
        preview_only: true,
        elevation_built: false,
        vertical_datum_verified: false,
        runway_alignment_verified: false,
        lidar_built: false,
        release_ready: false,
    };
    download::write_json(&out.join("build-report.json"), &report)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(Vec<u8>);
    impl transport::Remote for Fixture {
        fn allows(&self, url: &str) -> bool {
            transport::exact_host(url, "dataset.example.org")
        }
        fn open(&mut self, _: &str, _: u64, _: Option<&str>) -> Result<transport::Response, String> {
            Ok(transport::Response {
                status: 200,
                validator: Some("\"original-city-fixture\"".into()),
                range: None,
                reader: Box::new(std::io::Cursor::new(self.0.clone())),
            })
        }
    }
    #[test]
    fn reviewed_glb_keeps_roof_shape_and_builds_registered_local_tiles_with_lods() {
        let root = std::env::temp_dir().join(format!("fc-city-fixture-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let triangles = vec![
            [
                DVec3::new(0., 10., 0.),
                DVec3::new(20., 10., 0.),
                DVec3::new(10., 16., -10.),
            ],
            [
                DVec3::new(0., 10., -20.),
                DVec3::new(10., 16., -10.),
                DVec3::new(20., 10., -20.),
            ],
            [
                DVec3::new(0., 0., 0.),
                DVec3::new(20., 0., 0.),
                DVec3::new(20., 10., 0.),
            ],
            [
                DVec3::new(0., 0., 0.),
                DVec3::new(20., 10., 0.),
                DVec3::new(0., 10., 0.),
            ],
        ];
        let bytes = encode(&triangles, 0.).unwrap();
        let earth = DMat4::from_cols_array(&tiles3d::enu_transform(40.75, -74., 0.).unwrap())
            * DMat4::from_rotation_x(std::f64::consts::FRAC_PI_2);
        let model = Model {
            url: "https://dataset.example.org/original.glb".into(),
            gltf_to_ecef: earth.to_cols_array(),
        };
        let review = Review {
            documentation_url: "https://github.com/tum-gis/3d-model-new-york-city".into(),
            approved_host: "dataset.example.org".into(),
            licence: "Original test fixture, Apache-2.0; not a TUM license claim".into(),
            licence_evidence: "Original test triangles".into(),
            attribution: "Original FlightCraft fixture".into(),
            redistribution_allowed: false,
            source_crs: "Local glTF Y-up meters with an explicit ECEF transform".into(),
            models: vec![model.clone()],
        };
        let request = Request {
            operation: Operation::Build,
            region: "new-york".into(),
            area: "nyc-lod2".into(),
            cache: root.join("cache"),
            out: root.join("built"),
            sample: None,
            allow_http: false,
        };
        transport::fetch(
            &mut Fixture(bytes),
            &model.url,
            &request.cache.join("objects"),
            tiles3d::MAX_TILE_CONTENT_BYTES as u64,
        )
        .unwrap();
        std::fs::create_dir_all(&request.out).unwrap();
        let report = build(&request, &review, &request.out).unwrap();
        assert!(!report.release_ready);
        let set = Tileset::from_json(
            &download::read(&request.out.join("tileset.json"), tiles3d::MAX_TILESET_BYTES).unwrap(),
        )
        .unwrap();
        assert_eq!(
            set.extras.as_ref().unwrap().pointer("/flightcraft/localOnly"),
            Some(&json!(true))
        );
        let coarse = &set.root.children[0];
        let middle = &coarse.children[0];
        let fine = &middle.children[0];
        assert!(
            coarse.geometric_error > middle.geometric_error && middle.geometric_error > fine.geometric_error
        );
        let source_region =
            Region::from_json(include_str!("../../../assets/terrain/new-york/region.json")).unwrap();
        let transform = set.root.transform.unwrap();
        let inverse = DMat4::from_cols_array(&transform).inverse();
        let restored = super::triangles(
            &download::read(
                &request.out.join(&fine.content.as_ref().unwrap().uri),
                tiles3d::MAX_TILE_CONTENT_BYTES,
            )
            .unwrap(),
            &Model {
                url: model.url.clone(),
                gltf_to_ecef: (DMat4::from_cols_array(&transform)
                    * DMat4::from_rotation_x(std::f64::consts::FRAC_PI_2))
                .to_cols_array(),
            },
            inverse,
        )
        .unwrap();
        let expected = super::triangles(&encode(&triangles, 0.).unwrap(), &model, inverse).unwrap();
        assert_eq!(restored.len(), expected.len());
        for (a, b) in restored.iter().flatten().zip(expected.iter().flatten()) {
            assert!((*a - *b).length() < 0.02);
        }
        assert_eq!(source_region.id, "new-york");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn missing_license_or_host_evidence_fails_before_network_and_crs_errors_fail_closed() {
        let path = std::env::temp_dir().join(format!("fc-missing-city-review-{}.json", std::process::id()));
        assert!(review(&path).unwrap_err().contains("no verified"));
        assert!(!transport::exact_host(
            "https://dataset.example.org.attacker.invalid/file.glb",
            "dataset.example.org"
        ));
        assert!(!transport::exact_host(
            "https://dataset.example.org@attacker.invalid/file.glb",
            "dataset.example.org"
        ));
        assert!(!transport::exact_host(
            "http://dataset.example.org/file.glb",
            "dataset.example.org"
        ));
        assert!(!transport::exact_host(
            "https://dataset.example.org:443/file.glb",
            "dataset.example.org"
        ));
        assert!(!affine([0.; 16]));
        let bytes = encode(&[[DVec3::ZERO, DVec3::X, DVec3::Y]], 0.).unwrap();
        assert!(
            triangles(
                &bytes,
                &Model {
                    url: "unused".into(),
                    gltf_to_ecef: DMat4::IDENTITY.to_cols_array()
                },
                DMat4::IDENTITY
            )
            .is_err()
        );
    }
}
