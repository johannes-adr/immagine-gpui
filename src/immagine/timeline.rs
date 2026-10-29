//! Timeline: all photos newest first, a virtualized grid of server thumbnails.

use super::Immagine;
use crate::theme::Palette;
use gpui_kit::*;

const GAP: f32 = 2.;

impl Immagine {
    pub(super) fn timeline_view(&self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let vw: f32 = window.viewport_size().width.into();
        let columns = ((vw / 120.).floor() as usize).clamp(3, 8);
        let cell = ((vw - GAP * (columns as f32 - 1.)) / columns as f32).floor();
        let rows = self.items.len().div_ceil(columns);
        let footer = self.bottom.max(10.) + 84.;

        let items = self.items.clone();
        let origin = self.origin.clone();
        let placeholder = p.chip;
        let view = cx.weak_entity();
        let bounds_map = self.tile_bounds.clone();
        let list = uniform_list("timeline", rows + 1, move |range, _, _| {
            range
                .map(|row| {
                    let tiles = (0..columns).filter_map(|c| items.get(row * columns + c).map(|item| (row * columns + c, item))).map(|(idx, item)| {
                        let view = view.clone();
                        let bounds_map = bounds_map.clone();
                        let url = format!("{origin}/api/assets/{}/thumbnail?size=thumbnail", item.id);
                        div()
                            .id(("tile", idx))
                            .relative()
                            .size(px(cell))
                            .flex_none()
                            .bg(placeholder)
                            .child(img(SharedUri::from(url)).size_full().object_fit(ObjectFit::Cover))
                            .child(
                                canvas(
                                    move |bounds, _, _| {
                                        let mut map = bounds_map.borrow_mut();
                                        if map.len() > 600 {
                                            map.clear();
                                        }
                                        map.insert(idx, bounds);
                                    },
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .size_full(),
                            )
                            .on_click(move |_, _, cx| {
                                view.update(cx, |this, cx| this.open_viewer(idx, cx)).ok();
                            })
                    });
                    if row >= rows {
                        return div().h(px(footer)).into_any_element();
                    }
                    div().h(px(cell + GAP)).flex().gap(px(GAP)).children(tiles).into_any_element()
                })
                .collect()
        })
        .flex_1();

        let count = self.items.len();
        let subtitle = if self.status.is_empty() { format!("{count} Fotos") } else { self.status.clone() };

        div()
            .size_full()
            .flex()
            .flex_col()
            .pt(px(self.top.max(12.)))
            .child(
                div()
                    .px_4()
                    .pt(px(40.))
                    .pb_2()
                    .flex()
                    .flex_row()
                    .items_end()
                    .justify_between()
                    .child(
                        div()
                            .child(div().text_size(px(34.)).font_weight(FontWeight::BOLD).child("Mediathek"))
                            .child(div().text_size(px(15.)).text_color(p.secondary).child(subtitle)),
                    )
                    .child(
                        div()
                            .id("logout")
                            .text_size(px(15.))
                            .text_color(p.secondary)
                            .active(|s| s.opacity(0.6))
                            .child("Abmelden")
                            .on_click(cx.listener(|this, _, _, cx| this.log_out(cx))),
                    ),
            )
            .child(list)
    }
}
