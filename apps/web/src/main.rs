//! Browser entry point. wgpu chooses WebGPU, with WebGL2 as its fallback.
#[cfg(target_arch = "wasm32")]
mod packs;
#[cfg(target_arch = "wasm32")]
mod regions;
#[cfg(target_arch = "wasm32")]
fn main() {
    if web_sys::window().is_none() {
        return;
    }
    // Route `log` (including wgpu's GPU errors and device loss) to the browser console.
    let _ = eframe::WebLogger::init(log::LevelFilter::Info);
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
    let layout = load_layout().await?;
    let packs = packs::load().await;
    let regions = regions::load().await;
    let runner = eframe::WebRunner::new();
    runner
        .start(
            canvas,
            web_options()?,
            Box::new(move |cc| {
                let app = flightcraft_ui::screens::web::ScreenApp::new(
                    cc,
                    flightcraft_ui::AppOptions {
                        layout,
                        script,
                        packs,
                        regions,
                    },
                )?;
                // Tests and bug reports read this line to know which GPU path ran.
                web_sys::console::info_1(&format!("FlightCraft renderer: {}", app.renderer()).into());
                Ok(Box::new(app))
            }),
        )
        .await?;
    if let Some(loading) = document.get_element_by_id("loading") {
        loading.remove();
    }
    // WebRunner's registered callbacks retain the runner for the lifetime of the page.
    Ok(())
}

/// `?gpu=webgl2` forces WebGL 2 (a fallback for a machine whose WebGPU misbehaves) and `?gpu=webgpu`
/// allows WebGPU only (for tests); anything else lets wgpu choose WebGPU with WebGL 2 as fallback.
#[cfg(target_arch = "wasm32")]
fn web_options() -> Result<eframe::WebOptions, wasm_bindgen::JsValue> {
    use eframe::{egui_wgpu, wgpu};
    let window =
        web_sys::window().ok_or_else(|| wasm_bindgen::JsValue::from_str("Browser window unavailable"))?;
    let query = web_sys::UrlSearchParams::new_with_str(&window.location().search()?)?;
    let backends = match query.get("gpu").as_deref() {
        Some("webgl2") => wgpu::Backends::GL,
        Some("webgpu") => wgpu::Backends::BROWSER_WEBGPU,
        _ => return Ok(eframe::WebOptions::default()),
    };
    let mut options = eframe::WebOptions::default();
    if let egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup {
        setup.instance_descriptor.backends = backends;
    }
    Ok(options)
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

#[cfg(target_arch = "wasm32")]
mod worker {
    use flightcraft_control::Session;
    use flightcraft_ui::{clock::FixedStep, script::Script};
    use std::cell::RefCell;
    use wasm_bindgen::prelude::*;
    struct Owner {
        session: Session,
        clock: FixedStep,
        script: Script,
        previous: f64,
        external_until: f64,
        last_save: f64,
        last_weather_poll: f64,
    }
    thread_local! {static OWNER:RefCell<Option<Owner>>=const{RefCell::new(None)};}
    #[wasm_bindgen]
    pub fn screen_worker_start(seed: &str, script: &str) -> Result<(), JsValue> {
        if seed.len() > 256 * 1024 * 1024 {
            return Err(JsValue::from_str("worker seed too large"));
        }
        let seed = serde_json::from_str(seed).map_err(|e| JsValue::from_str(&e.to_string()))?;
        let session = Session::from_worker_seed(seed).map_err(|e| JsValue::from_str(&e))?;
        let script = Script::parse(script).map_err(|e| JsValue::from_str(&e))?;
        OWNER.with_borrow_mut(|owner| {
            if owner.is_none() {
                *owner = Some(Owner {
                    session,
                    clock: FixedStep::default(),
                    script,
                    previous: js_sys::Date::now(),
                    external_until: 0.,
                    last_weather_poll: 0.,
                    last_save: 0.,
                });
            }
        });
        Ok(())
    }
    #[wasm_bindgen]
    pub fn screen_worker_tick() -> Result<(), JsValue> {
        OWNER.with_borrow_mut(|owner| {
            let o = owner
                .as_mut()
                .ok_or_else(|| JsValue::from_str("worker not started"))?;
            let now = js_sys::Date::now();
            let elapsed = (now - o.previous) / 1000.;
            o.previous = now;
            // Background observations must finish and follow while physics is paused too.
            if o.session.paused() && now - o.last_weather_poll >= 250. {
                o.session
                    .run(flightcraft_control::WeatherSources::default())
                    .map_err(|e| JsValue::from_str(&e.to_string()))?;
                o.last_weather_poll = now;
            }
            if now - o.last_save >= 5000. {
                o.session.autosave_flight();
                o.last_save = now;
            }
            o.session.advance_walkaround_paused(elapsed);
            let steps = o.clock.advance(elapsed * o.session.rate(), o.session.paused());
            for _ in 0..steps {
                let time = o.session.sim().state().time_s;
                o.script
                    .run_due(&mut o.session, time)
                    .map_err(|e| JsValue::from_str(&e))?;
                if o.session.paused() {
                    o.clock.reset();
                    break;
                }
                o.session
                    .run(flightcraft_control::Step { steps: 1 })
                    .map_err(|e| JsValue::from_str(&e.to_string()))?;
            }
            Ok(())
        })
    }
    #[wasm_bindgen]
    pub fn screen_worker_command(line: &str) -> String {
        OWNER.with_borrow_mut(|owner| {
            let result = (|| {
                if line.len() > 65536 {
                    return Err("worker command too large".to_string());
                }
                let value: serde_json::Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
                let name = value
                    .get("command")
                    .and_then(|v| v.as_str())
                    .ok_or("command missing")?;
                let input = value.get("input").cloned().ok_or("input missing")?;
                let owner = owner.as_mut().ok_or("worker not started")?;
                if name == "sim.set_controls" {
                    if value.get("source").and_then(|v| v.as_str()) == Some("keyboard") {
                        if js_sys::Date::now() < owner.external_until
                            || owner.script.active()
                            || owner.session.holds_active()
                        {
                            return Ok(
                                serde_json::json!({"id":value.get("id"),"ok":owner.session.sim().controls()}),
                            );
                        }
                    } else {
                        owner.external_until = js_sys::Date::now() + 3000.;
                    }
                }
                if name == "sim.reset" {
                    owner.clock.reset();
                }
                owner
                    .session
                    .execute(name, input)
                    .map(|v| serde_json::json!({"id":value.get("id"),"ok":v}))
                    .map_err(|e| e.to_string())
            })();
            result
                .unwrap_or_else(|e| serde_json::json!({"error":e}))
                .to_string()
        })
    }
    #[wasm_bindgen]
    pub fn screen_worker_snapshot() -> String {
        OWNER.with_borrow(|owner| {
            owner
                .as_ref()
                .map(|owner| {
                    serde_json::json!({
                    "type": "snapshot",
                    "permission": "control",
                    "scripted": owner.script.active(),
                    "external": js_sys::Date::now() < owner.external_until || owner.session.holds_active(),
                    "snapshot": owner.session.screen_snapshot(),
                }).to_string()
                })
                .unwrap_or_default()
        })
    }
}

#[cfg(target_arch = "wasm32")]
async fn load_layout() -> Result<Option<flightcraft_control::screens::Layout>, wasm_bindgen::JsValue> {
    use wasm_bindgen::JsValue;
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("browser unavailable"))?;
    let query = web_sys::UrlSearchParams::new_with_str(&window.location().search()?)?;
    let Some(url) = query.get("layout") else {
        return Ok(None);
    };
    let bytes = crate::packs::fetch_bytes(&window, &url, 1024 * 1024).await?;
    let text = std::str::from_utf8(&bytes).map_err(|e| JsValue::from_str(&e.to_string()))?;
    flightcraft_control::screens::Layout::from_json(text)
        .map(Some)
        .map_err(|e| JsValue::from_str(&e))
}
