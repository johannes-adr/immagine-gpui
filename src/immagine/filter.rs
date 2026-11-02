//! The filter button left of "Auswählen" in the Mediathek header: a glass menu (morphing out of the button) with
//! "Nicht in Alben", "Favoriten", "Nur Fotos", "Nur Videos". The pure rules are in `logic::filter`.
//!
//! The filtered list is computed once and cached (by the source list, its length, the filter and
//! `AlbumsState::generation`), so rendering a frame costs nothing extra. It applies to the Alle view only; Jahre and
//! Monate keep showing everything.

use super::Immagine;
use crate::components::button::{self, Surface};
use crate::components::icon::{glyph, Glyph};
use crate::components::measure;
use crate::components::morph::{self, Morph};
use crate::logic::filter::{Filter, Kind};
use crate::logic::models::timeline::TimelineItem;
use crate::logic::store::Store;
use crate::theme::{Palette, BLUE};
use crate::util::now_ms;
use gpui_kit::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const ROW: f32 = 50.;
const WIDTH: f32 = 236.;
const BUTTON: f32 = 36.;

/// Source list, its length, the filter, albums generation, number of stored downloads, result.
type Cached = (Rc<Vec<TimelineItem>>, usize, Filter, u32, usize, Rc<Vec<TimelineItem>>);

#[derive(Default)]
pub(super) struct FilterState {
    pub(super) filter: Filter,
    morph: Morph,
    /// Where the button is on screen (measured every frame), the morph's anchor.
    button: Rc<Cell<Option<morph::Rect>>>,
    cache: RefCell<Option<Cached>>,
}

impl Immagine {
    /// The timeline as the Alle view shows it: `items` through the filter.
    pub(super) fn shown_items(&self) -> Rc<Vec<TimelineItem>> {
        let f = self.filter.filter;
        if !f.is_active() {
            return self.items.clone();
        }
        let serial = self.albums.generation;
        let stored = self.downloads.ids.borrow().len();
        if let Some((src, len, cf, cg, cd, out)) = self.filter.cache.borrow().as_ref() {
            if Rc::ptr_eq(src, &self.items) && *len == self.items.len() && *cf == f && *cg == serial && *cd == stored {
                return out.clone();
            }
        }
        let out = Rc::new(match self.engine.as_ref() {
            Some(engine) => {
                let downloads = self.downloads.ids.borrow();
                f.apply(&self.items, |ids| engine.store().asset_ids_in_any_album(ids), |id| downloads.contains(id))
            },
            None => Vec::new(),
        });
        *self.filter.cache.borrow_mut() = Some((self.items.clone(), self.items.len(), f, serial, stored, out.clone()));
        out
    }

    fn set_filter(&mut self, f: Filter, cx: &mut Context<Self>) {
        self.filter.filter = f;
        // The list changes under the scroll position: start at the newest photos again.
        self.timeline_placed.set(false);
        cx.notify();
    }

    fn open_filter(&mut self, cx: &mut Context<Self>) {
        if self.filter.button.get().is_some() && self.filter.morph.open(now_ms()) {
            cx.notify();
        }
    }

    fn close_filter(&mut self, cx: &mut Context<Self>) {
        if !self.filter.morph.close(now_ms()) {
            return;
        }
        cx.notify();
        Morph::finish_after(cx, |t: &mut Self| Some(&mut t.filter.morph));
    }

    /// The round filter button; blue while a filter is on.
    pub(super) fn filter_button(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        if self.filter.morph.active() {
            let (w, h) = self.filter.button.get().map_or((BUTTON, BUTTON), |(_, _, w, h)| (w, h));
            return div().w(px(w)).h(px(h)).flex_none().into_any_element();
        }
        let rect = self.filter.button.clone();
        let color = if self.filter.filter.is_active() { rgb(BLUE).into() } else { p.label };
        button::circle("tl-filter", Surface::new(p.bar_tint), BUTTON)
            .relative()
            .occlude()
            .active(button::dim)
            .child(glyph(Glyph::Filter, 20., color))
            .child(measure::bounds(move |b| rect.set(Some(measure::rect(b)))))
            .on_click(cx.listener(|this, _, _, cx| this.open_filter(cx)))
            .into_any_element()
    }

    pub(super) fn filter_overlay(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let m = self.filter.morph;
        if !m.active() {
            return None;
        }
        let (ax, ay, aw, ah) = self.filter.button.get()?;
        let vw: f32 = window.viewport_size().width.into();
        let f = self.filter.filter;
        let white: Rgba = rgb(0xffffff).into();
        let open = m.is_open();
        let row = |id: &'static str, label: &'static str, on: bool, next: Filter| {
            let r = div()
                .id(id)
                .h(px(ROW))
                .px(px(10.))
                .rounded(px(16.))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .text_size(px(17.))
                .text_color(white)
                .active(|s| s.opacity(0.6))
                .child(div().w(px(24.)).flex_none().flex().justify_center().children(on.then(|| glyph(Glyph::Check, 18., rgb(BLUE).into()))))
                .child(label);
            if open {
                r.on_click(cx.listener(move |this, _, _, cx| this.set_filter(next, cx)))
            } else {
                r
            }
        };
        let photos = if f.kind == Kind::Photos { Kind::All } else { Kind::Photos };
        let videos = if f.kind == Kind::Videos { Kind::All } else { Kind::Videos };
        let content = div()
            .size_full()
            .p(px(8.))
            .flex()
            .flex_col()
            .child(row("filter-no-album", "Nicht in Alben", f.no_album, Filter { no_album: !f.no_album, ..f }))
            .child(row("filter-favorites", "Favoriten", f.favorites, Filter { favorites: !f.favorites, ..f }))
            .child(row("filter-downloaded", "Heruntergeladen", f.downloaded, Filter { downloaded: !f.downloaded, ..f }))
            .child(row("filter-photos", "Nur Fotos", f.kind == Kind::Photos, Filter { kind: photos, ..f }))
            .child(row("filter-videos", "Nur Videos", f.kind == Kind::Videos, Filter { kind: videos, ..f }))
            .into_any_element();
        let (cw, ch) = (WIDTH, 5. * ROW + 16.);
        let spec = morph::Spec {
            id: "filter",
            anchor: (ax, ay, aw, ah),
            card: (ax.min(vw - 16. - cw).max(16.), ay, cw, ch),
            tint: morph::DIALOG_TINT,
            anchor_radius: ah / 2.,
            card_radius: 26.,
        };
        morph::overlay(m, spec, now_ms(), window, content, |this: &mut Self, cx| this.close_filter(cx), cx)
    }
}
