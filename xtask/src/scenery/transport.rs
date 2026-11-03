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
pub(super) const MAX_CITY_ARCHIVE: u64 = 8_000_000_000;
pub(super) const TUM_PREFIX: &str = "http://www.3dcitydb.net/3dcitydb/fileadmin/public/datasets/NYC/";
pub(super) fn tum_url(url: &str, allow_http: bool) -> bool {
    allow_http
        && url.strip_prefix(TUM_PREFIX).is_some_and(|path| {
            !path.is_empty()
                && path
                    .split('/')
                    .all(|part| !part.is_empty() && part != "." && part != "..")
        })
        && !url.contains(['@', '\\', '\r', '\n', '?', '#', '%'])
}
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
    fn max_object(&self) -> u64 {
        MAX_OBJECT
    }
    fn allows(&self, url: &str) -> bool {
        allowed_url(url)
    }
    fn open(&mut self, url: &str, offset: u64, validator: Option<&str>) -> Result<Response, String>;
}
pub(super) struct Http {
    agent: ureq::Agent,
    last: Option<Instant>,
    approved_host: Option<String>,
    allow_http: bool,
}
impl Default for Http {
    fn default() -> Self {
        Self {
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(900)))
                .max_redirects(0)
                .build()
                .into(),
            last: None,
            approved_host: None,
            allow_http: false,
        }
    }
}
impl Http {
    pub(super) fn for_tum(allow_http: bool) -> Self {
        Self {
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(3600)))
                .https_only(!allow_http)
                .max_redirects(0)
                .build()
                .into(),
            allow_http,
            ..Self::default()
        }
    }
    pub(super) fn for_host(host: &str) -> Self {
        Self {
            approved_host: Some(host.into()),
            ..Self::default()
        }
    }
}
impl Remote for Http {
    fn max_object(&self) -> u64 {
        if self.allow_http {
            MAX_CITY_ARCHIVE
        } else {
            MAX_OBJECT
        }
    }
    fn allows(&self, url: &str) -> bool {
        if self.allow_http {
            return tum_url(url, true);
        }
        self.approved_host
            .as_ref()
            .map_or_else(|| allowed_url(url), |host| exact_host(url, host))
    }
    fn open(&mut self, url: &str, offset: u64, validator: Option<&str>) -> Result<Response, String> {
        if !self.allows(url) {
            return Err("scenery source is outside its approved URL policy".into());
        }
        if let Some(last) = self.last {
            std::thread::sleep(Duration::from_secs(1).saturating_sub(last.elapsed()));
        }
        self.last = Some(Instant::now());
        let mut request = self
            .agent
            .get(url)
            .header("User-Agent", "FlightCraft-scenery/1 (reviewed regional sources)")
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
pub(super) fn exact_host(url: &str, host: &str) -> bool {
    !host.is_empty()
        && host.contains('.')
        && !host.ends_with(".invalid")
        && host
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
        && url.starts_with(&format!("https://{host}/"))
        && !url.contains(['\r', '\n', '@', '\\', '#'])
}
pub(super) fn allowed_url(url: &str) -> bool {
    [
        "https://imagery.nationalmap.gov/",
        "https://tnmaccess.nationalmap.gov/",
        "https://prd-tnm.s3.amazonaws.com/",
        "https://rockyweb.usgs.gov/",
        "https://s3-us-west-2.amazonaws.com/usgs-lidar-public/",
        "https://geodesy.noaa.gov/PC_PROD/GEOID18/Format_pc/",
        "https://geodesy.noaa.gov/api/geoid/ght?",
    ]
    .iter()
    .any(|prefix| url.starts_with(prefix))
        && !url.contains(['\r', '\n', '@', '\\'])
        && !url.split('?').next().is_some_and(|path| {
            let lower = path.to_ascii_lowercase();
            path.split('/').any(|part| matches!(part, "." | ".."))
                || ["%2e", "%2f", "%5c", "%00"]
                    .iter()
                    .any(|code| lower.contains(code))
        })
}
pub(super) fn digest(path: &Path) -> Result<(u64, String), String> {
    digest_limit(path, MAX_OBJECT)
}
fn digest_limit(path: &Path, limit: u64) -> Result<(u64, String), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > limit {
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
        if size > limit {
            return Err("source exceeds its approved size limit".into());
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
    if !remote.allows(url) || limit == 0 || limit > remote.max_object() {
        return Err("invalid source URL or limit".into());
    }
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let key = download::sha256(url.as_bytes());
    let path = directory.join(&key);
    let receipt = directory.join(format!("{key}.json"));
    if receipt.exists() {
        let source: Source = download::json(&receipt)?;
        let (bytes, sha256) = digest_limit(&path, limit)?;
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
        429 => return Err("scenery source returned HTTP 429; no retry was made".into()),
        status => return Err(format!("scenery source returned HTTP {status}")),
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
    let (bytes, sha256) = digest_limit(&part, limit)?;
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

#[cfg(test)]
mod city_policy_tests {
    use super::*;
    #[test]
    fn explicit_http_permission_is_scoped_to_the_exact_tum_prefix() {
        let url = format!("{TUM_PREFIX}NYC_buildings_CityGML_LoD2/NYC_Buildings_LoD2_CityGML.zip");
        let allowed = Http::for_tum(true);
        let denied = Http::for_tum(false);
        assert!(allowed.allows(&url));
        assert_eq!(allowed.max_object(), 8_000_000_000);
        assert!(!denied.allows(&url));
        assert!(!tum_url(&url, false));
        for url in [
            "http://www.3dcitydb.net.attacker.invalid/3dcitydb/fileadmin/public/datasets/NYC/x.zip",
            "http://www.3dcitydb.net/mydata/Cesium_NYC_Demo/CityGML/old.zip",
            "https://prd-tnm.s3.amazonaws.com/product.tif",
            "http://www.3dcitydb.net:80/3dcitydb/fileadmin/public/datasets/NYC/x.zip",
        ] {
            assert!(!allowed.allows(url));
        }
        for path in [
            "../x.zip",
            "%2e%2e/x.zip",
            "x.zip?redirect=evil",
            "x//y.zip",
            "x.zip#anchor",
        ] {
            assert!(!allowed.allows(&format!("{TUM_PREFIX}{path}")));
        }
        assert_eq!(Http::default().max_object(), 2_000_000_000);
    }
}

#[cfg(test)]
mod ept_allowlist_tests {
    #[test]
    fn ept_bucket_prefix_and_paths_are_exact() {
        assert!(super::allowed_url(
            "https://s3-us-west-2.amazonaws.com/usgs-lidar-public/NJ_NW6Co_2017/ept.json"
        ));
        for url in [
            "https://s3-us-west-2.amazonaws.com/usgs-lidar-public.evil/ept.json",
            "https://s3-us-west-2.amazonaws.com/usgs-lidar-public/../private/ept.json",
            "https://s3-us-west-2.amazonaws.com/usgs-lidar-public/%2e%2e/private/ept.json",
            "https://s3-us-west-2.amazonaws.com/usgs-lidar-public/%2Fother-bucket/ept.json",
            "https://s3-us-west-2.amazonaws.com/usgs-lidar-public@evil/ept.json",
        ] {
            assert!(!super::allowed_url(url));
        }
        assert!(super::allowed_url(
            "https://tnmaccess.nationalmap.gov/api/v1/products?datasets=Lidar%20Point%20Cloud"
        ));
    }
}
