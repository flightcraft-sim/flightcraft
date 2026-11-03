//! Redistributable static-host tiles, through the same 3D Tiles tree and bounded residency.
//! This adapter never handles account keys or arbitrary remote origins.
use super::{ContentRequest, Location, Provider, Stream, decode_glb};
use flightcraft_world::tiles3d::{self, Tileset};
use std::sync::{Arc, mpsc};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
pub struct BrowserProvider;
pub(super) struct Worker {
    base: String,
    origin: [f64; 2],
    context: eframe::egui::Context,
    sender: mpsc::Sender<(usize, Result<Arc<super::DecodedTile>, String>)>,
    pub(super) results: mpsc::Receiver<(usize, Result<Arc<super::DecodedTile>, String>)>,
}
impl BrowserProvider {
    pub async fn open(
        settings: &Provider,
        origin: [f64; 2],
        context: eframe::egui::Context,
    ) -> Result<Stream, String> {
        let Location::Url(path) = &settings.location else {
            return Err("Browser scenery needs a static tileset URL".into());
        };
        if !settings.enabled || settings.token_config_key.is_some() || !tiles3d::local_uri(path) {
            return Err(
                "Browser scenery requires an enabled same-origin static path without credentials".into(),
            );
        }
        let window = web_sys::window().ok_or("Browser window unavailable")?;
        let href = window
            .location()
            .href()
            .map_err(|_| "Browser location unavailable")?;
        let url = web_sys::Url::new_with_base(path, &href).map_err(|_| "Invalid static scenery path")?;
        let location = web_sys::Url::new(&href).map_err(|_| "Invalid browser location")?;
        if !matches!(url.protocol().as_str(), "http:" | "https:") || url.origin() != location.origin() {
            return Err("Static scenery must use the website's origin".into());
        }
        let bytes = fetch(&url.href(), tiles3d::MAX_TILESET_BYTES).await?;
        let set = Tileset::from_json(&bytes).map_err(|e| e.to_string())?;
        super::validate_static_scenery(&set)?;
        let mut stream = Stream::new(&set, origin)?;
        let (sender, results) = mpsc::channel();
        stream.browser = Some(Worker {
            base: url.href(),
            origin,
            context,
            sender,
            results,
        });
        Ok(stream)
    }
}
impl Worker {
    pub(super) fn dispatch(&self, requests: &[ContentRequest]) {
        for request in requests {
            let request = request.clone();
            let sender = self.sender.clone();
            let base = self.base.clone();
            let origin = self.origin;
            let context = self.context.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let result = async {
                    if !tiles3d::local_uri(&request.uri) {
                        return Err("Invalid static scenery content path".into());
                    }
                    let url = web_sys::Url::new_with_base(&request.uri, &base)
                        .map_err(|_| "Invalid static scenery URL")?;
                    let bytes = fetch(&url.href(), tiles3d::MAX_TILE_CONTENT_BYTES).await?;
                    decode_glb(&bytes, request.transform, origin)
                }
                .await;
                let _ = sender.send((request.id, result));
                context.request_repaint();
            });
        }
    }
}
async fn fetch(url: &str, max: usize) -> Result<Vec<u8>, String> {
    let window = web_sys::window().ok_or("Browser window unavailable")?;
    let init = web_sys::RequestInit::new();
    init.set_redirect(web_sys::RequestRedirect::Error);
    init.set_credentials(web_sys::RequestCredentials::Omit);
    init.set_mode(web_sys::RequestMode::SameOrigin);
    let request = web_sys::Request::new_with_str_and_init(url, &init)
        .map_err(|_| "Cannot prepare static scenery request")?;
    let response = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|_| "Static scenery request failed")?
        .dyn_into::<web_sys::Response>()
        .map_err(|_| "Invalid static scenery response")?;
    if !response.ok() {
        return Err(format!("Static scenery returned HTTP {}", response.status()));
    }
    let reader = response
        .body()
        .ok_or("Static scenery response has no body")?
        .get_reader()
        .dyn_into::<web_sys::ReadableStreamDefaultReader>()
        .map_err(|_| "Static scenery stream unavailable")?;
    let mut bytes = Vec::new();
    loop {
        let chunk = JsFuture::from(reader.read())
            .await
            .map_err(|_| "Static scenery download interrupted")?;
        if js_sys::Reflect::get(&chunk, &"done".into())
            .ok()
            .and_then(|v| v.as_bool())
            == Some(true)
        {
            break;
        }
        let value =
            js_sys::Reflect::get(&chunk, &"value".into()).map_err(|_| "Invalid static scenery chunk")?;
        let array = js_sys::Uint8Array::new(&value);
        if bytes.len().saturating_add(array.length() as usize) > max {
            let _ = reader.cancel();
            return Err("Static scenery exceeds its content budget".into());
        }
        bytes.extend(array.to_vec());
    }
    Ok(bytes)
}
