//! Scrollbar of the timeline grid (Alle view of the Mediathek): a thin thumb at the right edge that shows while
//! the list scrolls and fades out about 1.2 s after it stops. Grabbing it (touch or mouse) scrolls the list to
//! the thumb's position, with a bubble naming the month and year there ("März 2024"). Pure geometry, hit area and
//! fade: `logic::grid::scrollbar`.
//!
//! The thumb is painted by a `canvas` that reads the list's scroll offset at paint time, so it follows the list
//! even when a scroll does not re-render the view; it is the only thing that asks for animation frames, and only
//! while fading or widening. After the last movement one timer wakes the view when the hold time is over (no
//! frames while idle). Input comes the way the selection paint gets it: raw `Pinch::Down/Drag/Up` for touch
//! (routed by `grid_pinch`) and capture-phase window listeners for the mouse. While the thumb is held, touch scroll
//! events are swallowed in `render` (the finger's own pan must not also scroll), `sync_range_bar` keeps the current
//! bar, and the press must not reach tiles (`sb_swallow_click`, mouse-down stopped in the capture phase).

use super::Immagine;
use crate::host::Pinch;
use crate::logic::grid::scrollbar::{self as geo, EDGE_GAP, WIDTH, WIDTH_GRABBED};
use crate::theme::Palette;
use crate::util::now_ms;
use gpui_kit::*;
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

/// Per-frame state shared with the paint callback.
#[derive(Default)]
pub(super) struct Shared {
    primed: Cell<bool>,
    last_offset: Cell<f32>,
    last_active: Cell<f64>,
    /// (top, length) of the thumb as painted last, `None` while invisible: this is what a press can grab.
    thumb: Cell<Option<(f32, f32)>>,
    /// Screen y of the track's ends.
    track: Cell<(f32, f32)>,
    /// A wake-up timer is pending.
    timer: Cell<bool>,
    /// 0..1 how far the thumb has widened.
    wide: Cell<f32>,
    last_paint: Cell<f64>,
    grabbed: Cell<bool>,
    /// Tests have no layout, so no `max_offset`.
    #[cfg(test)]
    max: Cell<Option<f32>>,
}

#[derive(Default)]
pub(super) struct ScrollbarState {
    shared: Rc<Shared>,
    /// Pt between the top of the thumb and the pointer.
    drag: Option<f32>,
    swallow_click: bool,
}

impl ScrollbarState {
    pub(super) fn dragging(&self) -> bool {
        self.drag.is_some()
    }
}

impl Immagine {
    /// The thumb may show and be grabbed: timeline grid at rest, nothing on top of it.
    fn sb_enabled(&self) -> bool {
        self.grid_active()
            && !self.grid_zooming()
            && !self.settings.sheet.visible()
            && !self.jobs.sheet.visible()
            && !self.select.painting()
            && !self.select.overlay_open()
    }

    /// Called from `render`: when the thumb is not allowed any more, a drag ends and the fade state resets.
    pub(super) fn sb_check(&mut self) {
        if !self.sb_enabled() {
            self.sb_release();
            self.scrollbar.shared.primed.set(false);
            self.scrollbar.shared.thumb.set(None);
        }
    }

    /// A tap that started on the thumb must not open the photo underneath.
    pub(super) fn sb_swallow_click(&mut self) -> bool {
        std::mem::take(&mut self.scrollbar.swallow_click)
    }

    fn sb_scroll(&self) -> (f32, f32) {
        let h = self.timeline_scroll.0.borrow().base_handle.clone();
        #[cfg(test)]
        if let Some(max) = self.scrollbar.shared.max.get() {
            return (h.offset().y.into(), max);
        }
        (h.offset().y.into(), h.max_offset().y.into())
    }

    fn sb_thumb_now(&self) -> Option<(f32, f32)> {
        let (top, bottom) = self.scrollbar.shared.track.get();
        let (offset, max) = self.sb_scroll();
        geo::thumb(top, bottom, max, offset)
    }

    // MARK: - Dragging

    /// Starts a drag when (x, y) is on the visible thumb. Returns whether it took the press.
    fn sb_grab(&mut self, x: f32, y: f32) -> bool {
        if self.scrollbar.drag.is_some() {
            return true;
        }
        if !self.sb_enabled() {
            return false;
        }
        let Some((top, len)) = self.scrollbar.shared.thumb.get() else { return false };
        let width = self.select.geom.get().width;
        if !geo::hit(x, y, width, top, len) {
            return false;
        }
        self.scrollbar.drag = Some((y - top).clamp(0., len));
        self.scrollbar.shared.grabbed.set(true);
        self.scrollbar.swallow_click = true;
        // A press that began on a tile must not turn into a selection paint.
        self.sel_forget_press();
        true
    }

    fn sb_move(&mut self, y: f32) {
        let Some(grab) = self.scrollbar.drag else { return };
        let (top, bottom) = self.scrollbar.shared.track.get();
        let (offset, max) = self.sb_scroll();
        let Some((_, len)) = geo::thumb(top, bottom, max, offset) else { return };
        let target = geo::offset_for(y, grab, top, bottom, len, max);
        self.timeline_scroll.0.borrow().base_handle.set_offset(point(px(0.), px(target)));
        self.scrollbar.shared.last_active.set(now_ms());
    }

    fn sb_release(&mut self) {
        if self.scrollbar.drag.take().is_some() {
            self.scrollbar.shared.grabbed.set(false);
            self.scrollbar.shared.last_active.set(now_ms());
        }
    }

    /// Touch (from `grid_pinch`). Returns true when the scrollbar used the event.
    pub(super) fn sb_touch(&mut self, event: Pinch, cx: &mut Context<Self>) -> bool {
        match event {
            Pinch::Down { x, y } => {
                self.scrollbar.swallow_click = false;
                let taken = self.sb_grab(x, y);
                if taken {
                    self.sb_move(y);
                    cx.notify();
                }
                taken
            }
            Pinch::Drag { y, .. } if self.scrollbar.drag.is_some() => {
                self.sb_move(y);
                cx.notify();
                true
            }
            Pinch::Up { x, y } if self.scrollbar.drag.is_some() => {
                if x >= 0. {
                    self.sb_move(y);
                }
                self.sb_release();
                cx.notify();
                true
            }
            _ => false,
        }
    }

    /// A second finger (pinch) lets go of the thumb.
    pub(super) fn sb_cancel_touch(&mut self, cx: &mut Context<Self>) {
        if self.scrollbar.drag.is_some() {
            self.sb_release();
            cx.notify();
        }
    }

    /// Mouse: window listeners in the capture phase. A press on the thumb is stopped there, so no tile sees it.
    pub(super) fn sb_input(&self, cx: &mut Context<Self>) -> Canvas<()> {
        let view = cx.weak_entity();
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let v = view.clone();
                window.on_mouse_event(move |e: &MouseDownEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture || e.button != MouseButton::Left {
                        return;
                    }
                    let (x, y): (f32, f32) = (e.position.x.into(), e.position.y.into());
                    let mut taken = false;
                    v.update(cx, |t, cx| {
                        t.scrollbar.swallow_click = false;
                        taken = t.sb_grab(x, y);
                        if taken {
                            t.sb_move(y);
                            cx.notify();
                        }
                    })
                    .ok();
                    if taken {
                        cx.stop_propagation();
                    }
                });
                let v = view.clone();
                window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture || e.pressed_button != Some(MouseButton::Left) {
                        return;
                    }
                    let y: f32 = e.position.y.into();
                    let mut used = false;
                    v.update(cx, |t, cx| {
                        if t.scrollbar.drag.is_some() {
                            used = true;
                            t.sb_move(y);
                            cx.notify();
                        }
                    })
                    .ok();
                    if used {
                        cx.stop_propagation();
                    }
                });
                let v = view.clone();
                window.on_mouse_event(move |e: &MouseUpEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture || e.button != MouseButton::Left {
                        return;
                    }
                    v.update(cx, |t, cx| {
                        if t.scrollbar.drag.is_some() {
                            t.sb_release();
                            cx.notify();
                        }
                    })
                    .ok();
                });
            },
        )
        .absolute()
        .size_0()
    }

    // MARK: - Views

    /// The thumb (painted) and, while grabbed, the date bubble. `None` when the thumb is not allowed.
    pub(super) fn sb_layer(&self, p: &Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.sb_enabled() || self.items.is_empty() {
            return None;
        }
        let sh = self.scrollbar.shared.clone();
        let (head, toolbar) = self.select.edges.get();
        sh.track.set((head + 6., toolbar - 6.));
        let handle = self.timeline_scroll.0.borrow().base_handle.clone();
        let width = self.select.geom.get().width;
        let dark = self.dark;
        let view = cx.weak_entity();
        let paint = canvas(
            |_, _, _| {},
            move |_, _, window, cx| {
                let now = now_ms();
                let (offset, max): (f32, f32) = (handle.offset().y.into(), handle.max_offset().y.into());
                if !sh.primed.replace(true) {
                    sh.last_offset.set(offset);
                } else if (offset - sh.last_offset.get()).abs() > 0.01 {
                    sh.last_offset.set(offset);
                    sh.last_active.set(now);
                }
                if sh.grabbed.get() {
                    sh.last_active.set(now);
                }
                let (top, bottom) = sh.track.get();
                let alpha = geo::alpha(now - sh.last_active.get());
                let Some((t, len)) = geo::thumb(top, bottom, max, offset).filter(|_| alpha > 0.) else {
                    sh.thumb.set(None);
                    return;
                };
                // Widen towards the grabbed width in ~120 ms.
                let target = if sh.grabbed.get() { 1. } else { 0. };
                let dt = (now - sh.last_paint.get()).clamp(0., 50.) as f32;
                sh.last_paint.set(now);
                let mut wide = sh.wide.get();
                wide += (target - wide).clamp(-dt / 120., dt / 120.);
                sh.wide.set(wide);
                let w = WIDTH + (WIDTH_GRABBED - WIDTH) * wide;
                let bounds = Bounds::new(point(px(width - EDGE_GAP - w), px(t)), size(px(w), px(len)));
                let (fill_c, edge_c) = if dark { (0xd8d8dce0u32, 0x00000066u32) } else { (0x6c6c70d9, 0xffffff80) };
                let fade = |c: u32| {
                    let mut c: Hsla = rgba(c).into();
                    c.a *= alpha;
                    c
                };
                window.paint_quad(quad(bounds, px(w / 2.), fade(fill_c), px(0.5), fade(edge_c), BorderStyle::default()));
                sh.thumb.set(Some((t, len)));
                if alpha < 1. || wide != target {
                    window.request_animation_frame();
                } else if !sh.grabbed.get() && !sh.timer.replace(true) {
                    // Idle at full opacity: one timer wakes the view when the hold time is over.
                    let wait = (geo::HOLD_MS - (now - sh.last_active.get())).max(0.) + 30.;
                    let view = view.clone();
                    let sh = sh.clone();
                    window.defer(cx, move |_, cx| {
                        let _ = view.update(cx, |_, cx| {
                            cx.spawn(async move |this, cx| {
                                cx.background_executor().timer(Duration::from_millis(wait as u64)).await;
                                sh.timer.set(false);
                                let _ = this.update(cx, |_, cx| cx.notify());
                            })
                            .detach();
                        });
                    });
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        let bubble = self.scrollbar.drag.and_then(|_| self.sb_bubble(p));
        Some(div().absolute().top_0().left_0().size_full().child(paint).children(bubble).into_any_element())
    }

    /// "März 2024" beside the grabbed thumb.
    fn sb_bubble(&self, p: &Palette) -> Option<Div> {
        let (top, len) = self.sb_thumb_now()?;
        let (offset, max) = self.sb_scroll();
        let (t0, t1) = self.scrollbar.shared.track.get();
        let frac = (-offset / max).clamp(0., 1.);
        // The photo at the thumb's relative height on screen: the first row at the top, the last at the bottom.
        let y = t0 + frac * (t1 - t0);
        let i = geo::item_at(&self.select.geom.get(), offset, y)?;
        let label = geo::month_year(&self.items.get(i)?.date_bucket)?;
        let h = 38.;
        let mid = top + len / 2.;
        let (bg, border) = if self.dark { (rgba(0x2c2c2ef2), rgba(0xffffff26)) } else { (rgba(0xf6f6f8f2), rgba(0x0000001f)) };
        Some(
            div()
                .absolute()
                .right(px(EDGE_GAP + WIDTH_GRABBED + 12.))
                .top(px((mid - h / 2.).clamp(t0, (t1 - h).max(t0))))
                .h(px(h))
                .px(px(16.))
                .flex()
                .items_center()
                .rounded_full()
                .bg(bg)
                .border_1()
                .border_color(border)
                .text_size(px(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.label)
                .child(label),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::grid::hit::HitGeom;
    use crate::logic::models::timeline::TimelineItem;
    use super::super::Screen;
    use ::gpui::TestAppContext;

    fn items(n: usize) -> Vec<TimelineItem> {
        (0..n)
            .map(|i| TimelineItem {
                id: format!("a{i}"),
                asset_type: "IMAGE".into(),
                thumbhash: None,
                ratio: Some(1.),
                is_favorite: false,
                is_trashed: false,
                date_bucket: format!("2024-{:02}-01", i / 3 % 12 + 1),
                sort_date: None,
                duration: None,
            })
            .collect()
    }

    /// 300 photos in 3 columns of 100 pt (102 pt rows): max offset 9000, track 100..700, thumb painted at the top.
    fn open(app: &mut Immagine) {
        app.screen = Screen::Timeline;
        app.items = Rc::new(items(300));
        app.select.geom.set(HitGeom { width: 306., count: 300, columns: 3, cell: 100., gap: 2., top_rows: 0, top: 0. });
        app.select.edges.set((94., 706.));
        let sh = &app.scrollbar.shared;
        sh.track.set((100., 700.));
        sh.max.set(Some(9000.));
        let (top, len) = geo::thumb(100., 700., 9000., 0.).unwrap();
        sh.thumb.set(Some((top, len)));
    }

    fn offset(app: &Immagine) -> f32 {
        app.timeline_scroll.0.borrow().base_handle.offset().y.into()
    }

    #[::core::prelude::v1::test]
    fn dragging_the_thumb_scrolls_and_swallows_the_press() {
        let mut cx = TestAppContext::single();
        let (view, cx) = cx.add_window_view(|w, cx| Immagine::new(0., 0., false, false, w, cx));
        view.update(cx, |app, cx| {
            open(app);
            // Select mode on: the thumb still wins over a tile at the edge.
            app.select_begin(cx);
            // Beside the thumb's reach, and away from the edge: nothing is taken.
            assert!(!app.sb_touch(Pinch::Down { x: 100., y: 120. }, cx));
            assert!(!app.sb_touch(Pinch::Down { x: 300., y: 500. }, cx));
            assert!(!app.scrollbar.dragging());

            let (top, _) = app.scrollbar.shared.thumb.get().unwrap();
            assert!(app.sb_touch(Pinch::Down { x: 300., y: top + 10. }, cx));
            assert!(app.scrollbar.dragging());
            assert!(app.sb_touch(Pinch::Drag { x: 300., y: top + 10. + 250. }, cx));
            let mid = offset(app);
            assert!(mid < -1000. && mid > -9000., "moved down: {mid}");
            // Past the track's end: the end of the list.
            assert!(app.sb_touch(Pinch::Drag { x: 300., y: 9999. }, cx));
            assert_eq!(offset(app), -9000.);
            assert!(app.sb_touch(Pinch::Up { x: 300., y: 0. }, cx), "x < 0: no position");
            assert!(!app.scrollbar.dragging());
            assert!(app.sb_swallow_click(), "the tap that ends it does not open a photo");
            assert!(!app.sb_swallow_click());
            // Without a drag nothing is consumed.
            assert!(!app.sb_touch(Pinch::Drag { x: 300., y: 300. }, cx));
        });
    }

    #[::core::prelude::v1::test]
    fn hidden_zooming_or_sheet_open_means_no_grab() {
        let mut cx = TestAppContext::single();
        let (view, cx) = cx.add_window_view(|w, cx| Immagine::new(0., 0., false, false, w, cx));
        view.update(cx, |app, cx| {
            open(app);
            let (top, _) = app.scrollbar.shared.thumb.get().unwrap();
            app.scrollbar.shared.thumb.set(None);
            assert!(!app.sb_touch(Pinch::Down { x: 300., y: top + 5. }, cx), "invisible thumb");
            app.scrollbar.shared.thumb.set(Some((top, 100.)));
            app.grid_pinching = true;
            assert!(!app.sb_touch(Pinch::Down { x: 300., y: top + 5. }, cx), "zooming");
            app.grid_pinching = false;
            app.range_mode = 0;
            assert!(!app.sb_touch(Pinch::Down { x: 300., y: top + 5. }, cx), "not the Alle view");
            app.range_mode = 2;
            assert!(app.sb_touch(Pinch::Down { x: 300., y: top + 5. }, cx));
            // A second finger lets go; render-time check ends it too.
            app.sb_cancel_touch(cx);
            assert!(!app.scrollbar.dragging());
            assert!(app.sb_touch(Pinch::Down { x: 300., y: top + 5. }, cx));
            app.grid_pinching = true;
            app.sb_check();
            assert!(!app.scrollbar.dragging(), "zoom ends the drag");
        });
    }

    #[::core::prelude::v1::test]
    fn bubble_names_the_month_at_the_thumb() {
        let mut cx = TestAppContext::single();
        let (view, cx) = cx.add_window_view(|w, cx| Immagine::new(0., 0., false, false, w, cx));
        view.update(cx, |app, cx| {
            open(app);
            let p = Palette::new(false);
            let (top, _) = app.scrollbar.shared.thumb.get().unwrap();
            assert!(app.sb_touch(Pinch::Down { x: 300., y: top + 5. }, cx));
            assert!(app.sb_bubble(&p).is_some());
            // At the very start the first photo's month; the generated dates cycle through the months.
            let g = app.select.geom.get();
            let i = geo::item_at(&g, offset(app), 100.).unwrap();
            assert_eq!(i, 0);
            assert_eq!(geo::month_year(&app.items[i].date_bucket).as_deref(), Some("Januar 2024"));
        });
    }
}
