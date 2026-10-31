//! Browser implementation of [`crate::host::Host`]: `fetch`, IndexedDB, `localStorage`, `createImageBitmap`,
//! WebCodecs/WebAudio for video and pointer events. Everything here is wasm-only.

pub mod entry;
pub mod codecs;
pub mod fetch;
pub mod idb;
pub mod image_loader;
pub mod local_storage;
pub mod pinch;
pub mod selftest;

use crate::host::{AppInfo, Decoded, Host, Pinch, VideoConfig};

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

/// Key under which the app saves the server the user logged in to (`immagine::SERVER_KEY`).
const SERVER_KEY: &str = "immagine.server";
/// Key of the session token (`immagine::TOKEN_KEY`).
const TOKEN_KEY: &str = "immagine.token";

/// The saved server, else the page's own origin (the `serve.py` proxy). A saved server on another origin is
/// reached directly and needs CORS on its side; requests then carry the bearer token instead of the cookie.
pub(crate) fn origin() -> String {
    local_storage::get(SERVER_KEY)
        .map(|s| s.trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(origin_of_page)
}

/// The origin the page itself was loaded from.
pub(crate) fn origin_of_page() -> String {
    web_sys::window().and_then(|w| w.location().origin().ok()).unwrap_or_default()
}

pub struct Web;

impl Host for Web {
    type Transport = fetch::FetchTransport;
    type Clock = WebClock;
    type VideoDecoder = codecs::WebVideoDecoder;
    type AudioOut = codecs::WebAudioOut;
    type Gestures = pinch::PinchHandle;

    fn transport() -> Self::Transport {
        fetch::FetchTransport
    }

    fn clock() -> Self::Clock {
        WebClock
    }

    fn origin() -> String {
        origin()
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

    async fn blob_delete(db: &str, key: &str) -> Result<(), String> {
        idb::delete(db, key).await
    }

    async fn blob_clear(db: &str) -> Result<(), String> {
        idb::clear(db).await
    }

    async fn blob_size(db: &str) -> Result<u64, String> {
        idb::size(db).await
    }

    async fn blob_keys(db: &str) -> Result<Vec<String>, String> {
        idb::keys(db).await
    }

    fn renderer_name(webgpu: bool) -> &'static str {
        if webgpu { "WebGPU" } else { "WebGL2" }
    }

    async fn app_info() -> AppInfo {
        let probe = js_sys::Function::new_no_args(
            "return (async()=>{const k=('caches' in self)?await caches.keys():[];\
             const v=k.filter(n=>n.startsWith('immagine-')).sort().pop();\
             const e=(navigator.storage&&navigator.storage.estimate)?await navigator.storage.estimate().catch(()=>null):null;\
             return JSON.stringify({v:v?v.slice(9):null,u:e?e.usage:null,q:e?e.quota:null,\
             s:matchMedia('(display-mode: standalone)').matches||navigator.standalone===true,\
             t:self.crossOriginIsolated===true})})()",
        );
        let text = match probe.call0(&wasm_bindgen::JsValue::NULL) {
            Ok(promise) => wasm_bindgen_futures::JsFuture::from(js_sys::Promise::from(promise)).await.ok().and_then(|v| v.as_string()),
            Err(_) => None,
        };
        let json: serde_json::Value = text.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        AppInfo {
            pwa_version: json["v"].as_str().map(str::to_string),
            standalone: json["s"].as_bool().unwrap_or(false),
            threads: json["t"].as_bool().unwrap_or(false),
            storage_used: json["u"].as_f64().map(|n| n as u64),
            storage_quota: json["q"].as_f64().map(|n| n as u64),
            native: false,
        }
    }

    async fn decode_bytes(bytes: Vec<u8>, min_side: u32) -> Result<Decoded, String> {
        image_loader::decode_bytes(&bytes, min_side).await
    }

    async fn fetch_image(url: &str, cancel: &crate::host::Cancel) -> Result<Vec<u8>, String> {
        image_loader::fetch(url, cancel).await
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

    async fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
        fetch::bytes(url).await
    }

    async fn fetch_range(url: &str, start: u64, end_inclusive: u64) -> Result<crate::host::RangeResponse, String> {
        fetch::range(url, start, end_inclusive).await
    }

    fn video_decoder(config: VideoConfig, on_frame: impl Fn(i64, Decoded) + 'static) -> Option<Self::VideoDecoder> {
        codecs::WebVideoDecoder::new(config, on_frame)
    }

    fn audio_output() -> Option<Self::AudioOut> {
        codecs::WebAudioOut::new()
    }

    fn install_gestures(on_event: impl Fn(Pinch) + 'static) -> Option<Self::Gestures> {
        pinch::install(on_event)
    }
}
