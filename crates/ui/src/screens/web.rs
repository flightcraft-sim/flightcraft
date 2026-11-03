//! Browser screens use the same egui canvas, backed by a SharedWorker or paired LAN host.
use super::Canvas;
use crate::theme;
use eframe::egui;
use flightcraft_control::{
    Session,
    presentation::{RegionTransfer, Snapshot},
    screens::*,
};
use serde_json::{Value, json};
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::{JsCast, closure::Closure};
#[derive(Default)]
struct Inbox {
    latest: Option<Snapshot>,
    regions: Option<Vec<RegionTransfer>>,
    permission: bool,
    error: Option<String>,
}
struct Link {
    worker: Option<web_sys::SharedWorker>,
    socket: Option<web_sys::WebSocket>,
    onmessage: Closure<dyn FnMut(web_sys::MessageEvent)>,
    onerror: Closure<dyn FnMut(web_sys::Event)>,
    inbox: Rc<RefCell<Inbox>>,
}
impl Link {
    fn receive(ctx: egui::Context, inbox: Rc<RefCell<Inbox>>) -> Closure<dyn FnMut(web_sys::MessageEvent)> {
        Closure::new(move |event: web_sys::MessageEvent| {
            let Some(text) = event.data().as_string() else {
                return;
            };
            if text.len() > 256 * 1024 * 1024 {
                return;
            }
            let result = (|| {
                let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                let mut inbox = inbox.borrow_mut();
                if let Some(error) = value.get("error").and_then(Value::as_str) {
                    inbox.error = Some(error.into());
                }
                if let Some(regions) = value.get("regions") {
                    inbox.regions = Some(serde_json::from_value(regions.clone()).map_err(|e| e.to_string())?);
                }
                if let Some(snapshot) = value.get("snapshot") {
                    inbox.latest = Some(serde_json::from_value(snapshot.clone()).map_err(|e| e.to_string())?);
                }
                if let Some(permission) = value.get("permission").and_then(Value::as_str) {
                    inbox.permission = permission == "control";
                }
                Ok::<_, String>(())
            })();
            if let Err(e) = result {
                inbox.borrow_mut().error = Some(e);
            }
            ctx.request_repaint();
        })
    }
    fn worker(
        ctx: &egui::Context,
        seed: flightcraft_control::presentation::WorkerSeed,
        script: String,
    ) -> Result<Self, String> {
        let options = web_sys::WorkerOptions::new();
        options.set_type(web_sys::WorkerType::Module);
        options.set_name("flightcraft.session.v1");
        let module = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.query_selector("link[rel='modulepreload']").ok().flatten())
            .and_then(|e| e.get_attribute("href"));
        let worker_url = module
            .and_then(|module| {
                web_sys::window()
                    .and_then(|w| w.location().href().ok())
                    .and_then(|base| web_sys::Url::new_with_base(&module, &base).ok())
                    .map(|url| {
                        format!(
                            "./screen-worker.js?module={}",
                            js_sys::encode_uri_component(&url.href())
                        )
                    })
            })
            .unwrap_or_else(|| "./screen-worker.js".into());
        let worker = web_sys::SharedWorker::new_with_worker_options(&worker_url, &options)
            .map_err(|e| format!("SharedWorker unavailable: {e:?}"))?;
        let inbox = Rc::new(RefCell::new(Inbox::default()));
        let onmessage = Self::receive(ctx.clone(), inbox.clone());
        let errors = inbox.clone();
        let context = ctx.clone();
        let onerror = Closure::new(move |_: web_sys::Event| {
            errors.borrow_mut().error = Some(
                "Simulation Worker failed to load; use the packaged browser build, served over HTTP(S)."
                    .into(),
            );
            context.request_repaint();
        });
        worker
            .port()
            .set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        worker.set_onerror(Some(onerror.as_ref().unchecked_ref()));
        worker.port().start();
        worker
            .port()
            .post_message(
                &json!({"type":"start","seed":seed,"script":script})
                    .to_string()
                    .into(),
            )
            .map_err(|e| format!("{e:?}"))?;
        Ok(Self {
            worker: Some(worker),
            socket: None,
            onmessage,
            onerror,
            inbox,
        })
    }
    fn remote(ctx: &egui::Context, token: &str, name: &str) -> Result<Self, String> {
        let location = web_sys::window().ok_or("browser unavailable")?.location();
        let protocol = if location.protocol().map_err(|e| format!("{e:?}"))? == "https:" {
            "wss"
        } else {
            "ws"
        };
        let socket = web_sys::WebSocket::new(&format!(
            "{protocol}://{}/screens",
            location.host().map_err(|e| format!("{e:?}"))?
        ))
        .map_err(|e| format!("{e:?}"))?;
        let inbox = Rc::new(RefCell::new(Inbox::default()));
        let onmessage = Self::receive(ctx.clone(), inbox.clone());
        let errors = inbox.clone();
        let context = ctx.clone();
        let onerror = Closure::new(move |_: web_sys::Event| {
            errors.borrow_mut().error =
                Some("Panel connection closed or failed. Reconnect with the token shown on the host.".into());
            context.request_repaint();
        });
        socket.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        socket.set_onerror(Some(onerror.as_ref().unchecked_ref()));
        socket.set_onclose(Some(onerror.as_ref().unchecked_ref()));
        // Queue the hello until the egui update observes OPEN; callbacks stay owned by Link.
        let _ = (token, name);
        Ok(Self {
            worker: None,
            socket: Some(socket),
            onmessage,
            onerror,
            inbox,
        })
    }
    fn send(&self, value: Value) -> Result<(), String> {
        let text = value.to_string();
        if let Some(worker) = &self.worker {
            worker
                .port()
                .post_message(&wasm_bindgen::JsValue::from_str(&text))
                .map_err(|e| format!("{e:?}"))?;
        }
        if let Some(socket) = &self.socket {
            socket.send_with_str(&text).map_err(|e| format!("{e:?}"))?;
        }
        Ok(())
    }
}
impl Drop for Link {
    fn drop(&mut self) {
        if let Some(worker) = &self.worker {
            let _ = worker
                .port()
                .post_message(&json!({"type":"disconnect"}).to_string().into());
            worker.port().set_onmessage(None);
            worker.set_onerror(None);
            worker.port().close();
        }
        if let Some(socket) = &self.socket {
            socket.set_onmessage(None);
            socket.set_onerror(None);
            socket.set_onclose(None);
            let _ = socket.close();
        }
        let _ = (&self.onmessage, &self.onerror);
    }
}
pub struct ScreenApp {
    session: Session,
    canvas: Canvas,
    link: Option<Link>,
    remote: bool,
    paired: bool,
    token: String,
    name: String,
    screen: u64,
    page: Option<PageKind>,
    next_id: u64,
    last_poll: f64,
    error: Option<String>,
    renderer: &'static str,
    import: String,
    layout: Option<Layout>,
    input: crate::input::Input,
    keymap: crate::keymap::Keymap,
    audio: crate::audio::Audio,
    radio: crate::radio::RadioUi,
    previous: f64,
    radio_ptt: [bool; 2],
    glance: Option<[f64; 2]>,
}
impl ScreenApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        options: crate::AppOptions,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        theme::install(&cc.egui_ctx);
        let render = cc
            .wgpu_render_state
            .as_ref()
            .ok_or_else(|| std::io::Error::other("wgpu renderer required"))?;
        flightcraft_render::initialize(render, theme::scene());
        flightcraft_render::initialize_synthetic_vision(render, &cc.egui_ctx);
        let renderer = match render.adapter.get_info().backend {
            eframe::wgpu::Backend::BrowserWebGpu => "WebGPU",
            eframe::wgpu::Backend::Gl => "WebGL 2",
            _ => "wgpu",
        };
        let mut session = Session::new()?;
        for region in options.regions.fetched {
            session.install_region(region)?;
        }
        for pack in options.packs.fetched {
            session.install_pack(&pack)?;
        }
        session.enable_app();
        if let Some(saved) = cc.storage.and_then(|s| s.get_string("flightcraft.screens.v1"))
            && saved.len() <= 1024 * 1024
        {
            if let Ok(set) = serde_json::from_str(&saved) {
                let _ = session.restore_screens(set);
            }
        }
        if let Some(world) = session.worker_seed().regions.first() {
            let choice = world.region.id.clone();
            session.execute("sim.reset", json!({"world":choice,"start":{"kind":"runway"}}))?;
        }
        let window = web_sys::window().ok_or_else(|| std::io::Error::other("browser unavailable"))?;
        let query = web_sys::UrlSearchParams::new_with_str(&window.location().search().unwrap_or_default())
            .map_err(|_| std::io::Error::other("invalid query"))?;
        let remote = query.get("panel").as_deref() == Some("remote");
        let screen = query.get("screen").and_then(|s| s.parse().ok()).unwrap_or(0);
        let page = query
            .get("page")
            .and_then(|s| serde_json::from_value(Value::String(s)).ok());
        if !remote && let Some(layout) = &options.layout {
            session.execute("layout.set", json!({"id":0,"layout":layout}))?;
        }
        let layout = if remote { options.layout } else { None };
        let script = options
            .script
            .unwrap_or_default()
            .jsonl()
            .map_err(std::io::Error::other)?;
        let link = if remote {
            None
        } else {
            Some(Link::worker(&cc.egui_ctx, session.worker_seed(), script).map_err(std::io::Error::other)?)
        };
        Ok(Self {
            session,
            canvas: Canvas::default(),
            link,
            remote,
            paired: !remote,
            token: String::new(),
            name: "Browser screen".into(),
            screen,
            page,
            next_id: 1,
            last_poll: 0.,
            error: None,
            renderer,
            import: String::new(),
            layout,
            input: Default::default(),
            keymap: Default::default(),
            audio: crate::audio::Audio::with_preferences(Default::default()),
            radio: Default::default(),
            previous: 0.,
            radio_ptt: [false; 2],
            glance: None,
        })
    }
    pub fn renderer(&self) -> &'static str {
        self.renderer
    }
}
impl eframe::App for ScreenApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = ctx.input(|i| i.time);
        if let Some(link) = &self.link {
            if let Some(socket) = &link.socket
                && socket.ready_state() == web_sys::WebSocket::OPEN
            {
                if !self.paired {
                    let result =
                        socket.send_with_str(&json!({"token":self.token,"name":self.name}).to_string());
                    if result.is_ok() {
                        self.paired = true;
                        link.inbox.borrow_mut().error = None;
                    }
                }
                if now - self.last_poll > 1. / 15. {
                    let _ = link.send(json!({"type":"snapshot"}));
                    self.last_poll = now;
                }
            }
            let mut inbox = link.inbox.borrow_mut();
            if let Some(regions) = inbox.regions.take()
                && let Err(e) = self.session.install_screen_regions(regions)
            {
                self.error = Some(e);
            }
            if let Some(snapshot) = inbox.latest.take()
                && let Err(e) = self.session.apply_screen_snapshot(snapshot)
            {
                self.error = Some(e);
            }
            if let Some(e) = inbox.error.take() {
                self.error = Some(e);
            }
        }
        let control = self.link.as_ref().is_some_and(|l| l.inbox.borrow().permission);
        let elapsed = (now - self.previous).clamp(0., 0.1);
        self.previous = now;
        if ctx.input(|i| i.pointer.any_pressed()) {
            self.audio.user_gesture();
        }
        let frame = self.input.collect(ctx, &self.keymap);
        if control {
            use crate::keymap::Action;
            if frame.pressed(Action::Pause) {
                let _ = self.session.execute(
                    if self.session.paused() {
                        "sim.resume"
                    } else {
                        "sim.pause"
                    },
                    json!({}),
                );
            }
            if frame.pressed(Action::Reset) {
                let _ = crate::actions::reset(&mut self.session);
            }
            if frame.pressed(Action::Camera) {
                let _ = crate::actions::toggle_camera(&mut self.session);
            }
            let held = [frame.held(Action::ComPtt), frame.held(Action::IntercomPtt)];
            if held != self.radio_ptt {
                self.radio_ptt = held;
                let _ = self
                    .session
                    .execute("comms.audio_panel", json!({"ptt":held[0],"intercom_ptt":held[1]}));
            }
            let [yaw, pitch] = frame.cockpit_look();
            if yaw != 0. || pitch != 0. {
                if let Some(camera) = self.session.camera() {
                    let [base_yaw, base_pitch] = *self
                        .glance
                        .get_or_insert([camera.look_yaw_rad, camera.look_pitch_rad]);
                    let _ = crate::actions::cockpit_look(
                        &mut self.session,
                        base_yaw + yaw * 1.35,
                        base_pitch + pitch * 0.9,
                    );
                }
            } else if let Some([yaw, pitch]) = self.glance.take() {
                let _ = crate::actions::cockpit_look(&mut self.session, yaw, pitch);
            }
            if frame.pressed(Action::PlanningMap) {
                self.page = if self.page == Some(PageKind::Map) {
                    None
                } else {
                    Some(PageKind::Map)
                };
            }
            for action in [
                Action::ComPtt,
                Action::IntercomPtt,
                Action::RadioPanel,
                Action::Kneeboard,
                Action::RadioTransmit,
            ] {
                if frame.pressed(action) {
                    self.radio.shortcut(action);
                }
            }
            if !self.remote && self.screen == 0 {
                self.input.trim = self.session.sim().systems().controls.trim;
                let controls = self
                    .input
                    .controls(&frame, self.session.sim().controls(), elapsed);
                if self.input.trim != self.session.sim().systems().controls.trim {
                    let _ = self.session.execute(
                        "systems.set",
                        json!({"setting":{"kind":"trim","value":self.input.trim}}),
                    );
                }
                let request = json!({"id":0,"source":"keyboard","command":"sim.set_controls","input":{"elevator":controls.elevator,"aileron":controls.aileron,"rudder":controls.rudder,"throttle":controls.throttle,"flaps":controls.flaps,"brake":controls.brake}});
                if let Some(link) = &self.link {
                    let _ = link.send(json!({"type":"command","request":request}));
                }
            }
        }
        self.audio.observe(
            self.session.flight_generation(),
            self.session
                .sim()
                .installed_autopilot()
                .map_or(0, |a| a.disconnect_sequence),
        );
        self.radio.show(
            ctx,
            &mut self.session,
            &self.keymap,
            self.audio.preferences.gain(),
        );
        egui::TopBottomPanel::top("screen-toolbar").show(ctx,|ui|{
            ui.horizontal_wrapped(|ui|{
                ui.strong("FLIGHTCRAFT · SCREENS");ui.label(if control{"CONTROL"}else{"READ ONLY"});
                egui::ComboBox::from_id_salt("page").selected_text(self.page.map_or("Window layout",PageKind::label)).show_ui(ui,|ui|{ui.selectable_value(&mut self.page,None,"Window layout");for kind in PageKind::ALL{ui.selectable_value(&mut self.page,Some(kind),kind.label());}});
                if ui.add_enabled(control,egui::Checkbox::new(&mut self.canvas.edit,"Edit layout")).changed() && self.canvas.edit {
                    if let Some(kind)=self.page.take() { let _=self.session.execute("screen.set_page",json!({"id":self.screen,"page":{"kind":kind}})); }
                    if let Some(layout)=self.layout.take() { let _=self.session.execute("layout.set",json!({"id":self.screen,"layout":layout})); }
                }
                if !self.remote&&ui.button("Open another screen").clicked(){
                    let page=self.page.unwrap_or(PageKind::Panel);
                    match self.session.execute("screen.open",json!({"page":{"kind":page}})) {
                        Ok(screen)=>{if let Some(id)=screen.get("id").and_then(Value::as_u64)&&let Some(window)=web_sys::window(){let _=window.open_with_url(&format!("?screen={id}"));}},
                        Err(error)=>self.error=Some(error.to_string()),
                    }
                }
                if ui.button("Undo layout").clicked()&&control{let _=self.session.execute("layout.undo",json!({}));}
                ui.menu_button("Import / export layout",|ui|{ui.label("Paste a layout JSON document");ui.add(egui::TextEdit::multiline(&mut self.import).desired_rows(4));if ui.add_enabled(control,egui::Button::new("Apply layout")).clicked(){match Layout::from_json(&self.import){Ok(layout)=>{if let Err(e)=self.session.execute("layout.set",json!({"id":self.screen,"layout":layout})){self.error=Some(e.to_string());}},Err(e)=>self.error=Some(e)}}if ui.button("Copy layout JSON").clicked()&&let Some(screen)=self.session.screens().windows.iter().find(|w|w.id==self.screen)&&let Ok(text)=serde_json::to_string_pretty(&screen.layout){ctx.copy_text(text);}});
            });
            if self.remote&&self.link.is_none(){ui.label("Enter the pairing token shown in the host's Screens menu. The host grants control separately.");ui.horizontal(|ui|{ui.label("Screen name");ui.text_edit_singleline(&mut self.name);ui.label("Token");ui.add(egui::TextEdit::singleline(&mut self.token).password(true));if ui.button("Pair").clicked(){match Link::remote(ctx,&self.token,&self.name){Ok(link)=>{self.link=Some(link);self.paired=false;},Err(e)=>self.error=Some(e)}}});}
            if self.remote&&ui.button("Disconnect / reconnect").clicked(){self.link=None;self.paired=false;}
            if let Some(error)=&self.error{ui.colored_label(theme::AMBER,error);}
        });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BACKGROUND))
            .show(ctx, |ui| {
                if let Some(mut screen) = self
                    .session
                    .screens()
                    .windows
                    .iter()
                    .find(|w| w.id == self.screen)
                    .cloned()
                {
                    if let Some(layout) = &self.layout {
                        screen.layout = layout.clone();
                    }
                    if let Some(kind) = self.page {
                        screen.layout = Layout::single(Page::new(kind));
                    }
                    self.canvas.show(ui, &screen, &mut self.session, control);
                }
            });
        for (command, input) in self.session.take_screen_commands() {
            if !control {
                continue;
            }
            let request = json!({"id":self.next_id,"command":command,"input":input});
            self.next_id = self.next_id.wrapping_add(1);
            if let Some(link) = &self.link {
                let message = if self.remote {
                    request
                } else {
                    json!({"type":"command","request":request})
                };
                if let Err(e) = link.send(message) {
                    self.error = Some(e);
                }
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(1. / 60.));
    }
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Ok(layout) = serde_json::to_string(self.session.screens()) {
            storage.set_string("flightcraft.screens.v1", layout);
        }
    }
}
