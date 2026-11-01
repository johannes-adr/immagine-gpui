//! Liquid glass primitives. They need the vendored `Window::paint_glass` (see the note in `components/mod.rs`).

use gpui_kit::*;

/// Liquid glass: a div whose backdrop is blurred and refracted by the renderer's glass pass.
/// Children are painted after the glass, so they stay crisp on top of it.
pub(crate) fn glass_div(tint: Hsla, radius: f32) -> Div {
    glass_div_faded(tint, radius, 1.)
}

/// Only the backdrop blur and a tint, without refraction or rim light: for large bars that should not show an edge.
pub(crate) fn glass_blur(tint: Hsla, k: f32, fade: f32) -> Div {
    let style = GlassStyle {
        corner_radius: px(0.),
        blur: px(9. * k),
        bezel: px(12.),
        refraction: px(0.),
        fade: px(fade),
        light: 0.,
        tint: Hsla { a: tint.a * k, ..tint },
        fresh_backdrop: false,
    };
    div().relative().child(canvas(|_, _, _| (), move |bounds, _, window, _| window.paint_glass(bounds, style)).absolute().top_0().left_0().size_full())
}

/// Glass has no opacity, so `k` (1 = full, 0 = gone) scales every effect of the pass down to an invisible one.
pub(crate) fn glass_div_faded(tint: Hsla, radius: f32, k: f32) -> Div {
    glass_div_with(tint, radius, k, false)
}

/// `glass_div_faded` for glass inside an opaque overlay: `fresh_backdrop` makes it sample the frame painted so far
/// (the overlay's own background) rather than the snapshot taken at the first glass of the frame (the page below).
pub(crate) fn glass_div_with(tint: Hsla, radius: f32, k: f32, fresh_backdrop: bool) -> Div {
    let style = GlassStyle {
        corner_radius: px(radius),
        blur: px(12. * k),
        bezel: px((radius * 0.7).clamp(10., 22.)),
        refraction: px((radius * 0.5).clamp(6., 14.) * k),
        fade: px(0.),
        light: k,
        tint: Hsla { a: tint.a * k, ..tint },
        fresh_backdrop,
    };
    let base = div().relative().rounded(px(radius));
    // A fully faded glass still blits the frame's backdrop snapshot over its box, which wipes out whatever glass
    // was painted there before (the tab bar's invisible thumb ate the left part of the range bar while they swap).
    if k <= 0. {
        return base;
    }
    base.child(
        canvas(|_, _, _| (), move |bounds, _, window, _| window.paint_glass(bounds, style))
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
    )
}
