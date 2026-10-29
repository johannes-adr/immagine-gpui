use gpui_kit::*;

/// Liquid glass: a div whose backdrop is blurred and refracted by the renderer's glass pass.
/// Children are painted after the glass, so they stay crisp on top of it.
pub(crate) fn glass_div(tint: Hsla, radius: f32) -> Div {
    let style = GlassStyle {
        corner_radius: px(radius),
        blur: px(12.),
        bezel: px((radius * 0.7).clamp(10., 22.)),
        refraction: px((radius * 0.5).clamp(6., 14.)),
        dispersion: 0.06,
        light: 1.0,
        tint,
    };
    div().relative().rounded(px(radius)).child(
        canvas(|_, _, _| (), move |bounds, _, window, _| window.paint_glass(bounds, style))
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
    )
}
