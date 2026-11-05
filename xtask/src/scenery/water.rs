//! USGS National Hydrography Dataset (public domain) acquisition and water-polygon tiles.
//!
//! Acquisition goes through the same bounded transport as the rest of the scenery pipeline: one
//! request per second, SHA-256 receipts, resumable downloads and the allowlisted USGS hosts.
//! The TNM Access API lists the NHD Best Resolution HU-8 subbasin shapefiles that intersect the
//! area; only products under the exact NHD staging prefix are accepted.
use super::{Input, InputKind, Request, Sources, acquire, transport};
use crate::download;
use flightcraft_world::scenery;

/// Exact staged-product prefix for the HU-8 shapefile subbasin downloads.
pub(super) const NHD_PREFIX: &str =
    "https://prd-tnm.s3.amazonaws.com/StagedProducts/Hydrography/NHD/HU8/Shape/";
const DATASET: &str = "National%20Hydrography%20Dataset%20%28NHD%29%20Best%20Resolution";
/// Subbasin archives are about 25 MB; the cap stops an unexpectedly large state or national file.
const MAX_ARCHIVE: u64 = 300 * 1024 * 1024;
const MAX_PRODUCTS: usize = 6;

pub(super) fn discovery_url(bounds: [f64; 4]) -> String {
    let [w, s, e, n] = bounds;
    format!(
        "{}?datasets={DATASET}&bbox={w:.9},{s:.9},{e:.9},{n:.9}&prodFormats=Shapefile&prodExtents=HU-8%20Subbasin&max=50&offset=0",
        super::TNM
    )
}
pub(super) fn geographic_bounds(bounds: [f64; 4]) -> Result<[f64; 4], String> {
    let [w, s, e, n] = bounds;
    let [south, west] = scenery::inverse_mercator(w, s).map_err(|e| e.to_string())?;
    let [north, east] = scenery::inverse_mercator(e, n).map_err(|e| e.to_string())?;
    Ok([west, south, east, north])
}
/// Product downloads named by a TNM response, restricted to the NHD HU-8 shapefile prefix.
pub(super) fn products(listing: &serde_json::Value) -> Result<Vec<String>, String> {
    let items = listing
        .get("items")
        .and_then(|v| v.as_array())
        .ok_or("TNM NHD response missing items")?;
    let total = listing
        .get("total")
        .and_then(|v| v.as_u64())
        .ok_or("TNM NHD response missing total")?;
    if total as usize != items.len() {
        return Err("TNM NHD listing is paginated; narrow the area".into());
    }
    let mut urls = std::collections::BTreeSet::new();
    for item in items {
        let url = item
            .get("downloadURL")
            .and_then(|v| v.as_str())
            .ok_or("NHD product missing downloadURL")?;
        let size = item.get("sizeInBytes").and_then(|v| v.as_u64()).unwrap_or(0);
        if !url.starts_with(NHD_PREFIX)
            || !url.ends_with(".zip")
            || url.contains(['?', '#', '%', '@'])
            || !transport::allowed_url(url)
            || size > MAX_ARCHIVE
        {
            return Err(format!(
                "NHD product outside the approved HU-8 shapefile policy: {url}"
            ));
        }
        urls.insert(url.to_owned());
    }
    if urls.is_empty() {
        return Err("no NHD HU-8 shapefile products cover this area".into());
    }
    if urls.len() > MAX_PRODUCTS {
        return Err("too many NHD subbasins; narrow the area".into());
    }
    Ok(urls.into_iter().collect())
}
pub(super) fn fetch(
    request: &Request,
    sources: &mut Sources,
    remote: &mut impl transport::Remote,
) -> Result<(), String> {
    let url = discovery_url(geographic_bounds(sources.bounds)?);
    let listing_path = acquire(
        request,
        sources,
        remote,
        InputKind::NhdDiscovery,
        None,
        &url,
        4 * 1024 * 1024,
    )?;
    let listing: serde_json::Value = download::json(&listing_path)?;
    for product in products(&listing)? {
        acquire(
            request,
            sources,
            remote,
            InputKind::Nhd,
            None,
            &product,
            MAX_ARCHIVE,
        )?;
    }
    Ok(())
}

pub(super) fn archives(sources: &Sources) -> impl Iterator<Item = &Input> {
    sources.inputs.iter().filter(|i| i.kind == InputKind::Nhd)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovery_is_one_bounded_tnm_query_and_products_are_prefix_checked() {
        let url = discovery_url([-75.02, 40.95, -74.97, 40.99]);
        assert!(url.starts_with("https://tnmaccess.nationalmap.gov/api/v1/products?"));
        assert!(url.contains("prodFormats=Shapefile") && url.contains("HU-8"));
        let good = format!("{NHD_PREFIX}NHD_H_02040105_HU8_Shape.zip");
        let listing = serde_json::json!({"total":1,"items":[{"downloadURL":good,"sizeInBytes":26_000_000}]});
        assert_eq!(products(&listing).unwrap(), vec![good]);
        for bad in [
            "https://prd-tnm.s3.amazonaws.com/StagedProducts/Hydrography/NHD/State/Shape/NHD_H_New_Jersey_State_Shape.zip",
            "https://example.com/StagedProducts/Hydrography/NHD/HU8/Shape/x.zip",
            "https://prd-tnm.s3.amazonaws.com.evil.invalid/StagedProducts/Hydrography/NHD/HU8/Shape/x.zip",
            "https://prd-tnm.s3.amazonaws.com/StagedProducts/Hydrography/NHD/HU8/Shape/x.zip?x=1",
        ] {
            let listing = serde_json::json!({"total":1,"items":[{"downloadURL":bad,"sizeInBytes":1}]});
            assert!(products(&listing).is_err(), "{bad}");
        }
        let huge =
            serde_json::json!({"total":1,"items":[{"downloadURL":good_url(),"sizeInBytes":900_000_000_u64}]});
        assert!(products(&huge).is_err());
        assert!(products(&serde_json::json!({"total":0,"items":[]})).is_err());
        assert!(products(&serde_json::json!({"total":2,"items":[]})).is_err());
    }
    fn good_url() -> String {
        format!("{NHD_PREFIX}NHD_H_02040105_HU8_Shape.zip")
    }
}
