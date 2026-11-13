//! Browser SpeechRecognition fallback, explicitly opted into in Preferences.
use super::pipeline::Recognized;
use js_sys::{Function, Reflect};
use std::{cell::RefCell, collections::VecDeque, rc::Rc};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
type Completion = (u64, bool, Result<Recognized, String>);
#[derive(Default)]
struct State {
    active: bool,
    serial: u64,
    epoch: u64,
    intercom: bool,
    vox: bool,
    released: f64,
    recognition: Option<JsValue>,
    stream: Option<web_sys::MediaStream>,
    context: Option<web_sys::AudioContext>,
    analyser: Option<web_sys::AnalyserNode>,
    result: Option<Closure<dyn FnMut(JsValue)>>,
    error: Option<Closure<dyn FnMut(JsValue)>>,
    queue: VecDeque<Completion>,
}
#[derive(Default)]
pub struct Browser {
    state: Rc<RefCell<State>>,
    requested: bool,
    started_ms: f64,
}
fn property(value: &JsValue, name: &str) -> Result<JsValue, String> {
    Reflect::get(value, &name.into()).map_err(|_| "Browser speech recognition is unavailable.".into())
}
fn call(value: &JsValue, name: &str, argument: Option<&JsValue>) -> Result<(), String> {
    let method: Function = property(value, name)?
        .dyn_into()
        .map_err(|_| "Browser speech recognition is unsupported.")?;
    let result = if let Some(argument) = argument {
        method.call1(value, argument)
    } else {
        method.call0(value)
    };
    result.map(|_| ()).map_err(|_| {
        "Microphone access or browser speech recognition was refused. Check site permissions.".into()
    })
}
fn stop_tracks(state: &mut State) {
    if let Some(stream) = state.stream.take() {
        for track in stream
            .get_tracks()
            .iter()
            .filter_map(|v| v.dyn_into::<web_sys::MediaStreamTrack>().ok())
        {
            track.stop();
        }
    }
    if let Some(context) = state.context.take() {
        let _ = context.close();
    }
    state.analyser = None;
}
impl Browser {
    pub fn cancel(&mut self) {
        self.requested = false;
        let Ok(mut state) = self.state.try_borrow_mut() else {
            return;
        };
        state.active = false;
        state.serial = state.serial.wrapping_add(1);
        if let Some(recognition) = state.recognition.take() {
            let _ = Reflect::set(&recognition, &"onresult".into(), &JsValue::NULL);
            let _ = Reflect::set(&recognition, &"onerror".into(), &JsValue::NULL);
            let _ = call(&recognition, "abort", None);
        }
        stop_tracks(&mut state);
        state.result = None;
        state.error = None;
        state.queue.clear();
    }
    pub fn requested(&self) -> bool {
        self.requested
    }
    pub fn observe(&mut self, active: bool, intercom: bool, vox: bool, epoch: u64) {
        let route_changed = active
            && self.requested
            && self
                .state
                .try_borrow()
                .is_ok_and(|state| state.intercom != intercom);
        if route_changed {
            self.cancel();
        }
        if self.requested == active {
            if active
                && js_sys::Date::now() - self.started_ms >= 29_000.
                && let Ok(mut state) = self.state.try_borrow_mut()
                && state.active
            {
                state.active = false;
                state.released = js_sys::Date::now();
                if let Some(recognition) = &state.recognition {
                    let _ = call(recognition, "stop", None);
                }
                stop_tracks(&mut state);
            }
            return;
        }
        self.started_ms = js_sys::Date::now();
        self.requested = active;
        if !active {
            let Ok(mut state) = self.state.try_borrow_mut() else {
                return;
            };
            state.active = false;
            state.released = js_sys::Date::now();
            if let Some(recognition) = &state.recognition {
                let _ = call(recognition, "stop", None);
            }
            stop_tracks(&mut state);
            return;
        }
        let serial = {
            let Ok(mut state) = self.state.try_borrow_mut() else {
                return;
            };
            if let Some(recognition) = state.recognition.take() {
                let _ = Reflect::set(&recognition, &"onresult".into(), &JsValue::NULL);
                let _ = Reflect::set(&recognition, &"onerror".into(), &JsValue::NULL);
                let _ = call(&recognition, "abort", None);
            }
            state.result = None;
            state.error = None;
            state.active = true;
            state.epoch = epoch;
            state.intercom = intercom;
            state.vox = vox;
            state.released = js_sys::Date::now();
            state.serial = state.serial.wrapping_add(1);
            state.serial
        };
        let shared = self.state.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(error) = start(shared.clone(), serial).await {
                let Ok(mut state) = shared.try_borrow_mut() else {
                    return;
                };
                if state.serial != serial {
                    return;
                }
                state.queue.push_back((epoch, intercom, Err(error)));
                state.active = false;
                stop_tracks(&mut state);
            }
        });
    }
    pub fn level(&self) -> f32 {
        let Ok(state) = self.state.try_borrow() else {
            return 0.;
        };
        if let Some(analyser) = &state.analyser {
            let mut values = [128u8; 256];
            analyser.get_byte_time_domain_data(&mut values);
            values
                .iter()
                .map(|v| (f32::from(*v) - 128.).abs() / 128.)
                .fold(0., f32::max)
        } else {
            0.
        }
    }
    pub fn poll(&mut self) -> Option<Completion> {
        self.state.try_borrow_mut().ok()?.queue.pop_front()
    }
}
async fn start(shared: Rc<RefCell<State>>, serial: u64) -> Result<(), String> {
    let window = web_sys::window().ok_or("Browser window unavailable.")?;
    let ctor = property(&window, "SpeechRecognition")
        .ok()
        .filter(|v| v.is_function())
        .or_else(|| {
            property(&window, "webkitSpeechRecognition")
                .ok()
                .filter(|v| v.is_function())
        })
        .ok_or(
            "This browser does not support speech recognition. Use desktop offline speech or type a call.",
        )?;
    let recognition = Reflect::construct(
        &ctor
            .dyn_into::<Function>()
            .map_err(|_| "Speech constructor unavailable.")?,
        &js_sys::Array::new(),
    )
    .map_err(|_| "Browser speech unavailable.")?;
    let constraints = web_sys::MediaStreamConstraints::new();
    constraints.set_audio(&JsValue::TRUE);
    let media = window
        .navigator()
        .media_devices()
        .map_err(|_| "Microphone needs a secure page (HTTPS or localhost).")?;
    let stream: web_sys::MediaStream = wasm_bindgen_futures::JsFuture::from(
        media
            .get_user_media_with_constraints(&constraints)
            .map_err(|_| "Microphone request failed.")?,
    )
    .await
    .map_err(|_| "Microphone permission denied or no input device available.")?
    .dyn_into()
    .map_err(|_| "Microphone stream unavailable.")?;
    let mut state = shared
        .try_borrow_mut()
        .map_err(|_| "Speech state is busy; retry.")?;
    if !state.active || state.serial != serial {
        for track in stream
            .get_tracks()
            .iter()
            .filter_map(|v| v.dyn_into::<web_sys::MediaStreamTrack>().ok())
        {
            track.stop();
        }
        return Ok(());
    }
    state.stream = Some(stream.clone());
    let epoch = state.epoch;
    let intercom = state.intercom;
    let source = shared.clone();
    let onresult = Closure::wrap(Box::new(move |event: JsValue| {
        let Ok(mut state) = source.try_borrow_mut() else {
            return;
        };
        if state.epoch != epoch || state.serial != serial {
            return;
        }
        let result = (|| {
            let results = property(&event, "results")?;
            let index = property(&event, "resultIndex")?.as_f64().unwrap_or(0.) as u32;
            let result = Reflect::get(&results, &index.into()).map_err(|_| "Speech result unavailable.")?;
            if !property(&result, "isFinal")?.as_bool().unwrap_or(false) {
                return Err("Waiting for final recognition.".into());
            }
            let alternative =
                Reflect::get(&result, &0.into()).map_err(|_| "Speech alternative unavailable.")?;
            let text = property(&alternative, "transcript")?
                .as_string()
                .ok_or("No speech recognized.")?;
            if text.len() > 1024 {
                return Err("Call too long; say again in a shorter transmission.".into());
            }
            Ok(Recognized {
                text,
                confidence: property(&alternative, "confidence")?.as_f64().unwrap_or(0.) as f32,
                latency_ms: (js_sys::Date::now() - state.released).max(0.) as u64,
                alternatives: Vec::new(),
            })
        })();
        if state.queue.len() < 8 {
            state.queue.push_back((epoch, intercom, result));
        }
    }) as Box<dyn FnMut(JsValue)>);
    let source = shared.clone();
    let onerror = Closure::wrap(Box::new(move |_event: JsValue| {
        let Ok(mut state) = source.try_borrow_mut() else {
            return;
        };
        if state.serial == serial && state.queue.len() < 8 {
            state.queue.push_back((epoch, intercom, Err("Browser recognition failed. Check microphone permission and network access; hold PTT to retry.".into())));
        }
    }) as Box<dyn FnMut(JsValue)>);
    Reflect::set(&recognition, &"onresult".into(), onresult.as_ref())
        .map_err(|_| "Cannot receive speech results.")?;
    Reflect::set(&recognition, &"onerror".into(), onerror.as_ref())
        .map_err(|_| "Cannot receive speech errors.")?;
    Reflect::set(&recognition, &"lang".into(), &"en-US".into()).map_err(|_| "Cannot set speech language.")?;
    Reflect::set(&recognition, &"continuous".into(), &state.vox.into())
        .map_err(|_| "Cannot configure speech.")?;
    Reflect::set(&recognition, &"interimResults".into(), &JsValue::FALSE)
        .map_err(|_| "Cannot configure speech.")?;
    let context = web_sys::AudioContext::new().map_err(|_| "Audio level meter unavailable.")?;
    state.context = Some(context.clone());
    let _ = context.resume();
    let analyser = context
        .create_analyser()
        .map_err(|_| "Audio level meter unavailable.")?;
    analyser.set_fft_size(256);
    let input = context
        .create_media_stream_source(&stream)
        .map_err(|_| "Microphone meter unavailable.")?;
    input
        .connect_with_audio_node(&analyser)
        .map_err(|_| "Microphone meter connection failed.")?;
    // The standard API uses the browser-selected microphone; getUserMedia grants access and meters it.
    call(&recognition, "start", None)?;
    state.stream = Some(stream);
    state.context = Some(context);
    state.analyser = Some(analyser);
    state.recognition = Some(recognition);
    state.result = Some(onresult);
    state.error = Some(onerror);
    Ok(())
}

impl Drop for Browser {
    fn drop(&mut self) {
        self.cancel();
    }
}
