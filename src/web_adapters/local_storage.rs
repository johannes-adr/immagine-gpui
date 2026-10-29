//! Thin `localStorage` wrapper for small per-device values (the session token).
//! Every value is mirrored into a long-lived cookie, so it survives when Safari clears `localStorage`
//! (iOS evicts script-writable storage after a week without use; cookies set by script share that limit
//! only partly, and the Immich cookie from the proxy is a second line of defence).
//! Every call degrades to "nothing stored" when storage is blocked (private windows).

use wasm_bindgen::JsValue;

const YEAR: u32 = 400 * 24 * 3600;

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

fn cookie() -> String {
    let doc = web_sys::window().and_then(|w| w.document());
    doc.and_then(|d| js_sys::Reflect::get(&d, &"cookie".into()).ok()).and_then(|v| v.as_string()).unwrap_or_default()
}

fn set_cookie(value: &str) {
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        let _ = js_sys::Reflect::set(&doc, &"cookie".into(), &JsValue::from_str(value));
    }
}

pub fn get(key: &str) -> Option<String> {
    if let Some(v) = storage().and_then(|s| s.get_item(key).ok().flatten()).filter(|v| !v.is_empty()) {
        return Some(v);
    }
    let prefix = format!("{key}=");
    cookie().split("; ").find_map(|c| c.strip_prefix(&prefix).map(str::to_owned)).filter(|v| !v.is_empty())
}

pub fn set(key: &str, value: &str) {
    if let Some(s) = storage() {
        let _ = s.set_item(key, value);
    }
    set_cookie(&format!("{key}={value}; Max-Age={YEAR}; Path=/; SameSite=Lax; Secure"));
}

pub fn remove(key: &str) {
    if let Some(s) = storage() {
        let _ = s.remove_item(key);
    }
    set_cookie(&format!("{key}=; Max-Age=0; Path=/; SameSite=Lax; Secure"));
}
