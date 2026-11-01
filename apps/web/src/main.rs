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
    let runner = eframe::WebRunner::new();
    runner
        .start(
            canvas,
            eframe::WebOptions::default(),
            Box::new(|cc| {
                Ok(Box::new(flightcraft_ui::FlightCraftApp::new(
                    cc,
                    flightcraft_ui::AppOptions::default(),
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

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("Build this app for wasm32-unknown-unknown with trunk serve.");
}
