//! The seam between the app and the platform it runs on.
//!
//! Everything the app needs from outside GPUI (network, persistence, image decoding, video, extra input
//! gestures, clocks) is the [`Host`] trait. `app`, `immagine` and `logic` only ever talk to [`Current`];
//! no `web_sys`/`wasm_bindgen` outside `host/web`, no filesystem or HTTP-client crates outside `host/desktop`.
//!
//! * `web` (wasm32): `fetch`, IndexedDB, `localStorage`, `createImageBitmap`, WebCodecs + WebAudio, pointer events.
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

pub use crate::logic::movie::Chunk;

/// Answer to a range request.
pub struct RangeResponse {
    pub bytes: Vec<u8>,
    /// Size of the whole file when the server said so (`Content-Range: bytes a-b/total`; web: the header must be
    /// CORS-exposed on another origin), else for a full answer the body length.
    pub total: Option<u64>,
    /// Status 206: `bytes` is the requested range. `false`: the server ignored `Range` and `bytes` is the whole file.
    pub partial: bool,
}

/// Parses `bytes a-b/total` (`total` may be `*`).
pub fn content_range_total(value: &str) -> Option<u64> {
    value.rsplit('/').next()?.trim().parse().ok()
}

pub struct VideoConfig {
    pub codec: String,
    pub description: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub struct AudioConfig {
    pub codec: String,
    pub description: Vec<u8>,
    pub sample_rate: u32,
    pub channels: u32,
}

/// A hardware or software video decoder fed with encoded samples in decode order.
pub trait VideoDecode {
    /// Queues a sample. The first one after creation or `reset` must be a keyframe.
    fn decode(&self, chunk: &Chunk);
    /// Samples queued and not yet decoded (feed more only while this is small).
    fn queue_len(&self) -> u32;
    /// Drops everything queued and decoded so far (seek).
    fn reset(&self);
    /// All samples were queued: pictures held back for reordering (B-frames) are output now.
    fn flush(&self);
    /// The decoder failed (unsupported stream, ...): a message for the user.
    fn error(&self) -> Option<String>;
}

/// Audio decoder and speaker. After `start_clock` it is also the playback clock, so picture and sound stay in sync.
pub trait AudioOutput {
    /// Prepares decoding of a track. `false` if the platform cannot decode it (play without sound).
    fn configure(&self, config: AudioConfig) -> bool;
    /// Queues a sample; once decoded it is scheduled at its timestamp relative to the clock.
    fn decode(&self, chunk: &Chunk);
    fn queue_len(&self) -> u32;
    /// Stops all sound, drops queued samples and the clock (seek).
    fn reset(&self);
    /// No more samples follow: decode what is still held back.
    fn flush(&self);
    /// Starts the clock at `media_us` now (running unless paused).
    fn start_clock(&self, media_us: i64);
    /// Media position in microseconds, `None` until the clock was started.
    fn clock_us(&self) -> Option<i64>;
    fn set_paused(&self, paused: bool);
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
    /// Decodes H.264 samples into RGBA pictures. Dropping it releases the decoder.
    type VideoDecoder: VideoDecode;
    /// Decodes AAC and plays it through the speakers, and is the playback clock. Dropping it stops the sound.
    type AudioOut: AudioOutput;
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

    /// Downloads `url` with the session's credentials. Used for videos, which the app demuxes and plays itself.
    async fn fetch_bytes(url: &str) -> Result<Vec<u8>, String>;
    /// Downloads bytes `start..=end_inclusive` of `url` with the session's credentials (HTTP `Range`). A server
    /// that ignores the header answers with the whole file (`partial == false`).
    async fn fetch_range(url: &str, start: u64, end_inclusive: u64) -> Result<RangeResponse, String>;
    /// A video decoder for `config`; every decoded picture (presentation order, scaled to a sane size) goes to
    /// `on_frame` with its timestamp in microseconds. `None` if the platform has no video decoder (desktop).
    fn video_decoder(config: VideoConfig, on_frame: impl Fn(i64, Decoded) + 'static) -> Option<Self::VideoDecoder>;
    /// The audio output. Must be called from a user gesture (the browser only unlocks sound then). `None` if the
    /// platform has none.
    fn audio_output() -> Option<Self::AudioOut>;
    /// Hands a file to the system share sheet. `Err` when the platform has none (desktop) or the user cancels.
    async fn share_file(name: &str, mime: &str, bytes: Vec<u8>) -> Result<(), String>;
    /// Reports pinch and raw single-touch events. `None` if the platform has no such input source.
    fn install_gestures(on_event: impl Fn(Pinch) + 'static) -> Option<Self::Gestures>;
}

pub type Transport = <Current as Host>::Transport;
pub type SystemClock = <Current as Host>::Clock;
pub type VideoDecoder = <Current as Host>::VideoDecoder;
pub type AudioOut = <Current as Host>::AudioOut;
pub type GestureHandle = <Current as Host>::Gestures;
