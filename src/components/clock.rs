//! The one time source of the components (animations of sheets, morphs, the rubber band, the FPS meter).
//!
//! `std::time::Instant` panics on `wasm32-unknown-unknown`, so the components never read a clock themselves: the
//! host application installs one function once at startup (`clock::install(my_now_ms)`, milliseconds on any
//! monotonic scale). Installing twice keeps the first. Before an install `now_ms()` returns 0, so animations do
//! not advance; an app (or test) that uses animated components must install a clock.

use std::sync::OnceLock;

static NOW: OnceLock<fn() -> f64> = OnceLock::new();

/// Sets the time source (milliseconds). Later calls are ignored.
pub(crate) fn install(now: fn() -> f64) {
    let _ = NOW.set(now);
}

/// Milliseconds from the installed source, 0 before `install`.
pub(crate) fn now_ms() -> f64 {
    NOW.get().map_or(0., |f| f())
}
