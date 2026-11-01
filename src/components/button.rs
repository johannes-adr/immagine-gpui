//! Buttons. Glass ones (`pill`, `circle`, `chip`, `back`) need the vendored `Window::paint_glass` (see `glass`);
//! `filled` is plain.
//!
//! Every function returns a `Stateful<Div>` the caller finishes: `.child(..)`, `.on_click(..)`, `.occlude()` (a
//! button on top of a tappable layer must occlude, or the tap also reaches the layer below), `.active(dim)` for the
//! pressed state, widths and padding. Nothing here occludes or dims by itself.

use crate::components::glass::glass_div_with;
use crate::components::icon::{chevron, Dir};
use gpui_kit::*;

/// How a glass button looks: its tint, how visible it is (`k`: 1 = full, 0 = gone, see `glass_div_with`) and
/// whether it sits in an opaque overlay (`fresh`: sample the frame painted so far, not the page snapshot).
#[derive(Clone, Copy)]
pub(crate) struct Surface {
    pub tint: Hsla,
    pub k: f32,
    pub fresh: bool,
}

impl Surface {
    pub(crate) fn new(tint: Hsla) -> Self {
        Self { tint, k: 1., fresh: false }
    }

    pub(crate) fn faded(self, k: f32) -> Self {
        Self { k, ..self }
    }

    pub(crate) fn fresh(self) -> Self {
        Self { fresh: true, ..self }
    }
}

/// Opacity of a pressed button.
const PRESSED: f32 = 0.6;

/// Style for `.active(dim)`: the pressed look of every tappable thing.
pub(crate) fn dim(s: StyleRefinement) -> StyleRefinement {
    s.opacity(PRESSED)
}

/// A glass pill: `h` high, fully rounded, content centered. Width comes from the content, `.px(..)` or `.w(..)`.
pub(crate) fn pill(id: impl Into<ElementId>, s: Surface, h: f32) -> Stateful<Div> {
    glass_div_with(s.tint, h / 2., s.k, s.fresh).id(id).h(px(h)).flex().flex_row().items_center().justify_center()
}

/// A round glass button of `size` px.
pub(crate) fn circle(id: impl Into<ElementId>, s: Surface, size: f32) -> Stateful<Div> {
    pill(id, s, size).w(px(size)).flex_none()
}

/// A glass pill holding a text label with `pad` px at both sides. Set `.text_size`, `.text_color`, `.font_weight`
/// on the result.
pub(crate) fn chip(id: impl Into<ElementId>, s: Surface, h: f32, pad: f32, label: impl Into<SharedString>) -> Stateful<Div> {
    pill(id, s, h).px(px(pad)).child(label.into())
}

/// The glass back button: a chevron in a 48 x 36 pill, pressed state included. `color` fades with `s.k`.
pub(crate) fn back(id: impl Into<ElementId>, s: Surface, color: Rgba) -> Stateful<Div> {
    let color = Rgba { a: color.a * s.k, ..color };
    pill(id, s, 36.).w(px(48.)).active(dim).child(chevron(Dir::Left, color))
}

/// A flat rounded button on `fill` with a centered label, pressed state included (dialog actions, inline
/// buttons). Set the text style on the result.
pub(crate) fn filled(id: impl Into<ElementId>, fill: Rgba, h: f32, radius: f32, label: impl Into<SharedString>) -> Stateful<Div> {
    div().id(id).h(px(h)).rounded(px(radius)).bg(fill).flex().items_center().justify_center().active(dim).child(label.into())
}
