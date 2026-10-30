use super::Immagine;
use crate::components::glass::glass_div_faded;
use crate::theme::{Palette, BLUE};
use crate::util::{lerp, now_ms, spring_out};
use gpui_kit::*;
use std::time::Duration;

pub(super) const TABS: [&str; 3] = ["Mediathek", "Alben", "Personen"];

/// Horizontal margin between the screen edge and the first tab slot (bar margin 20 + bar padding 4).
const SIDE: f32 = 24.;
/// Finger travel below which a press still counts as a tap on the slot under it.
const TAP_SLOP: f32 = 6.;
/// Length of the thumb animation.
const THUMB_MS: f64 = 520.;

#[derive(Clone, Copy)]
pub(super) struct TabDrag {
    /// Thumb position in tab units (0 = first slot, fractional while between slots).
    pos: f32,
    /// Finger x minus thumb center x at the moment of the press, so grabbing does not make the thumb jump.
    grab: f32,
    start_x: f32,
    moved: bool,
}

fn slot_width(viewport_w: f32) -> f32 {
    (viewport_w - 2. * SIDE) / TABS.len() as f32
}

impl Immagine {
    pub(super) fn select_tab(&mut self, tab: usize, cx: &mut Context<Self>) {
        if tab != self.tab {
            self.tab_from = self.tab as f32;
            self.bubble_from = 0.;
            self.tab = tab;
            self.tab_gen += 1;
            self.bump_thumb();
            cx.notify();
        }
    }

    /// Restarts the thumb animation (bubble and slide) from now on.
    fn bump_thumb(&mut self) {
        self.thumb_gen += 1;
        self.thumb_at = now_ms();
    }

    fn tab_drag_begin(&mut self, x: f32, viewport_w: f32, cx: &mut Context<Self>) {
        let slot = slot_width(viewport_w);
        let pos = self.tab as f32;
        self.tab_drag = Some(TabDrag { pos, grab: x - (SIDE + (pos + 0.5) * slot), start_x: x, moved: false });
        self.bump_thumb();
        cx.notify();
    }

    fn tab_drag_move(&mut self, x: f32, viewport_w: f32, cx: &mut Context<Self>) {
        let slot = slot_width(viewport_w);
        let Some(d) = self.tab_drag.as_mut() else { return };
        d.moved |= (x - d.start_x).abs() > TAP_SLOP;
        if d.moved {
            d.pos = ((x - d.grab - SIDE) / slot - 0.5).clamp(0., TABS.len() as f32 - 1.);
            cx.notify();
        }
    }

    fn tab_drag_end(&mut self, x: f32, viewport_w: f32, cx: &mut Context<Self>) {
        let slot = slot_width(viewport_w);
        let Some(d) = self.tab_drag.take() else { return };
        let last = TABS.len() - 1;
        let (from, target) = if d.moved {
            (d.pos, (d.pos.round() as usize).min(last))
        } else {
            (self.tab as f32, (((x - SIDE) / slot).floor().max(0.) as usize).min(last))
        };
        self.tab_from = from;
        self.bubble_from = 1.;
        if target != self.tab {
            self.tab = target;
            self.tab_gen += 1;
        }
        self.bump_thumb();
        cx.notify();
    }

    pub(super) fn tab_bar(&self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let slot = slot_width(window.viewport_size().width.into());
        // While dragging, the thumb sits at the finger and the bubble swells; on release it slides to its slot
        // while the bubble shrinks. One animation keyed by `thumb_gen` drives both, and re-rendering during the
        // drag keeps the same animation running with the new captured position.
        let (pos0, pos1, b0, b1) = match self.tab_drag {
            Some(d) => (d.pos, d.pos, 0., 1.),
            // The bar is torn down while a photo is open; once the last animation is over a rebuilt bar must
            // start at rest, not replay the slide.
            None if now_ms() - self.thumb_at > THUMB_MS => (self.tab as f32, self.tab as f32, 0., 0.),
            None => (self.tab_from, self.tab as f32, self.bubble_from, 0.),
        };
        // While a photo closes the bar appears in step with the grid showing through: the same ease-out as the
        // viewer's backdrop. Glass has no opacity, so its effects and the contents are scaled by `k`.
        let k = match self.viewer.as_ref().filter(|v| v.closing) {
            Some(v) => {
                let x = ((now_ms() - v.close_at) / super::viewer::CLOSE_MS as f64).clamp(0., 1.) as f32;
                1. - (1. - x).powi(5)
            }
            None => 1.,
        };
        let fade = |c: Rgba| Rgba { a: c.a * k, ..c };
        let thumb = glass_div_faded(p.bar_tint, 28., k)
            .id(("thumb", self.thumb_gen))
            .absolute()
            .bg(fade(p.tab_thumb))
            .with_animation(
                ("thumb-anim", self.thumb_gen),
                Animation::new(Duration::from_millis(THUMB_MS as u64)).with_easing(spring_out),
                move |el, d| {
                    let pos = lerp(pos0, pos1, d);
                    let b = lerp(b0, b1, d).max(0.);
                    let (dw, dh) = (18. * b, 14. * b);
                    el.left(px(pos * slot - dw / 2.)).top(px(-dh / 2.)).w(px(slot + dw)).h(px(56. + dh))
                },
            );
        let shown = match self.tab_drag {
            Some(d) if d.moved => d.pos.round() as usize,
            _ => self.tab,
        };

        let color = |ix: usize| -> Rgba { fade(if shown == ix { rgb(BLUE).into() } else { p.tab_inactive }) };
        let photo_icon = |c: Rgba| {
            div()
                .size(px(24.))
                .rounded(px(6.))
                .border_2()
                .border_color(c)
                .flex()
                .items_center()
                .justify_center()
                .child(div().size(px(8.)).rounded_full().bg(c))
        };
        let album_icon = |c: Rgba| {
            div().size(px(26.)).relative().child(div().absolute().top_0().left(px(4.)).w(px(18.)).h(px(3.)).rounded_full().bg(c)).child(
                div().absolute().top(px(6.)).left_0().size(px(24.)).rounded(px(6.)).border_2().border_color(c),
            )
        };
        let person_icon = |c: Rgba| {
            div()
                .size(px(26.))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(2.))
                .child(div().size(px(11.)).rounded_full().bg(c))
                .child(div().w(px(22.)).h(px(11.)).rounded_t(px(11.)).bg(c))
        };

        // The tap target is much larger than the pill: it reaches 14 px above it and down to the screen edge, and
        // selects on press so a slightly moving finger does not lose the tap.
        let bottom = self.bottom.max(10.);
        let item = |ix: usize, icon: Div| {
            let active = shown == ix;
            div()
                .flex_1()
                .h_full()
                .flex()
                .flex_col()
                .justify_end()
                .pb(px(bottom))
                .child(
                    div()
                        .h(px(64.))
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(3.))
                        .child(icon)
                        .child(
                            div()
                                .text_size(px(11.))
                                .font_weight(if active { FontWeight::BOLD } else { FontWeight::MEDIUM })
                                .text_color(color(ix))
                                .child(TABS[ix]),
                        ),
                )
        };

        let wrapper = div().id("tabbar").absolute().left_0().right_0().bottom_0().h(px(64. + bottom + 14.)).occlude();
        let wrapper = wrapper
            .child(
                div().absolute().left(px(20.)).right(px(20.)).bottom(px(bottom)).child(
                    glass_div_faded(p.bar_tint, 32., k).h(px(64.)).p(px(4.)).child(div().relative().size_full().child(thumb)),
                ),
            )
            .child(
                div()
                    .absolute()
                    .left(px(24.))
                    .right(px(24.))
                    .top_0()
                    .bottom_0()
                    .flex()
                    .child(item(0, photo_icon(color(0))))
                    .child(item(1, album_icon(color(1))))
                    .child(item(2, person_icon(color(2))))
                    .child(input_layer(cx.weak_entity())),
            );
        wrapper.into_any_element()
    }
}

/// Window-level pointer listeners for the tab bar, registered at paint time. Touch goes through the touch-drag
/// claim (the press is ours from the first moment, so the bubble can swell before any movement); mouse uses
/// press, move and release. Everything at or below the top edge of this layer belongs to the bar.
fn input_layer(view: WeakEntity<Immagine>) -> Canvas<()> {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let top: f32 = bounds.origin.y.into();
            let touch_view = view.clone();
            window.on_mouse_event(move |e: &TouchDragEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let w: f32 = window.viewport_size().width.into();
                let x: f32 = e.position.x.into();
                match e.phase {
                    TouchPhase::Started if f32::from(e.start_position.y) >= top => {
                        window.prevent_default();
                        touch_view.update(cx, |t, cx| t.tab_drag_begin(x, w, cx)).ok();
                    }
                    TouchPhase::Moved => {
                        touch_view.update(cx, |t, cx| t.tab_drag_move(x, w, cx)).ok();
                    }
                    TouchPhase::Ended | TouchPhase::Cancelled => {
                        touch_view.update(cx, |t, cx| t.tab_drag_end(x, w, cx)).ok();
                    }
                    _ => {}
                }
            });
            let down_view = view.clone();
            window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble && e.button == MouseButton::Left && f32::from(e.position.y) >= top {
                    let w: f32 = window.viewport_size().width.into();
                    down_view.update(cx, |t, cx| t.tab_drag_begin(e.position.x.into(), w, cx)).ok();
                }
            });
            let move_view = view.clone();
            window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble && e.pressed_button == Some(MouseButton::Left) {
                    let w: f32 = window.viewport_size().width.into();
                    move_view.update(cx, |t, cx| t.tab_drag_move(e.position.x.into(), w, cx)).ok();
                }
            });
            let up_view = view.clone();
            window.on_mouse_event(move |e: &MouseUpEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble && e.button == MouseButton::Left {
                    let w: f32 = window.viewport_size().width.into();
                    up_view.update(cx, |t, cx| t.tab_drag_end(e.position.x.into(), w, cx)).ok();
                }
            });
        },
    )
    .absolute()
    .size_full()
}
