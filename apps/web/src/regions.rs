//! Static region manifests supplied with repeatable ?region= URLs, like aircraft packs.
use flightcraft_control::{
    MAX_AIRPORT_BYTES, MAX_AIRSPACE_BYTES, MAX_REGION_BYTES, MAX_REGION_TILE_BYTES, MAX_REGIONS,
    MAX_TERRAIN_BYTES, Region, RegionOptions, attach_region_airspace, region_world_with_airports,
};
use wasm_bindgen::JsValue;

pub async fn load() -> RegionOptions {
    let mut options = RegionOptions::default();
    if let Err(e) = load_all(&mut options).await {
        options.errors.push(format!("Terrain regions: {e:?}"));
    }
    options
}
async fn load_all(options: &mut RegionOptions) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("Browser window unavailable"))?;
    let query = web_sys::UrlSearchParams::new_with_str(&window.location().search()?)?;
    let urls = query.get_all("region");
    if urls.length() as usize > MAX_REGIONS {
        return Err(JsValue::from_str("At most 64 region URLs"));
    }
    let mut remaining = MAX_TERRAIN_BYTES;
    for value in urls.iter() {
        let Some(url) = value.as_string() else {
            continue;
        };
        match load_one(&window, &url, remaining).await {
            Ok(world) => {
                if let Some(region) = world.region() {
                    remaining = remaining.saturating_sub(region.tile_bytes());
                }
                options.fetched.push(world);
            }
            Err(e) => options.errors.push(format!("Terrain region {url}: {e:?}")),
        }
    }
    Ok(())
}
async fn load_one(
    window: &web_sys::Window,
    path: &str,
    remaining: usize,
) -> Result<flightcraft_control::TerrainWorld, JsValue> {
    let url = web_sys::Url::new_with_base(path, &window.location().href()?)?;
    if !matches!(url.protocol().as_str(), "http:" | "https:") {
        return Err(JsValue::from_str("Region URLs require HTTP or HTTPS"));
    }
    let bytes = crate::packs::fetch_bytes(window, &url.href(), MAX_REGION_BYTES).await?;
    let region =
        Region::from_json(std::str::from_utf8(&bytes).map_err(|e| JsValue::from_str(&e.to_string()))?)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
    region
        .validate_manifest()
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    if region.tile_bytes() > remaining {
        return Err(JsValue::from_str("Regions exceed 128 MiB tile budget"));
    }
    let mut tiles = Vec::new();
    for name in &region.tiles {
        let tile_url = web_sys::Url::new_with_base(name, &url.href())?;
        tiles.push(crate::packs::fetch_bytes(window, &tile_url.href(), MAX_REGION_TILE_BYTES).await?);
    }
    let airports = if let Some(name) = &region.airports {
        let airport_url = web_sys::Url::new_with_base(name, &url.href())?;
        Some(crate::packs::fetch_bytes(window, &airport_url.href(), MAX_AIRPORT_BYTES).await?)
    } else {
        None
    };
    let world =
        region_world_with_airports(region, tiles, airports).map_err(|e| JsValue::from_str(&e.to_string()))?;
    let nav_url = web_sys::Url::new_with_base("navdata.json", &url.href())?;
    let world = if let Some(bytes) =
        crate::packs::fetch_optional_bytes(window, &nav_url.href(), flightcraft_control::MAX_NAVDATA_BYTES)
            .await?
    {
        flightcraft_control::attach_region_navdata(
            world,
            std::str::from_utf8(&bytes).map_err(|e| JsValue::from_str(&e.to_string()))?,
        )
        .map_err(|e| JsValue::from_str(&e.to_string()))?
    } else {
        world
    };
    let airspace_url = web_sys::Url::new_with_base("airspace.json", &url.href())?;
    if let Some(bytes) =
        crate::packs::fetch_optional_bytes(window, &airspace_url.href(), MAX_AIRSPACE_BYTES).await?
    {
        attach_region_airspace(
            world,
            std::str::from_utf8(&bytes).map_err(|e| JsValue::from_str(&e.to_string()))?,
        )
        .map_err(|e| JsValue::from_str(&e.to_string()))
    } else {
        Ok(world)
    }
}
