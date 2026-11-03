use super::WeatherSource;
#[cfg(not(target_arch = "wasm32"))]
pub fn directory() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(std::path::PathBuf::from(path).join("flightcraft/weather-sources"));
    }
    #[cfg(target_os = "windows")]
    if let Some(path) = std::env::var_os("APPDATA") {
        return Some(std::path::PathBuf::from(path).join("FlightCraft/weather-sources"));
    }
    std::env::var_os("HOME").map(|path| {
        let path = std::path::PathBuf::from(path);
        if cfg!(target_os = "macos") {
            path.join("Library/Application Support/FlightCraft/weather-sources")
        } else {
            path.join(".config/flightcraft/weather-sources")
        }
    })
}
#[cfg(not(target_arch = "wasm32"))]
pub fn load() -> (Vec<WeatherSource>, Vec<String>) {
    use std::io::Read;
    let mut sources = Vec::new();
    let mut errors = Vec::new();
    let Some(dir) = directory() else {
        return (sources, errors);
    };
    let entries = match std::fs::read_dir(dir) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (sources, errors),
        Err(e) => {
            return (
                sources,
                vec![format!("Cannot read weather source directory: {e}")],
            );
        }
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|v| v == "json"))
        .collect::<Vec<_>>();
    paths.sort();
    if paths.len() > 128 {
        errors.push("Only the first 128 weather source files are loaded".into());
    }
    for path in paths.into_iter().take(128) {
        let mut text = String::new();
        let result = std::fs::File::open(&path)
            .map_err(|e| e.to_string())
            .and_then(|f| {
                f.take(64 * 1024 + 1)
                    .read_to_string(&mut text)
                    .map_err(|e| e.to_string())
            })
            .and_then(|_| WeatherSource::from_json(&text));
        match result {
            Ok(s) => sources.push(s),
            Err(e) => errors.push(format!(
                "Weather source file {}: {e}",
                path.file_name().unwrap_or_default().to_string_lossy()
            )),
        }
    }
    (sources, errors)
}
#[cfg(not(target_arch = "wasm32"))]
pub fn save(source: &WeatherSource) -> Result<(), String> {
    source.validate()?;
    let dir = directory().ok_or("User configuration directory unavailable")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.json", source.id));
    // Never replace an existing user source through this dialog.
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|e| format!("Cannot save weather source: {e}"))?;
    let text = serde_json::to_string_pretty(source).map_err(|e| e.to_string())?;
    file.write_all(text.as_bytes()).map_err(|e| e.to_string())
}
#[cfg(target_arch = "wasm32")]
const KEY: &str = "flightcraft.weather-sources.v1";
#[cfg(target_arch = "wasm32")]
pub fn load() -> (Vec<WeatherSource>, Vec<String>) {
    let text = web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(KEY).ok().flatten());
    let Some(text) = text else {
        return (Vec::new(), Vec::new());
    };
    if text.len() > 128 * 64 * 1024 {
        return (Vec::new(), vec!["Saved weather sources exceed size limit".into()]);
    }
    match serde_json::from_str::<Vec<WeatherSource>>(&text) {
        Ok(sources) if sources.len() <= 128 => {
            let mut valid = Vec::new();
            let mut errors = Vec::new();
            for source in sources {
                match source.validate() {
                    Ok(()) => valid.push(source),
                    Err(e) => errors.push(e),
                }
            }
            (valid, errors)
        }
        _ => (Vec::new(), vec!["Saved weather sources are invalid".into()]),
    }
}
#[cfg(target_arch = "wasm32")]
pub fn save(source: &WeatherSource) -> Result<(), String> {
    source.validate()?;
    // SharedWorker commands commit their saved catalog; the connected window persists it.
    if web_sys::window().is_none() {
        return Ok(());
    }
    let (mut sources, _) = load();
    if sources.len() >= 128 || sources.iter().any(|s| s.id == source.id) {
        return Err("Source already exists or the source limit was reached".into());
    }
    sources.push(source.clone());
    let text = serde_json::to_string(&sources).map_err(|e| e.to_string())?;
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .ok_or("Browser storage unavailable")?
        .set_item(KEY, &text)
        .map_err(|_| "Cannot save weather sources".into())
}

#[cfg(target_arch = "wasm32")]
pub fn replace_browser_sources(sources: &[WeatherSource]) -> Result<(), String> {
    if !load().1.is_empty() {
        return Err("Existing weather-source settings are invalid and have been preserved. Repair them before saving another source.".into());
    }
    if sources.len() > 128 {
        return Err("Saved weather source limit reached".into());
    }
    for source in sources {
        source.validate()?;
    }
    let text = serde_json::to_string(sources).map_err(|e| e.to_string())?;
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .ok_or("Browser storage unavailable")?
        .set_item(KEY, &text)
        .map_err(|_| "Cannot save weather sources in browser storage".into())
}
