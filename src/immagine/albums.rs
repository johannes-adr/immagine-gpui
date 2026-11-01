//! Owner: albums agent. Swift sources: `../immagine/Immagine/UI/Albums/*.swift` (AlbumsView, AlbumDetailView).
//! Albums tab: album and folder tiles from the store, album page with its photos (`photo_grid`).

use super::{thumbs, Immagine};
use crate::components::glass::glass_div;
use crate::host::{Current as H, Host};
use crate::logic::client::ImmichClient;
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

/// The extra pages at the bottom of the top-level browser. They are views on the store (no server album, no
/// links), so they never go through `repair_albums` or the album picker.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Special {
    Videos,
    Trash,
    /// Hidden entry: its tile is an empty, tappable slot.
    Archive,
}

impl Special {
    const ALL: [Special; 3] = [Special::Videos, Special::Trash, Special::Archive];

    fn title(self) -> &'static str {
        match self {
            Special::Videos => "Videos",
            Special::Trash => "Papierkorb",
            Special::Archive => "Archiviert",
        }
    }

    fn empty(self) -> &'static str {
        match self {
            Special::Videos => "Keine Videos",
            Special::Trash => "Papierkorb ist leer",
            Special::Archive => "Archiv ist leer",
        }
    }

    /// Glyph for an empty tile; the hidden Archiviert slot shows none.
    fn glyph(self) -> Option<crate::components::icon::Glyph> {
        use crate::components::icon::Glyph;
        match self {
            Special::Videos => Some(Glyph::Play),
            Special::Trash => Some(Glyph::Trash),
            Special::Archive => None,
        }
    }

    /// Newest first.
    fn items(self, store: &MemoryStore) -> Vec<TimelineItem> {
        use crate::logic::store::Store;
        match self {
            Special::Videos => store.video_items(),
            Special::Trash => store.trashed_items(),
            Special::Archive => store.archived_items(),
        }
    }
}

/// The album page that is open, with its photos newest first.
struct OpenAlbum {
    id: String,
    /// Set for the extra pages (Videos, Papierkorb); `id` is then synthetic.
    special: Option<Special>,
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
    /// Count and newest asset of the extra pages, in `Special::ALL` order.
    specials: Vec<(usize, Option<Cover>)>,
    /// Bumped on every open of the Papierkorb, so a late server answer for an earlier open is dropped.
    trash_serial: u32,
}

impl AlbumsState {
    /// Scroll handle of the open album page's photo list.
    pub(super) fn open_scroll(&self) -> Option<&UniformListScrollHandle> {
        self.open.as_ref().map(|o| &o.scroll)
    }

    pub(super) fn album_ids(&self) -> Vec<String> {
        self.albums.iter().map(|a| a.id.clone()).collect()
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
                // A cover that is archived or hidden by the "show from" date falls back to the newest shown photo.
                let cover = match album.album_thumbnail_asset_id.as_deref().filter(|id| !id.is_empty() && !store.is_hidden(id) && !store.is_archived(id)) {
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
        self.specials = Special::ALL
            .iter()
            .map(|special| {
                let items = special.items(store);
                let cover = items.first().map(|item| Cover { asset_id: item.id.clone(), thumbhash: item.thumbhash.clone() });
                (items.len(), cover)
            })
            .collect();
        if let Some(open) = self.open.as_mut() {
            open.items = Rc::new(match open.special {
                Some(special) => special.items(store),
                None => newest_first(store.album_assets(&open.id)),
            });
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

/// The Archiviert entry: an empty tile-sized slot (nothing painted, no pressed state), tappable through the
/// caller's `on_click`.
fn hidden_slot(i: usize, tile: f32) -> Stateful<Div> {
    div().id(("album-special", i)).w(px(tile)).h(px(tile))
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
        self.albums.open = Some(OpenAlbum { id: id.to_string(), special: None, name, items, scroll: UniformListScrollHandle::new() });
        cx.notify();
        self.repair_albums(vec![id.to_string()], cx);
    }

    /// Opens Videos or Papierkorb: the store's current contents at once, refreshed on every sync.
    /// The Papierkorb is also asked from the server (see `load_trash`).
    fn open_special(&mut self, special: Special, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.as_ref() else { return };
        let items = Rc::new(special.items(engine.store()));
        self.albums.open = Some(OpenAlbum {
            id: format!("special:{}", special.title()),
            special: Some(special),
            name: special.title().to_string(),
            items,
            scroll: UniformListScrollHandle::new(),
        });
        cx.notify();
        if special == Special::Trash {
            self.load_trash(cx);
        }
    }

    /// The store may lack trashed assets (a full sync lists with `withDeleted: false`; only the sync stream delivers
    /// them). Lists the server's trash and adds the assets the store does not know, marked trashed. Additive: what
    /// the store already has (e.g. a photo restored locally whose op is still queued) is left alone.
    fn load_trash(&mut self, cx: &mut Context<Self>) {
        self.albums.trash_serial += 1;
        let serial = self.albums.trash_serial;
        let (origin, token) = (self.origin.clone(), self.token.clone());
        cx.spawn(async move |this, cx| {
            let Ok(client) = ImmichClient::new(&origin, H::transport()) else { return };
            let mut found = Vec::new();
            for page in 1..=100u32 {
                let Ok(answer) = client.trashed_assets(page, 1000, &token).await else { break };
                let got = answer.assets.items.len();
                found.extend(answer.assets.items);
                if answer.assets.next_page.is_none() || got == 0 {
                    break;
                }
            }
            let _ = this.update(cx, |t, cx| {
                use crate::logic::store::Store;
                if t.albums.trash_serial != serial || found.is_empty() {
                    return;
                }
                let Some(engine) = t.engine.as_mut() else { return };
                let store = engine.store_mut();
                let missing: Vec<_> = found.into_iter().filter(|a| store.asset(&a.id).is_none()).collect();
                if missing.is_empty() {
                    return;
                }
                let ids: Vec<String> = missing.iter().map(|a| a.id.clone()).collect();
                store.upsert_assets(&missing);
                store.trash_assets(&ids);
                t.albums.refresh(engine.store());
                cx.notify();
            });
        })
        .detach();
    }

    /// Self-heal for a store whose album links are missing (a client that synced before link events were handled
    /// never gets them again: its stream checkpoint is past them). Lists the albums on the server and adds the links
    /// the store lacks; additive, so links of archived or trashed photos stay. Refreshes the view once at the end.
    pub(super) fn repair_albums(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        let (origin, token) = (self.origin.clone(), self.token.clone());
        cx.spawn(async move |this, cx| {
            let Ok(client) = ImmichClient::new(&origin, H::transport()) else { return };
            let mut changed = false;
            for id in ids {
                let mut listed: Vec<String> = Vec::new();
                let mut complete = true;
                for page in 1..=100u32 {
                    let Ok(answer) = client.album_assets(&id, page, 1000, &token).await else {
                        complete = false;
                        break;
                    };
                    let got = answer.assets.items.len();
                    listed.extend(answer.assets.items.into_iter().map(|a| a.id));
                    if answer.assets.next_page.is_none() || got == 0 {
                        break;
                    }
                }
                if !complete && listed.is_empty() {
                    continue;
                }
                let Ok(grew) = this.update(cx, |t, _| {
                    use crate::logic::store::Store;
                    let Some(engine) = t.engine.as_mut() else { return false };
                    let before = engine.store().album_visible_counts().get(&id).copied().unwrap_or(0);
                    engine.store_mut().add_album_assets(&id, &listed);
                    engine.store().album_visible_counts().get(&id).copied().unwrap_or(0) != before
                }) else {
                    return;
                };
                changed |= grew;
            }
            if !changed {
                return;
            }
            let _ = this.update(cx, |t, cx| {
                let Some(engine) = t.engine.as_ref() else { return };
                t.albums.refresh(engine.store());
                let snapshot = engine.store().snapshot().ok();
                cx.notify();
                if let Some(bytes) = snapshot {
                    cx.spawn(async move |_, _| {
                        let _ = H::blob_put(super::DB, super::SNAPSHOT_KEY, &bytes).await;
                    })
                    .detach();
                }
            });
        })
        .detach();
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
        let subtitle = match open.special {
            Some(Special::Videos) => count_label(open.items.len(), "Video", "Videos"),
            Some(special) if open.items.is_empty() => special.empty().to_string(),
            _ => count_label(open.items.len(), "Foto", "Fotos"),
        };
        let back = div().flex().flex_row().child(crate::components::icon::back_chip(p.glass_tint, "album-back", rgb(BLUE).into()).on_click(cx.listener(|this, _, _, cx| this.close_album(cx))));
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

        // Videos, Papierkorb and the hidden Archiviert slot below the albums, set apart by a rule.
        let mut extras: Vec<AnyElement> = Vec::new();
        if st.path.is_empty() {
            for (i, special) in Special::ALL.iter().copied().enumerate() {
                if special == Special::Archive {
                    // Invisible but real: an empty slot with a click handler and no feedback, no cover requested.
                    extras.push(
                        hidden_slot(i, tile)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if !window.tap_caught_fling() {
                                    this.open_special(special, cx)
                                }
                            }))
                            .into_any_element(),
                    );
                    continue;
                }
                let (count, cover) = st.specials.get(i).map(|(n, c)| (*n, c.as_ref())).unwrap_or((0, None));
                let picture = self.cover_image(cover, &mut tally, cx);
                let cell = div()
                    .size(px(tile))
                    .rounded(px(16.))
                    .overflow_hidden()
                    .bg(p.chip)
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(special.glyph().filter(|_| picture.is_none()).map(|g| crate::components::icon::glyph(g, 40., p.secondary)))
                    .children(picture.map(|image| img(image).size_full().object_fit(ObjectFit::Cover)));
                let (one, many) = if special == Special::Videos { ("Video", "Videos") } else { ("Foto", "Fotos") };
                extras.push(
                    div()
                        .id(("album-special", i))
                        .w(px(tile))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .active(|s| s.opacity(0.6))
                        .child(cell)
                        .child(caption(p, special.title(), count_label(count, one, many)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if !window.tap_caught_fling() {
                                this.open_special(special, cx)
                            }
                        }))
                        .into_any_element(),
                );
            }
        }

        // Breadcrumb and back chip inside nested folders.
        let mut header = div().px(px(PAD)).pt(px(20.)).pb_2().flex().flex_col().gap_1();
        let title = if st.path.is_empty() {
            "Alben".to_string()
        } else {
            let parent = AlbumPath::parent_of(&st.path);
            header = header.child(
                div().flex().flex_row().child(
                    crate::components::icon::back_chip(p.glass_tint, "albums-up", rgb(BLUE).into()).on_click(cx.listener(move |this, _, _, cx| this.go_to_folder(parent.clone(), cx))),
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
                    .child(self.jobs_button(p, cx))
                    .child(
                        glass_div(p.glass_tint, 18.)
                            .id("albums-settings")
                            .size(px(36.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .active(|s| s.opacity(0.6))
                            .child(crate::components::icon::glyph(crate::components::icon::Glyph::Gear, 20., p.label))
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
        let extras = (!extras.is_empty()).then(|| {
            div()
                .px(px(PAD))
                .pt(px(28.))
                .flex()
                .flex_col()
                .gap_3()
                .child(div().h(px(1.)).bg(p.chip))
                .child(div().flex().flex_wrap().gap(px(GAP)).children(extras))
        });

        self.rubber
            .shift(div().size_full().flex().flex_col().pt(px(self.top.max(12.))).child(header).child(
                div()
                    .id("albums-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.albums_scroll)
                    .child(body)
                    .children(extras)
                    .child(div().h(px(self.bottom.max(10.) + 84.))),
            ))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::gpui::TestAppContext;
    use std::cell::Cell;

    struct Probe(Rc<Cell<u32>>);

    impl Render for Probe {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let taps = self.0.clone();
            div().size_full().child(hidden_slot(2, 120.).on_click(move |_, _, _| taps.set(taps.get() + 1)))
        }
    }

    /// The invisible tile has no children and paints nothing, yet a click on its area arrives. (A test of
    /// `album_browser` itself needs a synced engine; this covers the element that carries the handler.)
    #[::core::prelude::v1::test]
    fn hidden_slot_is_tappable() {
        let mut cx = TestAppContext::single();
        let taps = Rc::new(Cell::new(0));
        let (_, cx) = cx.add_window_view({
            let taps = taps.clone();
            move |_, _| Probe(taps)
        });
        cx.simulate_click(point(px(60.), px(60.)), Modifiers::none());
        assert_eq!(taps.get(), 1);
        cx.simulate_click(point(px(300.), px(300.)), Modifiers::none());
        assert_eq!(taps.get(), 1, "outside the slot nothing happens");
    }
}
