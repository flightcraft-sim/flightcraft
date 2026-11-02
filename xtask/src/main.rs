//! Repository checks. `cargo xtask ci` runs what CI runs.

// Indexing a `serde_json::Value` returns `Null` for a missing key; it never panics.
#![allow(clippy::indexing_slicing)]

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use syn::visit::Visit;

mod airspace_faa;
mod charts_faa;
mod data;
mod download;
mod national;
mod navdata_faa;
mod navigation_faa;
mod procedures_faa;
mod refresh;
mod region;
mod terrain_srtm;
mod validation;
mod web;

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
    (
        "xtask",
        &["flightcraft-control", "flightcraft-aero", "flightcraft-world"],
    ),
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
        "region" => region::run(),
        "data" => data::run(),
        "terrain" => terrain(),
        "terrain-srtm" => terrain_srtm::run(),
        "charts-faa" => charts_faa::run(),
        "airspace-faa" => airspace_faa::run(),
        "navdata-faa" => navdata_faa::run(),
        "procedures-faa" => procedures_faa::run(),
        "navigation-faa" => navigation_faa::run(),
        "aircraft" => migrate_aircraft(),
        "schemas" => match std::env::args().nth(2).as_deref() {
            None => schemas(false),
            Some("--check") if std::env::args().nth(3).is_none() => schemas(true),
            _ => Err("usage: cargo xtask schemas [--check]".into()),
        },
        "determinism" => determinism(),
        "web" => web::build(),
        "ci" => ci(),
        _ => Err("usage: cargo xtask <ci|terrain|terrain-srtm --region <region.json> --hgt <directory> --out <new-directory>|airspace-faa --region <region.json> --airspace <Class_Airspace.geojson> --out <airspace.json>|aircraft migrate <v1.json> <new-directory>|layers|schemas [--check]|determinism|web>".to_string()),
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
    data::fetch(
        false,
        &root()?.join(".cache/data"),
        std::env::var("FLIGHTCRAFT_DATA_URL").ok().as_deref(),
    )?;
    layers()?;
    schemas(true)?;
    determinism()?;
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
        "-p",
        "flightcraft-mcp",
    ])?;
    eprintln!("xtask: ci passed (CI also runs cargo deny and the web build)");
    Ok(())
}

fn root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "xtask must be inside the workspace".into())
}

fn schemas(check: bool) -> Result<(), String> {
    let documents = [
        (
            "cell-index.json",
            serde_json::to_string_pretty(&schemars::schema_for!(region::CellIndex)),
        ),
        (
            "region-build.json",
            serde_json::to_string_pretty(&schemars::schema_for!(region::Request)),
        ),
        (
            "sources.json",
            serde_json::to_string_pretty(&schemars::schema_for!(region::Sources)),
        ),
        (
            "data-lock.json",
            serde_json::to_string_pretty(&schemars::schema_for!(data::Lock)),
        ),
        (
            "navigation-index.json",
            serde_json::to_string_pretty(&schemars::schema_for!(national::Index)),
        ),
        (
            "validation.json",
            serde_json::to_string_pretty(&schemars::schema_for!(validation::Report)),
        ),
        (
            "procedures.json",
            serde_json::to_string_pretty(&schemars::schema_for!(flightcraft_world::Procedures)),
        ),
        (
            "pins.json",
            serde_json::to_string_pretty(&schemars::schema_for!(flightcraft_control::PinsFile)),
        ),
        (
            "charts.json",
            serde_json::to_string_pretty(&schemars::schema_for!(flightcraft_control::ChartIndex)),
        ),
        (
            "plan.json",
            serde_json::to_string_pretty(&schemars::schema_for!(flightcraft_control::FlightPlan)),
        ),
        (
            "navdata.json",
            serde_json::to_string_pretty(&schemars::schema_for!(flightcraft_world::Navdata)),
        ),
        (
            "airspace.json",
            serde_json::to_string_pretty(&schemars::schema_for!(flightcraft_world::AirspaceFile)),
        ),
        (
            "airports.json",
            serde_json::to_string_pretty(&schemars::schema_for!(flightcraft_world::Airports)),
        ),
        (
            "region.json",
            serde_json::to_string_pretty(&schemars::schema_for!(flightcraft_world::Region)),
        ),
        (
            "commands.json",
            serde_json::to_string_pretty(&flightcraft_control::Session::commands()),
        ),
        (
            "aircraft.json",
            serde_json::to_string_pretty(&schemars::schema_for!(flightcraft_aero::LayerFile)),
        ),
        (
            "aircraft-v1.json",
            serde_json::to_string_pretty(&schemars::schema_for!(flightcraft_aero::Aircraft)),
        ),
        (
            "pack.json",
            serde_json::to_string_pretty(&schemars::schema_for!(flightcraft_aero::PackManifest)),
        ),
    ];
    for (name, value) in documents {
        let path = root()?.join("schema").join(name);
        let mut generated = value.map_err(|e| e.to_string())?;
        generated.push('\n');
        if check {
            let current = std::fs::read_to_string(&path)
                .map_err(|e| format!("cannot read schema/{name}: {e}; run cargo xtask schemas"))?;
            if current != generated {
                return Err(format!("schema/{name} is stale; run cargo xtask schemas"));
            }
        } else {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(path, generated).map_err(|e| e.to_string())?;
        }
        eprintln!("xtask: schema/{name} {}", if check { "ok" } else { "written" });
    }
    Ok(())
}

/// Methods whose platform implementation can compromise deterministic trajectories.
const TRANSCENDENTAL: &[&str] = &[
    "sin", "cos", "tan", "sin_cos", "asin", "acos", "atan", "atan2", "sinh", "cosh", "tanh", "asinh",
    "acosh", "atanh", "exp", "exp2", "exp_m1", "ln", "ln_1p", "log", "log2", "log10", "powf", "powi", "sqrt",
    "cbrt", "hypot",
];

#[derive(Default)]
struct MathCheck {
    methods: Vec<String>,
}

fn test_only(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("test")
            || (attribute.path().is_ident("cfg")
                && attribute
                    .parse_args::<syn::Path>()
                    .is_ok_and(|path| path.is_ident("test")))
    })
}

impl<'ast> Visit<'ast> for MathCheck {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if !test_only(&item.attrs) {
            syn::visit::visit_item_mod(self, item);
        }
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if !test_only(&item.attrs) {
            syn::visit::visit_item_fn(self, item);
        }
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        if !test_only(&item.attrs) {
            syn::visit::visit_item_impl(self, item);
        }
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        if !test_only(&item.attrs) {
            syn::visit::visit_impl_item_fn(self, item);
        }
    }

    fn visit_macro(&mut self, invocation: &'ast syn::Macro) {
        // Macros have opaque token bodies. Textual scanning still catches a method hidden in
        // a non-test assert!, vec!, or macro_rules! definition, without expanding the macro.
        let tokens: String = invocation
            .tokens
            .to_string()
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        for method in TRANSCENDENTAL {
            if tokens.contains(&format!(".{method}("))
                || tokens.contains(&format!("f64::{method}("))
                || tokens.contains(&format!("f32::{method}("))
            {
                self.methods.push((*method).into());
            }
        }
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let method = call.method.to_string();
        if TRANSCENDENTAL.contains(&method.as_str()) {
            self.methods.push(method);
        }
        syn::visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        // Also catch associated calls, e.g. f64::sqrt(x), but permit libm::sqrt(x).
        if let syn::Expr::Path(path) = call.func.as_ref() {
            let segments = &path.path.segments;
            let float_type = segments
                .iter()
                .any(|segment| segment.ident == "f64" || segment.ident == "f32");
            if float_type && let Some(last) = segments.last() {
                let method = last.ident.to_string();
                if TRANSCENDENTAL.contains(&method.as_str()) {
                    self.methods.push(method);
                }
            }
        }
        syn::visit::visit_expr_call(self, call);
    }
}

fn rust_sources(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in std::fs::read_dir(directory).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.is_dir() {
            rust_sources(&path, files)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    Ok(())
}

fn determinism() -> Result<(), String> {
    let root = root()?;
    let mut files = Vec::new();
    for name in ["world", "aero", "sim"] {
        rust_sources(&root.join("crates").join(name).join("src"), &mut files)?;
    }
    files.sort();
    let mut errors = Vec::new();
    for path in files {
        let source = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
        let syntax = syn::parse_file(&source).map_err(|error| format!("{}: {error}", path.display()))?;
        let mut check = MathCheck::default();
        check.visit_file(&syntax);
        if !check.methods.is_empty() {
            let relative = path.strip_prefix(&root).unwrap_or(&path);
            errors.push(format!(
                "{}: use libm instead of float methods: {}",
                relative.display(),
                check.methods.join(", ")
            ));
        }
    }
    if errors.is_empty() {
        eprintln!("xtask: deterministic maths ok");
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
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

fn terrain() -> Result<(), String> {
    let directory = root()?.join("assets/terrain/training-valley");
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let tiles = flightcraft_world::generate_training_valley(flightcraft_world::TRAINING_SEED)
        .map_err(|e| e.to_string())?;
    for (index, tile) in tiles.iter().enumerate() {
        std::fs::write(directory.join(format!("{index:02}.fcht")), tile.write())
            .map_err(|e| e.to_string())?;
    }
    eprintln!("xtask: wrote {} training valley tiles", tiles.len());
    Ok(())
}

fn migrate_aircraft() -> Result<(), String> {
    use std::io::Read;
    let mut args = std::env::args().skip(2);
    let usage = "usage: cargo xtask aircraft migrate <version-1.json> <new-directory>";
    if args.next().as_deref() != Some("migrate") {
        return Err(usage.into());
    }
    let input = args.next().ok_or(usage)?;
    let output = args.next().ok_or(usage)?;
    if args.next().is_some() {
        return Err(usage.into());
    }
    let mut text = String::new();
    std::fs::File::open(input)
        .map_err(|e| e.to_string())?
        .take(flightcraft_aero::MAX_AIRCRAFT_BYTES as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    let files = flightcraft_aero::migrate_v1(&text).map_err(|e| e.to_string())?;
    let mut catalog = flightcraft_aero::AircraftCatalog::default();
    for file in &files {
        catalog
            .insert(file.clone(), flightcraft_aero::DefinitionSource::BuiltIn)
            .map_err(|e| e.to_string())?;
    }
    catalog.validate().map_err(|e| e.to_string())?;
    std::fs::create_dir(&output).map_err(|e| e.to_string())?;
    for file in files {
        let value = serde_json::to_value(file).map_err(|e| e.to_string())?;
        let kind = value
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .ok_or("missing file kind")?;
        let text = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
        std::fs::write(
            Path::new(&output).join(format!("{kind}.json")),
            format!("{text}\n"),
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn backstop_catches_float_calls_and_skips_tests() {
        let syntax = syn::parse_file(r#"
            fn deterministic(x: f64) { let _ = libm::sqrt(x); }
            #[cfg(test)] mod tests { fn float_math(x: f64) { let _ = x.sin(); } }
            #[test] fn test_float_math() { let _ = 2.0_f64.sqrt(); }
            fn prohibited(x: f64) { let _ = x.sin(); let _ = x . cos (); let _ = f64::sqrt(x); assert!(x.exp() > 0.0); }
        "#).unwrap();
        let mut check = MathCheck::default();
        check.visit_file(&syntax);
        assert_eq!(check.methods, ["sin", "cos", "sqrt", "exp"]);
    }
}
