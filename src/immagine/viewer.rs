//! Full-screen photo viewer: zoom-in from the tapped tile, swipe left/right between photos, swipe down or
//! "Fertig" to close; double-tap, trackpad pinch (ctrl+wheel) and two-finger pinch zoom, drag to pan;
//! favorite toggle, info panel (EXIF), video playback. Pages use the grid thumbnail (already loaded) as a
//! stand-in until the larger preview arrives, and the full-size rendition once zoomed in.

use super::{thumbs, Immagine};
use crate::components::glass::glass_div;
use crate::logic::client::ImmichClient;
use crate::logic::models::album_op::AlbumOp;
use crate::logic::models::asset::Asset;
use crate::logic::models::asset_details::InfoFormat;
use crate::logic::models::asset_info::AssetInfoFormat;
use crate::logic::models::timeline::TimelineItem;
use crate::theme::{BLUE, GLASS_TINT_DARK, RED};
use crate::util::{anim, lerp};
use crate::web_adapters::fetch::FetchTransport;
use crate::web_adapters::pinch::{self, Pinch};
use crate::web_adapters::video::{self, VideoHandle};
use gpui_kit::*;
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// A swipe farther than this fraction of the screen width turns the page.
const PAGE_FRACTION: f32 = 0.2;
/// A downward swipe farther than this many px closes the viewer.
const CLOSE_DISTANCE: f32 = 120.;
const MAX_ZOOM: f32 = 6.;
const DOUBLE_TAP_ZOOM: f32 = 2.5;
/// Above this zoom the full-size rendition is loaded on top of the preview.
const FULLSIZE_ZOOM: f32 = 1.2;
/// Movement (px) that turns a press into a drag, so a tap with a little jitter still counts as a tap.
const TAP_SLOP: f32 = 8.;

pub(super) struct Viewer {
    items: Rc<Vec<TimelineItem>>,
    index: usize,
    from: Option<Bounds<Pixels>>,
    closing: bool,
    serial: u32,
    /// Pointer position where the current drag started, the last one seen, and the distance moved.
    start: Option<(f32, f32)>,
    last: (f32, f32),
    moved: f32,
    dx: f32,
    dy: f32,
    zoom: f32,
    pan: (f32, f32),
    /// Two fingers are down: single-pointer drags are ignored.
    pinching: bool,
    /// Viewport size seen by the last render (needed to clamp the pan).
    viewport: Cell<(f32, f32)>,
    info: bool,
    details: Option<Rc<Asset>>,
    video: Option<VideoHandle>,
}

fn xy(p: Point<Pixels>) -> (f32, f32) {
    (p.x.into(), p.y.into())
}

impl Viewer {
    fn reset_view(&mut self) {
        self.zoom = 1.;
        self.pan = (0., 0.);
        self.dx = 0.;
        self.dy = 0.;
        self.video = None;
        self.details = None;
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
    /// Opens the viewer on `items[index]`, zooming out of the grid tile with that index if it was painted.
    pub(super) fn open_viewer(&mut self, items: Rc<Vec<TimelineItem>>, index: usize, cx: &mut Context<Self>) {
        let from = self.tile_bounds.borrow().get(&index).copied();
        self.viewer_serial += 1;
        self.viewer = Some(Viewer {
            items,
            index,
            from,
            closing: false,
            serial: self.viewer_serial,
            start: None,
            last: (0., 0.),
            moved: 0.,
            dx: 0.,
            dy: 0.,
            zoom: 1.,
            pan: (0., 0.),
            pinching: false,
            viewport: Cell::new((0., 0.)),
            info: false,
            details: None,
            video: None,
        });
        if self.pinch.is_none() {
            let weak = cx.weak_entity();
            let async_cx = cx.to_async();
            self.pinch = pinch::install(move |event| {
                let mut app = async_cx.clone();
                weak.update(&mut app, |this, cx| this.pinch(event, cx)).ok();
            });
        }
        cx.notify();
    }

    pub(super) fn close_viewer(&mut self, cx: &mut Context<Self>) {
        let Some(v) = self.viewer.as_mut() else { return };
        if v.closing {
            return;
        }
        v.closing = true;
        v.start = None;
        v.video = None;
        v.zoom = 1.;
        v.pan = (0., 0.);
        self.viewer_serial += 1;
        v.serial = self.viewer_serial;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(400)).await;
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
        if let Some(v) = self.viewer.as_mut().filter(|v| !v.closing && !v.pinching) {
            v.start = Some(xy(at));
            v.last = xy(at);
            v.moved = 0.;
            v.dx = 0.;
            v.dy = 0.;
            cx.notify();
        }
    }

    fn drag_move(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(v) = self.viewer.as_mut().filter(|v| !v.pinching) else { return };
        let Some((sx, sy)) = v.start else { return };
        let (x, y) = xy(at);
        v.moved += (x - v.last.0).abs() + (y - v.last.1).abs();
        if v.zoom > 1. {
            v.pan = (v.pan.0 + x - v.last.0, v.pan.1 + y - v.last.1);
            v.clamp_pan();
        } else {
            v.dx = x - sx;
            v.dy = (y - sy).max(0.);
        }
        v.last = (x, y);
        cx.notify();
    }

    fn drag_end(&mut self, width: f32, cx: &mut Context<Self>) {
        let Some(v) = self.viewer.as_mut() else { return };
        if v.start.take().is_none() {
            return;
        }
        if v.zoom > 1. {
            cx.notify();
            return;
        }
        let (dx, dy) = (v.dx, v.dy);
        v.dx = 0.;
        v.dy = 0.;
        if dy > CLOSE_DISTANCE && dy > dx.abs() {
            self.close_viewer(cx);
            return;
        }
        let count = v.items.len();
        if dx.abs() > width * PAGE_FRACTION {
            if dx < 0. && v.index + 1 < count {
                v.index += 1;
                v.reset_view();
            } else if dx > 0. && v.index > 0 {
                v.index -= 1;
                v.reset_view();
            }
        }
        self.refresh_details(cx);
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

    fn pinch(&mut self, event: Pinch, cx: &mut Context<Self>) {
        let Some(v) = self.viewer.as_mut().filter(|v| !v.closing) else { return };
        match event {
            Pinch::Begin => {
                v.pinching = true;
                v.start = None;
                v.dx = 0.;
                v.dy = 0.;
            }
            Pinch::Move { ratio, x, y } => v.zoom_to(v.zoom * ratio, (x, y)),
            Pinch::End => v.pinching = false,
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
                let client = ImmichClient::new(&origin, FetchTransport).ok()?;
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
        let top = self.top.max(12.) + 52.;
        let origin = self.origin.clone();
        let Some(v) = self.viewer.as_mut().filter(|v| !v.closing) else { return };
        let Some(item) = v.items.get(v.index).filter(|i| i.is_video()) else { return };
        let url = format!("{origin}/api/assets/{}/video/playback", item.id);
        v.video = video::show(&url, top, 110.);
        cx.notify();
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

    pub(super) fn viewer_overlay(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let v = self.viewer.as_ref()?;
        let vs = window.viewport_size();
        let (vw, vh): (f32, f32) = (vs.width.into(), vs.height.into());
        v.viewport.set((vw, vh));
        let (index, closing, dx, dy, zoom, pan) = (v.index, v.closing, v.dx, v.dy, v.zoom, v.pan);
        let from = v.from.map(|b| {
            let (x, y): (f32, f32) = (b.origin.x.into(), b.origin.y.into());
            let (w, h): (f32, f32) = (b.size.width.into(), b.size.height.into());
            (x, y, w, h)
        });
        let items = v.items.clone();
        let origin = self.origin.clone();
        let top = self.top.max(12.);
        let count = items.len();
        let current = items.get(index).cloned();

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
            let url = |size: &str| format!("{origin}/api/assets/{}/thumbnail?size={size}", item.id);
            let thumb = thumbs::get(&store, &format!("t:{}", item.id), || url("thumbnail"), cx);
            let preview = thumbs::get(&store, &format!("p:{}", item.id), || url("preview"), cx);
            let full = if k == index && zoom > FULLSIZE_ZOOM { thumbs::get(&store, &format!("f:{}", item.id), || url("fullsize"), cx) } else { None };
            pictures.insert(k, [thumb, preview, full]);
        }
        let page = move |k: usize| -> Option<AnyElement> {
            let layers = pictures.get(&k)?;
            let mut page = div().size_full();
            for image in layers.iter().flatten() {
                page = page.child(img(image.clone()).absolute().size_full().object_fit(ObjectFit::Contain));
            }
            Some(page.into_any_element())
        };
        let date = current.as_ref().map(|i| i.date_bucket.clone()).unwrap_or_default();
        let is_video = current.as_ref().is_some_and(|i| i.is_video());
        let favorite = current.as_ref().is_some_and(|i| i.is_favorite);
        let playing = v.video.is_some();
        let bottom = self.bottom.max(10.);

        let chip = |id: &'static str, label: String, color: Rgba| {
            glass_div(GLASS_TINT_DARK, 18.)
                .id(id)
                .h(px(36.))
                .px_4()
                .flex()
                .items_center()
                .text_color(color)
                .text_size(px(17.))
                .child(label)
        };

        let info_panel = (v.info && !closing).then(|| {
            let lines = match &v.details {
                Some(asset) => Self::info_lines(asset),
                None => vec![("", "Lädt …".to_string())],
            };
            glass_div(GLASS_TINT_DARK, 24.)
                .absolute()
                .left(px(16.))
                .right(px(16.))
                .bottom(px(bottom + 108.))
                .p_4()
                .flex()
                .flex_col()
                .gap_2()
                .text_color(rgb(0xffffff))
                .children(lines.into_iter().map(|(label, value)| {
                    div()
                        .flex()
                        .flex_row()
                        .gap_3()
                        .child(div().w(px(104.)).flex_none().text_size(px(13.)).text_color(rgba(0xffffffaa)).child(label))
                        .child(div().flex_1().text_size(px(15.)).child(value))
                }))
        });

        Some(
            div()
                .id("viewer-root")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .occlude()
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
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .with_animation(("viewer-anim", v.serial), anim(if closing { 360 } else { 420 }), move |el, d| {
                            let t = if closing { 1. - d } else { d };
                            // While dragging down the backdrop fades and the photo follows the finger.
                            let fade = (1. - dy / 400.).clamp(0.3, 1.);
                            let settled = t >= 1.;
                            let (x0, y0, w, h) = match from {
                                Some((fx, fy, fw, fh)) if !settled => (lerp(fx, 0., t), lerp(fy, 0., t), lerp(fw, vw, t), lerp(fh, vh, t)),
                                _ if settled => {
                                    let (w, h) = (vw * zoom, vh * zoom);
                                    ((vw - w) / 2. + pan.0 + dx, (vh - h) / 2. + pan.1 + dy, w, h)
                                }
                                _ => (0., 0., vw, vh),
                            };
                            let mut el = el
                                .child(div().absolute().size_full().bg(rgba(0x000000ff)).opacity(t * fade))
                                .child(div().absolute().left(px(x0)).top(px(y0)).w(px(w)).h(px(h)).children(page(index)));
                            if settled && zoom == 1. {
                                // Neighbours slide in beside the current page.
                                if index > 0 {
                                    el = el.child(div().absolute().left(px(dx - vw)).top_0().w(px(vw)).h(px(vh)).children(page(index - 1)));
                                }
                                if index + 1 < count {
                                    el = el.child(div().absolute().left(px(dx + vw)).top_0().w(px(vw)).h(px(vh)).children(page(index + 1)));
                                }
                            }
                            el.child(
                                div()
                                    .absolute()
                                    .bottom(px(bottom + 4.))
                                    .w_full()
                                    .flex()
                                    .justify_center()
                                    .opacity(t * fade)
                                    .text_color(rgb(0xffffff))
                                    .text_size(px(15.))
                                    .child(format!("{date}  ·  {} von {count}", index + 1)),
                            )
                        }),
                )
                .children(info_panel)
                .children((!closing).then(|| {
                    div()
                        .absolute()
                        .bottom(px(bottom + 40.))
                        .w_full()
                        .flex()
                        .justify_center()
                        .gap_3()
                        .child(
                            glass_div(GLASS_TINT_DARK, 18.)
                                .id("viewer-fav")
                                .h(px(36.))
                                .w(px(48.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(heart(favorite))
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_favorite(cx))),
                        )
                        .child(
                            chip("viewer-info", "Info".into(), if v.info { rgb(BLUE).into() } else { rgb(0xffffff).into() })
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_info(cx))),
                        )
                        .children(
                            (is_video && !playing).then(|| chip("viewer-play", "Abspielen".into(), rgb(0xffffff).into()).on_click(cx.listener(|this, _, _, cx| this.play_video(cx)))),
                        )
                }))
                .children((!closing).then(|| {
                    glass_div(GLASS_TINT_DARK, 18.)
                        .id("viewer-done")
                        .absolute()
                        .top(px(top + 8.))
                        .right(px(16.))
                        .h(px(36.))
                        .px_4()
                        .flex()
                        .items_center()
                        .text_color(rgb(0xffffff))
                        .text_size(px(17.))
                        .child("Fertig")
                        .on_click(cx.listener(|this, _, _, cx| this.close_viewer(cx)))
                }))
                .into_any_element(),
        )
    }
}

/// A heart, filled red when `filled`, otherwise a white outline (the font has no heart glyph).
fn heart(filled: bool) -> Canvas<()> {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let (x, y): (f32, f32) = (bounds.origin.x.into(), bounds.origin.y.into());
            let k = f32::from(bounds.size.width) / 24.;
            let at = |px_: f32, py_: f32| point(px(x + px_ * k), px(y + py_ * k));
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
                window.paint_path(path, if filled { rgb(RED) } else { rgb(0xffffff) });
            }
        },
    )
    .size(px(22.))
}
