/// Milliseconds on the host's monotonic clock. The components get the same clock through
/// `components::clock::install` (done once in `Immagine::new`).
pub(crate) fn now_ms() -> f64 {
    <crate::host::Current as crate::host::Host>::now_ms()
}
