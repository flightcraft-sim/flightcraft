//! Fly NYC: the `new-york/hudson-corridor` area (docs/scenery.md, "Hudson corridor").
//!
//! The 1N7 pipeline builds one square, complete, single-resolution quadtree. The corridor is a
//! 20 km river with a skyline on one bank, so it needs a *sparse, variable-resolution* quadtree
//! (0.6 m imagery near the Hudson, coarser outward), lidar processed in bounded blocks, and
//! building tiles that share a tiny procedural atlas instead of one texture per building. This
//! module owns the plan (which tiles exist, which lidar nodes are wanted) and the acquisition;
//! `corridor_build` assembles tiles and `corridor_buildings` reconstructs and meshes buildings.
use super::{Input, InputKind, Request, Sources, acquire, download, ept, geoid, transport, water};
use flightcraft_world::scenery::{self, TileKey};

pub(super) const AREA: &str = "hudson-corridor";
/// Level of the 0.6 m tiles. The root square is 2^8 tiles of 512 px at 0.6 m ground sample.
pub(super) const FINEST: u8 = 8;
/// Coarsest content level. Tiles above it are content-less grouping nodes.
pub(super) const FOREST: u8 = 3;
pub(super) const TILE_PX: u32 = 512;
/// Mercator scale reference latitude for the nominal ground sample.
pub(super) const LAT0: f64 = 40.75;
pub(super) const GROUND_SAMPLE_M: f64 = 0.6;
pub(super) const ROOT_CENTER: [f64; 2] = [40.76, -73.99];
/// Buildings and trees are reconstructed this far (ground metres) from the Hudson centreline.
pub(super) const BUILDING_REACH_M: f64 = 4200.;
/// Hudson River centreline, Upper Bay to the George Washington Bridge (lat, lon).
const HUDSON: [[f64; 2]; 11] = [
    [40.640, -74.045],
    [40.668, -74.040],
    [40.690, -74.030],
    [40.703, -74.020],
    [40.725, -74.017],
    [40.745, -74.012],
    [40.765, -74.003],
    [40.785, -73.994],
    [40.805, -73.982],
    [40.828, -73.964],
    [40.853, -73.952],
];
/// Teterboro (KTEB) and the straight departure spur to the Hudson at the bridge.
pub(super) const KTEB: [f64; 2] = [40.8501, -74.0608];
const SPUR: [[f64; 2]; 2] = [KTEB, [40.853, -73.952]];
/// fly-nyc.json envelope: west, south, east, north (degrees).
pub(super) const ENVELOPE: [f64; 4] = flightcraft_render::scenery::NYC_BOUNDS;

pub(super) fn merc(lat: f64, lon: f64) -> [f64; 2] {
    scenery::mercator(lat, lon).unwrap_or([0.; 2])
}
fn cos0() -> f64 {
    libm::cos(LAT0.to_radians())
}
#[derive(Clone, Debug)]
enum Shape {
    Line(Vec<[f64; 2]>),
    Rect([f64; 4]),
}
/// Ground-metre radius per level 4..=8 (negative: the shape asks nothing at that level).
#[derive(Clone, Debug)]
struct Zone {
    shape: Shape,
    radius_m: [f64; 5],
}
fn point_rect(p: [f64; 2], r: [f64; 4]) -> f64 {
    let dx = (r[0] - p[0]).max(0.).max(p[0] - r[2]);
    let dy = (r[1] - p[1]).max(0.).max(p[1] - r[3]);
    libm::hypot(dx, dy)
}
fn point_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length2 = dx * dx + dy * dy;
    let t = if length2 == 0. {
        0.
    } else {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length2).clamp(0., 1.)
    };
    libm::hypot(p[0] - (a[0] + t * dx), p[1] - (a[1] + t * dy))
}
fn segment_hits_rect(a: [f64; 2], b: [f64; 2], r: [f64; 4]) -> bool {
    let (mut t0, mut t1) = (0_f64, 1_f64);
    for (p, q) in [
        (-(b[0] - a[0]), a[0] - r[0]),
        (b[0] - a[0], r[2] - a[0]),
        (-(b[1] - a[1]), a[1] - r[1]),
        (b[1] - a[1], r[3] - a[1]),
    ] {
        if p == 0. {
            if q < 0. {
                return false;
            }
        } else {
            let t = q / p;
            if p < 0. {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
            if t0 > t1 {
                return false;
            }
        }
    }
    true
}
/// Distance in Mercator metres from a rectangle to a polyline (zero when they touch).
fn rect_line(r: [f64; 4], line: &[[f64; 2]]) -> f64 {
    if let [only] = line {
        return point_rect(*only, r);
    }
    let corners = [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]];
    let mut best = f64::INFINITY;
    for pair in line.windows(2) {
        let [a, b] = pair else { continue };
        if segment_hits_rect(*a, *b, r) {
            return 0.;
        }
        best = best.min(point_rect(*a, r)).min(point_rect(*b, r));
        for c in corners {
            best = best.min(point_segment(c, *a, *b));
        }
    }
    best
}
fn rect_rect(a: [f64; 4], b: [f64; 4]) -> f64 {
    let dx = (a[0] - b[2]).max(b[0] - a[2]).max(0.);
    let dy = (a[1] - b[3]).max(b[1] - a[3]).max(0.);
    libm::hypot(dx, dy)
}
#[derive(Clone, Debug)]
pub(super) struct Plan {
    /// Square EPSG:3857 root: west, south, east, north.
    pub root: [f64; 4],
    zones: Vec<Zone>,
    hudson: Vec<[f64; 2]>,
    envelope: [f64; 4],
}
pub(super) fn plan() -> Plan {
    let pixel = GROUND_SAMPLE_M / cos0();
    let width = pixel * f64::from(TILE_PX) * f64::from(1_u32 << FINEST);
    let c = merc(ROOT_CENTER[0], ROOT_CENTER[1]);
    let line = |points: &[[f64; 2]]| points.iter().map(|p| merc(p[0], p[1])).collect::<Vec<_>>();
    let envelope = {
        let [x0, y0] = merc(ENVELOPE[1], ENVELOPE[0]);
        let [x1, y1] = merc(ENVELOPE[3], ENVELOPE[2]);
        [x0, y0, x1, y1]
    };
    let hudson = line(&HUDSON);
    Plan {
        root: [
            c[0] - width / 2.,
            c[1] - width / 2.,
            c[0] + width / 2.,
            c[1] + width / 2.,
        ],
        zones: vec![
            // Levels:                 4        5       6       7      8
            Zone {
                shape: Shape::Line(hudson.clone()),
                radius_m: [18_000., 12_000., 8_000., 5_000., 2_200.],
            },
            Zone {
                shape: Shape::Line(line(&SPUR)),
                radius_m: [10_000., 6_000., 3_500., 1_800., 700.],
            },
            Zone {
                shape: Shape::Line(line(&[KTEB])),
                radius_m: [10_000., 7_000., 4_500., 2_500., 1_400.],
            },
            Zone {
                shape: Shape::Rect(envelope),
                radius_m: [12_000., 7_000., 3_000., 0., -1.],
            },
        ],
        hudson,
        envelope,
    }
}
impl Plan {
    pub fn tile_bounds(&self, key: TileKey) -> [f64; 4] {
        let size = (self.root[2] - self.root[0]) / f64::from(1_u32 << key.level);
        let west = self.root[0] + f64::from(key.x) * size;
        let north = self.root[3] - f64::from(key.y) * size;
        [west, north - size, west + size, north]
    }
    /// Ground metres per pixel at a level (nominal, at the reference latitude).
    #[cfg(test)]
    pub fn ground_sample(&self, level: u8) -> f64 {
        (self.root[2] - self.root[0]) * cos0() / f64::from(1_u32 << level) / f64::from(TILE_PX)
    }
    pub fn envelope(&self) -> [f64; 4] {
        self.envelope
    }
    fn wants(&self, bounds: [f64; 4], level: u8) -> bool {
        let slot = usize::from(level - 4);
        self.zones.iter().any(|zone| {
            let radius = zone.radius_m[slot];
            if radius < 0. {
                return false;
            }
            let distance = match &zone.shape {
                Shape::Line(line) => rect_line(bounds, line),
                Shape::Rect(rect) => rect_rect(bounds, *rect),
            };
            distance * cos0() <= radius
        })
    }
    fn subdivide(&self, key: TileKey) -> bool {
        key.level < FINEST
            && self
                .children(key)
                .iter()
                .any(|c| self.wants(self.tile_bounds(*c), c.level))
    }
    pub fn children(&self, key: TileKey) -> [TileKey; 4] {
        let l = key.level + 1;
        [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(dx, dy)| TileKey {
            level: l,
            x: key.x * 2 + dx,
            y: key.y * 2 + dy,
        })
    }
    /// The complete sparse quadtree: every tile that gets content, with its leaf flag. Each
    /// refined tile has all four children, so replacement refinement never leaves a hole.
    pub fn tiles(&self) -> Vec<(TileKey, bool)> {
        let mut out = Vec::new();
        let side = 1_u32 << FOREST;
        let mut stack: Vec<TileKey> = Vec::new();
        for y in 0..side {
            for x in 0..side {
                let key = TileKey { level: FOREST, x, y };
                if self.wants(self.tile_bounds(key), FOREST + 1) {
                    stack.push(key);
                }
            }
        }
        while let Some(key) = stack.pop() {
            let leaf = !self.subdivide(key);
            out.push((key, leaf));
            if !leaf {
                stack.extend(self.children(key));
            }
        }
        out.sort();
        out
    }
    pub fn leaves(&self) -> Vec<TileKey> {
        self.tiles()
            .into_iter()
            .filter(|(_, leaf)| *leaf)
            .map(|(k, _)| k)
            .collect()
    }
    /// Is a Mercator rectangle part of the lidar reconstruction zone (envelope, near the Hudson)?
    pub fn lidar_wants(&self, rect: [f64; 4]) -> bool {
        rect_rect(rect, self.envelope) <= 0. && rect_line(rect, &self.hudson) * cos0() <= BUILDING_REACH_M
    }
    /// Bounds of the tiles at 2.4 m or finer: where water surfaces are drawn.
    pub fn water_bounds(&self) -> [f64; 4] {
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for key in self.leaves().into_iter().filter(|k| k.level >= 6) {
            let t = self.tile_bounds(key);
            b = [b[0].min(t[0]), b[1].min(t[1]), b[2].max(t[2]), b[3].max(t[3])];
        }
        b
    }
    /// Union Mercator bounds of every tile (not square): what acquisition must cover.
    pub fn data_bounds(&self) -> [f64; 4] {
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for key in self.leaves() {
            let t = self.tile_bounds(key);
            b = [b[0].min(t[0]), b[1].min(t[1]), b[2].max(t[2]), b[3].max(t[3])];
        }
        b
    }
}

pub(super) fn new_sources(plan: &Plan) -> Sources {
    Sources {
        version: 1,
        region: "new-york".into(),
        area: AREA.into(),
        bounds: plan.data_bounds(),
        tile_pixels: TILE_PX,
        max_level: FINEST,
        ground_sample_m: GROUND_SAMPLE_M,
        inputs: Vec::new(),
        ept_selection: None,
    }
}
fn imagery_url(bounds: [f64; 4]) -> String {
    let [w, s, e, n] = bounds;
    format!(
        "{}/exportImage?bbox={w:.9},{s:.9},{e:.9},{n:.9}&bboxSR=3857&imageSR=3857&size=512,512&format=jpg&interpolation=RSP_BilinearInterpolation&adjustAspectRatio=false&f=image",
        super::NAIP
    )
}
/// The EPT depth for the corridor: 0.75 m ground voxels (about 2 returns per square metre of
/// roof), enough for the one-metre DSM that roof planes are fitted to.
pub(super) const EPT_SPACING_M: f64 = 0.75;

pub(super) fn fetch(request: &Request, remote: &mut impl transport::Remote) -> Result<Sources, String> {
    std::fs::create_dir_all(&request.cache).map_err(|e| e.to_string())?;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(request.cache.join(".fetch-lock"))
        .map_err(|e| e.to_string())?;
    lock.try_lock()
        .map_err(|e| format!("another scenery fetch may be using this cache: {e}"))?;
    let plan = plan();
    let planned = new_sources(&plan);
    let mut sources: Sources = if request.cache.join("sources.json").exists() {
        serde_json::from_slice(&download::read(
            &request.cache.join("sources.json"),
            256 * 1024 * 1024,
        )?)
        .map_err(|e| e.to_string())?
    } else {
        planned.clone()
    };
    if sources.region != planned.region
        || sources.area != planned.area
        || sources.bounds != planned.bounds
        || sources.max_level != planned.max_level
        || sources.inputs.len() > 100_000
    {
        return Err("existing scenery source catalog does not match the hudson-corridor plan".into());
    }
    let layer = request.fetch_layer;
    use super::FetchLayer::{All, Geoid, Lidar, Terrain, Water};
    if matches!(layer, All | Geoid) {
        geoid::fetch(request, &mut sources, remote)?;
    }
    if matches!(layer, All | Water) {
        // NHD HU-8 subbasins are large; ask only for the area that gets water surfaces.
        let all = sources.bounds;
        sources.bounds = plan.water_bounds();
        let result = water::fetch(request, &mut sources, remote);
        sources.bounds = all;
        result?;
        super::write_sources(&request.cache, &sources)?;
    }
    if matches!(layer, All | Terrain) {
        fetch_dems(request, &mut sources, remote)?;
        fetch_imagery(request, &plan, &mut sources, remote)?;
    }
    if matches!(layer, All | Lidar) {
        sources.ept_selection = None;
        super::write_sources(&request.cache, &sources)?;
        let fraction = std::env::var("FLIGHTCRAFT_HUDSON_ZONE").ok();
        let zone = |rect: [f64; 4]| match fraction.as_deref() {
            // Developer knob: a Midtown test window, to validate reconstruction early.
            Some("midtown") => {
                let c = merc(40.7536, -73.9832);
                rect_rect(rect, [c[0] - 900., c[1] - 900., c[0] + 900., c[1] + 900.]) <= 0.
            }
            _ => plan.lidar_wants(rect),
        };
        // The query rectangle bounds the zone; the zone predicate is exact.
        let mut query = plan.envelope();
        query[0] -= 150.;
        query[1] -= 150.;
        query[2] += 150.;
        query[3] += 150.;
        let selection = ept::fetch_zone(
            ept::NYC_ROOT,
            query,
            &zone,
            EPT_SPACING_M,
            ept::ZoneBudget {
                points: 600_000_000,
                nodes: 12_000,
            },
            &mut super::EptCache {
                request,
                sources: &mut sources,
                remote,
            },
        )?;
        sources.ept_selection = Some(selection);
        super::write_sources(&request.cache, &sources)?;
    }
    Ok(sources)
}
fn fetch_dems(
    request: &Request,
    sources: &mut Sources,
    remote: &mut impl transport::Remote,
) -> Result<(), String> {
    let [w, s, e, n] = sources.bounds;
    let [south, west] = scenery::inverse_mercator(w, s).map_err(|e| e.to_string())?;
    let [north, east] = scenery::inverse_mercator(e, n).map_err(|e| e.to_string())?;
    // Only products that touch a tile worth 4.8 m or finer imagery, plus Teterboro's runways.
    let plan = plan();
    let mut wanted: Vec<[f64; 4]> = Vec::new();
    for key in plan.leaves().into_iter().filter(|k| k.level >= 5) {
        let b = plan.tile_bounds(key);
        let [s0, w0] = scenery::inverse_mercator(b[0], b[1]).map_err(|e| e.to_string())?;
        let [n0, e0] = scenery::inverse_mercator(b[2], b[3]).map_err(|e| e.to_string())?;
        wanted.push([w0, s0, e0, n0]);
    }
    wanted.push([KTEB[1] - 0.02, KTEB[0] - 0.02, KTEB[1] + 0.02, KTEB[0] + 0.02]);
    let keep = |item: &serde_json::Value| -> bool {
        let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("");
        // The 2014 Sandy-era NJ product is superseded by the 2017 and 2023 NJ lidar DEMs.
        if title.contains("NJ SdL5 2014") {
            return false;
        }
        let Some(b) = item.get("boundingBox") else {
            return false;
        };
        let get = |k: &str| b.get(k).and_then(|v| v.as_f64());
        let (Some(x0), Some(y0), Some(x1), Some(y1)) = (get("minX"), get("minY"), get("maxX"), get("maxY"))
        else {
            return false;
        };
        wanted
            .iter()
            .any(|w| x0 < w[2] && x1 > w[0] && y0 < w[3] && y1 > w[1])
    };
    super::fetch_dem_products(request, sources, remote, [west, south, east, north], Some(&keep))?;
    // Products fetched by an earlier, wider request that the filter no longer wants are
    // intermediate data: drop their objects and receipts so the cache stays small.
    let discovered = super::dem_products_kept(request, sources, &keep)?;
    let mut removed = 0_u64;
    sources.inputs.retain(|input| {
        if input.kind != InputKind::Dem || discovered.contains(&input.source.url) {
            return true;
        }
        let object = request
            .cache
            .join("objects")
            .join(download::sha256(input.source.url.as_bytes()));
        for path in [object.clone(), object.with_extension("json")] {
            let _ = std::fs::remove_file(path);
        }
        removed += input.source.bytes;
        false
    });
    if removed > 0 {
        eprintln!(
            "corridor: dropped {} MB of DEM products outside the plan",
            removed / 1_000_000
        );
        super::write_sources(&request.cache, sources)?;
    }
    Ok(())
}
fn fetch_imagery(
    request: &Request,
    plan: &Plan,
    sources: &mut Sources,
    remote: &mut impl transport::Remote,
) -> Result<(), String> {
    let metadata = acquire(
        request,
        sources,
        remote,
        InputKind::NaipMetadata,
        None,
        &format!("{}?f=json", super::NAIP),
        4 * 1024 * 1024,
    )?;
    let metadata: serde_json::Value = download::json(&metadata)?;
    if metadata.get("error").is_some() {
        return Err("NAIP returned an ArcGIS service error".into());
    }
    for name in ["maxImageWidth", "maxImageHeight"] {
        if metadata
            .get(name)
            .and_then(|v| v.as_u64())
            .is_none_or(|n| n < u64::from(TILE_PX))
        {
            return Err(format!("NAIP {name} is smaller than the 512-pixel tile request"));
        }
    }
    let mut leaves = plan.leaves();
    // A Midtown test window first (so reconstruction can be checked early), then coarse tiles
    // before fine ones: a stopped run still leaves a complete low-resolution world.
    let c = merc(40.7536, -73.9832);
    let midtown = [c[0] - 1_200., c[1] - 1_200., c[0] + 1_200., c[1] + 1_200.];
    leaves.sort_by_key(|k| (rect_rect(plan.tile_bounds(*k), midtown) > 0., k.level, k.y, k.x));
    // Tiles that already have a verified receipt are not requested again.
    let have: std::collections::BTreeSet<TileKey> = sources
        .inputs
        .iter()
        .filter(|i| i.kind == InputKind::Imagery)
        .filter_map(|i| i.key)
        .collect();
    let mut todo: std::collections::VecDeque<(TileKey, String)> = leaves
        .into_iter()
        .filter(|k| !have.contains(k))
        .map(|k| (k, imagery_url(plan.tile_bounds(k))))
        .collect();
    // Verify the receipts we are skipping, exactly as `acquire` would.
    for key in &have {
        let url = imagery_url(plan.tile_bounds(*key));
        let path = acquire(
            request,
            sources,
            remote,
            InputKind::Imagery,
            Some(*key),
            &url,
            8 * 1024 * 1024,
        )?;
        super::decode_image(&download::read(&path, 8 * 1024 * 1024)?, Some(TILE_PX))?;
    }
    fetch_tiles(request, sources, &mut todo)
}

/// In-flight imagery exports. The NAIP ImageServer takes 20 to 30 s to render one 512-pixel tile,
/// so one request at a time would need over a day. The rule is one request *start* per second
/// across the whole process (`Gated`); this many may be waiting for answers at once.
const IMAGERY_IN_FLIGHT: usize = 24;
const IMAGERY_ATTEMPTS: u32 = 8;

/// Spaces the start of every request, from every worker, at least one second apart.
struct Gated {
    inner: transport::Http,
    gate: std::sync::Arc<std::sync::Mutex<std::time::Instant>>,
}
impl transport::Remote for Gated {
    fn open(
        &mut self,
        url: &str,
        offset: u64,
        validator: Option<&str>,
    ) -> Result<transport::Response, String> {
        let start = {
            let mut next = self.gate.lock().map_err(|_| "request gate poisoned")?;
            let at = (*next).max(std::time::Instant::now());
            *next = at + std::time::Duration::from_secs(1);
            at
        };
        std::thread::sleep(start.saturating_duration_since(std::time::Instant::now()));
        self.inner.open(url, offset, validator)
    }
}
type Fetched = Result<(TileKey, String, download::Source), String>;

fn fetch_tiles(
    request: &Request,
    sources: &mut Sources,
    todo: &mut std::collections::VecDeque<(TileKey, String)>,
) -> Result<(), String> {
    let total = todo.len();
    if total == 0 {
        return Ok(());
    }
    let queue = std::sync::Mutex::new(std::mem::take(todo));
    let stop = std::sync::atomic::AtomicBool::new(false);
    let gate = std::sync::Arc::new(std::sync::Mutex::new(std::time::Instant::now()));
    let objects = request.cache.join("objects");
    let (sender, receiver) = std::sync::mpsc::channel::<Fetched>();
    let mut failure: Option<String> = None;
    std::thread::scope(|scope| {
        for _ in 0..IMAGERY_IN_FLIGHT.min(total) {
            let sender = sender.clone();
            let (queue, stop, gate, objects) = (&queue, &stop, gate.clone(), &objects);
            scope.spawn(move || {
                let mut remote = Gated {
                    inner: transport::Http::default(),
                    gate,
                };
                loop {
                    if stop.load(std::sync::atomic::Ordering::Relaxed) {
                        return;
                    }
                    let Some((key, url)) = queue.lock().ok().and_then(|mut q| q.pop_front()) else {
                        return;
                    };
                    let mut last = String::new();
                    let mut done = None;
                    for attempt in 0..IMAGERY_ATTEMPTS {
                        if stop.load(std::sync::atomic::Ordering::Relaxed) {
                            return;
                        }
                        match transport::fetch(&mut remote, &url, objects, 8 * 1024 * 1024).and_then(
                            |(path, source)| {
                                super::decode_image(&download::read(&path, 8 * 1024 * 1024)?, Some(TILE_PX))?;
                                Ok(source)
                            },
                        ) {
                            Ok(source) => {
                                done = Some(source);
                                break;
                            }
                            // HTTP 429 means stop everything; other failures (a gateway timeout, a
                            // truncated body) are retried through the same one-per-second gate.
                            Err(e) if e.contains("429") => {
                                stop.store(true, std::sync::atomic::Ordering::Relaxed);
                                let _ = sender.send(Err(e));
                                return;
                            }
                            Err(e) => {
                                last = e;
                                // A corrupt partial object is dropped so the retry starts clean.
                                let key = download::sha256(url.as_bytes());
                                for suffix in [".part", ".partial.json"] {
                                    let _ = std::fs::remove_file(objects.join(format!("{key}{suffix}")));
                                }
                                std::thread::sleep(std::time::Duration::from_secs(
                                    3 * u64::from(attempt + 1),
                                ));
                            }
                        }
                    }
                    let message = match done {
                        Some(source) => Ok((key, url, source)),
                        None => Err(format!(
                            "tile {} failed after {IMAGERY_ATTEMPTS} attempts: {last}",
                            key.name()
                        )),
                    };
                    if sender.send(message).is_err() {
                        return;
                    }
                }
            });
        }
        drop(sender);
        let mut done = 0_usize;
        while let Ok(message) = receiver.recv() {
            match message {
                Ok((key, url, source)) => {
                    let input = Input {
                        kind: InputKind::Imagery,
                        key: Some(key),
                        source,
                    };
                    if let Some(previous) = sources
                        .inputs
                        .iter_mut()
                        .find(|i| i.kind == input.kind && i.key == input.key && i.source.url == url)
                    {
                        *previous = input;
                    } else {
                        sources.inputs.push(input);
                    }
                    done += 1;
                    if done.is_multiple_of(25) || done == total {
                        // Receipts are committed in batches: an interrupted run re-verifies the
                        // objects it finds on disk and requests only what has no receipt.
                        if let Err(e) = super::write_sources(&request.cache, sources) {
                            failure.get_or_insert(e);
                            stop.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                    if done.is_multiple_of(100) {
                        eprintln!("corridor imagery {done}/{total}");
                    }
                }
                Err(e) => {
                    failure.get_or_insert(e);
                    stop.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
    });
    let _ = super::write_sources(&request.cache, sources);
    failure.map_or(Ok(()), Err)
}

pub(super) fn run(request: &Request) -> Result<(), String> {
    if request.region != "new-york" {
        return Err("hudson-corridor belongs to the new-york region".into());
    }
    if request.sample.is_some() || request.allow_http {
        return Err("--sample and --allow-http do not apply to hudson-corridor".into());
    }
    match request.operation {
        super::Operation::Fetch => {
            if request.lidar_source != super::LidarSource::Ept {
                return Err("hudson-corridor buildings come only from the NY_NewYorkCity EPT".into());
            }
            let sources = fetch(request, &mut transport::Http::default())?;
            println!(
                "fetched {} receipts; ept {:?}",
                sources.inputs.len(),
                sources.ept_selection
            );
            Ok(())
        }
        super::Operation::Build => super::corridor_build::build(request),
    }
}

#[allow(dead_code)]
pub(super) fn input_for(sources: &Sources, key: TileKey) -> Option<&Input> {
    sources
        .inputs
        .iter()
        .find(|i| i.kind == InputKind::Imagery && i.key == Some(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_is_a_complete_sparse_quadtree_with_fine_tiles_by_the_hudson() {
        let plan = plan();
        let tiles = plan.tiles();
        let set: std::collections::BTreeMap<_, _> = tiles.iter().copied().collect();
        for (key, leaf) in &tiles {
            if !leaf {
                assert!(plan.children(*key).iter().all(|c| set.contains_key(c)), "{key:?}");
            }
            if let Some(parent) = key.parent().filter(|p| p.level >= FOREST) {
                assert_eq!(set.get(&parent), Some(&false));
            }
        }
        let count = |level| tiles.iter().filter(|(k, leaf)| *leaf && k.level == level).count();
        eprintln!("leaves by level: {:?}", (4..=8).map(count).collect::<Vec<_>>());
        eprintln!("data bounds: {:?}", plan.data_bounds());
        assert!(count(8) > 800 && count(8) < 2400);
        assert!(plan.leaves().len() < 3200);
        // Teterboro, the Battery, the bridge and Midtown are all at the finest or second level.
        for (lat, lon, min_level) in [
            (40.8501, -74.0608, 8),
            (40.7025, -74.0167, 8),
            (40.8517, -73.9527, 8),
            (40.7536, -73.9832, 7),
        ] {
            let p = merc(lat, lon);
            let leaf = plan.leaves().into_iter().find(|k| {
                let b = plan.tile_bounds(*k);
                p[0] >= b[0] && p[0] < b[2] && p[1] >= b[1] && p[1] < b[3]
            });
            assert!(
                leaf.is_some_and(|k| k.level >= min_level),
                "{lat},{lon}: {leaf:?}"
            );
        }
        let root = plan.root;
        assert!(((root[2] - root[0]) - (root[3] - root[1])).abs() < 1e-6);
        let b = plan.data_bounds();
        assert!(b[0] >= root[0] && b[1] >= root[1] && b[2] <= root[2] && b[3] <= root[3]);
        let ground = plan.ground_sample(FINEST);
        assert!((ground - 0.6).abs() < 1e-9);
    }
    #[test]
    fn lidar_zone_stays_inside_the_fly_nyc_envelope() {
        let plan = plan();
        let m = merc(40.7536, -73.9832);
        assert!(plan.lidar_wants([m[0] - 10., m[1] - 10., m[0] + 10., m[1] + 10.]));
        let teb = merc(40.8501, -74.0608);
        assert!(!plan.lidar_wants([teb[0] - 10., teb[1] - 10., teb[0] + 10., teb[1] + 10.]));
    }
}
