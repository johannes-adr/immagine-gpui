//! Owner: A7. Swift source: `../immagine/Immagine/Database/Database.swift`.
//! Spec: `../immagine/ImmagineTests/` -> DatabaseTests, OfflineTests (store parts), FavoritesTests
//! (database parts), CoreTests (database parts).
//!
//! The local source of truth as a synchronous trait (`Store`) with one in-memory implementation
//! (`MemoryStore`). No SQL, no threads, no async. `MemoryStore` serializes as a whole, so a later
//! IndexedDB layer can persist `snapshot()` and feed it back through `restore()`.
//!
//! Differences from the GRDB original, on purpose:
//! - Migrations are not ported. A fresh `MemoryStore` starts in the state a fresh database reaches
//!   after migration v3 (`needs_full_sync = 1`).
//! - Clock-dependent writes take `now` (seconds since 1970) from the caller instead of `Date()`.
//! - Capture times are compared as epoch milliseconds from `iso_date`. Rows without a capture date
//!   sort as 0, which surfaces as `sort_date: None`, as in Swift.
//! - The `json` blob and the `status` column of `offline_file` are not stored. Every write path in
//!   Database.swift writes status `done`, so each stored row counts as done.
//! - Not ported here: persons (no table in Database.swift), the mini-bitmap file store (only its
//!   slot table is here), the `AlbumOutbox.apply` logic (sync layer; the tests use a local stand-in),
//!   `OfflineManager`, `SortTarget`.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::iso_date;
use super::models::album::Album;
use super::models::album_op::AlbumOp;
use super::models::asset::Asset;
use super::models::asset_info::AssetInfo;
use super::models::sync::{SyncAlbum, SyncAsset};
use super::models::timeline::TimelineItem;

// MARK: - Store-owned row types (Swift: Models/Local.swift)

/// A photo of an album kept on this device: what its original download needs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OfflineMember {
    pub album_id: String,
    pub asset_id: String,
    pub is_video: bool,
    pub original_file_name: Option<String>,
    pub mime_type: Option<String>,
    /// Capture time in seconds since 1970 (0 when unknown).
    pub sort_date: f64,
}

/// A local Photos-library asset tracked for upload.
/// `upload_status`: new, uploading, uploaded, failed, excluded, offloaded (and others set by callers).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalAsset {
    pub photokit_id: String,
    pub checksum: Option<String>,
    pub modified_at: Option<f64>,
    pub server_asset_id: Option<String>,
    pub upload_status: String,
    pub has_edits: bool,
    pub attempts: u32,
    pub last_error: Option<String>,
    pub next_attempt_at: Option<f64>,
    pub task_id: Option<i64>,
    pub body_path: Option<String>,
    /// Seconds since 1970 of the last upload-state change.
    pub updated_at: Option<f64>,
}

impl LocalAsset {
    fn new(photokit_id: &str, status: &str) -> Self {
        LocalAsset {
            photokit_id: photokit_id.to_string(),
            checksum: None,
            modified_at: None,
            server_asset_id: None,
            upload_status: status.to_string(),
            has_edits: false,
            attempts: 0,
            last_error: None,
            next_attempt_at: None,
            task_id: None,
            body_path: None,
            updated_at: None,
        }
    }
}

// MARK: - The trait

/// Local data operations used by the sync engine and the UI. Writes take `&mut self`, reads `&self`.
/// Every method is infallible: the in-memory implementation cannot fail, and a persistent layer
/// persists snapshots outside this trait.
pub trait Store {
    // Assets

    /// Full upsert (initial sync, detail fetch). Returns the ids whose rendered image changed
    /// (thumbhash or dimensions); new assets are not changes. Keeps the cached `file_size` unless
    /// the asset carries one.
    fn upsert_assets(&mut self, assets: &[Asset]) -> BTreeSet<String>;
    /// Partial upsert from sync-stream payloads. Never touches `original_path`, `original_mime_type`,
    /// `is_archived`, `is_offline` or `file_size`.
    fn upsert_sync_assets(&mut self, assets: &[SyncAsset]) -> BTreeSet<String>;
    /// The asset rebuilt from the stored columns (`exif_info` is not stored).
    fn asset(&self, id: &str) -> Option<Asset>;
    fn asset_info(&self, id: &str) -> Option<AssetInfo>;
    /// Non-trashed assets.
    fn asset_count(&self) -> usize;
    fn asset_ids(&self) -> BTreeSet<String>;
    fn thumbhash(&self, id: &str) -> Option<String>;
    fn edited_asset_ids(&self) -> BTreeSet<String>;
    fn set_file_size(&mut self, id: &str, bytes: i64);
    /// Optimistic state right after a server-side edit (dimensions, edited flag).
    fn apply_local_edit(&mut self, id: &str, width: i64, height: i64, edited: bool);
    fn set_favorite(&mut self, ids: &[String], favorite: bool);
    fn is_favorite(&self, id: &str) -> bool;
    fn trash_assets(&mut self, ids: &[String]);
    fn untrash_assets(&mut self, ids: &[String]);
    /// Removes the assets and their album links. Album counts are not recounted (as in Swift).
    fn delete_assets(&mut self, ids: &[String]);
    /// The subset of `ids` that belongs to at least one album.
    fn asset_ids_in_any_album(&self, ids: &[String]) -> BTreeSet<String>;
    /// Assets without a mini bitmap slot, newest first, not trashed.
    fn assets_without_mini_bitmap(&self, limit: usize) -> Vec<String>;

    // Timeline and favorites

    /// The main timeline: newest first, no trash, no archive, no hidden Live Photo parts.
    fn fetch_timeline(&self) -> Vec<TimelineItem>;
    /// Timeline rows of the given assets, newest first (e.g. a person's photos).
    fn timeline_items(&self, ids: &[String]) -> Vec<TimelineItem>;
    /// Favorites, shown like an album (archived ones too), newest first.
    fn favorite_items(&self) -> Vec<TimelineItem>;
    /// Count and newest favorite (the cover).
    fn favorite_summary(&self) -> (usize, Option<String>);

    // Albums

    fn upsert_albums(&mut self, albums: &[Album]);
    fn upsert_sync_album(&mut self, album: &SyncAlbum);
    /// Removes the album and its links. Its offline flag goes with it.
    fn delete_album(&mut self, id: &str);
    /// Albums by name (case-insensitive). The count is what the album shows; the stored count
    /// stands in only while no links are known.
    fn albums(&self) -> Vec<Album>;
    /// Per album with links: the photos it shows.
    fn album_visible_counts(&self) -> BTreeMap<String, usize>;
    /// Album photos, newest first, trash and hidden parts left out.
    fn album_assets(&self, album_id: &str) -> Vec<TimelineItem>;
    fn album_id_named(&self, name: &str) -> Option<String>;
    fn album_ids_containing(&self, asset_id: &str) -> BTreeSet<String>;
    fn add_album_assets(&mut self, album_id: &str, asset_ids: &[String]);
    fn remove_album_assets(&mut self, album_id: &str, asset_ids: &[String]);
    /// Replaces the album's links with `asset_ids`.
    fn set_album_assets(&mut self, album_id: &str, asset_ids: &[String]);
    /// Brings the links in line with the server listing; trashed photos keep their link.
    /// Returns whether anything changed.
    fn merge_album_assets(&mut self, album_id: &str, server_asset_ids: &[String]) -> bool;
    /// Sets every album's count from its links.
    fn refresh_album_counts(&mut self);
    /// Local flag, untouched by upserts.
    fn set_album_offline(&mut self, id: &str, offline: bool);
    fn offline_album_ids(&self) -> BTreeSet<String>;
    fn rename_album_locally(&mut self, id: &str, name: &str);
    fn set_album_thumbnail_locally(&mut self, id: &str, asset_id: &str);

    // Local folders (folders without an album yet)

    fn local_folders(&self) -> Vec<String>;
    fn add_local_folder(&mut self, path: &str);
    /// Removes `path` and every folder below it.
    fn remove_local_folders_under(&mut self, path: &str);
    /// Moves `from` and its descendants to `to`.
    fn move_local_folders(&mut self, from: &str, to: &str);

    // Offline originals

    /// Asset id -> file name (last path component only; older builds stored absolute paths).
    fn offline_files(&self) -> BTreeMap<String, String>;
    fn set_offline_file(&mut self, asset_id: &str, file_name: &str, size: i64);
    fn remove_offline_files(&mut self, asset_ids: &[String]);
    /// Every photo the offline albums show. A photo in two offline albums appears twice.
    fn offline_members(&self) -> Vec<OfflineMember>;

    // Local (device) assets

    fn local_asset(&self, photokit_id: &str) -> Option<LocalAsset>;
    fn local_assets_by_id(&self) -> BTreeMap<String, LocalAsset>;
    /// `needing_upload = true`: everything not yet uploaded. `false`: all.
    fn local_assets(&self, needing_upload: bool) -> Vec<LocalAsset>;
    fn local_assets_with_status(&self, status: &str) -> Vec<LocalAsset>;
    /// Uploaded photos whose server asset is alive: safe to remove from the device.
    fn offloadable_local_assets(&self) -> Vec<LocalAsset>;
    /// Re-scan bookkeeping. Keeps the upload status of existing rows.
    fn upsert_local_asset(&mut self, photokit_id: &str, checksum: Option<&str>, modified_at: f64, has_edits: bool);
    fn remove_local_asset(&mut self, photokit_id: &str);
    fn mark_local_asset_uploaded(&mut self, photokit_id: &str, server_asset_id: &str);
    fn set_local_asset_status(&mut self, photokit_ids: &[String], status: &str);
    /// Local photos whose server copy was deleted in the app: never upload them again.
    fn exclude_local_assets(&mut self, server_asset_ids: &[String]);
    /// Restored on the server: excluded photos count as uploaded again.
    fn include_local_assets(&mut self, server_asset_ids: &[String]);

    // Upload pipeline. `now` is seconds since 1970.

    /// A background-session task now carries the photo.
    fn mark_uploading(&mut self, photokit_id: &str, task_id: i64, body_path: &str, checksum: &str, modified_at: f64, has_edits: bool, now: f64);
    /// Uploaded, or found on the server: clears all retry state.
    fn mark_uploaded(
        &mut self,
        photokit_id: &str,
        server_asset_id: &str,
        checksum: Option<&str>,
        modified_at: Option<f64>,
        has_edits: Option<bool>,
        now: f64,
    );
    /// A failed attempt. `body_path` stays when the body can be resent as it is.
    fn mark_upload_failed(&mut self, photokit_id: &str, attempts: u32, next_attempt_at: Option<f64>, error: Option<&str>, body_path: Option<&str>, now: f64);
    /// Back to the queue without counting an attempt.
    fn requeue_upload(&mut self, photokit_id: &str, body_path: Option<&str>, now: f64);
    /// Body files still referenced.
    fn upload_body_paths(&self) -> BTreeSet<String>;
    fn upload_status_counts(&self) -> BTreeMap<String, usize>;
    /// Failed uploads with at least `min_attempts` attempts, most recently failed first.
    fn failed_uploads(&self, min_attempts: u32) -> Vec<LocalAsset>;
    /// Gives failed uploads a fresh start.
    fn reset_failed_uploads(&mut self);

    // Inbox queue (server assets still to be added to the inbox album)

    fn enqueue_inbox(&mut self, asset_ids: &[String], now: f64);
    /// Oldest first.
    fn inbox_queue(&self) -> Vec<String>;
    fn remove_from_inbox_queue(&mut self, asset_ids: &[String]);

    // Album outbox (changes not yet confirmed by the server)

    /// Stores the op and returns its queue id.
    fn enqueue_album_op(&mut self, op: &AlbumOp, now: f64) -> i64;
    /// Oldest first, with `id` and `attempts` set.
    fn pending_album_ops(&self) -> Vec<AlbumOp>;
    fn delete_album_op(&mut self, id: i64);
    fn mark_album_op_failed(&mut self, id: i64, error: &str);

    // Mini bitmap slots (the pixels live in a file store outside this trait)

    fn mini_bitmap_slots(&self) -> BTreeMap<String, i64>;
    fn mini_bitmap_slot(&self, asset_id: &str) -> Option<i64>;
    /// Write-once: an existing slot is kept.
    fn insert_mini_bitmap(&mut self, asset_id: &str, slot: i64);
    /// A never-used slot. Slots of deleted bitmaps are never handed out again.
    fn allocate_mini_bitmap_slot(&mut self) -> i64;
    fn delete_mini_bitmaps(&mut self, asset_ids: &[String]);
    fn mini_bitmap_count(&self) -> usize;

    // Meta (sync checkpoints and flags)

    fn meta(&self, key: &str) -> Option<String>;
    fn set_meta(&mut self, key: &str, value: &str);
}

// MARK: - In-memory implementation

const NEEDS_FULL_SYNC: &str = "needs_full_sync";
const MINI_NEXT_SLOT: &str = "mini_next_slot";

/// Row of the `asset` table.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct AssetRow {
    asset_type: Option<String>,
    original_path: Option<String>,
    original_file_name: Option<String>,
    original_mime_type: Option<String>,
    thumbhash: Option<String>,
    checksum: Option<String>,
    width: Option<i64>,
    height: Option<i64>,
    file_created_at: Option<String>,
    local_date_time: Option<String>,
    updated_at: Option<String>,
    /// Raw ISO string of the capture instant.
    capture_date: Option<String>,
    /// Epoch milliseconds of the capture instant, 0 when unknown (the sort key).
    sort_millis: i64,
    date_bucket: Option<String>,
    is_favorite: bool,
    is_archived: bool,
    is_trashed: bool,
    visibility: Option<String>,
    duration: Option<String>,
    live_photo_video_id: Option<String>,
    is_offline: bool,
    is_edited: bool,
    file_size: Option<i64>,
}

impl AssetRow {
    /// `is_trashed = 0 AND (visibility IS NULL OR visibility = 'timeline')`
    fn in_timeline(&self) -> bool {
        !self.is_trashed && matches!(self.visibility.as_deref(), None | Some("timeline"))
    }

    /// `is_trashed = 0 AND (visibility IS NULL OR visibility IN ('timeline', 'archive'))`
    fn in_albums(&self) -> bool {
        !self.is_trashed && matches!(self.visibility.as_deref(), None | Some("timeline") | Some("archive"))
    }

    fn visual(&self) -> VisualKey {
        VisualKey { thumbhash: self.thumbhash.clone(), width: self.width, height: self.height }
    }
}

/// What a cached thumbnail depends on.
#[derive(Debug, Clone, PartialEq)]
struct VisualKey {
    thumbhash: Option<String>,
    width: Option<i64>,
    height: Option<i64>,
}

/// Row of the `album` table. Links live in `MemoryStore::album_links`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct AlbumRow {
    album_name: String,
    description: Option<String>,
    thumbnail_asset_id: Option<String>,
    asset_count: i64,
    created_at: Option<String>,
    updated_at: Option<String>,
    is_offline: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct OfflineFile {
    path: String,
    size: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct StoredAlbumOp {
    op: AlbumOp,
    created_at: f64,
    last_error: Option<String>,
}

/// Synchronous in-memory `Store`. Base tables serialize as a whole; the two indexes
/// (`timeline_index`, `asset_albums`) are derived and rebuilt on `restore`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MemoryStore {
    assets: BTreeMap<String, AssetRow>,
    albums: BTreeMap<String, AlbumRow>,
    /// Album id -> linked asset ids (`album_asset`). Never holds an empty set.
    album_links: BTreeMap<String, BTreeSet<String>>,
    local_folders: BTreeSet<String>,
    offline_files: BTreeMap<String, OfflineFile>,
    local_assets: BTreeMap<String, LocalAsset>,
    /// Asset id -> queued_at (seconds).
    inbox: BTreeMap<String, f64>,
    album_ops: BTreeMap<i64, StoredAlbumOp>,
    next_album_op_id: i64,
    mini_bitmaps: BTreeMap<String, i64>,
    meta: BTreeMap<String, String>,

    /// Derived: every asset ordered by capture date descending, then id ascending.
    #[serde(skip)]
    timeline_index: BTreeSet<(Reverse<i64>, String)>,
    /// Derived: asset id -> albums that link it.
    #[serde(skip)]
    asset_albums: BTreeMap<String, BTreeSet<String>>,
    /// Display filter ("show photos from"): epoch ms, 0 = off. Rows captured earlier stay in the tables
    /// (sync keeps working, the date can be lowered again without a resync) but no query returns them.
    /// Per device, set from a setting, not part of the snapshot.
    #[serde(skip)]
    hidden_before: i64,
}

fn ratio_of(width: Option<i64>, height: Option<i64>) -> Option<f64> {
    match (width, height) {
        (Some(w), Some(h)) if h != 0 => Some(w as f64 / h as f64),
        _ => None,
    }
}

fn millis_of(capture: Option<&str>) -> i64 {
    capture.and_then(iso_date::parse).unwrap_or(0)
}

fn timeline_item(id: &str, row: &AssetRow) -> TimelineItem {
    TimelineItem {
        id: id.to_string(),
        asset_type: row.asset_type.clone().unwrap_or_default(),
        thumbhash: row.thumbhash.clone(),
        ratio: ratio_of(row.width, row.height),
        is_favorite: row.is_favorite,
        is_trashed: row.is_trashed,
        date_bucket: row.date_bucket.clone().unwrap_or_default(),
        // sort_date 0 means "no capture date" and must not surface as 1 January 1970.
        sort_date: if row.sort_millis == 0 { None } else { row.capture_date.clone() },
        duration: row.duration.clone(),
    }
}

impl MemoryStore {
    /// An empty store in the state a fresh database has after migration v3.
    pub fn new() -> Self {
        let mut store = Self::default();
        store.meta.insert(NEEDS_FULL_SYNC.to_string(), "1".to_string());
        store
    }

    /// The whole store as JSON bytes.
    pub fn snapshot(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// Rebuilds a store from `snapshot` bytes, including its derived indexes.
    pub fn restore(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        let mut store: MemoryStore = serde_json::from_slice(bytes)?;
        store.rebuild_indexes();
        Ok(store)
    }

    fn rebuild_indexes(&mut self) {
        self.timeline_index = self.assets.iter().map(|(id, row)| (Reverse(row.sort_millis), id.clone())).collect();
        self.asset_albums.clear();
        for (album, assets) in &self.album_links {
            for asset in assets {
                self.asset_albums.entry(asset.clone()).or_default().insert(album.clone());
            }
        }
    }

    fn put_asset(&mut self, id: &str, row: AssetRow) {
        if let Some(old) = self.assets.get(id) {
            self.timeline_index.remove(&(Reverse(old.sort_millis), id.to_string()));
        }
        self.timeline_index.insert((Reverse(row.sort_millis), id.to_string()));
        self.assets.insert(id.to_string(), row);
    }

    fn drop_asset(&mut self, id: &str) {
        if let Some(old) = self.assets.remove(id) {
            self.timeline_index.remove(&(Reverse(old.sort_millis), id.to_string()));
        }
    }

    /// Adds a link. Returns whether it was new.
    fn link(&mut self, album: &str, asset: &str) -> bool {
        let inserted = self.album_links.entry(album.to_string()).or_default().insert(asset.to_string());
        if inserted {
            self.asset_albums.entry(asset.to_string()).or_default().insert(album.to_string());
        }
        inserted
    }

    /// Removes a link. Returns whether it existed.
    fn unlink(&mut self, album: &str, asset: &str) -> bool {
        let Some(links) = self.album_links.get_mut(album) else {
            return false;
        };
        let removed = links.remove(asset);
        if links.is_empty() {
            self.album_links.remove(album);
        }
        if removed {
            if let Some(albums) = self.asset_albums.get_mut(asset) {
                albums.remove(album);
                if albums.is_empty() {
                    self.asset_albums.remove(asset);
                }
            }
        }
        removed
    }

    /// `album.asset_count` follows the links.
    fn recount(&mut self, album: &str) {
        let count = self.album_links.get(album).map_or(0, |links| links.len()) as i64;
        if let Some(row) = self.albums.get_mut(album) {
            row.asset_count = count;
        }
    }

    /// Sets the display cutoff (epoch ms of the first shown instant, 0 = show everything).
    /// Cheap: queries read it, nothing is rebuilt here; callers rebuild their derived lists once.
    pub fn set_hidden_before(&mut self, cutoff_millis: i64) {
        self.hidden_before = cutoff_millis.max(0);
    }

    pub fn hidden_before(&self) -> i64 {
        self.hidden_before
    }

    /// Whether the asset exists and is hidden by the date setting.
    pub fn is_hidden(&self, id: &str) -> bool {
        self.assets.get(id).is_some_and(|row| self.date_hidden(row))
    }

    /// Photos the date setting hides (those that would otherwise show in an album or the timeline).
    pub fn hidden_count(&self) -> usize {
        if self.hidden_before <= 0 {
            return 0;
        }
        self.assets.values().filter(|row| row.in_albums() && self.date_hidden(row)).count()
    }

    fn date_hidden(&self, row: &AssetRow) -> bool {
        super::hidden_before::is_hidden(row.sort_millis, self.hidden_before)
    }

    /// `AssetRow::in_timeline` and not hidden by date.
    fn shows_in_timeline(&self, row: &AssetRow) -> bool {
        row.in_timeline() && !self.date_hidden(row)
    }

    /// `AssetRow::in_albums` and not hidden by date.
    fn shows_in_albums(&self, row: &AssetRow) -> bool {
        row.in_albums() && !self.date_hidden(row)
    }

    /// Links whose asset exists and is shown in an album.
    fn visible_count(&self, links: &BTreeSet<String>) -> usize {
        links.iter().filter(|id| self.assets.get(id.as_str()).is_some_and(|row| self.shows_in_albums(row))).count()
    }

    /// Rows for `ids`, in the timeline order (capture date descending, id ascending), filtered by `keep`.
    fn sorted_items<'a>(&self, ids: impl IntoIterator<Item = &'a str>, keep: impl Fn(&AssetRow) -> bool) -> Vec<TimelineItem> {
        let mut found: Vec<(i64, &str, &AssetRow)> = ids
            .into_iter()
            .filter_map(|id| {
                let (key, row) = self.assets.get_key_value(id)?;
                keep(row).then(|| (row.sort_millis, key.as_str(), row))
            })
            .collect();
        found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
        found.into_iter().map(|(_, id, row)| timeline_item(id, row)).collect()
    }

    fn visual_keys<'a>(&self, ids: impl Iterator<Item = &'a str>) -> BTreeMap<String, VisualKey> {
        ids.filter_map(|id| self.assets.get(id).map(|row| (id.to_string(), row.visual()))).collect()
    }
}

impl Store for MemoryStore {
    // MARK: Assets

    fn upsert_assets(&mut self, assets: &[Asset]) -> BTreeSet<String> {
        let before = self.visual_keys(assets.iter().map(|a| a.id.as_str()));
        let mut changed = BTreeSet::new();
        for a in assets {
            let capture = a.capture_date().map(str::to_owned);
            let mut row = self.assets.get(&a.id).cloned().unwrap_or_default();
            row.asset_type = a.asset_type.clone();
            row.original_path = a.original_path.clone();
            row.original_file_name = a.original_file_name.clone();
            row.original_mime_type = a.original_mime_type.clone();
            row.thumbhash = a.thumbhash.clone();
            row.checksum = a.checksum.clone();
            row.width = a.width;
            row.height = a.height;
            row.file_created_at = a.file_created_at.clone();
            row.local_date_time = a.local_date_time.clone();
            row.updated_at = a.updated_at.clone();
            row.sort_millis = millis_of(capture.as_deref());
            row.capture_date = capture;
            row.date_bucket = a.date_bucket();
            row.is_favorite = a.is_favorite;
            row.is_archived = a.is_archived;
            row.is_trashed = a.is_trashed;
            row.visibility = a.visibility.clone();
            row.duration = a.duration.clone();
            row.live_photo_video_id = a.live_photo_video_id.clone();
            row.is_offline = a.is_offline;
            row.is_edited = a.is_edited;
            row.file_size = a.exif_info.as_ref().and_then(|e| e.file_size_in_byte).or(row.file_size);

            let new_key = VisualKey { thumbhash: a.thumbhash.clone(), width: a.width, height: a.height };
            if before.get(&a.id).is_some_and(|old| *old != new_key) {
                changed.insert(a.id.clone());
            }
            self.put_asset(&a.id, row);
        }
        changed
    }

    fn upsert_sync_assets(&mut self, assets: &[SyncAsset]) -> BTreeSet<String> {
        let before = self.visual_keys(assets.iter().map(|a| a.id.as_str()));
        let mut changed = BTreeSet::new();
        for a in assets {
            let capture = a.capture_date().map(str::to_owned);
            let mut row = self.assets.get(&a.id).cloned().unwrap_or_default();
            row.asset_type = a.asset_type.clone();
            row.thumbhash = a.thumbhash.clone();
            row.checksum = a.checksum.clone();
            row.width = a.width;
            row.height = a.height;
            row.file_created_at = a.file_created_at.clone();
            row.local_date_time = a.local_date_time.clone();
            // The sync stream's createdAt is what the `updated_at` column has always held.
            row.updated_at = a.created_at.clone();
            row.sort_millis = millis_of(capture.as_deref());
            row.capture_date = capture;
            row.date_bucket = a.date_bucket();
            row.is_favorite = a.is_favorite.unwrap_or(false);
            row.is_trashed = a.deleted_at.is_some();
            row.duration = a.duration.clone();
            row.original_file_name = a.original_file_name.clone();
            row.is_edited = a.is_edited.unwrap_or(false);
            row.visibility = a.visibility.clone();
            row.live_photo_video_id = a.live_photo_video_id.clone();

            let new_key = VisualKey { thumbhash: a.thumbhash.clone(), width: a.width, height: a.height };
            if before.get(&a.id).is_some_and(|old| *old != new_key) {
                changed.insert(a.id.clone());
            }
            self.put_asset(&a.id, row);
        }
        changed
    }

    fn asset(&self, id: &str) -> Option<Asset> {
        let row = self.assets.get(id)?;
        Some(Asset {
            id: id.to_string(),
            asset_type: row.asset_type.clone(),
            original_path: row.original_path.clone(),
            original_file_name: row.original_file_name.clone(),
            original_mime_type: row.original_mime_type.clone(),
            thumbhash: row.thumbhash.clone(),
            file_created_at: row.file_created_at.clone(),
            file_modified_at: None,
            local_date_time: row.local_date_time.clone(),
            created_at: None,
            updated_at: row.updated_at.clone(),
            is_favorite: row.is_favorite,
            is_archived: false,
            is_trashed: row.is_trashed,
            visibility: row.visibility.clone(),
            duration: row.duration.clone(),
            live_photo_video_id: row.live_photo_video_id.clone(),
            checksum: row.checksum.clone(),
            is_offline: row.is_offline,
            is_edited: row.is_edited,
            width: row.width,
            height: row.height,
            resized: None,
            has_metadata: None,
            owner_id: None,
            exif_info: None,
        })
    }

    fn asset_info(&self, id: &str) -> Option<AssetInfo> {
        let row = self.assets.get(id)?;
        Some(AssetInfo {
            id: id.to_string(),
            asset_type: row.asset_type.clone().unwrap_or_default(),
            file_name: row.original_file_name.clone(),
            mime_type: row.original_mime_type.clone(),
            width: row.width,
            height: row.height,
            file_size: row.file_size,
            is_favorite: row.is_favorite,
            is_edited: row.is_edited,
            live_photo_video_id: row.live_photo_video_id.clone(),
            capture_date: if row.sort_millis == 0 { None } else { row.capture_date.clone() },
            duration: row.duration.clone(),
        })
    }

    fn asset_count(&self) -> usize {
        self.assets.values().filter(|row| !row.is_trashed).count()
    }

    fn asset_ids(&self) -> BTreeSet<String> {
        self.assets.keys().cloned().collect()
    }

    fn thumbhash(&self, id: &str) -> Option<String> {
        self.assets.get(id)?.thumbhash.clone()
    }

    fn edited_asset_ids(&self) -> BTreeSet<String> {
        self.assets.iter().filter(|(_, row)| row.is_edited).map(|(id, _)| id.clone()).collect()
    }

    fn set_file_size(&mut self, id: &str, bytes: i64) {
        if let Some(row) = self.assets.get_mut(id) {
            row.file_size = Some(bytes);
        }
    }

    fn apply_local_edit(&mut self, id: &str, width: i64, height: i64, edited: bool) {
        if let Some(row) = self.assets.get_mut(id) {
            row.width = Some(width);
            row.height = Some(height);
            row.is_edited = edited;
        }
    }

    fn set_favorite(&mut self, ids: &[String], favorite: bool) {
        for id in ids {
            if let Some(row) = self.assets.get_mut(id.as_str()) {
                row.is_favorite = favorite;
            }
        }
    }

    fn is_favorite(&self, id: &str) -> bool {
        self.assets.get(id).is_some_and(|row| row.is_favorite)
    }

    fn trash_assets(&mut self, ids: &[String]) {
        for id in ids {
            if let Some(row) = self.assets.get_mut(id.as_str()) {
                row.is_trashed = true;
            }
        }
    }

    fn untrash_assets(&mut self, ids: &[String]) {
        for id in ids {
            if let Some(row) = self.assets.get_mut(id.as_str()) {
                row.is_trashed = false;
            }
        }
    }

    fn delete_assets(&mut self, ids: &[String]) {
        for id in ids {
            let albums: Vec<String> = self
                .asset_albums
                .get(id.as_str())
                .map(|albums| albums.iter().cloned().collect())
                .unwrap_or_default();
            for album in albums {
                self.unlink(&album, id);
            }
            self.drop_asset(id);
        }
    }

    fn asset_ids_in_any_album(&self, ids: &[String]) -> BTreeSet<String> {
        ids.iter().filter(|id| self.asset_albums.contains_key(id.as_str())).cloned().collect()
    }

    fn assets_without_mini_bitmap(&self, limit: usize) -> Vec<String> {
        self.timeline_index
            .iter()
            .filter(|(_, id)| {
                !self.mini_bitmaps.contains_key(id.as_str())
                    && self.assets.get(id.as_str()).is_some_and(|row| !row.is_trashed && !self.date_hidden(row))
            })
            .take(limit)
            .map(|(_, id)| id.clone())
            .collect()
    }

    // MARK: Timeline and favorites

    fn fetch_timeline(&self) -> Vec<TimelineItem> {
        self.timeline_index
            .iter()
            .filter_map(|(_, id)| {
                let row = self.assets.get(id.as_str())?;
                self.shows_in_timeline(row).then(|| timeline_item(id, row))
            })
            .collect()
    }

    fn timeline_items(&self, ids: &[String]) -> Vec<TimelineItem> {
        let unique: BTreeSet<&str> = ids.iter().map(String::as_str).collect();
        self.sorted_items(unique, |row| self.shows_in_albums(row))
    }

    fn favorite_items(&self) -> Vec<TimelineItem> {
        self.timeline_index
            .iter()
            .filter_map(|(_, id)| {
                let row = self.assets.get(id.as_str())?;
                (row.is_favorite && self.shows_in_albums(row)).then(|| timeline_item(id, row))
            })
            .collect()
    }

    fn favorite_summary(&self) -> (usize, Option<String>) {
        let count = self.assets.values().filter(|row| row.is_favorite && self.shows_in_albums(row)).count();
        let cover = self
            .timeline_index
            .iter()
            .find(|(_, id)| self.assets.get(id.as_str()).is_some_and(|row| row.is_favorite && self.shows_in_albums(row)))
            .map(|(_, id)| id.clone());
        (count, cover)
    }

    // MARK: Albums

    fn upsert_albums(&mut self, albums: &[Album]) {
        for a in albums {
            let mut row = self.albums.get(&a.id).cloned().unwrap_or_default();
            row.album_name = a.album_name.clone();
            row.description = a.description.clone();
            row.thumbnail_asset_id = a.album_thumbnail_asset_id.clone();
            row.created_at = a.created_at.clone();
            row.updated_at = a.updated_at.clone();
            // The server's count stands in only while no links are known.
            let links = self.album_links.get(&a.id).map_or(0, |links| links.len());
            row.asset_count = if links > 0 { links as i64 } else { a.asset_count };
            self.albums.insert(a.id.clone(), row);
        }
    }

    fn upsert_sync_album(&mut self, album: &SyncAlbum) {
        let mut row = self.albums.get(&album.id).cloned().unwrap_or_default();
        row.album_name = album.name.clone().unwrap_or_default();
        row.description = album.description.clone();
        row.thumbnail_asset_id = album.thumbnail_asset_id.clone();
        row.created_at = album.created_at.clone();
        row.updated_at = album.updated_at.clone();
        self.albums.insert(album.id.clone(), row);
    }

    fn delete_album(&mut self, id: &str) {
        if let Some(links) = self.album_links.remove(id) {
            for asset in &links {
                if let Some(albums) = self.asset_albums.get_mut(asset) {
                    albums.remove(id);
                    if albums.is_empty() {
                        self.asset_albums.remove(asset);
                    }
                }
            }
        }
        self.albums.remove(id);
    }

    fn albums(&self) -> Vec<Album> {
        let mut out: Vec<Album> = self
            .albums
            .iter()
            .map(|(id, row)| {
                let asset_count = match self.album_links.get(id) {
                    Some(links) => self.visible_count(links) as i64,
                    None => row.asset_count,
                };
                Album {
                    id: id.clone(),
                    album_name: row.album_name.clone(),
                    description: row.description.clone(),
                    album_thumbnail_asset_id: row.thumbnail_asset_id.clone(),
                    created_at: row.created_at.clone(),
                    updated_at: row.updated_at.clone(),
                    asset_count,
                    shared: false,
                    has_shared_link: false,
                    order: None,
                }
            })
            .collect();
        out.sort_by(|a, b| {
            a.album_name
                .to_ascii_lowercase()
                .cmp(&b.album_name.to_ascii_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        out
    }

    fn album_visible_counts(&self) -> BTreeMap<String, usize> {
        self.album_links.iter().map(|(id, links)| (id.clone(), self.visible_count(links))).collect()
    }

    fn album_assets(&self, album_id: &str) -> Vec<TimelineItem> {
        match self.album_links.get(album_id) {
            Some(links) => self.sorted_items(links.iter().map(String::as_str), |row| self.shows_in_albums(row)),
            None => Vec::new(),
        }
    }

    fn album_id_named(&self, name: &str) -> Option<String> {
        self.albums
            .iter()
            .filter(|(_, row)| row.album_name == name)
            .min_by(|a, b| a.1.created_at.cmp(&b.1.created_at).then_with(|| a.0.cmp(b.0)))
            .map(|(id, _)| id.clone())
    }

    fn album_ids_containing(&self, asset_id: &str) -> BTreeSet<String> {
        self.asset_albums.get(asset_id).cloned().unwrap_or_default()
    }

    fn add_album_assets(&mut self, album_id: &str, asset_ids: &[String]) {
        let mut inserted = false;
        for id in asset_ids {
            inserted |= self.link(album_id, id);
        }
        if inserted {
            self.recount(album_id);
        }
    }

    fn remove_album_assets(&mut self, album_id: &str, asset_ids: &[String]) {
        let mut removed = false;
        for id in asset_ids {
            removed |= self.unlink(album_id, id);
        }
        if removed {
            self.recount(album_id);
        }
    }

    fn set_album_assets(&mut self, album_id: &str, asset_ids: &[String]) {
        let old: Vec<String> = self.album_links.get(album_id).map(|links| links.iter().cloned().collect()).unwrap_or_default();
        for id in &old {
            self.unlink(album_id, id);
        }
        for id in asset_ids {
            self.link(album_id, id);
        }
        self.recount(album_id);
    }

    fn merge_album_assets(&mut self, album_id: &str, server_asset_ids: &[String]) -> bool {
        let listed: BTreeSet<&str> = server_asset_ids.iter().map(String::as_str).collect();
        let mut changed = 0;
        for id in &listed {
            if self.link(album_id, id) {
                changed += 1;
            }
        }
        // Trashed photos are not listed by the server but return with a restore, so they keep their link.
        let local: Vec<String> = self
            .album_links
            .get(album_id)
            .map(|links| {
                links
                    .iter()
                    .filter(|id| self.assets.get(id.as_str()).is_none_or(|row| !row.is_trashed))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        for id in &local {
            if !listed.contains(id.as_str()) && self.unlink(album_id, id) {
                changed += 1;
            }
        }
        if changed > 0 {
            self.recount(album_id);
        }
        changed > 0
    }

    fn refresh_album_counts(&mut self) {
        let ids: Vec<String> = self.albums.keys().cloned().collect();
        for id in ids {
            self.recount(&id);
        }
    }

    fn set_album_offline(&mut self, id: &str, offline: bool) {
        if let Some(row) = self.albums.get_mut(id) {
            row.is_offline = offline;
        }
    }

    fn offline_album_ids(&self) -> BTreeSet<String> {
        self.albums.iter().filter(|(_, row)| row.is_offline).map(|(id, _)| id.clone()).collect()
    }

    fn rename_album_locally(&mut self, id: &str, name: &str) {
        if let Some(row) = self.albums.get_mut(id) {
            row.album_name = name.to_string();
        }
    }

    fn set_album_thumbnail_locally(&mut self, id: &str, asset_id: &str) {
        if let Some(row) = self.albums.get_mut(id) {
            row.thumbnail_asset_id = Some(asset_id.to_string());
        }
    }

    // MARK: Local folders

    fn local_folders(&self) -> Vec<String> {
        self.local_folders.iter().cloned().collect()
    }

    fn add_local_folder(&mut self, path: &str) {
        self.local_folders.insert(path.to_string());
    }

    fn remove_local_folders_under(&mut self, path: &str) {
        let prefix = format!("{path}/");
        self.local_folders.retain(|p| p.as_str() != path && !p.starts_with(&prefix));
    }

    fn move_local_folders(&mut self, from: &str, to: &str) {
        let prefix = format!("{from}/");
        let moving: Vec<String> = self
            .local_folders
            .iter()
            .filter(|p| p.as_str() == from || p.starts_with(&prefix))
            .cloned()
            .collect();
        for path in moving {
            let moved = format!("{to}{}", &path[from.len()..]);
            self.local_folders.remove(&path);
            self.local_folders.insert(moved);
        }
    }

    // MARK: Offline originals

    fn offline_files(&self) -> BTreeMap<String, String> {
        self.offline_files
            .iter()
            .filter(|(_, file)| !file.path.is_empty())
            .map(|(id, file)| (id.clone(), file.path.rsplit('/').next().unwrap_or(file.path.as_str()).to_string()))
            .collect()
    }

    fn set_offline_file(&mut self, asset_id: &str, file_name: &str, size: i64) {
        self.offline_files.insert(asset_id.to_string(), OfflineFile { path: file_name.to_string(), size });
    }

    fn remove_offline_files(&mut self, asset_ids: &[String]) {
        for id in asset_ids {
            self.offline_files.remove(id.as_str());
        }
    }

    fn offline_members(&self) -> Vec<OfflineMember> {
        let mut members = Vec::new();
        for (album_id, album) in &self.albums {
            if !album.is_offline {
                continue;
            }
            let Some(links) = self.album_links.get(album_id) else {
                continue;
            };
            for asset_id in links {
                let Some(row) = self.assets.get(asset_id.as_str()) else {
                    continue;
                };
                if !self.shows_in_albums(row) {
                    continue;
                }
                members.push(OfflineMember {
                    album_id: album_id.clone(),
                    asset_id: asset_id.clone(),
                    is_video: row.asset_type.as_deref() == Some("VIDEO"),
                    original_file_name: row.original_file_name.clone(),
                    mime_type: row.original_mime_type.clone(),
                    sort_date: row.sort_millis as f64 / 1000.0,
                });
            }
        }
        members
    }

    // MARK: Local assets

    fn local_asset(&self, photokit_id: &str) -> Option<LocalAsset> {
        self.local_assets.get(photokit_id).cloned()
    }

    fn local_assets_by_id(&self) -> BTreeMap<String, LocalAsset> {
        self.local_assets.clone()
    }

    fn local_assets(&self, needing_upload: bool) -> Vec<LocalAsset> {
        self.local_assets
            .values()
            .filter(|l| !needing_upload || l.upload_status != "uploaded")
            .cloned()
            .collect()
    }

    fn local_assets_with_status(&self, status: &str) -> Vec<LocalAsset> {
        self.local_assets.values().filter(|l| l.upload_status == status).cloned().collect()
    }

    fn offloadable_local_assets(&self) -> Vec<LocalAsset> {
        self.local_assets
            .values()
            .filter(|l| {
                l.upload_status == "uploaded"
                    && l.server_asset_id
                        .as_deref()
                        .and_then(|s| self.assets.get(s))
                        .is_some_and(|row| !row.is_trashed)
            })
            .cloned()
            .collect()
    }

    fn upsert_local_asset(&mut self, photokit_id: &str, checksum: Option<&str>, modified_at: f64, has_edits: bool) {
        let row = self.local_assets.entry(photokit_id.to_string()).or_insert_with(|| LocalAsset::new(photokit_id, "new"));
        row.checksum = checksum.map(str::to_owned);
        row.modified_at = Some(modified_at);
        row.has_edits = has_edits;
    }

    fn remove_local_asset(&mut self, photokit_id: &str) {
        self.local_assets.remove(photokit_id);
    }

    fn mark_local_asset_uploaded(&mut self, photokit_id: &str, server_asset_id: &str) {
        if let Some(row) = self.local_assets.get_mut(photokit_id) {
            row.upload_status = "uploaded".to_string();
            row.server_asset_id = Some(server_asset_id.to_string());
        }
    }

    fn set_local_asset_status(&mut self, photokit_ids: &[String], status: &str) {
        for id in photokit_ids {
            if let Some(row) = self.local_assets.get_mut(id.as_str()) {
                row.upload_status = status.to_string();
            }
        }
    }

    fn exclude_local_assets(&mut self, server_asset_ids: &[String]) {
        let wanted: BTreeSet<&str> = server_asset_ids.iter().map(String::as_str).collect();
        for row in self.local_assets.values_mut() {
            if row.server_asset_id.as_deref().is_some_and(|s| wanted.contains(s)) {
                row.upload_status = "excluded".to_string();
            }
        }
    }

    fn include_local_assets(&mut self, server_asset_ids: &[String]) {
        let wanted: BTreeSet<&str> = server_asset_ids.iter().map(String::as_str).collect();
        for row in self.local_assets.values_mut() {
            if row.upload_status == "excluded" && row.server_asset_id.as_deref().is_some_and(|s| wanted.contains(s)) {
                row.upload_status = "uploaded".to_string();
            }
        }
    }

    // MARK: Upload pipeline

    fn mark_uploading(&mut self, photokit_id: &str, task_id: i64, body_path: &str, checksum: &str, modified_at: f64, has_edits: bool, now: f64) {
        let row = self.local_assets.entry(photokit_id.to_string()).or_insert_with(|| LocalAsset::new(photokit_id, "uploading"));
        row.checksum = Some(checksum.to_string());
        row.modified_at = Some(modified_at);
        row.has_edits = has_edits;
        row.upload_status = "uploading".to_string();
        row.task_id = Some(task_id);
        row.body_path = Some(body_path.to_string());
        row.updated_at = Some(now);
    }

    fn mark_uploaded(
        &mut self,
        photokit_id: &str,
        server_asset_id: &str,
        checksum: Option<&str>,
        modified_at: Option<f64>,
        has_edits: Option<bool>,
        now: f64,
    ) {
        let row = self.local_assets.entry(photokit_id.to_string()).or_insert_with(|| LocalAsset::new(photokit_id, "uploaded"));
        if let Some(c) = checksum {
            row.checksum = Some(c.to_string());
        }
        if let Some(m) = modified_at {
            row.modified_at = Some(m);
        }
        if let Some(h) = has_edits {
            row.has_edits = h;
        }
        row.server_asset_id = Some(server_asset_id.to_string());
        row.upload_status = "uploaded".to_string();
        row.attempts = 0;
        row.last_error = None;
        row.next_attempt_at = None;
        row.task_id = None;
        row.body_path = None;
        row.updated_at = Some(now);
    }

    fn mark_upload_failed(&mut self, photokit_id: &str, attempts: u32, next_attempt_at: Option<f64>, error: Option<&str>, body_path: Option<&str>, now: f64) {
        if let Some(row) = self.local_assets.get_mut(photokit_id) {
            row.upload_status = "failed".to_string();
            row.attempts = attempts;
            row.next_attempt_at = next_attempt_at;
            row.last_error = error.map(str::to_owned);
            row.task_id = None;
            row.body_path = body_path.map(str::to_owned);
            row.updated_at = Some(now);
        }
    }

    fn requeue_upload(&mut self, photokit_id: &str, body_path: Option<&str>, now: f64) {
        if let Some(row) = self.local_assets.get_mut(photokit_id) {
            if row.upload_status == "uploading" {
                let next_status = if row.server_asset_id.is_none() { "new" } else { "uploaded" };
                row.upload_status = next_status.to_string();
                row.task_id = None;
                row.body_path = body_path.map(str::to_owned);
                row.updated_at = Some(now);
            }
        }
    }

    fn upload_body_paths(&self) -> BTreeSet<String> {
        self.local_assets.values().filter_map(|l| l.body_path.clone()).collect()
    }

    fn upload_status_counts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for l in self.local_assets.values() {
            *counts.entry(l.upload_status.clone()).or_insert(0) += 1;
        }
        counts
    }

    fn failed_uploads(&self, min_attempts: u32) -> Vec<LocalAsset> {
        let mut failed: Vec<LocalAsset> = self
            .local_assets
            .values()
            .filter(|l| l.upload_status == "failed" && l.attempts >= min_attempts)
            .cloned()
            .collect();
        // Newest failure first; rows without a timestamp last.
        failed.sort_by(|a, b| {
            b.updated_at.unwrap_or(f64::NEG_INFINITY).total_cmp(&a.updated_at.unwrap_or(f64::NEG_INFINITY))
        });
        failed
    }

    fn reset_failed_uploads(&mut self) {
        for row in self.local_assets.values_mut() {
            if row.upload_status == "failed" {
                row.upload_status = "new".to_string();
                row.attempts = 0;
                row.next_attempt_at = None;
                row.last_error = None;
            }
        }
    }

    // MARK: Inbox queue

    fn enqueue_inbox(&mut self, asset_ids: &[String], now: f64) {
        for id in asset_ids {
            self.inbox.entry(id.clone()).or_insert(now);
        }
    }

    fn inbox_queue(&self) -> Vec<String> {
        let mut queued: Vec<(&String, f64)> = self.inbox.iter().map(|(id, at)| (id, *at)).collect();
        queued.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(b.0)));
        queued.into_iter().map(|(id, _)| id.clone()).collect()
    }

    fn remove_from_inbox_queue(&mut self, asset_ids: &[String]) {
        for id in asset_ids {
            self.inbox.remove(id.as_str());
        }
    }

    // MARK: Album outbox

    fn enqueue_album_op(&mut self, op: &AlbumOp, now: f64) -> i64 {
        self.next_album_op_id += 1;
        let id = self.next_album_op_id;
        let mut stored = op.clone();
        stored.id = Some(id);
        stored.attempts = 0;
        self.album_ops.insert(id, StoredAlbumOp { op: stored, created_at: now, last_error: None });
        id
    }

    fn pending_album_ops(&self) -> Vec<AlbumOp> {
        self.album_ops
            .iter()
            .map(|(id, stored)| {
                let mut op = stored.op.clone();
                op.id = Some(*id);
                op
            })
            .collect()
    }

    fn delete_album_op(&mut self, id: i64) {
        self.album_ops.remove(&id);
    }

    fn mark_album_op_failed(&mut self, id: i64, error: &str) {
        if let Some(stored) = self.album_ops.get_mut(&id) {
            stored.op.attempts = stored.op.attempts.saturating_add(1);
            stored.last_error = Some(error.to_string());
        }
    }

    // MARK: Mini bitmap slots

    fn mini_bitmap_slots(&self) -> BTreeMap<String, i64> {
        self.mini_bitmaps.clone()
    }

    fn mini_bitmap_slot(&self, asset_id: &str) -> Option<i64> {
        self.mini_bitmaps.get(asset_id).copied()
    }

    fn insert_mini_bitmap(&mut self, asset_id: &str, slot: i64) {
        self.mini_bitmaps.entry(asset_id.to_string()).or_insert(slot);
    }

    fn allocate_mini_bitmap_slot(&mut self) -> i64 {
        let stored = self.meta.get(MINI_NEXT_SLOT).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
        let from_table = self.mini_bitmaps.values().max().map_or(0, |max| max + 1);
        let slot = stored.max(from_table);
        self.meta.insert(MINI_NEXT_SLOT.to_string(), (slot + 1).to_string());
        slot
    }

    fn delete_mini_bitmaps(&mut self, asset_ids: &[String]) {
        for id in asset_ids {
            self.mini_bitmaps.remove(id.as_str());
        }
    }

    fn mini_bitmap_count(&self) -> usize {
        self.mini_bitmaps.len()
    }

    // MARK: Meta

    fn meta(&self, key: &str) -> Option<String> {
        self.meta.get(key).cloned()
    }

    fn set_meta(&mut self, key: &str, value: &str) {
        self.meta.insert(key.to_string(), value.to_string());
    }
}

#[cfg(test)]
mod tests {
    //! Ported from DatabaseTests, FavoritesTests, OfflineTests (store parts) and CoreTests (database parts).
    //! Same inputs and expectations; names in snake_case.

    use super::*;

    const TAKEN: &str = "2026-10-01T20:17:30.923Z";
    /// Base instant of the favorites and offline tests (`1_790_000_000` seconds since 1970).
    const BASE_SECS: i64 = 1_790_000_000;

    const SAMPLE_ASSET_JSON: &str = r#"{"id":"a0437345-fed4-439f-adbf-f3bd38e1cff4","type":"IMAGE",
     "originalPath":"/data/upload/foo.jpg","originalFileName":"a.jpg",
     "originalMimeType":"image/jpeg","thumbhash":"GikKDQK6iGBbGKaHd3uniPN8M/p2",
     "fileCreatedAt":"2026-10-01T20:17:30.923Z","localDateTime":"2026-10-01T20:17:30.923Z",
     "updatedAt":"2026-10-01T20:17:44.383Z","isFavorite":false,"isArchived":false,
     "isTrashed":false,"visibility":"timeline","duration":null,"checksum":"IciyZg02A2tDHvuXe3ldf71pzOY=",
     "isOffline":false,"hasMetadata":true,"resized":true,"width":5648,"height":8472,"isEdited":false}"#;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn ids_of(items: &[TimelineItem]) -> Vec<String> {
        items.iter().map(|i| i.id.clone()).collect()
    }

    fn sync_asset(id: &str) -> SyncAsset {
        SyncAsset {
            id: id.to_string(),
            asset_type: Some("IMAGE".to_string()),
            thumbhash: Some("T".to_string()),
            checksum: None,
            width: Some(100),
            height: Some(50),
            file_created_at: Some(TAKEN.to_string()),
            file_modified_at: None,
            local_date_time: None,
            created_at: None,
            deleted_at: None,
            is_favorite: Some(false),
            is_edited: Some(false),
            duration: None,
            live_photo_video_id: None,
            original_file_name: Some(format!("{id}.jpg")),
            visibility: Some("timeline".to_string()),
        }
    }

    fn sync_album(id: &str, name: &str) -> SyncAlbum {
        SyncAlbum {
            id: id.to_string(),
            name: Some(name.to_string()),
            description: None,
            thumbnail_asset_id: None,
            created_at: None,
            updated_at: None,
            order: None,
        }
    }

    fn album_model(id: &str, name: &str, count: i64) -> Album {
        Album {
            id: id.to_string(),
            album_name: name.to_string(),
            description: None,
            album_thumbnail_asset_id: None,
            created_at: None,
            updated_at: None,
            asset_count: count,
            shared: false,
            has_shared_link: false,
            order: None,
        }
    }

    /// Civil date from days since 1970 (Howard Hinnant's algorithm).
    fn civil_from_days(z: i64) -> (i64, i64, i64) {
        let z = z + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        (year, month, day)
    }

    /// ISO timestamp `minutes` before the base instant, in the Immich format with milliseconds.
    fn taken_minutes_ago(minutes: i64) -> String {
        let secs = BASE_SECS - minutes * 60;
        let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
        let rem = secs.rem_euclid(86_400);
        format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.000Z", rem / 3600, (rem % 3600) / 60, rem % 60)
    }

    fn fav_asset(id: &str, favorite: bool, minutes_ago: i64, visibility: &str, trashed: bool) -> SyncAsset {
        let date = taken_minutes_ago(minutes_ago);
        SyncAsset {
            is_favorite: Some(favorite),
            visibility: Some(visibility.to_string()),
            deleted_at: trashed.then(|| date.clone()),
            file_created_at: Some(date),
            width: Some(4),
            height: Some(3),
            thumbhash: None,
            ..sync_asset(id)
        }
    }

    /// Offline-test asset: a video has a duration and a `.MOV` name.
    fn offline_asset(id: &str, minutes_ago: i64, trashed: bool, video: bool) -> SyncAsset {
        let date = taken_minutes_ago(minutes_ago);
        SyncAsset {
            asset_type: Some(if video { "VIDEO" } else { "IMAGE" }.to_string()),
            duration: video.then(|| "0:05".to_string()),
            original_file_name: Some(format!("{id}.{}", if video { "MOV" } else { "JPG" })),
            deleted_at: trashed.then(|| date.clone()),
            file_created_at: Some(date),
            width: Some(4),
            height: Some(3),
            thumbhash: None,
            ..sync_asset(id)
        }
    }

    /// Stand-in for `AlbumOutbox.apply` (sync layer, not ported): applies one op to the store.
    fn apply_op(store: &mut MemoryStore, op: &AlbumOp) {
        let ids = op.asset_ids.clone();
        match op.kind {
            super::super::models::album_op::AlbumOpKind::Add => {
                store.add_album_assets(op.album_id.as_deref().unwrap_or(""), &ids)
            }
            super::super::models::album_op::AlbumOpKind::Remove => {
                store.remove_album_assets(op.album_id.as_deref().unwrap_or(""), &ids)
            }
            super::super::models::album_op::AlbumOpKind::Trash => store.trash_assets(&ids),
            super::super::models::album_op::AlbumOpKind::Restore => store.untrash_assets(&ids),
            super::super::models::album_op::AlbumOpKind::Favorite => store.set_favorite(&ids, true),
            super::super::models::album_op::AlbumOpKind::Unfavorite => store.set_favorite(&ids, false),
        }
    }

    // MARK: DatabaseTests

    #[test]
    fn rendition_changes_are_detected() {
        let mut db = MemoryStore::new();
        assert!(db.upsert_sync_assets(&[sync_asset("a"), sync_asset("b")]).is_empty(), "new assets are not changes");
        assert!(db.upsert_sync_assets(&[sync_asset("a")]).is_empty(), "same rendition, no change");
        // A finished server-side edit: new thumbhash and dimensions.
        let edited_a = SyncAsset { thumbhash: Some("T2".to_string()), ..sync_asset("a") };
        let edited_b = SyncAsset { width: Some(50), height: Some(100), ..sync_asset("b") };
        assert_eq!(db.upsert_sync_assets(&[edited_a, edited_b]), set(&["a", "b"]));
        assert_eq!(db.asset_info("b").map(|i| i.width), Some(Some(50)));
    }

    #[test]
    fn timeline_hides_motion_parts_archive_and_trash() {
        let mut db = MemoryStore::new();
        db.upsert_sync_assets(&[
            sync_asset("visible"),
            SyncAsset { visibility: Some("hidden".into()), ..sync_asset("motion") },
            SyncAsset { visibility: Some("archive".into()), ..sync_asset("archived") },
            SyncAsset { visibility: Some("locked".into()), ..sync_asset("locked") },
            SyncAsset { deleted_at: Some("2026-10-02T00:00:00.000Z".into()), ..sync_asset("trashed") },
        ]);
        assert_eq!(ids_of(&db.fetch_timeline()), strings(&["visible"]));
        db.upsert_sync_album(&sync_album("al", "A"));
        db.add_album_assets("al", &strings(&["visible", "motion", "archived", "trashed"]));
        let shown: BTreeSet<String> = ids_of(&db.album_assets("al")).into_iter().collect();
        assert_eq!(shown, set(&["visible", "archived"]), "albums show archived photos too");
    }

    /// Inside an album the newest photo is on top, the oldest at the bottom, whatever the server's order.
    #[test]
    fn album_shows_newest_first() {
        let mut db = MemoryStore::new();
        db.upsert_sync_assets(&[
            SyncAsset { file_created_at: Some("2024-05-01T10:00:00.000Z".into()), ..sync_asset("old") },
            SyncAsset { file_created_at: Some("2026-10-20T10:00:00.000Z".into()), ..sync_asset("new") },
            SyncAsset { file_created_at: Some("2025-07-01T10:00:00.000Z".into()), ..sync_asset("mid") },
        ]);
        db.upsert_sync_album(&SyncAlbum { order: Some("asc".into()), ..sync_album("al", "A") });
        db.add_album_assets("al", &strings(&["old", "new", "mid"]));
        assert_eq!(ids_of(&db.album_assets("al")), strings(&["new", "mid", "old"]));
    }

    #[test]
    fn full_sync_upsert_keeps_cached_file_size() {
        let mut db = MemoryStore::new();
        let json = r#"{"id":"x","type":"IMAGE","isFavorite":false,"isTrashed":false,"thumbhash":"A","width":10,"height":20}"#;
        let asset: Asset = serde_json::from_str(json).unwrap();
        assert!(db.upsert_assets(std::slice::from_ref(&asset)).is_empty());
        db.set_file_size("x", 1234);
        assert!(db.upsert_assets(std::slice::from_ref(&asset)).is_empty());
        assert_eq!(db.asset_info("x").and_then(|i| i.file_size), Some(1234), "a re-sync without exif must not wipe the size");
        assert_eq!(db.asset_info("x").and_then(|i| i.height), Some(20));
    }

    #[test]
    fn local_folders() {
        let mut db = MemoryStore::new();
        db.add_local_folder("Reisen");
        db.add_local_folder("Reisen/2026");
        db.add_local_folder("Reisende");
        db.add_local_folder("Reisen"); // idempotent
        assert_eq!(db.local_folders(), strings(&["Reisen", "Reisen/2026", "Reisende"]));
        db.move_local_folders("Reisen", "Urlaub");
        assert_eq!(db.local_folders(), strings(&["Reisende", "Urlaub", "Urlaub/2026"]), "descendants move along, look-alikes stay");
        db.remove_local_folders_under("Urlaub");
        assert_eq!(db.local_folders(), strings(&["Reisende"]));
    }

    #[test]
    fn offload_and_exclusion_states() {
        let mut db = MemoryStore::new();
        db.upsert_sync_assets(&[
            sync_asset("s1"),
            sync_asset("s2"),
            SyncAsset { deleted_at: Some("2026-10-02T00:00:00.000Z".into()), ..sync_asset("s3") },
        ]);
        for (local, server) in [("p1", "s1"), ("p2", "s2"), ("p3", "s3")] {
            db.upsert_local_asset(local, Some("c"), 1.0, false);
            db.mark_local_asset_uploaded(local, server);
        }
        db.upsert_local_asset("p4", Some("c"), 1.0, false); // never uploaded
        let offloadable: BTreeSet<String> = db.offloadable_local_assets().into_iter().map(|l| l.photokit_id).collect();
        assert_eq!(offloadable, set(&["p1", "p2"]), "only photos whose server copy exists and is not trashed");
        db.exclude_local_assets(&strings(&["s2"]));
        assert_eq!(db.local_asset("p2").map(|l| l.upload_status), Some("excluded".to_string()));
        let offloadable: Vec<String> = db.offloadable_local_assets().into_iter().map(|l| l.photokit_id).collect();
        assert_eq!(offloadable, strings(&["p1"]));
        db.set_local_asset_status(&strings(&["p1"]), "offloaded");
        let offloaded: Vec<String> = db.local_assets_with_status("offloaded").into_iter().map(|l| l.photokit_id).collect();
        assert_eq!(offloaded, strings(&["p1"]));
        // Re-scan bookkeeping keeps the state.
        db.upsert_local_asset("p1", Some("c2"), 2.0, true);
        assert_eq!(db.local_asset("p1").map(|l| l.upload_status), Some("offloaded".to_string()));
    }

    #[test]
    fn assets_already_in_albums() {
        let mut db = MemoryStore::new();
        db.upsert_sync_album(&sync_album("al", "A"));
        db.add_album_assets("al", &strings(&["a"]));
        assert_eq!(db.asset_ids_in_any_album(&strings(&["a", "b"])), set(&["a"]));
        assert!(db.asset_ids_in_any_album(&[]).is_empty());
        assert_eq!(db.album_id_named("A"), Some("al".to_string()));
        assert_eq!(db.album_id_named("Posteingang"), None);
        let many: Vec<String> = (0..1200).map(|i| format!("id{i}")).collect();
        db.add_album_assets("al", &many);
        let mut query = many.clone();
        query.push("zzz".to_string());
        assert_eq!(db.asset_ids_in_any_album(&query).len(), 1200);
    }

    /// DB part of `testMiniBitmapSlotsAreNeverReused`: slots of deleted bitmaps are not handed out again.
    #[test]
    fn mini_bitmap_slots_are_never_reused() {
        let mut db = MemoryStore::new();
        let first = db.allocate_mini_bitmap_slot();
        db.insert_mini_bitmap("a", first);
        assert_eq!(db.mini_bitmap_slot("a"), Some(first));
        db.delete_mini_bitmaps(&strings(&["a"]));
        assert_eq!(db.mini_bitmap_slot("a"), None);
        let second = db.allocate_mini_bitmap_slot();
        assert!(second > first, "a refilled bitmap gets a fresh slot");
        db.insert_mini_bitmap("a", second);
        assert_eq!(db.mini_bitmap_count(), 1);
    }

    /// DB part of `testMiniBitmapStoreRoundtrip`: write-once slots.
    #[test]
    fn mini_bitmap_insert_is_write_once() {
        let mut db = MemoryStore::new();
        let slot = db.allocate_mini_bitmap_slot();
        db.insert_mini_bitmap("test1", slot);
        db.insert_mini_bitmap("test1", slot + 5);
        assert_eq!(db.mini_bitmap_count(), 1);
        assert_eq!(db.mini_bitmap_slot("test1"), Some(slot));
        assert_eq!(db.mini_bitmap_slot("unknown"), None);
    }

    #[test]
    fn optimistic_edit_updates_ratio() {
        let mut db = MemoryStore::new();
        db.upsert_sync_assets(&[SyncAsset { width: Some(4000), height: Some(3000), ..sync_asset("e") }]);
        db.apply_local_edit("e", 1500, 2000, true);
        let info = db.asset_info("e").expect("asset exists");
        assert_eq!(info.width, Some(1500));
        assert!(info.is_edited);
        let ratio = db.fetch_timeline().first().and_then(|i| i.ratio).unwrap_or(0.0);
        assert!((ratio - 0.75).abs() < 1e-9);
    }

    /// Full sync (server count) and stream backfill (links) in either order: never both added up.
    #[test]
    fn album_count_is_not_double_counted_by_sync_order() {
        let mut db = MemoryStore::new();
        db.upsert_albums(&[album_model("al", "Quelle", 3)]);
        assert_eq!(db.albums().first().map(|a| a.asset_count), Some(3), "server count until links are known");
        db.upsert_sync_assets(&["a", "b", "c"].map(sync_asset));
        for id in ["a", "b", "c"] {
            db.add_album_assets("al", &strings(&[id]));
        }
        assert_eq!(db.albums().first().map(|a| a.asset_count), Some(3), "links after the server count");

        // Links first, then a (stale) server listing: the links win.
        db.upsert_albums(&[album_model("al", "Quelle", 5)]);
        assert_eq!(db.albums().first().map(|a| a.asset_count), Some(3));
        db.set_album_assets("al", &strings(&["a"]));
        assert_eq!(db.albums().first().map(|a| a.asset_count), Some(1));
    }

    #[test]
    fn album_count_is_what_the_album_shows() {
        let mut db = MemoryStore::new();
        db.upsert_sync_assets(&[offline_asset("a", 0, false, false), offline_asset("b", 0, false, false), offline_asset("trashed", 0, true, false)]);
        db.upsert_sync_album(&sync_album("al", "A"));
        db.add_album_assets("al", &strings(&["a", "b", "trashed"]));
        assert_eq!(db.albums().first().map(|a| a.asset_count), Some(2), "a trashed photo keeps its link but leaves the album");
        db.upsert_albums(&[album_model("empty", "B", 7)]);
        assert_eq!(db.albums().iter().find(|a| a.id == "empty").map(|a| a.asset_count), Some(7), "the server's count while no links are known");
    }

    #[test]
    fn merge_restores_links_and_keeps_trashed_ones() {
        let mut db = MemoryStore::new();
        db.upsert_sync_assets(&[
            offline_asset("kept", 0, false, false),
            offline_asset("lost", 0, false, false),
            offline_asset("removed", 0, false, false),
            offline_asset("trashed", 0, true, false),
        ]);
        db.upsert_sync_album(&sync_album("al", "A"));
        db.add_album_assets("al", &strings(&["kept", "removed", "trashed"]));
        assert!(db.merge_album_assets("al", &strings(&["kept", "lost"])));
        let shown: BTreeSet<String> = ids_of(&db.album_assets("al")).into_iter().collect();
        assert_eq!(shown, set(&["kept", "lost"]));
        db.untrash_assets(&strings(&["trashed"]));
        assert!(ids_of(&db.album_assets("al")).contains(&"trashed".to_string()), "restored from the trash, back in the album");
        assert!(!db.merge_album_assets("al", &strings(&["kept", "lost", "trashed"])), "nothing to change");
    }

    // MARK: FavoritesTests

    #[test]
    fn favorites_are_what_the_album_shows() {
        let mut db = MemoryStore::new();
        db.upsert_sync_assets(&[
            fav_asset("old", true, 10, "timeline", false),
            fav_asset("new", true, 0, "timeline", false),
            fav_asset("archived", true, 5, "archive", false),
            fav_asset("motion", true, 0, "hidden", false),
            fav_asset("trashed", true, 0, "timeline", true),
            fav_asset("plain", false, 0, "timeline", false),
        ]);
        assert_eq!(ids_of(&db.favorite_items()), strings(&["new", "archived", "old"]), "newest first, archived too, like an album");
        let (count, cover) = db.favorite_summary();
        assert_eq!(count, 3);
        assert_eq!(cover.as_deref(), Some("new"), "the newest favorite is the cover");
        assert!(db.is_favorite("old"));
        assert!(!db.is_favorite("plain"));
    }

    #[test]
    fn empty_favorites() {
        let db = MemoryStore::new();
        let (count, cover) = db.favorite_summary();
        assert_eq!(count, 0);
        assert_eq!(cover, None);
    }

    /// Sorting a photo into the favorites: it gets the heart and leaves the album; undo brings both back.
    #[test]
    fn sorting_into_favorites_and_undo() {
        let mut db = MemoryStore::new();
        db.upsert_sync_assets(&[sync_asset("x")]);
        db.upsert_sync_album(&sync_album("in", "Posteingang"));
        db.add_album_assets("in", &strings(&["x"]));

        let move_ops = vec![AlbumOp::favorite(strings(&["x"])), AlbumOp::remove(strings(&["x"]), "in")];
        for op in &move_ops {
            apply_op(&mut db, op);
        }
        assert!(db.is_favorite("x"));
        assert!(db.album_assets("in").is_empty());
        assert_eq!(ids_of(&db.favorite_items()), strings(&["x"]));

        for op in move_ops.iter().rev() {
            apply_op(&mut db, &op.inverse());
        }
        assert!(!db.is_favorite("x"));
        assert_eq!(ids_of(&db.album_assets("in")), strings(&["x"]));
    }

    /// The queue part of `testFavoriteOpsQueueLikeAlbumChanges`.
    #[test]
    fn favorite_ops_queue_like_album_changes() {
        let mut db = MemoryStore::new();
        db.enqueue_album_op(&AlbumOp::favorite(strings(&["a"])), 1.0);
        db.enqueue_album_op(&AlbumOp::unfavorite(strings(&["b"])), 1.0);
        let ops = db.pending_album_ops();
        let kinds: Vec<_> = ops.iter().map(|o| o.kind).collect();
        assert_eq!(kinds, vec![super::super::models::album_op::AlbumOpKind::Favorite, super::super::models::album_op::AlbumOpKind::Unfavorite]);
        assert!(ops[0].album_id.is_none());
    }

    // MARK: OfflineTests (store parts)

    /// The store side of „Nicht mehr auf dem iPhone behalten“: only the originals go, the album keeps its photos.
    #[test]
    fn stop_keeping_removes_only_the_originals() {
        let mut db = MemoryStore::new();
        let ids: Vec<String> = (1..=9).map(|i| format!("t{i}")).collect();
        let assets: Vec<SyncAsset> = ids.iter().enumerate().map(|(i, id)| offline_asset(id, i as i64, false, false)).collect();
        db.upsert_sync_assets(&assets);
        db.upsert_sync_album(&sync_album("trst", "Reisen/Trst"));
        db.add_album_assets("trst", &ids);

        db.set_album_offline("trst", true);
        assert_eq!(db.offline_members().len(), 9);
        for member in db.offline_members() {
            db.set_offline_file(&member.asset_id, &format!("{}.jpg", member.asset_id), 1);
        }
        assert_eq!(db.offline_files().len(), 9);

        db.set_album_offline("trst", false);
        db.remove_offline_files(&db.offline_files().keys().cloned().collect::<Vec<_>>());
        assert!(db.offline_files().is_empty());
        assert_eq!(db.albums().first().map(|a| a.asset_count), Some(9), "the album keeps its photos");
        let shown: BTreeSet<String> = ids_of(&db.album_assets("trst")).into_iter().collect();
        assert_eq!(shown, ids.iter().cloned().collect::<BTreeSet<_>>());
        assert_eq!(db.fetch_timeline().len(), 9, "nothing was trashed");
    }

    #[test]
    fn original_shared_by_two_albums_is_listed_once_per_album() {
        let mut db = MemoryStore::new();
        db.upsert_sync_assets(&[offline_asset("x", 0, false, false), offline_asset("y", 1, false, false), offline_asset("z", 2, false, false)]);
        db.upsert_sync_album(&sync_album("a", "A"));
        db.add_album_assets("a", &strings(&["x", "y"]));
        db.upsert_sync_album(&sync_album("b", "B"));
        db.add_album_assets("b", &strings(&["x", "z"]));

        db.set_album_offline("a", true);
        let mut ids: Vec<String> = db.offline_members().into_iter().map(|m| m.asset_id).collect();
        ids.sort();
        assert_eq!(ids, strings(&["x", "y"]));
        db.set_album_offline("b", true);
        let mut ids: Vec<String> = db.offline_members().into_iter().map(|m| m.asset_id).collect();
        ids.sort();
        assert_eq!(ids, strings(&["x", "x", "y", "z"]), "x is listed once for both albums");
    }

    #[test]
    fn membership_changes_and_deleted_albums_are_followed() {
        let mut db = MemoryStore::new();
        db.upsert_sync_assets(&[offline_asset("p", 0, false, false), offline_asset("q", 1, false, false), offline_asset("r", 2, false, false)]);
        db.upsert_sync_album(&sync_album("a", "A"));
        db.add_album_assets("a", &strings(&["p", "q"]));
        db.set_album_offline("a", true);

        db.remove_album_assets("a", &strings(&["q"]));
        db.add_album_assets("a", &strings(&["r"]));
        db.trash_assets(&strings(&["p"]));
        let members: BTreeSet<String> = db.offline_members().into_iter().map(|m| m.asset_id).collect();
        assert_eq!(members, set(&["r"]), "moved out and trashed photos go, new ones come");

        db.delete_album("a");
        assert!(db.offline_members().is_empty());
    }

    /// The part of the disk scan that lives in the store: a legacy absolute path reads as its file name.
    #[test]
    fn disk_scan_reads_legacy_absolute_paths_as_file_names() {
        let mut db = MemoryStore::new();
        db.set_offline_file(
            "legacy",
            "/var/mobile/Containers/Data/Application/OLD/Library/Application Support/Originals/legacy.JPG",
            1,
        );
        db.set_offline_file("gone", "gone.jpg", 1);
        let files = db.offline_files();
        assert_eq!(files.get("legacy").map(String::as_str), Some("legacy.JPG"));
        assert_eq!(files.get("gone").map(String::as_str), Some("gone.jpg"));
    }

    #[test]
    fn album_count_of_offline_album_stays_the_visible_count() {
        let mut db = MemoryStore::new();
        db.upsert_sync_assets(&[offline_asset("a", 0, false, false)]);
        db.upsert_sync_album(&sync_album("al", "A"));
        db.add_album_assets("al", &strings(&["a"]));
        db.set_album_offline("al", true);
        assert_eq!(db.offline_album_ids(), set(&["al"]));
        assert_eq!(db.album_visible_counts().get("al"), Some(&1));
    }

    // MARK: CoreTests (database parts)

    #[test]
    fn database_asset_roundtrip() {
        let mut db = MemoryStore::new();
        let asset: Asset = serde_json::from_str(SAMPLE_ASSET_JSON).unwrap();
        db.upsert_assets(std::slice::from_ref(&asset));
        let timeline = db.fetch_timeline();
        assert_eq!(timeline.len(), 1);
        assert_eq!(timeline.first().map(|i| i.id.as_str()), Some(asset.id.as_str()));
        assert_eq!(timeline.first().map(|i| i.date_bucket.as_str()), Some("2026-10-01"));
        assert_eq!(db.asset_count(), 1);
    }

    #[test]
    fn database_album_upsert_keeps_offline_flag() {
        let mut db = MemoryStore::new();
        let album = album_model("a1", "Test", 3);
        db.upsert_albums(std::slice::from_ref(&album));
        db.set_album_offline("a1", true);
        db.upsert_albums(std::slice::from_ref(&album)); // re-sync must not reset the flag
        assert!(db.offline_album_ids().contains("a1"));
    }

    #[test]
    fn database_local_asset_upload_tracking() {
        let mut db = MemoryStore::new();
        db.upsert_local_asset("p1", Some("abc"), 123.0, true);
        assert_eq!(db.local_assets(true).len(), 1);
        db.mark_local_asset_uploaded("p1", "s1");
        assert_eq!(db.local_assets(true).len(), 0);
        assert_eq!(db.local_asset("p1").and_then(|l| l.server_asset_id), Some("s1".to_string()));
        assert_eq!(db.local_asset("p1").map(|l| l.upload_status), Some("uploaded".to_string()));
    }

    #[test]
    fn sync_asset_upsert_preserves_original_path() {
        let mut db = MemoryStore::new();
        let full: Asset = serde_json::from_str(SAMPLE_ASSET_JSON).unwrap();
        db.upsert_assets(std::slice::from_ref(&full));

        // Partial sync payload for the same id must not wipe original_path.
        let sync = SyncAsset {
            id: full.id.clone(),
            thumbhash: Some("X".to_string()),
            checksum: Some("Y".to_string()),
            width: Some(1),
            height: Some(1),
            is_favorite: Some(true),
            original_file_name: Some("a.jpg".to_string()),
            ..sync_asset(&full.id)
        };
        db.upsert_sync_assets(&[sync]);

        let fetched = db.asset(&full.id).expect("asset exists");
        assert_eq!(fetched.original_path.as_deref(), Some("/data/upload/foo.jpg"));
        assert!(fetched.is_favorite);
    }

    #[test]
    fn sync_album_upsert_and_delete() {
        let mut db = MemoryStore::new();
        let album = SyncAlbum { description: Some("d".into()), thumbnail_asset_id: Some("t1".into()), ..sync_album("al1", "Sync-Album") };
        db.upsert_sync_album(&album);
        db.set_album_offline("al1", true);
        db.upsert_sync_album(&album); // re-sync keeps the local flag
        assert!(db.offline_album_ids().contains("al1"));

        db.delete_album("al1");
        assert!(db.albums().is_empty());
        assert!(db.offline_album_ids().is_empty());
    }

    #[test]
    fn sync_album_asset_links_and_counts() {
        let mut db = MemoryStore::new();
        db.upsert_sync_album(&sync_album("al1", "A"));
        // The count is what the album shows: links to photos the database knows.
        db.upsert_sync_assets(&["x1", "x2"].map(sync_asset));
        db.add_album_assets("al1", &strings(&["x1", "x2"]));
        db.add_album_assets("al1", &strings(&["x2"])); // duplicate link is ignored
        assert_eq!(db.albums().first().map(|a| a.asset_count), Some(2));
        db.remove_album_assets("al1", &strings(&["x1"]));
        assert_eq!(db.albums().first().map(|a| a.asset_count), Some(1));
        db.remove_album_assets("al1", &strings(&["x1"])); // already gone, no underflow
        assert_eq!(db.albums().first().map(|a| a.asset_count), Some(1));
    }

    #[test]
    fn timeline_sort_date_zero_is_none() {
        let mut db = MemoryStore::new();
        let undated = SyncAsset { file_created_at: None, ..sync_asset("no-date") };
        db.upsert_sync_assets(&[undated]); // capture date missing: sort key 0
        let item = db.fetch_timeline().into_iter().next();
        assert_eq!(item.and_then(|i| i.sort_date), None, "sort_date 0 must map to None, not 1 January 1970");
    }

    // MARK: Upload pipeline and inbox (store-only behavior)

    #[test]
    fn upload_bookkeeping_follows_the_state_machine() {
        let mut db = MemoryStore::new();
        db.mark_uploading("p1", 7, "/body/p1", "c", 1.0, false, 10.0);
        assert_eq!(db.local_asset("p1").map(|l| l.upload_status), Some("uploading".to_string()));

        db.mark_upload_failed("p1", 1, Some(20.0), Some("timeout"), Some("/body/p1"), 11.0);
        let failed = db.local_asset("p1").expect("row exists");
        assert_eq!(failed.upload_status, "failed");
        assert_eq!(failed.attempts, 1);
        assert_eq!(failed.body_path.as_deref(), Some("/body/p1"));
        assert_eq!(db.failed_uploads(1).len(), 1);
        assert!(db.failed_uploads(2).is_empty());
        assert!(db.upload_body_paths().contains("/body/p1"));

        db.reset_failed_uploads();
        let reset = db.local_asset("p1").expect("row exists");
        assert_eq!((reset.upload_status.as_str(), reset.attempts), ("new", 0));

        db.mark_uploading("p1", 8, "/body/p1", "c", 1.0, false, 12.0);
        db.requeue_upload("p1", None, 13.0);
        let queued = db.local_asset("p1").expect("row exists");
        assert_eq!((queued.upload_status.as_str(), queued.task_id), ("new", None));

        db.mark_uploaded("p1", "s1", None, None, None, 14.0);
        let done = db.local_asset("p1").expect("row exists");
        assert_eq!(done.upload_status, "uploaded");
        assert_eq!(done.server_asset_id.as_deref(), Some("s1"));
        assert_eq!(done.body_path, None);
        assert_eq!(db.upload_status_counts().get("uploaded"), Some(&1));
    }

    #[test]
    fn inbox_queue_is_oldest_first_and_idempotent() {
        let mut db = MemoryStore::new();
        db.enqueue_inbox(&strings(&["b"]), 2.0);
        db.enqueue_inbox(&strings(&["a"]), 1.0);
        db.enqueue_inbox(&strings(&["b"]), 9.0); // already queued, keeps its place
        assert_eq!(db.inbox_queue(), strings(&["a", "b"]));
        db.remove_from_inbox_queue(&strings(&["a"]));
        assert_eq!(db.inbox_queue(), strings(&["b"]));
    }

    #[test]
    fn meta_starts_with_needs_full_sync() {
        let mut db = MemoryStore::new();
        assert_eq!(db.meta("needs_full_sync").as_deref(), Some("1"));
        db.set_meta("needs_full_sync", "0");
        assert_eq!(db.meta("needs_full_sync").as_deref(), Some("0"));
        assert_eq!(db.meta("missing"), None);
    }

    // MARK: Snapshot

    #[test]
    fn snapshot_roundtrip_restores_everything_including_indexes() {
        let mut db = MemoryStore::new();
        db.upsert_sync_assets(&[sync_asset("a"), fav_asset("b", true, 3, "timeline", false), fav_asset("c", false, 0, "timeline", true)]);
        db.upsert_sync_album(&sync_album("al", "Alben"));
        db.add_album_assets("al", &strings(&["a", "b"]));
        db.set_album_offline("al", true);
        db.set_offline_file("a", "a.jpg", 10);
        db.upsert_local_asset("p1", Some("c"), 1.0, false);
        db.mark_local_asset_uploaded("p1", "a");
        db.enqueue_album_op(&AlbumOp::add(strings(&["a"]), "al"), 5.0);
        db.enqueue_inbox(&strings(&["a"]), 4.0);
        db.add_local_folder("Reisen");
        let slot = db.allocate_mini_bitmap_slot();
        db.insert_mini_bitmap("a", slot);

        let bytes = db.snapshot().expect("snapshot");
        let restored = MemoryStore::restore(&bytes).expect("restore");
        assert_eq!(restored, db, "the whole store, indexes included, survives the roundtrip");
        assert_eq!(ids_of(&restored.fetch_timeline()), ids_of(&db.fetch_timeline()));
        assert_eq!(restored.pending_album_ops().len(), 1);
        assert_eq!(restored.mini_bitmap_slot("a"), Some(slot));
        assert_eq!(restored.inbox_queue(), strings(&["a"]));
        assert_eq!(restored.offline_album_ids(), set(&["al"]));
        assert!(restored.album_ids_containing("a").contains("al"));
    }
    // MARK: Hidden before (display date filter)

    const DAY_MIN: i64 = 24 * 60;

    /// new (now), mid (3 days ago), old (10 days ago), undated (no capture date); cutoff 5 days ago.
    fn hidden_fixture() -> (MemoryStore, i64) {
        let mut db = MemoryStore::new();
        let mut undated = fav_asset("undated", true, 0, "timeline", false);
        undated.file_created_at = None;
        db.upsert_sync_assets(&[
            fav_asset("new", true, 0, "timeline", false),
            fav_asset("mid", true, 3 * DAY_MIN, "timeline", false),
            fav_asset("old", true, 10 * DAY_MIN, "timeline", false),
            fav_asset("oldarch", true, 11 * DAY_MIN, "archive", false),
            undated,
        ]);
        db.upsert_sync_album(&sync_album("al", "A"));
        db.add_album_assets("al", &strings(&["new", "old", "oldarch"]));
        ((db), (BASE_SECS - 5 * 86_400) * 1000)
    }

    #[test]
    fn hidden_before_filters_every_query_and_restores() {
        let (mut db, cut) = hidden_fixture();
        let all_timeline = ids_of(&db.fetch_timeline());
        let all_album = ids_of(&db.album_assets("al"));
        let all_fav = ids_of(&db.favorite_items());
        assert_eq!(db.hidden_count(), 0);
        let mini_all = db.assets_without_mini_bitmap(10);

        db.set_hidden_before(cut);
        assert_eq!(set(&all_timeline.iter().map(String::as_str).collect::<Vec<_>>()), set(&["new", "mid", "old", "undated"]));
        let shown: BTreeSet<String> = ids_of(&db.fetch_timeline()).into_iter().collect();
        assert_eq!(shown, set(&["new", "mid", "undated"]), "undated stays visible");
        assert_eq!(ids_of(&db.album_assets("al")), vec!["new".to_string()]);
        assert_eq!(db.album_visible_counts().get("al"), Some(&1));
        let fav: BTreeSet<String> = ids_of(&db.favorite_items()).into_iter().collect();
        assert_eq!(fav, set(&["new", "mid", "undated"]));
        assert_eq!(db.favorite_summary().0, 3);
        assert_eq!(db.timeline_items(&strings(&["new", "old", "mid"])).len(), 2);
        assert_eq!(db.assets_without_mini_bitmap(10).len(), 3, "no mini work for hidden photos");
        assert_eq!(db.hidden_count(), 2, "old and the archived old one");
        assert!(db.is_hidden("old") && !db.is_hidden("mid") && !db.is_hidden("undated") && !db.is_hidden("nope"));
        // The rows are still there: nothing was dropped.
        assert_eq!(db.asset_count(), 5);
        assert!(db.asset("old").is_some());

        // Boundary: the cutoff instant itself is shown, one millisecond before is not.
        db.set_hidden_before((BASE_SECS - 10 * 86_400) * 1000 + 1);
        assert!(db.is_hidden("old"));
        db.set_hidden_before((BASE_SECS - 10 * 86_400) * 1000);
        assert!(!db.is_hidden("old"));

        // Back to no limit: everything returns without a sync.
        db.set_hidden_before(0);
        assert_eq!(ids_of(&db.fetch_timeline()), all_timeline);
        assert_eq!(ids_of(&db.album_assets("al")), all_album);
        assert_eq!(ids_of(&db.favorite_items()), all_fav);
        assert_eq!(db.assets_without_mini_bitmap(10), mini_all);
        assert_eq!(db.album_visible_counts().get("al"), Some(&3));
    }

    #[test]
    fn hidden_before_is_not_persisted_and_rows_survive_snapshots() {
        let (mut db, cut) = hidden_fixture();
        db.set_hidden_before(cut);
        let restored = MemoryStore::restore(&db.snapshot().unwrap()).unwrap();
        assert_eq!(restored.hidden_before(), 0, "per-device setting, applied from the settings at start");
        assert_eq!(restored.fetch_timeline().len(), 4);
    }
}
