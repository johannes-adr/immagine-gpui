//! Desktop implementation of [`crate::host::Host`] for the standard (non-web) GPUI platforms.
//!
//! * HTTP: `ureq` on a worker thread per request, one shared agent so the Immich cookie persists.
//! * Persistence: plain files below [`data_dir`].
//! * Images: downloaded and decoded with the `image` crate on a worker thread.
//! * Sharing: `UIActivityViewController` (iOS) / `NSSharingServicePicker` (macOS), see `share`; `Err` elsewhere.
//! * Video and audio: macOS and iOS decode H.264 with VideoToolbox and AAC with AudioToolbox (`apple_video`,
//!   `apple_audio`); other desktops have no decoder, `video_decoder` and `audio_output` return `None`.
//!   `install_gestures` reports pinch and raw touches from GPUI's touch observer (iOS only, see `gestures`); mouse and
//!   trackpad arrive through GPUI itself.

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple_audio;
#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple_ffi;
#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple_video;
mod blocking;
mod gestures;
pub mod entry;
mod http;
mod share;
mod store;

use crate::host::{AppInfo, Cancel, AudioConfig, AudioOutput, Chunk, Decoded, Host, Pinch, VideoConfig, VideoDecode};
use crate::logic::ports::Clock;
use std::sync::OnceLock;
use std::time::Instant;

pub struct SystemTimeClock;

impl Clock for SystemTimeClock {
    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
    }
}

pub struct Desktop;

#[cfg(any(target_os = "macos", target_os = "ios"))]
type VideoImpl = apple_video::AppleVideoDecoder;
#[cfg(any(target_os = "macos", target_os = "ios"))]
type AudioImpl = apple_audio::AppleAudioOut;
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
type VideoImpl = NoVideoDecoder;
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
type AudioImpl = NoAudioOut;

/// Server root: the one saved at login, else `IMMICH_URL`, else the same default `serve.py` proxies to.
fn server_origin() -> String {
    store::setting_get("immagine.server")
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("IMMICH_URL").ok())
        .unwrap_or_else(|| "http://immich.server.local".into())
        .trim_end_matches('/')
        .to_string()
}

impl Host for Desktop {
    type Transport = http::UreqTransport;
    type Clock = SystemTimeClock;
    type VideoDecoder = VideoImpl;
    type AudioOut = AudioImpl;
    type Gestures = gestures::GestureHandle;

    fn transport() -> Self::Transport {
        http::UreqTransport
    }

    fn clock() -> Self::Clock {
        SystemTimeClock
    }

    fn origin() -> String {
        server_origin()
    }

    fn now_ms() -> f64 {
        static START: OnceLock<Instant> = OnceLock::new();
        START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.
    }

    fn mark(label: &str) {
        eprintln!("[perf] {label} @{}ms", Self::now_ms() as i64);
    }

    fn setting_get(key: &str) -> Option<String> {
        store::setting_get(key)
    }

    fn setting_set(key: &str, value: &str) {
        store::setting_set(key, value)
    }

    fn setting_remove(key: &str) {
        store::setting_remove(key)
    }

    async fn blob_get(db: &str, key: &str) -> Result<Option<Vec<u8>>, String> {
        let path = store::blob_path(db, key);
        blocking::run(move || match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        })
        .await
    }

    async fn blob_delete(db: &str, key: &str) -> Result<(), String> {
        let path = store::blob_path(db, key);
        blocking::run(move || match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        })
        .await
    }

    async fn blob_clear(db: &str) -> Result<(), String> {
        let db = db.to_string();
        blocking::run(move || {
            store::blob_clear(&db);
            Ok(())
        })
        .await
    }

    async fn blob_size(db: &str) -> Result<u64, String> {
        let db = db.to_string();
        blocking::run(move || Ok(store::blob_size(&db))).await
    }

    async fn blob_keys(db: &str) -> Result<Vec<String>, String> {
        let db = db.to_string();
        blocking::run(move || Ok(store::blob_keys(&db))).await
    }

    fn renderer_name(_webgpu: bool) -> &'static str {
        "Metal"
    }

    /// No service worker, no installed-app mode, native threads; storage is the size of the data directory.
    async fn app_info() -> AppInfo {
        let used = blocking::run(|| Ok(store::data_size())).await.ok();
        AppInfo { threads: true, native: true, storage_used: used, ..AppInfo::default() }
    }

    async fn blob_put(db: &str, key: &str, bytes: &[u8]) -> Result<(), String> {
        let path = store::blob_path(db, key);
        let bytes = bytes.to_vec();
        blocking::run(move || {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            // Write-then-rename so a crash never leaves a half-written snapshot.
            let tmp = path.with_extension("tmp");
            std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
        })
        .await
    }

    async fn decode_bytes(bytes: Vec<u8>, min_side: u32) -> Result<Decoded, String> {
        blocking::run(move || {
            let mut image = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
            let short = image.width().min(image.height());
            if min_side > 0 && short > min_side {
                let k = min_side as f64 / short as f64;
                let (w, h) = (((image.width() as f64 * k).round() as u32).max(1), ((image.height() as f64 * k).round() as u32).max(1));
                image = image.resize_exact(w, h, image::imageops::FilterType::Triangle);
            }
            let image = image.into_rgba8();
            Ok(Decoded { width: image.width(), height: image.height(), rgba: image.into_raw() })
        })
        .await
    }

    /// `ureq` cannot be interrupted: a cancelled download still runs to the end, its bytes are dropped.
    async fn fetch_image(url: &str, cancel: &Cancel) -> Result<Vec<u8>, String> {
        if cancel.is_cancelled() {
            return Err("cancelled".into());
        }
        let url = url.to_string();
        let bytes = blocking::run(move || http::get_bytes(&url)).await?;
        if cancel.is_cancelled() {
            return Err("cancelled".into());
        }
        Ok(bytes)
    }

    async fn share_file(name: &str, mime: &str, bytes: Vec<u8>) -> Result<(), String> {
        share::share(name, mime, bytes).await
    }

    async fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
        let url = url.to_string();
        blocking::run(move || http::get_bytes(&url)).await
    }

    async fn fetch_range(url: &str, start: u64, end_inclusive: u64) -> Result<crate::host::RangeResponse, String> {
        let url = url.to_string();
        blocking::run(move || http::get_range(&url, start, end_inclusive)).await
    }

    fn video_decoder(config: VideoConfig, on_frame: impl Fn(i64, Decoded) + 'static) -> Option<Self::VideoDecoder> {
        VideoImpl::create(config, on_frame)
    }

    fn audio_output() -> Option<Self::AudioOut> {
        AudioImpl::create()
    }

    fn install_gestures(on_event: impl Fn(Pinch) + 'static) -> Option<Self::Gestures> {
        Some(gestures::install(on_event))
    }
}

// Platforms without a decoder (`video_decoder`/`audio_output` return `None`) use these; they are never built.
/// Uninhabited: no video decoder on this platform.
#[cfg_attr(any(target_os = "macos", target_os = "ios"), allow(dead_code))]
pub enum NoVideoDecoder {}
/// Uninhabited: no audio output on this platform.
#[cfg_attr(any(target_os = "macos", target_os = "ios"), allow(dead_code))]
pub enum NoAudioOut {}

#[cfg_attr(any(target_os = "macos", target_os = "ios"), allow(dead_code))]
impl NoVideoDecoder {
    fn create(_: VideoConfig, _: impl Fn(i64, Decoded) + 'static) -> Option<Self> {
        None
    }
}

#[cfg_attr(any(target_os = "macos", target_os = "ios"), allow(dead_code))]
impl NoAudioOut {
    fn create() -> Option<Self> {
        None
    }
}

impl VideoDecode for NoVideoDecoder {
    fn decode(&self, _: &Chunk) {
        match *self {}
    }
    fn queue_len(&self) -> u32 {
        match *self {}
    }
    fn reset(&self) {
        match *self {}
    }
    fn flush(&self) {
        match *self {}
    }
    fn error(&self) -> Option<String> {
        match *self {}
    }
}

impl AudioOutput for NoAudioOut {
    fn configure(&self, _: AudioConfig) -> bool {
        match *self {}
    }
    fn decode(&self, _: &Chunk) {
        match *self {}
    }
    fn queue_len(&self) -> u32 {
        match *self {}
    }
    fn reset(&self) {
        match *self {}
    }
    fn flush(&self) {
        match *self {}
    }
    fn start_clock(&self, _: i64) {
        match *self {}
    }
    fn clock_us(&self) -> Option<i64> {
        match *self {}
    }
    fn set_paused(&self, _: bool) {
        match *self {}
    }
}
