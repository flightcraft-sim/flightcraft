//! Scheduled refresh preparation. The workflow creates the PR; this command never publishes.
use super::{
    download::{self, Cache, Http},
    region::{self, Bundled, Request},
};
use serde_json::Value;
use std::{collections::BTreeSet, path::Path};

pub(super) fn edition(value: &Value) -> Option<String> {
    if let Some(object) = value.as_object() {
        for key in ["editionDate", "effectiveDate", "edition_date", "effective_date"] {
            if let Some(s) = object
                .get(key)
                .and_then(Value::as_str)
                .and_then(|s| s.get(..10))
                .filter(|s| flightcraft_world::navdata::iso_date(s))
            {
                return Some(s.into());
            }
        }
        for value in object.values() {
            if let Some(s) = edition(value) {
                return Some(s);
            }
        }
    }
    if let Some(array) = value.as_array() {
        for value in array {
            if let Some(s) = edition(value) {
                return Some(s);
            }
        }
    }
    None
}
fn identities(path: &Path, key: &str) -> Result<BTreeSet<String>, String> {
    let value: Value = download::json(path)?;
    Ok(value[key]
        .as_array()
        .into_iter()
        .flatten()
        .map(|v| {
            if key == "procedures" {
                format!("{}:{}:{}", v["airport"], v["ident"], v["kind"])
            } else {
                v["ident"].as_str().unwrap_or_default().to_owned()
            }
        })
        .collect())
}
pub fn run() -> Result<(), String> {
    let root = super::root()?;
    let work = root.join("out/refresh");
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let mut cache = Cache {
        transport: Http,
        directory: root.join(".cache/sources"),
        sources: Vec::new(),
    };
    let api: Value =
        serde_json::from_slice(&cache.get(region::CIFP, None, true)?).map_err(|e| e.to_string())?;
    let current_cycle = region::cycle(&api)?;
    let dtpp: Value =
        serde_json::from_slice(&cache.get(region::DTPP, None, true)?).map_err(|e| e.to_string())?;
    if region::cycle(&dtpp)? != current_cycle {
        return Err(
            "APRA CIFP and d-TPP current editions disagree; retry after publication completes".into(),
        );
    }
    let bundled: Vec<Bundled> = download::json(&root.join("datasets/regions.json"))?;
    let catalog: Vec<region::ChartSource> = download::json(&root.join("datasets/charts.json"))?;
    let mut changed = false;
    for preset in &bundled {
        let old: Value = download::json(
            &root
                .join("assets/terrain")
                .join(&preset.id)
                .join("procedures.json"),
        )?;
        changed |= old["cycle"].as_str() != Some(&current_cycle);
        for kind in &preset.charts {
            for chart in catalog
                .iter()
                .filter(|c| c.kind == *kind && region::intersects(c.bounds, preset.bbox))
            {
                let url = format!(
                    "https://external-api.faa.gov/apra/vfr/{kind}/chart?geoname={}&edition=current&format=tiff",
                    chart.name.replace(' ', "%20")
                );
                let response: Value =
                    serde_json::from_slice(&cache.get(&url, None, true)?).map_err(|e| e.to_string())?;
                let current =
                    edition(&response).ok_or("APRA chart response lacks a verified edition date")?;
                let old: Value = download::json(
                    &root
                        .join("assets/terrain")
                        .join(&preset.id)
                        .join("charts")
                        .join(kind)
                        .join("index.json"),
                )?;
                changed |= old["edition"].as_str() != Some(&current);
            }
        }
    }
    if !changed {
        eprintln!("region: FAA editions unchanged");
        return Ok(());
    }
    let mut summary = format!(
        "FAA cycle {current_cycle}\n\nADDS is refreshed as a snapshot with each 28-day procedure update. Charts are checked independently for their 56-day edition.\n\n| Region | Type | Added | Removed |\n|---|---|---:|---:|\n"
    );
    for preset in bundled {
        let output = work.join(&preset.id);
        if output.exists() {
            std::fs::remove_dir_all(&output).map_err(|e| e.to_string())?;
        }
        let request = Request {
            id: Some(preset.id.clone()),
            name: Some(preset.grid.name.clone()),
            origin: Some(preset.origin),
            bbox: Some(preset.bbox),
            cell: None,
            national: false,
            charts: Some(preset.charts),
            spacing: preset.grid.spacing_m,
            out: output.clone(),
            offline: None,
            cache: root.join(".cache/sources"),
            national_data: None,
            keep_v1: false,
            gnis: None,
        };
        region::build(request, Http)?;
        let old = root.join("assets/terrain").join(&preset.id);
        for (file, key) in [
            ("airports.json", "airports"),
            ("procedures.json", "procedures"),
            ("navdata.json", "fixes"),
        ] {
            let before = identities(&old.join(file), key)?;
            let after = identities(&output.join(file), key)?;
            summary.push_str(&format!(
                "| {} | {key} | {} | {} |\n",
                preset.id,
                after.difference(&before).count(),
                before.difference(&after).count()
            ));
        }
        std::fs::remove_dir_all(&old).map_err(|e| e.to_string())?;
        std::fs::rename(output, old).map_err(|e| e.to_string())?;
    }
    let mut identity = Vec::new();
    for path in super::data::files(&root.join("assets/terrain"))? {
        if path.extension().is_some_and(|e| e == "json")
            && path
                .file_name()
                .is_none_or(|n| n != "sources.json" && n != "validation.json")
        {
            identity.extend(download::read(&path, super::download::MAX_DOWNLOAD)?);
        }
    }
    let digest = super::download::sha256(&identity);
    let tag = format!(
        "region-data-{current_cycle}-{}",
        digest.get(..8).ok_or("release identity")?
    );
    super::data::publish(&tag, &root.join("out/data"))?;
    summary.push_str(&format!("\nBefore merging, publish the attached archives and manifests:\n\n```sh\ngh release create {tag} --repo flightcraft-sim/flightcraft --title 'Region data {tag}' --notes 'Public-domain terrain and FAA data.'\ngh release upload {tag} out/data/*.zip out/data/*.manifest.json --repo flightcraft-sim/flightcraft\n```\n"));
    std::fs::write(root.join("out/refresh-summary.md"), summary).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn edition_requires_a_verified_date() {
        assert_eq!(
            super::edition(&serde_json::json!({"data":[{"editionDate":"2026-10-29T09:01:00Z"}]})).as_deref(),
            Some("2026-10-29")
        );
        assert_eq!(
            super::edition(&serde_json::json!({"url":"https://example.test/chart.zip"})),
            None
        );
    }
}
