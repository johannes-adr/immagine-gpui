use super::PRESS_GROW;
use crate::app::Todos;
use crate::components::glass::glass_div;
use crate::components::photo::photo;
use crate::theme::GLASS_TINT_DARK;
use crate::util::{anim, lerp};
use gpui_kit::*;
use std::time::Duration;

/// The full-screen photo overlay: which photo, where it zooms from, and its animation state.
pub(crate) struct Viewer {
    ix: usize,
    from: Bounds<Pixels>,
    closing: bool,
    serial: u32,
}

impl Todos {
    pub(crate) fn open_photo(&mut self, ix: usize, cx: &mut Context<Self>) {
        let from = self.tile_bounds.borrow()[ix];
        let from = Bounds::new(
            point(from.origin.x - px(PRESS_GROW), from.origin.y - px(PRESS_GROW)),
            size(from.size.width + px(2. * PRESS_GROW), from.size.height + px(2. * PRESS_GROW)),
        );
        self.press = None;
        self.press_gen += 1;
        self.viewer = Some(Viewer { ix, from, closing: false, serial: self.press_gen });
        cx.notify();
    }

    pub(crate) fn close_photo(&mut self, cx: &mut Context<Self>) {
        if let Some(v) = self.viewer.as_mut() {
            if v.closing {
                return;
            }
            v.closing = true;
            self.press_gen += 1;
            v.serial = self.press_gen;
            cx.notify();
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(420)).await;
                this.update(cx, |this, cx| {
                    if this.viewer.as_ref().is_some_and(|v| v.closing) {
                        this.viewer = None;
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
    }

    pub(crate) fn viewer_overlay(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let v = self.viewer.as_ref()?;
        let (ix, from, closing) = (v.ix, v.from, v.closing);
        let vs = window.viewport_size();
        let (vw, vh): (f32, f32) = (vs.width.into(), vs.height.into());
        let (fx, fy): (f32, f32) = (from.origin.x.into(), from.origin.y.into());
        let (fw, fh): (f32, f32) = (from.size.width.into(), from.size.height.into());
        let (top, bottom) = (self.top, self.bottom);

        Some(
            div()
                .id(("viewer", v.serial))
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .occlude()
                .on_click(cx.listener(|this, _, _, cx| this.close_photo(cx)))
                .with_animation(("viewer-anim", v.serial), anim(if closing { 380 } else { 420 }), move |el, d| {
                    let t = if closing { 1. - d } else { d };
                    el.child(
                        div()
                            .absolute()
                            .size_full()
                            .bg(rgba(0x000000ff))
                            .opacity(t),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(lerp(fx, 0., t)))
                            .top(px(lerp(fy, 0., t)))
                            .child(photo(ix, lerp(fw, vw, t), lerp(fh, vh, t), lerp(4., 0., t))),
                    )
                    .child(
                        glass_div(GLASS_TINT_DARK, 18.)
                            .absolute()
                            .top(px(top.max(12.) + 8.))
                            .right(px(16.))
                            .opacity(t)
                            .h(px(36.))
                            .px_4()
                            .flex()
                            .items_center()
                            .text_color(rgb(0xffffff))
                            .text_size(px(17.))
                            .child("Done"),
                    )
                    .child(
                        div()
                            .absolute()
                            .bottom(px(bottom.max(10.) + 16.))
                            .w_full()
                            .flex()
                            .justify_center()
                            .opacity(t)
                            .text_color(rgb(0xffffff))
                            .text_size(px(15.))
                            .child(format!("Photo {}", ix + 1)),
                    )
                })
                .into_any_element(),
        )
    }
}
