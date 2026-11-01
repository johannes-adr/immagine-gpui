//! The dimmed layer behind sheets, dialogs and morphing cards.

use gpui_kit::*;

/// A full-size black layer of `alpha` over the parent (which must be `relative` or absolute). The caller adds
/// `.occlude()` and the tap handler: a scrim that does not occlude lets taps through to the page below.
pub(crate) fn scrim(id: impl Into<ElementId>, alpha: f32) -> Stateful<Div> {
    div().id(id).absolute().top_0().left_0().size_full().bg(hsla(0., 0., 0., alpha))
}
