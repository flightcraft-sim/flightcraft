//! Offline SRTM transport. Archives are read in memory, never extracted to paths.
use flightcraft_world::{
    Region,
    srtm::{HgtTile, MAX_HGT_BYTES, Srtm},
};
use std::{
    io::{Cursor, Read},
    path::Path,
};
const MAX_ARCHIVE_BYTES: usize = MAX_HGT_BYTES + 1024 * 1024;

fn bounded(path: &Path, max: usize) -> Result<Vec<u8>, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > max as u64 {
        return Err("requires a bounded regular file without symlinks".into());
    }
    let mut bytes = Vec::new();
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("requires regular file".into());
    }
    file.take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > max {
        return Err("file exceeds size limit".into());
    }
    Ok(bytes)
}

fn decode(name: &str, bytes: &[u8]) -> Result<HgtTile, String> {
    if let Some(hgt_name) = name.strip_suffix(".zip") {
        if !hgt_name.ends_with(".hgt") || bytes.len() > MAX_ARCHIVE_BYTES {
            return Err("expected bounded .hgt.zip".into());
        }
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
        if archive.len() != 1 {
            return Err("HGT ZIP must contain exactly one file".into());
        }
        let mut entry = archive.by_index(0).map_err(|e| e.to_string())?;
        if entry.name() != hgt_name
            || entry.is_dir()
            || entry.encrypted()
            || entry
                .unix_mode()
                .is_some_and(|m| !matches!(m & 0o170000, 0 | 0o100000))
            || !matches!(entry.size(), n if n == 1201*1201*2 || n == MAX_HGT_BYTES as u64)
        {
            return Err(
                "ZIP entry must be a matching, unencrypted regular HGT basename with a valid size".into(),
            );
        }
        let mut data = Vec::new();
        (&mut entry)
            .take(MAX_HGT_BYTES as u64 + 1)
            .read_to_end(&mut data)
            .map_err(|e| e.to_string())?;
        HgtTile::read(hgt_name, &data).map_err(|e| e.to_string())
    } else {
        HgtTile::read(name, bytes).map_err(|e| e.to_string())
    }
}

pub fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(2);
    let (mut region_path, mut hgt, mut out) = (None, None, None);
    while let Some(arg) = args.next() {
        let slot = match arg.as_str() { "--region" => &mut region_path, "--hgt" => &mut hgt, "--out" => &mut out,
            _ => return Err("usage: cargo xtask terrain-srtm --region <region.json> --hgt <directory> --out <new-directory>".into()) };
        if slot.is_some() {
            return Err(format!("duplicate {arg}"));
        }
        *slot = Some(args.next().ok_or_else(|| format!("{arg} needs a path"))?);
    }
    let region_path = region_path.ok_or("missing --region")?;
    let hgt = hgt.ok_or("missing --hgt")?;
    let out = out.ok_or("missing --out")?;
    let text = bounded(Path::new(&region_path), flightcraft_world::MAX_REGION_BYTES)?;
    let region = Region::from_json(std::str::from_utf8(&text).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(&hgt).map_err(|e| e.to_string())?.take(4097) {
        let entry = entry.map_err(|e| e.to_string())?;
        paths.push(entry.path());
    }
    if paths.len() > 4096 {
        return Err("HGT directory exceeds 4096 entries".into());
    }
    paths.sort();
    let mut source = Srtm::default();
    for path in paths {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("HGT filename requires UTF-8")?;
        if !name.ends_with(".hgt") && !name.ends_with(".hgt.zip") {
            continue;
        }
        let bytes = bounded(&path, MAX_ARCHIVE_BYTES).map_err(|e| format!("{name}: {e}"))?;
        source
            .insert(decode(name, &bytes).map_err(|e| format!("{name}: {e}"))?)
            .map_err(|e| format!("{name}: {e}"))?;
    }
    let (region, tiles) = source.import(&region).map_err(|e| e.to_string())?;
    // Refuse replacement so a failed import cannot corrupt an installed world.
    let directory = Path::new(&out);
    std::fs::create_dir_all(directory.parent().ok_or("output needs parent")?).map_err(|e| e.to_string())?;
    std::fs::create_dir(directory).map_err(|e| format!("output must be a new directory: {e}"))?;
    let write = || -> Result<(), String> {
        for (tile, name) in tiles.iter().zip(&region.tiles) {
            std::fs::write(directory.join(name), tile.write()).map_err(|e| e.to_string())?;
        }
        // Publish metadata last; discovery ignores incomplete directories.
        std::fs::write(
            directory.join("region.json"),
            serde_json::to_vec_pretty(&region).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    };
    if let Err(error) = write() {
        let _ = std::fs::remove_dir_all(directory);
        return Err(error);
    }
    eprintln!("xtask: wrote {} SRTM tiles for {}", tiles.len(), region.id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn archive(name: &str, data: &[u8], compressed: bool) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let method = if compressed {
            zip::CompressionMethod::Deflated
        } else {
            zip::CompressionMethod::Stored
        };
        writer
            .start_file(
                name,
                zip::write::SimpleFileOptions::default().compression_method(method),
            )
            .unwrap();
        writer.write_all(data).unwrap();
        writer.finish().unwrap().into_inner()
    }
    #[test]
    fn stored_and_deflated_hgt_and_hostile_archives() {
        let data = vec![0; 1201 * 1201 * 2];
        for compressed in [false, true] {
            assert!(decode("N00E000.hgt.zip", &archive("N00E000.hgt", &data, compressed)).is_ok());
        }
        assert!(decode("N00E000.hgt.zip", &archive("../N00E000.hgt", &data, false)).is_err());
        assert!(decode("N00E000.hgt.zip", &archive("N00E000.hgt", b"bad", true)).is_err());
        assert!(decode("N00E000.hgt.zip", b"bad").is_err());
        let mut corrupt = archive("N00E000.hgt", &data, false);
        corrupt[100] ^= 1;
        assert!(decode("N00E000.hgt.zip", &corrupt).is_err());
    }
}
