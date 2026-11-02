//! Package a browser build for a static host, without Trunk.
use std::path::PathBuf;
use std::process::Command;

pub fn build() -> Result<(), String> {
    let root = super::root()?;
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).map_err(|e| e.to_string())?;
    let version = bindgen_version(&lock)?;
    let bindgen = std::env::var_os("WASM_BINDGEN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("wasm-bindgen"));
    let output = Command::new(&bindgen)
        .arg("--version")
        .output()
        .map_err(|e| format!("cannot run wasm-bindgen: {e}; install wasm-bindgen-cli {version}"))?;
    if !output.status.success()
        || String::from_utf8_lossy(&output.stdout).trim() != format!("wasm-bindgen {version}")
    {
        return Err(format!(
            "wasm-bindgen must match Cargo.lock; install with: cargo install wasm-bindgen-cli --version {version} --locked"
        ));
    }

    // Respect Cargo configuration and build queues that supply their own target directory.
    let metadata = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .current_dir(&root)
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output()
        .map_err(|e| format!("cannot read Cargo metadata: {e}"))?;
    if !metadata.status.success() {
        return Err(String::from_utf8_lossy(&metadata.stderr).into_owned());
    }
    let metadata: serde_json::Value = serde_json::from_slice(&metadata.stdout).map_err(|e| e.to_string())?;
    let target = metadata
        .get("target_directory")
        .and_then(serde_json::Value::as_str)
        .ok_or("Cargo metadata has no target_directory")?;
    super::cargo(&[
        "build",
        "--locked",
        "--release",
        "--target",
        "wasm32-unknown-unknown",
        "-p",
        "flightcraft-web",
    ])?;

    let dist = root.join("dist");
    std::fs::create_dir_all(&dist).map_err(|e| e.to_string())?;
    let status = Command::new(bindgen)
        .arg(PathBuf::from(target).join("wasm32-unknown-unknown/release/flightcraft-web.wasm"))
        .args(["--target", "web", "--out-name", "flightcraft_web", "--out-dir"])
        .arg(&dist)
        .status()
        .map_err(|e| format!("cannot run wasm-bindgen: {e}"))?;
    if !status.success() {
        return Err("wasm-bindgen failed".into());
    }
    let index = std::fs::read_to_string(root.join("apps/web/index.html"))
        .map_err(|e| e.to_string())?
        .replace(
            "<link data-trunk rel=\"rust\" data-bin=\"flightcraft-web\">",
            "<script type=\"module\" src=\"./bootstrap.js\"></script>",
        )
        .replace(
            "    <link data-trunk rel=\"copy-file\" href=\"takeoff.jsonl\">\n",
            "",
        )
        .replace(
            "    <link data-trunk rel=\"copy-dir\" href=\"../../assets/terrain\">\n",
            "",
        );
    std::fs::write(dist.join("index.html"), index).map_err(|e| e.to_string())?;
    std::fs::copy(root.join("apps/web/bootstrap.js"), dist.join("bootstrap.js"))
        .map_err(|e| e.to_string())?;
    std::fs::copy(root.join("apps/web/takeoff.jsonl"), dist.join("takeoff.jsonl"))
        .map_err(|e| e.to_string())?;
    for id in ["new-york", "eastern-pa"] {
        let region_source = root.join("assets/terrain").join(id);
        let region_target = dist.join("terrain").join(id);
        std::fs::create_dir_all(&region_target).map_err(|e| e.to_string())?;
        copy_tree(&region_source.join("charts"), &region_target.join("charts"))?;
        for entry in std::fs::read_dir(region_source).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_type().map_err(|e| e.to_string())?.is_file() {
                std::fs::copy(entry.path(), region_target.join(entry.file_name()))
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    eprintln!("xtask: static web build ready in dist/");
    Ok(())
}

fn bindgen_version(lock: &str) -> Result<&str, String> {
    lock.split("[[package]]")
        .find(|package| {
            package
                .lines()
                .any(|line| line.trim() == "name = \"wasm-bindgen\"")
        })
        .and_then(|package| {
            package.lines().find_map(|line| {
                line.trim()
                    .strip_prefix("version = \"")
                    .and_then(|s| s.strip_suffix('"'))
            })
        })
        .ok_or_else(|| "Cargo.lock has no wasm-bindgen version".into())
}

/// Chart pyramids are copied as encoded files; packaging never decodes a raster tile.
fn copy_tree(source: &std::path::Path, target: &std::path::Path) -> Result<(), String> {
    if !source.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(target).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            copy_tree(&entry.path(), &target.join(entry.file_name()))?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), target.join(entry.file_name())).map_err(|e| e.to_string())?;
        } else {
            return Err("chart package contains a non-regular file".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comes_from_the_bindgen_package() {
        let lock = "version = 4\n[[package]]\nname = \"other\"\nversion = \"1.0\"\n\n[[package]]\nname = \"wasm-bindgen\"\nversion = \"0.2.130\"\n";
        assert_eq!(bindgen_version(lock).unwrap(), "0.2.130");
        assert!(bindgen_version("[[package]]\nname = \"wasm-bindgen-cli\"\nversion = \"0.2.130\"").is_err());
        assert!(bindgen_version("[[package]]\nname = \"wasm-bindgen\"").is_err());
    }
}
