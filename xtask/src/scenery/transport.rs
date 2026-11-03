//! Disk-streamed HTTP resume. A strong validator is required before appending a partial file.
use crate::download::{self, Source};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
    time::{Duration, Instant},
};

pub(super) const MAX_OBJECT: u64 = 2_000_000_000;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Partial {
    url: String,
    validator: String,
}
pub(super) struct Response {
    pub status: u16,
    pub validator: Option<String>,
    pub range: Option<String>,
    pub reader: Box<dyn Read>,
}
pub(super) trait Remote {
    fn open(&mut self, url: &str, offset: u64, validator: Option<&str>) -> Result<Response, String>;
}
pub(super) struct Http {
    agent: ureq::Agent,
    last: Option<Instant>,
}
impl Default for Http {
    fn default() -> Self {
        Self {
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(900)))
                .build()
                .into(),
            last: None,
        }
    }
}
impl Remote for Http {
    fn open(&mut self, url: &str, offset: u64, validator: Option<&str>) -> Result<Response, String> {
        if !allowed_url(url) {
            return Err("scenery source must be an approved USGS HTTPS endpoint".into());
        }
        if let Some(last) = self.last {
            std::thread::sleep(Duration::from_secs(1).saturating_sub(last.elapsed()));
        }
        self.last = Some(Instant::now());
        let mut request = self
            .agent
            .get(url)
            .header(
                "User-Agent",
                "FlightCraft-scenery/1 (public-domain regional cache)",
            )
            .header("Accept-Encoding", "identity");
        if offset > 0 {
            request = request
                .header("Range", &format!("bytes={offset}-"))
                .header("If-Range", validator.ok_or("resume needs validator")?);
        }
        let response = request.call().map_err(|e| e.to_string())?;
        let headers = response.headers();
        let validator = headers
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.starts_with("W/"))
            .or_else(|| headers.get("last-modified").and_then(|v| v.to_str().ok()))
            .map(str::to_owned);
        let range = headers
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let status = response.status().as_u16();
        let (_, body) = response.into_parts();
        Ok(Response {
            status,
            validator,
            range,
            reader: Box::new(body.into_reader()),
        })
    }
}
pub(super) fn allowed_url(url: &str) -> bool {
    [
        "https://imagery.nationalmap.gov/",
        "https://tnmaccess.nationalmap.gov/",
        "https://prd-tnm.s3.amazonaws.com/",
        "https://rockyweb.usgs.gov/",
    ]
    .iter()
    .any(|prefix| url.starts_with(prefix))
        && !url.contains(['\r', '\n', '@', '\\'])
}
pub(super) fn digest(path: &Path) -> Result<(u64, String), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_OBJECT {
        return Err("scenery source must be a bounded regular file".into());
    }
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        size += n as u64;
        if size > MAX_OBJECT {
            return Err("source exceeds 2 GB".into());
        }
        hasher.update(buffer.get(..n).ok_or("read length")?);
    }
    Ok((size, format!("{:x}", hasher.finalize())))
}
/// A completed URL is never re-downloaded silently: the existing receipt must still verify.
pub(super) fn fetch(
    remote: &mut impl Remote,
    url: &str,
    directory: &Path,
    limit: u64,
) -> Result<(std::path::PathBuf, Source), String> {
    if !allowed_url(url) || limit == 0 || limit > MAX_OBJECT {
        return Err("invalid source URL or limit".into());
    }
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let key = download::sha256(url.as_bytes());
    let path = directory.join(&key);
    let receipt = directory.join(format!("{key}.json"));
    if receipt.exists() {
        let source: Source = download::json(&receipt)?;
        let (bytes, sha256) = digest(&path)?;
        if source.url != url || source.bytes != bytes || source.sha256 != sha256 || bytes > limit {
            return Err("corrupt scenery cache receipt or payload".into());
        }
        return Ok((path, source));
    }
    let part = directory.join(format!("{key}.part"));
    let state = directory.join(format!("{key}.partial.json"));
    let previous = if state.exists() {
        Some(download::json::<Partial>(&state)?)
    } else {
        None
    };
    let offset = if previous
        .as_ref()
        .is_some_and(|p| p.url == url && !p.validator.is_empty())
    {
        std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };
    if offset > limit {
        return Err("partial scenery download exceeds limit".into());
    }
    let mut response = remote.open(url, offset, previous.as_ref().map(|p| p.validator.as_str()))?;
    let (start, expected) = match response.status {
        200 => (0, None),
        206 if offset > 0 => {
            if response.validator.as_deref() != previous.as_ref().map(|p| p.validator.as_str()) {
                return Err("resume validator changed".into());
            }
            let range = response
                .range
                .as_deref()
                .and_then(|s| s.strip_prefix("bytes "))
                .ok_or("missing Content-Range")?;
            let (interval, total) = range.split_once('/').ok_or("invalid Content-Range")?;
            let (first, last) = interval.split_once('-').ok_or("invalid Content-Range")?;
            let (first, last, total) = (
                first.parse::<u64>().map_err(|e| e.to_string())?,
                last.parse::<u64>().map_err(|e| e.to_string())?,
                total.parse::<u64>().map_err(|e| e.to_string())?,
            );
            if first != offset || last < first || last.checked_add(1) != Some(total) || total > limit {
                return Err("inconsistent or oversized resume range".into());
            }
            (offset, Some(total))
        }
        _ => return Err("unexpected scenery HTTP status".into()),
    };
    if std::fs::symlink_metadata(&part).is_ok_and(|m| !m.is_file()) {
        return Err("partial scenery object must be a regular file".into());
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(start == 0)
        .append(start > 0)
        .open(&part)
        .map_err(|e| e.to_string())?;
    // Commit the new validator only after a response is accepted and the old file truncated.
    download::write_json(
        &state,
        &Partial {
            url: url.into(),
            validator: response.validator.unwrap_or_default(),
        },
    )?;
    let mut total = start;
    let mut buffer = [0_u8; 65536];
    loop {
        let n = response
            .reader
            .read(&mut buffer)
            .map_err(|e| format!("download interrupted (rerun to resume): {e}"))?;
        if n == 0 {
            break;
        }
        total = total.checked_add(n as u64).ok_or("download length overflow")?;
        if total > limit {
            return Err("scenery source exceeds limit".into());
        }
        file.write_all(buffer.get(..n).ok_or("read length")?)
            .map_err(|e| e.to_string())?;
    }
    file.sync_all().map_err(|e| e.to_string())?;
    if total == 0 || expected.is_some_and(|n| n != total) {
        return Err("incomplete scenery response".into());
    }
    let (bytes, sha256) = digest(&part)?;
    let source = Source {
        url: url.into(),
        edition: None,
        downloaded_at: download::now()?,
        bytes,
        sha256,
    };
    std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
    download::write_json(&receipt, &source)?;
    std::fs::remove_file(state).map_err(|e| e.to_string())?;
    Ok((path, source))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fake {
        status: u16,
        data: Vec<u8>,
        offset: u64,
        range: Option<String>,
    }
    impl Remote for Fake {
        fn open(&mut self, _: &str, offset: u64, _: Option<&str>) -> Result<Response, String> {
            assert_eq!(offset, self.offset);
            Ok(Response {
                status: self.status,
                validator: Some("\"v1\"".into()),
                range: self.range.clone(),
                reader: Box::new(std::io::Cursor::new(self.data.clone())),
            })
        }
    }
    #[test]
    fn resumes_validated_ranges_restarts_200_and_rejects_corruption() {
        for status in [200, 206] {
            let dir = std::env::temp_dir().join(format!("scenery-resume-{}-{status}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let url = "https://prd-tnm.s3.amazonaws.com/test.tif";
            let key = download::sha256(url.as_bytes());
            std::fs::write(dir.join(format!("{key}.part")), b"ab").unwrap();
            download::write_json(
                &dir.join(format!("{key}.partial.json")),
                &Partial {
                    url: url.into(),
                    validator: "\"v1\"".into(),
                },
            )
            .unwrap();
            let mut remote = Fake {
                status,
                data: if status == 206 {
                    b"cd".to_vec()
                } else {
                    b"abcd".to_vec()
                },
                offset: 2,
                range: Some("bytes 2-3/4".into()),
            };
            let (path, source) = fetch(&mut remote, url, &dir, 10).unwrap();
            assert_eq!(source.sha256, download::sha256(b"abcd"));
            assert_eq!(std::fs::read(&path).unwrap(), b"abcd");
            std::fs::write(&path, b"corrupt").unwrap();
            assert!(fetch(&mut remote, url, &dir, 10).is_err());
            std::fs::remove_dir_all(dir).unwrap();
        }
        assert!(!allowed_url(
            "https://prd-tnm.s3.amazonaws.com.attacker.invalid/x"
        ));
    }
}
