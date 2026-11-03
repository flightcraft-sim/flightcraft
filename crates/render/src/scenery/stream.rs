#[cfg(not(target_arch = "wasm32"))]
use super::{CachePolicy, TileProvider};
use super::{DecodedTile, Quality};
use flightcraft_world::{
    geo::LocalProjection,
    tiles3d::{self, BoundingVolume, Refine, Tile, Tileset},
};
use glam::{DMat4, DVec3};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone)]
struct Node {
    uri: Option<String>,
    depth: usize,
    transform: [f64; 16],
    center: [f64; 3],
    radius: f64,
    error: f64,
    children: Vec<usize>,
    add: bool,
    geoid: Option<Arc<flightcraft_world::geoid::GeoidGrid>>,
}
/// A transport-neutral request. URIs are opaque relative content identifiers; providers
/// resolve their own paths or authenticated URLs outside the renderer.
#[derive(Clone, Debug)]
pub struct ContentRequest {
    pub id: usize,
    pub uri: String,
    pub transform: [f64; 16],
    /// Regional MSL conversion; never a provider-specific coordinate offset.
    pub geoid: Option<Arc<flightcraft_world::geoid::GeoidGrid>>,
}
impl ContentRequest {
    pub fn decode(&self, bytes: &[u8], origin: [f64; 2]) -> Result<Arc<DecodedTile>, String> {
        super::decode_glb_with_geoid(bytes, self.transform, origin, self.geoid.as_deref())
    }
}
fn content_request(nodes: &[Node], id: usize) -> Option<ContentRequest> {
    let node = nodes.get(id)?;
    Some(ContentRequest {
        id,
        uri: node.uri.clone()?,
        transform: node.transform,
        geoid: node.geoid.clone(),
    })
}
#[derive(Clone, Default)]
pub struct Selection {
    pub tiles: Vec<(usize, Arc<DecodedTile>)>,
    pub requests: Vec<ContentRequest>,
    pub(crate) fallback: Option<(usize, Arc<DecodedTile>)>,
    pub attribution: String,
    pub stats: StreamStats,
    pub message: Option<String>,
    pub(crate) live_tiles: Vec<(usize, Arc<DecodedTile>)>,
}
impl Selection {
    /// Both layers still use the same renderer. Namespaces prevent provider-local ID collisions.
    pub(crate) fn overlay(&mut self, mut layer: Self) {
        let offset = tiles3d::MAX_TILESET_TILES;
        for (id, _) in layer.tiles.iter_mut().chain(layer.live_tiles.iter_mut()) {
            *id += offset;
        }
        if let Some((id, _)) = &mut layer.fallback {
            *id += offset;
        }
        self.tiles.extend(layer.tiles);
        self.live_tiles.extend(layer.live_tiles);
        if self.fallback.is_none() {
            self.fallback = layer.fallback;
        }
        if !layer.attribution.is_empty() {
            if !self.attribution.is_empty() {
                self.attribution.push_str(" · ");
            }
            self.attribution.push_str(&layer.attribution);
        }
        self.message = layer.message.or(self.message.take());
        self.stats.resident_bytes += layer.stats.resident_bytes;
        self.stats.resident_tiles += layer.stats.resident_tiles;
        self.stats.pending_tiles += layer.stats.pending_tiles;
        self.stats.failed_tiles += layer.stats.failed_tiles;
        self.stats.selected_tiles += layer.stats.selected_tiles;
        self.stats.selected_instances += layer.stats.selected_instances;
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct StreamStats {
    pub resident_bytes: usize,
    pub resident_tiles: usize,
    pub pending_tiles: usize,
    pub failed_tiles: usize,
    pub selected_tiles: usize,
    pub selected_instances: usize,
}

struct Worker {
    requests: std::sync::mpsc::SyncSender<usize>,
    results: std::sync::mpsc::Receiver<(usize, Result<Arc<DecodedTile>, String>)>,
}
pub struct Stream {
    nodes: Arc<Vec<Node>>,
    attribution: String,
    resident: BTreeMap<usize, Arc<DecodedTile>>,
    pending: BTreeSet<usize>,
    failed: BTreeSet<usize>,
    worker: Option<Worker>,
    #[cfg(target_arch = "wasm32")]
    pub(super) browser: Option<super::browser::Worker>,
    last_selected: usize,
    last_error: Option<String>,
    last_used: BTreeMap<usize, u64>,
    tick: u64,
    last_instances: usize,
    observed_sizes: BTreeMap<usize, usize>,
}
impl Stream {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_provider(mut provider: Box<dyn TileProvider>, origin: [f64; 2]) -> Result<Self, String> {
        let tileset = Tileset::from_json(&provider.tileset()?).map_err(|e| e.to_string())?;
        let mut stream = Self::new(&tileset, origin)?;
        let (request_tx, request_rx) = std::sync::mpsc::sync_channel::<usize>(2);
        let (result_tx, result_rx) = std::sync::mpsc::sync_channel(2);
        let nodes = stream.nodes.clone();
        std::thread::Builder::new()
            .name("local-scenery".into())
            .spawn(move || {
                while let Ok(id) = request_rx.recv() {
                    let result = (|| {
                        let request = content_request(&nodes, id).ok_or("unknown or empty scenery tile")?;
                        let content = provider.content(&request.uri)?;
                        if content.cache_policy == CachePolicy::MemoryOnly && content.attribution.is_empty() {
                            return Err("Streaming scenery is missing attribution".into());
                        }
                        if content
                            .expires_at_unix_ms
                            .is_some_and(|expiry| super::unix_ms() >= expiry)
                        {
                            return Err("Streaming scenery expired before decoding".into());
                        }
                        let mut tile = request.decode(&content.bytes, origin)?;
                        let payload = Arc::get_mut(&mut tile).ok_or("shared scenery payload")?;
                        payload.expires_at_unix_ms = content.expires_at_unix_ms;
                        payload.attribution = content.attribution;
                        Ok(tile)
                    })();
                    if result_tx.send((id, result)).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        stream.worker = Some(Worker {
            requests: request_tx,
            results: result_rx,
        });
        Ok(stream)
    }

    pub fn new(set: &Tileset, origin: [f64; 2]) -> Result<Self, String> {
        set.validate().map_err(|e| e.to_string())?;
        let projection = LocalProjection::new(origin[0], origin[1]).map_err(|e| e.to_string())?;
        let geoid = set.geoid_grid().map_err(|e| e.to_string())?.map(Arc::new);
        let undulation_span_m = geoid.as_ref().map_or(0., |grid| {
            let [lo, hi] = grid
                .undulation_m
                .iter()
                .fold([f32::INFINITY, f32::NEG_INFINITY], |[lo, hi], &v| {
                    [lo.min(v), hi.max(v)]
                });
            f64::from(hi - lo)
        });
        let reference = Georeference {
            projection,
            geoid,
            undulation_span_m,
        };
        let mut nodes = Vec::new();
        flatten(
            &set.root,
            DMat4::IDENTITY,
            Refine::Replace,
            &reference,
            &mut nodes,
            0,
        )?;
        Ok(Self {
            nodes: Arc::new(nodes),
            attribution: set.asset.copyright.clone().unwrap_or_default(),
            resident: BTreeMap::new(),
            pending: BTreeSet::new(),
            failed: BTreeSet::new(),
            worker: None,
            #[cfg(target_arch = "wasm32")]
            browser: None,
            last_selected: 0,
            last_error: None,
            last_used: BTreeMap::new(),
            tick: 0,
            last_instances: 0,
            observed_sizes: BTreeMap::new(),
        })
    }
    pub fn content_request(&self, id: usize) -> Option<ContentRequest> {
        content_request(&self.nodes, id)
    }
    pub fn reject(&mut self, id: usize) {
        if id < self.nodes.len() {
            self.pending.remove(&id);
            self.failed.insert(id);
        }
    }
    pub fn stats(&self) -> StreamStats {
        StreamStats {
            resident_bytes: self.resident.values().map(|tile| tile.byte_size).sum(),
            resident_tiles: self.resident.len(),
            pending_tiles: self.pending.len(),
            failed_tiles: self.failed.len(),
            selected_tiles: self.last_selected,
            selected_instances: self.last_instances,
        }
    }
    /// Also used by browser/file adapters: accept decoded, bounded content off the draw thread.
    pub fn insert(&mut self, id: usize, tile: Arc<DecodedTile>, quality: Quality) -> Result<(), String> {
        if id >= self.nodes.len() || tile.byte_size > quality.resident_bytes() / 2 {
            return Err("scenery tile exceeds resident budget".into());
        }
        self.observed_sizes.insert(id, tile.byte_size);
        let used: usize = self
            .resident
            .iter()
            .filter(|(key, _)| **key != id)
            .map(|(_, t)| t.byte_size)
            .sum();
        if used.saturating_add(tile.byte_size) > quality.resident_bytes() {
            return Err("scenery resident budget is full".into());
        }
        self.resident.insert(id, tile);
        self.pending.remove(&id);
        Ok(())
    }
    /// No file reads, decoding, waits, or unbounded upload queues in the frame callback.
    pub fn select(&mut self, eye_ned: [f64; 3], height_px: f64, fov_rad: f64, quality: Quality) -> Selection {
        if eye_ned.iter().any(|v| !v.is_finite())
            || !height_px.is_finite()
            || !(1.0..=8192.).contains(&height_px)
            || !fov_rad.is_finite()
            || !(0.01..=3.).contains(&fov_rad)
        {
            return Selection::default();
        }
        if self.resident.values().map(|t| t.byte_size).sum::<usize>() > quality.resident_bytes() {
            self.resident
                .retain(|id, tile| *id == 0 && tile.byte_size <= quality.resident_bytes() / 2);
        }
        self.resident.retain(|_, tile| !tile.expired());
        self.tick = self.tick.saturating_add(1);
        let mut results = Vec::new();
        if let Some(worker) = &self.worker {
            for _ in 0..quality.uploads_per_frame() {
                match worker.results.try_recv() {
                    Ok(result) => results.push(result),
                    Err(_) => break,
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(browser) = &self.browser {
            for _ in 0..quality.uploads_per_frame() {
                match browser.results.try_recv() {
                    Ok(result) => results.push(result),
                    Err(_) => break,
                }
            }
        }
        for (id, result) in results {
            self.pending.remove(&id);
            match result {
                Ok(tile) => {
                    if tile.byte_size > quality.resident_bytes() / 2 {
                        self.failed.insert(id);
                    } else {
                        let _ = self.insert(id, tile, quality);
                    }
                }
                Err(error) => {
                    self.last_error = Some(error);
                    self.failed.insert(id);
                }
            }
        }
        let focal = height_px / (2. * (fov_rad / 2.).tan());
        let mut desired = BTreeSet::new();
        let mut selected = Vec::new();
        // Increase screen error when the requested refinement cannot fit. Otherwise a
        // fully visible tree can permanently fill the cache with incomplete sibling groups.
        // Tiny building/instance payloads must not each reserve a full imagery tile.
        // Remember decoded sizes even when a full cache defers insertion; the next selection
        // can then coarsen accurately instead of retrying an underestimated large tile forever.
        let unknown_size = 2 * 1024 * 1024;
        let mut target_error = quality.screen_error_px();
        for _ in 0..64 {
            desired.clear();
            selected.clear();
            visit(
                0,
                &self.nodes,
                &self.resident,
                eye_ned,
                focal,
                target_error,
                &mut desired,
                &mut selected,
                quality.resident_bytes() / (64 * 1024),
            );
            let estimated: usize = desired.iter().fold(0_usize, |sum, id| {
                sum.saturating_add(self.observed_sizes.get(id).copied().unwrap_or(unknown_size))
            });
            if estimated <= quality.resident_bytes() {
                break;
            }
            target_error *= 1.5;
        }
        // Keep the selected coverage and every ancestor. Evict least recently selected
        // unused content first; root fallback stays while children arrive.
        let mut used: BTreeSet<_> = selected.iter().copied().collect();
        used.extend(desired.iter().copied());
        let mut bytes: usize = self.resident.values().map(|t| t.byte_size).sum();
        let mut eviction: Vec<_> = self
            .resident
            .keys()
            .filter(|id| !used.contains(id) && **id != 0)
            .filter_map(|id| self.nodes.get(*id).map(|n| (*id, distance(n, eye_ned))))
            .collect();
        eviction.sort_by_key(|(id, _)| self.last_used.get(id).copied().unwrap_or(0));
        for (id, _) in eviction {
            if bytes.saturating_add(quality.resident_bytes() / 8) <= quality.resident_bytes() {
                break;
            }
            if let Some(tile) = self.resident.remove(&id) {
                bytes = bytes.saturating_sub(tile.byte_size);
            }
        }
        // A strict cap also prevents a malicious tree from requesting more siblings than fit.
        let mut requests: Vec<_> = desired
            .iter()
            .filter(|id| {
                !self.resident.contains_key(id) && !self.pending.contains(id) && !self.failed.contains(id)
            })
            .copied()
            .collect();
        requests.sort_by(|a, b| match (self.nodes.get(*a), self.nodes.get(*b)) {
            (Some(a), Some(b)) => a
                .depth
                .cmp(&b.depth)
                .then_with(|| distance(a, eye_ned).total_cmp(&distance(b, eye_ned))),
            _ => a.cmp(b),
        }); // Coarse coverage first, then the nearest refinement within that level.
        let mut external_requests = Vec::new();
        if let Some(worker) = &self.worker {
            for id in requests.iter().copied() {
                if self.pending.len() >= 2 || bytes >= quality.resident_bytes() {
                    break;
                }
                if worker.requests.try_send(id).is_ok() {
                    self.pending.insert(id);
                } else {
                    break;
                }
            }
        }
        if self.worker.is_none() {
            for id in requests
                .into_iter()
                .take(2_usize.saturating_sub(self.pending.len()))
            {
                if let Some(request) = self.content_request(id) {
                    self.pending.insert(id);
                    external_requests.push(request);
                }
            }
        }
        for id in &selected {
            self.last_used.insert(*id, self.tick);
        }
        self.last_used.retain(|id, _| self.resident.contains_key(id));
        let tiles: Vec<_> = selected
            .into_iter()
            .filter_map(|id| self.resident.get(&id).map(|tile| (id, tile.clone())))
            .collect();
        self.last_selected = tiles.len();
        #[cfg(target_arch = "wasm32")]
        if let Some(browser) = &self.browser {
            browser.dispatch(&external_requests);
            external_requests.clear();
        }
        self.last_instances = tiles.iter().map(|(_, t)| t.instance_count()).sum();
        let stats = StreamStats {
            resident_bytes: bytes,
            resident_tiles: self.resident.len(),
            pending_tiles: self.pending.len(),
            failed_tiles: self.failed.len(),
            selected_tiles: tiles.len(),
            selected_instances: self.last_instances,
        };
        let mut credits = BTreeSet::new();
        if !self.attribution.is_empty() {
            credits.insert(self.attribution.clone());
        }
        for tile in self.resident.values() {
            credits.extend(tile.attribution.iter().cloned());
        }
        Selection {
            message: self.last_error.clone(),
            live_tiles: self
                .resident
                .iter()
                .map(|(id, tile)| (*id, tile.clone()))
                .collect(),
            requests: external_requests,
            fallback: self.resident.get(&0).map(|tile| (0, tile.clone())),
            tiles,
            attribution: credits.into_iter().collect::<Vec<_>>().join(" · "),
            stats,
        }
    }
}
fn distance(node: &Node, eye: [f64; 3]) -> f64 {
    let [n, e, h] = node.center;
    let [en, ee, down] = eye;
    (((n - en).powi(2) + (e - ee).powi(2) + (h + down).powi(2)).sqrt() - node.radius).max(1.)
}
#[allow(clippy::too_many_arguments)]
fn visit(
    id: usize,
    nodes: &[Node],
    resident: &BTreeMap<usize, Arc<DecodedTile>>,
    eye: [f64; 3],
    focal: f64,
    error: f64,
    desired: &mut BTreeSet<usize>,
    selected: &mut Vec<usize>,
    limit: usize,
) -> bool {
    if desired.len() > limit {
        return false;
    }
    let Some(node) = nodes.get(id) else {
        return false;
    };
    if node.uri.is_some() {
        desired.insert(id);
    }
    let refine =
        !node.children.is_empty() && (node.uri.is_none() || node.error * focal / distance(node, eye) > error);
    if refine {
        let mut children = Vec::new();
        let mut ready = true;
        for child in &node.children {
            ready &= visit(
                *child,
                nodes,
                resident,
                eye,
                focal,
                error,
                desired,
                &mut children,
                limit,
            );
        }
        if ready {
            if node.add && resident.contains_key(&id) {
                selected.push(id);
            }
            selected.extend(children);
            return true;
        }
    }
    if resident.contains_key(&id) {
        selected.push(id);
        return true;
    }
    false
}
struct Georeference {
    projection: LocalProjection,
    geoid: Option<Arc<flightcraft_world::geoid::GeoidGrid>>,
    undulation_span_m: f64,
}
impl Georeference {
    fn msl(&self, lat: f64, lon: f64, height: f64) -> Result<f64, String> {
        self.geoid.as_ref().map_or(Ok(height), |grid| {
            grid.orthometric_m(lat, lon, height).map_err(|e| e.to_string())
        })
    }
}
fn flatten(
    tile: &Tile,
    parent: DMat4,
    refine: Refine,
    reference: &Georeference,
    nodes: &mut Vec<Node>,
    depth: usize,
) -> Result<usize, String> {
    let transform = parent
        * tile
            .transform
            .map(|m| DMat4::from_cols_array(&m))
            .unwrap_or(DMat4::IDENTITY);
    let (center, radius) = bounds(&tile.bounding_volume, transform, reference)?;
    let id = nodes.len();
    let refine = tile.refine.unwrap_or(refine);
    nodes.push(Node {
        depth,
        uri: tile.content.as_ref().map(|c| c.uri.clone()),
        transform: transform.to_cols_array(),
        center,
        radius,
        error: tile.geometric_error,
        children: Vec::new(),
        add: refine == Refine::Add,
        geoid: reference.geoid.clone(),
    });
    let mut children = Vec::new();
    for child in &tile.children {
        children.push(flatten(child, transform, refine, reference, nodes, depth + 1)?);
    }
    nodes.get_mut(id).ok_or("missing flattened tile")?.children = children;
    Ok(id)
}
fn bounds(
    volume: &BoundingVolume,
    transform: DMat4,
    reference: &Georeference,
) -> Result<([f64; 3], f64), String> {
    let projection = &reference.projection;
    if let Some([w, s, e, n, lo, hi]) = volume.region {
        let [lat, lon] = [((s + n) / 2.).to_degrees(), ((w + e) / 2.).to_degrees()];
        let [north, east] = projection.project(lat, lon).map_err(|e| e.to_string())?;
        let [cn, ce] = projection
            .project(s.to_degrees(), w.to_degrees())
            .map_err(|e| e.to_string())?;
        return Ok((
            [north, east, reference.msl(lat, lon, (lo + hi) / 2.)?],
            ((north - cn).powi(2) + (east - ce).powi(2) + ((hi - lo) / 2.).powi(2)).sqrt()
                + reference.undulation_span_m,
        ));
    }
    let (center, radius) = if let Some([x, y, z, r]) = volume.sphere {
        (DVec3::new(x, y, z), r)
    } else if let Some(b) = volume.box_volume {
        let [x, y, z, ax, ay, az, bx, by, bz, cx, cy, cz] = b;
        (
            DVec3::new(x, y, z),
            DVec3::new(ax, ay, az).length()
                + DVec3::new(bx, by, bz).length()
                + DVec3::new(cx, cy, cz).length(),
        )
    } else {
        return Err("missing 3D Tiles bounding volume".into());
    };
    let [lat, lon, height] =
        tiles3d::geodetic(transform.transform_point3(center).to_array()).map_err(|e| e.to_string())?;
    let [n, e] = projection.project(lat, lon).map_err(|e| e.to_string())?;
    let scale = transform
        .x_axis
        .truncate()
        .length()
        .max(transform.y_axis.truncate().length())
        .max(transform.z_axis.truncate().length());
    Ok((
        [n, e, reference.msl(lat, lon, height)?],
        radius * scale + reference.undulation_span_m,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flightcraft_world::tiles3d::{Asset, Content};
    fn set() -> Tileset {
        let leaf = || Tile {
            bounding_volume: BoundingVolume::region([-1.31, 0.71, -1.30, 0.72, 0., 200.]),
            geometric_error: 0.,
            refine: None,
            transform: None,
            content: Some(Content {
                uri: "tile.glb".into(),
            }),
            children: Vec::new(),
        };
        let mut root = leaf();
        root.geometric_error = 1000.;
        root.children = (0..4).map(|_| leaf()).collect();
        root.refine = Some(Refine::Replace);
        Tileset {
            asset: Asset {
                version: "1.1".into(),
                copyright: Some("Original fixture".into()),
            },
            geometric_error: 1000.,
            root,
            extensions_required: Vec::new(),
            extras: None,
        }
    }
    fn tile() -> Arc<DecodedTile> {
        Arc::new(DecodedTile {
            vertices: Vec::new(),
            instanced: Vec::new(),
            mips: vec![image::RgbaImage::new(1, 1)],
            byte_size: 4,
            terrain_bounds: None,
            expires_at_unix_ms: None,
            attribution: Vec::new(),
        })
    }
    #[test]
    fn small_building_tiles_do_not_force_unnecessary_imagery_coarsening() {
        let mut set = set();
        let child = set.root.children[0].clone();
        set.root.children = vec![child; 32];
        let mut stream = Stream::new(&set, [41., -75.]).unwrap();
        for id in 0..=32 {
            stream.insert(id, tile(), Quality::Low).unwrap();
        }
        let selected = stream.select([0., 0., -100.], 1080., 1., Quality::Low);
        assert_eq!(selected.tiles.len(), 32);
        assert!(selected.stats.resident_bytes <= Quality::Low.resident_bytes());
    }
    #[test]
    fn replacement_retains_parent_until_all_children_arrive_and_respects_budget() {
        let mut stream = Stream::new(&set(), [41., -75.]).unwrap();
        let sample = |s: &mut Stream| s.select([0., 0., -100.], 1080., 1., Quality::Low);
        assert!(sample(&mut stream).tiles.is_empty());
        stream.insert(0, tile(), Quality::Low).unwrap();
        for child in 1..4 {
            stream.insert(child, tile(), Quality::Low).unwrap();
            assert_eq!(
                sample(&mut stream)
                    .tiles
                    .iter()
                    .map(|(id, _)| *id)
                    .collect::<Vec<_>>(),
                vec![0]
            );
        }
        stream.insert(4, tile(), Quality::Low).unwrap();
        assert_eq!(sample(&mut stream).tiles.len(), 4);
        assert!(stream.insert(100, tile(), Quality::Low).is_err());
        assert!(
            stream
                .insert(
                    1,
                    Arc::new(DecodedTile {
                        vertices: Vec::new(),
                        instanced: Vec::new(),
                        mips: Vec::new(),
                        byte_size: Quality::Low.resident_bytes(),
                        terrain_bounds: None,
                        expires_at_unix_ms: None,
                        attribution: Vec::new(),
                    }),
                    Quality::Low
                )
                .is_err()
        );
        assert!(sample(&mut stream).stats.resident_bytes <= Quality::Low.resident_bytes());
        assert!(
            stream
                .select([f64::NAN, 0., 0.], 1080., 1., Quality::Low)
                .tiles
                .is_empty()
        );
    }
    #[test]
    fn local_city_and_imagery_layers_keep_content_ids_and_credits_separate() {
        let mut city = Stream::new(&set(), [41., -75.]).unwrap();
        let mut imagery = Stream::new(&set(), [41., -75.]).unwrap();
        let mut city_tile = tile();
        Arc::get_mut(&mut city_tile)
            .unwrap()
            .instanced
            .push(super::super::InstanceBatch {
                vertices: Vec::new(),
                instances: vec![
                    super::super::Instance {
                        matrix: glam::Mat4::IDENTITY.to_cols_array_2d()
                    };
                    3
                ],
            });
        city.insert(0, city_tile, Quality::Low).unwrap();
        imagery.insert(0, tile(), Quality::Low).unwrap();
        let mut selected = city.select([0., 0., -100.], 1080., 1., Quality::Low);
        let mut layer = imagery.select([0., 0., -100.], 1080., 1., Quality::Low);
        layer.attribution = "Imagery fixture credit".into();
        selected.overlay(layer);
        assert_eq!(
            selected.tiles.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![0, tiles3d::MAX_TILESET_TILES]
        );
        assert_eq!(selected.live_tiles.len(), 2);
        assert_eq!(selected.stats.selected_instances, 3);
        assert!(selected.attribution.contains("Original fixture"));
        assert!(selected.attribution.contains("Imagery fixture credit"));
    }
    #[test]
    fn expiry_purges_cpu_and_gpu_eligibility_and_preserves_valid_parent_coverage() {
        let mut stream = Stream::new(&set(), [41., -75.]).unwrap();
        stream.insert(0, tile(), Quality::Low).unwrap();
        for id in 1..=4 {
            stream.insert(id, tile(), Quality::Low).unwrap();
        }
        let selected = stream.select([0., 0., -100.], 1080., 1., Quality::Low);
        assert_eq!(selected.tiles.len(), 4);
        let mut expired = tile();
        let payload = Arc::get_mut(&mut expired).unwrap();
        payload.expires_at_unix_ms = Some(0);
        payload.attribution = vec!["Expired credit".into()];
        stream.insert(1, expired, Quality::Low).unwrap();
        let selection = stream.select([0., 0., -100.], 1080., 1., Quality::Low);
        assert_eq!(
            selection.tiles.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![0]
        );
        assert!(selection.live_tiles.iter().all(|(id, _)| *id != 1));
        assert!(!selection.attribution.contains("Expired credit"));
        assert!(selection.requests.iter().any(|request| request.id == 1));
    }
    #[test]
    fn stream_bounds_and_requests_share_the_regional_height_datum() {
        let mut tileset = set();
        let grid = flightcraft_world::geoid::GeoidGrid {
            model: flightcraft_world::geoid::GeoidModel::NgsGeoid18,
            source_sha256: "a".repeat(64),
            south_deg: 40.,
            west_deg: -76.,
            step_deg: [1. / 60.; 2],
            rows: 120,
            cols: 120,
            undulation_m: vec![-31.; 120 * 120],
        };
        tileset.extras = Some(serde_json::json!({"flightcraft":{"geoid":grid}}));
        let stream = Stream::new(&tileset, [41., -75.]).unwrap();
        assert!((stream.nodes[0].center[2] - 131.).abs() < 1e-9);
        let request = stream.content_request(0).unwrap();
        assert!(request.geoid.is_some());
        assert!(Arc::ptr_eq(
            request.geoid.as_ref().unwrap(),
            stream.content_request(1).unwrap().geoid.as_ref().unwrap()
        ));
        tileset.extras.as_mut().unwrap()["flightcraft"]["geoid"]["rows"] = serde_json::json!(u32::MAX);
        assert!(Stream::new(&tileset, [41., -75.]).is_err());
    }
    #[test]
    fn event_loop_requests_are_bounded_and_keep_the_inherited_transform() {
        let mut tileset = set();
        let root = DMat4::from_cols_array(&tiles3d::enu_transform(41., -75., 0.).unwrap());
        let child = DMat4::from_translation(DVec3::new(1., 2., 3.));
        tileset.root.transform = Some(root.to_cols_array());
        tileset.root.children.first_mut().unwrap().transform = Some(child.to_cols_array());
        let mut stream = Stream::new(&tileset, [41., -75.]).unwrap();
        let first = stream.select([0., 0., -100.], 1080., 1., Quality::Low);
        assert_eq!(first.requests.len(), 2);
        assert_eq!(first.requests.first().unwrap().id, 0);
        assert_eq!(first.requests.first().unwrap().uri, "tile.glb");
        assert_eq!(first.requests.first().unwrap().transform, root.to_cols_array());
        assert_eq!(
            stream.content_request(1).unwrap().transform,
            (root * child).to_cols_array()
        );
        assert!(
            stream
                .select([0., 0., -100.], 1080., 1., Quality::Low)
                .requests
                .is_empty()
        );
        stream.reject(0);
        assert_eq!(stream.stats().failed_tiles, 1);
        assert_eq!(
            stream
                .select([0., 0., -100.], 1080., 1., Quality::Low)
                .requests
                .len(),
            1
        );
        assert!(stream.content_request(usize::MAX).is_none());
        stream.reject(usize::MAX);
    }
    #[test]
    fn geometric_error_selects_coarser_content_at_distance() {
        let mut set = set();
        set.root.geometric_error = 1.;
        set.geometric_error = 1.;
        let mut stream = Stream::new(&set, [41., -75.]).unwrap();
        for id in 0..5 {
            stream.insert(id, tile(), Quality::Medium).unwrap();
        }
        let selected = stream.select([200_000., 200_000., -10_000.], 1080., 1., Quality::Medium);
        assert_eq!(selected.tiles.len(), 1);
        assert_eq!(selected.tiles.first().unwrap().0, 0);
    }
}
