//! Reading where an element ended up on screen.

use gpui_kit::*;

/// An invisible layer filling its (relative or absolute) parent that hands the parent's window bounds to `sink`
/// on every layout pass, so hit tests and animations can use the measured rectangle on a later frame.
pub(crate) fn bounds(sink: impl Fn(Bounds<Pixels>) + 'static) -> Canvas<()> {
    canvas(move |b, _, _| sink(b), |_, _, _, _| {}).absolute().top_0().left_0().size_full()
}

/// `Bounds` as (left, top, width, height) in px.
pub(crate) fn rect(b: Bounds<Pixels>) -> (f32, f32, f32, f32) {
    (b.origin.x.into(), b.origin.y.into(), b.size.width.into(), b.size.height.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn rect_reads_origin_then_size() {
        let b = Bounds { origin: point(px(1.), px(2.)), size: size(px(30.), px(40.)) };
        assert_eq!(rect(b), (1., 2., 30., 40.));
    }
}
