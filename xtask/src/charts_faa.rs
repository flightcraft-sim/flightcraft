//! Local-only GeoTIFF reprojection. Equations independently follow Snyder, USGS PP 1395.
use flightcraft_control::map::{BaseMap, ChartIndex, ChartTile, inverse_mercator};
use flightcraft_world::{GeoPoint, Region, geo::LocalProjection};
use image::{Rgba, RgbaImage};
use std::{
    collections::BTreeMap,
    io::{Cursor, Read},
    path::PathBuf,
};
use tiff::{
    decoder::{Decoder, DecodingResult},
    tags::Tag,
};

pub fn run() -> Result<(), String> {
    run_args(std::env::args().skip(2))
}

pub(super) fn run_args(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let mut options = BTreeMap::new();
    while let Some(key) = args.next() {
        if ![
            "--region",
            "--geotiff",
            "--tfw",
            "--kind",
            "--out",
            "--min-zoom",
            "--max-zoom",
            "--neatline",
        ]
        .contains(&key.as_str())
        {
            return Err(format!("unknown charts-faa option: {key}"));
        }
        let value = args.next().ok_or("missing charts-faa option value")?;
        if options.insert(key, value).is_some() {
            return Err("duplicate charts-faa option".into());
        }
    }
    let get = |key: &str| options.get(key).ok_or_else(|| format!("missing {key}"));
    let region =
        Region::from_json(&read_text(&PathBuf::from(get("--region")?), 65536)?).map_err(|e| e.to_string())?;
    region.validate().map_err(|e| e.to_string())?;
    let kind = match get("--kind")?.as_str() {
        "sectional" => BaseMap::Sectional,
        "tac" => BaseMap::Tac,
        _ => return Err("kind must be sectional or tac".into()),
    };
    let min = options
        .get("--min-zoom")
        .map(|s| s.parse::<u8>())
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or(if kind == BaseMap::Tac { 9 } else { 8 });
    let max = options
        .get("--max-zoom")
        .map(|s| s.parse::<u8>())
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or(11);
    if min > max || max > 13 {
        return Err("zoom range must be ordered and at most 13".into());
    }
    let source = PathBuf::from(get("--geotiff")?);
    let mut bytes = Vec::new();
    std::fs::File::open(&source)
        .map_err(|e| e.to_string())?
        .take(256 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 256 * 1024 * 1024 {
        return Err("GeoTIFF exceeds 256 MiB".into());
    }
    let world = WorldFile::parse(&read_text(&PathBuf::from(get("--tfw")?), 4096)?)?;
    let mut chart = Chart::decode(bytes, world)?;
    // A verified geographic map-face extent takes precedence over image-based collar detection.
    if let Some(extent) = options.get("--neatline") {
        let v = extent
            .split(',')
            .map(|s| s.parse::<f64>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let [w, s, e, n] = v.as_slice() else {
            return Err("neatline is west,south,east,north".into());
        };
        if !v.iter().all(|v| v.is_finite())
            || !(-180.0..180.0).contains(w)
            || *e > 180.0
            || *s < -85.0
            || *n > 85.0
            || w >= e
            || s >= n
        {
            return Err("invalid neatline".into());
        }
        chart.neatline = Some([*w, *s, *e, *n]);
    }
    let metadata = read_text(&source.with_extension("htm"), 1024 * 1024)?;
    let edition = date(&metadata, "begdate")?;
    let expires = date(&metadata, "enddate")?;
    let output = PathBuf::from(get("--out")?);
    if output.exists() {
        return Err("chart output must be a new directory".into());
    }
    let index = generate(&region, &chart, kind, min, max, edition, expires, &output)?;
    // A compact source overview makes collar and projection checks reviewable without decoding tiles.
    let overview = RgbaImage::from_fn(1000, (1000 * chart.height / chart.width) as u32, |x, y| {
        let px = x as usize * chart.width / 1000;
        let py = y as usize * chart.width / 1000;
        chart
            .rgb(px, py)
            .map_or(Rgba([0, 0, 0, 0]), |[r, g, b]| Rgba([r, g, b, 255]))
    });
    overview
        .save(std::env::temp_dir().join(if kind == BaseMap::Tac {
            "flightcraft-tac-source.png"
        } else {
            "flightcraft-sectional-source.png"
        }))
        .map_err(|e| e.to_string())?;
    eprintln!(
        "charts-faa: {} tiles, {} bytes, z{}–z{}, edition {} to {}",
        index.tiles.len(),
        index.bytes,
        min,
        max,
        index.edition,
        index.expires
    );
    Ok(())
}
fn read_text(path: &std::path::Path, limit: usize) -> Result<String, String> {
    let mut text = String::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > limit {
        return Err("input text exceeds bound".into());
    }
    Ok(text)
}
fn date(metadata: &str, key: &str) -> Result<String, String> {
    let marker = format!("element-value {key}\">");
    let text = metadata
        .split(&marker)
        .nth(1)
        .and_then(|s| s.get(..8))
        .ok_or("missing chart edition metadata")?;
    let year = text.get(..4).ok_or("date year")?;
    let month = text.get(4..6).ok_or("date month")?;
    let day = text.get(6..8).ok_or("date day")?;
    let value = format!("{year}-{month}-{day}");
    if !flightcraft_world::navdata::iso_date(&value) {
        return Err("invalid chart edition date".into());
    }
    Ok(value)
}
#[derive(Clone, Copy)]
struct WorldFile {
    a: f64,
    d: f64,
    b: f64,
    e: f64,
    c: f64,
    f: f64,
}
impl WorldFile {
    fn parse(text: &str) -> Result<Self, String> {
        let values = text
            .split_whitespace()
            .map(|v| v.parse::<f64>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let [a, d, b, e, c, f] = values.as_slice() else {
            return Err("world file requires six numbers".into());
        };
        if values.iter().any(|v| !v.is_finite()) || (a * e - b * d).abs() < 1e-12 {
            return Err("singular world file".into());
        }
        Ok(Self {
            a: *a,
            d: *d,
            b: *b,
            e: *e,
            c: *c,
            f: *f,
        })
    }
    fn pixel(self, [x, y]: [f64; 2]) -> [f64; 2] {
        let det = self.a * self.e - self.b * self.d;
        [
            (self.e * (x - self.c) - self.b * (y - self.f)) / det,
            (-self.d * (x - self.c) + self.a * (y - self.f)) / det,
        ]
    }
}
struct Lambert {
    a: f64,
    eccentricity: f64,
    n: f64,
    factor: f64,
    rho0: f64,
    lon0: f64,
    false_e: f64,
    false_n: f64,
}
impl Lambert {
    fn from_keys(keys: &[u16], doubles: &[f64]) -> Result<Self, String> {
        if keys.get(..4).is_none_or(|h| h.first() != Some(&1))
            || keys.len() != 4 + usize::from(*keys.get(3).ok_or("GeoKey header")?) * 4
        {
            return Err("invalid GeoKey directory".into());
        }
        let mut values = BTreeMap::new();
        for entry in keys.get(4..).ok_or("GeoKeys")?.chunks_exact(4) {
            let [key, location, count, offset] = entry else {
                continue;
            };
            if *count != 1 {
                continue;
            }
            let value = match *location {
                0 => f64::from(*offset),
                34736 => *doubles.get(usize::from(*offset)).ok_or("GeoDouble offset")?,
                _ => continue,
            };
            if !value.is_finite() || values.insert(*key, value).is_some() {
                return Err("invalid duplicate GeoKey".into());
            }
        }
        let get = |k| {
            values
                .get(&k)
                .copied()
                .ok_or_else(|| format!("missing GeoKey {k}"))
        };
        if get(1024)? != 1.0 || get(3075)? != 8.0 || get(3076)? != 9001.0 {
            return Err("requires metric Lambert conformal conic 2SP GeoTIFF".into());
        }
        if get(2048)? != 4269.0 && get(2048)? != 4326.0 {
            return Err("requires NAD83 or WGS84 chart".into());
        }
        let a = get(2057)?;
        let invf = get(2059)?;
        if !(6_000_000.0..7_000_000.0).contains(&a) || !(250.0..350.0).contains(&invf) {
            return Err("invalid ellipsoid".into());
        }
        let f = 1.0 / invf;
        let eccentricity = (f * (2.0 - f)).sqrt();
        let (p1, p2) = (get(3078)?.to_radians(), get(3079)?.to_radians());
        let (lat0, lon0) = (get(3085)?.to_radians(), get(3084)?.to_radians());
        if [p1, p2, lat0]
            .iter()
            .any(|v| v.abs() >= core::f64::consts::FRAC_PI_2)
            || lon0.abs() > core::f64::consts::PI
        {
            return Err("invalid Lambert angles".into());
        }
        let m = |p: f64| p.cos() / (1.0 - eccentricity.powi(2) * p.sin().powi(2)).sqrt();
        let t = |p| conformal_t(p, eccentricity);
        let n = if (p1 - p2).abs() < 1e-12 {
            p1.sin()
        } else {
            (m(p1) / m(p2)).ln() / (t(p1) / t(p2)).ln()
        };
        if !n.is_finite() || n <= 0.0 {
            return Err("unsupported Lambert cone".into());
        }
        let factor = m(p1) / (n * t(p1).powf(n));
        Ok(Self {
            a,
            eccentricity,
            n,
            factor,
            rho0: a * factor * t(lat0).powf(n),
            lon0,
            false_e: get(3086)?,
            false_n: get(3087)?,
        })
    }
    fn project(&self, p: GeoPoint) -> [f64; 2] {
        let rho =
            self.a * self.factor * conformal_t(p.latitude_deg.to_radians(), self.eccentricity).powf(self.n);
        let theta = self.n * (p.longitude_deg.to_radians() - self.lon0);
        [
            self.false_e + rho * theta.sin(),
            self.false_n + self.rho0 - rho * theta.cos(),
        ]
    }
}
fn conformal_t(phi: f64, e: f64) -> f64 {
    (core::f64::consts::FRAC_PI_4 - phi * 0.5).tan()
        / ((1.0 - e * phi.sin()) / (1.0 + e * phi.sin())).powf(e * 0.5)
}
struct Chart {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
    channels: usize,
    palette: Option<Vec<u16>>,
    projection: Lambert,
    world: WorldFile,
    mask: Vec<bool>,
    mask_width: usize,
    neatline: Option<[f64; 4]>,
}
impl Chart {
    fn decode(mut bytes: Vec<u8>, world: WorldFile) -> Result<Self, String> {
        let mut reader = Decoder::new(Cursor::new(&bytes)).map_err(|e| e.to_string())?;
        let (w, h) = reader.dimensions().map_err(|e| e.to_string())?;
        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 240_000_000 {
            return Err("chart dimensions exceed 240 million pixels".into());
        }
        let keys = reader
            .get_tag_u16_vec(Tag::Unknown(34735))
            .map_err(|e| e.to_string())?;
        let doubles = reader
            .get_tag_f64_vec(Tag::Unknown(34736))
            .map_err(|e| e.to_string())?;
        let projection = Lambert::from_keys(&keys, &doubles)?;
        let palette = if reader
            .get_tag_unsigned::<u16>(Tag::PhotometricInterpretation)
            .map_err(|e| e.to_string())?
            == 3
        {
            let palette = reader.get_tag_u16_vec(Tag::ColorMap).map_err(|e| e.to_string())?;
            if palette.len() != 768 {
                return Err("requires an eight-bit chart palette".into());
            }
            Some(palette)
        } else {
            None
        };
        drop(reader);
        // tiff decodes the compressed index plane as Gray8; the original palette is applied below.
        // Only this private in-memory IFD is changed. The source file is always read-only.
        if palette.is_some() {
            palette_as_gray(&mut bytes)?;
        }
        let mut reader = Decoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
        let channels = match reader.colortype().map_err(|e| e.to_string())? {
            tiff::ColorType::Gray(8) if palette.is_some() => 1,
            tiff::ColorType::RGB(8) => 3,
            _ => return Err("requires palette8 or RGB8 GeoTIFF".into()),
        };
        if u64::from(w) * u64::from(h) * channels as u64 > 256 * 1024 * 1024 {
            return Err("decoded chart exceeds 256 MiB".into());
        }
        let DecodingResult::U8(pixels) = reader.read_image().map_err(|e| e.to_string())? else {
            return Err("requires eight-bit samples".into());
        };
        let mut chart = Self {
            width: w as usize,
            height: h as usize,
            pixels,
            channels,
            palette,
            projection,
            world,
            mask: Vec::new(),
            mask_width: 0,
            neatline: None,
        };
        chart.detect_face();
        Ok(chart)
    }
    fn rgb(&self, x: usize, y: usize) -> Option<[u8; 3]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let offset = (y * self.width + x) * self.channels;
        let v = *self.pixels.get(offset)?;
        if let Some(p) = &self.palette {
            let i = usize::from(v);
            Some([
                (*p.get(i)? >> 8) as u8,
                (*p.get(i + 256)? >> 8) as u8,
                (*p.get(i + 512)? >> 8) as u8,
            ])
        } else {
            Some([v, *self.pixels.get(offset + 1)?, *self.pixels.get(offset + 2)?])
        }
    }
    fn detect_face(&mut self) {
        let step = 32;
        let (w, h) = (self.width.div_ceil(step), self.height.div_ceil(step));
        let mut colored = vec![false; w * h];
        for y in 0..h {
            for x in 0..w {
                let mut count = 0;
                for dy in [4, 12, 20, 28] {
                    for dx in [4, 12, 20, 28] {
                        if let Some([r, g, b]) = self.rgb(x * step + dx, y * step + dy)
                            && r.max(g).max(b) - r.min(g).min(b) > 18
                            && r.max(g).max(b) > 80
                        {
                            count += 1;
                        }
                    }
                }
                if let Some(v) = colored.get_mut(y * w + x) {
                    *v = count >= 7;
                }
            }
        }
        self.mask = face_mask(&colored, w, h);
        self.mask_width = w;
    }
    fn sample(&self, p: GeoPoint) -> Rgba<u8> {
        if let Some([w, s, e, n]) = self.neatline
            && (p.longitude_deg < w || p.longitude_deg > e || p.latitude_deg < s || p.latitude_deg > n)
        {
            return Rgba([0, 0, 0, 0]);
        }
        let [x, y] = self.world.pixel(self.projection.project(p));
        if !x.is_finite()
            || !y.is_finite()
            || x < 0.0
            || y < 0.0
            || x >= self.width as f64
            || y >= self.height as f64
        {
            return Rgba([0, 0, 0, 0]);
        }
        let (x, y) = (x.round() as usize, y.round() as usize);
        if self.neatline.is_none()
            && !self
                .mask
                .get((y / 32) * self.mask_width + x / 32)
                .copied()
                .unwrap_or(false)
        {
            return Rgba([0, 0, 0, 0]);
        }
        self.rgb(x, y)
            .map_or(Rgba([0, 0, 0, 0]), |[r, g, b]| Rgba([r, g, b, 255]))
    }
}
/// Select the largest coloured map-face component, then fill its enclosed label holes.
fn face_mask(colored: &[bool], w: usize, h: usize) -> Vec<bool> {
    if w == 0 || h == 0 || w.checked_mul(h) != Some(colored.len()) {
        return Vec::new();
    }
    let mut seen = vec![false; colored.len()];
    let mut largest = Vec::new();
    for start in 0..colored.len() {
        if !colored.get(start).copied().unwrap_or(false) || seen.get(start).copied().unwrap_or(true) {
            continue;
        }
        let mut component = vec![start];
        if let Some(v) = seen.get_mut(start) {
            *v = true;
        }
        let mut cursor = 0;
        while let Some(i) = component.get(cursor).copied() {
            cursor += 1;
            for next in neighbors(i, w, h) {
                if colored.get(next) == Some(&true) && seen.get(next) == Some(&false) {
                    if let Some(v) = seen.get_mut(next) {
                        *v = true;
                    }
                    component.push(next);
                }
            }
        }
        if component.len() > largest.len() {
            largest = component;
        }
    }
    let mut mask = vec![false; colored.len()];
    for i in largest {
        if let Some(v) = mask.get_mut(i) {
            *v = true;
        }
    }
    let mut outside = vec![false; mask.len()];
    let mut flood: Vec<usize> = (0..mask.len())
        .filter(|i| {
            (*i < w || *i >= w * (h - 1) || i % w == 0 || i % w == w - 1) && mask.get(*i) == Some(&false)
        })
        .collect();
    for i in &flood {
        if let Some(v) = outside.get_mut(*i) {
            *v = true;
        }
    }
    let mut cursor = 0;
    while let Some(i) = flood.get(cursor).copied() {
        cursor += 1;
        for next in neighbors(i, w, h) {
            if mask.get(next) == Some(&false) && outside.get(next) == Some(&false) {
                if let Some(v) = outside.get_mut(next) {
                    *v = true;
                }
                flood.push(next);
            }
        }
    }
    mask.iter_mut().zip(outside).for_each(|(m, o)| *m = !o);
    mask
}
fn neighbors(i: usize, w: usize, h: usize) -> impl Iterator<Item = usize> {
    [
        i.checked_sub(w),
        (i + w < w * h).then_some(i + w),
        (!i.is_multiple_of(w)).then(|| i - 1),
        (i % w + 1 < w).then_some(i + 1),
    ]
    .into_iter()
    .flatten()
}
fn palette_as_gray(bytes: &mut [u8]) -> Result<(), String> {
    let little = match bytes.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return Err("TIFF byte order".into()),
    };
    let u16_at = |b: &[u8], i| -> Result<u16, String> {
        let v: [u8; 2] = b
            .get(i..i + 2)
            .ok_or("short TIFF")?
            .try_into()
            .map_err(|_| "short TIFF")?;
        Ok(if little {
            u16::from_le_bytes(v)
        } else {
            u16::from_be_bytes(v)
        })
    };
    if u16_at(bytes, 2)? != 42 {
        return Err("requires classic TIFF".into());
    }
    let v: [u8; 4] = bytes
        .get(4..8)
        .ok_or("TIFF offset")?
        .try_into()
        .map_err(|_| "TIFF offset")?;
    let offset = if little {
        u32::from_le_bytes(v)
    } else {
        u32::from_be_bytes(v)
    } as usize;
    let count = u16_at(bytes, offset)?;
    for i in 0..usize::from(count) {
        let entry = offset + 2 + i * 12;
        if u16_at(bytes, entry)? == 262 {
            if u16_at(bytes, entry + 2)? != 3 {
                return Err("palette tag type".into());
            }
            let value = if little {
                1_u16.to_le_bytes()
            } else {
                1_u16.to_be_bytes()
            };
            bytes
                .get_mut(entry + 8..entry + 10)
                .ok_or("palette tag value")?
                .copy_from_slice(&value);
            return Ok(());
        }
    }
    Err("missing palette tag".into())
}
#[allow(clippy::too_many_arguments)]
fn generate(
    region: &Region,
    chart: &Chart,
    kind: BaseMap,
    min: u8,
    max: u8,
    edition: String,
    expires: String,
    output: &std::path::Path,
) -> Result<ChartIndex, String> {
    let projection = LocalProjection::new(region.origin.latitude_deg, region.origin.longitude_deg)
        .map_err(|e| e.to_string())?;
    let [sn, se] = region.southwest_m;
    let margin = 40.0 * 1852.0;
    let (sn, se) = (sn - margin, se - margin);
    let span = f64::from(region.tile_size - 1) * f64::from(region.spacing_m);
    let (nn, ee) = (
        sn + span * f64::from(region.tiles_north) + 2.0 * margin,
        se + span * f64::from(region.tiles_east) + 2.0 * margin,
    );
    let corners = [[sn, se], [sn, ee], [nn, se], [nn, ee]]
        .into_iter()
        .map(|[n, e]| {
            projection
                .unproject(n, e)
                .map(|[latitude_deg, longitude_deg]| GeoPoint {
                    latitude_deg,
                    longitude_deg,
                })
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    std::fs::create_dir_all(output).map_err(|e| e.to_string())?;
    let mut index = ChartIndex {
        coverage_margin_nm: 40.0,
        version: 2,
        kind,
        edition,
        expires,
        attribution: "FAA VFR raster charts, public domain (US government work); not for navigation".into(),
        min_zoom: min,
        max_zoom: max,
        tile_size: 256,
        tiles: Vec::new(),
        bytes: 0,
    };
    for z in min..=max {
        let tiles: Vec<_> = corners.iter().filter_map(|p| ChartTile::at(*p, z)).collect();
        let (x0, x1, y0, y1) = (
            tiles.iter().map(|t| t.x).min().ok_or("tile extent")?,
            tiles.iter().map(|t| t.x).max().ok_or("tile extent")?,
            tiles.iter().map(|t| t.y).min().ok_or("tile extent")?,
            tiles.iter().map(|t| t.y).max().ok_or("tile extent")?,
        );
        if u64::from(x1 - x0 + 1) * u64::from(y1 - y0 + 1) > 12000 {
            return Err("too many chart tiles".into());
        }
        let size = f64::from(1_u32 << z) * 256.0;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let mut visible = false;
                let image = RgbaImage::from_fn(256, 256, |px, py| {
                    let Some(p) = inverse_mercator([
                        (f64::from(x) * 256.0 + f64::from(px) + 0.5) / size,
                        (f64::from(y) * 256.0 + f64::from(py) + 0.5) / size,
                    ]) else {
                        return Rgba([0, 0, 0, 0]);
                    };
                    let Ok(local) = projection.project(p.latitude_deg, p.longitude_deg) else {
                        return Rgba([0, 0, 0, 0]);
                    };
                    let [north, east] = local;
                    if north < sn || north > nn || east < se || east > ee {
                        return Rgba([0, 0, 0, 0]);
                    }
                    let color = chart.sample(p);
                    visible |= color.0.get(3).copied().unwrap_or(0) > 0;
                    color
                });
                if !visible {
                    continue;
                }
                let tile = ChartTile { z, x, y };
                let path = output.join(index.tile_path(tile));
                std::fs::create_dir_all(path.parent().ok_or("tile parent")?).map_err(|e| e.to_string())?;
                image.save(&path).map_err(|e| e.to_string())?;
                index.bytes += std::fs::metadata(path).map_err(|e| e.to_string())?.len();
                if index.bytes > 65 * 1024 * 1024 {
                    return Err("chart exceeds 65 MiB; lower maximum zoom".into());
                }
                index.tiles.push(tile);
            }
        }
    }
    if index.tiles.is_empty() {
        return Err("no chart pixels intersect region".into());
    }
    let bytes = serde_json::to_vec_pretty(&index).map_err(|e| e.to_string())?;
    ChartIndex::from_json(&bytes)?;
    std::fs::write(output.join("index.json"), bytes).map_err(|e| e.to_string())?;
    Ok(index)
}
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    #[test]
    fn collar_mask_excludes_legend_and_fills_labels() {
        let mut image = vec![false; 20 * 16];
        for y in 2..14 {
            for x in 5..18 {
                image[y * 20 + x] = true;
            }
        }
        image[6 * 20 + 10] = false;
        for y in 2..5 {
            for x in 0..3 {
                image[y * 20 + x] = true;
            }
        }
        let mask = face_mask(&image, 20, 16);
        assert!(mask[6 * 20 + 10]);
        assert!(!mask[3 * 20 + 1]);
        assert!(!mask[15 * 20 + 10]);
    }
    fn keys() -> (Vec<u16>, Vec<f64>) {
        let values = [
            (1024, 1.),
            (2048, 4269.),
            (3075, 8.),
            (3076, 9001.),
            (2057, 6378137.),
            (2059, 298.257222101),
            (3078, 45.),
            (3079, 33.),
            (3084, -74.),
            (3085, 40.74166666666667),
            (3086, 0.),
            (3087, 0.),
        ];
        let mut keys = vec![1, 1, 0, values.len() as u16];
        let mut doubles = Vec::new();
        for (key, value) in values {
            keys.extend([key, 34736, 1, doubles.len() as u16]);
            doubles.push(value);
        }
        (keys, doubles)
    }
    #[test]
    fn synthetic_geotiff_decodes_georeference_and_pixels() {
        let (keys, doubles) = keys();
        let mut stream = Cursor::new(Vec::new());
        {
            let mut encoder = tiff::encoder::TiffEncoder::new(&mut stream).unwrap();
            let mut image = encoder
                .new_image::<tiff::encoder::colortype::RGB8>(64, 64)
                .unwrap();
            image
                .encoder()
                .write_tag(Tag::Unknown(34735), keys.as_slice())
                .unwrap();
            image
                .encoder()
                .write_tag(Tag::Unknown(34736), doubles.as_slice())
                .unwrap();
            image.write_data(&vec![128; 64 * 64 * 3]).unwrap();
        }
        let world = WorldFile::parse("100\n0\n0\n-100\n-3200\n3200").unwrap();
        let mut chart = Chart::decode(stream.into_inner(), world).unwrap();
        chart.neatline = Some([-75., 40., -73., 42.]);
        let p = GeoPoint {
            latitude_deg: 40.74166666666667,
            longitude_deg: -74.,
        };
        let [x, y] = chart.projection.project(p);
        assert!(x.abs() < 1e-8 && y.abs() < 1e-8);
        assert_eq!(chart.sample(p), Rgba([128, 128, 128, 255]));
        assert_eq!(
            chart.sample(GeoPoint {
                latitude_deg: 39.,
                longitude_deg: -74.
            }),
            Rgba([0, 0, 0, 0])
        );
        let mut region =
            Region::from_json(include_str!("../../assets/terrain/new-york/region.json")).unwrap();
        region.origin.latitude_deg = p.latitude_deg;
        region.origin.longitude_deg = p.longitude_deg;
        region.southwest_m = [-1000., -1000.];
        region.tiles_north = 1;
        region.tiles_east = 1;
        region.tile_size = 41;
        region.spacing_m = 50.;
        let directory = std::env::temp_dir().join(format!("flightcraft-geotiff-test-{}", std::process::id()));
        let index = generate(
            &region,
            &chart,
            BaseMap::Tac,
            10,
            10,
            "2026-10-29".into(),
            "2026-12-23".into(),
            &directory,
        )
        .unwrap();
        assert!(!index.tiles.is_empty());
        assert!(index.bytes > 0);
        assert_eq!(index.coverage_margin_nm, 40.);
        let beyond_grid = p.destination(270., 2500.);
        let tile = ChartTile::at(beyond_grid, 10).unwrap();
        let [mx, my] = flightcraft_control::mercator(beyond_grid).unwrap();
        let image = image::open(directory.join(index.tile_path(tile)))
            .unwrap()
            .into_rgba8();
        let pixel = image.get_pixel(
            ((mx * 1024.).fract() * 256.) as u32,
            ((my * 1024.).fract() * 256.) as u32,
        );
        assert_eq!(
            pixel.0[3], 255,
            "chart coverage must survive beyond the terrain grid"
        );
        let decoded = image::open(directory.join(index.tile_path(index.tiles[0]))).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (256, 256));
        ChartIndex::from_json(&std::fs::read(directory.join("index.json")).unwrap()).unwrap();
        std::fs::remove_dir_all(directory).unwrap();
        assert!(Chart::decode(vec![0; 32], world).is_err());
        assert!(WorldFile::parse("0 0 0 0 0 0").is_err());
        assert!(Lambert::from_keys(&[1, 1, 0, 99], &[]).is_err());
    }
}
