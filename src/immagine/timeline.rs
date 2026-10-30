//! Timeline: all photos newest first, a virtualized grid of server thumbnails.

use super::{thumbs, Immagine};
use crate::components::glass::glass_blur;
use crate::theme::{Palette, BLUE};
use crate::logic::models::timeline::TimelineItem;
use gpui_kit::*;
use std::rc::Rc;

const GAP: f32 = 2.;
impl Immagine {
    /// A virtualized, scrollable photo grid with a spacer for the tab bar at its end. Tapping a tile opens the
    /// viewer on `items`. Reused by the timeline, album pages and person pages; `id` must be unique per screen.
    /// `lead` is empty space above the first row (for a translucent header laid over the list).
    pub(super) fn photo_grid(
        &self,
        id: &'static str,
        items: Rc<Vec<TimelineItem>>,
        lead: f32,
        scroll: (&UniformListScrollHandle, Option<&std::cell::Cell<bool>>),
        p: &Palette,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let vw: f32 = window.viewport_size().width.into();
        let columns = ((vw / 120.).floor() as usize).clamp(3, 8);
        let cell = ((vw - GAP * (columns as f32 - 1.)) / columns as f32).floor();
        let rows = items.len().div_ceil(columns);
        let items_empty = items.is_empty();
        let footer = self.bottom.max(10.) + 76.;

        let origin = self.origin.clone();
        let placeholder = p.chip;
        let view = cx.weak_entity();
        let store = self.thumbs.clone();
        let bounds_map = self.tile_bounds.clone();
        let list_scroll = scroll.0.0.borrow().base_handle.clone();
        let grid_bounds = self.grid_bounds.clone();
        // A uniform list measures one row height for all items and does not account for its own padding when it
        // picks the visible rows (rows went missing at the edges). So the space above the first row (under the
        // header) and below the last one (above the tab bar) is made of whole empty rows, and the list's box is
        // shifted so the photos start exactly `lead` below the top and end `footer` above the bottom. The shift
        // also makes the list taller than the screen, so rows are built a little before they scroll in.
        let row_h = cell + GAP;
        let top_rows = if lead > 0. { (lead / row_h).ceil() as usize + 1 } else { 0 };
        let bottom_rows = (footer / row_h).ceil() as usize + usize::from(lead > 0.);
        let shift_top = lead - top_rows as f32 * row_h;
        let shift_bottom = footer - bottom_rows as f32 * row_h;
        let list = uniform_list(id, top_rows + rows + bottom_rows, move |range, window, cx| {
            store.borrow_mut().begin_frame();
            for evicted in store.borrow_mut().take_evicted() {
                let _ = window.drop_image(evicted);
            }
            range
                .map(|row| {
                    if row < top_rows || row >= top_rows + rows {
                        return div().h(px(row_h)).into_any_element();
                    }
                    let row = row - top_rows;
                    let tiles = (0..columns).filter_map(|c| items.get(row * columns + c).map(|item| (row * columns + c, item))).map(|(idx, item)| {
                        let view = view.clone();
                        let store = store.clone();
                        let bounds_map = bounds_map.clone();
                        let scroll = list_scroll.clone();
                        let all = items.clone();
                        let key = format!("t:{}", item.id);
                        let loaded = thumbs::get(&store, &key, || format!("{origin}/api/assets/{}/thumbnail?size=thumbnail", item.id), cx);
                        let picture = match (loaded, item.thumbhash.as_deref()) {
                            (Some(image), _) => Some(image),
                            (None, Some(hash)) => thumbs::placeholder(&store, &item.id, hash),
                            (None, None) => None,
                        };
                        div()
                            .id((id, idx))
                            .relative()
                            .size(px(cell))
                            .flex_none()
                            .bg(placeholder)
                            .overflow_hidden()
                            .children(picture.map(|image| img(image).absolute().top_0().left_0().size_full().object_fit(ObjectFit::Cover)))
                            .child(
                                canvas(
                                    move |bounds, _, _| {
                                        let mut map = bounds_map.borrow_mut();
                                        if map.len() > 600 {
                                            map.clear();
                                        }
                                        let offset = scroll.offset().y;
                                        map.insert(idx, super::TileRect { bounds, offset, scroll });
                                    },
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .size_full(),
                            )
                            .on_click(move |_, window, cx| {
                                if window.tap_caught_fling() {
                                    return;
                                }
                                view.update(cx, |this, cx| this.open_viewer(all.clone(), idx, cx)).ok();
                            })
                    });
                    div().h(px(cell + GAP)).flex().gap(px(GAP)).children(tiles).into_any_element()
                })
                .collect()
        })
        .size_full();
        let (handle, placed) = scroll;
        if let Some(placed) = placed {
            if !items_empty && !placed.replace(true) {
                handle.scroll_to_item(top_rows + rows + bottom_rows - 1, ScrollStrategy::Bottom);
            }
        }
        let list = list.track_scroll(handle);
        // The list's on-screen box, so the closing viewer can clip its shrinking photo to it.
        // (An absolute box with only top and bottom set does not give its percentage-sized child a height, so the
        // timeline, which fills the screen, gets an explicit one.)
        let vh: f32 = window.viewport_size().height.into();
        let list = if lead > 0. {
            div().absolute().top(px(shift_top + self.rubber.value())).left_0().w_full().h(px(vh - shift_top - shift_bottom)).child(list)
        } else {
            div().size_full().child(list)
        };
        div().flex_1().relative().child(list).child(canvas(move |bounds, _, _| grid_bounds.set(bounds), |_, _, _, _| {}).absolute().top_0().left_0().size_full())
    }

    pub(super) fn timeline_view(&self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The header is a glass strip laid over the grid, which scrolls underneath it (blurred).
        let head = self.top.max(12.) + 78.;
        let list = self.photo_grid("timeline", self.items.clone(), head, (&self.timeline_scroll, Some(&self.timeline_placed)), p, window, cx);

        let count = self.items.len();
        let subtitle = if self.status.is_empty() { format!("{count} Fotos") } else { self.status.clone() };

        div()
            .size_full()
            .relative()
            .child(div().absolute().top_0().left_0().size_full().flex().flex_col().child(list))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .w_full()
                    .h(px(head))
                    // One blur strip whose lower part fades out in the shader (a real gradient, no edge), reaching
                    // well below the header text, with a soft darkening that fades out the same way.
                    .child(glass_blur(p.bar_tint, 0.8, 120.).absolute().top_0().left_0().w_full().h(px(head + 64.)))
                    .child(
                        div().absolute().top_0().left_0().w_full().h(px(head + 64.)).bg(linear_gradient(
                            180.,
                            linear_color_stop(rgba(if self.dark { 0x0000004d } else { 0x00000014 }), 0.),
                            linear_color_stop(rgba(0x00000000), 1.),
                        )),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .px_4()
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
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_4()
                                    .child(
                                        div()
                                            .id("search-open")
                                            .text_size(px(15.))
                                            .text_color(rgb(BLUE))
                                            .active(|s| s.opacity(0.6))
                                            .child("Suchen")
                                            .on_click(cx.listener(|this, _, window, cx| this.open_search(window, cx))),
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
                            ),
                    ),
            )
    }
}
