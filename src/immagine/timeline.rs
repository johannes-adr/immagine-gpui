//! Timeline: all photos newest first, a virtualized grid of server thumbnails.

use super::{thumbs, Immagine, Screen};
use crate::host::Pinch;
use crate::logic::grid::hit::HitGeom;
use crate::logic::grid::layout::ZOOM_LEVELS;
use crate::components::glass::glass_blur;
use crate::theme::Palette;
use crate::logic::models::timeline::TimelineItem;
use crate::logic::range_groups::{format_range, month_label, parse_bucket};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::rc::Rc;

const GAP: f32 = 2.;
/// Window width from which the grid starts with 5 columns instead of 3 (iPad / desktop).
const WIDE_VIEWPORT: f32 = 700.;
/// How far (0..1, log scale between two levels) a pinch must go to take the next level when let go.
const SNAP_AT: f32 = 0.3;
const SETTLE_MS: f64 = 240.;
const WHEEL_IDLE_MS: f64 = 140.;
const FADE_MS: f64 = 200.;
/// Columns from which the timeline shows month labels over the grid.
const MONTH_LABEL_COLS: usize = 15;
/// (year, month) of a photo, for spotting where a month starts.
fn month_of(item: &TimelineItem) -> Option<(i32, u32)> {
    parse_bucket(&item.date_bucket).map(|(y, m, _)| (y, m))
}

impl Immagine {
    pub(super) fn grid_active(&self) -> bool {
        self.screen == Screen::Timeline && self.tab == 0 && self.range_mode == 2 && !self.search.open && self.viewer.is_none()
    }

    /// Pinching, settling or fading in after a level switch: the grid holds `grid_anchor` in place.
    pub(super) fn grid_zooming(&self) -> bool {
        self.grid_pinching || self.grid_anim.is_some() || self.grid_fade_at > 0.
    }

    /// Two-finger pinch on the timeline grid (touch via `Host::install_gestures`): the cells follow the fingers
    /// continuously; on release the grid settles on a column level (see `grid_release`).
    pub(super) fn grid_pinch(&mut self, event: Pinch, cx: &mut Context<Self>) {
        if !self.grid_active() {
            return;
        }
        match event {
            Pinch::Begin => {
                self.sb_cancel_touch(cx);
                self.sel_cancel_touch(cx);
                self.grid_begin()
            }
            Pinch::Move { ratio, .. } => self.grid_apply(ratio, cx),
            Pinch::End => self.grid_release(cx),
            Pinch::Down { .. } | Pinch::Drag { .. } | Pinch::Up { .. } => {
                // The scrollbar thumb takes its presses before selection painting sees them.
                if !self.sb_touch(event, cx) {
                    self.sel_touch(event, cx)
                }
            }
        }
    }

    /// Settled columns until the user zooms: 3 on phones, 5 on iPad / desktop sizes (follows the width, which is
    /// not final at the first frame).
    pub(super) fn grid_default(&mut self, vw: f32) {
        if self.grid_user || self.grid_pinching || self.grid_anim.is_some() {
            return;
        }
        let cols = if vw >= WIDE_VIEWPORT { 5 } else { 3 };
        if self.grid_cols != cols {
            self.grid_cols = cols;
            self.grid_f = cols as f32;
        }
    }

    fn grid_begin(&mut self) {
        if self.grid_pinching {
            return;
        }
        self.grid_pinching = true;
        self.grid_user = true;
        self.grid_anim = None;
        self.grid_fade = 1.;
        self.grid_fade_at = 0.;
        self.grid_from = ZOOM_LEVELS.iter().copied().min_by(|a, b| (*a as f32 - self.grid_f).abs().total_cmp(&(*b as f32 - self.grid_f).abs())).unwrap_or(3);
        // The photo under the middle of the screen, measured in the layout on screen now, so nothing moves when the
        // pinch begins.
        let (row_h, top_rows, mid) = self.grid_geom.get();
        let (cols, n) = (self.grid_cols.max(1), self.items.len());
        if row_h > 0. && n > 0 {
            let offset: f32 = self.timeline_scroll.0.borrow().base_handle.offset().y.into();
            let pos = ((mid - offset) / row_h - top_rows as f32).max(0.);
            let row = (pos as usize).min((n - 1) / cols);
            self.grid_anchor = ((row * cols + cols / 2).min(n - 1), (pos - row as f32).clamp(0., 1.));
        }
    }

    /// Spreading (ratio > 1) makes the cells larger: fewer columns.
    fn grid_apply(&mut self, ratio: f32, cx: &mut Context<Self>) {
        self.grid_begin();
        let lo = ZOOM_LEVELS[0] as f32 * 0.85;
        let hi = *ZOOM_LEVELS.last().unwrap() as f32 * 1.15;
        self.grid_f = (self.grid_f / ratio.powf(1.2)).clamp(lo, hi);
        cx.notify();
    }

    /// Fingers lifted: settle on a level. Like swiping a page, the next level is only taken when the pinch went
    /// past a critical point (`SNAP_AT` of the way, in log scale); letting go earlier springs back.
    fn grid_release(&mut self, cx: &mut Context<Self>) {
        self.grid_settle();
        cx.notify();
    }

    fn grid_settle(&mut self) {
        if !self.grid_pinching {
            return;
        }
        self.grid_pinching = false;
        self.grid_wheel_at = 0.;
        let f = self.grid_f;
        let from = self.grid_from as f32;
        let lo = ZOOM_LEVELS.iter().copied().filter(|&l| l as f32 <= f).max().unwrap_or(ZOOM_LEVELS[0]);
        let hi = ZOOM_LEVELS.iter().copied().filter(|&l| l as f32 >= f).min().unwrap_or(*ZOOM_LEVELS.last().unwrap());
        let target = if lo == hi {
            lo
        } else {
            let u = (f / lo as f32).ln() / (hi as f32 / lo as f32).ln();
            if from <= lo as f32 {
                if u >= SNAP_AT { hi } else { lo }
            } else if u <= 1. - SNAP_AT {
                lo
            } else {
                hi
            }
        };
        self.grid_anim = Some((crate::util::now_ms(), f, target));
    }

    /// Advances the settle animation and ends a ctrl+wheel gesture after a pause.
    pub(super) fn grid_tick(&mut self, window: &mut Window) {
        let now = crate::util::now_ms();
        if self.grid_wheel_at > 0. {
            if now - self.grid_wheel_at > WHEEL_IDLE_MS {
                self.grid_settle();
            } else {
                window.request_animation_frame();
            }
        }
        if let Some((start, from, to)) = self.grid_anim {
            let t = ((now - start) / SETTLE_MS).clamp(0., 1.) as f32;
            let e = 1. - (1. - t).powi(3);
            self.grid_f = from + (to as f32 - from) * e;
            let switching = to != self.grid_cols;
            self.grid_fade = if switching { 1. - ((t - 0.7) / 0.3).clamp(0., 1.) } else { 1. };
            if t >= 1. {
                self.grid_anim = None;
                self.grid_cols = to;
                self.grid_f = to as f32;
                if switching {
                    self.grid_fade_at = now;
                }
            }
            window.request_animation_frame();
        } else if self.grid_fade_at > 0. {
            let d = ((now - self.grid_fade_at) / FADE_MS).clamp(0., 1.) as f32;
            self.grid_fade = d;
            if d >= 1. {
                self.grid_fade_at = 0.;
            }
            window.request_animation_frame();
        }
    }

    /// Trackpad pinch and ctrl+wheel (desktop). Window listeners in the capture phase, so the list below never
    /// sees a pinch as a scroll.
    pub(super) fn grid_input(&self, cx: &mut Context<Self>) -> Canvas<()> {
        let view = cx.weak_entity();
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let v = view.clone();
                window.on_mouse_event(move |e: &ScrollWheelEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture || !e.modifiers.control || matches!(e.source, ScrollSource::Touch) {
                        return;
                    }
                    let dy: f32 = e.delta.pixel_delta(px(16.)).y.into();
                    let mut used = false;
                    v.update(cx, |this, cx| {
                        if this.grid_active() {
                            used = true;
                            this.grid_apply((dy * 0.01).exp(), cx);
                            this.grid_wheel_at = crate::util::now_ms();
                        }
                    })
                    .ok();
                    if used {
                        cx.stop_propagation();
                    }
                });
                let v = view.clone();
                window.on_mouse_event(move |e: &PinchEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let (delta, touch) = (e.delta, e.phase);
                    let mut used = false;
                    v.update(cx, |this, cx| {
                        if this.grid_active() {
                            used = true;
                            match touch {
                                TouchPhase::Ended | TouchPhase::Cancelled => this.grid_release(cx),
                                _ => this.grid_apply(1. + delta, cx),
                            }
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
        let timeline = scroll.1.is_some();
        // The timeline zooms by scaling the cells of the settled layout (`grid_cols` columns per row) about the middle
        // of the screen; no photo changes row while pinching. The layout only switches to the target level once the
        // settle animation is over, hidden by a short fade (`grid_fade`).
        let cols_f = if timeline { self.grid_f.max(0.5) } else { ((vw / 120.).floor() as usize).clamp(3, 8) as f32 };
        let columns = if timeline { self.grid_cols.max(1) } else { cols_f as usize };
        let gap = if cols_f >= MONTH_LABEL_COLS as f32 - 0.5 { 1. } else { GAP };
        let cell = (vw - gap * (cols_f - 1.)) / cols_f;
        // Whole device pixels: layout rounds the list's row height to them, and with thousands of rows a fraction
        // adds up to many rows between where the zoom puts the anchor and where the list draws it (the grid jumped
        // back and forth while pinching near the end of a long timeline).
        let scale = window.scale_factor();
        let side = thumbs::side_for(cell, scale);
        let zooming = self.grid_zooming();
        let cell = if cols_f.fract() == 0. { cell.floor() } else { (cell * scale).floor() / scale };
        let fade = if timeline { self.grid_fade } else { 1. };
        let visible = self.visible.clone();
        let rows = items.len().div_ceil(columns);
        let items_empty = items.is_empty();
        let footer = self.bottom.max(10.) + 76.;
        let downloaded = self.downloads.ids.clone();
        let selecting = timeline && self.select.on;
        let sel_ids = self.select.ids.clone();
        let count = items.len();

        let origin = self.origin.clone();
        let placeholder = p.chip;
        let view = cx.weak_entity();
        let store = self.thumbs.clone();
        let bounds_map = self.tile_bounds.clone();
        // The viewer's photo flies to/from its tile, so the tile looks empty meanwhile instead of showing it twice.
        let hidden_id = self.viewer_item_id();
        let list_scroll = scroll.0.0.borrow().base_handle.clone();
        let list_scroll_set = list_scroll.clone();
        let grid_bounds = self.grid_bounds.clone();
        // A uniform list measures one row height for all items and does not account for its own padding when it
        // picks the visible rows (rows went missing at the edges). So the space above the first row (under the
        // header) and below the last one (above the tab bar) is made of whole empty rows, and the list's box is
        // shifted so the photos start exactly `lead` below the top and end `footer` above the bottom. The shift
        // also makes the list taller than the screen, so rows are built a little before they scroll in.
        let row_h = cell + gap;
        let top_rows = if lead > 0. { (lead / row_h).ceil() as usize + 1 } else { 0 };
        let bottom_rows = (footer / row_h).ceil() as usize + usize::from(lead > 0.);
        let shift_top = lead - top_rows as f32 * row_h;
        let shift_bottom = footer - bottom_rows as f32 * row_h;
        let list = uniform_list(id, top_rows + rows + bottom_rows, move |range, window, cx| {
            if timeline {
                let first = range.start.saturating_sub(top_rows).min(rows.saturating_sub(1));
                let last = range.end.saturating_sub(top_rows + 1).min(rows.saturating_sub(1));
                visible.set((first * columns, ((last + 1) * columns).min(items.len()).saturating_sub(1)));
            }
            store.borrow_mut().begin_frame();
            if !zooming {
                // A fast fling holds back new thumbnail loads (the pinch moves the offset too, so it does not count).
                let offset: f32 = list_scroll.offset().y.into();
                store.borrow_mut().track_scroll(offset);
            }
            for evicted in store.borrow_mut().take_evicted() {
                let _ = window.drop_image(evicted);
            }
            let (range_start, range_end) = (range.start, range.end);
            let built: Vec<AnyElement> = range
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
                        let loaded = thumbs::get_sized(&store, &key, side, || thumbs::thumb_url(&origin, &item.id, "thumbnail", item.thumbhash.as_deref()), cx);
                        let hidden = hidden_id.as_deref() == Some(item.id.as_str());
                        let picture = match (loaded, item.thumbhash.as_deref()) {
                            (Some(image), _) => Some(image),
                            (None, Some(hash)) => thumbs::placeholder(&store, &item.id, hash),
                            (None, None) => None,
                        };
                        // Videos show their length at the bottom right (not in the tiniest cells).
                        let badge = item.duration.clone().filter(|_| item.is_video() && cell >= 40.);
                        let selected = selecting && sel_ids.borrow().contains(&item.id);
                        let tap_id = item.id.clone();
                        div()
                            .id((id, idx))
                            .relative()
                            .opacity(fade)
                            .size(px(cell))
                            .flex_none()
                            .when(!hidden, |d| d.bg(placeholder))
                            .overflow_hidden()
                            .children(picture.filter(|_| !hidden).map(|image| img(image).absolute().top_0().left_0().size_full().object_fit(ObjectFit::Cover)))
                            .children((selected && !hidden).then(|| div().absolute().top_0().left_0().size_full().bg(rgba(0xffffff5c))))
                            .children((!hidden && cell >= 40.).then(|| {
                                // Bottom row: a white heart at the left for favorites; at the right the video's length and a
                                // tiny mark for photos kept in IndexedDB.
                                let icon = if cell < 90. { 10. } else { 12. };
                                let stored = downloaded.borrow().contains(&item.id);
                                div()
                                    .absolute()
                                    .left(px(4.))
                                    .right(px(if selecting { 4. + icon * 1.9 } else { 4. }))
                                    .bottom(px(4.))
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .child(div().children(item.is_favorite.then(|| super::icons::heart_badge(icon, rgb(0xffffff).into()))))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap(px(3.))
                                            .children(badge.map(|text| {
                                                div()
                                                    .px(px(5.))
                                                    .py(px(1.))
                                                    .rounded(px(8.))
                                                    .bg(rgba(0x00000066))
                                                    .text_color(rgb(0xffffff))
                                                    .text_size(px(icon))
                                                    .child(text)
                                            }))
                                            .children(stored.then(|| super::icons::glyph(super::icons::Glyph::Downloaded, icon, rgb(0xffffff).into()))),
                                    )
                            }))
                            .children((selecting && !hidden && cell >= 24.).then(|| {
                                // iOS select mode: a ring on every tile, a blue disc with a check on selected ones.
                                let d = if cell < 60. { 16. } else { 22. };
                                let mark = div().absolute().right(px(4.)).bottom(px(4.)).size(px(d)).rounded_full().border_2().flex().items_center().justify_center();
                                if selected {
                                    mark.bg(rgb(crate::theme::BLUE)).border_color(rgb(0xffffff)).child(super::icons::glyph(super::icons::Glyph::Check, d * 0.62, rgb(0xffffff).into()))
                                } else {
                                    mark.border_color(rgba(0xffffffe6)).bg(rgba(0x00000026))
                                }
                            }))
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
                                view.update(cx, |this, cx| {
                                    if this.sb_swallow_click() {
                                        return;
                                    }
                                    if selecting {
                                        this.select_tap(&tap_id, cx)
                                    } else {
                                        this.open_viewer(all.clone(), idx, cx)
                                    }
                                })
                                .ok();
                            })
                    });
                    // At the smallest cells a label names the month that starts in this row.
                    let label = (timeline && cols_f >= MONTH_LABEL_COLS as f32 - 0.5)
                        .then(|| {
                            let (start, end) = (row * columns, ((row + 1) * columns).min(items.len()));
                            (start..end).find(|&i| i == 0 || month_of(&items[i]) != month_of(&items[i - 1])).and_then(|i| month_label(&items[i].date_bucket))
                        })
                        .flatten();
                    div()
                        .relative()
                        .w_full()
                        .h(px(cell + gap))
                        .flex()
                        .justify_center()
                        .gap(px(gap))
                        .children(tiles)
                        .children(label.map(|text| {
                            // Sized by the row, so labels of neighbouring rows do not overlap at 15 columns.
                            let h = (cell * 0.72).clamp(14., 28.);
                            div()
                                .absolute()
                                .top(px((cell - h) / 2.))
                                .left(px(4.))
                                .h(px(h))
                                .px(px(h * 0.4))
                                .flex()
                                .items_center()
                                .rounded_full()
                                .bg(rgba(0x000000a6))
                                .text_size(px(h * 0.55))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(0xffffff))
                                .child(text)
                        }))
                        .into_any_element()
                })
                .collect();
            if timeline && !zooming && !items.is_empty() {
                // Idle: warm the next screenful of rows in the scroll direction (low priority, see `thumbs::prefetch`).
                let span = range_end.saturating_sub(range_start).max(1);
                // At the end of the timeline (where it opens) there is nothing below, so the rows above are warmed.
                let at_end = range_end.saturating_sub(top_rows) >= rows;
                let (from, to) = if store.borrow().moving_up() || at_end {
                    ((range_start.saturating_sub(top_rows)).saturating_sub(span), range_start.saturating_sub(top_rows))
                } else {
                    let after = range_end.saturating_sub(top_rows);
                    (after, after + span)
                };
                let (from, to) = ((from * columns).min(items.len()), (to * columns).min(items.len()));
                let origin = &origin;
                let jobs = items[from..to]
                    .iter()
                    .map(|item| (item.id.as_str(), move || thumbs::thumb_url(origin, &item.id, "thumbnail", item.thumbhash.as_deref())));
                thumbs::prefetch(&store, jobs, side, cx);
            }
            built
        })
        .size_full();
        let (handle, placed) = scroll;
        if let Some(placed) = placed {
            if !items_empty && !placed.replace(true) {
                handle.scroll_to_item(top_rows + rows + bottom_rows - 1, ScrollStrategy::Bottom);
            }
        }
        if placed.is_some() {
            let vh: f32 = window.viewport_size().height.into();
            // Where swipe-to-select finds the tiles (see `selection.rs`).
            self.select.geom.set(HitGeom { width: vw, count, columns, cell, gap, top_rows, top: shift_top + self.rubber.value() });
            self.select.edges.set((lead, vh - (self.bottom.max(10.) + 70.)));
            let mid = vh / 2. - shift_top;
            self.grid_geom.set((row_h, top_rows, mid));
            if self.grid_zooming() {
                // Zooming: the anchor photo stays where it was under the middle of the screen, pixel-exact, also
                // across the switch to the target level.
                let (j, frac) = self.grid_anchor;
                let h = vh - shift_top - shift_bottom;
                let total = (top_rows + rows + bottom_rows) as f32 * row_h;
                let y = (((top_rows + j / columns) as f32 + frac) * row_h - mid).clamp(0., (total - h).max(0.));
                list_scroll_set.set_offset(point(px(0.), px(-y)));
                self.timeline_jump.take();
            } else if let Some(j) = self.timeline_jump.take() {
                handle.scroll_to_item(top_rows + j / columns, ScrollStrategy::Center);
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
        let was_placed = self.timeline_placed.get();
        let list = self.photo_grid("timeline", self.items.clone(), head, (&self.timeline_scroll, Some(&self.timeline_placed)), p, window, cx);
        if !was_placed && self.timeline_placed.get() {
            self.scroll_settle.set(crate::util::now_ms());
        }

        let count = self.items.len();
        // Scrolled away from the newest photos, the header names the dates on screen instead of the count.
        let (first, last) = self.visible.get();
        self.header_span.set(if self.range_bar { (first, last) } else { (0, 0) });
        let span = if self.range_bar { self.items.get(first).zip(self.items.get(last)).and_then(|(a, b)| format_range(&a.date_bucket, &b.date_bucket)) } else { None };
        let subtitle = match span {
            Some(span) => span,
            None if self.status.is_empty() => format!("{count} Fotos"),
            None => self.status.clone(),
        };

        div()
            .size_full()
            .relative()
            .child(div().absolute().top_0().left_0().size_full().flex().flex_col().child(list))
            .child(self.range_probe(cx))
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
                            .children((!self.items.is_empty()).then(|| self.sel_header_button(p, cx)))
                    ),
            )
    }
}
