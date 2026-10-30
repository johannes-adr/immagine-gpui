//! Browser implementation of [`crate::host::Host`]: `fetch`, IndexedDB, `localStorage`, `createImageBitmap`,
//! a DOM `<video>` and pointer events. Everything here is wasm-only.

pub mod entry;
pub mod fetch;
pub mod idb;
pub mod image_loader;
pub mod local_storage;
pub mod pinch;
pub mod selftest;
pub mod video;

use crate::host::{Decoded, Host, Pinch};

/// `Date.now()` as the logic layer's clock.
pub struct WebClock;

impl crate::logic::ports::Clock for WebClock {
    fn now_ms(&self) -> i64 {
        js_sys::Date::now() as i64
    }
}

mod js {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = performance, js_name = now)]
        pub fn now() -> f64;
    }
}

pub struct Web;

impl Host for Web {
    type Transport = fetch::FetchTransport;
    type Clock = WebClock;
    type Video = video::VideoHandle;
    type Gestures = pinch::PinchHandle;

    fn transport() -> Self::Transport {
        fetch::FetchTransport
    }

    fn clock() -> Self::Clock {
        WebClock
    }

    fn origin() -> String {
        web_sys::window().and_then(|w| w.location().origin().ok()).unwrap_or_default()
    }

    fn now_ms() -> f64 {
        js::now()
    }

    /// `[perf] label @ms` to the console, relative to page start.
    fn mark(label: &str) {
        let t = web_sys::window().and_then(|w| w.performance()).map(|p| p.now()).unwrap_or(0.);
        web_sys::console::log_1(&format!("[perf] {label} @{}ms", t as i64).into());
    }

    fn setting_get(key: &str) -> Option<String> {
        local_storage::get(key)
    }

    fn setting_set(key: &str, value: &str) {
        local_storage::set(key, value)
    }

    fn setting_remove(key: &str) {
        local_storage::remove(key)
    }

    async fn blob_get(db: &str, key: &str) -> Result<Option<Vec<u8>>, String> {
        idb::get(db, key).await
    }

    async fn blob_put(db: &str, key: &str, bytes: &[u8]) -> Result<(), String> {
        idb::put(db, key, bytes).await
    }

    async fn decode_url(url: &str) -> Result<Decoded, String> {
        image_loader::decode_url(url).await
    }

    async fn share_file(name: &str, mime: &str, bytes: Vec<u8>) -> Result<(), String> {
        let share = js_sys::Function::new_with_args(
            "name,mime,bytes",
            "if(!navigator.share)return Promise.reject('Teilen wird hier nicht unterstützt');\
             const f=new File([bytes],name,{type:mime});\
             return navigator.share({files:[f]});",
        );
        let array = js_sys::Uint8Array::from(bytes.as_slice());
        let promise = share
            .call3(&wasm_bindgen::JsValue::NULL, &name.into(), &mime.into(), &array)
            .map_err(|e| format!("{e:?}"))?;
        wasm_bindgen_futures::JsFuture::from(js_sys::Promise::from(promise)).await.map(|_| ()).map_err(|e| format!("{e:?}"))
    }

    fn show_video(url: &str, top: f32, bottom: f32) -> Option<Self::Video> {
        video::show(url, top, bottom)
    }

    fn install_gestures(on_event: impl Fn(Pinch) + 'static) -> Option<Self::Gestures> {
        pinch::install(on_event)
    }
}
