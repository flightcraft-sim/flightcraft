//! Browser entry point. wgpu chooses WebGPU, with WebGL2 as its fallback.
#[cfg(target_arch = "wasm32")]
fn main() {
    wasm_bindgen_futures::spawn_local(async {
        if let Err(error) = start().await {
            web_sys::console::error_1(&error);
            if let Some(document) = web_sys::window().and_then(|w| w.document())
                && let Some(message) = document.get_element_by_id("loading")
            {
                message.set_text_content(Some(&format!("FlightCraft could not start: {error:?}")));
            }
        }
    });
}

#[cfg(target_arch = "wasm32")]
async fn start() -> Result<(), wasm_bindgen::JsValue> {
    use wasm_bindgen::JsCast;
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| wasm_bindgen::JsValue::from_str("Browser document unavailable"))?;
    let canvas = document
        .get_element_by_id("flightcraft")
        .ok_or_else(|| wasm_bindgen::JsValue::from_str("Flight canvas missing"))?
        .dyn_into::<web_sys::HtmlCanvasElement>()?;
    let script = load_script().await?;
    let runner = eframe::WebRunner::new();
    runner
        .start(
            canvas,
            eframe::WebOptions::default(),
            Box::new(move |cc| {
                Ok(Box::new(flightcraft_ui::FlightCraftApp::new(
                    cc,
                    flightcraft_ui::AppOptions { script },
                )?))
            }),
        )
        .await?;
    if let Some(loading) = document.get_element_by_id("loading") {
        loading.remove();
    }
    // WebRunner's registered callbacks retain the runner for the lifetime of the page.
    Ok(())
}

/// Fetch relative URLs from the page's static host, or absolute URLs with CORS enabled.
#[cfg(target_arch = "wasm32")]
async fn load_script() -> Result<Option<flightcraft_ui::script::Script>, wasm_bindgen::JsValue> {
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;

    let window = web_sys::window().ok_or_else(|| JsValue::from_str("Browser window unavailable"))?;
    let query = web_sys::UrlSearchParams::new_with_str(&window.location().search()?)?;
    let Some(url) = query.get("script") else {
        return Ok(None);
    };
    if url.is_empty() {
        return Err(JsValue::from_str("script URL must not be empty"));
    }
    let response = JsFuture::from(window.fetch_with_str(&url))
        .await?
        .dyn_into::<web_sys::Response>()?;
    if !response.ok() {
        return Err(JsValue::from_str(&format!(
            "script {url}: HTTP {}",
            response.status()
        )));
    }
    let text = JsFuture::from(response.text()?)
        .await?
        .as_string()
        .ok_or_else(|| JsValue::from_str("Script response is not text"))?;
    // Bound parsing work in addition to the script parser's command count limit.
    if text.len() > 4 * 1024 * 1024 {
        return Err(JsValue::from_str("Script exceeds 4 MiB"));
    }
    flightcraft_ui::script::Script::parse(&text)
        .map(Some)
        .map_err(|e| JsValue::from_str(&format!("script {url}: {e}")))
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("Build this app for wasm32-unknown-unknown with cargo xtask web or trunk serve.");
}
