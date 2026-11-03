//! Capped streaming fetches of manifest-listed JSON documents from static hosts.
use flightcraft_control::{
    MAX_AIRCRAFT_BYTES, MAX_MANIFEST_BYTES, MAX_PACK_BYTES, MAX_PACKS, PackContents, PackManifest,
    PackOptions,
};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

pub async fn load() -> PackOptions {
    let mut result = PackOptions::default();
    if let Err(e) = load_all(&mut result).await {
        result.errors.push(format!("Aircraft pack URLs: {e:?}"));
    }
    result
}

async fn load_all(result: &mut PackOptions) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("Browser window unavailable"))?;
    let query = web_sys::UrlSearchParams::new_with_str(&window.location().search()?)?;
    let urls = query.get_all("pack");
    if urls.length() as usize > MAX_PACKS {
        return Err(JsValue::from_str("At most 64 pack URLs"));
    }
    for url in urls.iter() {
        let Some(url) = url.as_string() else {
            continue;
        };
        match load_one(&window, &url).await {
            Ok(pack) => result.fetched.push(pack),
            Err(e) => result.errors.push(format!("Aircraft pack {url}: {e:?}")),
        }
    }
    Ok(())
}

async fn load_one(window: &web_sys::Window, url: &str) -> Result<PackContents, JsValue> {
    let url = web_sys::Url::new_with_base(url, &window.location().href()?)?;
    if !matches!(url.protocol().as_str(), "http:" | "https:") {
        return Err(JsValue::from_str("Pack URLs require HTTP or HTTPS"));
    }
    let text = fetch(window, &url.href(), MAX_MANIFEST_BYTES).await?;
    let manifest = PackManifest::from_json(&text).map_err(|e| JsValue::from_str(&e.to_string()))?;
    let mut total = text.len();
    let mut files = std::collections::BTreeMap::new();
    for entry in &manifest.files {
        let file_url = web_sys::Url::new_with_base(&entry.path, &url.href())?;
        let remaining = MAX_PACK_BYTES.saturating_sub(total);
        let text = fetch(window, &file_url.href(), MAX_AIRCRAFT_BYTES.min(remaining)).await?;
        total = total.saturating_add(text.len());
        files.insert(entry.path.clone(), text);
    }
    let mut assets = std::collections::BTreeMap::new();
    for path in manifest.asset_paths() {
        let file_url = web_sys::Url::new_with_base(path, &url.href())?;
        let bytes = fetch_bytes(
            window,
            &file_url.href(),
            (4 * 1024 * 1024).min(MAX_PACK_BYTES.saturating_sub(total)),
        )
        .await?;
        total = total.saturating_add(bytes.len());
        assets.insert(path.to_owned(), bytes);
    }
    Ok(PackContents {
        manifest,
        files,
        assets,
    })
}

async fn fetch(window: &web_sys::Window, url: &str, max: usize) -> Result<String, JsValue> {
    let bytes = fetch_bytes(window, url, max).await?;
    String::from_utf8(bytes).map_err(|e| JsValue::from_str(&format!("Requires UTF-8: {e}")))
}

pub(crate) async fn fetch_bytes(window: &web_sys::Window, url: &str, max: usize) -> Result<Vec<u8>, JsValue> {
    let response = JsFuture::from(window.fetch_with_str(url))
        .await?
        .dyn_into::<web_sys::Response>()?;
    if !response.ok() {
        return Err(JsValue::from_str(&format!("HTTP {}", response.status())));
    }
    read_response(response, max).await
}

/// An absent optional region sidecar is allowed; every other HTTP failure is an error.
pub(crate) async fn fetch_optional_bytes(
    window: &web_sys::Window,
    url: &str,
    max: usize,
) -> Result<Option<Vec<u8>>, JsValue> {
    let response = JsFuture::from(window.fetch_with_str(url))
        .await?
        .dyn_into::<web_sys::Response>()?;
    if response.status() == 404 {
        return Ok(None);
    }
    if !response.ok() {
        return Err(JsValue::from_str(&format!("HTTP {}", response.status())));
    }
    read_response(response, max).await.map(Some)
}
async fn read_response(response: web_sys::Response, max: usize) -> Result<Vec<u8>, JsValue> {
    let reader = response
        .body()
        .ok_or_else(|| JsValue::from_str("Response has no body"))?
        .get_reader()
        .dyn_into::<web_sys::ReadableStreamDefaultReader>()?;
    let mut bytes = Vec::new();
    loop {
        let chunk = JsFuture::from(reader.read()).await?;
        if js_sys::Reflect::get(&chunk, &JsValue::from_str("done"))?.as_bool() == Some(true) {
            break;
        }
        let value = js_sys::Reflect::get(&chunk, &JsValue::from_str("value"))?;
        let array = js_sys::Uint8Array::new(&value);
        if bytes.len().saturating_add(array.length() as usize) > max {
            let _ = reader.cancel();
            return Err(JsValue::from_str(&format!("Response exceeds {max} bytes")));
        }
        bytes.extend(array.to_vec());
    }
    Ok(bytes)
}
