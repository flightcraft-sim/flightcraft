//! Application file transport around the existing, validated plan interchange commands.
use super::*;
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn export(format: &str, text: &str) -> Result<String, String> {
    let directory = flightcraft_control::navigation_data_directory()
        .ok_or("Local data directory unavailable; use Copy text")?
        .join("exports");
    let path = save_new(&directory, format, text)?;
    Ok(format!("Saved {}", path.display()))
}
#[cfg(not(target_arch = "wasm32"))]
fn save_new(directory: &std::path::Path, format: &str, text: &str) -> Result<std::path::PathBuf, String> {
    use std::io::Write;
    if !["json", "gpx", "fpl"].contains(&format) || text.len() > 1024 * 1024 {
        return Err("Invalid plan file format or size".into());
    }
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    for number in 1..=1000 {
        let path = directory.join(format!("flight-plan-{number}.{format}"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("Export directory is full; use Copy text or move previous exports".into())
}
#[cfg(target_arch = "wasm32")]
pub(super) fn export(format: &str, text: &str) -> Result<String, String> {
    use wasm_bindgen::{JsCast, JsValue};
    if !["json", "gpx", "fpl"].contains(&format) || text.len() > 1024 * 1024 {
        return Err("Invalid plan file format or size".into());
    }
    let parts = js_sys::Array::new();
    parts.push(&JsValue::from_str(text));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type(if format == "json" {
        "application/json"
    } else {
        "application/xml"
    });
    let blob =
        web_sys::Blob::new_with_str_sequence_and_options(&parts, &options).map_err(|e| format!("{e:?}"))?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(|e| format!("{e:?}"))?;
    let result = (|| {
        let document = web_sys::window()
            .and_then(|w| w.document())
            .ok_or("Browser document unavailable")?;
        let anchor = document
            .create_element("a")
            .map_err(|e| format!("{e:?}"))?
            .dyn_into::<web_sys::HtmlAnchorElement>()
            .map_err(|_| "Download link unavailable")?;
        let name = format!("flight-plan.{format}");
        anchor.set_href(&url);
        anchor.set_download(&name);
        let body = document.body().ok_or("Browser document body unavailable")?;
        body.append_child(&anchor).map_err(|e| format!("{e:?}"))?;
        anchor.click();
        anchor.remove();
        Ok(format!("Download requested: {name}"))
    })();
    if result.is_err() {
        let _ = web_sys::Url::revoke_object_url(&url);
    } else {
        // Keep the Blob alive until the browser has consumed the download navigation.
        wasm_bindgen_futures::spawn_local(async move {
            let ready = js_sys::Promise::new(&mut |resolve, _| {
                let scheduled = web_sys::window().is_some_and(|window| {
                    window
                        .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 1000)
                        .is_ok()
                });
                if !scheduled {
                    let _ = resolve.call0(&JsValue::NULL);
                }
            });
            let _ = wasm_bindgen_futures::JsFuture::from(ready).await;
            let _ = web_sys::Url::revoke_object_url(&url);
        });
    }
    result
}
pub(super) fn dropped(file: &egui::DroppedFile) -> Result<(String, String), String> {
    let format = file
        .path
        .as_ref()
        .and_then(|p| p.extension())
        .and_then(|s| s.to_str())
        .or_else(|| file.name.rsplit('.').next())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !["json", "gpx", "fpl"].contains(&format.as_str()) {
        return Err("Drop a GPX, FPL, or JSON flight plan".into());
    }
    let bytes = if let Some(bytes) = &file.bytes {
        if bytes.len() > 1024 * 1024 {
            return Err("Plan exceeds 1 MiB".into());
        }
        bytes.to_vec()
    } else {
        #[cfg(not(target_arch = "wasm32"))]
        {
            use std::io::Read;
            let mut bytes = Vec::new();
            std::fs::File::open(file.path.as_ref().ok_or("Dropped file path unavailable")?)
                .map_err(|e| e.to_string())?
                .take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() > 1024 * 1024 {
                return Err("Plan exceeds 1 MiB".into());
            }
            bytes
        }
        #[cfg(target_arch = "wasm32")]
        {
            return Err("Dropped file bytes unavailable; paste its text instead".into());
        }
    };
    let text = String::from_utf8(bytes).map_err(|_| "Plan file must be UTF-8")?;
    Ok((format, text))
}
#[cfg(all(test, not(target_arch = "wasm32")))]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn exports_do_not_replace_files_and_drops_validate_before_import() {
        let directory = std::env::temp_dir().join(format!("flightcraft-map-files-{}", std::process::id()));
        let first = save_new(&directory, "gpx", "<gpx/>").unwrap();
        let second = save_new(&directory, "gpx", "<gpx/>").unwrap();
        assert_ne!(first, second);
        let (format, text) = dropped(&egui::DroppedFile {
            path: Some(first),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(format, "gpx");
        assert_eq!(text, "<gpx/>");
        assert!(save_new(&directory, "../bad", "text").is_err());
        assert!(
            dropped(&egui::DroppedFile {
                name: "route.gpx".into(),
                bytes: Some(vec![0xff].into()),
                ..Default::default()
            })
            .is_err()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
