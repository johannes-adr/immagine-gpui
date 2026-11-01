mod viewer;

use super::Todos;
use crate::components::photo::photo;
use crate::theme::Palette;
use crate::components::ease::anim;
use gpui_kit::*;

pub(crate) use viewer::Viewer;

pub(crate) const PHOTO_COUNT: usize = 24;
const PRESS_GROW: f32 = 5.;
const TILE_GAP: f32 = 6.;

impl Todos {
    pub(crate) fn photos_view(&self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let vw: f32 = window.viewport_size().width.into();
        let cell = ((vw - TILE_GAP * 4.) / 3.).floor();

        let tiles: Vec<_> = (0..PHOTO_COUNT)
            .map(|ix| {
                let store = self.tile_bounds.clone();
                let tracker = canvas(
                    move |bounds, _, _| store.borrow_mut()[ix] = bounds,
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full();

                let base = div()
                    .id(("tile", ix))
                    .absolute()
                    .child(tracker);
                let tile: AnyElement = match self.press {
                    Some((pix, pressed, serial)) if pix == ix => base
                        .with_animation(("tile-anim", serial), anim(260), move |el, d| {
                            let g = PRESS_GROW * if pressed { d } else { 1. - d };
                            el.left(px(-g))
                                .top(px(-g))
                                .child(photo(ix, cell + 2. * g, cell + 2. * g, 4. + g))
                        })
                        .into_any_element(),
                    _ => base.child(photo(ix, cell, cell, 4.)).into_any_element(),
                };
                div()
                    .id(("cell", ix))
                    .relative()
                    .flex_none()
                    .size(px(cell))
                    .child(tile)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.press_gen += 1;
                            this.press = Some((ix, true, this.press_gen));
                            cx.notify();
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            if this.press.is_some_and(|(i, pressed, _)| i == ix && pressed) {
                                this.press_gen += 1;
                                this.press = Some((ix, false, this.press_gen));
                                cx.notify();
                            }
                        }),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.open_photo(ix, cx)))
                    .into_any_element()
            })
            .collect();

        div()
            .size_full()
            .flex()
            .flex_col()
            .pt(px(self.top.max(12.)))
            .child(
                div()
                    .px_5()
                    .pt(px(48.))
                    .pb_3()
                    .text_size(px(34.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.label)
                    .child("Photos"),
            )
            .child(
                div()
                    .id("photos")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(TILE_GAP))
                            .p(px(TILE_GAP))
                            .children(tiles),
                    )
                    .child(div().h(px(self.bottom.max(10.) + 90.))),
            )
    }
}
