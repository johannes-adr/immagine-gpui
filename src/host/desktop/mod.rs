//! Desktop implementation of [`crate::host::Host`] for the standard (non-web) GPUI platforms.
//!
//! * HTTP: `ureq` on a worker thread per request, one shared agent so the Immich cookie persists.
//! * Persistence: plain files below [`data_dir`].
//! * Images: downloaded and decoded with the `image` crate on a worker thread.
//! * No DOM, so no video element and no touch-gesture source: `show_video` and `install_gestures`
//!   return `None` (mouse and trackpad input arrive through GPUI itself).

mod blocking;
pub mod entry;
mod http;
mod store;

use crate::host::{Decoded, Host, Pinch};
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

/// Server root: `IMMICH_URL`, else the same default `serve.py` proxies to.
fn server_origin() -> String {
    std::env::var("IMMICH_URL").unwrap_or_else(|_| "http://immich.server.local".into()).trim_end_matches('/').to_string()
}

impl Host for Desktop {
    type Transport = http::UreqTransport;
    type Clock = SystemTimeClock;
    type Video = std::convert::Infallible;
    type Gestures = std::convert::Infallible;

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

    async fn decode_url(url: &str) -> Result<Decoded, String> {
        let url = url.to_string();
        blocking::run(move || {
            let bytes = http::get_bytes(&url)?;
            let image = image::load_from_memory(&bytes).map_err(|e| e.to_string())?.into_rgba8();
            Ok(Decoded { width: image.width(), height: image.height(), rgba: image.into_raw() })
        })
        .await
    }

    fn show_video(_url: &str, _top: f32, _bottom: f32) -> Option<Self::Video> {
        None
    }

    fn install_gestures(_on_event: impl Fn(Pinch) + 'static) -> Option<Self::Gestures> {
        None
    }
}
