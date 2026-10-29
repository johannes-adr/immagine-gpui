//! Small vector icons drawn as paths (the bundled font has no chevron or heart glyphs).

use crate::components::glass::glass_div;
use gpui_kit::*;

/// A "<" chevron, `color` stroked.
pub(super) fn chevron(color: Rgba) -> Canvas<()> {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let (x, y): (f32, f32) = (bounds.origin.x.into(), bounds.origin.y.into());
            let k = f32::from(bounds.size.width) / 24.;
            let at = |px_: f32, py_: f32| point(px(x + px_ * k), px(y + py_ * k));
            let mut b = PathBuilder::stroke(px(2.4));
            b.move_to(at(15., 4.));
            b.line_to(at(7., 12.));
            b.line_to(at(15., 20.));
            if let Ok(path) = b.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size(px(22.))
}

/// A round glass back button with a chevron.
pub(super) fn back_chip(tint: Hsla, id: &'static str, color: Rgba) -> Stateful<Div> {
    glass_div(tint, 18.).id(id).h(px(36.)).w(px(48.)).flex().items_center().justify_center().active(|s| s.opacity(0.6)).child(chevron(color))
}
