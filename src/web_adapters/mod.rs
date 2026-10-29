//! Browser implementations of the `logic::ports` traits: `fetch` transport and IndexedDB persistence.
//! Everything here is wasm-only; the platform-independent logic stays in `crate::logic`.

pub mod fetch;
pub mod idb;
pub mod image_loader;
pub mod local_storage;
pub mod selftest;

/// `Date.now()` as the logic layer's clock.
pub struct WebClock;

impl crate::logic::ports::Clock for WebClock {
    fn now_ms(&self) -> i64 {
        js_sys::Date::now() as i64
    }
}

/// `[perf] label @ms` to the console, relative to page start (cheap start-up profiling).
pub fn mark(label: &str) {
    let t = web_sys::window().and_then(|w| w.performance()).map(|p| p.now()).unwrap_or(0.);
    web_sys::console::log_1(&format!("[perf] {label} @{}ms", t as i64).into());
}
