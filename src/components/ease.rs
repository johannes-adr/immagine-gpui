//! Interpolation and easing used by the animated components.

use gpui_kit::*;
use std::time::Duration;

pub(crate) fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// `1 - (1 - t)^power`: fast start, soft landing (`power` 3 = cubic, 4 = quartic).
pub(crate) fn ease_out(t: f32, power: i32) -> f32 {
    1. - (1. - t).powi(power)
}

/// An `Animation` of `ms` with the quint ease-out used for most element animations.
pub(crate) fn anim(ms: u64) -> Animation {
    Animation::new(Duration::from_millis(ms)).with_easing(ease_out_quint())
}

/// Ease-out with a small overshoot, like a light iOS spring.
pub(crate) fn spring_out(t: f32) -> f32 {
    let (c1, c3) = (1.3_f32, 2.3_f32);
    1. + c3 * (t - 1.).powi(3) + c1 * (t - 1.).powi(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn easing_runs_from_zero_to_one() {
        assert_eq!(lerp(2., 4., 0.5), 3.);
        assert_eq!(ease_out(0., 3), 0.);
        assert_eq!(ease_out(1., 4), 1.);
        assert!(ease_out(0.5, 3) > 0.5);
        assert!((spring_out(1.) - 1.).abs() < 1e-6 && spring_out(0.).abs() < 1e-5);
    }
}
