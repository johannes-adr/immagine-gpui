//! Timeline: all photos newest first, a virtualized grid of server thumbnails.

use super::{thumbs, Immagine};
use crate::components::glass::glass_blur;
use crate::theme::Palette;
use crate::util::now_ms;
use crate::logic::models::timeline::TimelineItem;
use gpui_kit::*;
use std::rc::Rc;

const GAP: f32 = 2.;
/// How far the timeline can be pulled past its ends, and how long it takes to spring back.
const MAX_OVERSCROLL: f32 = 150.;
const BOUNCE_MS: f64 = 380.;
/// The finger counts as lifted when no scroll event arrived for this long.
const QUIET_MS: f64 = 70.;
/// Coasting from a fling may keep the band out this long before it springs back.
const COAST_MS: f64 = 110.;

/// Rubber band state: the offset past the end (positive pulls the content down), when the last scroll event
/// came in and, once released, when the spring-back began and from where.
#[derive(Clone, Copy, Default)]
pub(super) struct Over {
    value: f32,
    last: f64,
    from: f32,
    released_at: f64,
    /// A finger is on the screen (between the start and the end of a pan), and since when the band is out.
    down: bool,
    since: f64,
}

impl Immagine {
    /// A virtualized, scrollable photo grid with a spacer for the tab bar at its end. Tapping a tile opens the
    /// viewer on `items`. Reused by the timeline, album pages and person pages; `id` must be unique per screen.
    /// `lead` is empty space above the first row (for a translucent header laid over the list).
    pub(super) fn photo_grid(
        &self,
        id: &'static str,
        items: Rc<Vec<TimelineItem>>,
        lead: f32,
        scroll: Option<(&UniformListScrollHandle, &std::cell::Cell<bool>)>,
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
                                        map.insert(idx, bounds);
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
        let list = match scroll {
            Some((handle, placed)) => {
                if !items_empty && !placed.replace(true) {
                    handle.scroll_to_item(top_rows + rows + bottom_rows - 1, ScrollStrategy::Bottom);
                }
                list.track_scroll(handle)
            }
            None => list,
        };
        // The list's on-screen box, so the closing viewer can clip its shrinking photo to it.
        // (An absolute box with only top and bottom set does not give its percentage-sized child a height, so the
        // timeline, which fills the screen, gets an explicit one.)
        let vh: f32 = window.viewport_size().height.into();
        let list = if lead > 0. {
            div().absolute().top(px(shift_top + self.overscroll.get().value)).left_0().w_full().h(px(vh - shift_top - shift_bottom)).child(list)
        } else {
            div().size_full().child(list)
        };
        div().flex_1().relative().child(list).child(canvas(move |bounds, _, _| grid_bounds.set(bounds), |_, _, _, _| {}).absolute().top_0().left_0().size_full())
    }

    /// Called for every scroll event before the list sees it (capture phase). Returns true when the event was used up
    /// by the rubber band: the list is at an end and the finger pushes on, or the band is pulled out and the finger
    /// comes back.
    pub(super) fn overscroll_event(&mut self, dy: f32, phase: TouchPhase, cx: &mut Context<Self>) -> bool {
        if self.tab != 0 || self.screen != super::Screen::Timeline || self.viewer.as_ref().is_some_and(|v| !v.closing) {
            return false;
        }
        let handle = self.timeline_scroll.0.borrow().base_handle.clone();
        let (offset, max): (f32, f32) = (handle.offset().y.into(), handle.max_offset().y.into());
        let (at_top, at_bottom) = (offset >= -0.5, offset <= -max + 0.5);
        let mut o = self.overscroll.get();
        let now = now_ms();
        let fresh = phase == TouchPhase::Started;
        match phase {
            TouchPhase::Started => o.down = true,
            TouchPhase::Ended | TouchPhase::Cancelled => {
                o.down = false;
                // The finger lifted: spring back at once, the coasting that follows is swallowed.
                if o.value != 0. && o.released_at == 0. {
                    o.released_at = now;
                    o.from = o.value;
                }
            }
            TouchPhase::Moved => {}
        }
        // Coasting (no finger) may push the band out for a moment, then it springs back.
        if !o.down && o.value != 0. && o.released_at == 0. && now - o.since > COAST_MS {
            o.released_at = now;
            o.from = o.value;
        }
        self.overscroll.set(o);
        if dy == 0. {
            return o.released_at != 0. && o.value != 0. && !fresh;
        }
        // While it springs back, the coasting of the last fling is swallowed (it would fight the spring); only a
        // new touch takes the band over again.
        if o.released_at != 0. && o.value != 0. && !fresh {
            return true;
        }
        let pulled = o.value != 0.;
        let outward = (at_top && dy > 0.) || (at_bottom && dy < 0.);
        if !pulled && !outward {
            return false;
        }
        // Pushing on gets harder the farther out it is; coming back is one to one.
        let moving_out = o.value == 0. || (o.value > 0.) == (dy > 0.);
        let step = if moving_out { dy * 0.5 / (1. + o.value.abs() / 90.) } else { dy };
        let next = o.value + step;
        if o.value == 0. {
            o.since = now;
        }
        o.value = if o.value != 0. && (next > 0.) != (o.value > 0.) { 0. } else { next.clamp(-MAX_OVERSCROLL, MAX_OVERSCROLL) };
        o.last = now;
        o.released_at = 0.;
        self.overscroll.set(o);
        cx.notify();
        true
    }

    /// Springs the band back once the finger is gone; asks for frames until it is at rest.
    pub(super) fn overscroll_tick(&self, window: &mut Window) {
        let mut o = self.overscroll.get();
        if o.value == 0. {
            return;
        }
        let now = now_ms();
        if o.released_at == 0. {
            if now - o.last < QUIET_MS {
                window.request_animation_frame();
                return;
            }
            o.released_at = now;
            o.from = o.value;
        }
        let t = ((now - o.released_at) / BOUNCE_MS).clamp(0., 1.) as f32;
        let e = 1. - (1. - t).powi(3);
        o.value = if t >= 1. { 0. } else { o.from * (1. - e) };
        self.overscroll.set(o);
        if o.value != 0. {
            window.request_animation_frame();
        }
    }

    pub(super) fn timeline_view(&self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The header is a glass strip laid over the grid, which scrolls underneath it (blurred).
        let head = self.top.max(12.) + 78.;
        let list = self.photo_grid("timeline", self.items.clone(), head, Some((&self.timeline_scroll, &self.timeline_placed)), p, window, cx);

        let count = self.items.len();
        let subtitle = if self.status.is_empty() { format!("{count} Fotos") } else { self.status.clone() };

        div()
            .size_full()
            .relative()
            .child(overscroll_input(cx.weak_entity()))
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
                                    .id("logout")
                                    .text_size(px(15.))
                                    .text_color(p.secondary)
                                    .active(|s| s.opacity(0.6))
                                    .child("Abmelden")
                                    .on_click(cx.listener(|this, _, _, cx| this.log_out(cx))),
                            ),
                    ),
            )
    }
}

/// Feeds scroll events to the rubber band before the list gets them (window listener, capture phase).
fn overscroll_input(view: WeakEntity<Immagine>) -> Canvas<()> {
    canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            let view = view.clone();
            window.on_mouse_event(move |e: &ScrollWheelEvent, phase, _, cx| {
                if phase != DispatchPhase::Capture {
                    return;
                }
                let dy: f32 = e.delta.pixel_delta(px(16.)).y.into();
                if view.update(cx, |t, cx| t.overscroll_event(dy, e.touch_phase, cx)).unwrap_or(false) {
                    cx.stop_propagation();
                }
            });
        },
    )
    .absolute()
    .size_0()
}
