//! IndexedDB transactions are acknowledged only on completion, never on request success alone.
use super::*;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Event, IdbDatabase, IdbRequest, IdbTransaction, IdbTransactionMode};
fn error(v: JsValue) -> String {
    v.as_string().unwrap_or_else(|| format!("IndexedDB: {v:?}"))
}
async fn request(request: &IdbRequest) -> Result<JsValue, String> {
    let (mut success, mut failure) = (None, None);
    let promise = js_sys::Promise::new(&mut |resolve, reject| {
        let req = request.clone();
        let ok = Closure::<dyn FnMut(Event)>::new(move |_| match req.result() {
            Ok(value) => {
                let _ = resolve.call1(&JsValue::UNDEFINED, &value);
            }
            Err(e) => {
                let _ = resolve.call1(&JsValue::UNDEFINED, &e);
            }
        });
        let fail = Closure::<dyn FnMut(Event)>::new(move |_| {
            let _ = reject.call1(
                &JsValue::UNDEFINED,
                &JsValue::from_str("IndexedDB request failed (storage may be denied or full)"),
            );
        });
        request.set_onsuccess(Some(ok.as_ref().unchecked_ref()));
        request.set_onerror(Some(fail.as_ref().unchecked_ref()));
        success = Some(ok);
        failure = Some(fail);
    });
    let result = JsFuture::from(promise).await.map_err(error);
    request.set_onsuccess(None);
    request.set_onerror(None);
    drop((success, failure));
    result
}
struct Commit {
    promise: js_sys::Promise,
    complete: Closure<dyn FnMut(Event)>,
    abort: Closure<dyn FnMut(Event)>,
    transaction: IdbTransaction,
}
impl Commit {
    fn new(transaction: &IdbTransaction) -> Self {
        let (mut complete, mut abort) = (None, None);
        let promise = js_sys::Promise::new(&mut |resolve, reject| {
            let ok = Closure::<dyn FnMut(Event)>::new(move |_| {
                let _ = resolve.call0(&JsValue::UNDEFINED);
            });
            let fail = Closure::<dyn FnMut(Event)>::new(move |_| {
                let _ = reject.call1(
                    &JsValue::UNDEFINED,
                    &JsValue::from_str("IndexedDB transaction aborted; data was not saved"),
                );
            });
            transaction.set_oncomplete(Some(ok.as_ref().unchecked_ref()));
            transaction.set_onabort(Some(fail.as_ref().unchecked_ref()));
            complete = Some(ok);
            abort = Some(fail);
        });
        // The Promise executor runs synchronously. Fallback callbacks keep this constructor total.
        Self {
            promise,
            complete: complete.unwrap_or_else(|| Closure::new(|_: Event| {})),
            abort: abort.unwrap_or_else(|| Closure::new(|_: Event| {})),
            transaction: transaction.clone(),
        }
    }
    async fn wait(self) -> Result<(), String> {
        JsFuture::from(self.promise.clone())
            .await
            .map(|_| ())
            .map_err(error)
    }
}
impl Drop for Commit {
    fn drop(&mut self) {
        self.transaction.set_oncomplete(None);
        self.transaction.set_onabort(None);
        let _ = (&self.complete, &self.abort);
    }
}
async fn open() -> Result<IdbDatabase, String> {
    let factory = web_sys::window()
        .ok_or("no browser window")?
        .indexed_db()
        .map_err(error)?
        .ok_or("IndexedDB unavailable")?;
    let opening = factory.open_with_u32("flightcraft-flights", 1).map_err(error)?;
    let r = opening.clone();
    let upgrade = Closure::<dyn FnMut(Event)>::new(move |_| {
        if let Ok(value) = r.result()
            && let Ok(db) = value.dyn_into::<IdbDatabase>()
        {
            for name in ["flights", "summaries"] {
                if !db.object_store_names().contains(name) {
                    let _ = db.create_object_store(name);
                }
            }
        }
    });
    opening.set_onupgradeneeded(Some(upgrade.as_ref().unchecked_ref()));
    let value = request(opening.unchecked_ref::<IdbRequest>()).await;
    opening.set_onupgradeneeded(None);
    drop(upgrade);
    value?.dyn_into().map_err(error)
}
fn begin(state: &Arc<Mutex<ArchiveStatus>>) -> Result<(), String> {
    let mut s = state.lock().map_err(|_| "archive unavailable")?;
    s.pending_operations += 1;
    s.busy = true;
    s.error = None;
    Ok(())
}
fn end(state: &Arc<Mutex<ArchiveStatus>>, result: Result<(), String>) {
    if let Ok(mut s) = state.lock() {
        s.pending_operations = s.pending_operations.saturating_sub(1);
        s.busy = s.pending_operations > 0;
        if let Err(e) = result {
            s.error = Some(e);
        }
    }
}
pub(super) fn save(state: Arc<Mutex<ArchiveStatus>>, saved: SavedFlight) -> Result<(), String> {
    begin(&state)?;
    wasm_bindgen_futures::spawn_local(async move {
        let result = async {
            let text = serde_json::to_string(&saved).map_err(|e| e.to_string())?;
            if text.len() > 128 * 1024 * 1024 {
                return Err("flight archive exceeds 128 MiB".into());
            }
            let db = open().await?;
            let names = js_sys::Array::new();
            names.push(&"flights".into());
            names.push(&"summaries".into());
            let tx = db
                .transaction_with_str_sequence_and_mode(&names, IdbTransactionMode::Readwrite)
                .map_err(error)?;
            let commit = Commit::new(&tx);
            let key = JsValue::from_str(&saved.summary.id);
            let flight = tx
                .object_store("flights")
                .map_err(error)?
                .put_with_key(&JsValue::from_str(&text), &key)
                .map_err(error)?;
            tx.object_store("summaries")
                .map_err(error)?
                .put_with_key(
                    &JsValue::from_str(&serde_json::to_string(&saved.summary).map_err(|e| e.to_string())?),
                    &key,
                )
                .map_err(error)?;
            let request_result = request(&flight).await;
            let commit_result = commit.wait().await;
            db.close();
            request_result?;
            commit_result?;
            if let Ok(mut s) = state.lock() {
                s.last_saved = Some(saved.summary.id);
            }
            Ok(())
        }
        .await;
        end(&state, result);
    });
    Ok(())
}
pub(super) fn list(state: Arc<Mutex<ArchiveStatus>>) -> Result<(), String> {
    begin(&state)?;
    wasm_bindgen_futures::spawn_local(async move {
        let result = async {
            let db = open().await?;
            let tx = db.transaction_with_str("summaries").map_err(error)?;
            let commit = Commit::new(&tx);
            let req = tx
                .object_store("summaries")
                .map_err(error)?
                .get_all_with_key_and_limit(&JsValue::UNDEFINED, 10001)
                .map_err(error)?;
            let value = request(&req).await;
            let committed = commit.wait().await;
            db.close();
            committed?;
            let values = js_sys::Array::from(&value?);
            if values.length() > 10000 {
                return Err("flight listing exceeds 10000 entries".into());
            }
            let mut flights = Vec::<FlightSummary>::new();
            for value in values.iter() {
                let text = value.as_string().ok_or("invalid saved flight summary")?;
                if text.len() > 4096 {
                    return Err("oversized flight summary".into());
                }
                flights.push(serde_json::from_str(&text).map_err(|e| e.to_string())?);
            }
            flights.sort_by_key(|f| std::cmp::Reverse(f.created_unix_ms));
            if let Ok(mut s) = state.lock() {
                s.flights = flights;
            }
            Ok(())
        }
        .await;
        end(&state, result);
    });
    Ok(())
}
pub(super) fn load(state: Arc<Mutex<ArchiveStatus>>, id: String) -> Result<(), String> {
    begin(&state)?;
    wasm_bindgen_futures::spawn_local(async move {
        let result = async {
            let db = open().await?;
            let tx = db.transaction_with_str("flights").map_err(error)?;
            let commit = Commit::new(&tx);
            let req = tx
                .object_store("flights")
                .map_err(error)?
                .get(&JsValue::from_str(&id))
                .map_err(error)?;
            let value = request(&req).await;
            let committed = commit.wait().await;
            db.close();
            committed?;
            let saved = SavedFlight::from_json(&value?.as_string().ok_or("saved flight not found")?)?;
            if saved.summary.id != id {
                return Err("flight id differs from storage key".into());
            }
            if let Ok(mut s) = state.lock() {
                s.loaded = Some(saved);
            }
            Ok(())
        }
        .await;
        end(&state, result);
    });
    Ok(())
}
