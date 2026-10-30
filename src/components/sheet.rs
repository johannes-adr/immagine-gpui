//! Modal bottom sheet (Apple HIG "Sheets"): rises from the bottom edge over a dimmed page, rounded at the top,
//! a close circle at the top right. Only the title strip (`HEADER`) drags it, so content below can scroll. It
//! follows the finger down but never lifts above fully open; on release it closes when it was pulled past
//! `CLOSE_AT` of its height or flicked down, else it springs back open.
//!
//! Usage: keep one `Sheet` in the view, call `tick` once per render, put `overlay` last in the root's children
//! while `visible()`, and feed raw touch events to `hit`/`grab`/`drag_to`/`release` (desktop mouse events are
//! wired by `overlay` itself). `open`/`close` start the animations.

use crate::components::glass::glass_div_with;
use crate::util::{lerp, now_ms};
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

#[derive(Default)]
pub(crate) struct Sheet(Cell<State>);

fn ease_out(t: f32, power: i32) -> f32 {
    1. - (1. - t).powi(power)
}

impl Sheet {
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

/// The sheet's overlay: dimmed backdrop, the panel and its close circle. `content` fills the panel below the
/// title. `field` picks the `Sheet` out of the view; `close` runs when the circle or the backdrop is tapped.
pub(crate) fn overlay<V: 'static>(
    sheet: &Sheet,
    field: fn(&V) -> &Sheet,
    cx: &mut Context<V>,
    window: &Window,
    top_margin: f32,
    colors: (Rgba, Hsla, Rgba),
    title: &str,
    content: AnyElement,
    close: fn(&mut V, &mut Context<V>),
) -> AnyElement {
    let (background, glass_tint, label) = colors;
    let vh: f32 = window.viewport_size().height.into();
    let height = (vh - top_margin).max(100.);
    let shown = sheet.shown();
    let top = top_margin + (1. - shown) * height;
    sheet.edit(|s| {
        s.top = top;
        s.height = height;
    });
    let close_button = glass_div_with(glass_tint, 20., 1., true)
        .id("sheet-close")
        .absolute()
        .top(px(16.))
        .right(px(16.))
        .size(px(40.))
        .flex()
        .items_center()
        .justify_center()
        .active(|s| s.opacity(0.6))
        .child(cross(label))
        .on_click(cx.listener(move |this, _, _, cx| close(this, cx)));
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
        .bg(background)
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, _, _| field(this).grab(e.position.y.into())))
        .child(
            div()
                .h(px(HEADER))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(17.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(label)
                .child(title.to_string()),
        )
        .child(close_button)
        .child(content);
    div()
        .id("sheet-root")
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| {
            if field(this).dragging() {
                field(this).drag_to(e.position.y.into());
                cx.notify();
            }
        }))
        .on_mouse_up(MouseButton::Left, cx.listener(move |this, _: &MouseUpEvent, _, cx| {
            field(this).release();
            cx.notify();
        }))
        .on_mouse_up_out(MouseButton::Left, cx.listener(move |this, _: &MouseUpEvent, _, cx| {
            field(this).release();
            cx.notify();
        }))
        .child(
            div()
                .id("sheet-scrim")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .bg(rgba(0x00000000 | (0x66 as f32 * shown) as u32))
                .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                    // The panel is a sibling above the scrim but does not occlude it: only taps on the dimmed part close.
                    if !field(this).hit(e.position().y.into()) {
                        close(this, cx)
                    }
                })),
        )
        .child(panel)
        .into_any_element()
}

/// The "x" of the close circle.
fn cross(color: Rgba) -> Canvas<()> {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let (x, y): (f32, f32) = (bounds.origin.x.into(), bounds.origin.y.into());
            let k = f32::from(bounds.size.width) / 16.;
            for ((x0, y0), (x1, y1)) in [((2., 2.), (14., 14.)), ((14., 2.), (2., 14.))] {
                let mut b = PathBuilder::stroke(px(1.9 * k));
                b.move_to(point(px(x + x0 * k), px(y + y0 * k)));
                b.line_to(point(px(x + x1 * k), px(y + y1 * k)));
                if let Ok(path) = b.build() {
                    window.paint_path(path, color);
                }
            }
        },
    )
    .size(px(16.))
}
