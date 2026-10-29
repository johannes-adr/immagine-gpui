//! Thin `localStorage` wrapper for small per-device values (the session token).
//! Every call degrades to "nothing stored" when storage is blocked (private windows).

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

pub fn get(key: &str) -> Option<String> {
    storage()?.get_item(key).ok()?
}

pub fn set(key: &str, value: &str) {
    if let Some(s) = storage() {
        let _ = s.set_item(key, value);
    }
}

pub fn remove(key: &str) {
    if let Some(s) = storage() {
        let _ = s.remove_item(key);
    }
}
