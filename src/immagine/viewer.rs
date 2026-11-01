//! Full-screen photo viewer: zoom-in from the tapped tile, swipe left/right between photos, swipe down or
//! "Fertig" to close; double-tap, trackpad pinch (ctrl+wheel) and two-finger pinch zoom, drag to pan;
//! favorite toggle, info panel (EXIF), video playback. Pages use the grid thumbnail (already loaded) as a
//! stand-in until the larger preview arrives, and the full-size rendition once zoomed in.

use super::{thumbs, Immagine};
use crate::components::glass::glass_div_with;
use crate::components::morph::{self, Morph};
use crate::logic::client::ImmichClient;
use crate::logic::crop_geometry::{self, CropAspect, CropHandle, HANDLE_TOLERANCE};
use crate::logic::grid::layout::{Point as CPoint, Rect as CRect, Size as CSize};
use crate::logic::models::album_op::AlbumOp;
use crate::logic::models::asset_edit::AssetEdit;
use crate::logic::models::asset::Asset;
use crate::logic::models::asset_details::InfoFormat;
use crate::logic::models::asset_info::AssetInfoFormat;
use crate::logic::models::timeline::TimelineItem;
use crate::logic::store::Store;
use crate::theme::{BLUE, RED};
use crate::util::{anim, lerp, now_ms};
use super::video_player::{fmt_time, Player, Status};
use crate::host::{Current as H, Host, Pinch};
use gpui_kit::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// A swipe farther than this fraction of the screen width turns the page.
const PAGE_FRACTION: f32 = 0.2;
/// Releasing a downward drag with the photo shrunk below this scale (1 = full size) closes the viewer.
const CLOSE_SCALE: f32 = 0.82;
/// ... or a quick downward flick (px per move event) after at least this much travel.
const FLICK_SPEED: f32 = 14.;
/// Length of the open and close morphs.
pub(super) const OPEN_MS: u64 = 450;
pub(super) const CLOSE_MS: u64 = 450;
/// Black strip between neighbouring photos while swiping, so their edges stay apart.
const PAGE_GAP: f32 = 20.;
const MAX_ZOOM: f32 = 6.;
const DOUBLE_TAP_ZOOM: f32 = 2.5;
/// Above this zoom the full-size rendition is loaded on top of the preview.
const FULLSIZE_ZOOM: f32 = 1.2;
/// Movement (px) that turns a press into a drag, so a tap with a little jitter still counts as a tap.
const TAP_SLOP: f32 = 8.;
const CHROME_MS: f64 = 180.;
/// Dark grey of the viewer's glass buttons, over the viewer's black background.
const VIEWER_TINT: Hsla = Hsla { h: 0., s: 0., l: 0.28, a: 0.62 };
/// Darker tint of the delete confirmation, which is bigger and carries text.
const DIALOG_TINT: Hsla = Hsla { h: 0., s: 0., l: 0.12, a: 0.94 };
const DIALOG_SIZE: (f32, f32) = (292., 268.);
/// The "..." menu: width and height of one row.
const MENU_WIDTH: f32 = 262.;
const MENU_ROW: f32 = 52.;
/// Smallest side of the crop box (fraction of the image).
const MIN_CROP: f64 = 0.08;

/// The crop editor: the box in normalized image coordinates, the handle being dragged.
struct Editor {
    rect: CRect,
    aspect: CropAspect,
    /// Handle grabbed, the box when it was grabbed and the pointer position then.
    grab: Option<(CropHandle, CRect, (f32, f32))>,
    /// Pixel size of the image, known once the asset record arrived (needed to save).
    pixels: Option<(f64, f64)>,
    saving: bool,
    /// Image rectangle (x, y, w, h) of the last render, for hit testing.
    area: Cell<(f32, f32, f32, f32)>,
}

#[derive(Clone, Copy, PartialEq)]
enum DragMode {
    None,
    /// Finger down, direction not decided yet.
    Undecided,
    /// Horizontal: turns the page.
    Page,
    /// Downward: the photo follows the finger and shrinks; released small enough it closes.
    Dismiss,
}

/// An ease-out back to rest, driven by the clock from the render pass (one step per animation frame).
#[derive(Clone, Copy)]
struct Snap {
    start: f64,
    dx0: f32,
    dy0: f32,
    ds0: f32,
}

const SNAP_MS: f64 = 260.;

/// Where the photo was (full-viewport box before aspect fitting) and how faded the backdrop when a drag closed it.
#[derive(Clone, Copy)]
struct Release {
    rect: (f32, f32, f32, f32),
    fade: f32,
}

pub(super) struct Viewer {
    items: Rc<Vec<TimelineItem>>,
    index: usize,
    from: Option<Bounds<Pixels>>,
    pub(super) closing: bool,
    /// Decided when closing starts: the tile of the shown photo was on screen, so the photo shrinks into it
    /// (following it while the grid scrolls) instead of fading out.
    into_tile: bool,
    pub(super) serial: u32,
    /// Pointer position where the current drag started, the last one seen, and the distance moved.
    start: Option<(f32, f32)>,
    last: (f32, f32),
    moved: f32,
    dx: f32,
    dy: f32,
    /// Scale of the photo while dismiss-dragging (1 = full size), scaled about `anchor`.
    ds: f32,
    anchor: (f32, f32),
    mode: DragMode,
    vel_y: f32,
    release: Option<Release>,
    snap: Option<Snap>,
    /// Controls' visibility when closing began, and when it did: they fade out over the first part of the morph.
    chrome0: f32,
    /// Tap on the photo hides the controls; `chrome_at` is when that last changed (they fade over `CHROME_MS`).
    chrome_hidden: bool,
    chrome_at: f64,
    pub(super) close_at: f64,
    /// A drag just ended: the next render cancels the scroll coast GPUI starts from the same finger.
    stop_momentum: Cell<bool>,
    zoom: f32,
    pan: (f32, f32),
    /// Two fingers are down: single-pointer drags are ignored.
    pinching: bool,
    /// Viewport size seen by the last render (needed to clamp the pan).
    viewport: Cell<(f32, f32)>,
    info: bool,
    details: Option<Rc<Asset>>,
    /// The native player while a video plays (see `video_player`).
    video: Option<Rc<RefCell<Player>>>,
    /// Left, top, width and height (px) of the scrubber's track, measured by the last render, and the fraction under the finger.
    scrub_track: Rc<Cell<(f32, f32, f32, f32)>>,
    scrub_frac: f32,
    /// Delete confirmation morphing out of the trash button (bottom right).
    trash: Morph,
    /// The "..." menu morphing out of its button (top right), and whether the shown photo was archived when it opened.
    menu: Morph,
    menu_archived: bool,
    /// Screen rect of the info panel as last drawn (raw touches inside it must not page or dismiss the viewer), its
    /// scroll position, and whether the finger now down went down inside it.
    info_rect: Rc<Cell<Option<(f32, f32, f32, f32)>>>,
    info_scroll: ScrollHandle,
    touch_in_panel: bool,
    editor: Option<Editor>,
}

/// Photo scale while dragging down by `dy`: 1 at rest, 0.5 at half a screen height.
fn dismiss_scale(dy: f32, vh: f32) -> f32 {
    1. - 0.5 * (dy.max(0.) / (vh * 0.5).max(1.)).clamp(0., 1.)
}

/// A glass button of the viewer. It occludes, otherwise a click on it also reaches the photo layer underneath and
/// toggles the controls (info pill, share, trash).
pub(crate) fn glass_button(id: &'static str, w: f32, k: f32) -> Stateful<Div> {
    glass_div_with(VIEWER_TINT, 24., k, true).id(id).occlude().h(px(48.)).w(px(w)).flex().items_center().justify_center()
}

/// How visible the viewer's controls are (1 = full) while the photo is dragged down at scale `ds`: gone shortly
/// after the point where letting go closes the viewer, and back again as the photo springs back.
fn chrome_alpha(ds: f32) -> f32 {
    (1. - (1. - ds) / 0.2).clamp(0., 1.)
}

/// The largest box of aspect ratio `ar` (w/h) centered in `(x, y, w, h)`.
fn fit(rect: (f32, f32, f32, f32), ar: f32) -> (f32, f32, f32, f32) {
    let (x, y, w, h) = rect;
    let (fw, fh) = if w / h > ar { (h * ar, h) } else { (w, w / ar) };
    (x + (w - fw) / 2., y + (h - fh) / 2., fw, fh)
}

fn xy(p: Point<Pixels>) -> (f32, f32) {
    (p.x.into(), p.y.into())
}

impl Viewer {
    /// Opacity factor of the controls from the tap toggle (1 = shown, 0 = hidden), fading over `CHROME_MS`.
    fn chrome_vis(&self, now: f64) -> f32 {
        let t = ((now - self.chrome_at) / CHROME_MS).clamp(0., 1.) as f32;
        if self.chrome_hidden { 1. - t } else { t.max(if self.chrome_at == 0. { 1. } else { 0. }) }
    }

    fn reset_view(&mut self) {
        self.zoom = 1.;
        self.pan = (0., 0.);
        self.dx = 0.;
        self.dy = 0.;
        self.ds = 1.;
        self.video = None;
        self.details = None;
        self.menu.reset();
        self.info_scroll.set_offset(point(px(0.), px(0.)));
    }

    fn clamp_pan(&mut self) {
        let (vw, vh) = self.viewport.get();
        let (mx, my) = ((vw * self.zoom - vw).max(0.) / 2., (vh * self.zoom - vh).max(0.) / 2.);
        self.pan = (self.pan.0.clamp(-mx, mx), self.pan.1.clamp(-my, my));
    }

    /// Zooms to `target`, keeping the image point under `around` (screen px) where it is.
    fn zoom_to(&mut self, target: f32, around: (f32, f32)) {
        let (vw, vh) = self.viewport.get();
        let target = target.clamp(1., MAX_ZOOM);
        let r = target / self.zoom;
        let (px_, py_) = (around.0 - vw / 2., around.1 - vh / 2.);
        self.pan = (r * self.pan.0 + (1. - r) * px_, r * self.pan.1 + (1. - r) * py_);
        self.zoom = if target < 1.02 { 1. } else { target };
        if self.zoom == 1. {
            self.pan = (0., 0.);
        }
        self.clamp_pan();
    }
}

impl Immagine {
    /// Id of the photo the viewer currently shows, so its grid tile can be drawn empty while the photo is on top.
    pub(super) fn viewer_item(&self) -> Option<TimelineItem> {
        self.viewer.as_ref().filter(|v| !v.closing).and_then(|v| v.items.get(v.index)).cloned()
    }

    pub(super) fn viewer_item_id(&self) -> Option<String> {
        self.viewer.as_ref().and_then(|v| v.items.get(v.index)).map(|i| i.id.clone())
    }

    /// Opens the viewer on `items[index]`, zooming out of the grid tile with that index if it was painted.
    pub(super) fn open_viewer(&mut self, items: Rc<Vec<TimelineItem>>, index: usize, cx: &mut Context<Self>) {
        let from = self.tile_bounds.borrow().get(&index).map(|t| t.now());
        self.viewer_serial += 1;
        self.viewer = Some(Viewer {
            items,
            index,
            from,
            closing: false,
            into_tile: false,
            serial: self.viewer_serial,
            start: None,
            last: (0., 0.),
            moved: 0.,
            dx: 0.,
            dy: 0.,
            ds: 1.,
            anchor: (0., 0.),
            mode: DragMode::None,
            vel_y: 0.,
            release: None,
            snap: None,
            chrome0: 1.,
            chrome_hidden: false,
            chrome_at: 0.,
            close_at: 0.,
            stop_momentum: Cell::new(false),
            zoom: 1.,
            pan: (0., 0.),
            pinching: false,
            viewport: Cell::new((0., 0.)),
            info: false,
            details: None,
            video: None,
            scrub_track: Rc::new(Cell::new((0., 0., 1., 0.))),
            scrub_frac: 0.,
            trash: Morph::default(),
            menu: Morph::default(),
            menu_archived: false,
            info_rect: Rc::new(Cell::new(None)),
            info_scroll: ScrollHandle::new(),
            touch_in_panel: false,
            editor: None,
        });
        self.ensure_gestures(cx);
        cx.notify();
    }

    pub(super) fn close_viewer(&mut self, cx: &mut Context<Self>) {
        let Some(v) = self.viewer.as_mut() else { return };
        if v.closing {
            return;
        }
        v.closing = true;
        let vh = v.viewport.get().1;
        v.into_tile = self.tile_bounds.borrow().get(&v.index).map(|t| t.now()).is_some_and(|b| f32::from(b.origin.y) < vh && f32::from(b.origin.y + b.size.height) > 0.);
        v.close_at = now_ms();
        v.start = None;
        v.video = None;
        v.zoom = 1.;
        v.pan = (0., 0.);
        self.viewer_serial += 1;
        v.serial = self.viewer_serial;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(CLOSE_MS + 60)).await;
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
        let (x, y) = xy(at);
        if self.scrub_hit(x, y) {
            return self.scrub_down(x, cx);
        }
        if self.viewer.as_ref().is_some_and(|v| v.editor.is_some()) {
            return self.editor_down(xy(at), cx);
        }
        if let Some(v) = self.viewer.as_mut().filter(|v| !v.closing && !v.pinching && !v.trash.active() && !v.menu.active()) {
            v.start = Some(xy(at));
            v.anchor = xy(at);
            v.last = xy(at);
            v.moved = 0.;
            v.dx = 0.;
            v.dy = 0.;
            v.ds = 1.;
            v.vel_y = 0.;
            v.mode = DragMode::Undecided;
            v.snap = None;
            cx.notify();
        }
    }

    fn drag_move(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        if self.scrub_move(xy(at).0, cx) {
            return;
        }
        if self.viewer.as_ref().is_some_and(|v| v.editor.is_some()) {
            return self.editor_move(xy(at), cx);
        }
        let Some(v) = self.viewer.as_mut().filter(|v| !v.pinching) else { return };
        let Some((sx, sy)) = v.start else { return };
        let (x, y) = xy(at);
        v.moved += (x - v.last.0).abs() + (y - v.last.1).abs();
        v.vel_y = y - v.last.1;
        if v.zoom > 1. {
            v.pan = (v.pan.0 + x - v.last.0, v.pan.1 + y - v.last.1);
            v.clamp_pan();
        } else {
            let (ddx, ddy) = (x - sx, y - sy);
            if v.mode == DragMode::Undecided && (ddx.abs() > TAP_SLOP || ddy.abs() > TAP_SLOP) {
                v.mode = if ddy > ddx.abs() { DragMode::Dismiss } else { DragMode::Page };
            }
            match v.mode {
                DragMode::Page => {
                    v.dx = ddx;
                    v.dy = 0.;
                }
                DragMode::Dismiss => {
                    v.dx = ddx;
                    v.dy = ddy;
                    v.ds = dismiss_scale(ddy, v.viewport.get().1);
                }
                _ => {}
            }
        }
        v.last = (x, y);
        cx.notify();
    }

    fn drag_end(&mut self, width: f32, cx: &mut Context<Self>) {
        if self.scrub_up(cx) {
            return;
        }
        if let Some(ed) = self.viewer.as_mut().and_then(|v| v.editor.as_mut()) {
            ed.grab = None;
            cx.notify();
            return;
        }
        let Some(v) = self.viewer.as_mut() else { return };
        if v.start.take().is_none() {
            return;
        }
        let mode = std::mem::replace(&mut v.mode, DragMode::None);
        v.stop_momentum.set(true);
        if v.zoom > 1. {
            cx.notify();
            return;
        }
        let (dx, dy, ds) = (v.dx, v.dy, v.ds);
        let count = v.items.len();
        match mode {
            DragMode::Dismiss => {
                if ds < CLOSE_SCALE || (dy > 40. && v.vel_y > FLICK_SPEED) {
                    let (vw, vh) = v.viewport.get();
                    let (ax, ay) = v.anchor;
                    v.chrome0 = chrome_alpha(ds) * v.chrome_vis(now_ms());
                    v.release = Some(Release {
                        rect: (ax - ax * ds + dx, ay - ay * ds + dy, vw * ds, vh * ds),
                        fade: (1. - dy / (vh * 0.5)).clamp(0.3, 1.),
                    });
                    v.dx = 0.;
                    v.dy = 0.;
                    v.ds = 1.;
                    self.close_viewer(cx);
                    return;
                }
                self.snap_back(dx, dy, ds, cx);
            }
            DragMode::Page => {
                let mut start_dx = dx;
                if dx.abs() > width * PAGE_FRACTION {
                    if dx < 0. && v.index + 1 < count {
                        v.index += 1;
                        v.reset_view();
                        start_dx = dx + width + PAGE_GAP;
                    } else if dx > 0. && v.index > 0 {
                        v.index -= 1;
                        v.reset_view();
                        start_dx = dx - width - PAGE_GAP;
                    }
                }
                self.snap_back(start_dx, 0., 1., cx);
                self.refresh_details(cx);
            }
            _ => {
                v.dx = 0.;
                v.dy = 0.;
                v.ds = 1.;
            }
        }
        cx.notify();
    }

    /// Eases the drag offset and scale back to rest (or to the new page's place). The motion is computed from
    /// the clock in `viewer_overlay`, which asks for a frame until it is over.
    fn snap_back(&mut self, dx0: f32, dy0: f32, ds0: f32, cx: &mut Context<Self>) {
        let Some(v) = self.viewer.as_mut() else { return };
        v.dx = 0.;
        v.dy = 0.;
        v.ds = 1.;
        v.snap = Some(Snap { start: now_ms(), dx0, dy0, ds0 });
        cx.notify();
    }

    /// Double-tap toggles between fit and a closer look at the tapped point.
    fn double_tap(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(v) = self.viewer.as_mut().filter(|v| !v.closing) else { return };
        if v.moved > TAP_SLOP {
            return;
        }
        if v.zoom > 1. {
            v.zoom_to(1., xy(at));
        } else {
            v.zoom_to(DOUBLE_TAP_ZOOM, xy(at));
        }
        cx.notify();
    }

    fn wheel(&mut self, e: &ScrollWheelEvent, cx: &mut Context<Self>) {
        if matches!(e.source, ScrollSource::Touch) {
            return; // single-finger drags come from the raw pointer listener (see `pinch`)
        }
        let Some(v) = self.viewer.as_mut().filter(|v| !v.closing) else { return };
        let delta = e.delta.pixel_delta(px(16.));
        let (dx, dy): (f32, f32) = (delta.x.into(), delta.y.into());
        if e.modifiers.control {
            // Trackpad pinch arrives as ctrl+wheel.
            let target = v.zoom * (-dy * 0.01).exp();
            v.zoom_to(target, xy(e.position));
        } else if v.zoom > 1. {
            v.pan = (v.pan.0 + dx, v.pan.1 + dy);
            v.clamp_pan();
        } else {
            return;
        }
        cx.notify();
    }

    /// Installs the raw touch listener once; the viewer and the timeline grid both read it.
    pub(super) fn ensure_gestures(&mut self, cx: &mut Context<Self>) {
        if self.pinch.is_none() {
            let weak = cx.weak_entity();
            let async_cx = cx.to_async();
            self.pinch = H::install_gestures(move |event| {
                let mut app = async_cx.clone();
                weak.update(&mut app, |this, cx| this.pinch(event, cx)).ok();
            });
        }
    }

    fn pinch(&mut self, event: Pinch, cx: &mut Context<Self>) {
        if self.picker.sheet.visible() {
            return self.picker_pinch(event, cx);
        }
        if self.settings.sheet.visible() {
            return self.settings_pinch(event, cx);
        }
        if self.jobs.sheet.visible() {
            return self.jobs_pinch(event, cx);
        }
        if self.viewer.is_none() {
            return self.grid_pinch(event, cx);
        }
        // Raw touches have no hit test: a finger that went down on the info panel (it scrolls by itself) or while the
        // menu is open must not page, dismiss or zoom the photo behind.
        if let Some(v) = self.viewer.as_mut() {
            if let Pinch::Down { x, y } = event {
                v.touch_in_panel = !v.closing && v.info_rect.get().is_some_and(|(l, t, w, h)| x >= l && x <= l + w && y >= t && y <= t + h);
            }
            if v.touch_in_panel || (v.menu.active() && v.start.is_none()) {
                if matches!(event, Pinch::Up { .. }) {
                    v.touch_in_panel = false;
                }
                return;
            }
        }
        let width = self.viewer.as_ref().map(|v| v.viewport.get().0).unwrap_or(0.);
        match event {
            Pinch::Down { x, y } => return self.drag_start(point(px(x), px(y)), cx),
            Pinch::Drag { x, y } => return self.drag_move(point(px(x), px(y)), cx),
            Pinch::Up { x, y } => {
                if x >= 0. {
                    self.drag_move(point(px(x), px(y)), cx);
                }
                return self.drag_end(width, cx);
            }
            _ => {}
        }
        let Some(v) = self.viewer.as_mut().filter(|v| !v.closing) else { return };
        match event {
            Pinch::Begin => {
                v.pinching = true;
                v.start = None;
                v.mode = DragMode::None;
                v.snap = None;
                v.dx = 0.;
                v.dy = 0.;
                v.ds = 1.;
            }
            Pinch::Move { ratio, x, y } => v.zoom_to(v.zoom * ratio, (x, y)),
            Pinch::End => v.pinching = false,
            _ => {}
        }
        cx.notify();
    }

    /// Favorite on/off: applied locally at once, sent to the server by the outbox on the next sync.
    fn toggle_favorite(&mut self, cx: &mut Context<Self>) {
        let Some(v) = self.viewer.as_mut().filter(|v| !v.closing) else { return };
        let Some(item) = v.items.get(v.index) else { return };
        let (id, favorite) = (item.id.clone(), !item.is_favorite);
        let Some(engine) = self.engine.as_mut() else {
            self.status = "Bitte kurz warten, es wird synchronisiert.".into();
            cx.notify();
            return;
        };
        let op = if favorite { AlbumOp::favorite(vec![id.clone()]) } else { AlbumOp::unfavorite(vec![id.clone()]) };
        engine.enqueue(&[op]);
        for list in [&mut v.items, &mut self.items] {
            if let Some(item) = Rc::make_mut(list).iter_mut().find(|i| i.id == id) {
                item.is_favorite = favorite;
            }
        }
        self.sync(cx);
        cx.notify();
    }

    /// Filmstrip tap: shows photo `j`.
    fn jump_to(&mut self, j: usize, cx: &mut Context<Self>) {
        let Some(v) = self.viewer.as_mut().filter(|v| !v.closing && j < v.items.len()) else { return };
        v.index = j;
        v.reset_view();
        self.refresh_details(cx);
        cx.notify();
    }

    // MARK: - Delete

    fn open_trash_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(v) = self.viewer.as_mut().filter(|v| !v.closing && !v.trash.active() && !v.menu.active() && v.editor.is_none()) else { return };
        v.trash.open(now_ms());
        cx.notify();
    }

    fn close_trash_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(v) = self.viewer.as_mut() else { return };
        if !v.trash.close(now_ms()) {
            return;
        }
        cx.notify();
        Morph::finish_after(cx, |t: &mut Self| t.viewer.as_mut().map(|v| &mut v.trash));
    }

    /// "Foto löschen": moves the shown photo to the server trash (applied locally at once, sent by the outbox).
    fn trash_current(&mut self, cx: &mut Context<Self>) {
        let Some(v) = self.viewer.as_mut().filter(|v| !v.closing) else { return };
        v.trash.reset();
        let Some(id) = v.items.get(v.index).map(|i| i.id.clone()) else { return };
        let Some(engine) = self.engine.as_mut() else {
            self.status = "Bitte kurz warten, es wird synchronisiert.".into();
            cx.notify();
            return;
        };
        engine.enqueue(&[AlbumOp::trash(vec![id.clone()])]);
        Rc::make_mut(&mut self.items).retain(|i| i.id != id);
        Rc::make_mut(&mut v.items).retain(|i| i.id != id);
        if v.items.is_empty() {
            self.sync(cx);
            self.close_viewer(cx);
            return;
        }
        v.index = v.index.min(v.items.len() - 1);
        v.reset_view();
        self.refresh_details(cx);
        self.sync(cx);
        cx.notify();
    }

    // MARK: - "..." menu

    fn open_more_menu(&mut self, cx: &mut Context<Self>) {
        let archived = self.viewer_item_id().is_some_and(|id| self.engine.as_ref().is_some_and(|e| e.store().is_archived(&id)));
        let Some(v) = self.viewer.as_mut().filter(|v| !v.closing && !v.trash.active() && v.editor.is_none()) else { return };
        if v.menu.open(now_ms()) {
            v.menu_archived = archived;
            cx.notify();
        }
    }

    fn close_more_menu(&mut self, cx: &mut Context<Self>) {
        let Some(v) = self.viewer.as_mut() else { return };
        if !v.menu.close(now_ms()) {
            return;
        }
        cx.notify();
        Morph::finish_after(cx, |t: &mut Self| t.viewer.as_mut().map(|v| &mut v.menu));
    }

    /// Removes the photo from the lists (timeline and viewer) and shows the next one; closes the viewer when it was
    /// the last. Then syncs, which sends the queued op.
    fn viewer_drop(&mut self, id: &str, cx: &mut Context<Self>) {
        Rc::make_mut(&mut self.items).retain(|i| i.id != id);
        let Some(v) = self.viewer.as_mut() else { return };
        Rc::make_mut(&mut v.items).retain(|i| i.id != id);
        if v.items.is_empty() {
            self.sync(cx);
            self.close_viewer(cx);
            return;
        }
        v.index = v.index.min(v.items.len() - 1);
        v.reset_view();
        self.refresh_details(cx);
        self.sync(cx);
        cx.notify();
    }

    /// "Wiederherstellen" (the shown photo is in the Papierkorb): takes it out of the server trash, no confirmation.
    /// Applied locally at once (timeline and special pages are rebuilt), sent by the outbox.
    fn restore_current(&mut self, cx: &mut Context<Self>) {
        let Some(v) = self.viewer.as_mut().filter(|v| !v.closing) else { return };
        let Some(id) = v.items.get(v.index).map(|i| i.id.clone()) else { return };
        let Some(engine) = self.engine.as_mut() else {
            self.status = "Bitte kurz warten, es wird synchronisiert.".into();
            cx.notify();
            return;
        };
        engine.enqueue(&[AlbumOp::restore(vec![id.clone()])]);
        {
            use crate::logic::store::Store;
            self.items = Rc::new(super::oldest_first(engine.store().fetch_timeline()));
            self.albums.refresh(engine.store());
        }
        Rc::make_mut(&mut v.items).retain(|i| i.id != id);
        if v.items.is_empty() {
            self.sync(cx);
            self.close_viewer(cx);
            return;
        }
        v.index = v.index.min(v.items.len() - 1);
        v.reset_view();
        self.refresh_details(cx);
        self.sync(cx);
        cx.notify();
    }

    /// "Archivieren" / "Aus Archiv holen": applied locally at once (an archived photo leaves the Mediathek and the
    /// viewer shows the next one, like after deleting), sent by the outbox.
    pub(super) fn archive_current(&mut self, cx: &mut Context<Self>) {
        let Some(v) = self.viewer.as_mut().filter(|v| !v.closing) else { return };
        v.menu.reset();
        let Some(id) = v.items.get(v.index).map(|i| i.id.clone()) else { return };
        let Some(engine) = self.engine.as_mut() else {
            self.status = "Bitte kurz warten, es wird synchronisiert.".into();
            cx.notify();
            return;
        };
        if engine.store().is_archived(&id) {
            engine.enqueue(&[AlbumOp::unarchive(vec![id])]);
            // The photo is back in the Mediathek; the viewer keeps showing it.
            self.items = Rc::new(super::oldest_first(engine.store().fetch_timeline()));
            self.albums.refresh(engine.store());
            self.sync(cx);
            cx.notify();
        } else {
            engine.enqueue(&[AlbumOp::archive(vec![id.clone()])]);
            self.albums.refresh(engine.store());
            self.viewer_drop(&id, cx);
        }
    }

    /// The menu rows: download (or delete the stored copy), archive, add to album. Tapping a row closes the menu.
    fn more_menu(&self, v: &Viewer, now: f64, vw: f32, top: f32, downloaded: bool, downloading: Option<f32>, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let open = v.menu.is_open();
        let white: Rgba = rgb(0xffffff).into();
        let row = |id: &'static str, icon: Canvas<()>, label: String| {
            div()
                .id(id)
                .h(px(MENU_ROW))
                .px(px(14.))
                .rounded(px(16.))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(12.))
                .text_size(px(17.))
                .text_color(white)
                .active(|s| s.opacity(0.6))
                .child(div().w(px(28.)).flex_none().flex().justify_center().child(icon))
                .child(label)
        };
        let (download_icon, download_label) = match (downloaded, downloading) {
            (true, _) => (super::icons::glyph(super::icons::Glyph::Downloaded, 24., white), "Heruntergeladen".to_string()),
            (false, Some(f)) => (super::icons::progress_ring(f, 24., white), format!("Lädt … {} %", (f * 100.).round() as u32)),
            _ => (super::icons::glyph(super::icons::Glyph::Download, 24., white), "Herunterladen".to_string()),
        };
        let mut download = row("viewer-menu-download", download_icon, download_label);
        let mut archive = row(
            "viewer-menu-archive",
            super::icons::glyph(super::icons::Glyph::Archive, 24., white),
            if v.menu_archived { "Aus Archiv holen" } else { "Archivieren" }.to_string(),
        );
        let mut album = row("viewer-menu-album", super::icons::glyph(super::icons::Glyph::AddToAlbum, 24., white), "Zu Album hinzufügen".to_string());
        if open {
            download = download.on_click(cx.listener(|this, _, _, cx| {
                this.close_more_menu(cx);
                this.download_current(cx);
            }));
            archive = archive.on_click(cx.listener(|this, _, _, cx| this.archive_current(cx)));
            album = album.on_click(cx.listener(|this, _, _, cx| {
                this.close_more_menu(cx);
                if let Some(id) = this.viewer_item_id() {
                    this.open_album_picker(vec![id], false, cx);
                }
            }));
        }
        let (cw, ch) = (MENU_WIDTH, 3. * MENU_ROW + 16.);
        let content = div().size_full().p(px(8.)).flex().flex_col().child(download).child(archive).child(album).into_any_element();
        // From the "..." button (44 pt, top right) to the menu box under the same corner.
        let spec = morph::Spec {
            id: "viewer-menu",
            anchor: (vw - 16. - 44., top + 2., 44., 44.),
            card: (vw - 16. - cw, top + 2., cw, ch),
            tint: DIALOG_TINT,
            anchor_radius: 22.,
            card_radius: 26.,
        };
        morph::overlay(v.menu, spec, now, window, content, |this: &mut Self, cx| this.close_more_menu(cx), cx)
    }

    // MARK: - Share

    fn share_current(&mut self, cx: &mut Context<Self>) {
        let (origin, token) = (self.origin.clone(), self.token.clone());
        let Some(item) = self.viewer.as_ref().filter(|v| !v.closing).and_then(|v| v.items.get(v.index)) else { return };
        let id = item.id.clone();
        let video = item.is_video();
        // A stored download (original, or the H.264 file of a video) is shared from disk, no request.
        let stored = self.downloads.ids.borrow().contains(&id);
        self.status = if video && !stored { "Lade Video zum Teilen …".into() } else { "Bereite zum Teilen vor …".into() };
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = async {
                let client = ImmichClient::new(&origin, H::transport()).map_err(|e| e.to_string())?;
                // Name and size come from the asset record; without it the file gets a generic name.
                let details = client.asset(&id, &token).await.ok();
                let local = if stored { H::blob_get(super::downloads::DOWNLOAD_DB, &id).await.ok().flatten().filter(|b| !b.is_empty()) } else { None };
                let bytes = match local {
                    Some(b) => b,
                    None if video => {
                        let too_big = || format!("Das Video ist größer als {} MB und lässt sich nicht teilen.", SHARE_MAX_BYTES / 1_000_000);
                        if details.as_ref().and_then(|a| a.exif_info.as_ref()).and_then(|e| e.file_size_in_byte).is_some_and(|n| n > SHARE_MAX_BYTES as i64) {
                            return Err(too_big());
                        }
                        let url = client.video_playback_url(&id);
                        // The probe tells the size of the (transcoded) file; a server that ignores Range sends it all.
                        let probe = H::fetch_range(&url, 0, 0).await?;
                        if !probe.partial {
                            probe.bytes
                        } else {
                            if probe.total.is_some_and(|n| n > SHARE_MAX_BYTES) {
                                return Err(too_big());
                            }
                            H::fetch_bytes(&url).await?
                        }
                    }
                    None => client.original(&id, &token).await.map_err(|e| e.to_string())?,
                };
                let (name, mime) = share_name(details.as_ref().and_then(|a| a.original_file_name.as_deref()), &bytes, video);
                // The sheet is about to open: the busy text goes now, not when it is dismissed.
                this.update(cx, |t, cx| {
                    t.status = String::new();
                    cx.notify();
                })
                .ok();
                H::share_file(&name, mime, bytes).await
            }
            .await;
            this.update(cx, |t, cx| {
                t.status = match result {
                    // Dismissing the share sheet is reported as an error too; nothing to tell the user then.
                    Err(e) if !e.contains("AbortError") => e,
                    _ => String::new(),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // MARK: - Crop editor

    fn open_editor(&mut self, cx: &mut Context<Self>) {
        let (origin, token) = (self.origin.clone(), self.token.clone());
        let Some(v) = self.viewer.as_mut().filter(|v| !v.closing && !v.trash.active() && !v.menu.active() && v.editor.is_none()) else { return };
        let Some(item) = v.items.get(v.index).filter(|i| i.is_image()) else { return };
        let (id, ratio) = (item.id.clone(), item.ratio.unwrap_or(1.));
        v.reset_view();
        v.editor = Some(Editor { rect: CRect::new(0., 0., 1., 1.), aspect: CropAspect::Freeform, grab: None, pixels: None, saving: false, area: Cell::new((0., 0., 0., 0.)) });
        cx.notify();
        cx.spawn(async move |this, cx| {
            let asset = async {
                let client = ImmichClient::new(&origin, H::transport()).ok()?;
                client.asset(&id, &token).await.ok()
            }
            .await;
            this.update(cx, |t, cx| {
                let Some(ed) = t.viewer.as_mut().and_then(|v| v.editor.as_mut()) else { return };
                if let Some((w, h)) = asset.and_then(|a| Some((a.width? as f64, a.height? as f64))).filter(|(w, h)| *w > 0. && *h > 0.) {
                    // The record may hold the sensor orientation; the displayed ratio decides which way round it is.
                    let swap = (w > h) != (ratio > 1.) && (ratio - 1.).abs() > 0.01;
                    ed.pixels = Some(if swap { (h, w) } else { (w, h) });
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn close_editor(&mut self, cx: &mut Context<Self>) {
        if let Some(v) = self.viewer.as_mut() {
            v.editor = None;
        }
        cx.notify();
    }

    fn editor_size(v: &Viewer) -> CSize {
        let ed = v.editor.as_ref();
        match ed.and_then(|e| e.pixels) {
            Some((w, h)) => CSize::new(w, h),
            None => CSize::new(v.items.get(v.index).and_then(|i| i.ratio).unwrap_or(1.) * 1000., 1000.),
        }
    }

    fn editor_down(&mut self, at: (f32, f32), cx: &mut Context<Self>) {
        let Some(ed) = self.viewer.as_mut().and_then(|v| v.editor.as_mut()) else { return };
        let (ix, iy, iw, ih) = ed.area.get();
        let (iw, ih) = (iw as f64, ih as f64);
        let r = ed.rect;
        let px_rect = CRect::new(r.x * iw, r.y * ih, r.width * iw, r.height * ih);
        let p = CPoint::new((at.0 - ix) as f64, (at.1 - iy) as f64);
        ed.grab = crop_geometry::handle_at(p, px_rect, HANDLE_TOLERANCE).map(|h| (h, r, at));
        cx.notify();
    }

    fn editor_move(&mut self, at: (f32, f32), cx: &mut Context<Self>) {
        let Some(v) = self.viewer.as_mut() else { return };
        let size = Self::editor_size(v);
        let Some(ed) = v.editor.as_mut() else { return };
        let Some((handle, start, from)) = ed.grab else { return };
        let (_, _, iw, ih) = ed.area.get();
        if iw < 1. || ih < 1. {
            return;
        }
        let delta = CSize::new(((at.0 - from.0) / iw) as f64, ((at.1 - from.1) / ih) as f64);
        let ratio = ed.aspect.pixel_ratio(size).map(|r| crop_geometry::normalized_ratio(r, size));
        ed.rect = crop_geometry::drag(start, handle, delta, ratio, MIN_CROP);
        cx.notify();
    }

    fn set_aspect(&mut self, aspect: CropAspect, cx: &mut Context<Self>) {
        let Some(v) = self.viewer.as_mut() else { return };
        let size = Self::editor_size(v);
        let Some(ed) = v.editor.as_mut() else { return };
        ed.aspect = aspect;
        if let Some(r) = aspect.pixel_ratio(size) {
            ed.rect = crop_geometry::fit(CRect::new(0., 0., 1., 1.), crop_geometry::normalized_ratio(r, size));
        }
        cx.notify();
    }

    /// "Fertig": sends the crop to the server, which re-renders the photo's thumbnails.
    fn save_editor(&mut self, cx: &mut Context<Self>) {
        let (origin, token) = (self.origin.clone(), self.token.clone());
        let Some(v) = self.viewer.as_mut() else { return };
        let Some(id) = v.items.get(v.index).map(|i| i.id.clone()) else { return };
        let Some(ed) = v.editor.as_mut().filter(|e| !e.saving) else { return };
        let Some((w, h)) = ed.pixels else { return };
        let r = ed.rect;
        let (x, y) = ((r.x * w).round() as i64, (r.y * h).round() as i64);
        let (cw, ch) = (((r.width * w).round() as i64).max(1), ((r.height * h).round() as i64).max(1));
        let edit = AssetEdit::Crop { x, y, width: cw.min(w as i64 - x).max(1), height: ch.min(h as i64 - y).max(1) };
        ed.saving = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = async {
                let client = ImmichClient::new(&origin, H::transport()).map_err(|e| e.to_string())?;
                client.edit_asset(&id, &[edit], &token).await.map_err(|e| e.to_string())
            }
            .await;
            this.update(cx, |t, cx| {
                match result {
                    Ok(()) => {
                        t.thumbs.borrow_mut().forget(&id);
                        if let Some(v) = t.viewer.as_mut() {
                            v.editor = None;
                            v.details = None;
                        }
                    }
                    Err(e) => {
                        t.status = e;
                        if let Some(ed) = t.viewer.as_mut().and_then(|v| v.editor.as_mut()) {
                            ed.saving = false;
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Tap on the photo: hide or show the surrounding controls.
    fn toggle_chrome(&mut self, cx: &mut Context<Self>) {
        let Some(v) = self.viewer.as_mut().filter(|v| !v.closing) else { return };
        if v.moved > TAP_SLOP {
            return;
        }
        let now = now_ms();
        let vis = v.chrome_vis(now);
        v.chrome_hidden = !v.chrome_hidden;
        // Continue from the current opacity when toggled mid-fade.
        let t = if v.chrome_hidden { vis } else { 1. - vis };
        v.chrome_at = now - t as f64 * CHROME_MS;
        cx.notify();
    }

    fn toggle_info(&mut self, cx: &mut Context<Self>) {
        let Some(v) = self.viewer.as_mut().filter(|v| !v.closing) else { return };
        v.info = !v.info;
        self.refresh_details(cx);
        cx.notify();
    }

    /// Loads the full asset record (EXIF) for the shown photo while the info panel is open.
    fn refresh_details(&mut self, cx: &mut Context<Self>) {
        let Some(v) = self.viewer.as_ref().filter(|v| v.info && v.details.is_none()) else { return };
        let Some(item) = v.items.get(v.index) else { return };
        let (id, origin, token) = (item.id.clone(), self.origin.clone(), self.token.clone());
        cx.spawn(async move |this, cx| {
            let asset = async {
                let client = ImmichClient::new(&origin, H::transport()).ok()?;
                client.asset(&id, &token).await.ok()
            }
            .await;
            this.update(cx, |t, cx| {
                if let Some(v) = t.viewer.as_mut() {
                    if v.items.get(v.index).is_some_and(|i| i.id == id) {
                        v.details = asset.map(Rc::new);
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    fn play_video(&mut self, cx: &mut Context<Self>) {
        let origin = self.origin.clone();
        let Some(v) = self.viewer.as_mut().filter(|v| !v.closing) else { return };
        let Some(item) = v.items.get(v.index).filter(|i| i.is_video()) else { return };
        let url = format!("{origin}/api/assets/{}/video/playback", item.id);
        let local = self.downloads.ids.borrow().contains(&item.id).then(|| item.id.clone());
        v.video = Some(Rc::new(RefCell::new(Player::open(url, local, cx))));
        cx.notify();
    }

    /// Whether a press at (`x`, `y`) lands on the visible scrubber (its track plus some slop around it).
    fn scrub_hit(&self, x: f32, y: f32) -> bool {
        let Some(v) = self.viewer.as_ref().filter(|v| !v.closing && v.chrome_vis(now_ms()) > 0.5) else { return false };
        let ready = v.video.as_ref().is_some_and(|p| p.borrow().status == Status::Ready);
        let (left, top, width, height) = v.scrub_track.get();
        ready && x >= left - 12. && x <= left + width + 12. && y >= top - 12. && y <= top + height + 12.
    }

    /// A finger went down on the scrubber at `x`.
    fn scrub_down(&mut self, x: f32, cx: &mut Context<Self>) {
        let Some(v) = self.viewer.as_mut() else { return };
        let Some(player) = v.video.clone() else { return };
        let (left, _, width, _) = v.scrub_track.get();
        v.scrub_frac = ((x - left) / width.max(1.)).clamp(0., 1.);
        let mut p = player.borrow_mut();
        p.scrub_begin();
        p.scrub_to(v.scrub_frac);
        cx.notify();
    }

    /// Whether the scrubber is being dragged; then the pointer belongs to it, not to the page swipe.
    fn scrub_move(&mut self, x: f32, cx: &mut Context<Self>) -> bool {
        let Some(v) = self.viewer.as_mut() else { return false };
        let Some(player) = v.video.clone().filter(|p| p.borrow().scrubbing()) else { return false };
        let (left, _, width, _) = v.scrub_track.get();
        v.scrub_frac = ((x - left) / width.max(1.)).clamp(0., 1.);
        player.borrow_mut().scrub_to(v.scrub_frac);
        cx.notify();
        true
    }

    fn scrub_up(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(v) = self.viewer.as_mut() else { return false };
        let Some(player) = v.video.clone().filter(|p| p.borrow().scrubbing()) else { return false };
        player.borrow_mut().scrub_end(v.scrub_frac);
        cx.notify();
        true
    }

    fn toggle_play(&mut self, cx: &mut Context<Self>) {
        if let Some(player) = self.viewer.as_ref().and_then(|v| v.video.clone()) {
            player.borrow_mut().toggle_play();
            cx.notify();
        }
    }

    /// Play/pause button, scrubber and times of the native player, in a glass pill above the bottom bar.
    fn video_controls(&self, player: &Rc<RefCell<Player>>, track: Rc<Cell<(f32, f32, f32, f32)>>, k: f32, bottom: f32, live: bool, cx: &mut Context<Self>) -> AnyElement {
        let white = Rgba { a: k, ..rgb(0xffffff).into() };
        let p = player.borrow();
        let pill = glass_div_with(VIEWER_TINT, 26., k, true).id("viewer-controls").occlude().h(px(52.)).px_4().flex().flex_row().items_center().gap_3().text_color(white).text_size(px(13.));
        let body = match &p.status {
            Status::Loading => pill.justify_center().child("Lädt …"),
            Status::Failed(message) => pill.justify_center().child(message.clone()),
            Status::Ready => {
                let fraction = (p.position_us as f32 / p.duration_us.max(1) as f32).clamp(0., 1.);
                let mut button = div().id("viewer-play-pause").w(px(32.)).h(px(40.)).flex_none().flex().items_center().justify_center().child(super::icons::glyph(
                    if p.is_playing() { super::icons::Glyph::Pause } else { super::icons::Glyph::Play },
                    24.,
                    white,
                ));
                if live {
                    button = button.on_click(cx.listener(|this, _, _, cx| this.toggle_play(cx)));
                }
                let bar = div().id("viewer-scrub").flex_1().h(px(40.)).relative().flex().items_center();
                let bar = bar
                    .child(canvas(move |bounds, _, _| track.set((bounds.origin.x.into(), bounds.origin.y.into(), bounds.size.width.into(), bounds.size.height.into())), |_, _, _, _| {}).absolute().top_0().left_0().size_full())
                    .child(div().w_full().h(px(4.)).rounded(px(2.)).bg(Rgba { a: 0.3 * k, ..rgb(0xffffff).into() }))
                    .child(div().absolute().left_0().h(px(4.)).w(relative(fraction)).rounded(px(2.)).bg(white))
                    .child(div().absolute().left(relative(fraction)).ml(px(-7.)).w(px(14.)).h(px(14.)).rounded(px(7.)).bg(white));
                pill.child(button)
                    .child(div().w(px(44.)).flex_none().child(if p.buffering { "Lädt …".to_string() } else { fmt_time(p.position_us) }))
                    .child(bar)
                    .child(div().w(px(40.)).flex_none().text_right().child(fmt_time(p.duration_us)))
            }
        };
        div().absolute().left(px(16.)).right(px(16.)).bottom(px(bottom + 112.)).child(body).into_any_element()
    }

    fn info_lines(asset: &Asset) -> Vec<(&'static str, String)> {
        let mut lines = Vec::new();
        let exif = asset.exif_info.clone().unwrap_or_default();
        if let Some(name) = InfoFormat::title(asset.original_file_name.as_deref()) {
            lines.push(("Datei", name));
        }
        let when = asset.local_date_time.as_deref().or(asset.file_created_at.as_deref()).unwrap_or("");
        let part = |r: std::ops::Range<usize>| when.get(r).and_then(|s| s.parse::<u32>().ok());
        if let (Some(y), Some(m), Some(d), Some(h), Some(min)) = (part(0..4), part(5..7), part(8..10), part(11..13), part(14..16)) {
            lines.push(("Aufgenommen", InfoFormat::date_line(y as i32, m, d, h, min)));
        }
        if let Some(camera) = InfoFormat::camera(exif.make.as_deref(), exif.model.as_deref()) {
            lines.push(("Kamera", camera));
        }
        if let Some(lens) = InfoFormat::lens(exif.lens_model.as_deref(), exif.model.as_deref(), exif.focal_length, exif.f_number) {
            lines.push(("Objektiv", lens));
        }
        if let Some(exposure) = InfoFormat::exposure(&exif) {
            lines.push(("Belichtung", exposure.join("   ")));
        }
        let (w, h) = (exif.exif_image_width.or(asset.width), exif.exif_image_height.or(asset.height));
        if let Some(size) = AssetInfoFormat::dimensions(w, h) {
            let size = match AssetInfoFormat::megapixels(w, h) {
                Some(mp) => format!("{size}  ·  {mp}"),
                None => size,
            };
            lines.push(("Größe", size));
        }
        if let Some(bytes) = AssetInfoFormat::file_size(exif.file_size_in_byte) {
            lines.push(("Dateigröße", bytes));
        }
        lines
    }

    /// The crop editor: the photo, the box with its handles, "Abbrechen" / "Fertig" and the aspect presets.
    fn editor_overlay(&self, ed: &Editor, item: &TimelineItem, vw: f32, vh: f32, cx: &mut Context<Self>) -> AnyElement {
        let top = self.top.max(12.);
        let bottom = self.bottom.max(10.);
        let area = (16., top + 64., vw - 32., (vh - bottom - 130. - top - 64.).max(100.));
        let ar = item.ratio.filter(|r| *r > 0.).map(|r| r as f32).unwrap_or(1.);
        let (ix, iy, iw, ih) = fit(area, ar);
        ed.area.set((ix, iy, iw, ih));
        let (id, origin) = (item.id.clone(), self.origin.clone());
        let image = thumbs::get(&self.thumbs, &format!("p:{id}"), || thumbs::thumb_url(&origin, &id, "preview", item.thumbhash.as_deref()), cx)
            .or_else(|| thumbs::peek(&self.thumbs, &format!("t:{id}")));
        let r = ed.rect;
        let (l, t, w, h) = (ix + r.x as f32 * iw, iy + r.y as f32 * ih, r.width as f32 * iw, r.height as f32 * ih);
        let rect = |x: f32, y: f32, w: f32, h: f32, color: u32| div().absolute().left(px(x)).top(px(y)).w(px(w.max(0.))).h(px(h.max(0.))).bg(rgba(color));
        let dim = 0x000000a8;
        let mut layer = div()
            .id("viewer-editor")
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(rgb(0x000000))
            .child(div().absolute().left(px(ix)).top(px(iy)).w(px(iw)).h(px(ih)).children(image.map(|i| img(i).size_full().object_fit(ObjectFit::Fill))))
            .child(rect(ix, iy, iw, t - iy, dim))
            .child(rect(ix, t + h, iw, iy + ih - (t + h), dim))
            .child(rect(ix, t, l - ix, h, dim))
            .child(rect(l + w, t, ix + iw - (l + w), h, dim))
            .child(div().absolute().left(px(l)).top(px(t)).w(px(w)).h(px(h)).border_1().border_color(rgba(0xffffffcc)));
        for f in [1. / 3., 2. / 3.] {
            layer = layer.child(rect(l + w * f, t, 1., h, 0xffffff55)).child(rect(l, t + h * f, w, 1., 0xffffff55));
        }
        // Corner marks: an L at each corner of the box, lying on the border.
        for (x, y, sx, sy) in [(l, t, 1., 1.), (l + w, t, -1., 1.), (l, t + h, 1., -1.), (l + w, t + h, -1., -1.)] {
            let (hx, vy) = (if sx > 0. { x - 1.5 } else { x - 20.5 }, if sy > 0. { y - 1.5 } else { y - 20.5 });
            layer = layer.child(rect(hx, y - 1.5, 22., 3., 0xffffffff)).child(rect(x - 1.5, vy, 3., 22., 0xffffffff));
        }
        let chip = |id: &'static str, label: &str, color: Rgba| {
            glass_div_with(VIEWER_TINT, 22., 1., true).id(id).h(px(44.)).px_5().flex().items_center().text_color(color).text_size(px(17.)).child(label.to_string())
        };
        let ready = ed.pixels.is_some() && !ed.saving;
        let cancel = chip("editor-cancel", "Abbrechen", rgb(0xffffff).into()).on_click(cx.listener(|this, _, _, cx| this.close_editor(cx)));
        let mut done = chip("editor-done", if ed.saving { "Sichere …" } else { "Fertig" }, rgb(0xffd60a).into()).opacity(if ready { 1. } else { 0.45 });
        if ready {
            done = done.on_click(cx.listener(|this, _, _, cx| this.save_editor(cx)));
        }
        layer = layer.child(
            div()
                .absolute()
                .top(px(top + 2.))
                .left(px(16.))
                .right(px(16.))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(cancel)
                .child(div().text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).text_color(rgb(0xffffff)).child("Zuschneiden"))
                .child(done),
        );
        let presets = CropAspect::ALL.iter().enumerate().map(|(i, a)| {
            let a = *a;
            let color: Rgba = if a == ed.aspect { rgb(0xffd60a).into() } else { rgb(0xffffff).into() };
            div()
                .id(("editor-aspect", i))
                .flex_none()
                .child(glass_div_with(VIEWER_TINT, 20., 1., true).h(px(40.)).px_4().flex().items_center().text_color(color).text_size(px(15.)).child(a.title()))
                .on_click(cx.listener(move |this, _, _, cx| this.set_aspect(a, cx)))
        });
        layer
            .child(
                div()
                    .id("editor-aspects")
                    .absolute()
                    .bottom(px(bottom + 8.))
                    .left_0()
                    .w_full()
                    .px(px(16.))
                    .flex()
                    .flex_row()
                    .gap(px(8.))
                    .overflow_x_scroll()
                    .children(presets),
            )
            .into_any_element()
    }

    pub(super) fn viewer_overlay(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let v = self.viewer.as_ref()?;
        let vs = window.viewport_size();
        let (vw, vh): (f32, f32) = (vs.width.into(), vs.height.into());
        v.viewport.set((vw, vh));
        let (index, closing, dx, dy, zoom, pan) = (v.index, v.closing, v.dx, v.dy, v.zoom, v.pan);
        if v.stop_momentum.replace(false) {
            window.cancel_touch_momentum();
        }
        let (mut ds, anchor, release) = (v.ds, v.anchor, v.release);
        let grid = {
            let g = self.grid_bounds.get();
            let (x, y, w, h): (f32, f32, f32, f32) = (g.origin.x.into(), g.origin.y.into(), g.size.width.into(), g.size.height.into());
            (w > 1. && h > 1.).then_some((x, y, w, h))
        };
        let (mut dx, mut dy) = (dx, dy);
        let now = now_ms();
        let snap = v.snap.filter(|s| now - s.start < SNAP_MS);
        if let Some(s) = snap {
            let t = ((now - s.start) / SNAP_MS) as f32;
            let e = 1. - (1. - t).powi(3);
            (dx, dy, ds) = (s.dx0 * (1. - e), s.dy0 * (1. - e), s.ds0 + (1. - s.ds0) * e);
            window.request_animation_frame();
        }
        let snapping = snap.is_some();
        let chrome_vis = v.chrome_vis(now);
        if now - v.chrome_at < CHROME_MS {
            window.request_animation_frame();
        }
        let mut k = chrome_alpha(ds) * chrome_vis;
        if closing {
            let t = ((now - v.close_at) / (CLOSE_MS as f64 * 0.5)).clamp(0., 1.) as f32;
            k = v.chrome0 * (1. - t);
            if now - v.close_at < CLOSE_MS as f64 {
                window.request_animation_frame();
            }
        }
        // Closing shrinks into the tile of the photo that is shown now (not the one the viewer was opened on), read
        // anew every frame so the photo follows the grid when it is scrolled meanwhile; when that tile was not on
        // screen as closing began the photo just fades out.
        let tile = if closing { self.tile_bounds.borrow().get(&index).filter(|_| v.into_tile).map(|t| t.now()) } else { v.from };
        let from = tile.map(|b| {
            let (x, y): (f32, f32) = (b.origin.x.into(), b.origin.y.into());
            let (w, h): (f32, f32) = (b.size.width.into(), b.size.height.into());
            (x, y, w, h)
        });
        let items = v.items.clone();
        let origin = self.origin.clone();
        let top = self.top.max(12.);
        let count = items.len();
        let current = items.get(index).cloned();

        // The native player advances once per frame while it plays; its current picture goes on top of the page.
        let (video_frame, controls) = match v.video.clone() {
            Some(player) => {
                if player.borrow_mut().tick(window) {
                    window.request_animation_frame();
                }
                let frame = player.borrow().frame();
                (frame, Some(self.video_controls(&player, v.scrub_track.clone(), k, self.bottom.max(10.), !closing, cx)))
            }
            None => (None, None),
        };

        // Decoded images for the visible page and its neighbours (loads are queued by the store).
        let store = self.thumbs.clone();
        store.borrow_mut().begin_frame();
        for evicted in store.borrow_mut().take_evicted() {
            let _ = window.drop_image(evicted);
        }
        type Layers = [Option<Arc<RenderImage>>; 3];
        let mut pictures: HashMap<usize, Layers> = HashMap::new();
        let near = if zoom > 1. { index..=index } else { index.saturating_sub(1)..=(index + 1).min(count.saturating_sub(1)) };
        for k in near {
            let Some(item) = items.get(k) else { continue };
            let url = |size: &str| thumbs::thumb_url(&origin, &item.id, size, item.thumbhash.as_deref());
            // While the photos slide into place only what is decoded already is used: a decode now would drop frames.
            let (tk, pk, fk) = (format!("t:{}", item.id), format!("p:{}", item.id), format!("f:{}", item.id));
            let thumb = if snapping { thumbs::peek(&store, &tk) } else { thumbs::get(&store, &tk, || url("thumbnail"), cx) };
            let preview = if snapping { thumbs::peek(&store, &pk) } else { thumbs::get(&store, &pk, || url("preview"), cx) };
            let full = if k == index && zoom > FULLSIZE_ZOOM && !snapping { thumbs::get(&store, &fk, || url("fullsize"), cx) } else { None };
            pictures.insert(k, [thumb, preview, full]);
        }
        let page = move |k: usize, cover: bool| -> Option<AnyElement> {
            let layers = pictures.get(&k)?;
            let mut page = div().size_full();
            for image in layers.iter().flatten() {
                page = page.child(img(image.clone()).absolute().size_full().object_fit(if cover { ObjectFit::Cover } else { ObjectFit::Contain }));
            }
            if let Some(frame) = video_frame.clone().filter(|_| k == index) {
                page = page.child(img(frame).absolute().size_full().object_fit(ObjectFit::Contain));
            }
            Some(page.into_any_element())
        };
        let aspect = current.as_ref().and_then(|i| i.ratio).filter(|r| *r > 0.).map(|r| r as f32).unwrap_or(vw / vh);
        let date = current.as_ref().map(|i| i.date_bucket.clone()).unwrap_or_default();
        let is_video = current.as_ref().is_some_and(|i| i.is_video());
        let favorite = current.as_ref().is_some_and(|i| i.is_favorite);
        let current_id = current.as_ref().map(|i| i.id.clone()).unwrap_or_default();
        let downloaded = self.downloads.ids.borrow().contains(&current_id);
        let downloading = self.downloads.busy.get(&current_id).copied();
        let playing = v.video.is_some();
        let bottom = self.bottom.max(10.);

        let chip = |id: &'static str, label: String, color: Rgba| {
            let color = Rgba { a: color.a * k, ..color };
            glass_div_with(VIEWER_TINT, 22., k, true)
                .id(id)
                .occlude()
                .h(px(44.))
                .px_5()
                .flex()
                .items_center()
                .text_color(color)
                .text_size(px(17.))
                .child(label)
        };
        // Round or pill-shaped dark glass button of the top and bottom bars.
        let button = |id: &'static str, w: f32| glass_button(id, w, k);
        let white: Rgba = Rgba { a: k, ..rgb(0xffffff).into() };

        // Filmstrip: neighbours around the shown photo, the shown one wider.
        // The strip follows the swipe: `pos` is the (fractional) photo index under the centre of the screen.
        let page_dx = if v.mode == DragMode::Dismiss { 0. } else { dx };
        let pos = (index as f32 - page_dx / (vw + PAGE_GAP)).clamp(0., count.saturating_sub(1) as f32);
        let strip: Vec<(usize, f32, f32, Option<Arc<RenderImage>>)> = if k > 0. && count > 0 {
            let lo = (pos.round() as usize).saturating_sub(10);
            let hi = (pos.round() as usize + 10).min(count - 1);
            let mut left = 0.;
            let mut tiles: Vec<(usize, f32, f32, Option<Arc<RenderImage>>)> = (lo..=hi)
                .filter_map(|j| {
                    let item = items.get(j)?;
                    let key = format!("t:{}", item.id);
                    let image = if snapping { thumbs::peek(&store, &key) } else { thumbs::get(&store, &key, || thumbs::thumb_url(&origin, &item.id, "thumbnail", item.thumbhash.as_deref()), cx) };
                    let w = lerp(22., 38., (1. - (j as f32 - pos).abs()).max(0.));
                    let tile = (j, left, w, image);
                    left += w + 2.;
                    Some(tile)
                })
                .collect();
            let center = |t: &(usize, f32, f32, Option<Arc<RenderImage>>)| t.1 + t.2 / 2.;
            let fl = (pos.floor() as usize).clamp(lo, hi);
            let a = tiles.iter().find(|t| t.0 == fl).map(center).unwrap_or(0.);
            let b = tiles.iter().find(|t| t.0 == (fl + 1).min(hi)).map(center).unwrap_or(a);
            let c = lerp(a, b, pos - fl as f32);
            for t in tiles.iter_mut() {
                t.1 += vw / 2. - c;
            }
            tiles
        } else {
            Vec::new()
        };
        let when = current.as_ref().and_then(|i| i.sort_date.clone()).unwrap_or_default();
        let time = when.get(11..16).unwrap_or("").to_string();
        let day = match (date.get(8..10), date.get(5..7), date.get(0..4)) {
            (Some(d), Some(m), Some(y)) => format!("{d}.{m}.{y}"),
            _ => String::new(),
        };

        let info_max = (vh - (bottom + 126.) - (top + 54.)).max(120.);
        if !(v.info && k > 0.) {
            v.info_rect.set(None);
        }
        let info_panel = (v.info && k > 0.).then(|| {
            let lines = match &v.details {
                Some(asset) => Self::info_lines(asset),
                None => vec![("", "Lädt …".to_string())],
            };
            let rect = v.info_rect.clone();
            // Lines scroll inside the panel, which never grows above the top bar. Values wrap (`min_w_0` lets the
            // flex column shrink below its text; the line wrapper breaks words longer than the column).
            let list = div()
                .id("viewer-info-scroll")
                .max_h(px(info_max - 32.))
                .overflow_y_scroll()
                .track_scroll(&v.info_scroll)
                .flex()
                .flex_col()
                .gap_2()
                .children(lines.into_iter().map(|(label, value)| {
                    div()
                        .flex()
                        .flex_row()
                        .gap_3()
                        .child(div().w(px(104.)).flex_none().text_size(px(13.)).text_color(rgba(0xffffffaa)).child(label))
                        .child(div().flex_1().min_w_0().text_size(px(15.)).child(value))
                }));
            glass_div_with(VIEWER_TINT, 24., k, true)
                .id("viewer-info-panel")
                .occlude()
                .absolute()
                .left(px(16.))
                .right(px(16.))
                .bottom(px(bottom + 126.))
                .max_h(px(info_max))
                .p_4()
                .text_color(rgb(0xffffff))
                .opacity(k)
                .child(
                    canvas(|_, _, _| (), move |b, _, _, _| {
                        let (x, y, w, h): (f32, f32, f32, f32) = (b.origin.x.into(), b.origin.y.into(), b.size.width.into(), b.size.height.into());
                        rect.set(Some((x, y, w, h)));
                    })
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                )
                .child(list)
        });

        let dialog = if closing {
            None
        } else {
            let (bw, bh) = DIALOG_SIZE;
            let open = v.trash.is_open();
            let noun = if is_video { "Video" } else { "Foto" };
            let mut confirm = div()
                .id("viewer-dialog-confirm")
                .h(px(54.))
                .rounded(px(27.))
                .bg(rgba(0xffffff1f))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(20.))
                .text_color(rgb(0xff5a52))
                .child(format!("{noun} löschen"));
            if open {
                confirm = confirm.on_click(cx.listener(|this, _, _, cx| this.trash_current(cx)));
            }
            let content = div()
                .size_full()
                .p(px(20.))
                .flex()
                .flex_col()
                .justify_between()
                .child(div().text_size(px(17.)).text_color(rgb(0xffffff)).child(format!("Dieses {noun} wird aus der Mediathek gelöscht. Du kannst noch 30 Tage in „Zuletzt gelöscht“ darauf zugreifen.")))
                .child(confirm)
                .into_any_element();
            // From the trash button (48 pt, bottom right) to the confirmation box.
            let spec = morph::Spec {
                id: "viewer-dialog",
                anchor: (vw - 24. - 48., vh - bottom - 48., 48., 48.),
                card: (vw - 16. - bw, vh - bottom - bh, bw, bh),
                tint: DIALOG_TINT,
                anchor_radius: 24.,
                card_radius: 34.,
            };
            morph::overlay(v.trash, spec, now, window, content, |this: &mut Self, cx| this.close_trash_dialog(cx), cx)
        };
        let menu = if closing { None } else { self.more_menu(v, now, vw, top, downloaded, downloading, window, cx) };
        let editor = match (v.editor.as_ref(), current.as_ref()) {
            (Some(ed), Some(item)) if !closing => Some(self.editor_overlay(ed, item, vw, vh, cx)),
            _ => None,
        };

        // While it shrinks away the viewer lets everything through, so the page below can be scrolled at once.
        let mut root = div().id("viewer-root").absolute().top_0().left_0().size_full();
        if !closing {
            root = root.occlude();
        }
        Some(
            root
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
                .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, _, cx| this.wheel(e, cx)))
                .on_click(cx.listener(|this, e: &ClickEvent, _, cx| {
                    if e.click_count() == 2 {
                        this.double_tap(e.position(), cx);
                    }
                }))
                .child(
                    div()
                        .id(("viewer", v.serial))
                        .on_click(cx.listener(|this, e: &ClickEvent, _, cx| {
                            // Click 2 undoes the toggle of click 1 of a double tap (which zooms instead).
                            if e.click_count() <= 2 {
                                this.toggle_chrome(cx);
                            }
                        }))
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .with_animation(("viewer-anim", v.serial), anim(if closing { CLOSE_MS } else { OPEN_MS }), move |el, d| {
                            let t = if closing { 1. - d } else { d };
                            // While dragging down the backdrop fades and the photo follows the finger.
                            let fade = match release {
                                Some(r) if closing => r.fade,
                                _ => (1. - dy / (vh * 0.5)).clamp(0.3, 1.),
                            };
                            let settled = t >= 1. && !closing;
                            // Opening and closing morph a box between the grid tile and the aspect-fitted photo, with the
                            // image covering the box: it is cropped like the tile at one end and exactly the photo at the
                            // other, so its aspect ratio never differs from the tile's at the end of the animation.
                            let target = match release {
                                Some(r) if closing => r.rect,
                                _ => (0., 0., vw, vh),
                            };
                            let (x0, y0, w, h, cover) = match from {
                                Some((fx, fy, fw, fh)) if !settled => {
                                    let (tx, ty, tw, th) = fit(target, aspect);
                                    (lerp(fx, tx, t), lerp(fy, ty, t), lerp(fw, tw, t), lerp(fh, th, t), true)
                                }
                                _ if !settled => (target.0, target.1, target.2, target.3, false),
                                _ => {
                                    let (w, h) = (vw * zoom * ds, vh * zoom * ds);
                                    if zoom == 1. {
                                        (anchor.0 - anchor.0 * ds + dx, anchor.1 - anchor.1 * ds + dy, w, h, false)
                                    } else {
                                        ((vw - w) / 2. + pan.0 + dx, (vh - h) / 2. + pan.1 + dy, w, h, false)
                                    }
                                }
                            };
                            let backdrop = t * fade;
                            let photo = |ox: f32, oy: f32| div().absolute().left(px(x0 - ox)).top(px(y0 - oy)).w(px(w)).h(px(h)).overflow_hidden().children(page(index, cover));
                            let mut el = el.child(div().absolute().size_full().bg(rgba(0x000000ff)).opacity(backdrop));
                            el = match grid {
                                // Shrinking into a tile that scrolls: the photo is clipped to the grid, so it never
                                // floats over the header or the tab bar.
                                // Only the top edge is clipped (the header): it moves from the screen's top to the grid's
                                // top once the photo is half way to its tile. Sides and bottom stay open so a wrong
                                // measurement can never cut the photo off.
                                Some((_, gy, _, _)) if closing => {
                                    let k = ((0.5 - t) / 0.5).clamp(0., 1.);
                                    let cy_ = lerp(0., gy, k);
                                    el.child(div().absolute().left_0().top(px(cy_)).w(px(vw)).h(px(vh - cy_)).overflow_hidden().child(photo(0., cy_)))
                                }
                                _ => el.child(photo(0., 0.)),
                            };
                            if settled && zoom == 1. && ds >= 0.999 && dy.abs() < 0.5 {
                                // Neighbours slide in beside the current page.
                                if index > 0 {
                                    el = el.child(div().absolute().left(px(dx - vw - PAGE_GAP)).top_0().w(px(vw)).h(px(vh)).children(page(index - 1, false)));
                                }
                                if index + 1 < count {
                                    el = el.child(div().absolute().left(px(dx + vw + PAGE_GAP)).top_0().w(px(vw)).h(px(vh)).children(page(index + 1, false)));
                                }
                            }
                            el
                        }),
                )
                .children(info_panel)
                .children((k > 0.).then(|| {
                    let live = !closing;
                    let mut fav = div().id("viewer-fav").w(px(52.)).h(px(48.)).flex().items_center().justify_center().child(heart(favorite, k));
                    let mut info = div().id("viewer-info").w(px(52.)).h(px(48.)).flex().items_center().justify_center().child(super::icons::glyph(super::icons::Glyph::Info, 26., if v.info { Rgba { a: k, ..rgb(BLUE).into() } } else { white }));
                    if live {
                        fav = fav.on_click(cx.listener(|this, _, _, cx| this.toggle_favorite(cx)));
                        info = info.on_click(cx.listener(|this, _, _, cx| this.toggle_info(cx)));
                    }
                    let mut sliders = div().id("viewer-edit").w(px(52.)).h(px(48.)).flex().items_center().justify_center().child(super::icons::glyph(super::icons::Glyph::Sliders, 26., white));
                    if live && !is_video {
                        sliders = sliders.on_click(cx.listener(|this, _, _, cx| this.open_editor(cx)));
                    }
                    let pill = button("viewer-pill", 164.).child(fav).child(info).child(sliders);
                    let mut share = button("viewer-share", 48.).child(super::icons::glyph(super::icons::Glyph::Share, 26., white));
                    // A photo in the Papierkorb is restored instead of deleted again.
                    let in_trash = v.items.get(v.index).is_some_and(|i| i.is_trashed);
                    let mut trash = button("viewer-trash", 48.).child(super::icons::glyph(if in_trash { super::icons::Glyph::Restore } else { super::icons::Glyph::Trash }, 26., white));
                    if live {
                        share = share.on_click(cx.listener(|this, _, _, cx| this.share_current(cx)));
                        trash = if in_trash {
                            trash.on_click(cx.listener(|this, _, _, cx| this.restore_current(cx)))
                        } else {
                            trash.on_click(cx.listener(|this, _, _, cx| this.open_trash_dialog(cx)))
                        };
                    }
                    // While the confirmation morphs out of the trash button, the button itself is gone.
                    let trash = if v.trash.active() { div().w(px(48.)).h(px(48.)).into_any_element() } else { trash.into_any_element() };
                    let bar = div()
                        .absolute()
                        .bottom(px(bottom))
                        .left(px(24.))
                        .right(px(24.))
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .child(share)
                        .child(pill)
                        .child(trash);
                    let filmstrip = div()
                        .absolute()
                        .bottom(px(bottom + 66.))
                        .left_0()
                        .w_full()
                        .h(px(34.))
                        .overflow_hidden()
                        .opacity(k)
                        .children(strip.into_iter().map(|(j, x, w, image)| {
                            let tile = div().id(("viewer-film", j)).absolute().left(px(x)).top_0().w(px(w)).h(px(34.)).rounded(px(3.)).overflow_hidden().bg(rgb(0x1c1c1e));
                            let tile = match image {
                                Some(image) => tile.child(img(image).size_full().object_fit(ObjectFit::Cover)),
                                None => tile,
                            };
                            if live && j != index {
                                tile.on_click(cx.listener(move |this, _, _, cx| this.jump_to(j, cx)))
                            } else {
                                tile
                            }
                        }));
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .child(filmstrip)
                        .child(bar)
                        .children(controls)
                        .children((is_video && !playing).then(|| {
                            let play = chip("viewer-play", "Abspielen".into(), rgb(0xffffff).into());
                            let play = if live { play.on_click(cx.listener(|this, _, _, cx| this.play_video(cx))) } else { play };
                            div().absolute().bottom(px(bottom + 112.)).w_full().flex().justify_center().child(play)
                        }))
                }))
                .children((k > 0.).then(|| {
                    let live = !closing;
                    let mut back = button("viewer-done", 44.).h(px(44.)).active(|s| s.opacity(0.6)).child(super::icons::chevron(white));
                    if live {
                        back = back.on_click(cx.listener(|this, _, _, cx| this.close_viewer(cx)));
                    }
                    let title = glass_div_with(VIEWER_TINT, 22., k, true)
                        .w(px(160.))
                        .h(px(44.))
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .text_color(white)
                        .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(day))
                        .child(div().text_size(px(14.)).child(time));
                    // The "..." button opens the menu (download, archive, album). While a download runs, its progress ring
                    // circles the dots. The button itself is gone while the menu morphs out of it.
                    let mut more = button("viewer-more", 44.).h(px(44.)).active(|s| s.opacity(0.6));
                    if let Some(fraction) = downloading {
                        more = more.child(div().absolute().size_full().flex().items_center().justify_center().child(super::icons::progress_ring(fraction, 38., white)));
                    }
                    more = more.child(super::icons::glyph(super::icons::Glyph::More, 24., white));
                    if live {
                        more = more.on_click(cx.listener(|this, _, _, cx| this.open_more_menu(cx)));
                    }
                    let download = if v.menu.active() { div().w(px(44.)).h(px(44.)).into_any_element() } else { more.into_any_element() };
                    div()
                        .absolute()
                        .top(px(top + 2.))
                        .left(px(16.))
                        .right(px(16.))
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .child(back)
                        .child(title)
                        .child(download)
                }))
                .children(dialog)
                .children(menu)
                .children(editor)
                .into_any_element(),
        )
    }
}

/// A heart, filled red when `filled`, otherwise a white outline (the font has no heart glyph).
fn heart(filled: bool, k: f32) -> Canvas<()> {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let (x, y): (f32, f32) = (bounds.origin.x.into(), bounds.origin.y.into());
            let scale = f32::from(bounds.size.width) / 24.;
            let at = |px_: f32, py_: f32| point(px(x + px_ * scale), px(y + py_ * scale));
            let mut b = if filled { PathBuilder::fill() } else { PathBuilder::stroke(px(1.8)) };
            b.move_to(at(12., 21.5));
            b.curve_to(at(2.2, 8.2), at(2., 14.5));
            b.curve_to(at(7.2, 3.), at(2.5, 3.));
            b.curve_to(at(12., 6.8), at(10.8, 3.));
            b.curve_to(at(16.8, 3.), at(13.2, 3.));
            b.curve_to(at(21.8, 8.2), at(21.5, 3.));
            b.curve_to(at(12., 21.5), at(22., 14.5));
            b.close();
            if let Ok(path) = b.build() {
                let color: Rgba = if filled { rgb(RED).into() } else { rgb(0xffffff).into() };
                window.paint_path(path, Rgba { a: k, ..color });
            }
        },
    )
    .size(px(22.))
}

#[cfg(test)]
mod tests {
    #[::core::prelude::v1::test]
    fn share_names_follow_the_bytes_sent() {
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let mut heic = vec![0, 0, 0, 24];
        heic.extend_from_slice(b"ftypheic0000");
        assert_eq!(super::share_name(Some("IMG_1.HEIC"), &jpeg, false), ("IMG_1.jpg".to_string(), "image/jpeg"));
        assert_eq!(super::share_name(Some("a/b.jpg"), &heic, false), ("b.heic".to_string(), "image/heic"));
        assert_eq!(super::share_name(Some("clip.MOV"), b"\0\0\0\x18ftypisom0000", true), ("clip.mp4".to_string(), "video/mp4"));
        assert_eq!(super::share_name(None, b"", true), ("video.mp4".to_string(), "video/mp4"));
    }

    use super::*;
    use ::gpui::TestAppContext;
    use std::cell::Cell;

    struct Probe {
        photo: Rc<Cell<u32>>,
        button: Rc<Cell<u32>>,
    }

    impl Render for Probe {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let (photo, button) = (self.photo.clone(), self.button.clone());
            div()
                .size_full()
                .child(div().id("photo").absolute().size_full().on_click(move |_, _, _| photo.set(photo.get() + 1)))
                .child(
                    glass_button("probe-button", 100., 1.)
                        .absolute()
                        .top(px(10.))
                        .left(px(10.))
                        .on_click(move |_, _, _| button.set(button.get() + 1)),
                )
        }
    }

    // `use super::*` pulls in gpui's `test` attribute macro; name the std one.
    #[::core::prelude::v1::test]
    fn button_click_does_not_reach_the_photo() {
        let mut cx = TestAppContext::single();
        let (photo, button) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
        let (p, b) = (photo.clone(), button.clone());
        let (_, cx) = cx.add_window_view(|_, _| Probe { photo: p, button: b });
        cx.simulate_click(point(px(50.), px(30.)), Modifiers::none());
        assert_eq!((button.get(), photo.get()), (1, 0), "button action only");
        cx.simulate_click(point(px(300.), px(300.)), Modifiers::none());
        assert_eq!((button.get(), photo.get()), (1, 1), "photo tap still works");
    }

    fn photos(n: usize) -> Vec<TimelineItem> {
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

    /// The "..." button opens the menu without toggling the controls; a row runs its action and closes the menu.
    #[::core::prelude::v1::test]
    fn more_button_opens_the_menu_and_a_row_runs_its_action() {
        let mut cx = TestAppContext::single();
        let (view, cx) = cx.add_window_view(|w, cx| Immagine::new(0., 0., false, false, w, cx));
        view.update(cx, |app, cx| {
            app.screen = super::super::Screen::Timeline;
            app.open_viewer(Rc::new(photos(3)), 1, cx);
            // Settled: no open animation, so the chrome is fully visible.
            app.viewer.as_mut().unwrap().chrome_at = 0.;
        });
        cx.run_until_parked();
        let vw: f32 = cx.update(|w, _| w.viewport_size().width.into());
        // The button: 44 pt, 16 pt from the right edge, 2 pt below the top inset (12 pt minimum).
        cx.simulate_click(point(px(vw - 16. - 22.), px(12. + 2. + 22.)), Modifiers::none());
        view.update(cx, |app, _| {
            let v = app.viewer.as_ref().unwrap();
            assert!(v.menu.is_open(), "the tap opened the menu");
            assert!(!v.chrome_hidden, "and did not toggle the controls");
        });
        // Let the morph run to its end, then tap "Zu Album hinzufügen" (third row).
        view.update(cx, |app, cx| {
            let v = app.viewer.as_mut().unwrap();
            v.menu.reset();
            v.menu.open(now_ms() - 1000.);
            cx.notify();
        });
        cx.run_until_parked();
        let row_y = 12. + 2. + 8. + 2. * MENU_ROW + MENU_ROW / 2.;
        cx.simulate_click(point(px(vw - 16. - MENU_WIDTH / 2.), px(row_y)), Modifiers::none());
        view.update(cx, |app, _| {
            assert!(app.picker.sheet.is_open(), "the album row opened the picker");
            let v = app.viewer.as_ref().unwrap();
            assert!(!v.menu.is_open() && !v.chrome_hidden, "menu closes, controls untouched");
        });
    }
}

/// Largest video that is loaded into memory to be shared (web memory is 1 GiB and the bytes are copied on the way).
const SHARE_MAX_BYTES: u64 = 200_000_000;

/// File name and MIME type for sharing `bytes`: the asset's own name where known, with the extension of what is
/// really sent (an edited/preview JPEG of a `.HEIC` is `.jpg`, a transcoded video `.mp4`).
fn share_name(original: Option<&str>, bytes: &[u8], video: bool) -> (String, &'static str) {
    let (ext, mime) = sniff(bytes).unwrap_or(if video { ("mp4", "video/mp4") } else { ("jpg", "image/jpeg") });
    let stem = original
        .map(|n| n.rsplit(['/', '\\']).next().unwrap_or(n))
        .map(|n| n.rsplit_once('.').map_or(n, |(stem, _)| stem))
        .filter(|s| !s.is_empty())
        .unwrap_or(if video { "video" } else { "foto" });
    (format!("{stem}.{ext}"), mime)
}

fn sniff(b: &[u8]) -> Option<(&'static str, &'static str)> {
    if b.starts_with(&[0xFF, 0xD8]) {
        Some(("jpg", "image/jpeg"))
    } else if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some(("png", "image/png"))
    } else if b.starts_with(b"GIF8") {
        Some(("gif", "image/gif"))
    } else if b.len() > 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some(("webp", "image/webp"))
    } else if b.len() > 12 && &b[4..8] == b"ftyp" {
        Some(match &b[8..12] {
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"heim" | b"heis" | b"mif1" | b"msf1" => ("heic", "image/heic"),
            b"avif" => ("avif", "image/avif"),
            b"qt  " => ("mov", "video/quicktime"),
            _ => ("mp4", "video/mp4"),
        })
    } else {
        None
    }
}
