//! "Zum Album hinzufügen": a bottom sheet (`components::sheet`, like settings and jobs) listing the synced albums.
//! `Immagine::open_album_picker(ids, from_selection, cx)` opens it for any list of asset ids; the selection
//! toolbar and the viewer's "..." menu both use it. A tap on an album queues `AlbumOp::add` (applied locally at
//! once, sent by the outbox), closes the sheet and, when it came from the selection, ends select mode.
//!
//! It is routed like the other sheets (`Immagine::top_sheet`) and drawn after the viewer overlay, so it also
//! covers the viewer.

use super::Immagine;
use crate::components::sheet::{self, Sheet};
use crate::logic::models::album_op::AlbumOp;
use crate::logic::models::album_tree::AlbumPath;
use crate::theme::Palette;
use gpui_kit::*;

/// Height of one album row (tall enough to tap).
const ROW: f32 = 56.;

#[derive(Default)]
pub(super) struct PickerState {
    pub(super) sheet: Sheet,
    /// The assets a pick adds, and whether they are the current selection (select mode ends after the pick).
    ids: Vec<String>,
    from_selection: bool,
}

impl Immagine {
    /// Opens the picker for `ids`. Nothing happens without ids.
    pub(super) fn open_album_picker(&mut self, ids: Vec<String>, from_selection: bool, cx: &mut Context<Self>) {
        if ids.is_empty() {
            return;
        }
        self.picker.ids = ids;
        self.picker.from_selection = from_selection;
        self.picker.sheet.open();
        cx.notify();
    }

    fn close_album_picker(&mut self, cx: &mut Context<Self>) {
        self.picker.sheet.close();
        cx.notify();
    }

    /// Adds the picker's assets to `album`.
    pub(super) fn pick_album(&mut self, album: &str, cx: &mut Context<Self>) {
        if !self.picker.sheet.is_open() {
            return;
        }
        let ids = std::mem::take(&mut self.picker.ids);
        let from_selection = self.picker.from_selection;
        self.picker.sheet.close();
        if ids.is_empty() {
            return;
        }
        let Some(engine) = self.engine.as_mut() else {
            self.status = "Bitte kurz warten, es wird synchronisiert.".into();
            cx.notify();
            return;
        };
        engine.enqueue(&[AlbumOp::add(ids, album)]);
        if let Some(e) = &self.engine {
            self.albums.refresh(e.store());
        }
        if from_selection {
            self.select_end();
        }
        self.sync(cx);
        cx.notify();
    }

    pub(super) fn picker_overlay(&self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let sheet = &self.picker.sheet;
        if !sheet.visible() {
            return None;
        }
        let row_bg = p.ui.card;
        let mut list = div().px_4().pt_2().pb(px(self.bottom.max(10.) + 16.)).flex().flex_col().gap(px(1.));
        if self.albums.albums.is_empty() {
            list = list.child(div().py_4().text_size(px(15.)).text_color(p.secondary).child("Keine Alben vorhanden."));
        }
        let count = self.albums.albums.len();
        for (i, album) in self.albums.albums.iter().enumerate() {
            let id = album.id.clone();
            let mut row = div()
                .id(("picker-album", i))
                .h(px(ROW))
                .px(px(16.))
                .bg(row_bg)
                .flex()
                .items_center()
                .text_size(px(17.))
                .text_color(p.label)
                .active(crate::components::button::dim)
                .child(AlbumPath::leaf_of_album_name(&album.album_name));
            if i == 0 {
                row = row.rounded_t(px(14.));
            }
            if i + 1 == count {
                row = row.rounded_b(px(14.));
            }
            list = list.child(row.on_click(cx.listener(move |this, _, window, cx| {
                if !window.tap_caught_fling() {
                    this.pick_album(&id, cx)
                }
            })));
        }
        Some(sheet::overlay(
            sheet,
            |this: &Immagine| &this.picker.sheet,
            cx,
            window,
            sheet::Spec { id: "picker", title: "Zum Album hinzufügen", top_margin: self.top.max(12.) + 10., theme: &p.ui },
            list,
            |this, cx| this.close_album_picker(cx),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn picker_state_starts_closed() {
        let state = PickerState::default();
        assert!(!state.sheet.visible());
        assert!(state.ids.is_empty());
    }

    #[::core::prelude::v1::test]
    fn pick_from_selection_queues_closes_and_ends_select_mode() {
        let mut cx = ::gpui::TestAppContext::single();
        let (view, cx) = cx.add_window_view(|w, cx| Immagine::new(0., 0., false, false, w, cx));
        view.update(cx, |app, cx| {
            app.install_engine(crate::logic::store::MemoryStore::new(), cx);
            app.select.on = true;
            app.select.ids.borrow_mut().insert("a1".into());
            app.open_album_picker(vec!["a1".into()], true, cx);
            assert!(app.picker.sheet.is_open());
            app.pick_album("al", cx);
            assert!(!app.picker.sheet.is_open(), "the sheet closes");
            assert!(!app.select.on, "select mode ends after a pick from the selection");
            assert!(app.syncing, "the op was queued and the outbox is being flushed");
        });
    }

    #[::core::prelude::v1::test]
    fn pick_from_the_viewer_keeps_select_mode_and_empty_lists_do_not_open() {
        let mut cx = ::gpui::TestAppContext::single();
        let (view, cx) = cx.add_window_view(|w, cx| Immagine::new(0., 0., false, false, w, cx));
        view.update(cx, |app, cx| {
            app.install_engine(crate::logic::store::MemoryStore::new(), cx);
            app.open_album_picker(Vec::new(), false, cx);
            assert!(!app.picker.sheet.visible());
            app.select.on = true;
            app.open_album_picker(vec!["a1".into()], false, cx);
            app.pick_album("al", cx);
            assert!(app.select.on && app.syncing);
            // A second tap on the closing sheet does nothing.
            app.pick_album("al", cx);
        });
    }
}
