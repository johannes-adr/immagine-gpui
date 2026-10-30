//! The seam between the app and the platform it runs on.
//!
//! Everything the app needs from outside GPUI (network, persistence, image decoding, video, extra input
//! gestures, clocks) is the [`Host`] trait. `app`, `immagine` and `logic` only ever talk to [`Current`];
//! no `web_sys`/`wasm_bindgen` outside `host/web`, no filesystem or HTTP-client crates outside `host/desktop`.
//!
//! * `web` (wasm32): `fetch`, IndexedDB, `localStorage`, `createImageBitmap`, DOM `<video>`, pointer events.
//! * `desktop` (everything else): `ureq`, files under the user's data dir, the `image` crate.
//!
//! Dispatch is static (`Current` is selected by `cfg`), so the traits can use `async fn` and need no `Send`.

use crate::logic::ports::{Clock, HttpTransport};

#[cfg(target_family = "wasm")]
pub mod web;
#[cfg(not(target_family = "wasm"))]
pub mod desktop;

#[cfg(target_family = "wasm")]
pub use web::Web as Current;
#[cfg(not(target_family = "wasm"))]
pub use desktop::Desktop as Current;

/// Straight-alpha RGBA, row-major.
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Touch gestures GPUI does not deliver itself (it has single-pointer input only on the web platform).
#[derive(Clone, Copy, Debug)]
pub enum Pinch {
    /// A second finger touched down.
    Begin,
    /// `ratio` is this move's change of the finger distance (1.0 = unchanged); `x`/`y` the midpoint in px.
    Move { ratio: f32, x: f32, y: f32 },
    /// One of the two fingers lifted.
    End,
    /// A single finger touched down, moved (raw position, no gesture slop or axis lock) or lifted.
    Down { x: f32, y: f32 },
    Drag { x: f32, y: f32 },
    Up { x: f32, y: f32 },
}

#[allow(async_fn_in_trait)]
pub trait Host: 'static {
    type Transport: HttpTransport + 'static;
    type Clock: Clock + 'static;
    /// Dropping it stops and removes the video.
    type Video;
    /// Dropping it uninstalls the gesture listeners.
    type Gestures;

    fn transport() -> Self::Transport;
    fn clock() -> Self::Clock;

    /// Server root all API paths and asset URLs hang off (`https://host`, no trailing slash).
    fn origin() -> String;
    /// Monotonic milliseconds for animations and frame timing.
    fn now_ms() -> f64;
    /// Start-up profiling line.
    fn mark(label: &str);

    /// Small per-device values (the session token). Degrade to "nothing stored" when unavailable.
    fn setting_get(key: &str) -> Option<String>;
    fn setting_set(key: &str, value: &str);
    fn setting_remove(key: &str);

    /// Large opaque blobs (the store snapshot), namespaced by `db`.
    async fn blob_get(db: &str, key: &str) -> Result<Option<Vec<u8>>, String>;
    async fn blob_put(db: &str, key: &str, bytes: &[u8]) -> Result<(), String>;

    /// Downloads an image with the session's credentials and decodes it to pixels.
    async fn decode_url(url: &str) -> Result<Decoded, String>;

    /// Plays `url` between `top` and `bottom` px of the window. `None` if the platform cannot.
    fn show_video(url: &str, top: f32, bottom: f32) -> Option<Self::Video>;
    /// Reports pinch and raw single-touch events. `None` if the platform has no such input source.
    fn install_gestures(on_event: impl Fn(Pinch) + 'static) -> Option<Self::Gestures>;
}

pub type Transport = <Current as Host>::Transport;
pub type SystemClock = <Current as Host>::Clock;
pub type VideoHandle = <Current as Host>::Video;
pub type GestureHandle = <Current as Host>::Gestures;
