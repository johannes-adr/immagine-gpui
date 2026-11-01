//! Click handlers for things inside a flinging list.
//!
//! On touch, the tap that stops a coasting list is also delivered as a click on whatever lies under the finger
//! (`Window::tap_caught_fling`). A row or tile that opens something must not react to it, or stopping a fling
//! would open a random item. These wrap the usual handlers and drop such taps.

use gpui_kit::*;

/// `cx.listener(..)` for a click that ignores the tap which stopped a fling.
pub(crate) fn listener<V: 'static>(cx: &mut Context<V>, f: impl Fn(&mut V, &mut Context<V>) + 'static) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    cx.listener(move |this, _, window, cx| {
        if !window.tap_caught_fling() {
            f(this, cx);
        }
    })
}

/// The same for handlers built from a weak handle to the view (rows built outside `render`'s `cx`).
pub(crate) fn weak<V: 'static>(view: WeakEntity<V>, f: impl Fn(&mut V, &mut Context<V>) + 'static) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    move |_, window, cx| {
        if !window.tap_caught_fling() {
            view.update(cx, |this, cx| f(this, cx)).ok();
        }
    }
}
