//! Opt-in native transport. No disk cache, acquisition receipts, URLs with keys, or bulk API.
use super::{CachePolicy, Content, Location, Provider, Stream, TileProvider, surface_glb};
use flightcraft_world::{
    World, scenery,
    tiles3d::{self, Asset, BoundingVolume, Refine, Tile, Tileset},
};
use std::{io::Read, time::Duration};

pub const ENDPOINT: &str = "https://ibasemaps-api.arcgis.com/arcgis/rest/services/World_Imagery/MapServer";
const CREDIT: &str =
    "Powered by Esri · World Imagery: Esri, Maxar, Earthstar Geographics, and the GIS User Community";
const MAX_RESPONSE: u64 = 4 * 1024 * 1024;
/// A single regional quadtree, not a global download. Only selected content is requested.
const ROOT_LEVEL: u8 = 9;
const LEAF_LEVEL: u8 = 15;

pub struct EsriProvider {
    agent: ureq::Agent,
    key: String,
    endpoint: String,
    world: World,
    root_range: [u32; 4],
    coverage: [f64; 4],
    heights: [f64; 2],
    stopped: Option<&'static str>,
    credit: Option<String>,
}
impl EsriProvider {
    pub fn endpoint() -> &'static str {
        ENDPOINT
    }
    pub fn new(settings: &Provider, key: String, world: World) -> Result<Self, String> {
        if !settings.enabled {
            return Err("Streaming imagery is disabled".into());
        }
        if !matches!(&settings.location, Location::Url(url) if url == ENDPOINT)
            || settings.token_config_key.as_deref() != Some("esri-imagery")
        {
            return Err("Choose the supported Esri imagery provider".into());
        }
        Self::create(key, world, ENDPOINT.into())
    }
    fn create(key: String, world: World, endpoint: String) -> Result<Self, String> {
        if key.is_empty() || key.len() > 4096 || !key.bytes().all(|b| b.is_ascii_graphic()) {
            return Err("Enter a valid ArcGIS API key".into());
        }
        let region = world
            .region()
            .ok_or("Streaming imagery requires an installed region")?;
        let projection = flightcraft_world::geo::LocalProjection::new(
            region.origin.latitude_deg,
            region.origin.longitude_deg,
        )
        .map_err(|e| e.to_string())?;
        let extent = f64::from(region.tile_size - 1) * f64::from(region.spacing_m);
        let [south, west] = region.southwest_m;
        let north = south + f64::from(region.tiles_north) * extent;
        let east = west + f64::from(region.tiles_east) * extent;
        let mut coverage = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for (n, e) in [(south, west), (north, west), (south, east), (north, east)] {
            let [lat, lon] = projection.unproject(n, e).map_err(|e| e.to_string())?;
            let [x, y] = scenery::mercator(lat, lon).map_err(|e| e.to_string())?;
            let [w, s, e, n] = &mut coverage;
            *w = w.min(x);
            *s = s.min(y);
            *e = e.max(x);
            *n = n.max(y);
        }
        let half = std::f64::consts::PI * scenery::MERCATOR_RADIUS_M;
        let side = 1_u32 << ROOT_LEVEL;
        let index = |v: f64| {
            (((v + half) / (2. * half) * f64::from(side))
                .floor()
                .clamp(0., f64::from(side - 1))) as u32
        };
        let [w, s, e, n] = coverage;
        // Read source grid vertices once. Tall arbitrary volumes distort geometric-error LOD.
        let mut heights = [region.origin.field_elevation_m; 2];
        for tile in world.tiles() {
            let [on, oe] = tile.origin_m();
            for north in 0..tile.size() {
                for east in 0..tile.size() {
                    let n = on + (f64::from(north) * tile.spacing_m()).min(tile.extent_m() - 1e-8);
                    let e = oe + (f64::from(east) * tile.spacing_m()).min(tile.extent_m() - 1e-8);
                    if let Some(ground) = tile.sample(n, e) {
                        let [low, high] = &mut heights;
                        *low = low.min(ground.height_msl_m);
                        *high = high.max(ground.height_msl_m);
                    }
                }
            }
        }
        for height in world.runway_strips().iter().flat_map(|strip| strip.elevations_m) {
            let [low, high] = &mut heights;
            *low = low.min(height);
            *high = high.max(height);
        }
        let [low, high] = heights;
        let heights = [low - 1., high + 1.];
        let root_range = [index(w), index(-n), index(e), index(-s)];
        let [left, top, right, bottom] = root_range;
        if (right - left + 1).saturating_mul(bottom - top + 1) > 64 {
            return Err("Installed region exceeds the streaming imagery coverage limit".into());
        }
        Ok(Self {
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(15)))
                .max_redirects(0)
                .http_status_as_error(false)
                .build()
                .into(),
            key,
            endpoint,
            world,
            root_range,
            coverage,
            heights,
            stopped: None,
            credit: None,
        })
    }
    /// Creating the stream reads no network; all requests run on its bounded decoding worker.
    pub fn open(settings: &Provider, key: String, world: World) -> Result<Stream, String> {
        let region = world
            .region()
            .ok_or("Streaming imagery requires an installed region")?;
        let origin = [region.origin.latitude_deg, region.origin.longitude_deg];
        Stream::with_provider(Box::new(Self::new(settings, key, world)?), origin)
    }
    fn request(&mut self, path: &str) -> Result<Vec<u8>, String> {
        if let Some(message) = self.stopped {
            return Err(message.into());
        }
        // Never format transport errors, response bodies, headers, or redirects: servers may echo keys.
        let mut request = self
            .agent
            .get(format!("{}{path}", self.endpoint))
            .header("Authorization", format!("Bearer {}", self.key))
            .header("Cache-Control", "no-store");
        if let Some(value) = request
            .headers_mut()
            .and_then(|headers| headers.get_mut("authorization"))
        {
            value.set_sensitive(true);
        }
        let mut response = match request.call() {
            Ok(response) => response,
            Err(_) => {
                let message = "Streaming imagery connection failed; streaming has stopped.";
                self.stopped = Some(message);
                return Err(message.into());
            }
        };
        let status = response.status().as_u16();
        let message = match status {
            200 => None,
            402 | 429 => Some(
                "Streaming imagery limit reached. Check your ArcGIS account's free allowance; streaming has stopped.",
            ),
            401 | 403 | 498 | 499 => Some(
                "Streaming imagery key was rejected. Check your ArcGIS key and account allowance; streaming has stopped.",
            ),
            400..=499 => Some(
                "Streaming imagery request was rejected; streaming has stopped. Check your ArcGIS account.",
            ),
            _ => Some("Streaming imagery service is unavailable; streaming has stopped."),
        };
        if let Some(message) = message {
            self.stopped = Some(message);
            return Err(message.into());
        }
        let mut bytes = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(MAX_RESPONSE + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Streaming imagery response was interrupted")?;
        if bytes.is_empty() || bytes.len() as u64 > MAX_RESPONSE {
            return Err("Streaming imagery response exceeds size limits".into());
        }
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes)
            && value.get("error").is_some()
        {
            let message = match value.pointer("/error/code").and_then(|v| v.as_u64()) {
                Some(402 | 429) => {
                    "Streaming imagery limit reached. Check your ArcGIS account's free allowance; streaming has stopped."
                }
                _ => {
                    "Streaming imagery key or allowance was rejected; streaming has stopped. Check your ArcGIS account."
                }
            };
            self.stopped = Some(message);
            return Err(message.into());
        }
        Ok(bytes)
    }
}
fn bounds(level: u8, x: u32, y: u32) -> [f64; 4] {
    let half = std::f64::consts::PI * scenery::MERCATOR_RADIUS_M;
    let size = 2. * half / f64::from(1_u32 << level);
    let w = -half + f64::from(x) * size;
    let n = half - f64::from(y) * size;
    [w, n - size, w + size, n]
}
fn node(
    level: u8,
    x: u32,
    y: u32,
    coverage: [f64; 4],
    heights: [f64; 2],
    count: &mut usize,
) -> Result<Tile, String> {
    *count += 1;
    if *count > 40_000 {
        return Err("Installed region exceeds the streaming imagery tile limit".into());
    }
    let [w, s, e, n] = bounds(level, x, y);
    let [south, west] = scenery::inverse_mercator(w, s).map_err(|e| e.to_string())?;
    let [north, east] = scenery::inverse_mercator(e, n).map_err(|e| e.to_string())?;
    let mut children = Vec::new();
    if level < LEAF_LEVEL {
        for dy in 0..2 {
            for dx in 0..2 {
                let [cw, cs, ce, cn] = bounds(level + 1, x * 2 + dx, y * 2 + dy);
                let [rw, rs, re, rn] = coverage;
                if cw < re && ce > rw && cs < rn && cn > rs {
                    children.push(node(level + 1, x * 2 + dx, y * 2 + dy, coverage, heights, count)?);
                }
            }
        }
    }
    Ok(Tile {
        bounding_volume: BoundingVolume::region([
            west.to_radians(),
            south.to_radians(),
            east.to_radians(),
            north.to_radians(),
            heights[0],
            heights[1],
        ]),
        geometric_error: if children.is_empty() { 0. } else { (e - w) / 256. },
        refine: Some(Refine::Replace),
        transform: None,
        content: Some(tiles3d::Content {
            uri: format!("imagery/{level}/{x}/{y}.glb"),
        }),
        children,
    })
}
impl TileProvider for EsriProvider {
    fn tileset(&mut self) -> Result<Vec<u8>, String> {
        let region = self
            .world
            .region()
            .ok_or("Streaming imagery requires an installed region")?;
        let [left, top, right, bottom] = self.root_range;
        let mut children = Vec::new();
        let mut count = 1;
        for y in top..=bottom {
            for x in left..=right {
                let [cw, cs, ce, cn] = bounds(ROOT_LEVEL, x, y);
                let [w, s, e, n] = self.coverage;
                if cw < e && ce > w && cs < n && cn > s {
                    children.push(node(ROOT_LEVEL, x, y, self.coverage, self.heights, &mut count)?);
                }
            }
        }
        let [w, s, _, _] = bounds(ROOT_LEVEL, left, bottom);
        let [_, _, e, n] = bounds(ROOT_LEVEL, right, top);
        let [south, west] = scenery::inverse_mercator(w, s).map_err(|e| e.to_string())?;
        let [north, east] = scenery::inverse_mercator(e, n).map_err(|e| e.to_string())?;
        let mut root = Tile {
            bounding_volume: BoundingVolume::region([
                west.to_radians(),
                south.to_radians(),
                east.to_radians(),
                north.to_radians(),
                self.heights[0],
                self.heights[1],
            ]),
            geometric_error: 10000.,
            refine: Some(Refine::Add),
            transform: None,
            content: None,
            children,
        };
        root.transform = Some(
            tiles3d::enu_transform(region.origin.latitude_deg, region.origin.longitude_deg, 0.)
                .map_err(|e| e.to_string())?,
        );
        let set = Tileset {
            asset: Asset {
                version: "1.1".into(),
                copyright: Some(CREDIT.into()),
            },
            geometric_error: root.geometric_error,
            root,
            extensions_required: Vec::new(),
            extras: None,
        };
        set.validate().map_err(|e| e.to_string())?;
        serde_json::to_vec(&set).map_err(|_| "Cannot encode imagery tileset".into())
    }
    fn content(&mut self, uri: &str) -> Result<Content, String> {
        let result = (|| {
            if uri.len() > 128 {
                return Err("Invalid imagery content identifier".into());
            }
            let parts: Vec<_> = uri.split('/').collect();
            let ["imagery", level, x, y] = parts.as_slice() else {
                return Err("Invalid imagery content identifier".into());
            };
            let level = level.parse::<u8>().map_err(|_| "Invalid imagery level")?;
            let x = x.parse::<u32>().map_err(|_| "Invalid imagery column")?;
            let y = y
                .strip_suffix(".glb")
                .ok_or("Invalid imagery content")?
                .parse::<u32>()
                .map_err(|_| "Invalid imagery row")?;
            if !(ROOT_LEVEL..=LEAF_LEVEL).contains(&level) {
                return Err("Invalid imagery level".into());
            }
            let [left, top, right, bottom] = self.root_range;
            let (root_x, root_y) = (x >> (level - ROOT_LEVEL), y >> (level - ROOT_LEVEL));
            if !(left..=right).contains(&root_x) || !(top..=bottom).contains(&root_y) {
                return Err("Imagery tile is outside the selected region".into());
            }
            let [w, s, e, n] = bounds(level, x, y);
            let [rw, rs, re, rn] = self.coverage;
            if w >= re || e <= rw || s >= rn || n <= rs {
                return Err("Imagery tile is outside the installed region".into());
            }
            if self.credit.is_none() {
                let metadata = self.request("?f=json")?;
                let value: serde_json::Value = serde_json::from_slice(&metadata)
                    .map_err(|_| "Streaming imagery attribution is unavailable")?;
                let credit = value
                    .get("copyrightText")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty() && s.len() <= 4096)
                    .ok_or("Streaming imagery attribution is unavailable")?;
                self.credit = Some(credit.replace(&self.key, "[redacted]").replace(['\r', '\n'], " "));
            }
            let bytes = self.request(&format!("/tile/{level}/{y}/{x}"))?;
            // Reject JSON service errors, oversized images, and decompression bombs without echoing data.
            let mut reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
                .with_guessed_format()
                .map_err(|_| "Invalid streaming imagery image")?;
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(256);
            limits.max_image_height = Some(256);
            limits.max_alloc = Some(1024 * 1024);
            reader.limits(limits);
            let image = reader.decode().map_err(
            |_| "Streaming imagery did not return a supported tile. Check your key and account allowance.",
        )?;
            if image.width() != 256 || image.height() != 256 {
                return Err("Streaming imagery returned an unexpected tile size".into());
            }
            Ok(Content {
                bytes: surface_glb(&self.world, bounds(level, x, y), &bytes)?,
                attribution: vec![self.credit.clone().ok_or("Missing streaming attribution")?],
                cache_policy: CachePolicy::MemoryOnly,
                // Conservative session residency; no offline right is inferred from service headers.
                expires_at_unix_ms: Some(super::unix_ms().saturating_add(5 * 60 * 1000)),
            })
        })();
        if result.is_err() {
            self.stopped.get_or_insert("Streaming imagery content is unavailable; streaming has stopped. Check your key and account.");
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use ureq::unversioned::transport::{
        Buffers, ConnectionDetails, Connector, LazyBuffers, NextTimeout, Transport,
    };
    fn world() -> World {
        let region = flightcraft_world::Region::from_json(include_str!(
            "../../../../assets/terrain/new-york/region.json"
        ))
        .unwrap();
        let mut region = region;
        region.tiles_north = 1;
        region.tiles_east = 1;
        region.tile_size = 33;
        region.spacing_m = 125.;
        region.southwest_m = [-1000., -1000.];
        region.tiles = vec!["000.fcht".into()];
        let tile = flightcraft_world::HeightTile::new(
            [-1000., -1000.],
            125.,
            33,
            vec![112.; 33 * 33],
            vec![flightcraft_world::Surface::Grass; 32 * 32],
        )
        .unwrap();
        World::from_region(region, vec![tile]).unwrap()
    }
    // A local HTTP tile server carried in process. No sockets or DNS, even in CI.
    #[derive(Debug)]
    struct Server {
        status: u16,
        body: Vec<u8>,
        key: &'static str,
        calls: Arc<AtomicUsize>,
    }
    #[derive(Debug)]
    struct Connection {
        buffers: LazyBuffers,
        request: Vec<u8>,
        response: std::io::Cursor<Vec<u8>>,
        status: u16,
        body: Vec<u8>,
        key: &'static str,
    }
    impl Connector for Server {
        type Out = Connection;
        fn connect(
            &self,
            details: &ConnectionDetails,
            _: Option<()>,
        ) -> Result<Option<Connection>, ureq::Error> {
            assert_eq!(details.uri.host(), Some("127.0.0.1"));
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(Some(Connection {
                buffers: LazyBuffers::new(8192, 8192),
                request: Vec::new(),
                response: std::io::Cursor::new(Vec::new()),
                status: self.status,
                body: self.body.clone(),
                key: self.key,
            }))
        }
    }
    impl Transport for Connection {
        fn buffers(&mut self) -> &mut dyn Buffers {
            &mut self.buffers
        }
        fn transmit_output(&mut self, amount: usize, _: NextTimeout) -> Result<(), ureq::Error> {
            self.request.extend_from_slice(&self.buffers.output()[..amount]);
            if self.request.ends_with(b"\r\n\r\n") {
                let request = String::from_utf8(self.request.clone()).unwrap();
                assert!(!request.lines().next().unwrap().contains(self.key));
                assert!(
                    request
                        .to_ascii_lowercase()
                        .contains(&format!("authorization: bearer {}", self.key))
                );
                assert!(request.contains("no-store"));
                let metadata = format!(
                    r#"{{"copyrightText":"Original local test imagery credits {}"}}"#,
                    self.key
                );
                let data = if request.lines().next().unwrap().contains("?f=json") {
                    metadata.as_bytes()
                } else {
                    self.body.as_slice()
                };
                let mut response = format!(
                    "HTTP/1.1 {} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    self.status,
                    data.len()
                )
                .into_bytes();
                response.extend(data);
                self.response = std::io::Cursor::new(response);
            }
            Ok(())
        }
        fn await_input(&mut self, _: NextTimeout) -> Result<bool, ureq::Error> {
            let count = self.response.read(self.buffers.input_append_buf()).unwrap();
            self.buffers.input_appended(count);
            Ok(count > 0)
        }
        fn is_open(&mut self) -> bool {
            self.request.is_empty() || self.response.position() < self.response.get_ref().len() as u64
        }
    }
    fn server(status: u16, body: Vec<u8>, key: &'static str) -> (ureq::Agent, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let config = ureq::Agent::config_builder()
            .proxy(None)
            .max_redirects(0)
            .http_status_as_error(false)
            .build();
        let agent = ureq::Agent::with_parts(
            config,
            Server {
                status,
                body,
                key,
                calls: calls.clone(),
            },
            ureq::unversioned::resolver::DefaultResolver::default(),
        );
        (agent, calls)
    }
    fn with_server(agent: ureq::Agent, key: &str) -> EsriProvider {
        let mut provider = EsriProvider::create(key.into(), world(), "http://127.0.0.1:9".into()).unwrap();
        provider.agent = agent;
        provider
    }
    #[test]
    fn local_server_drapes_image_and_keeps_key_out_of_tileset_and_content() {
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode(
                &vec![120; 256 * 256 * 3],
                256,
                256,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        let key = "secret-fixture-key";
        let (agent, calls) = server(200, jpeg, key);
        let mut provider = with_server(agent, key);
        let set = provider.tileset().unwrap();
        assert!(!String::from_utf8_lossy(&set).contains(key));
        let parsed = Tileset::from_json(&set).unwrap();
        let uri = parsed
            .root
            .children
            .first()
            .unwrap()
            .content
            .as_ref()
            .unwrap()
            .uri
            .clone();
        let content = provider.content(&uri).unwrap();
        assert_eq!(content.cache_policy, CachePolicy::MemoryOnly);
        assert!(!content.attribution.is_empty());
        assert!(content.expires_at_unix_ms.is_some());
        assert!(!content.attribution.join(" ").contains(key));
        assert!(
            !content
                .bytes
                .windows(key.len())
                .any(|bytes| bytes == key.as_bytes())
        );
        let region = provider.world.region().unwrap();
        let decoded = super::super::decode_glb(
            &content.bytes,
            parsed.root.transform.unwrap(),
            [region.origin.latitude_deg, region.origin.longitude_deg],
        )
        .unwrap();
        assert!(decoded.terrain_bounds.is_some());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
    #[test]
    fn cap_auth_redirect_and_echoed_keys_are_redacted_and_stop_requests() {
        for status in [429, 402, 403, 498, 302] {
            let key = "never-print-this-key";
            let (agent, calls) = server(status, key.as_bytes().to_vec(), key);
            let mut provider = with_server(agent, key);
            let error = provider.request("/tile/12/1/1").unwrap_err();
            assert!(!error.contains(key));
            if status == 429 || status == 402 {
                assert!(error.contains("limit reached"));
            }
            // The stopped session must not ask the local server again.
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert_eq!(provider.request("/tile/12/1/1").unwrap_err(), error);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        }
    }
    #[test]
    fn json_cap_errors_with_http_200_stop_without_echoing_the_key() {
        let key = "json-error-secret";
        let (agent, calls) = server(
            200,
            format!(r#"{{"error":{{"code":429,"message":"{key}"}}}}"#).into_bytes(),
            key,
        );
        let mut provider = with_server(agent, key);
        let error = provider.request("/tile/12/1/1").unwrap_err();
        assert!(error.contains("limit reached"));
        assert!(!error.contains(key));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(provider.request("/tile/12/1/1").unwrap_err(), error);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn installed_new_york_region_fits_the_bounded_quadtree_without_network() {
        let region = flightcraft_world::Region::from_json(include_str!(
            "../../../../assets/terrain/new-york/region.json"
        ))
        .unwrap();
        let extent = f64::from(region.tile_size - 1) * f64::from(region.spacing_m);
        let width = region.tile_size as usize;
        let tiles = (0..region.tiles.len())
            .map(|index| {
                let north = index % region.tiles_north as usize;
                let east = index / region.tiles_north as usize;
                flightcraft_world::HeightTile::new(
                    [
                        region.southwest_m[0] + north as f64 * extent,
                        region.southwest_m[1] + east as f64 * extent,
                    ],
                    region.spacing_m,
                    region.tile_size,
                    vec![112.; width * width],
                    vec![flightcraft_world::Surface::Grass; (width - 1) * (width - 1)],
                )
                .unwrap()
            })
            .collect();
        let world = World::from_region(region, tiles).unwrap();
        let mut provider = EsriProvider::create("original-test-key".into(), world, ENDPOINT.into()).unwrap();
        let bytes = provider.tileset().unwrap();
        let set = Tileset::from_json(&bytes).unwrap();
        assert!(set.root.children.len() > 1);
        let [_, _, _, _, low, high] = set.root.bounding_volume.region.unwrap();
        assert!(low <= 111. && high >= 113. && high - low < 3.);
        assert!(bytes.len() <= tiles3d::MAX_TILESET_BYTES);
    }
    #[test]
    fn raw_transport_trace_logging_is_compiled_out() {
        assert!(log::STATIC_MAX_LEVEL < log::LevelFilter::Trace);
    }
    #[test]
    fn opt_in_and_endpoint_are_required_and_foreign_content_is_rejected() {
        let mut settings = Provider {
            location: Location::Url(ENDPOINT.into()),
            token_config_key: Some("esri-imagery".into()),
            enabled: false,
        };
        assert!(EsriProvider::new(&settings, "key".into(), world()).is_err());
        settings.enabled = true;
        settings.location = Location::Url("https://attacker.invalid/".into());
        assert!(EsriProvider::new(&settings, "key".into(), world()).is_err());
        let mut provider = EsriProvider::create("key".into(), world(), ENDPOINT.into()).unwrap();
        for uri in [
            "../secret",
            "imagery/255/0/0.glb",
            "imagery/12/0/0.glb",
            "https://invalid/tile",
        ] {
            assert!(provider.content(uri).is_err());
        }
    }
}
