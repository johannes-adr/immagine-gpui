//! Full-screen photo viewer: zoom-in from the tapped tile, swipe left/right between photos,
//! swipe down or "Fertig" to close. Pages use the grid thumbnail (already loaded) as a stand-in
//! until the larger preview arrives.

use super::Immagine;
use crate::components::glass::glass_div;
use crate::theme::GLASS_TINT_DARK;
use crate::util::{anim, lerp};
use gpui_kit::*;
use std::time::Duration;

/// A swipe farther than this fraction of the screen width turns the page.
const PAGE_FRACTION: f32 = 0.2;
/// A downward swipe farther than this many px closes the viewer.
const CLOSE_DISTANCE: f32 = 120.;

pub(super) struct Viewer {
    index: usize,
    from: Option<Bounds<Pixels>>,
    closing: bool,
    serial: u32,
    /// Pointer position where the current drag started.
    start: Option<(f32, f32)>,
    dx: f32,
    dy: f32,
}

fn xy(p: Point<Pixels>) -> (f32, f32) {
    (p.x.into(), p.y.into())
}

impl Immagine {
    pub(super) fn open_viewer(&mut self, index: usize, cx: &mut Context<Self>) {
        let from = self.tile_bounds.borrow().get(&index).copied();
        self.viewer_serial += 1;
        self.viewer = Some(Viewer { index, from, closing: false, serial: self.viewer_serial, start: None, dx: 0., dy: 0. });
        cx.notify();
    }

    pub(super) fn close_viewer(&mut self, cx: &mut Context<Self>) {
        let Some(v) = self.viewer.as_mut() else { return };
        if v.closing {
            return;
        }
        v.closing = true;
        v.start = None;
        self.viewer_serial += 1;
        v.serial = self.viewer_serial;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(400)).await;
            this.update(cx, |t, cx| {
                if t.viewer.as_ref().is_some_and(|v| v.closing) {
                    t.viewer = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn drag_start(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        if let Some(v) = self.viewer.as_mut().filter(|v| !v.closing) {
            v.start = Some(xy(at));
            v.dx = 0.;
            v.dy = 0.;
            cx.notify();
        }
    }

    fn drag_move(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(v) = self.viewer.as_mut() else { return };
        let Some((sx, sy)) = v.start else { return };
        let (x, y) = xy(at);
        v.dx = x - sx;
        v.dy = (y - sy).max(0.);
        cx.notify();
    }

    fn drag_end(&mut self, width: f32, cx: &mut Context<Self>) {
        let count = self.items.len();
        let Some(v) = self.viewer.as_mut() else { return };
        if v.start.take().is_none() {
            return;
        }
        let (dx, dy) = (v.dx, v.dy);
        v.dx = 0.;
        v.dy = 0.;
        if dy > CLOSE_DISTANCE && dy > dx.abs() {
            self.close_viewer(cx);
            return;
        }
        if dx.abs() > width * PAGE_FRACTION {
            if dx < 0. && v.index + 1 < count {
                v.index += 1;
            } else if dx > 0. && v.index > 0 {
                v.index -= 1;
            }
        }
        cx.notify();
    }

    pub(super) fn viewer_overlay(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let v = self.viewer.as_ref()?;
        let vs = window.viewport_size();
        let (vw, vh): (f32, f32) = (vs.width.into(), vs.height.into());
        let (index, closing, dx, dy) = (v.index, v.closing, v.dx, v.dy);
        let from = v.from.map(|b| {
            let (x, y): (f32, f32) = (b.origin.x.into(), b.origin.y.into());
            let (w, h): (f32, f32) = (b.size.width.into(), b.size.height.into());
            (x, y, w, h)
        });
        let items = self.items.clone();
        let origin = self.origin.clone();
        let top = self.top.max(12.);
        let count = items.len();
        
        let page = move |k: usize| -> Option<AnyElement> {
            let item = items.get(k)?;
            let thumb = format!("{origin}/api/assets/{}/thumbnail?size=thumbnail", item.id);
            let preview = format!("{origin}/api/assets/{}/thumbnail?size=preview", item.id);
            Some(
                div()
                    .size_full()
                    .child(img(SharedUri::from(thumb)).absolute().size_full().object_fit(ObjectFit::Contain))
                    .child(img(SharedUri::from(preview)).absolute().size_full().object_fit(ObjectFit::Contain))
                    .into_any_element(),
            )
        };
        let date = self.items.get(index).map(|i| i.date_bucket.clone()).unwrap_or_default();

        Some(
            div()
                .id("viewer-root")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .occlude()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, _, cx| this.drag_start(e.position, cx)))
                .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| this.drag_move(e.position, cx)))
                .on_mouse_up(MouseButton::Left, cx.listener(|this, _: &MouseUpEvent, window, cx| {
                    let w: f32 = window.viewport_size().width.into();
                    this.drag_end(w, cx)
                }))
                .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _: &MouseUpEvent, window, cx| {
                    let w: f32 = window.viewport_size().width.into();
                    this.drag_end(w, cx)
                }))
                .child(
                    div()
                        .id(("viewer", v.serial))
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .with_animation(("viewer-anim", v.serial), anim(if closing { 360 } else { 420 }), move |el, d| {
                    let t = if closing { 1. - d } else { d };
                    // While dragging down the backdrop fades and the photo follows the finger.
                    let fade = (1. - dy / 400.).clamp(0.3, 1.);
                    let (x0, y0, w, h) = match from {
                        Some((fx, fy, fw, fh)) => (lerp(fx, 0., t), lerp(fy, 0., t), lerp(fw, vw, t), lerp(fh, vh, t)),
                        None => (0., 0., vw, vh),
                    };
                    let mut el = el
                        .child(div().absolute().size_full().bg(rgba(0x000000ff)).opacity(t * fade))
                        .child(
                            div()
                                .absolute()
                                .left(px(x0 + if t >= 1. { dx } else { 0. }))
                                .top(px(y0 + if t >= 1. { dy } else { 0. }))
                                .w(px(w))
                                .h(px(h))
                                .overflow_hidden()
                                .children(page(index)),
                        );
                    if t >= 1. {
                        // Neighbours slide in beside the current page.
                        if index > 0 {
                            el = el.child(div().absolute().left(px(dx - vw)).top_0().w(px(vw)).h(px(vh)).children(page(index - 1)));
                        }
                        if index + 1 < count {
                            el = el.child(div().absolute().left(px(dx + vw)).top_0().w(px(vw)).h(px(vh)).children(page(index + 1)));
                        }
                    }
                    el
                    .child(
                        div()
                            .absolute()
                            .bottom(px(24.))
                            .w_full()
                            .flex()
                            .justify_center()
                            .opacity(t * fade)
                            .text_color(rgb(0xffffff))
                            .text_size(px(15.))
                            .child(format!("{date}  ·  {} von {count}", index + 1)),
                    )
                }),
                )
                .child(
                    glass_div(GLASS_TINT_DARK, 18.)
                        .id("viewer-done")
                        .absolute()
                        .top(px(top + 8.))
                        .right(px(16.))
                        .h(px(36.))
                        .px_4()
                        .flex()
                        .items_center()
                        .text_color(rgb(0xffffff))
                        .text_size(px(17.))
                        .child("Fertig")
                        .on_click(cx.listener(|this, _, _, cx| this.close_viewer(cx))),
                )
                .into_any_element(),
        )
    }
}
