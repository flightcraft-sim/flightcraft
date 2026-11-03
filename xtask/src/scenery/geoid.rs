//! Windowed reader for the NGS binary grid: four f64s, three i32s, south-first f32 rows.
use flightcraft_world::geoid::{GeoidGrid, GeoidModel, MAX_GEOID_SIDE};
use std::io::{Read, Seek, SeekFrom};
fn four(bytes: &[u8], offset: usize) -> Result<[u8; 4], String> {
    bytes
        .get(offset..offset + 4)
        .ok_or("truncated NGS integer")?
        .try_into()
        .map_err(|_| "NGS integer".into())
}
fn eight(bytes: &[u8], offset: usize) -> Result<[u8; 8], String> {
    bytes
        .get(offset..offset + 8)
        .ok_or("truncated NGS double")?
        .try_into()
        .map_err(|_| "NGS double".into())
}
#[derive(Clone, Copy)]
struct Header {
    south: f64,
    west: f64,
    dy: f64,
    dx: f64,
    rows: usize,
    cols: usize,
    little: bool,
}
impl Header {
    fn require_conus_coverage(&self) -> Result<(), String> {
        let north = self.south + (self.rows - 1) as f64 * self.dy;
        let east = self.west + (self.cols - 1) as f64 * self.dx;
        // A conservative envelope of the contiguous United States. u0 identifies CONUS,
        // but the actual binary registration must establish coverage before it is used.
        if self.west > -125. || self.south > 24. || east < -66. || north < 50. {
            return Err("GEOID18 g2018u0 header does not cover CONUS".into());
        }
        Ok(())
    }
    fn parse(bytes: &[u8], length: u64, little: bool) -> Result<Self, String> {
        let d = |offset| -> Result<f64, String> {
            let b = eight(bytes, offset)?;
            Ok(if little {
                f64::from_le_bytes(b)
            } else {
                f64::from_be_bytes(b)
            })
        };
        let i = |offset| -> Result<i32, String> {
            let b = four(bytes, offset)?;
            Ok(if little {
                i32::from_le_bytes(b)
            } else {
                i32::from_be_bytes(b)
            })
        };
        if i(40)? != 1 {
            return Err("NGS grid needs IEEE float32 samples".into());
        }
        let rows = i(32)?;
        let cols = i(36)?;
        if !(2..=20000).contains(&rows) || !(2..=40000).contains(&cols) {
            return Err("NGS dimensions exceed limits".into());
        }
        let raw_west = d(8)?;
        if !raw_west.is_finite() || !(-180.0..=360.0).contains(&raw_west) {
            return Err("invalid NGS west longitude".into());
        }
        let result = Self {
            south: d(0)?,
            west: (raw_west + 180.).rem_euclid(360.) - 180.,
            dy: d(16)?,
            dx: d(24)?,
            rows: rows as usize,
            cols: cols as usize,
            little,
        };
        if [result.south, result.west, result.dy, result.dx]
            .iter()
            .any(|v| !v.is_finite())
            || (result.dy - 1. / 60.).abs() > 1e-10
            || (result.dx - 1. / 60.).abs() > 1e-10
            || result.south < -90.
            || result.south + (result.rows - 1) as f64 * result.dy > 90.
            || result.west + (result.cols - 1) as f64 * result.dx > 180.
        {
            return Err("invalid GEOID18 registration".into());
        }
        let cells = (rows as u64)
            .checked_mul(cols as u64)
            .ok_or("NGS count overflow")?;
        let expected = cells
            .checked_mul(4)
            .and_then(|n| n.checked_add(44))
            .ok_or("NGS length overflow")?;
        if expected != length || expected > 512 * 1024 * 1024 {
            return Err("NGS file length or sample count mismatch".into());
        }
        Ok(result)
    }
}
fn read_header(reader: &mut (impl Read + Seek)) -> Result<Header, String> {
    let length = reader.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
    reader.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut bytes = [0_u8; 44];
    reader.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    Header::parse(&bytes, length, true).or_else(|_| Header::parse(&bytes, length, false))
}
pub fn subset(
    reader: &mut (impl Read + Seek),
    bounds: [f64; 4],
    source_sha256: String,
) -> Result<GeoidGrid, String> {
    let h = read_header(reader)?;
    let [w, s, e, n] = bounds;
    if bounds.iter().any(|v| !v.is_finite()) || w >= e || s >= n {
        return Err("invalid geoid subset bounds".into());
    }
    let x0 = (w - h.west) / h.dx;
    let y0 = (s - h.south) / h.dy;
    let x1 = (e - h.west) / h.dx;
    let y1 = (n - h.south) / h.dy;
    if x0 < 0. || y0 < 0. || x1 > (h.cols - 1) as f64 || y1 > (h.rows - 1) as f64 {
        return Err("GEOID18 file does not cover region".into());
    }
    let col0 = (libm::floor(x0) as usize).saturating_sub(2);
    let row0 = (libm::floor(y0) as usize).saturating_sub(2);
    let col1 = ((libm::ceil(x1) as usize) + 2).min(h.cols - 1);
    let row1 = ((libm::ceil(y1) as usize) + 2).min(h.rows - 1);
    let cols = col1 - col0 + 1;
    let rows = row1 - row0 + 1;
    if cols > MAX_GEOID_SIDE || rows > MAX_GEOID_SIDE {
        return Err("geoid subset exceeds region budget".into());
    }
    let mut values = Vec::with_capacity(rows * cols);
    let mut row_bytes = vec![0_u8; cols * 4];
    for row in row0..=row1 {
        let offset = 44 + ((row * h.cols + col0) as u64) * 4;
        reader.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
        reader.read_exact(&mut row_bytes).map_err(|e| e.to_string())?;
        for b in row_bytes.as_chunks::<4>().0.iter().copied() {
            values.push(if h.little {
                f32::from_le_bytes(b)
            } else {
                f32::from_be_bytes(b)
            });
        }
    }
    let grid = GeoidGrid {
        model: GeoidModel::NgsGeoid18,
        source_sha256,
        south_deg: h.south + row0 as f64 * h.dy,
        west_deg: h.west + col0 as f64 * h.dx,
        step_deg: [h.dy, h.dx],
        rows: rows as u32,
        cols: cols as u32,
        undulation_m: values,
    };
    grid.validate().map_err(|e| e.to_string())?;
    Ok(grid)
}
pub(super) const MODEL_URL: &str = "https://geodesy.noaa.gov/PC_PROD/GEOID18/Format_pc/g2018u0.bin";
fn geographic_bounds(bounds: [f64; 4]) -> Result<[f64; 4], String> {
    let [w, s, e, n] = bounds;
    let [south, west] = flightcraft_world::scenery::inverse_mercator(w, s).map_err(|e| e.to_string())?;
    let [north, east] = flightcraft_world::scenery::inverse_mercator(e, n).map_err(|e| e.to_string())?;
    Ok([west, south, east, north])
}
pub(super) fn control_url(lat: f64, lon: f64) -> String {
    format!("https://geodesy.noaa.gov/api/geoid/ght?lat={lat:.9}&lon={lon:.9}")
}
pub(super) fn fetch(
    request: &super::Request,
    sources: &mut super::Sources,
    remote: &mut impl super::transport::Remote,
) -> Result<(), String> {
    let [w, s, e, n] = geographic_bounds(sources.bounds)?;
    super::acquire(
        request,
        sources,
        remote,
        super::InputKind::GeoidModel,
        None,
        MODEL_URL,
        512 * 1024 * 1024,
    )?;
    // A separately implemented NGS service provides an independent check of sample ordering,
    // sign, registration, and interpolation once the flight lead returns the response.
    let url = control_url((s + n) / 2., (w + e) / 2.);
    let control_path = super::acquire(
        request,
        sources,
        remote,
        super::InputKind::GeoidControl,
        None,
        &url,
        64 * 1024,
    )?;
    // Omit the service-specific numeric model ID. Require the returned model name instead;
    // an HTTP 200 may contain only an error, and defaults may change in the future.
    super::datum::ngs_height(
        &super::download::read(&control_path, 64 * 1024)?,
        (s + n) / 2.,
        (w + e) / 2.,
    )?;
    Ok(())
}
pub(super) fn load(request: &super::Request, sources: &super::Sources) -> Result<Option<GeoidGrid>, String> {
    let inputs: Vec<_> = sources
        .inputs
        .iter()
        .filter(|i| i.kind == super::InputKind::GeoidModel)
        .collect();
    let Some(input) = inputs.first() else {
        return Ok(None);
    };
    if inputs.len() != 1 || input.source.url != MODEL_URL || input.source.bytes > 512 * 1024 * 1024 {
        return Err("GEOID18 needs one bounded official CONUS grid source".into());
    }
    let path = request
        .cache
        .join("objects")
        .join(super::download::sha256(input.source.url.as_bytes()));
    let (bytes, sha256) = super::transport::digest(&path)?;
    if bytes != input.source.bytes || sha256 != input.source.sha256 {
        return Err("GEOID18 differs from its receipt".into());
    }
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    read_header(&mut file)?.require_conus_coverage()?;
    subset(&mut file, geographic_bounds(sources.bounds)?, sha256).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    fn original_fixture(little: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        for v in [40_f64, 285., 1. / 60., 1. / 60.] {
            bytes.extend(if little { v.to_le_bytes() } else { v.to_be_bytes() });
        }
        for v in [20_i32, 20, 1] {
            bytes.extend(if little { v.to_le_bytes() } else { v.to_be_bytes() });
        }
        for row in 0..20 {
            for col in 0..20 {
                let v = -32_f32 + row as f32 * 0.1 + col as f32 * 0.2;
                bytes.extend(if little { v.to_le_bytes() } else { v.to_be_bytes() });
            }
        }
        bytes
    }
    #[test]
    fn conus_naming_is_checked_against_the_binary_header_extent() {
        for little in [true, false] {
            let mut header = Vec::new();
            for value in [24_f64, 230., 1. / 60., 1. / 60.] {
                header.extend(if little {
                    value.to_le_bytes()
                } else {
                    value.to_be_bytes()
                });
            }
            for value in [2041_i32, 4201, 1] {
                header.extend(if little {
                    value.to_le_bytes()
                } else {
                    value.to_be_bytes()
                });
            }
            let model = Header::parse(&header, 44 + 2041 * 4201 * 4, little).unwrap();
            model.require_conus_coverage().unwrap();
            assert_eq!(model.west, -130.);
            let regional = Header::parse(&original_fixture(little), 44 + 20 * 20 * 4, little).unwrap();
            assert!(regional.require_conus_coverage().is_err());
        }
        assert!(MODEL_URL.ends_with("/Format_pc/g2018u0.bin"));
        assert!(super::super::transport::allowed_url(MODEL_URL));
        assert!(!super::super::transport::allowed_url(
            "https://geodesy.noaa.gov/PC_PROD/GEOID18/CONUS/g2018u0.bin"
        ));
    }
    #[test]
    fn both_endian_grids_preserve_registration_and_subset_only_needed_rows() {
        for little in [true, false] {
            let mut reader = Cursor::new(original_fixture(little));
            let g = subset(&mut reader, [-74.96, 40.04, -74.94, 40.06], "a".repeat(64)).unwrap();
            assert_eq!((g.rows, g.cols), (7, 7));
            assert!((g.separation_m(40.05, -74.95).unwrap() + 31.1).abs() < 3e-6);
            assert_eq!(g.undulation_m.len(), 49);
        }
    }
    #[test]
    fn bad_headers_truncation_and_missing_coverage_are_errors() {
        let bounds = [-74.96, 40.04, -74.94, 40.06];
        let mut v = original_fixture(true);
        v.truncate(v.len() - 4);
        assert!(subset(&mut Cursor::new(v), bounds, "a".repeat(64)).is_err());
        for offset in [16, 24, 32, 36, 40] {
            let mut v = original_fixture(true);
            v.get_mut(offset..offset + 4)
                .unwrap()
                .copy_from_slice(&u32::MAX.to_le_bytes());
            assert!(subset(&mut Cursor::new(v), bounds, "a".repeat(64)).is_err());
        }
        assert!(
            subset(
                &mut Cursor::new(original_fixture(true)),
                [-80., 40., -79., 41.],
                "a".repeat(64)
            )
            .is_err()
        );
        assert!(subset(&mut Cursor::new(original_fixture(true)), bounds, "invalid".into()).is_err());
    }
}
