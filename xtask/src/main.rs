//! Repository checks. `cargo xtask ci` runs what CI runs.

// Indexing a `serde_json::Value` returns `Null` for a missing key; it never panics.
#![allow(clippy::indexing_slicing)]

use std::process::{Command, ExitCode};

/// Each workspace crate and the workspace crates it may depend on.
const LAYERS: &[(&str, &[&str])] = &[
    ("flightcraft-world", &[]),
    ("flightcraft-aero", &[]),
    ("flightcraft-sim", &["flightcraft-world", "flightcraft-aero"]),
    (
        "flightcraft-control",
        &["flightcraft-sim", "flightcraft-world", "flightcraft-aero"],
    ),
    (
        "flightcraft-render",
        &["flightcraft-sim", "flightcraft-world", "flightcraft-aero"],
    ),
    (
        "flightcraft-ui",
        &[
            "flightcraft-control",
            "flightcraft-render",
            "flightcraft-sim",
            "flightcraft-world",
            "flightcraft-aero",
        ],
    ),
    (
        "flightcraft-desktop",
        &[
            "flightcraft-ui",
            "flightcraft-control",
            "flightcraft-render",
            "flightcraft-sim",
        ],
    ),
    (
        "flightcraft-web",
        &[
            "flightcraft-ui",
            "flightcraft-control",
            "flightcraft-render",
            "flightcraft-sim",
        ],
    ),
    ("flightcraft-mcp", &["flightcraft-control", "flightcraft-sim"]),
    ("xtask", &[]),
];

/// Crates the deterministic core must never use.
const CORE: &[&str] = &["flightcraft-world", "flightcraft-aero", "flightcraft-sim"];
const CORE_BANNED: &[&str] = &[
    "rand",
    "getrandom",
    "rayon",
    "winit",
    "wgpu",
    "egui",
    "eframe",
    "tokio",
    "instant",
    "web-time",
];

fn main() -> ExitCode {
    let task = std::env::args().nth(1).unwrap_or_default();
    let result = match task.as_str() {
        "layers" => layers(),
        "ci" => ci(),
        _ => Err("usage: cargo xtask <ci|layers>".to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cargo(args: &[&str]) -> Result<(), String> {
    eprintln!("xtask: cargo {}", args.join(" "));
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(args)
        .status()
        .map_err(|e| format!("cannot run cargo: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("cargo {} failed", args.join(" ")))
    }
}

fn ci() -> Result<(), String> {
    cargo(&["fmt", "--all", "--check"])?;
    layers()?;
    cargo(&["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])?;
    cargo(&["test", "--workspace"])?;
    cargo(&[
        "check",
        "--target",
        "wasm32-unknown-unknown",
        "-p",
        "flightcraft-world",
        "-p",
        "flightcraft-aero",
        "-p",
        "flightcraft-sim",
        "-p",
        "flightcraft-control",
    ])?;
    eprintln!("xtask: ci passed (CI also runs cargo deny and the web build)");
    Ok(())
}

fn layers() -> Result<(), String> {
    let out = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["metadata", "--format-version", "1"])
        .output()
        .map_err(|e| format!("cannot run cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err("cargo metadata failed".into());
    }
    let meta: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("bad cargo metadata: {e}"))?;
    let members: Vec<&str> = meta["workspace_members"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
        .collect();
    let packages = meta["packages"].as_array().ok_or("no packages in metadata")?;
    let mut errors = Vec::new();
    for pkg in packages
        .iter()
        .filter(|p| p["id"].as_str().is_some_and(|id| members.contains(&id)))
    {
        let name = pkg["name"].as_str().unwrap_or_default();
        let Some((_, allowed)) = LAYERS.iter().find(|(n, _)| *n == name) else {
            errors.push(format!("{name} has no layer in xtask/src/main.rs"));
            continue;
        };
        for dep in pkg["dependencies"].as_array().into_iter().flatten() {
            if dep["kind"].as_str() == Some("dev") {
                continue;
            }
            let dname = dep["name"].as_str().unwrap_or_default();
            if dname.starts_with("flightcraft-") && !allowed.contains(&dname) {
                errors.push(format!("{name} may not depend on {dname}"));
            }
            if CORE.contains(&name) && CORE_BANNED.contains(&dname) {
                errors.push(format!(
                    "{name} is deterministic core and may not depend on {dname}"
                ));
            }
        }
    }
    if errors.is_empty() {
        eprintln!("xtask: layers ok");
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}
