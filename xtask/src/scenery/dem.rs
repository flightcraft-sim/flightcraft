//! Bounded, windowed reading of USGS float GeoTIFF/COG DEMs. Never decode an entire project.
use super::{InputKind, Request, Sources, download, transport};

use std::{fs::File, path::Path};
use tiff::{
    decoder::{ChunkType, Decoder, DecodingResult, Limits},
    tags::Tag,
};

struct Dem {
    decoder: Decoder<File>,
    width: u32,
    height: u32,
    chunk_width: u32,
    chunk_height: u32,
    scale: [f64; 2],
    tie: [f64; 4],
    point: bool,
    geographic: bool,
    nodata: Option<f32>,
    cache: std::collections::BTreeMap<u32, (u32, Vec<f32>)>,
}
pub(super) struct Dems {
    dems: Vec<Dem>,
}
impl Dems {
    pub fn open(request: &Request, sources: &Sources) -> Result<Self, String> {
        let mut dems = Vec::new();
        for input in sources.inputs.iter().filter(|i| i.kind == InputKind::Dem) {
            if dems.len() >= 256 {
                return Err("too many DEMs for one area".into());
            }
            let path = request
                .cache
                .join("objects")
                .join(download::sha256(input.source.url.as_bytes()));
            let (bytes, sha256) = transport::digest(&path)?;
            if bytes != input.source.bytes || sha256 != input.source.sha256 {
                return Err("DEM differs from source receipt".into());
            }
            dems.push(Dem::open(&path)?);
        }
        if dems.is_empty() {
            return Err("no verified 1 m DEM inputs; run scenery fetch first".into());
        }
        Ok(Self { dems })
    }
    pub fn height(&mut self, lat: f64, lon: f64) -> Result<f64, String> {
        let cached: usize = self
            .dems
            .iter()
            .flat_map(|dem| dem.cache.values())
            .map(|(_, data)| data.len() * 4)
            .sum();
        if cached > 64 * 1024 * 1024 {
            for dem in &mut self.dems {
                dem.cache.clear();
            }
        }
        for dem in &mut self.dems {
            if let Some(value) = dem.height(lat, lon)? {
                return Ok(value);
            }
        }
        Err(format!(
            "1 m DEM coverage or valid samples missing at {lat:.8},{lon:.8}"
        ))
    }
}
impl Dem {
    fn open(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| e.to_string())?;
        let mut limits = Limits::default();
        limits.decoding_buffer_size = 16 * 1024 * 1024;
        limits.intermediate_buffer_size = 32 * 1024 * 1024;
        limits.ifd_value_size = 4 * 1024 * 1024;
        let mut decoder = Decoder::new(file).map_err(|e| e.to_string())?.with_limits(limits);
        let (width, height) = decoder.dimensions().map_err(|e| e.to_string())?;
        if width < 2
            || height < 2
            || width > 100_000
            || height > 100_000
            || decoder.colortype().map_err(|e| e.to_string())? != tiff::ColorType::Gray(32)
        {
            return Err("DEM must be a bounded single-band float32 raster".into());
        }
        if decoder.get_tag_f64_vec(Tag::ModelTransformationTag).is_ok()
            || decoder
                .get_tag_u16_vec(Tag::Orientation)
                .is_ok_and(|v| v.as_slice() != [1])
        {
            return Err("DEM must be north-up without a second affine transform".into());
        }
        let scale = decoder
            .get_tag_f64_vec(Tag::ModelPixelScaleTag)
            .map_err(|e| e.to_string())?;
        let tie = decoder
            .get_tag_f64_vec(Tag::ModelTiepointTag)
            .map_err(|e| e.to_string())?;
        let [sx, sy, _] = scale.as_slice() else {
            return Err("DEM needs three pixel scales".into());
        };
        let [i, j, _, x, y, _] = tie.as_slice() else {
            return Err("DEM needs one tie point".into());
        };
        if scale.iter().chain(&tie).any(|v| !v.is_finite())
            || *sx <= 0.
            || *sy <= 0.
            || *sx > 100.
            || *sy > 100.
        {
            return Err("invalid DEM geotransform".into());
        }
        let keys = decoder
            .get_tag_u16_vec(Tag::GeoKeyDirectoryTag)
            .map_err(|e| e.to_string())?;
        let [1, 1, _, count] = keys.get(..4).ok_or("DEM GeoKey header")? else {
            return Err("unsupported DEM GeoKeys".into());
        };
        if keys.len() != 4 + usize::from(*count) * 4 {
            return Err("DEM GeoKey length mismatch".into());
        }
        let mut values = std::collections::BTreeMap::new();
        for entry in keys.get(4..).ok_or("DEM GeoKeys")?.chunks_exact(4) {
            let [key, location, count, value] = entry else {
                return Err("GeoKey size".into());
            };
            if *location == 0 && *count == 1 && values.insert(*key, *value).is_some() {
                return Err("duplicate DEM GeoKey".into());
            }
        }
        let projected = values.get(&3072).copied();
        let geographic = projected.is_none() && matches!(values.get(&2048), Some(4326 | 4269 | 6318));
        if !geographic && !matches!(projected, Some(32618 | 26918 | 6347)) {
            return Err("initial DEM profile supports WGS84/NAD83 (including 2011) geographic or UTM zone 18 north; inspect this product's CRS".into());
        }
        if values.get(&4099).is_some_and(|v| *v != 9001) || values.get(&3076).is_some_and(|v| *v != 9001) {
            return Err("DEM vertical and projected units must be meters".into());
        }
        let point = match values.get(&1025) {
            Some(1) | None => false,
            Some(2) => true,
            _ => return Err("unknown DEM raster registration".into()),
        };
        let nodata = decoder
            .get_tag_ascii_string(Tag::GdalNodata)
            .ok()
            .map(|s| s.trim_matches('\0').trim().parse::<f32>())
            .transpose()
            .map_err(|e| e.to_string())?;
        let (chunk_width, chunk_height) = decoder.chunk_dimensions();
        if chunk_width == 0
            || chunk_height == 0
            || u64::from(chunk_width) * u64::from(chunk_height) > 4 * 1024 * 1024
        {
            return Err("DEM chunk exceeds 16 MiB".into());
        }
        let _ = match decoder.get_chunk_type() {
            ChunkType::Tile => decoder.tile_count(),
            ChunkType::Strip => decoder.strip_count(),
        }
        .map_err(|e| e.to_string())?;
        Ok(Self {
            decoder,
            width,
            height,
            chunk_width,
            chunk_height,
            scale: [*sx, *sy],
            tie: [*i, *j, *x, *y],
            point,
            geographic,
            nodata,
            cache: std::collections::BTreeMap::new(),
        })
    }
    fn sample(&mut self, x: u32, y: u32) -> Result<Option<f32>, String> {
        let chunk = (y / self.chunk_height)
            .checked_mul(self.width.div_ceil(self.chunk_width))
            .and_then(|n| n.checked_add(x / self.chunk_width))
            .ok_or("DEM chunk address overflow")?;
        if !self.cache.contains_key(&chunk) {
            if self.cache.len() >= 4 {
                self.cache.clear();
            }
            let (w, _) = self.decoder.chunk_data_dimensions(chunk);
            let DecodingResult::F32(data) = self.decoder.read_chunk(chunk).map_err(|e| e.to_string())? else {
                return Err("DEM samples must be float32".into());
            };
            self.cache.insert(chunk, (w, data));
        }
        let (w, data) = self.cache.get(&chunk).ok_or("DEM chunk not cached")?;
        let index = (y % self.chunk_height) as usize * (*w as usize) + (x % self.chunk_width) as usize;
        let value = *data.get(index).ok_or("DEM chunk sample out of bounds")?;
        if !value.is_finite() || !(-500.0..=9000.).contains(&value) || self.nodata == Some(value) {
            return Ok(None);
        }
        Ok(Some(value))
    }
    fn height(&mut self, lat: f64, lon: f64) -> Result<Option<f64>, String> {
        let [x, y] = if self.geographic {
            [lon, lat]
        } else {
            let [n, e] = flightcraft_world::geo::utm(lat, lon, 18).map_err(|e| e.to_string())?;
            [e, n]
        };
        let [sx, sy] = self.scale;
        let [i, j, tx, ty] = self.tie;
        let shift = if self.point { 0. } else { 0.5 };
        let px = (x - tx) / sx + i - shift;
        let py = (ty - y) / sy + j - shift;
        if px < 0. || py < 0. || px >= f64::from(self.width - 1) || py >= f64::from(self.height - 1) {
            return Ok(None);
        }
        let (ix, iy) = (px.floor() as u32, py.floor() as u32);
        let (Some(a), Some(b), Some(c), Some(d)) = (
            self.sample(ix, iy)?,
            self.sample(ix + 1, iy)?,
            self.sample(ix, iy + 1)?,
            self.sample(ix + 1, iy + 1)?,
        ) else {
            return Ok(None);
        };
        let (u, v) = (px - f64::from(ix), py - f64::from(iy));
        Ok(Some(
            (f64::from(a) * (1. - u) + f64::from(b) * u) * (1. - v)
                + (f64::from(c) * (1. - u) + f64::from(d) * u) * v,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiff::encoder::{TiffEncoder, colortype::Gray32Float};
    #[test]
    fn nad83_2011_utm18_metadata_uses_projected_meter_coordinates() {
        let path = std::env::temp_dir().join(format!("fc-scenery-utm-dem-{}.tif", std::process::id()));
        let mut file = File::create(&path).unwrap();
        let mut encoder = TiffEncoder::new(&mut file).unwrap();
        let mut image = encoder.new_image::<Gray32Float>(4, 4).unwrap();
        let [north, east] = flightcraft_world::geo::utm(41., -75., 18).unwrap();
        image
            .encoder()
            .write_tag(Tag::ModelPixelScaleTag, &[1_f64, 1., 0.][..])
            .unwrap();
        image
            .encoder()
            .write_tag(
                Tag::ModelTiepointTag,
                &[0_f64, 0., 0., east - 0.5, north + 0.5, 0.][..],
            )
            .unwrap();
        image
            .encoder()
            .write_tag(
                Tag::GeoKeyDirectoryTag,
                &[
                    1_u16, 1, 0, 5, 1024, 0, 1, 1, 1025, 0, 1, 1, 2048, 0, 1, 6318, 3072, 0, 1, 6347, 4099,
                    0, 1, 9001,
                ][..],
            )
            .unwrap();
        image.write_data(&[100.; 16]).unwrap();
        drop(file);
        let mut dem = Dem::open(&path).unwrap();
        assert!(!dem.geographic);
        assert_eq!(dem.height(41., -75.).unwrap(), Some(100.));
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn pixel_area_registration_bilinear_and_nodata() {
        let path = std::env::temp_dir().join(format!("fc-scenery-dem-{}.tif", std::process::id()));
        let mut file = File::create(&path).unwrap();
        let mut encoder = TiffEncoder::new(&mut file).unwrap();
        let mut image = encoder.new_image::<Gray32Float>(4, 4).unwrap();
        image
            .encoder()
            .write_tag(Tag::ModelPixelScaleTag, &[0.00001_f64, 0.00001, 0.][..])
            .unwrap();
        image
            .encoder()
            .write_tag(Tag::ModelTiepointTag, &[0_f64, 0., 0., -75., 41., 0.][..])
            .unwrap();
        image
            .encoder()
            .write_tag(
                Tag::GeoKeyDirectoryTag,
                &[1_u16, 1, 0, 3, 1024, 0, 1, 2, 1025, 0, 1, 1, 2048, 0, 1, 4326][..],
            )
            .unwrap();
        image.encoder().write_tag(Tag::GdalNodata, "-9999").unwrap();
        let data: Vec<f32> = (0..16)
            .map(|v| if v == 15 { -9999. } else { 100. + v as f32 })
            .collect();
        image.write_data(&data).unwrap();
        drop(file);
        let mut dem = Dem::open(&path).unwrap();
        assert!((dem.height(41. - 0.000005, -75. + 0.000005).unwrap().unwrap() - 100.).abs() < 1e-7);
        assert!((dem.height(41. - 0.00001, -75. + 0.00001).unwrap().unwrap() - 102.5).abs() < 1e-7);
        assert!(dem.height(42., -75.).unwrap().is_none());
        assert!(dem.height(41. - 0.00003, -75. + 0.00003).unwrap().is_none());
        std::fs::remove_file(path).unwrap();
    }
}
