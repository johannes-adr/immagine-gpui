//! Modal bottom sheet (Apple HIG "Sheets"): rises from the bottom edge over a dimmed page, rounded at the top,
//! a close circle at the top right. Only the title strip (`HEADER`) drags it, so content below can scroll. It
//! follows the finger down but never lifts above fully open; on release it closes when it was pulled past
//! `CLOSE_AT` of its height or flicked down, else it springs back open.
//!
//! Usage: keep one `Sheet` per sheet in the view (it owns the motion, the content's scroll handle and its own
//! rubber band, so the page's band does not bounce behind the scrim), call `tick` once per render, and put
//! `overlay` last in the root's children while `visible()`. Routing, each a one-liner in the host:
//! - raw touches (touch screens do not deliver them as mouse events): map them to `Pointer` and call `pointer`;
//! - touch scroll steps (from `rubber_band::input`): call `scroll_event` while the sheet is visible;
//! - `open`/`close` start the animations; desktop mouse events are wired by `overlay` itself.

use crate::components::button::{self, Surface};
use crate::components::rubber_band::RubberBand;
use crate::components::scrim::scrim;
use crate::components::theme::Theme;
use crate::components::icon::{glyph, Glyph};
use crate::components::clock::now_ms;
use crate::components::ease::{ease_out, lerp};
use gpui_kit::*;
use std::cell::Cell;

const OPEN_MS: f64 = 460.;
const CLOSE_MS: f64 = 300.;
/// Pulled down farther than this fraction of the sheet's height, a release closes it.
const CLOSE_AT: f32 = 0.25;
/// Downward speed (px/ms) of a release that closes it however far it was pulled.
const FLICK: f32 = 0.6;
const CORNER: f32 = 38.;
/// Height of the title strip; only a press there drags the sheet, so the content below can scroll.
pub(crate) const HEADER: f32 = 72.;

#[derive(Clone, Copy, Default)]
struct Drag {
    start_y: f32,
    start_shown: f32,
    last_y: f32,
    last_at: f64,
    /// Smoothed speed, px/ms, positive = downward.
    vel: f32,
    moved: f32,
}

#[derive(Clone, Copy, Default)]
struct State {
    /// 0 = hidden below the screen, 1 = fully open.
    shown: f32,
    open: bool,
    /// (start ms, shown at start, target)
    anim: Option<(f64, f32, f32)>,
    drag: Option<Drag>,
    /// Where the panel's top edge and its height were when it was last built (for raw touch events).
    top: f32,
    height: f32,
}

/// A pointer event on the sheet, y in window coordinates.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Pointer {
    Down(f32),
    Drag(f32),
    /// Lifted at `y`, or `None` when the position is unknown (cancelled).
    Up(Option<f32>),
}

/// The sheet's motion: a `Cell` of a `Copy` state, edited from event handlers and the render pass alike.
#[derive(Default)]
struct Motion(Cell<State>);

impl Motion {
    fn edit(&self, f: impl FnOnce(&mut State)) {
        let mut s = self.0.get();
        f(&mut s);
        self.0.set(s);
    }

    /// Shown, or on its way in or out.
    pub(crate) fn visible(&self) -> bool {
        let s = self.0.get();
        s.open || s.shown > 0.
    }

    pub(crate) fn is_open(&self) -> bool {
        self.0.get().open
    }

    pub(crate) fn open(&self) {
        self.edit(|s| {
            s.open = true;
            s.drag = None;
            s.anim = Some((now_ms(), s.shown, 1.));
        });
    }

    pub(crate) fn close(&self) {
        self.edit(|s| {
            s.open = false;
            s.drag = None;
            s.anim = Some((now_ms(), s.shown, 0.));
        });
    }

    /// Advances the open / close / spring-back motion; call from `render`.
    pub(crate) fn tick(&self, window: &mut Window) {
        let mut s = self.0.get();
        let Some((start, from, to)) = s.anim else { return };
        let (ms, power) = if to > from { (OPEN_MS, 4) } else { (CLOSE_MS, 3) };
        let t = (((now_ms() - start) / ms) as f32).clamp(0., 1.);
        s.shown = lerp(from, to, ease_out(t, power));
        if t >= 1. {
            s.anim = None;
            s.shown = to;
        } else {
            window.request_animation_frame();
        }
        self.0.set(s);
    }

    pub(crate) fn shown(&self) -> f32 {
        self.0.get().shown
    }

    /// A pointer at `y` is on the panel (not on the dimmed page above it).
    pub(crate) fn hit(&self, y: f32) -> bool {
        let s = self.0.get();
        s.open && y >= s.top
    }

    /// A pointer went down on the panel's title strip: the sheet now follows it. Presses on the content are ignored.
    pub(crate) fn grab(&self, y: f32) {
        self.edit(|s| {
            if s.open && s.drag.is_none() && y < s.top + HEADER {
                s.anim = None;
                s.drag = Some(Drag { start_y: y, start_shown: s.shown, last_y: y, last_at: now_ms(), ..Drag::default() });
            }
        });
    }

    pub(crate) fn dragging(&self) -> bool {
        self.0.get().drag.is_some()
    }

    pub(crate) fn drag_to(&self, y: f32) {
        self.edit(|s| {
            let (Some(mut d), true) = (s.drag, s.height > 0.) else { return };
            let now = now_ms();
            let dt = (now - d.last_at).max(1.) as f32;
            d.vel = 0.6 * d.vel + 0.4 * (y - d.last_y) / dt;
            d.moved += (y - d.last_y).abs();
            d.last_y = y;
            d.last_at = now;
            // The panel stays on the bottom edge: it can be pulled down, never lifted above fully open.
            s.shown = (d.start_shown - (y - d.start_y) / s.height).clamp(0., 1.);
            s.drag = Some(d);
        });
    }

    /// The pointer lifted: close past the critical point (or on a downward flick), else spring back open.
    pub(crate) fn release(&self) {
        let s = self.0.get();
        let Some(d) = s.drag else { return };
        if d.moved < 4. {
            self.edit(|s| s.drag = None);
            return;
        }
        let pulled = 1. - s.shown;
        if pulled > CLOSE_AT || (d.vel > FLICK && pulled > 0.02) {
            self.close();
        } else {
            self.open();
        }
    }
}

/// A modal bottom sheet: motion, scrolling content and its own overscroll band.
#[derive(Default)]
pub(crate) struct Sheet {
    motion: Motion,
    scroll: ScrollHandle,
    rubber: RubberBand,
}

impl Sheet {
    pub(crate) fn visible(&self) -> bool {
        self.motion.visible()
    }

    pub(crate) fn is_open(&self) -> bool {
        self.motion.is_open()
    }

    pub(crate) fn open(&self) {
        self.motion.open()
    }

    pub(crate) fn close(&self) {
        self.motion.close()
    }

    /// Advances the sheet's motion and its rubber band; call from `render`.
    pub(crate) fn tick(&self, window: &mut Window) {
        self.motion.tick(window);
        self.rubber.tick(window);
    }

    /// A raw pointer event (the touch path of the title strip's drag). True when it was used and the view should
    /// redraw.
    pub(crate) fn pointer(&self, event: Pointer) -> bool {
        let m = &self.motion;
        match event {
            Pointer::Down(y) if m.hit(y) => m.grab(y),
            Pointer::Drag(y) => m.drag_to(y),
            Pointer::Up(y) => {
                if let Some(y) = y {
                    m.drag_to(y);
                }
                m.release();
            }
            _ => return false,
        }
        true
    }

    /// A touch scroll step while the sheet is shown: the content scrolls like any `overflow_y_scroll` div, this adds
    /// the overscroll band and swallows the steps while the title strip drags the sheet, so the list does not scroll
    /// along. Returns (used, redraw) for `rubber_band::input`.
    pub(crate) fn scroll_event(&self, dy: f32, phase: TouchPhase, window: &mut Window) -> (bool, bool) {
        if self.motion.dragging() {
            window.cancel_touch_momentum();
            return (true, false);
        }
        self.rubber.event(dy, phase, Some(&self.scroll), window)
    }
}

/// What `overlay` needs to know about one sheet.
pub(crate) struct Spec<'a> {
    /// Prefix of the scroll area's element id.
    pub id: &'static str,
    pub title: &'a str,
    /// Distance from the top of the window to the sheet's top edge when fully open.
    pub top_margin: f32,
    pub theme: &'a Theme,
}

/// The sheet's overlay: dimmed backdrop, the panel with its title and close circle, and `content` (an element
/// taller than the panel scrolls, with the sheet's own overscroll band). `field` picks the `Sheet` out of the view;
/// `close` runs when the circle or the backdrop is tapped.
pub(crate) fn overlay<V: 'static>(
    sheet: &Sheet,
    field: fn(&V) -> &Sheet,
    cx: &mut Context<V>,
    window: &Window,
    spec: Spec,
    content: Div,
    close: fn(&mut V, &mut Context<V>),
) -> AnyElement {
    let theme = spec.theme;
    let vh: f32 = window.viewport_size().height.into();
    let height = (vh - spec.top_margin).max(100.);
    let shown = sheet.motion.shown();
    let top = spec.top_margin + (1. - shown) * height;
    sheet.motion.edit(|s| {
        s.top = top;
        s.height = height;
    });
    let close_button = button::circle("sheet-close", Surface::new(theme.glass).fresh(), 40.)
        .absolute()
        .top(px(16.))
        .right(px(16.))
        .active(button::dim)
        .child(glyph(Glyph::Close, 20., theme.label))
        .on_click(cx.listener(move |this, _, _, cx| close(this, cx)));
    // Scrolls below the title strip (absolute with top + bottom = a definite height, so `max_offset` is the overflow).
    let scrolling = div()
        .id(SharedString::from(format!("{}-scroll", spec.id)))
        .absolute()
        .top(px(HEADER))
        .left_0()
        .right_0()
        .bottom_0()
        .overflow_y_scroll()
        .track_scroll(&sheet.scroll)
        .child(sheet.rubber.shift(content));
    let panel = div()
        .id("sheet-panel")
        .absolute()
        .left_0()
        .top(px(top))
        .w_full()
        .h(px(height))
        .rounded_tl(px(CORNER))
        .rounded_tr(px(CORNER))
        .overflow_hidden()
        .bg(theme.background)
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, _, _| field(this).motion.grab(e.position.y.into())))
        .child(
            div()
                .h(px(HEADER))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(17.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.label)
                .child(spec.title.to_string()),
        )
        .child(close_button)
        .child(scrolling);
    div()
        .id("sheet-root")
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| {
            if field(this).motion.dragging() {
                field(this).motion.drag_to(e.position.y.into());
                cx.notify();
            }
        }))
        .on_mouse_up(MouseButton::Left, cx.listener(move |this, _: &MouseUpEvent, _, cx| {
            field(this).motion.release();
            cx.notify();
        }))
        .on_mouse_up_out(MouseButton::Left, cx.listener(move |this, _: &MouseUpEvent, _, cx| {
            field(this).motion.release();
            cx.notify();
        }))
        .child(
            scrim("sheet-scrim", 0.4 * shown).on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                // The panel is a sibling above the scrim but does not occlude it: only taps on the dimmed part close.
                if !field(this).motion.hit(e.position().y.into()) {
                    close(this, cx)
                }
            })),
        )
        .child(panel)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn open_close_and_visibility() {
        let s = Sheet::default();
        assert!(!s.visible() && !s.is_open());
        s.open();
        assert!(s.visible() && s.is_open());
        s.close();
        assert!(!s.is_open(), "closing starts at once");
    }

    #[::core::prelude::v1::test]
    fn pointer_events_are_used_only_when_they_belong_to_the_sheet() {
        let s = Sheet::default();
        // Closed: a press has nothing to grab.
        assert!(!s.pointer(Pointer::Down(100.)));
        s.open();
        // Never laid out (top = 0): everything is on the panel, only the title strip grabs.
        assert!(s.pointer(Pointer::Down(HEADER + 10.)));
        assert!(!s.motion.dragging(), "a press below the title strip does not grab");
        assert!(s.pointer(Pointer::Down(10.)));
        assert!(s.motion.dragging());
        assert!(s.pointer(Pointer::Up(None)));
        assert!(!s.motion.dragging());
    }
}
