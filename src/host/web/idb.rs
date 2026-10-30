//! A minimal IndexedDB key-value store (one object store, `kv`) for `MemoryStore` snapshots.
//! localStorage cannot hold a 25k-asset snapshot (megabytes), IndexedDB can.

use js_sys::{Promise, Uint8Array};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{EventTarget, IdbDatabase, IdbOpenDbRequest, IdbTransactionMode};

const STORE: &str = "kv";

fn err(error: JsValue) -> String {
    error.as_string().unwrap_or_else(|| format!("{error:?}"))
}

/// Resolves on the first `ok` event of `target`, rejects on `fail`.
fn once(target: &EventTarget, ok: &str, fail: &str) -> JsFuture {
    let promise = Promise::new(&mut |resolve, reject| {
        let on_ok = Closure::once_into_js(move || {
            let _ = resolve.call0(&JsValue::NULL);
        });
        let on_fail = Closure::once_into_js(move || {
            let _ = reject.call1(&JsValue::NULL, &JsValue::from_str("IndexedDB request failed"));
        });
        let _ = target.add_event_listener_with_callback(ok, on_ok.unchecked_ref());
        let _ = target.add_event_listener_with_callback(fail, on_fail.unchecked_ref());
    });
    JsFuture::from(promise)
}

async fn open(name: &str) -> Result<IdbDatabase, String> {
    let factory = web_sys::window()
        .ok_or("no window")?
        .indexed_db()
        .map_err(err)?
        .ok_or("IndexedDB unavailable")?;
    let request: IdbOpenDbRequest = factory.open_with_u32(name, 1).map_err(err)?;
    let upgrade = Closure::once_into_js({
        let request = request.clone();
        move || {
            if let Ok(db) = request.result().and_then(|r| r.dyn_into::<IdbDatabase>()) {
                let _ = db.create_object_store(STORE);
            }
        }
    });
    request.set_onupgradeneeded(Some(upgrade.unchecked_ref()));
    once(&request, "success", "error").await.map_err(err)?;
    request.result().map_err(err)?.dyn_into().map_err(err)
}

pub async fn put(db_name: &str, key: &str, bytes: &[u8]) -> Result<(), String> {
    let db = open(db_name).await?;
    let tx = db.transaction_with_str_and_mode(STORE, IdbTransactionMode::Readwrite).map_err(err)?;
    tx.object_store(STORE)
        .map_err(err)?
        .put_with_key(&Uint8Array::from(bytes), &JsValue::from_str(key))
        .map_err(err)?;
    // `complete` means the data is durable, not only queued.
    once(&tx, "complete", "error").await.map_err(err)?;
    db.close();
    Ok(())
}

pub async fn get(db_name: &str, key: &str) -> Result<Option<Vec<u8>>, String> {
    let db = open(db_name).await?;
    let tx = db.transaction_with_str_and_mode(STORE, IdbTransactionMode::Readonly).map_err(err)?;
    let request = tx.object_store(STORE).map_err(err)?.get(&JsValue::from_str(key)).map_err(err)?;
    once(&request, "success", "error").await.map_err(err)?;
    let value = request.result().map_err(err)?;
    db.close();
    Ok(if value.is_undefined() { None } else { Some(Uint8Array::new(&value).to_vec()) })
}
