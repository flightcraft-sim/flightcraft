//! Bounded relative image assets for a layout background or placard.
use eframe::egui;
#[derive(Default)]
pub struct Picture {
    key: String,
    image: Option<Result<egui::TextureHandle, String>>,
    #[cfg(target_arch = "wasm32")]
    pending: std::rc::Rc<std::cell::RefCell<Option<Result<egui::ColorImage, String>>>>,
}
fn decode(bytes: &[u8]) -> Result<egui::ColorImage, String> {
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("layout image exceeds 4 MiB".into());
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(|e| e.to_string())?.into_rgba8();
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    ))
}
impl Picture {
    pub fn show(&mut self, ui: &mut egui::Ui, path: &str) -> Result<(), String> {
        if path.len() > 1024
            || path.is_empty()
            || path.starts_with('/')
            || path.contains('\\')
            || path.split('/').any(|c| c == ".." || c == "." || c.is_empty())
        {
            return Err("image needs a relative asset path without traversal".into());
        }
        if self.key != path {
            self.key = path.into();
            self.image = None;
            #[cfg(not(target_arch = "wasm32"))]
            {
                let root = std::env::current_dir()
                    .and_then(|p| p.canonicalize())
                    .map_err(|e| e.to_string())?;
                let result = (|| {
                    let asset = root.join(path).canonicalize().map_err(|e| e.to_string())?;
                    if !asset.starts_with(&root)
                        || std::fs::metadata(&asset).map_err(|e| e.to_string())?.len() > 4 * 1024 * 1024
                    {
                        return Err("layout image outside assets or exceeds 4 MiB".into());
                    }
                    decode(&std::fs::read(asset).map_err(|e| e.to_string())?)
                })();
                self.image = Some(result.map(|image| ui.ctx().load_texture(path, image, Default::default())));
            }
            #[cfg(target_arch = "wasm32")]
            {
                let output = self.pending.clone();
                let context = ui.ctx().clone();
                let path = path.to_owned();
                wasm_bindgen_futures::spawn_local(async move {
                    use wasm_bindgen::JsCast;
                    use wasm_bindgen_futures::JsFuture;
                    let result = async {
                        let window = web_sys::window().ok_or("browser unavailable")?;
                        let url = web_sys::Url::new_with_base(
                            &path,
                            &window.location().href().map_err(|e| format!("{e:?}"))?,
                        )
                        .map_err(|e| format!("{e:?}"))?;
                        if url.origin() != window.location().origin().map_err(|e| format!("{e:?}"))? {
                            return Err("layout image must share the app origin".into());
                        }
                        let response = JsFuture::from(window.fetch_with_str(&url.href()))
                            .await
                            .map_err(|e| format!("{e:?}"))?
                            .dyn_into::<web_sys::Response>()
                            .map_err(|e| format!("{e:?}"))?;
                        if !response.ok() {
                            return Err(format!("image HTTP {}", response.status()));
                        }
                        let buffer = JsFuture::from(response.array_buffer().map_err(|e| format!("{e:?}"))?)
                            .await
                            .map_err(|e| format!("{e:?}"))?;
                        let bytes = js_sys::Uint8Array::new(&buffer);
                        if bytes.length() > 4 * 1024 * 1024 {
                            return Err("layout image exceeds 4 MiB".into());
                        }
                        decode(&bytes.to_vec())
                    }
                    .await;
                    *output.borrow_mut() = Some(result);
                    context.request_repaint();
                });
            }
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(result) = self.pending.borrow_mut().take() {
            self.image = Some(result.map(|image| ui.ctx().load_texture(path, image, Default::default())));
        }
        match &self.image {
            Some(Ok(texture)) => {
                ui.add(egui::Image::new(texture).fit_to_exact_size(ui.available_size()));
            }
            Some(Err(error)) => return Err(error.clone()),
            None => {
                ui.label("Loading image…");
            }
        }
        Ok(())
    }
}
