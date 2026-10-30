use gpui_kit::*;
use std::time::Duration;

pub(crate) fn now_ms() -> f64 {
    <crate::host::Current as crate::host::Host>::now_ms()
}

pub(crate) fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub(crate) fn anim(ms: u64) -> Animation {
    Animation::new(Duration::from_millis(ms)).with_easing(ease_out_quint())
}

/// Ease-out with a small overshoot, like a light iOS spring.
pub(crate) fn spring_out(t: f32) -> f32 {
    let (c1, c3) = (1.3_f32, 2.3_f32);
    1. + c3 * (t - 1.).powi(3) + c1 * (t - 1.).powi(2)
}
