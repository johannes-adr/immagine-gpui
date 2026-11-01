//! Multi-selection in the timeline grid (iOS Photos): "Auswählen" in the header, tap to toggle, swipe-to-select
//! (a mainly horizontal one-finger drag that starts on a tile paints a range in reading order, see
//! `logic::grid::selection::SelectionPaint`), a bottom bar with add-to-album and delete.
//!
//! Selection is by asset id, so it survives zoom level changes and timeline refreshes. Touch input comes from the
//! raw single-finger events of `Host::install_gestures` (`Pinch::Down/Drag/Up`, routed here by `grid_pinch`): the
//! first 5 px decide between painting (horizontal) and scrolling (anything else; the press is then forgotten and
//! GPUI's own pan scrolls). While painting, touch scroll events are swallowed (see `Immagine::render`) and
//! `sel_tick` scrolls the list itself when the finger rests near the visible top or bottom edge. The mouse takes
//! the same path through window listeners (`sel_input`); a drag that painted must not also click the tile it ends
//! on (`swallow_click`).

use crate::components::icon::{glyph, Glyph};
use super::{Immagine, Screen};
use crate::components::button::{self, Surface};
use crate::components::morph::{self, Morph};
use crate::host::Pinch;
use crate::logic::grid::hit::{autoscroll_speed, HitGeom};
use crate::logic::grid::selection::SelectionPaint;
use crate::logic::models::album_op::AlbumOp;
use crate::logic::models::timeline::TimelineItem;
use crate::theme::{Palette, RED};
use crate::util::now_ms;
use gpui_kit::*;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

/// Finger travel (px) after which a press becomes a drag (and the direction is decided).
const DRAG_SLOP: f32 = 5.;
/// Width of the header button's corner, where a press does not start a selection.
const BUTTON_ZONE: f32 = 150.;
/// The toolbar's round buttons.
const TRASH: f32 = 52.;

struct Press {
    start: (f32, f32),
    index: usize,
    touch: bool,
}

struct Painting {
    paint: SelectionPaint,
    /// The items the indices refer to (the timeline may refresh while painting).
    items: Rc<Vec<TimelineItem>>,
    finger: (f32, f32),
}

#[derive(Default)]
pub(super) struct SelectState {
    pub(super) on: bool,
    /// Selected asset ids, shared with the tile builders.
    pub(super) ids: Rc<RefCell<HashSet<String>>>,
    /// Grid geometry and (header bottom, toolbar top) of the last timeline paint.
    pub(super) geom: Cell<HitGeom>,
    pub(super) edges: Cell<(f32, f32)>,
    press: Option<Press>,
    painting: Option<Painting>,
    swallow_click: bool,
    last_tick: f64,
    /// Delete confirmation, morphing out of the toolbar's trash button (the album picker is a sheet, see `album_picker`).
    pub(super) delete: Morph,
}

impl SelectState {
    pub(super) fn painting(&self) -> bool {
        self.painting.is_some()
    }

    /// An overlay covers the page: gestures belong to it.
    pub(super) fn overlay_open(&self) -> bool {
        self.delete.active()
    }

    pub(super) fn count(&self) -> usize {
        self.ids.borrow().len()
    }
}

/// "Objekte auswählen" / "1 Objekt ausgewählt" / "N Objekte ausgewählt".
pub(super) fn title(count: usize) -> String {
    match count {
        0 => "Objekte auswählen".into(),
        1 => "1 Objekt ausgewählt".into(),
        n => format!("{n} Objekte ausgewählt"),
    }
}

impl Immagine {
    /// Select mode only makes sense on the Alle view of the Mediathek.
    fn select_valid(&self) -> bool {
        self.screen == Screen::Timeline && self.tab == 0 && self.range_mode == 2 && !self.search.open
    }

    /// Called from `render`: leaving the Alle view or the tab ends select mode.
    pub(super) fn select_check(&mut self) {
        if self.select.on && !self.select_valid() {
            self.select_end();
        }
    }

    pub(super) fn select_begin(&mut self, cx: &mut Context<Self>) {
        if self.viewer.is_some() || self.items.is_empty() {
            return;
        }
        self.select.on = true;
        cx.notify();
    }

    pub(super) fn select_end(&mut self) {
        self.select = SelectState { geom: self.select.geom.clone(), edges: self.select.edges.clone(), ids: Rc::new(RefCell::new(HashSet::new())), ..Default::default() };
    }

    fn select_cancel(&mut self, cx: &mut Context<Self>) {
        self.select_end();
        cx.notify();
    }

    fn select_toggle(&mut self, id: &str, cx: &mut Context<Self>) {
        let mut ids = self.select.ids.borrow_mut();
        if !ids.remove(id) {
            ids.insert(id.to_string());
        }
        drop(ids);
        cx.notify();
    }

    /// A tap or click on a tile in select mode (not the end of a painting mouse drag).
    pub(super) fn select_tap(&mut self, id: &str, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.select.swallow_click) {
            return;
        }
        self.select_toggle(id, cx);
    }

    // MARK: - Hit testing

    /// The tile under a screen point, `None` over the header button, the toolbar, gaps and empty space.
    fn sel_tile_at(&self, x: f32, y: f32) -> Option<usize> {
        let (head, toolbar) = self.select.edges.get();
        let g = self.select.geom.get();
        if y >= toolbar || (y < head && x > g.width - BUTTON_ZONE) {
            return None;
        }
        g.item_at(x, g.content_y(y, self.list_offset()))
    }

    fn list_offset(&self) -> f32 {
        self.timeline_scroll.0.borrow().base_handle.offset().y.into()
    }

    fn sel_can_start(&self) -> bool {
        self.select.on && self.grid_active() && !self.grid_zooming() && !self.select.overlay_open() && !self.scrollbar.dragging()
    }

    /// A press on the scrollbar thumb takes over: forget a press that began on a tile.
    pub(super) fn sel_forget_press(&mut self) {
        self.select.press = None;
    }

    // MARK: - Painting

    fn sel_begin_paint(&mut self, press: &Press) {
        let items = self.items.clone();
        let base: HashSet<usize> = {
            let ids = self.select.ids.borrow();
            if ids.is_empty() { HashSet::new() } else { items.iter().enumerate().filter(|(_, it)| ids.contains(&it.id)).map(|(i, _)| i).collect() }
        };
        let selecting = !base.contains(&press.index);
        self.select.painting = Some(Painting { paint: SelectionPaint::new(press.index, selecting, base), items, finger: press.start });
        self.select.press = None;
        self.sel_move_to(press.index);
    }

    fn sel_move_to(&mut self, index: usize) {
        let Some(p) = self.select.painting.as_mut() else { return };
        if index == p.paint.current() && p.paint.started() {
            return;
        }
        let changes = p.paint.move_to(index);
        let mut ids = self.select.ids.borrow_mut();
        for (i, on) in changes {
            if let Some(item) = p.items.get(i) {
                if on {
                    ids.insert(item.id.clone());
                } else {
                    ids.remove(&item.id);
                }
            }
        }
    }

    /// The finger is at (x, y): extend or shrink the painted range to the tile nearest to it.
    fn sel_paint_at(&mut self, x: f32, y: f32) {
        let Some(p) = self.select.painting.as_mut() else { return };
        p.finger = (x, y);
        let g = self.select.geom.get();
        let Some(index) = g.nearest(x, g.content_y(y, self.list_offset())) else { return };
        let last = self.select.painting.as_ref().map_or(0, |p| p.items.len().saturating_sub(1));
        self.sel_move_to(index.min(last));
    }

    fn sel_end_paint(&mut self) {
        self.select.painting = None;
        self.select.press = None;
        self.select.last_tick = 0.;
    }

    /// Called from `render` every frame: while painting with the finger near the top or bottom edge the list
    /// scrolls on its own (faster the closer to the edge) and the range keeps following the finger.
    pub(super) fn sel_tick(&mut self, window: &mut Window) {
        let Some(p) = self.select.painting.as_ref() else {
            self.select.last_tick = 0.;
            return;
        };
        let (fx, fy) = p.finger;
        let (head, toolbar) = self.select.edges.get();
        let speed = autoscroll_speed(fy, head, toolbar);
        let now = now_ms();
        let dt = if self.select.last_tick > 0. { ((now - self.select.last_tick) / 1000.).min(0.05) as f32 } else { 0. };
        if speed == 0. {
            self.select.last_tick = 0.;
            return;
        }
        self.select.last_tick = now;
        let handle = self.timeline_scroll.0.borrow().base_handle.clone();
        let (off, max): (f32, f32) = (handle.offset().y.into(), handle.max_offset().y.into());
        let next = (off - speed * dt).clamp(-max.max(0.), 0.);
        if next != off {
            handle.set_offset(point(px(0.), px(next)));
            self.sel_paint_at(fx, fy);
        }
        window.request_animation_frame();
    }

    // MARK: - Touch (from `Host::install_gestures`)

    /// A second finger (pinch) ends whatever one finger was doing.
    pub(super) fn sel_cancel_touch(&mut self, cx: &mut Context<Self>) {
        if self.select.painting.is_some() || self.select.press.is_some() {
            self.sel_end_paint();
            cx.notify();
        }
    }

    pub(super) fn sel_touch(&mut self, event: Pinch, cx: &mut Context<Self>) {
        match event {
            Pinch::Down { x, y } if self.sel_can_start() => {
                self.select.press = self.sel_tile_at(x, y).map(|index| Press { start: (x, y), index, touch: true });
            }
            Pinch::Drag { x, y } => {
                if self.select.painting.is_some() {
                    self.sel_paint_at(x, y);
                    cx.notify();
                } else if let Some(press) = self.select.press.as_ref().filter(|p| p.touch) {
                    let (dx, dy) = (x - press.start.0, y - press.start.1);
                    if dx.hypot(dy) >= DRAG_SLOP {
                        if dx.abs() > dy.abs() {
                            let press = self.select.press.take().unwrap();
                            self.sel_begin_paint(&press);
                            self.sel_paint_at(x, y);
                            cx.notify();
                        } else {
                            // Mainly vertical: the list scrolls as usual.
                            self.select.press = None;
                        }
                    }
                }
            }
            Pinch::Up { x, y } => {
                if self.select.painting.is_some() {
                    if x >= 0. {
                        self.sel_paint_at(x, y);
                    }
                    self.sel_end_paint();
                    cx.notify();
                } else if self.select.press.as_ref().is_some_and(|p| p.touch) {
                    self.select.press = None;
                }
            }
            _ => {}
        }
    }

    // MARK: - Mouse

    /// Window listeners for the mouse (desktop and browsers with a mouse): a press on a tile followed by a drag
    /// paints in any direction (the wheel scrolls). Capture phase and never stops the event, so the tile's click
    /// still happens for a plain press.
    pub(super) fn sel_input(&self, cx: &mut Context<Self>) -> Canvas<()> {
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
                    v.update(cx, |t, _| {
                        t.select.swallow_click = false;
                        t.select.press = if t.sel_can_start() { t.sel_tile_at(x, y).map(|index| Press { start: (x, y), index, touch: false }) } else { None };
                    })
                    .ok();
                });
                let v = view.clone();
                window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture || e.pressed_button != Some(MouseButton::Left) {
                        return;
                    }
                    let (x, y): (f32, f32) = (e.position.x.into(), e.position.y.into());
                    v.update(cx, |t, cx| {
                        if t.select.painting.is_some() {
                            t.sel_paint_at(x, y);
                            cx.notify();
                        } else if let Some(press) = t.select.press.as_ref().filter(|p| !p.touch) {
                            if (x - press.start.0).hypot(y - press.start.1) >= DRAG_SLOP {
                                let press = t.select.press.take().unwrap();
                                t.sel_begin_paint(&press);
                                t.select.swallow_click = true;
                                t.sel_paint_at(x, y);
                                cx.notify();
                            }
                        }
                    })
                    .ok();
                });
                let v = view.clone();
                window.on_mouse_event(move |e: &MouseUpEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture || e.button != MouseButton::Left {
                        return;
                    }
                    v.update(cx, |t, cx| {
                        if t.select.painting.is_some() {
                            t.sel_end_paint();
                            cx.notify();
                        }
                        t.select.press = None;
                    })
                    .ok();
                });
            },
        )
        .absolute()
        .size_0()
    }

    // MARK: - Actions

    /// Moves the selected photos to the server trash: applied locally at once, sent by the outbox.
    /// Starts a download job for the selected assets that are not stored yet and leaves select mode.
    fn sel_download(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<String> = {
            let stored = self.downloads.ids.borrow();
            let selected = self.select.ids.borrow();
            // Timeline order (newest first), not the hash set's.
            self.items.iter().rev().filter(|i| selected.contains(&i.id) && !stored.contains(&i.id)).map(|i| i.id.clone()).collect()
        };
        self.jobs_download(ids, cx);
        self.select_end();
        cx.notify();
    }

    fn sel_delete(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<String> = self.select.ids.borrow().iter().cloned().collect();
        self.select.delete.reset();
        if ids.is_empty() {
            return;
        }
        let Some(engine) = self.engine.as_mut() else {
            self.status = "Bitte kurz warten, es wird synchronisiert.".into();
            cx.notify();
            return;
        };
        engine.enqueue(&[AlbumOp::trash(ids.clone())]);
        let gone: HashSet<&str> = ids.iter().map(String::as_str).collect();
        Rc::make_mut(&mut self.items).retain(|i| !gone.contains(i.id.as_str()));
        self.select_end();
        self.sync(cx);
        cx.notify();
    }

    // MARK: - Views

    /// "Auswählen" / "Abbrechen" at the top right of the Mediathek header.
    pub(super) fn sel_header_button(&self, p: &Palette, cx: &mut Context<Self>) -> Stateful<Div> {
        let on = self.select.on;
        button::chip("sel-toggle", Surface::new(p.bar_tint), 36., 14., if on { "Abbrechen" } else { "Auswählen" })
            .occlude()
            .active(button::dim)
            .text_size(px(15.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(p.label)
            .on_click(cx.listener(move |this, _, _, cx| if on { this.select_cancel(cx) } else { this.select_begin(cx) }))
    }

    /// The bottom bar of select mode: add to album, count, delete.
    pub(super) fn sel_toolbar(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let count = self.select.count();
        let bottom = self.bottom.max(10.);
        let enabled = count > 0;
        let icon_color = Rgba { a: p.label.a * if enabled { 1. } else { 0.3 }, ..p.label };
        let round = |id: &'static str, g: Glyph, color: Rgba| button::circle(id, Surface::new(p.bar_tint), TRASH).child(glyph(g, 24., color));
        let mut add = round("sel-album", Glyph::AddToAlbum, icon_color);
        let mut trash = round("sel-trash", Glyph::Trash, if enabled { rgb(RED).into() } else { icon_color });
        // Downloads whatever of the selection is not stored yet, as a job (see the jobs sheet).
        let missing = {
            let stored = self.downloads.ids.borrow();
            self.select.ids.borrow().iter().filter(|id| !stored.contains(*id)).count()
        };
        let mut download = round("sel-download", Glyph::Download, if missing > 0 { p.label } else { icon_color });
        if missing > 0 {
            download = download.active(button::dim).on_click(cx.listener(|this, _, _, cx| this.sel_download(cx)));
        }
        if enabled {
            add = add.active(button::dim).on_click(cx.listener(|this, _, _, cx| {
                let ids: Vec<String> = this.select.ids.borrow().iter().cloned().collect();
                this.open_album_picker(ids, true, cx);
            }));
            trash = trash.active(button::dim).on_click(cx.listener(|this, _, _, cx| this.open_sel_delete(cx)));
        }
        div()
            .id("selbar")
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .h(px(64. + bottom + 14.))
            .occlude()
            .child(
                div()
                    .absolute()
                    .left(px(20.))
                    .right(px(20.))
                    .bottom(px(bottom + 4.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(div().flex().flex_row().gap_2().child(add).child(download))
                    .child(
                        button::pill("sel-count", Surface::new(p.bar_tint), 52.)
                            .px(px(14.))
                            .mx_2()
                            .min_w_0()
                            .justify_start()
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.label)
                            .child(title(count)),
                    )
                    // While the confirmation morphs out of the trash button, the button itself is gone.
                    .child(if self.select.delete.active() { div().size(px(TRASH)).flex_none().into_any_element() } else { trash.into_any_element() }),
            )
            .into_any_element()
    }

    fn open_sel_delete(&mut self, cx: &mut Context<Self>) {
        if self.select.delete.open(now_ms()) {
            cx.notify();
        }
    }

    fn close_sel_delete(&mut self, cx: &mut Context<Self>) {
        if self.select.delete.close(now_ms()) {
            cx.notify();
            Morph::finish_after(cx, |t: &mut Self| Some(&mut t.select.delete));
        }
    }

    /// The delete confirmation, from the toolbar's trash button (bottom right) to a card above it.
    pub(super) fn sel_overlay(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let m = self.select.delete;
        if !m.active() {
            return None;
        }
        let n = self.select.count();
        let label = if n == 1 { "Objekt löschen".to_string() } else { format!("{n} Objekte löschen") };
        let size = window.viewport_size();
        let (vw, vh): (f32, f32) = (size.width.into(), size.height.into());
        let bottom = self.bottom.max(10.);
        let (bw, bh) = (292., 200.);
        let content = morph::confirm_content(
            "sel-confirm-delete",
            "Gelöschte Objekte liegen 30 Tage im Papierkorb deines Immich-Servers.",
            label,
            rgb(0xffffff).into(),
            rgb(0xff5a52).into(),
            m.is_open().then(|| cx.listener(|this, _, _, cx| this.sel_delete(cx))),
        );
        let spec = morph::Spec {
            id: "sel-confirm",
            anchor: (vw - 20. - TRASH, vh - (bottom + 4.) - TRASH, TRASH, TRASH),
            card: (vw - 16. - bw, vh - bottom - bh, bw, bh),
            tint: morph::DIALOG_TINT,
            anchor_radius: TRASH / 2.,
            card_radius: 34.,
        };
        morph::overlay(m, spec, now_ms(), window, content, |this: &mut Self, cx| this.close_sel_delete(cx), cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::gpui::TestAppContext;

    #[::core::prelude::v1::test]
    fn titles_count_objects() {
        assert_eq!(title(0), "Objekte auswählen");
        assert_eq!(title(1), "1 Objekt ausgewählt");
        assert_eq!(title(12), "12 Objekte ausgewählt");
    }

    fn items(n: usize) -> Vec<TimelineItem> {
        (0..n)
            .map(|i| TimelineItem {
                id: format!("a{i}"),
                asset_type: "IMAGE".into(),
                thumbhash: None,
                ratio: Some(1.),
                is_favorite: false,
                is_trashed: false,
                date_bucket: "2026-01-01".into(),
                sort_date: None,
                duration: None,
            })
            .collect()
    }

    fn selected(app: &mut Immagine) -> Vec<String> {
        let mut ids: Vec<String> = app.select.ids.borrow().iter().cloned().collect();
        ids.sort();
        ids
    }

    /// 3 columns of 100 px with a 2 px gap, 4 rows (10 photos), no empty rows above, list at y = 0.
    fn open(cx: &mut Context<Immagine>, app: &mut Immagine) {
        app.screen = Screen::Timeline;
        app.items = Rc::new(items(10));
        app.select.geom.set(HitGeom { width: 304., count: 10, columns: 3, cell: 100., gap: 2., top_rows: 0, top: 0. });
        app.select.edges.set((20., 700.));
        app.select_begin(cx);
    }

    fn centre(index: usize) -> (f32, f32) {
        ((index % 3) as f32 * 102. + 52., (index / 3) as f32 * 102. + 50.)
    }

    #[::core::prelude::v1::test]
    fn delete_confirmation_morphs_open_blocks_gestures_and_ends_with_select_mode() {
        let mut cx = TestAppContext::single();
        let (view, cx) = cx.add_window_view(|w, cx| Immagine::new(0., 0., false, false, w, cx));
        view.update(cx, |app, cx| {
            open(cx, app);
            app.select_toggle("a1", cx);
            assert!(!app.select.overlay_open());
            app.open_sel_delete(cx);
            assert!(app.select.delete.is_open() && app.select.overlay_open(), "the card owns the gestures");
            app.close_sel_delete(cx);
            assert!(app.select.delete.active() && !app.select.delete.is_open(), "closing keeps the animation until it finishes");
            app.select.delete.reset();
            app.open_sel_delete(cx);
            app.sel_delete(cx);
            assert!(!app.select.delete.active(), "confirming resets the morph");
        });
    }

    #[::core::prelude::v1::test]
    fn swipe_selects_a_range_and_a_swipe_on_a_selected_tile_deselects() {
        let mut cx = TestAppContext::single();
        let (view, cx) = cx.add_window_view(|w, cx| Immagine::new(0., 0., false, false, w, cx));
        view.update(cx, |app, cx| {
            open(cx, app);
            assert!(app.select.on);
            // Down on tile 1, sideways: paints from there to the tile under the finger, rows included.
            let (x, y) = centre(1);
            app.sel_touch(Pinch::Down { x, y }, cx);
            let (x2, y2) = centre(1);
            app.sel_touch(Pinch::Drag { x: x2 + 20., y: y2 + 2. }, cx);
            assert_eq!(selected(app), vec!["a1"], "the anchor selects at once");
            let (x, y) = centre(4);
            app.sel_touch(Pinch::Drag { x, y }, cx);
            assert_eq!(selected(app), vec!["a1", "a2", "a3", "a4"], "down a row: everything in between");
            let (x, y) = centre(2);
            app.sel_touch(Pinch::Up { x, y }, cx);
            assert_eq!(selected(app), vec!["a1", "a2"], "moving back gives photos back");
            assert!(!app.select.painting());

            // Starting on a selected tile deselects.
            let (x, y) = centre(1);
            app.sel_touch(Pinch::Down { x, y }, cx);
            app.sel_touch(Pinch::Drag { x: x + 12., y }, cx);
            assert!(app.select.painting());
            let (x, y) = centre(2);
            app.sel_touch(Pinch::Up { x, y }, cx);
            assert!(selected(app).is_empty());
        });
    }

    #[::core::prelude::v1::test]
    fn vertical_drag_scrolls_instead_and_taps_toggle() {
        let mut cx = TestAppContext::single();
        let (view, cx) = cx.add_window_view(|w, cx| Immagine::new(0., 0., false, false, w, cx));
        view.update(cx, |app, cx| {
            open(cx, app);
            let (x, y) = centre(4);
            app.sel_touch(Pinch::Down { x, y }, cx);
            app.sel_touch(Pinch::Drag { x: x + 2., y: y + 30. }, cx);
            assert!(!app.select.painting() && app.select.ids.borrow().is_empty(), "mainly vertical: the list scrolls");
            app.sel_touch(Pinch::Up { x, y: y + 30. }, cx);

            app.select_tap("a3", cx);
            app.select_tap("a4", cx);
            app.select_tap("a3", cx);
            assert_eq!(selected(app), vec!["a4"]);
            assert_eq!(app.select.count(), 1);

            // Down on the toolbar or the header button starts nothing.
            app.sel_touch(Pinch::Down { x: 50., y: 720. }, cx);
            app.sel_touch(Pinch::Drag { x: 90., y: 720. }, cx);
            assert!(!app.select.painting());

            // A tab switch ends select mode and clears the selection.
            app.tab = 1;
            app.select_check();
            assert!(!app.select.on && app.select.count() == 0);
        });
    }
}
