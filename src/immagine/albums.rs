//! Owner: albums agent. Swift sources: `../immagine/Immagine/UI/Albums/*.swift` (AlbumsView, AlbumDetailView).
//! Albums tab: album and folder tiles from the store, album page with its photos (`photo_grid`).

use super::Immagine;
use crate::logic::models::album::Album;
use crate::logic::store::MemoryStore;
use crate::theme::Palette;
use gpui_kit::*;

/// UI state of the Albums tab; extend as needed.
#[derive(Default)]
pub(super) struct AlbumsState {
    pub(super) albums: Vec<Album>,
}

impl AlbumsState {
    /// Called after every sync and after the first load; copy what the view needs out of the store.
    pub(super) fn refresh(&mut self, store: &MemoryStore) {
        use crate::logic::store::Store;
        self.albums = store.albums();
    }
}

impl Immagine {
    pub(super) fn albums_view(&mut self, p: &Palette, _window: &Window, _cx: &mut Context<Self>) -> AnyElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(17.))
            .text_color(p.secondary)
            .child(format!("{} Alben", self.albums.albums.len()))
            .into_any_element()
    }
}
