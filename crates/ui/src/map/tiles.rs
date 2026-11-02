//! On-demand chart reads with a fixed GPU/CPU footprint. No chart mosaic is decoded at once.
use super::*;
use flightcraft_control::{ChartIndex, ChartTile, inverse_mercator};
use std::collections::{BTreeMap, VecDeque};
#[derive(Default)]
pub struct Tiles {
    pub root: Option<String>,
    indexes: BTreeMap<String, Result<ChartIndex, String>>,
    textures: BTreeMap<String, egui::TextureHandle>,
    lru: VecDeque<String>,
    #[cfg(target_arch = "wasm32")]
    pending: std::sync::Arc<std::sync::Mutex<BTreeMap<String, Option<Result<Vec<u8>, String>>>>>,
    failures: std::collections::BTreeSet<String>,
}
impl Tiles {
    pub fn with_root(root: Option<String>) -> Self {
        Self {
            root,
            ..Default::default()
        }
    }
    fn base(&self, id: &str, kind: &str) -> String {
        #[cfg(not(target_arch = "wasm32"))]
        {
            format!(
                "{}/{id}/charts/{kind}",
                self.root.as_deref().unwrap_or("assets/terrain")
            )
        }
        #[cfg(target_arch = "wasm32")]
        {
            let query_prefix = web_sys::window()
                .and_then(|w| w.location().search().ok())
                .and_then(|q| web_sys::UrlSearchParams::new_with_str(&q).ok())
                .and_then(|p| {
                    p.get_all("region")
                        .iter()
                        .filter_map(|v| v.as_string())
                        .find(|s| s.ends_with(&format!("/{id}/region.json")))
                })
                .map(|s| s.trim_end_matches("region.json").to_owned());
            let prefix = self
                .root
                .as_ref()
                .map(|root| format!("{root}/{id}/"))
                .or(query_prefix)
                .unwrap_or_else(|| format!("terrain/{id}/"));
            format!("{prefix}charts/{kind}")
        }
    }
    pub fn paint(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        view: &View,
        id: &str,
        state: &MapState,
    ) -> String {
        let kind = if state.base_map == BaseMap::Tac {
            "tac"
        } else {
            "sectional"
        };
        let base = self.base(id, kind);
        let index_key = format!("{base}/index.json");
        if !self.indexes.contains_key(&index_key) {
            if self.indexes.len() >= 4 {
                self.indexes.clear();
            }
            match self.read(&index_key, ui.ctx()) {
                Some(Ok(bytes)) => {
                    self.indexes
                        .insert(index_key.clone(), ChartIndex::from_json(&bytes));
                }
                Some(Err(e)) => {
                    self.indexes.insert(index_key.clone(), Err(e));
                }
                None => return "Loading chart index…".into(),
            }
        }
        let Some(index) = self.indexes.get(&index_key) else {
            return "Chart unavailable".into();
        };
        let index = match index {
            Ok(v) => v,
            Err(e) => return format!("Chart unavailable: {e}"),
        };
        let edition = format!(
            "FAA {kind} · {}–{} · not for navigation",
            index.edition, index.expires
        );
        let latitude = view.position(view.rect.center()).map_or(40., |p| p.latitude_deg);
        let desired = (156543.033928 * latitude.to_radians().cos() * view.pixels_per_m)
            .log2()
            .round();
        let zoom = (desired as u8).clamp(index.min_zoom, index.max_zoom);
        let tiles: Vec<_> = index.tiles.iter().filter(|t| t.z == zoom).copied().collect();
        let mut loads = 0;
        for tile in tiles {
            let Some(corners) = tile_corners(tile, view) else {
                continue;
            };
            let bounds = Rect::from_points(&corners);
            if !view.rect.intersects(bounds) {
                continue;
            }
            let key = format!("{base}/{}", tile.path());
            if !self.textures.contains_key(&key) && !self.failures.contains(&key) && loads < 4 {
                loads += 1;
                if let Some(result) = self.read(&key, ui.ctx()) {
                    let decoded = result.and_then(|bytes| {
                        if bytes.len() > 2 * 1024 * 1024 {
                            return Err("tile exceeds 2 MiB".into());
                        }
                        let mut reader = image::ImageReader::with_format(
                            std::io::Cursor::new(bytes),
                            image::ImageFormat::Png,
                        );
                        let mut limits = image::Limits::default();
                        limits.max_image_width = Some(256);
                        limits.max_image_height = Some(256);
                        limits.max_alloc = Some(1024 * 1024);
                        reader.limits(limits);
                        let image = reader.decode().map_err(|e| e.to_string())?.into_rgba8();
                        if image.width() != 256 || image.height() != 256 {
                            return Err("invalid tile dimensions".into());
                        }
                        Ok(egui::ColorImage::from_rgba_unmultiplied(
                            [256, 256],
                            image.as_raw(),
                        ))
                    });
                    match decoded {
                        Ok(image) => {
                            while self.textures.len() >= 96 {
                                if let Some(old) = self.lru.pop_front() {
                                    self.textures.remove(&old);
                                } else {
                                    break;
                                }
                            }
                            self.textures.insert(
                                key.clone(),
                                ui.ctx().load_texture(&key, image, egui::TextureOptions::LINEAR),
                            );
                            ui.ctx().request_repaint();
                        }
                        Err(_) => {
                            if self.failures.len() >= 1024 {
                                self.failures.clear();
                            }
                            self.failures.insert(key.clone());
                        }
                    }
                }
            }
            if let Some(texture) = self.textures.get(&key) {
                self.lru.retain(|v| v != &key);
                self.lru.push_back(key);
                // Subdivide each Mercator tile in the region frame; rotation and projection remain shared.
                let [first, _, _, _] = corners;
                let n = f64::from(1_u32 << tile.z);
                let mut mesh = egui::Mesh::with_texture(texture.id());
                let tint = if state.night {
                    theme::MAP_CHART_NIGHT
                } else {
                    theme::MAP_CHART_DAY
                };
                for y in 0..=8 {
                    for x in 0..=8 {
                        let uv = egui::pos2(x as f32 / 8., y as f32 / 8.);
                        let position = inverse_mercator([
                            (f64::from(tile.x) + f64::from(x) / 8.) / n,
                            (f64::from(tile.y) + f64::from(y) / 8.) / n,
                        ])
                        .and_then(|p| view.point(p))
                        .unwrap_or(first);
                        mesh.vertices.push(egui::epaint::Vertex {
                            pos: position,
                            uv,
                            color: tint,
                        });
                    }
                }
                for y in 0..8 {
                    for x in 0..8 {
                        let a = y * 9 + x;
                        mesh.indices.extend([a, a + 1, a + 9, a + 1, a + 10, a + 9]);
                    }
                }
                painter.add(egui::Shape::mesh(mesh));
            }
        }
        if loads >= 4 {
            ui.ctx().request_repaint();
        }
        if !self.failures.is_empty() {
            format!("{edition} · some tiles unavailable")
        } else {
            edition
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn read(&mut self, key: &str, _ctx: &egui::Context) -> Option<Result<Vec<u8>, String>> {
        use std::io::Read;
        Some((|| {
            let mut bytes = Vec::new();
            std::fs::File::open(key)
                .map_err(|e| e.to_string())?
                .take(4 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() > 4 * 1024 * 1024 {
                return Err("chart resource too large".into());
            }
            Ok(bytes)
        })())
    }
    #[cfg(target_arch = "wasm32")]
    fn read(&mut self, key: &str, ctx: &egui::Context) -> Option<Result<Vec<u8>, String>> {
        let mut pending = self.pending.lock().ok()?;
        if let Some(slot) = pending.get_mut(key) {
            let result = slot.take();
            if result.is_some() {
                pending.remove(key);
            }
            return result;
        }
        if pending.len() >= 8 {
            return None;
        }
        pending.insert(key.into(), None);
        drop(pending);
        let key = key.to_owned();
        let pending = self.pending.clone();
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let result = fetch(&key).await;
            if let Ok(mut values) = pending.lock() {
                values.insert(key, Some(result));
            }
            ctx.request_repaint();
        });
        None
    }
}
fn tile_corners(tile: ChartTile, view: &View) -> Option<[egui::Pos2; 4]> {
    let n = f64::from(1_u32 << tile.z);
    let point = |dx, dy| {
        inverse_mercator([(f64::from(tile.x) + dx) / n, (f64::from(tile.y) + dy) / n])
            .and_then(|p| view.point(p))
    };
    Some([point(0., 0.)?, point(1., 0.)?, point(1., 1.)?, point(0., 1.)?])
}
#[cfg(target_arch = "wasm32")]
async fn fetch(url: &str) -> Result<Vec<u8>, String> {
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;
    let window = web_sys::window().ok_or("browser window unavailable")?;
    let response = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(|e| format!("{e:?}"))?
        .dyn_into::<web_sys::Response>()
        .map_err(|_| "chart response")?;
    if !response.ok() {
        return Err(format!("HTTP {}", response.status()));
    }
    let reader = response
        .body()
        .ok_or("chart response body")?
        .get_reader()
        .dyn_into::<web_sys::ReadableStreamDefaultReader>()
        .map_err(|_| "chart stream reader")?;
    let mut bytes = Vec::new();
    loop {
        let chunk = JsFuture::from(reader.read())
            .await
            .map_err(|e| format!("{e:?}"))?;
        if js_sys::Reflect::get(&chunk, &JsValue::from_str("done"))
            .ok()
            .and_then(|v| v.as_bool())
            == Some(true)
        {
            break;
        }
        let value =
            js_sys::Reflect::get(&chunk, &JsValue::from_str("value")).map_err(|_| "chart stream chunk")?;
        let array = js_sys::Uint8Array::new(&value);
        if bytes.len() + array.length() as usize > 4 * 1024 * 1024 {
            let _ = reader.cancel();
            return Err("chart resource exceeds 4 MiB".into());
        }
        bytes.extend(array.to_vec());
    }
    Ok(bytes)
}
