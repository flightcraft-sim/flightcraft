//! Bounded streaming source decoder. Coordinates retain the LAS header's units and datum.
//! Classification does not imply that a source's horizontal/vertical CRS has been accepted.
use serde::Serialize;
use std::io::{Read, Seek, SeekFrom};
const MAX_POINTS: u64 = 50_000_000;
const MAX_METADATA: u64 = 1024 * 1024;
const MAX_CHUNK: u64 = 64 * 1024 * 1024;
fn slice(bytes: &[u8], offset: usize, count: usize) -> Result<&[u8], String> {
    bytes
        .get(offset..offset.checked_add(count).ok_or("LAS field overflow")?)
        .ok_or_else(|| "truncated LAS field".into())
}
fn u16_at(b: &[u8], o: usize) -> Result<u16, String> {
    Ok(u16::from_le_bytes(
        slice(b, o, 2)?.try_into().map_err(|_| "LAS integer")?,
    ))
}
fn u32_at(b: &[u8], o: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(
        slice(b, o, 4)?.try_into().map_err(|_| "LAS integer")?,
    ))
}
fn u64_at(b: &[u8], o: usize) -> Result<u64, String> {
    Ok(u64::from_le_bytes(
        slice(b, o, 8)?.try_into().map_err(|_| "LAS integer")?,
    ))
}
fn f64_at(b: &[u8], o: usize) -> Result<f64, String> {
    Ok(f64::from_le_bytes(
        slice(b, o, 8)?.try_into().map_err(|_| "LAS number")?,
    ))
}
fn read_at(r: &mut (impl Read + Seek), offset: u64, count: usize) -> Result<Vec<u8>, String> {
    if count as u64 > MAX_METADATA {
        return Err("LAS metadata allocation budget exceeded".into());
    }
    r.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
    let mut b = vec![0; count];
    r.read_exact(&mut b).map_err(|e| e.to_string())?;
    Ok(b)
}
#[derive(Clone, Debug, Serialize)]
pub struct Header {
    pub point_count: u64,
    pub format: u8,
    pub compressed: bool,
    pub scale: [f64; 3],
    pub offset: [f64; 3],
    /// min X/Y/Z, max X/Y/Z, in source units.
    pub bounds: [f64; 6],
}
fn preflight(r: &mut (impl Read + Seek)) -> Result<Header, String> {
    let length = r.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
    if !(227..=super::transport::MAX_OBJECT).contains(&length) {
        return Err("LAS file size outside bounds".into());
    }
    let b = read_at(r, 0, 227)?;
    if slice(&b, 0, 4)? != b"LASF" || b.get(24) != Some(&1) || !matches!(b.get(25), Some(2..=4)) {
        return Err("LAS 1.2 through 1.4 required".into());
    }
    let minor = *b.get(25).ok_or("LAS version")?;
    let header_size = u64::from(u16_at(&b, 94)?);
    let points_start = u64::from(u32_at(&b, 96)?);
    let vlrs = u32_at(&b, 100)?;
    let record_size = u16_at(&b, 105)?;
    let fmt = *b.get(104).ok_or("LAS format")?;
    let format = fmt & 63;
    let minimum = match format {
        0 => 20,
        1 => 28,
        2 => 26,
        3 => 34,
        4 => 57,
        5 => 63,
        6 => 30,
        7 => 36,
        8 => 38,
        9 => 59,
        10 => 67,
        _ => return Err("unsupported LAS point format".into()),
    };
    if fmt & 64 != 0
        || header_size
            < match minor {
                2 => 227,
                3 => 235,
                _ => 375,
            }
        || header_size > MAX_METADATA
        || points_start < header_size
        || points_start > MAX_METADATA
        || points_start >= length
        || vlrs > 64
        || !(minimum..=1024).contains(&record_size)
        || (format >= 6 && minor != 4)
    {
        return Err("LAS header/point layout exceeds budget or is inconsistent".into());
    }
    let extended = if minor == 4 {
        Some(read_at(r, 227, 148)?)
    } else {
        None
    };
    let point_count = if let Some(ext) = &extended {
        let n = u64_at(ext, 20)?;
        if n > 0 { n } else { u64::from(u32_at(&b, 107)?) }
    } else {
        u64::from(u32_at(&b, 107)?)
    };
    if !(1..=MAX_POINTS).contains(&point_count) {
        return Err("LAS point count exceeds per-node budget".into());
    }
    let header = Header {
        point_count,
        format,
        compressed: fmt & 128 != 0,
        scale: [f64_at(&b, 131)?, f64_at(&b, 139)?, f64_at(&b, 147)?],
        offset: [f64_at(&b, 155)?, f64_at(&b, 163)?, f64_at(&b, 171)?],
        bounds: [
            f64_at(&b, 187)?,
            f64_at(&b, 203)?,
            f64_at(&b, 219)?,
            f64_at(&b, 179)?,
            f64_at(&b, 195)?,
            f64_at(&b, 211)?,
        ],
    };
    if header
        .scale
        .iter()
        .any(|v| !v.is_finite() || *v <= 0. || *v > 100.)
        || header
            .offset
            .iter()
            .chain(&header.bounds)
            .any(|v| !v.is_finite() || v.abs() > 100_000_000.)
        || (0..3).any(|i| header.bounds.get(i) > header.bounds.get(i + 3))
    {
        return Err("invalid LAS coordinate transforms or bounds".into());
    }
    let mut position = header_size;
    let mut zip = None;
    for _ in 0..vlrs {
        let h = read_at(r, position, 54)?;
        let count = usize::from(u16_at(&h, 20)?);
        let end = position
            .checked_add(54 + count as u64)
            .ok_or("LAS VLR overflow")?;
        if end > points_start {
            return Err("LAS VLR crosses point data".into());
        }
        let user = slice(&h, 2, 16)?;
        if user.starts_with(b"laszip encoded") && u16_at(&h, 18)? == 22204 {
            if zip.is_some() {
                return Err("duplicate LASzip VLR".into());
            }
            zip = Some(read_at(r, position + 54, count)?);
        }
        position = end;
    }
    if let Some(ext) = extended {
        let start = u64_at(&ext, 8)?;
        let count = u32_at(&ext, 16)?;
        if count > 64 || (count > 0 && (start < points_start || start >= length)) {
            return Err("invalid LAS EVLR range".into());
        }
        if count > 0 && !header.compressed {
            let end = points_start + point_count * u64::from(record_size);
            if start < end || start - end > MAX_METADATA {
                return Err("LAS point padding exceeds metadata budget".into());
            }
        }
        let mut cursor = start;
        let mut total = 0_u64;
        for _ in 0..count {
            let h = read_at(r, cursor, 60)?;
            let n = u64_at(&h, 20)?;
            let size = n.checked_add(60).ok_or("LAS EVLR overflow")?;
            total = total.checked_add(size).ok_or("LAS EVLR overflow")?;
            cursor = cursor.checked_add(size).ok_or("LAS EVLR overflow")?;
            if total > MAX_METADATA || cursor > length {
                return Err("LAS EVLR metadata budget exceeded".into());
            }
        }
    }
    if header.compressed {
        validate_chunks(
            r,
            points_start,
            length,
            record_size,
            point_count,
            &zip.ok_or("compressed LAS missing LASzip VLR")?,
        )?;
    } else if points_start
        .checked_add(point_count * u64::from(record_size))
        .is_none_or(|end| end > length)
    {
        return Err("truncated uncompressed LAS points".into());
    }
    r.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    Ok(header)
}
fn validate_chunks(
    r: &mut (impl Read + Seek),
    start: u64,
    length: u64,
    record_size: u16,
    points: u64,
    zip: &[u8],
) -> Result<(), String> {
    let count = usize::from(u16_at(zip, 32)?);
    if count == 0
        || count > 16
        || zip.len() != 34 + count * 6
        || !matches!(u16_at(zip, 0)?, 2 | 3)
        || u16_at(zip, 2)? != 0
    {
        return Err("unsupported LASzip items or compressor".into());
    }
    let layered = u16_at(zip, 0)? == 3;
    let mut layers = 0_usize;
    let mut item_size = 0_u64;
    for item in slice(zip, 34, count * 6)?.as_chunks::<6>().0.iter() {
        let kind = u16_at(item, 0)?;
        let size = u16_at(item, 2)?;
        if size == 0 || size > 1024 {
            return Err("invalid LASzip item size".into());
        }
        item_size += u64::from(size);
        if layered {
            layers += match (kind, size) {
                (10, 30) => 9,
                (11, 6) => 1,
                (12, 8) => 2,
                (13, 29) => 1,
                (14, _) => usize::from(size),
                _ => return Err("unsupported layered LASzip item".into()),
            };
        }
    }
    if item_size != u64::from(record_size) {
        return Err("LASzip record size mismatch".into());
    }
    let mut table = u64_at(&read_at(r, start, 8)?, 0)?;
    if table <= start || table > length.saturating_sub(8) {
        table = u64_at(&read_at(r, length - 8, 8)?, 0)?;
    }
    if table <= start + 8 || table > length.saturating_sub(8) {
        return Err("invalid LASzip chunk-table offset".into());
    }
    let prefix = read_at(r, table, 8)?;
    if u32_at(&prefix, 0)? != 0 || !(1..=100_000).contains(&u32_at(&prefix, 4)?) {
        return Err("LASzip chunk-table budget exceeded".into());
    }
    let vlr = laz::LazVlr::from_buffer(zip).map_err(|e| e.to_string())?;
    if !vlr.uses_variable_size_chunks() && !(1..=1_000_000).contains(&vlr.chunk_size()) {
        return Err("LASzip chunk point budget exceeded".into());
    }
    r.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
    let chunks = laz::laszip::ChunkTable::read_from(&mut *r, &vlr).map_err(|e| e.to_string())?;
    let mut cursor = start + 8;
    let mut remaining = points;
    for chunk in &chunks {
        let n = chunk.point_count.min(remaining);
        if n == 0 || chunk.point_count > 1_000_000 || chunk.byte_count > MAX_CHUNK || chunk.byte_count == 0 {
            return Err("LASzip chunk exceeds point/byte budget".into());
        }
        let end = cursor
            .checked_add(chunk.byte_count)
            .ok_or("LASzip chunk overflow")?;
        if end > table {
            return Err("LASzip chunk crosses its table".into());
        }
        if layered {
            let h = read_at(r, cursor + u64::from(record_size), 4 + layers * 4)?;
            if u64::from(u32_at(&h, 0)?) != n {
                return Err("LASzip layered point count mismatch".into());
            }
            let mut total = u64::from(record_size) + 4 + layers as u64 * 4;
            for i in 0..layers {
                total += u64::from(u32_at(&h, 4 + i * 4)?);
            }
            if total > chunk.byte_count {
                return Err("LASzip layers exceed bounded chunk size".into());
            }
        }
        cursor = end;
        remaining -= n;
    }
    if cursor != table || remaining != 0 {
        return Err("LASzip chunk table does not cover the declared points".into());
    }
    Ok(())
}
#[derive(Clone, Copy, Debug)]
pub struct Point {
    pub xyz: [f64; 3],
    pub classification: u8,
    pub withheld: bool,
}
#[derive(Debug, Serialize)]
pub struct Audit {
    pub header: Header,
    pub classifications: std::collections::BTreeMap<u8, u64>,
    pub withheld_points: u64,
    pub wkt: Vec<String>,
}
/// At most one point is materialized by this adapter; callers choose their own bounded grids.
pub fn scan<R: Read + Seek + Send + Sync + 'static>(
    mut input: R,
    mut accept: impl FnMut(Point) -> Result<(), String>,
) -> Result<Audit, String> {
    // Convert a dependency's malformed-stream panic to a CLI error, in addition to the
    // explicit metadata, chunk-table, and layer-allocation guards above.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let header = preflight(&mut input)?;
        let mut reader = las::Reader::new(input).map_err(|e| e.to_string())?;
        let wkt = reader
            .header()
            .vlrs()
            .iter()
            .chain(reader.header().evlrs())
            .filter(|v| v.user_id == "LASF_Projection" && v.record_id == 2112)
            .map(|v| {
                String::from_utf8(v.data.clone())
                    .map(|s| s.trim_end_matches('\0').to_owned())
                    .map_err(|e| e.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut audit = Audit {
            header,
            classifications: std::collections::BTreeMap::new(),
            withheld_points: 0,
            wkt,
        };
        let mut read = 0_u64;
        while let Some(p) = reader.read_point().map_err(|e| e.to_string())? {
            read += 1;
            if read > audit.header.point_count
                || [p.x, p.y, p.z]
                    .iter()
                    .any(|v| !v.is_finite() || v.abs() > 100_000_000.)
            {
                return Err("invalid decoded LAS coordinate or count".into());
            }
            let point = Point {
                xyz: [p.x, p.y, p.z],
                classification: u8::from(p.classification),
                withheld: p.is_withheld,
            };
            for (axis, value) in point.xyz.iter().enumerate() {
                let tolerance = audit.header.scale.get(axis).ok_or("LAS scale axis")? * 2.;
                let lo = audit.header.bounds.get(axis).ok_or("LAS bounds axis")?;
                let hi = audit.header.bounds.get(axis + 3).ok_or("LAS bounds axis")?;
                if *value < lo - tolerance || *value > hi + tolerance {
                    return Err("decoded LAS coordinate outside declared bounds".into());
                }
            }
            *audit.classifications.entry(point.classification).or_insert(0) += 1;
            audit.withheld_points += u64::from(point.withheld);
            accept(point)?;
        }
        if read != audit.header.point_count {
            return Err("LAS point count did not match decoded stream".into());
        }
        Ok(audit)
    }))
    .map_err(|_| "LASzip rejected a malformed compressed stream".to_string())?
}

/// Audits each receipt-backed cloud without retaining the complete collection in memory.
/// Source coordinates/datum are preserved for the next reconstruction pass; acceptance stays false.
pub(super) fn audit_sources(
    request: &super::Request,
    sources: &super::Sources,
    out: &std::path::Path,
    dem: &mut super::dem::Dems,
    geoid: Option<&flightcraft_world::geoid::GeoidGrid>,
    reconstruct: bool,
) -> Result<(u64, Option<super::surface::Grid>), String> {
    use std::io::Write;
    let ept = sources
        .inputs
        .iter()
        .any(|i| i.kind == super::InputKind::EptLidar);
    if ept {
        let roots: Vec<_> = sources
            .inputs
            .iter()
            .filter(|i| i.kind == super::InputKind::EptMetadata)
            .collect();
        let [root] = roots.as_slice() else {
            return Err("EPT build requires one receipt-backed project root".into());
        };
        let prefix = root
            .source
            .url
            .strip_suffix("/ept.json")
            .filter(|p| matches!(*p, super::ept::ROOT | super::ept::NYC_ROOT))
            .ok_or("EPT build requires a reviewed public USGS project")?;
        let metadata: super::ept::Metadata =
            serde_json::from_slice(&super::input_bytes(request, root)?).map_err(|e| e.to_string())?;
        metadata.validate()?;
        let prefix = format!("{prefix}/ept-data/");
        let mut keys = std::collections::BTreeSet::new();
        for input in sources
            .inputs
            .iter()
            .filter(|i| i.kind == super::InputKind::EptLidar)
        {
            let key = input
                .source
                .url
                .strip_prefix(&prefix)
                .and_then(|v| v.strip_suffix(".laz"))
                .ok_or("EPT point source is outside its public project")?;
            if !keys.insert(super::ept::Key::parse(key)?) {
                return Err("Duplicate EPT point node".into());
            }
        }
        if reconstruct && sources.ept_selection.is_none() {
            return Err("Complete the EPT selection before reconstructing tree density and roofs".into());
        }
    }
    let mut log = std::io::BufWriter::new(
        std::fs::File::create(out.join("lidar-sources.jsonl")).map_err(|e| e.to_string())?,
    );
    let mut surface = if reconstruct && ept {
        Some(super::surface::Grid::new(sources.bounds)?)
    } else {
        None
    };
    let mut ground = super::lidar_ground::Check::default();
    let mut files = 0_u64;
    let mut total = 0_u64;
    let mut classes = std::collections::BTreeMap::<u8, u64>::new();
    for input in sources.inputs.iter().filter(|i| {
        if ept {
            i.kind == super::InputKind::EptLidar
        } else {
            i.kind == super::InputKind::Lidar
        }
    }) {
        let path = request
            .cache
            .join("objects")
            .join(super::download::sha256(input.source.url.as_bytes()));
        let (bytes, hash) = super::transport::digest(&path)?;
        if bytes != input.source.bytes || hash != input.source.sha256 {
            return Err("lidar object differs from receipt".into());
        }
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let audit = scan(std::io::BufReader::new(file), |p| {
            if ept {
                ground.sample(p, sources.bounds, dem, geoid)?;
                if let Some(grid) = surface.as_mut() {
                    grid.insert(p)?;
                }
            }
            Ok(())
        })?;
        total = total
            .checked_add(audit.header.point_count)
            .ok_or("lidar audit point overflow")?;
        if total > 100_000_000 {
            return Err("lidar region point budget exceeded".into());
        }
        for (class, count) in &audit.classifications {
            *classes.entry(*class).or_default() += count;
        }
        serde_json::to_writer(
            &mut log,
            &serde_json::json!({"source":input.source,"decoded":audit}),
        )
        .map_err(|e| e.to_string())?;
        log.write_all(b"\n").map_err(|e| e.to_string())?;
        files += 1;
    }
    if ept
        && let Some(selection) = &sources.ept_selection
        && (total != selection.selected_points_before_clipping || files != selection.node_count as u64)
    {
        return Err("Decoded EPT count differs from completed hierarchy selection".into());
    }
    log.flush().map_err(|e| e.to_string())?;
    let ground_audit = ground.finish();
    super::download::write_json(&out.join("lidar-ground-audit.json"), &ground_audit)?;
    super::download::write_json(
        &out.join("lidar-audit.json"),
        &serde_json::json!({
            "source_files":files,"decoded_points":total,"classifications":classes,
            "complete_ept_selection":ept && sources.ept_selection.is_some(),
            "coordinate_datum_verified":reconstruct && ept && ground_audit.ept_z_agrees_with_navd88_meters,"reconstruction_built":false,
            "clipped_points":ground.clipped_points,"ground_agrees_with_navd88_meters":ground_audit.ept_z_agrees_with_navd88_meters,
            "sources_file":"lidar-sources.jsonl",
        }),
    )?;
    if let Some(grid) = surface.as_mut() {
        if !ground_audit.ept_z_agrees_with_navd88_meters {
            return Err("EPT source heights failed independent meter/NAVD88 ground agreement".into());
        }
        grid.normalize(dem)?;
    }
    Ok((total, surface))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    fn original_fixture(compressed: bool) -> Vec<u8> {
        let mut builder = las::Builder::from((1, 4));
        builder.point_format = las::point::Format::new(6).unwrap();
        builder.point_format.is_compressed = compressed;
        builder.transforms = las::Vector {
            x: las::Transform {
                scale: 0.001,
                offset: -8309760.,
            },
            y: las::Transform {
                scale: 0.001,
                offset: 4990314.,
            },
            z: las::Transform {
                scale: 0.001,
                offset: 247.,
            },
        };
        let mut writer = las::Writer::new(Cursor::new(Vec::new()), builder.into_header().unwrap()).unwrap();
        for (index, (class, z)) in [(2, 111.5), (3, 113.), (4, 119.), (5, 126.), (6, 117.8), (9, 110.)]
            .into_iter()
            .enumerate()
        {
            writer
                .write_point(las::Point {
                    x: -8309759. + index as f64 * 0.1,
                    y: 4990315. + index as f64 * 0.1,
                    z,
                    classification: las::point::Classification::new(class).unwrap(),
                    return_number: 1,
                    number_of_returns: 1,
                    gps_time: Some(100. + index as f64),
                    is_withheld: class == 3,
                    ..Default::default()
                })
                .unwrap();
        }
        writer.into_inner().unwrap().into_inner()
    }
    #[test]
    fn compressed_and_plain_las_decode_classification_flags_and_scaled_coordinates() {
        for compressed in [true, false] {
            let mut points = Vec::new();
            let audit = scan(Cursor::new(original_fixture(compressed)), |point| {
                points.push(point);
                Ok(())
            })
            .unwrap();
            assert_eq!(audit.header.point_count, 6);
            assert_eq!(audit.header.compressed, compressed);
            assert_eq!(audit.withheld_points, 1);
            assert_eq!(audit.classifications[&6], 1);
            assert_eq!(
                points.iter().map(|p| p.classification).collect::<Vec<_>>(),
                [2, 3, 4, 5, 6, 9]
            );
            assert!((points[0].xyz[0] + 8309759.).abs() < 0.001);
            assert!((points[4].xyz[2] - 117.8).abs() < 0.001);
        }
    }
    #[test]
    fn hostile_headers_chunk_tables_and_layers_fail_before_unbounded_allocations() {
        let valid = original_fixture(true);
        for length in [0, 4, 104, 226, 374, valid.len() - 10] {
            assert!(scan(Cursor::new(valid[..length].to_vec()), |_| Ok(())).is_err());
        }
        for (offset, bad) in [
            (96, u32::MAX.to_le_bytes().to_vec()),
            (100, u32::MAX.to_le_bytes().to_vec()),
            (247, u64::MAX.to_le_bytes().to_vec()),
            (131, f64::NAN.to_le_bytes().to_vec()),
            (243, u32::MAX.to_le_bytes().to_vec()),
        ] {
            let mut bytes = valid.clone();
            bytes[offset..offset + bad.len()].copy_from_slice(&bad);
            assert!(scan(Cursor::new(bytes), |_| Ok(())).is_err());
        }
        let start = u32_at(&valid, 96).unwrap() as usize;
        let table = u64_at(&valid, start).unwrap() as usize;
        let mut bytes = valid.clone();
        bytes[table + 4..table + 8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(scan(Cursor::new(bytes), |_| Ok(())).is_err());
        let mut bytes = valid.clone();
        bytes[start + 8 + 30 + 4..start + 8 + 30 + 8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(scan(Cursor::new(bytes), |_| Ok(())).is_err());
        let mut bytes = valid;
        bytes[start..start + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(scan(Cursor::new(bytes), |_| Ok(())).is_err());
    }
    #[test]
    fn callback_errors_cancel_streaming_without_materializing_the_cloud() {
        let mut calls = 0;
        let error = scan(Cursor::new(original_fixture(true)), |_| {
            calls += 1;
            Err("bounded consumer stopped".into())
        })
        .unwrap_err();
        assert_eq!(calls, 1);
        assert_eq!(error, "bounded consumer stopped");
    }
    #[test]
    #[ignore = "requires flight lead's receipt-backed 1N7 LAZ cache"]
    fn real_1n7_laz_decodes_and_audits_classifications() {
        let cache = std::path::Path::new("/home/ray/builds/codex-jobs/scenery/eastern-pa/1n7-test/cache");
        let sources: super::super::Sources =
            super::super::download::json(&cache.join("sources.json")).unwrap();
        let input = sources
            .inputs
            .iter()
            .find(|i| i.kind == super::super::InputKind::Lidar)
            .unwrap();
        let path = cache
            .join("objects")
            .join(super::super::download::sha256(input.source.url.as_bytes()));
        let (bytes, hash) = super::super::transport::digest(&path).unwrap();
        assert_eq!(bytes, input.source.bytes);
        assert_eq!(hash, input.source.sha256);
        let file = std::fs::File::open(path).unwrap();
        let audit = scan(std::io::BufReader::new(file), |_| Ok(())).unwrap();
        assert!(audit.header.compressed);
        assert_eq!(audit.header.format, 6);
        assert!(audit.classifications.get(&2).copied().unwrap_or(0) > 1000);
        assert_eq!(
            audit.classifications.values().sum::<u64>(),
            audit.header.point_count
        );
        assert!(
            audit
                .wkt
                .iter()
                .any(|w| w.contains("NAVD88") && w.contains("6527"))
        );
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../out/scenery");
        std::fs::create_dir_all(&out).unwrap();
        super::super::download::write_json(&out.join("laz-audit-real.json"), &audit).unwrap();
        println!(
            "Decoded {} real source points, classes {:?}; datum and classification coverage acceptance are separate",
            audit.header.point_count, audit.classifications
        );
    }
}
