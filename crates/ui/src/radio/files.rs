//! Explicit JSON export transport; storage commands themselves never open a dialog.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn export(name: &str, text: &str) -> Result<String, String> {
    use std::io::Write;
    if text.len() > 128 * 1024 * 1024 {
        return Err("flight export exceeds 128 MiB".into());
    }
    let directory = flightcraft_control::navigation_data_directory()
        .ok_or("application data directory unavailable")?
        .join("exports");
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let stem = name.strip_suffix(".json").unwrap_or("flight");
    if !stem.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err("invalid export name".into());
    }
    for n in 1..=10000 {
        let path = directory.join(format!("{stem}-{n}.json"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
                file.sync_all().map_err(|e| e.to_string())?;
                return Ok(format!("Saved {}", path.display()));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("export directory contains too many matching names".into())
}
#[cfg(target_arch = "wasm32")]
pub(super) fn export(name: &str, text: &str) -> Result<String, String> {
    use wasm_bindgen::{JsCast, JsValue};
    if text.len() > 128 * 1024 * 1024 {
        return Err("flight export exceeds 128 MiB".into());
    }
    let pieces = js_sys::Array::new();
    pieces.push(&JsValue::from_str(text));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type("application/json");
    let blob =
        web_sys::Blob::new_with_str_sequence_and_options(&pieces, &options).map_err(|e| format!("{e:?}"))?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(|e| format!("{e:?}"))?;
    let result = (|| {
        let doc = web_sys::window()
            .and_then(|w| w.document())
            .ok_or("browser document unavailable")?;
        let anchor = doc
            .create_element("a")
            .map_err(|e| format!("{e:?}"))?
            .dyn_into::<web_sys::HtmlAnchorElement>()
            .map_err(|_| "download link unavailable")?;
        anchor.set_download(name);
        anchor.set_href(&url);
        doc.body()
            .ok_or("document body unavailable")?
            .append_child(&anchor)
            .map_err(|e| format!("{e:?}"))?;
        anchor.click();
        anchor.remove();
        Ok(format!("Download requested: {name}"))
    })();
    wasm_bindgen_futures::spawn_local(async move {
        let delay = js_sys::Promise::new(&mut |resolve, _| {
            if let Some(window) = web_sys::window() {
                let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 1000);
            } else {
                let _ = resolve.call0(&JsValue::UNDEFINED);
            }
        });
        let _ = wasm_bindgen_futures::JsFuture::from(delay).await;
        let _ = web_sys::Url::revoke_object_url(&url);
    });
    result
}
