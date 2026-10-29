use gpui_kit::*;

/// A generated "photo": sky gradient, sun and two hills, all sized in px.
pub(crate) fn photo(ix: usize, w: f32, h: f32, radius: f32) -> Div {
    let hue = ((ix * 47) % 360) as f32 / 360.;
    let wrap = |v: f32| v.rem_euclid(1.);
    let side = w.min(h);
    let var = (ix % 3) as f32;
    div()
        .relative()
        .overflow_hidden()
        .w(px(w))
        .h(px(h))
        .rounded(px(radius))
        .bg(linear_gradient(
            180.,
            linear_color_stop(hsla(hue, 0.75, 0.62, 1.), 0.),
            linear_color_stop(hsla(wrap(hue + 0.12), 0.8, 0.38, 1.), 1.),
        ))
        .child(
            div()
                .absolute()
                .left(px(w * (0.15 + 0.3 * var)))
                .top(px(h * 0.14))
                .size(px(side * 0.26))
                .rounded_full()
                .bg(hsla(wrap(hue + 0.1), 0.9, 0.88, 0.9)),
        )
        .child(
            div()
                .absolute()
                .left(px(-w * 0.25))
                .top(px(h * 0.62))
                .size(px(w * 1.2))
                .rounded_full()
                .bg(hsla(wrap(hue - 0.05), 0.55, 0.24, 0.85)),
        )
        .child(
            div()
                .absolute()
                .left(px(w * 0.3))
                .top(px(h * 0.74))
                .size(px(w * 1.0))
                .rounded_full()
                .bg(hsla(wrap(hue + 0.02), 0.5, 0.16, 0.9)),
        )
}
