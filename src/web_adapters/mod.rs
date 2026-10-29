//! Browser implementations of the `logic::ports` traits: `fetch` transport and IndexedDB persistence.
//! Everything here is wasm-only; the platform-independent logic stays in `crate::logic`.

pub mod fetch;
pub mod idb;
pub mod selftest;

/// `Date.now()` as the logic layer's clock.
pub struct WebClock;

impl crate::logic::ports::Clock for WebClock {
    fn now_ms(&self) -> i64 {
        js_sys::Date::now() as i64
    }
}
