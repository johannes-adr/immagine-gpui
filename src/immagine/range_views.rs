//! The Jahre and Monate views of the Mediathek: one big card per year, and per month a heading with a big card
//! for its busiest day and up to three smaller ones. Built from the in-memory timeline (`logic::range_groups`).
//! Both are virtualized lists with a fixed row height, laid out like `photo_grid` (empty rows above and below
//! stand in for padding, the list's box is shifted to match).

use super::{thumbs, Immagine};
use crate::logic::models::timeline::TimelineItem;
use crate::logic::range_groups::{group, Groups};
use crate::theme::Palette;
use crate::components::tap;
use gpui_kit::*;
use std::rc::Rc;

const SIDE: f32 = 16.;
const GAP: f32 = 10.;
const RADIUS: f32 = 16.;
const MONTHS: [&str; 12] = ["Jan.", "Feb.", "März", "Apr.", "Mai", "Juni", "Juli", "Aug.", "Sept.", "Okt.", "Nov.", "Dez."];

/// One rounded photo card with `label` at its top left.
fn card(
    id: impl Into<ElementId>,
    item: &TimelineItem,
    label: String,
    (w, h): (Option<f32>, f32),
    label_size: f32,
    store: &thumbs::SharedThumbs,
    origin: &str,
    placeholder: Rgba,
    scale: f32,
    cx: &mut App,
) -> Stateful<Div> {
    let key = format!("t:{}", item.id);
    // Cover fit: the short side of the picture needs about the longer side of the card. Full width cards: full size.
    let side = w.map_or(0, |w| thumbs::side_for(w.max(h), scale));
    let loaded = thumbs::get_sized(store, &key, side, || thumbs::thumb_url(origin, &item.id, "thumbnail", item.thumbhash.as_deref()), cx);
    let picture = match (loaded, item.thumbhash.as_deref()) {
        (Some(image), _) => Some(image),
        (None, Some(hash)) => thumbs::placeholder(store, &item.id, hash),
        (None, None) => None,
    };
    let base = div().id(id).relative().h(px(h)).flex_none().rounded(px(RADIUS)).overflow_hidden().bg(placeholder);
    let base = match w {
        Some(w) => base.w(px(w)),
        None => base.w_full(),
    };
    #[cfg(test)]
    let base = base.debug_selector(|| format!("card-{}", item.id));
    base.children(picture.map(|image| img(image).absolute().top_0().left_0().size_full().object_fit(ObjectFit::Cover)))
        .child(div().absolute().top_0().left_0().w_full().h(px(label_size * 3.)).bg(linear_gradient(
            180.,
            linear_color_stop(rgba(0x00000066), 0.),
            linear_color_stop(rgba(0x00000000), 1.),
        )))
        .child(
            div()
                .absolute()
                .top(px(label_size * 0.45))
                .left(px(label_size * 0.6))
                .text_size(px(label_size))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(0xffffff))
                .child(label),
        )
}

impl Immagine {
    /// The groups of the current items, recomputed only when the timeline changed.
    fn range_groups(&self) -> Rc<Groups> {
        let mut cache = self.range_cache.borrow_mut();
        match cache.as_ref() {
            Some((items, groups)) if Rc::ptr_eq(items, &self.items) => groups.clone(),
            _ => {
                let groups = Rc::new(group(&self.items));
                *cache = Some((self.items.clone(), groups.clone()));
                groups
            }
        }
    }

    /// Jahre: one card per year, tapping it opens Monate at that year.
    pub(super) fn years_view(&self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let vw: f32 = window.viewport_size().width.into();
        let scale = window.scale_factor();
        let groups = self.range_groups();
        let card_h = ((vw - 2. * SIDE) * 0.62).round();
        let (store, origin, items, view) = (self.thumbs.clone(), self.origin.clone(), self.items.clone(), cx.weak_entity());
        let placeholder = p.chip;
        let count = groups.years.len();
        self.range_list("years", count, card_h + GAP, &self.years_scroll, &self.years_placed, None, window, move |row, window, cx| {
            let _ = window;
            let year = &groups.years[row];
            let view = view.clone();
            let first_month = year.first_month;
            let c = card(("year", row), &items[year.cover], year.year.to_string(), (None, card_h), 30., &store, &origin, placeholder, scale, cx).on_click(
                tap::weak(view, move |this, cx| this.show_months_at(first_month, cx)),
            );
            div().h(px(card_h + GAP)).px(px(SIDE)).pb(px(GAP)).child(c).into_any_element()
        })
    }

    /// Monate: heading, the busiest day as a big card, up to three more days as small cards.
    pub(super) fn months_view(&self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let vw: f32 = window.viewport_size().width.into();
        let scale = window.scale_factor();
        let groups = self.range_groups();
        let inner = vw - 2. * SIDE;
        let big_h = (inner * 0.66).round();
        let small = ((inner - 2. * GAP) / 3.).floor();
        let head_h = 46.;
        let row_h = head_h + big_h + GAP + small + 26.;
        let (store, origin, items, view) = (self.thumbs.clone(), self.origin.clone(), self.items.clone(), cx.weak_entity());
        let (placeholder, label) = (p.chip, p.label);
        let count = groups.months.len();
        let jump = Some(&self.months_jump);
        self.range_list("months", count, row_h, &self.months_scroll, &self.months_placed, jump, window, move |row, _, cx| {
            let month = &groups.months[row];
            let mut cards = month.highlights().into_iter().enumerate().map(|(i, day)| {
                let view = view.clone();
                let first = day.first;
                let (size, text) = if i == 0 { ((None, big_h), 30.) } else { ((Some(small), small), 22.) };
                let c = card(("day", row * 8 + i), &items[first], day.day.to_string(), size, text, &store, &origin, placeholder, scale, cx);
                c.on_click(tap::weak(view, move |this, cx| this.show_all_at(first, cx)))
            });
            let big = cards.next();
            let heading = format!("{} {}", MONTHS[(month.month - 1) as usize], month.year);
            // `w_full`: without it the row is as wide as its content and the big card's `w_full` resolves to 0.
            div()
                .w_full()
                .h(px(row_h))
                .px(px(SIDE))
                .flex()
                .flex_col()
                .child(div().h(px(head_h)).flex().items_end().pb(px(8.)).text_size(px(26.)).font_weight(FontWeight::BOLD).text_color(label).child(heading))
                .children(big)
                .child(div().h(px(small + GAP)).pt(px(GAP)).flex().gap(px(GAP)).children(cards))
                .into_any_element()
        })
    }

    /// A virtualized list of `count` rows of `row_h`, with space for the header above and the tab bar below.
    /// `jump` is a row to bring to the top once (set when another view sends the user here).
    #[allow(clippy::too_many_arguments)]
    fn range_list(
        &self,
        id: &'static str,
        count: usize,
        row_h: f32,
        handle: &UniformListScrollHandle,
        placed: &std::cell::Cell<bool>,
        jump: Option<&std::cell::Cell<Option<usize>>>,
        window: &Window,
        row: impl Fn(usize, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> AnyElement {
        let lead = self.top.max(12.) + 12.;
        let footer = self.bottom.max(10.) + 76.;
        let top_rows = (lead / row_h).ceil() as usize + 1;
        let bottom_rows = (footer / row_h).ceil() as usize + 1;
        let shift_top = lead - top_rows as f32 * row_h;
        let shift_bottom = footer - bottom_rows as f32 * row_h;
        let store = self.thumbs.clone();
        let list = uniform_list(id, top_rows + count + bottom_rows, move |range, window, cx| {
            store.borrow_mut().begin_frame();
            for evicted in store.borrow_mut().take_evicted() {
                let _ = window.drop_image(evicted);
            }
            range
                .map(|i| {
                    if i < top_rows || i >= top_rows + count {
                        div().h(px(row_h)).into_any_element()
                    } else {
                        row(i - top_rows, window, cx)
                    }
                })
                .collect()
        })
        .size_full();
        if count > 0 && !placed.replace(true) {
            handle.scroll_to_item(top_rows + count + bottom_rows - 1, ScrollStrategy::Bottom);
        }
        if let Some(row) = jump.and_then(|j| j.take()) {
            // Item `row` sits `top_rows` rows below list row `row`, i.e. exactly `lead` below the box's top.
            handle.scroll_to_item(row.min(count.saturating_sub(1)), ScrollStrategy::Top);
        }
        let list = list.track_scroll(handle);
        let vh: f32 = window.viewport_size().height.into();
        div()
            .size_full()
            .relative()
            .child(div().absolute().top(px(shift_top + self.rubber.value())).left_0().w_full().h(px(vh - shift_top - shift_bottom)).child(list))
            .into_any_element()
    }

    /// Jahre -> Monate, scrolled to the month at `month` in the month list.
    fn show_months_at(&mut self, month: usize, cx: &mut Context<Self>) {
        self.months_jump.set(Some(month));
        self.select_range(1, cx);
    }

    /// Monate -> Alle, scrolled to the photo at `index`.
    fn show_all_at(&mut self, index: usize, cx: &mut Context<Self>) {
        self.timeline_jump.set(Some(index));
        self.scroll_settle.set(crate::util::now_ms());
        self.select_range(2, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::gpui::TestAppContext;

    fn it(id: &str, date: &str) -> TimelineItem {
        TimelineItem { id: id.into(), asset_type: "IMAGE".into(), thumbhash: None, ratio: Some(1.), is_favorite: false, is_trashed: false, date_bucket: date.into(), sort_date: None, duration: None }
    }

    #[::core::prelude::v1::test]
    fn month_row_cards_have_size() {
        let mut cx = TestAppContext::single();
        let (view, cx) = cx.add_window_view(|w, cx| Immagine::new(0., 0., false, false, w, cx));
        view.update(cx, |app, cx| {
            app.screen = super::super::Screen::Timeline;
            app.range_mode = 1;
            app.items = Rc::new(vec![it("a", "2026-09-09"), it("b", "2026-09-13"), it("b2", "2026-09-13"), it("c", "2026-09-15")]);
            cx.notify();
        });
        cx.update(|w, _| w.refresh());
        let vw: f32 = cx.update(|w, _| w.viewport_size().width.into());
        // The busiest day ("b") gets the big card: it must span the row, not collapse to the width of its (absolute) content.
        let big = cx.debug_bounds("card-b").expect("big card painted");
        assert_eq!(f32::from(big.size.width), vw - 2. * SIDE);
        let small = cx.debug_bounds("card-a").expect("small card painted");
        assert!(f32::from(small.size.width) > 0.);
    }
}
