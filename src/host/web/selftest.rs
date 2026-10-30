//! Browser smoke test for the adapters against a real Immich server (called from the console:
//! `await wasm.selftest(email, password)`). Credentials are never stored.

use super::{fetch::FetchTransport, idb};
use crate::logic::client::ImmichClient;
use crate::logic::store::{MemoryStore, Store};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub async fn selftest(email: String, password: String) -> Result<String, JsValue> {
    let fail = |e: String| JsValue::from_str(&e);
    let origin = web_sys::window().ok_or_else(|| fail("no window".into()))?.location().origin()?;
    let client = ImmichClient::new(&origin, FetchTransport).map_err(|e| fail(e.to_string()))?;

    let auth = client.login(&email, &password).await.map_err(|e| fail(e.to_string()))?;
    let token = auth.access_token.ok_or_else(|| fail("login returned no access token".into()))?;
    let page = client.search_assets(1, 100, &token).await.map_err(|e| fail(e.to_string()))?;
    let mut store = MemoryStore::new();
    store.upsert_assets(&page.assets.items);

    let bytes = store.snapshot().map_err(|e| fail(e.to_string()))?;
    idb::put("selftest", "store", &bytes).await.map_err(fail)?;
    let back = idb::get("selftest", "store").await.map_err(fail)?.ok_or_else(|| fail("snapshot missing".into()))?;
    let restored = MemoryStore::restore(&back).map_err(|e| fail(e.to_string()))?;

    Ok(format!(
        "server total={}, page={}, stored={}, snapshot={} bytes, restored={}",
        page.assets.total,
        page.assets.items.len(),
        store.asset_count(),
        bytes.len(),
        restored.asset_count()
    ))
}

/// Runs the real sync engine twice (full sync, then the incremental stream) against the server.
#[wasm_bindgen]
pub async fn sync_selftest(email: String, password: String) -> Result<String, JsValue> {
    use crate::logic::sync::engine::SyncEngine;
    let fail = |e: String| JsValue::from_str(&e);
    let origin = web_sys::window().ok_or_else(|| fail("no window".into()))?.location().origin()?;
    let login = ImmichClient::new(&origin, FetchTransport).map_err(|e| fail(e.to_string()))?;
    let token = login
        .login(&email, &password)
        .await
        .map_err(|e| fail(e.to_string()))?
        .access_token
        .ok_or_else(|| fail("no access token".into()))?;

    let mut engine = SyncEngine::new(MemoryStore::new(), FetchTransport, super::WebClock, &origin).map_err(|e| fail(e.to_string()))?;
    let t0 = js_sys::Date::now();
    let first = engine.sync(&token).await.map_err(|e| fail(format!("first sync: {e}")))?;
    let t1 = js_sys::Date::now();
    let second = engine.sync(&token).await.map_err(|e| fail(format!("second sync: {e}")))?;
    let t2 = js_sys::Date::now();
    Ok(format!(
        "first: full={} assets={} albums={} {}ms | second: full={} summary={:?} {}ms",
        first.full_sync,
        engine.store().asset_count(),
        engine.store().albums().len(),
        (t1 - t0) as i64,
        second.full_sync,
        second.stream.summary(),
        (t2 - t1) as i64,
    ))
}
