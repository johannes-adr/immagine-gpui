//! Owner: albums agent. Swift sources: `../immagine/Immagine/UI/Albums/*.swift` (AlbumsView, AlbumDetailView).
//! Albums tab: album and folder tiles from the store, album page with its photos (`photo_grid`).

use super::{thumbs, Immagine};
use crate::components::glass::glass_div;
use crate::logic::models::album::Album;
use crate::logic::models::album_tree::{AlbumPath, AlbumTree};
use crate::logic::models::timeline::TimelineItem;
use crate::logic::store::MemoryStore;
use crate::theme::{Palette, BLUE};
use gpui_kit::*;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;

const GAP: f32 = 16.;
const PAD: f32 = 20.;
/// Tiles are at least this wide on phones; wider windows get more columns.
const TILE_MIN: f32 = 200.;

/// The asset that stands for an album or folder tile.
#[derive(Clone)]
struct Cover {
    asset_id: String,
    thumbhash: Option<String>,
}

/// The album page that is open, with its photos newest first.
struct OpenAlbum {
    id: String,
    name: String,
    items: Rc<Vec<TimelineItem>>,
    scroll: UniformListScrollHandle,
}

/// UI state of the Albums tab; extend as needed.
#[derive(Default)]
pub(super) struct AlbumsState {
    pub(super) albums: Vec<Album>,
    tree: AlbumTree,
    /// Visible asset count per album id (falls back to `asset_count`).
    counts: BTreeMap<String, usize>,
    covers: HashMap<String, Cover>,
    /// Folder shown in the browser ("" = top level).
    path: String,
    open: Option<OpenAlbum>,
}

impl AlbumsState {
    /// Scroll handle of the open album page's photo list.
    pub(super) fn open_scroll(&self) -> Option<&UniformListScrollHandle> {
        self.open.as_ref().map(|o| &o.scroll)
    }

    /// Called after every sync and after the first load; copy what the view needs out of the store.
    pub(super) fn refresh(&mut self, store: &MemoryStore) {
        use crate::logic::store::Store;
        self.albums = store.albums();
        self.counts = store.album_visible_counts();
        self.tree = AlbumTree::new(&self.albums, &store.local_folders());
        self.covers = self
            .albums
            .iter()
            .filter_map(|album| {
                let cover = match album.album_thumbnail_asset_id.as_deref().filter(|id| !id.is_empty()) {
                    Some(id) => Cover { asset_id: id.to_string(), thumbhash: store.thumbhash(id) },
                    None => store
                        .album_assets(&album.id)
                        .into_iter()
                        .max_by(|a, b| a.sort_date.cmp(&b.sort_date))
                        .map(|item| Cover { asset_id: item.id, thumbhash: item.thumbhash })?,
                };
                Some((album.id.clone(), cover))
            })
            .collect();
        if let Some(open) = self.open.as_mut() {
            open.items = Rc::new(newest_first(store.album_assets(&open.id)));
        }
        if !self.tree.contains_folder(&self.path) {
            self.path.clear();
        }
    }
}

/// Cover counts of one browser render, for the debug line: tiles asking, tiles with a cover asset, covers with a
/// thumbhash, covers whose thumbnail is loaded.
#[derive(Default)]
struct CoverTally {
    tiles: usize,
    covers: usize,
    hashes: usize,
    loaded: usize,
}

fn newest_first(mut items: Vec<TimelineItem>) -> Vec<TimelineItem> {
    items.sort_by(|a, b| b.sort_date.cmp(&a.sort_date));
    items
}

fn count_label(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

/// Name and count under a tile.
fn caption(p: &Palette, name: &str, detail: String) -> Div {
    div()
        .flex()
        .flex_col()
        .px_1()
        .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).truncate().child(name.to_string()))
        .child(div().text_size(px(13.)).text_color(p.secondary).child(detail))
}

/// Glass chip used for the back buttons.
fn chip(p: &Palette, id: &'static str, label: String) -> Stateful<Div> {
    glass_div(p.glass_tint, 18.)
        .id(id)
        .h(px(36.))
        .px_4()
        .flex()
        .items_center()
        .text_size(px(17.))
        .text_color(rgb(BLUE))
        .active(|s| s.opacity(0.6))
        .child(label)
}

impl Immagine {
    pub(super) fn albums_view(&mut self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        if self.albums.open.is_some() {
            self.album_page(p, window, cx)
        } else {
            self.album_browser(p, window, cx)
        }
    }

    /// Opens an album page; its photos are read from the store now and again on every sync.
    pub(super) fn open_album(&mut self, id: &str, cx: &mut Context<Self>) {
        use crate::logic::store::Store;
        let Some(engine) = self.engine.as_ref() else { return };
        let name = self
            .albums
            .albums
            .iter()
            .find(|a| a.id == id)
            .map(|a| AlbumPath::leaf_of_album_name(&a.album_name))
            .unwrap_or_default();
        let items = Rc::new(newest_first(engine.store().album_assets(id)));
        self.albums.open = Some(OpenAlbum { id: id.to_string(), name, items, scroll: UniformListScrollHandle::new() });
        cx.notify();
    }

    fn close_album(&mut self, cx: &mut Context<Self>) {
        self.albums.open = None;
        cx.notify();
    }

    fn go_to_folder(&mut self, path: String, cx: &mut Context<Self>) {
        self.albums.path = path;
        cx.notify();
    }

    /// The picture for a cover: the grid thumbnail when loaded, else its thumbhash stand-in. Counts into `tally`
    /// for the debug line.
    fn cover_image(&self, cover: Option<&Cover>, tally: &mut CoverTally, cx: &mut App) -> Option<Arc<RenderImage>> {
        tally.tiles += 1;
        let cover = cover?;
        tally.covers += 1;
        tally.hashes += cover.thumbhash.is_some() as usize;
        let loaded = thumbs::get(
            &self.thumbs,
            &format!("t:{}", cover.asset_id),
            || thumbs::thumb_url(&self.origin, &cover.asset_id, "thumbnail", cover.thumbhash.as_deref()),
            cx,
        );
        tally.loaded += loaded.is_some() as usize;
        loaded.or_else(|| cover.thumbhash.as_deref().and_then(|hash| thumbs::placeholder(&self.thumbs, &cover.asset_id, hash)))
    }

    /// Debug line (setting `immagine.debug`, "FPS anzeigen"): the image store's counters, plus cover counts in the
    /// browser.
    fn thumbs_debug(&self, p: &Palette, tally: Option<&CoverTally>) -> Option<Div> {
        if !self.debug {
            return None;
        }
        let mut text = self.thumbs.borrow().stats();
        if let Some(t) = tally {
            text = format!("covers {}/{} hash{} loaded{} | {text}", t.covers, t.tiles, t.hashes, t.loaded);
        }
        Some(div().text_size(px(11.)).text_color(p.secondary).child(text))
    }

    /// The album page: back chip, album name, then the photo grid.
    fn album_page(&self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(open) = self.albums.open.as_ref() else {
            return div().into_any_element();
        };
        let subtitle = count_label(open.items.len(), "Foto", "Fotos");
        let back = div().flex().flex_row().child(super::icons::back_chip(p.glass_tint, "album-back", rgb(BLUE).into()).on_click(cx.listener(|this, _, _, cx| this.close_album(cx))));
        let header = div()
            .px(px(PAD))
            .pt(px(20.))
            .pb_2()
            .flex()
            .flex_col()
            .gap_1()
            .child(back)
            .child(div().text_size(px(34.)).font_weight(FontWeight::BOLD).truncate().child(open.name.clone()))
            .child(div().text_size(px(15.)).text_color(p.secondary).child(subtitle))
            .children(self.thumbs_debug(p, None));
        let list = self.photo_grid("album-photos", open.items.clone(), 0., (&open.scroll, None), p, window, cx);
        self.rubber.shift(div().size_full().flex().flex_col().pt(px(self.top.max(12.))).child(header).child(list)).into_any_element()
    }

    /// One folder level: a breadcrumb and back chip for nested folders, then folder and album tiles.
    fn album_browser(&self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let st = &self.albums;
        // Ages the image store (retries, aborts, LRU), which otherwise only the grids and the people list do.
        self.thumbs.borrow_mut().begin_frame();
        let vw: f32 = window.viewport_size().width.into();
        let columns = ((vw / TILE_MIN).floor() as usize).clamp(2, 6);
        let tile = ((vw - 2. * PAD - GAP * (columns as f32 - 1.)) / columns as f32).floor().max(40.);
        let half = (tile / 2.).floor();
        let mut tally = CoverTally::default();
        let folders = st.tree.folders(&st.path);
        let albums = st.tree.albums(&st.path);

        let mut tiles: Vec<AnyElement> = Vec::new();
        for (i, folder) in folders.iter().enumerate() {
            // 2x2 collage of the first albums below the folder.
            let mut covers: Vec<Option<Arc<RenderImage>>> = Vec::new();
            for album in st.tree.collage_albums(&folder.path, 4) {
                covers.push(self.cover_image(st.covers.get(&album.id), &mut tally, cx));
            }
            let quads = (0..4).map(|k| {
                let picture = covers.get(k).cloned().flatten();
                div()
                    .size(px(half))
                    .bg(p.chip)
                    .children(picture.map(|image| img(image).size_full().object_fit(ObjectFit::Cover)))
                    .into_any_element()
            });
            let cell = div()
                .size(px(tile))
                .rounded(px(16.))
                .overflow_hidden()
                .bg(p.chip)
                .flex()
                .flex_wrap()
                .children(quads);
            let target = folder.path.clone();
            tiles.push(
                div()
                    .id(("album-folder", i))
                    .w(px(tile))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .active(|s| s.opacity(0.6))
                    .child(cell)
                    .child(caption(p, &folder.name, count_label(folder.album_count, "Album", "Alben")))
                    .on_click(cx.listener(move |this, _, _, cx| this.go_to_folder(target.clone(), cx)))
                    .into_any_element(),
            );
        }
        for (i, album) in albums.iter().enumerate() {
            let picture = self.cover_image(st.covers.get(&album.id), &mut tally, cx);
            let count = st.counts.get(&album.id).copied().unwrap_or(album.asset_count.max(0) as usize);
            let cell = div()
                .size(px(tile))
                .rounded(px(16.))
                .overflow_hidden()
                .bg(p.chip)
                .children(picture.map(|image| img(image).size_full().object_fit(ObjectFit::Cover)));
            let id = album.id.clone();
            tiles.push(
                div()
                    .id(("album-tile", i))
                    .w(px(tile))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .active(|s| s.opacity(0.6))
                    .child(cell)
                    .child(caption(p, &AlbumPath::leaf_of_album_name(&album.album_name), count_label(count, "Foto", "Fotos")))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if !window.tap_caught_fling() {
                            this.open_album(&id, cx)
                        }
                    }))
                    .into_any_element(),
            );
        }

        // Breadcrumb and back chip inside nested folders.
        let mut header = div().px(px(PAD)).pt(px(20.)).pb_2().flex().flex_col().gap_1();
        let title = if st.path.is_empty() {
            "Alben".to_string()
        } else {
            let parent = AlbumPath::parent_of(&st.path);
            header = header.child(
                div().flex().flex_row().child(
                    super::icons::back_chip(p.glass_tint, "albums-up", rgb(BLUE).into()).on_click(cx.listener(move |this, _, _, cx| this.go_to_folder(parent.clone(), cx))),
                ),
            );
            let mut crumbs = div().flex().flex_row().flex_wrap().gap_1().text_size(px(15.)).text_color(p.secondary);
            let mut prefix = String::new();
            crumbs = crumbs.child(
                div()
                    .id("album-crumb-root")
                    .child("Alben")
                    .on_click(cx.listener(|this, _, _, cx| this.go_to_folder(String::new(), cx))),
            );
            for (i, part) in AlbumPath::components(&st.path).into_iter().enumerate() {
                prefix = AlbumPath::join(&prefix, part);
                let target = prefix.clone();
                crumbs = crumbs.child("›").child(
                    div()
                        .id(("album-crumb", i))
                        .child(part.to_string())
                        .on_click(cx.listener(move |this, _, _, cx| this.go_to_folder(target.clone(), cx))),
                );
            }
            header = header.child(crumbs);
            AlbumPath::name_of(&st.path)
        };
        let subtitle = format!("{} Ordner · {}", folders.len(), count_label(albums.len(), "Album", "Alben"));
        header = header
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(div().flex_1().min_w_0().text_size(px(34.)).font_weight(FontWeight::BOLD).truncate().child(title))
                    .child(
                        glass_div(p.glass_tint, 18.)
                            .id("albums-settings")
                            .size(px(36.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .active(|s| s.opacity(0.6))
                            .child(super::icons::glyph(super::icons::Glyph::Gear, 20., p.label))
                            .on_click(cx.listener(|this, _, _, cx| this.open_settings(cx))),
                    ),
            )
            .child(div().text_size(px(15.)).text_color(p.secondary).child(subtitle))
            .children(self.thumbs_debug(p, Some(&tally)));

        let body = if tiles.is_empty() {
            let empty = if st.path.is_empty() { "Keine Alben" } else { "Leerer Ordner" };
            div().px(px(PAD)).pt(px(16.)).text_size(px(15.)).text_color(p.secondary).child(empty).into_any_element()
        } else {
            div().px(px(PAD)).pt(px(8.)).flex().flex_wrap().gap(px(GAP)).children(tiles).into_any_element()
        };

        self.rubber
            .shift(div().size_full().flex().flex_col().pt(px(self.top.max(12.))).child(header).child(
                div()
                    .id("albums-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.albums_scroll)
                    .child(body)
                    .child(div().h(px(self.bottom.max(10.) + 84.))),
            ))
            .into_any_element()
    }
}
