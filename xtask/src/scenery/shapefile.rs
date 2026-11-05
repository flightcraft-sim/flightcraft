//! Bounded reader for ESRI polygon shapefiles with their dBASE attributes.
//! Handles Polygon, PolygonM and PolygonZ (the NHD Best Resolution layers use PolygonZM-style
//! records); every length, offset and count is checked before use.
use std::collections::BTreeMap;

pub const MAX_RECORDS: usize = 2_000_000;
pub const MAX_POINTS_PER_RECORD: usize = 500_000;

#[derive(Clone, Debug)]
pub struct Polygon {
    /// Outer ring first, then holes, as (longitude, latitude) degrees; rings are closed.
    pub rings: Vec<Vec<[f64; 2]>>,
    pub attributes: BTreeMap<String, String>,
}

fn le_i32(bytes: &[u8], at: usize) -> Result<i32, String> {
    let b: [u8; 4] = bytes
        .get(at..at.saturating_add(4))
        .ok_or("truncated shapefile")?
        .try_into()
        .map_err(|_| "shapefile integer")?;
    Ok(i32::from_le_bytes(b))
}
fn be_i32(bytes: &[u8], at: usize) -> Result<i32, String> {
    let b: [u8; 4] = bytes
        .get(at..at.saturating_add(4))
        .ok_or("truncated shapefile")?
        .try_into()
        .map_err(|_| "shapefile integer")?;
    Ok(i32::from_be_bytes(b))
}
fn le_f64(bytes: &[u8], at: usize) -> Result<f64, String> {
    let b: [u8; 8] = bytes
        .get(at..at.saturating_add(8))
        .ok_or("truncated shapefile")?
        .try_into()
        .map_err(|_| "shapefile double")?;
    let v = f64::from_le_bytes(b);
    if v.is_finite() {
        Ok(v)
    } else {
        Err("non-finite shapefile coordinate".into())
    }
}

/// dBASE III/IV table: one `BTreeMap` of trimmed strings per record.
pub fn read_dbf(bytes: &[u8]) -> Result<Vec<BTreeMap<String, String>>, String> {
    let count = le_i32(bytes, 4)? as usize;
    let header = usize::from(u16::from_le_bytes([
        *bytes.get(8).ok_or("truncated dBASE")?,
        *bytes.get(9).ok_or("truncated dBASE")?,
    ]));
    let length = usize::from(u16::from_le_bytes([
        *bytes.get(10).ok_or("truncated dBASE")?,
        *bytes.get(11).ok_or("truncated dBASE")?,
    ]));
    if count > MAX_RECORDS || header < 33 || length < 2 || header + count.saturating_mul(length) > bytes.len()
    {
        return Err("dBASE table exceeds bounds".into());
    }
    let mut fields = Vec::new();
    let mut offset = 1;
    let mut at = 32;
    while at + 32 <= header && bytes.get(at) != Some(&0x0d) {
        let raw = bytes.get(at..at + 11).ok_or("dBASE field")?;
        let name: String = raw
            .iter()
            .take_while(|b| **b != 0)
            .map(|b| char::from(*b).to_ascii_uppercase())
            .collect();
        let width = usize::from(*bytes.get(at + 16).ok_or("dBASE field width")?);
        fields.push((name, offset, width));
        offset += width;
        at += 32;
    }
    if offset > length {
        return Err("dBASE fields exceed record length".into());
    }
    let mut records = Vec::with_capacity(count);
    for i in 0..count {
        let start = header + i * length;
        let record = bytes.get(start..start + length).ok_or("dBASE record")?;
        let mut map = BTreeMap::new();
        for (name, offset, width) in &fields {
            let raw = record.get(*offset..offset + width).ok_or("dBASE value")?;
            map.insert(
                name.clone(),
                raw.iter()
                    .map(|b| char::from(*b))
                    .collect::<String>()
                    .trim()
                    .to_owned(),
            );
        }
        records.push(map);
    }
    Ok(records)
}

fn signed_area(ring: &[[f64; 2]]) -> f64 {
    ring.windows(2)
        .map(|w| w[0][0] * w[1][1] - w[1][0] * w[0][1])
        .sum::<f64>()
        / 2.
}
fn contains(ring: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut inside = false;
    for e in ring.windows(2) {
        let (a, b) = (e[0], e[1]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
    }
    inside
}

/// Reads polygon records whose bounding box intersects `bbox` ([west, south, east, north]).
/// ESRI rings: clockwise rings are exteriors, counter-clockwise rings are holes.
pub fn read_polygons(shp: &[u8], dbf: &[u8], bbox: [f64; 4]) -> Result<Vec<Polygon>, String> {
    if shp.len() < 100
        || be_i32(shp, 0)? != 9994
        || le_i32(shp, 28)? != 1000
        || be_i32(shp, 24)? as usize * 2 != shp.len()
    {
        return Err("not an ESRI shapefile".into());
    }
    let kind = le_i32(shp, 32)?;
    if !matches!(kind, 5 | 15 | 25) {
        return Err(format!("shapefile type {kind} is not a polygon type"));
    }
    let attributes = read_dbf(dbf)?;
    let mut polygons = Vec::new();
    let mut at = 100;
    let mut index = 0;
    while at + 8 <= shp.len() {
        let words = be_i32(shp, at + 4)?;
        if words < 0 {
            return Err("negative shapefile record length".into());
        }
        let size = words as usize * 2;
        let body = shp
            .get(at + 8..at + 8 + size)
            .ok_or("truncated shapefile record")?;
        at += 8 + size;
        let attribute = attributes
            .get(index)
            .ok_or("shapefile has more records than its dBASE table")?;
        index += 1;
        if index > MAX_RECORDS {
            return Err("too many shapefile records".into());
        }
        let shape = le_i32(body, 0)?;
        if shape == 0 {
            continue;
        }
        if shape != kind {
            return Err("mixed shapefile record types".into());
        }
        let (xmin, ymin, xmax, ymax) = (
            le_f64(body, 4)?,
            le_f64(body, 12)?,
            le_f64(body, 20)?,
            le_f64(body, 28)?,
        );
        if xmax < bbox[0] || xmin > bbox[2] || ymax < bbox[1] || ymin > bbox[3] {
            continue;
        }
        let parts = le_i32(body, 36)?;
        let points = le_i32(body, 40)?;
        if parts < 1
            || points < 4
            || points as usize > MAX_POINTS_PER_RECORD
            || parts as usize > points as usize
        {
            return Err("invalid shapefile part or point count".into());
        }
        let (parts, points) = (parts as usize, points as usize);
        let mut starts = Vec::with_capacity(parts + 1);
        for p in 0..parts {
            starts.push(le_i32(body, 44 + 4 * p)? as usize);
        }
        starts.push(points);
        let base = 44 + 4 * parts;
        let mut rings: Vec<Vec<[f64; 2]>> = Vec::new();
        for w in starts.windows(2) {
            if w[0] >= w[1] || w[1] > points {
                return Err("invalid shapefile part offsets".into());
            }
            let mut ring = Vec::with_capacity(w[1] - w[0] + 1);
            for p in w[0]..w[1] {
                ring.push([le_f64(body, base + 16 * p)?, le_f64(body, base + 16 * p + 8)?]);
            }
            if ring.first() != ring.last() {
                ring.push(ring[0]);
            }
            if ring.len() >= 4 {
                rings.push(ring);
            }
        }
        // Group into polygons: a clockwise ring starts a new exterior, others are holes.
        let mut grouped: Vec<Vec<Vec<[f64; 2]>>> = Vec::new();
        for ring in rings {
            if signed_area(&ring) <= 0. || grouped.is_empty() {
                grouped.push(vec![ring]);
            } else {
                let probe = ring[0];
                let owner = grouped
                    .iter()
                    .rposition(|g| g.first().is_some_and(|outer| contains(outer, probe)))
                    .unwrap_or(grouped.len() - 1);
                grouped[owner].push(ring);
            }
        }
        for rings in grouped {
            polygons.push(Polygon {
                rings,
                attributes: attribute.clone(),
            });
        }
    }
    Ok(polygons)
}

#[cfg(test)]
pub(super) mod fixtures {
    //! Original synthetic shapefile writer for tests; no real hydrography is embedded.
    pub fn polygon_shapefile(shapes: &[Vec<Vec<[f64; 2]>>]) -> Vec<u8> {
        let mut records = Vec::new();
        for (i, rings) in shapes.iter().enumerate() {
            let points: Vec<[f64; 2]> = rings.iter().flatten().copied().collect();
            let mut body = Vec::new();
            body.extend(5_i32.to_le_bytes());
            let xs = points.iter().map(|p| p[0]);
            let ys = points.iter().map(|p| p[1]);
            for v in [
                xs.clone().fold(f64::INFINITY, f64::min),
                ys.clone().fold(f64::INFINITY, f64::min),
                xs.fold(f64::NEG_INFINITY, f64::max),
                ys.fold(f64::NEG_INFINITY, f64::max),
            ] {
                body.extend(v.to_le_bytes());
            }
            body.extend((rings.len() as i32).to_le_bytes());
            body.extend((points.len() as i32).to_le_bytes());
            let mut start = 0;
            for ring in rings {
                body.extend((start as i32).to_le_bytes());
                start += ring.len();
            }
            for p in &points {
                body.extend(p[0].to_le_bytes());
                body.extend(p[1].to_le_bytes());
            }
            records.extend((i as i32 + 1).to_be_bytes());
            records.extend((body.len() as i32 / 2).to_be_bytes());
            records.extend(body);
        }
        let mut file = Vec::new();
        file.extend(9994_i32.to_be_bytes());
        file.extend([0; 20]);
        file.extend(((100 + records.len()) as i32 / 2).to_be_bytes());
        file.extend(1000_i32.to_le_bytes());
        file.extend(5_i32.to_le_bytes());
        file.extend([0; 64]);
        file.extend(records);
        file
    }
    /// A dBASE table with fixed C(12) NAME, N(6) FTYPE, N(6) FCODE columns.
    pub fn dbf(rows: &[(&str, u32, u32)]) -> Vec<u8> {
        let fields = [("GNIS_NAME", 12_u8), ("FTYPE", 6), ("FCODE", 6)];
        let record_length: usize = 1 + fields.iter().map(|f| usize::from(f.1)).sum::<usize>();
        let header = 32 + 32 * fields.len() + 1;
        let mut out = vec![3, 26, 11, 5];
        out.extend((rows.len() as u32).to_le_bytes());
        out.extend((header as u16).to_le_bytes());
        out.extend((record_length as u16).to_le_bytes());
        out.extend([0; 20]);
        for (name, width) in fields {
            let mut descriptor = [0_u8; 32];
            descriptor[..name.len()].copy_from_slice(name.as_bytes());
            descriptor[11] = b'C';
            descriptor[16] = width;
            out.extend(descriptor);
        }
        out.push(0x0d);
        for (name, ftype, fcode) in rows {
            out.push(b' ');
            out.extend(format!("{name:<12}{ftype:<6}{fcode:<6}").bytes());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    fn square(x: f64, y: f64, s: f64, clockwise: bool) -> Vec<[f64; 2]> {
        let mut ring = vec![[x, y], [x + s, y], [x + s, y + s], [x, y + s], [x, y]];
        // [x,y]->[x+s,y]->[x+s,y+s] is counter-clockwise in a y-up plane.
        if clockwise {
            ring.reverse();
        }
        ring
    }
    #[test]
    fn reads_exteriors_holes_attributes_and_filters_by_bbox() {
        let pond = vec![
            square(-75.0, 40.97, 0.004, true),
            square(-74.999, 40.971, 0.001, false),
        ];
        let far = vec![square(-70.0, 40.0, 0.004, true)];
        let shp = polygon_shapefile(&[pond, far]);
        let dbf = dbf(&[("Test Pond", 390, 39004), ("Far", 390, 39004)]);
        let polygons = read_polygons(&shp, &dbf, [-75.1, 40.9, -74.9, 41.0]).unwrap();
        assert_eq!(polygons.len(), 1);
        assert_eq!(polygons[0].rings.len(), 2);
        assert_eq!(polygons[0].attributes["GNIS_NAME"], "Test Pond");
        assert_eq!(polygons[0].attributes["FTYPE"], "390");
        assert!(signed_area(&polygons[0].rings[0]) < 0.);
        assert!(signed_area(&polygons[0].rings[1]) > 0.);
    }
    #[test]
    fn hostile_files_are_rejected() {
        let shp = polygon_shapefile(&[vec![square(0., 0., 1., true)]]);
        let dbf = dbf(&[("a", 1, 1)]);
        for cut in [0, 50, 99, 120, shp.len() - 3] {
            assert!(read_polygons(&shp[..cut], &dbf, [-1., -1., 2., 2.]).is_err());
        }
        let mut bad = shp.clone();
        bad[3] = 0;
        assert!(read_polygons(&bad, &dbf, [-1., -1., 2., 2.]).is_err());
        let mut nonfinite = shp.clone();
        nonfinite[100 + 8 + 4..100 + 8 + 12].copy_from_slice(&f64::NAN.to_le_bytes());
        assert!(read_polygons(&nonfinite, &dbf, [-1., -1., 2., 2.]).is_err());
        assert!(read_polygons(&shp, &dbf[..40], [-1., -1., 2., 2.]).is_err());
        // A dBASE table with fewer rows than shapes must not index out of range.
        assert!(read_polygons(&shp, &super::fixtures::dbf(&[]), [-1., -1., 2., 2.]).is_err());
    }
}
