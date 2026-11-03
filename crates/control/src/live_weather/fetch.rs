//! Network access is opt-in and lives above the deterministic simulation.
use super::source::{MAX_BODY, WeatherSource};
#[cfg(not(target_arch = "wasm32"))]
pub fn fetch(source: &WeatherSource) -> Result<Vec<u8>, String> {
    use std::{io::Read, time::Duration};
    source.validate()?;
    // The cookies feature is disabled in the workspace. A fresh agent also avoids ambient state.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .proxy(None)
        .timeout_global(Some(Duration::from_secs(u64::from(source.timeout_seconds))))
        .max_redirects(3)
        .https_only(!source.allow_http)
        .build()
        .into();
    let mut response = agent
        .get(&source.url)
        .header(
            "User-Agent",
            concat!(
                "FlightCraft/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/flightcraft-sim/flightcraft)"
            ),
        )
        .call()
        .map_err(|e| format!("Weather request failed: {e}"))?;
    if !response.status().is_success() {
        return Err("Weather server did not return a successful response".into());
    }
    if response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .is_some_and(|n| n > MAX_BODY)
    {
        return Err("Weather response exceeds 1 MB".into());
    }
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_BODY as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BODY {
        return Err("Weather response exceeds 1 MB".into());
    }
    Ok(bytes)
}
#[cfg(target_arch = "wasm32")]
pub async fn fetch_browser(source: &WeatherSource) -> Result<Vec<u8>, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    const DESKTOP: &str = "This source works in the desktop app. The browser requires CORS permission, HTTPS from an HTTPS app, and a direct URL without redirects.";
    source.validate()?;
    if !source.cors {
        return Err(DESKTOP.into());
    }
    let options = web_sys::RequestInit::new();
    options.set_method("GET");
    options.set_mode(web_sys::RequestMode::Cors);
    options.set_credentials(web_sys::RequestCredentials::Omit);
    // Fetch hides redirect counts; reject all redirects to enforce the native three-hop cap.
    options.set_redirect(web_sys::RequestRedirect::Error);
    options.set_referrer_policy(web_sys::ReferrerPolicy::NoReferrer);
    let signal = web_sys::AbortSignal::timeout_with_u32(source.timeout_seconds * 1000);
    options.set_signal(Some(&signal));
    let request =
        web_sys::Request::new_with_str_and_init(&source.url, &options).map_err(|_| DESKTOP.to_string())?;
    // The session runs in a SharedWorker; fetch exists on both worker and window globals.
    let global = js_sys::global();
    let fetch: js_sys::Function = js_sys::Reflect::get(&global, &"fetch".into())
        .map_err(|_| DESKTOP.to_string())?
        .dyn_into()
        .map_err(|_| DESKTOP.to_string())?;
    let promise: js_sys::Promise = fetch
        .call1(&global, &request)
        .map_err(|_| DESKTOP.to_string())?
        .dyn_into()
        .map_err(|_| DESKTOP.to_string())?;
    let response: web_sys::Response = JsFuture::from(promise)
        .await
        .map_err(|_| DESKTOP.to_string())?
        .dyn_into()
        .map_err(|_| DESKTOP.to_string())?;
    if !response.ok() {
        return Err(format!("Weather server returned HTTP {}", response.status()));
    }
    let stream = response.body().ok_or("Weather response has no body")?;
    let reader: web_sys::ReadableStreamDefaultReader = stream
        .get_reader()
        .dyn_into()
        .map_err(|_| "Weather reader unavailable")?;
    let mut bytes = Vec::new();
    loop {
        let chunk = JsFuture::from(reader.read())
            .await
            .map_err(|_| DESKTOP.to_string())?;
        if js_sys::Reflect::get(&chunk, &"done".into())
            .ok()
            .and_then(|v| v.as_bool())
            == Some(true)
        {
            break;
        }
        let value = js_sys::Reflect::get(&chunk, &"value".into()).map_err(|_| "Invalid weather response")?;
        let chunk = js_sys::Uint8Array::new(&value);
        if bytes.len().saturating_add(chunk.length() as usize) > MAX_BODY {
            let _ = JsFuture::from(reader.cancel()).await;
            return Err("Weather response exceeds 1 MB".into());
        }
        bytes.extend(chunk.to_vec());
    }
    Ok(bytes)
}
