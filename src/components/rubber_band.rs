//! Rubber band (iOS overscroll) for any scroll container: one shared look and feel.
//!
//! Usage: keep one `RubberBand` per view, call `tick` once per render, and wrap the scrolling content in
//! `shift` so it moves with the band. Scroll events arrive through `input`, a zero-size canvas painted once at the
//! root, which asks `target` for the scroll handle of whatever scrolls at that moment (`None` = no band, e.g. while
//! another layer owns the gesture).

use crate::logic::rubber_band::{Phase, RubberBand as Band};
use crate::util::now_ms;
use gpui_kit::*;
use std::cell::Cell;

#[derive(Default)]
pub(crate) struct RubberBand(Cell<Band>);

impl RubberBand {
    /// Current offset past the end (positive = content pulled down).
    pub(crate) fn value(&self) -> f32 {
        self.0.get().value()
    }

    /// Moves `content` by the band offset (layout is unchanged; the parent should clip).
    pub(crate) fn shift(&self, content: Div) -> Div {
        match self.value() {
            v if v == 0. => content,
            v => content.relative().top(px(v)),
        }
    }

    /// Advances spring-back and bounce; call from `render`.
    pub(crate) fn tick(&self, window: &mut Window) {
        let mut b = self.0.get();
        let more = b.tick(now_ms());
        self.0.set(b);
        if more {
            window.request_animation_frame();
        }
    }

    /// Feeds one touch scroll event. `scroll` is the active scroller's handle, `None` when the band must stay out
    /// of it. Returns true when the event was used up.
    pub(crate) fn event(&self, dy: f32, phase: TouchPhase, scroll: Option<&ScrollHandle>, window: &mut Window) -> (bool, bool) {
        let phase = match phase {
            TouchPhase::Started => Phase::Started,
            TouchPhase::Moved => Phase::Moved,
            _ => Phase::Ended,
        };
        let (offset, max) = scroll.map(|h| (h.offset().y.into(), h.max_offset().y.into())).unwrap_or((0., 0.));
        let mut b = self.0.get();
        let r = b.on_scroll(dy, phase, offset, max, now_ms(), scroll.is_some());
        self.0.set(b);
        if r.cancel_momentum {
            window.cancel_touch_momentum();
        }
        (r.used, r.notify)
    }
}

/// Zero-size canvas that hands every touch scroll event to `handle(dy, phase, window, cx)` before the list gets it
/// (window listener, capture phase). `handle` returns (used, notify); a used event is not propagated.
pub(crate) fn input<V: 'static>(view: WeakEntity<V>, handle: impl Fn(&mut V, f32, TouchPhase, &mut Window) -> (bool, bool) + 'static) -> Canvas<()> {
    let handle = std::rc::Rc::new(handle);
    canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            let view = view.clone();
            let handle = handle.clone();
            window.on_mouse_event(move |e: &ScrollWheelEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture || e.source != ScrollSource::Touch {
                    return;
                }
                let dy: f32 = e.delta.pixel_delta(px(16.)).y.into();
                let mut used = false;
                view.update(cx, |t, cx| {
                    let (u, notify) = handle(t, dy, e.touch_phase, window);
                    used = u;
                    if notify {
                        cx.notify();
                    }
                })
                .ok();
                if used {
                    cx.stop_propagation();
                }
            });
        },
    )
    .absolute()
    .size_0()
}
