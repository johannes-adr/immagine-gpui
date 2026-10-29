//! Owner: A5. Swift source: `../immagine/Immagine/Thumbnails/ThumbnailStore.swift (cache policy)`.
//! Spec: `../immagine/ImmagineTests/` -> RenditionCacheTests.
//! Port the tests first (same names in snake_case), then implement until `cargo test --lib logic::rendition_cache` is green.
//!
//! The cache policy of `ThumbnailStore` and `MiniBitmapStore`, as state machines without I/O:
//! - which memory pool a rendition lives in, and the budgets (NSCache's cost limits, here an LRU);
//! - request coalescing: one download per rendition, waiters counted, the download cancelled when
//!   the last waiter leaves;
//! - invalidation by epoch: after an edit, a download that was already running must not put the
//!   old rendition back;
//! - tier-B slots: write-once, never reused after an invalidation.
//!
//! The caller owns the actual work: it performs the download or decode, keeps the disk files
//! (the names come from `file_name`) and the image handles `V` (for example a JS `ImageBitmap`).
//! Nothing here may touch the network, the DOM or the file system.

use std::collections::HashMap;

/// Size stages of a rendition. The names are the Swift size strings used in the disk file names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Rendition {
    /// Grid thumbnail, from the server's "thumbnail" rendition.
    Thumbnail,
    /// Viewer preview, from the server's "preview" rendition.
    Preview,
    /// An offline original decoded to screen size. Memory only, never on disk.
    Local,
}

impl Rendition {
    /// All renditions, in the order `invalidate` visits them.
    pub const ALL: [Rendition; 3] = [Rendition::Thumbnail, Rendition::Preview, Rendition::Local];

    /// The Swift size string, as used in keys and file names.
    pub fn as_str(self) -> &'static str {
        match self {
            Rendition::Thumbnail => "thumbnail",
            Rendition::Preview => "preview",
            Rendition::Local => "local",
        }
    }

    /// Grid thumbnails and viewer previews live in separate pools: a few swipes through big
    /// previews must not evict the grid's thumbnails.
    pub fn pool(self) -> Pool {
        match self {
            Rendition::Thumbnail => Pool::Thumbnails,
            Rendition::Preview | Rendition::Local => Pool::Previews,
        }
    }
}

/// A memory pool of decoded images.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pool {
    Thumbnails,
    Previews,
}

/// Budget of the thumbnail pool: about 300 decoded grid thumbnails.
pub const THUMBNAIL_BUDGET: usize = 100 * 1024 * 1024;
/// Budget of the preview pool: about 12 decoded previews (the viewer's prefetch window plus the
/// pages just left behind).
pub const PREVIEW_BUDGET: usize = 140 * 1024 * 1024;

/// Memory cost of a decoded 4-byte-per-pixel bitmap (bytes per row × height).
pub fn decoded_cost(width: u32, height: u32) -> usize {
    width as usize * 4 * height as usize
}

/// Name of the disk file for a server rendition: `{id}-{size}.bin`.
pub fn file_name(id: &str, rendition: Rendition) -> String {
    format!("{}.bin", key(id, rendition))
}

fn key(id: &str, rendition: Rendition) -> String {
    format!("{}-{}", id, rendition.as_str())
}

/// Cost-bounded LRU. Stands in for `NSCache`: once the total cost passes the limit the least
/// recently used entries go first. An entry larger than the whole budget is not kept.
pub struct MemoryCache<V> {
    limit: usize,
    total: usize,
    clock: u64,
    entries: HashMap<String, Entry<V>>,
}

struct Entry<V> {
    value: V,
    cost: usize,
    last_used: u64,
}

impl<V: Clone> MemoryCache<V> {
    pub fn new(limit: usize) -> Self {
        Self { limit, total: 0, clock: 0, entries: HashMap::new() }
    }

    /// Looks up a value and marks it as recently used.
    pub fn get(&mut self, key: &str) -> Option<V> {
        self.clock += 1;
        let clock = self.clock;
        self.entries.get_mut(key).map(|entry| {
            entry.last_used = clock;
            entry.value.clone()
        })
    }

    /// Stores a value with its cost, replacing an old entry for the key, then evicts the least
    /// recently used entries until the total fits the budget.
    pub fn insert(&mut self, key: String, value: V, cost: usize) {
        self.remove(&key);
        if cost > self.limit {
            return;
        }
        self.clock += 1;
        self.total += cost;
        self.entries.insert(key, Entry { value, cost, last_used: self.clock });
        while self.total > self.limit {
            let oldest = self.entries.iter().min_by_key(|(_, entry)| entry.last_used).map(|(key, _)| key.clone());
            match oldest {
                Some(key) => {
                    self.remove(&key);
                }
                None => break,
            }
        }
    }

    pub fn remove(&mut self, key: &str) -> Option<V> {
        let entry = self.entries.remove(key)?;
        self.total -= entry.cost;
        Some(entry.value)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.total = 0;
    }

    /// Sum of the costs of the kept entries.
    pub fn total_cost(&self) -> usize {
        self.total
    }
}

/// Handed out by `RenditionCache::join`: either the caller starts the download, or one is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ticket {
    /// Nothing is downloading: the caller starts it and reports back with this ticket's number.
    Start(u64),
    /// A download is already running; the caller waits for its result (it counts as a waiter).
    Join,
}

struct Download {
    ticket: u64,
    epoch: u64,
    waiters: usize,
}

/// Memory caches of decoded renditions, request coalescing and invalidation.
pub struct RenditionCache<V> {
    thumbnails: MemoryCache<V>,
    previews: MemoryCache<V>,
    /// Bumped per key by `invalidate` and `store`; a download that started earlier is stale.
    epochs: HashMap<String, u64>,
    downloads: HashMap<String, Download>,
    next_ticket: u64,
}

impl<V: Clone> RenditionCache<V> {
    pub fn new() -> Self {
        Self::with_budgets(THUMBNAIL_BUDGET, PREVIEW_BUDGET)
    }

    pub fn with_budgets(thumbnail_budget: usize, preview_budget: usize) -> Self {
        Self {
            thumbnails: MemoryCache::new(thumbnail_budget),
            previews: MemoryCache::new(preview_budget),
            epochs: HashMap::new(),
            downloads: HashMap::new(),
            next_ticket: 1,
        }
    }

    fn pool_mut(&mut self, rendition: Rendition) -> &mut MemoryCache<V> {
        match rendition.pool() {
            Pool::Thumbnails => &mut self.thumbnails,
            Pool::Previews => &mut self.previews,
        }
    }

    /// Memory hit only. Never starts or joins a download (the Swift `cachedImage`).
    pub fn cached(&mut self, id: &str, rendition: Rendition) -> Option<V> {
        let key = key(id, rendition);
        self.pool_mut(rendition).get(&key)
    }

    /// Asks for a rendition that is not in memory: starts a download, or joins the one running.
    pub fn join(&mut self, id: &str, rendition: Rendition) -> Ticket {
        let key = key(id, rendition);
        if let Some(download) = self.downloads.get_mut(&key) {
            download.waiters += 1;
            return Ticket::Join;
        }
        let ticket = self.next_ticket;
        self.next_ticket += 1;
        let epoch = self.epoch_of(&key);
        self.downloads.insert(key, Download { ticket, epoch, waiters: 1 });
        Ticket::Start(ticket)
    }

    /// A waiter goes away (its cell scrolled out of reach). Returns `true` when it was the last
    /// one: the caller cancels the download, and its result is dropped by `finish`.
    pub fn leave(&mut self, id: &str, rendition: Rendition) -> bool {
        let key = key(id, rendition);
        let Some(download) = self.downloads.get_mut(&key) else {
            return false;
        };
        download.waiters -= 1;
        if download.waiters > 0 {
            return false;
        }
        self.downloads.remove(&key);
        true
    }

    /// Reports the download's result. `result` is the decoded image and its cost, or `None` when
    /// the download failed. The image is stored, and returned, only when this download is still
    /// the current one: not cancelled by `leave`, not overtaken by `invalidate` or `store`.
    /// Otherwise it is dropped.
    pub fn finish(&mut self, id: &str, rendition: Rendition, ticket: u64, result: Option<(V, usize)>) -> Option<V> {
        let key = key(id, rendition);
        let epoch = match self.downloads.get(&key) {
            Some(download) if download.ticket == ticket => download.epoch,
            _ => return None,
        };
        self.downloads.remove(&key);
        let (value, cost) = result?;
        if self.epoch_of(&key) != epoch {
            return None;
        }
        self.pool_mut(rendition).insert(key, value.clone(), cost);
        Some(value)
    }

    /// Seeds the cache with a locally rendered image (an edit shown before the server re-renders).
    /// Like an invalidation, it makes any running download of the rendition stale.
    pub fn store(&mut self, id: &str, rendition: Rendition, value: V, cost: usize) {
        let key = key(id, rendition);
        self.bump(&key);
        self.pool_mut(rendition).insert(key, value, cost);
    }

    /// Memory only, for images decoded from local files: they must not count as the server's
    /// rendition, so no epoch changes and no download is affected.
    pub fn remember(&mut self, id: &str, rendition: Rendition, value: V, cost: usize) {
        let key = key(id, rendition);
        self.pool_mut(rendition).insert(key, value, cost);
    }

    /// Drops every rendition of the assets from memory and makes their running downloads stale.
    /// Returns the disk file names the caller must delete.
    pub fn invalidate(&mut self, ids: &[&str]) -> Vec<String> {
        let mut files = Vec::new();
        for id in ids {
            for rendition in Rendition::ALL {
                let key = key(id, rendition);
                self.bump(&key);
                self.pool_mut(rendition).remove(&key);
                files.push(file_name(id, rendition));
            }
        }
        files
    }

    /// Empties both memory pools (the disk cache is the caller's).
    pub fn clear_all(&mut self) {
        self.thumbnails.clear();
        self.previews.clear();
    }

    /// Memory cost currently kept in a pool.
    pub fn memory_cost(&self, pool: Pool) -> usize {
        match pool {
            Pool::Thumbnails => self.thumbnails.total_cost(),
            Pool::Previews => self.previews.total_cost(),
        }
    }

    fn epoch_of(&self, key: &str) -> u64 {
        self.epochs.get(key).copied().unwrap_or(0)
    }

    fn bump(&mut self, key: &str) {
        *self.epochs.entry(key.to_string()).or_insert(0) += 1;
        self.downloads.remove(key);
    }
}

/// Tier-B thumbnails (`MiniBitmapStore`): 96 × 96 bitmaps in one memory-mapped file.
pub mod mini_bitmap {
    use std::collections::HashMap;

    /// Side of a tier-B bitmap in pixels.
    pub const SIDE: u32 = 96;
    /// Bytes of one slot (96 × 96 × 4).
    pub const SLOT_BYTES: usize = (SIDE * SIDE * 4) as usize;
    /// Slots per file chunk; the file grows one chunk at a time.
    pub const SLOTS_PER_CHUNK: usize = 8192;
    /// Bytes of one chunk (≈ 288 MB virtual, sparse on APFS).
    pub const CHUNK_BYTES: usize = SLOTS_PER_CHUNK * SLOT_BYTES;
    /// Longest side the decoder is asked for: 4 × the side, so the scaled-down bitmap stays sharp.
    pub const DECODE_LONG_SIDE: u32 = SIDE * 4;

    /// `(chunk, byte offset within the chunk)` of a slot.
    pub fn locate(slot: usize) -> (usize, usize) {
        (slot / SLOTS_PER_CHUNK, (slot % SLOTS_PER_CHUNK) * SLOT_BYTES)
    }

    /// How many chunks must be mapped before `slot` can be written.
    pub fn chunks_for_slot(slot: usize) -> usize {
        slot / SLOTS_PER_CHUNK + 1
    }

    /// The aspect-fill rectangle `(x, y, width, height)` of an image of the given size in the
    /// 96 × 96 square: the image is scaled to cover the square and centred, so it overflows on
    /// its longer axis and gets cropped there.
    pub fn aspect_fill_rect(width: u32, height: u32) -> (f64, f64, f64, f64) {
        let side = SIDE as f64;
        let scale = (side / width as f64).max(side / height as f64);
        let (drawn_w, drawn_h) = (width as f64 * scale, height as f64 * scale);
        ((side - drawn_w) / 2.0, (side - drawn_h) / 2.0, drawn_w, drawn_h)
    }

    /// Asset id → slot, plus the next free slot. Slots are write-once: an invalidated asset gets
    /// a fresh slot, so a bitmap still on screen is never overwritten.
    #[derive(Debug, Default, Clone, PartialEq, Eq)]
    pub struct SlotTable {
        slots: HashMap<String, usize>,
        next: usize,
    }

    impl SlotTable {
        /// Table from the stored rows (the database on startup).
        pub fn from_rows(rows: HashMap<String, usize>) -> Self {
            let next = rows.values().map(|slot| slot + 1).max().unwrap_or(0);
            Self { slots: rows, next }
        }

        /// The slot of a filled asset.
        pub fn slot(&self, id: &str) -> Option<usize> {
            self.slots.get(id).copied()
        }

        /// A fresh slot for an asset that has none, to be written and then `publish`ed. `None`
        /// when the asset already has a bitmap (write-once).
        pub fn allocate(&mut self, id: &str) -> Option<usize> {
            if self.slots.contains_key(id) {
                return None;
            }
            let slot = self.next;
            self.next += 1;
            Some(slot)
        }

        /// Makes a written slot visible to readers. Call only after the bytes are in place.
        pub fn publish(&mut self, id: &str, slot: usize) {
            self.slots.insert(id.to_string(), slot);
        }

        /// Forgets the bitmaps of re-rendered assets. Their slots are not reused.
        pub fn invalidate(&mut self, ids: &[&str]) {
            for id in ids {
                self.slots.remove(*id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renditions_map_to_swift_names_and_pools() {
        assert_eq!(Rendition::Thumbnail.as_str(), "thumbnail");
        assert_eq!(Rendition::Preview.as_str(), "preview");
        assert_eq!(Rendition::Local.as_str(), "local");
        assert_eq!(Rendition::Thumbnail.pool(), Pool::Thumbnails);
        assert_eq!(Rendition::Preview.pool(), Pool::Previews);
        assert_eq!(Rendition::Local.pool(), Pool::Previews);
        assert_eq!(file_name("a", Rendition::Preview), "a-preview.bin");
    }

    #[test]
    fn decoded_cost_is_bytes_per_row_times_height() {
        assert_eq!(decoded_cost(96, 96), 36_864);
        assert_eq!(decoded_cost(1206, 2622), 1206 * 4 * 2622);
    }

    #[test]
    fn download_overtaken_by_an_edit_is_not_cached() {
        // Port of testDownloadOvertakenByAnEditIsNotCached: the old image is dropped, the new one fetched.
        let mut cache: RenditionCache<&str> = RenditionCache::new();
        let ticket = start(cache.join("a", Rendition::Thumbnail));
        cache.invalidate(&["a"]); // the edit lands while the old image is on its way
        assert_eq!(cache.finish("a", Rendition::Thumbnail, ticket, Some(("old", 16))), None, "the overtaken download is dropped");
        assert_eq!(cache.cached("a", Rendition::Thumbnail), None, "and not cached");

        let ticket = start(cache.join("a", Rendition::Thumbnail));
        assert_eq!(cache.finish("a", Rendition::Thumbnail, ticket, Some(("new", 16))), Some("new"), "the new rendition is fetched");
        assert_eq!(cache.cached("a", Rendition::Thumbnail), Some("new"));
    }

    #[test]
    fn local_original_is_decoded_once_and_shared() {
        // Port of the sharing rule of testLocalOriginalIsDecodedSmallAndShared: the second
        // caller joins the first one's decode, and the result is kept for the viewer.
        let mut cache: RenditionCache<&str> = RenditionCache::new();
        let ticket = start(cache.join("local-1", Rendition::Local));
        assert_eq!(cache.join("local-1", Rendition::Local), Ticket::Join, "one decode for both callers");
        assert_eq!(cache.finish("local-1", Rendition::Local, ticket, Some(("decoded at 1387", 1387 * 4))), Some("decoded at 1387"));
        assert_eq!(cache.cached("local-1", Rendition::Local), Some("decoded at 1387"), "kept for the viewer's first frame");

        // An edit invalidates it: the next look decodes anew.
        cache.invalidate(&["local-1"]);
        assert_eq!(cache.cached("local-1", Rendition::Local), None);
    }

    #[test]
    fn stored_lookup_never_starts_a_download() {
        // Port of the intent of testStoredImageNeverUsesTheNetwork: a miss starts nothing.
        let mut cache: RenditionCache<u32> = RenditionCache::new();
        assert_eq!(cache.cached("missing", Rendition::Preview), None);
        assert!(matches!(cache.join("missing", Rendition::Preview), Ticket::Start(_)), "the lookup left no download behind");
    }

    #[test]
    fn last_waiter_leaving_cancels_the_download() {
        let mut cache: RenditionCache<u32> = RenditionCache::new();
        let ticket = start(cache.join("a", Rendition::Thumbnail));
        assert_eq!(cache.join("a", Rendition::Thumbnail), Ticket::Join);
        assert!(!cache.leave("a", Rendition::Thumbnail), "one waiter is still there");
        assert!(cache.leave("a", Rendition::Thumbnail), "the last one: cancel the download");
        assert_eq!(cache.finish("a", Rendition::Thumbnail, ticket, Some((1, 4))), None, "the cancelled result is dropped");

        let next = start(cache.join("a", Rendition::Thumbnail));
        assert_ne!(next, ticket, "a new request is a new download");
    }

    #[test]
    fn store_makes_a_running_download_stale() {
        let mut cache: RenditionCache<u32> = RenditionCache::new();
        let ticket = start(cache.join("a", Rendition::Preview));
        cache.store("a", Rendition::Preview, 7, 4);
        assert_eq!(cache.finish("a", Rendition::Preview, ticket, Some((1, 4))), None);
        assert_eq!(cache.cached("a", Rendition::Preview), Some(7), "the locally rendered edit stays");
    }

    #[test]
    fn remember_keeps_downloads_running() {
        let mut cache: RenditionCache<u32> = RenditionCache::new();
        let ticket = start(cache.join("a", Rendition::Thumbnail));
        cache.remember("a", Rendition::Thumbnail, 5, 4);
        assert_eq!(cache.finish("a", Rendition::Thumbnail, ticket, Some((6, 4))), Some(6), "remember does not invalidate");
    }

    #[test]
    fn failed_download_is_forgotten_and_retried() {
        let mut cache: RenditionCache<u32> = RenditionCache::new();
        let ticket = start(cache.join("a", Rendition::Thumbnail));
        assert_eq!(cache.finish("a", Rendition::Thumbnail, ticket, None), None);
        assert!(matches!(cache.join("a", Rendition::Thumbnail), Ticket::Start(_)));
    }

    #[test]
    fn invalidate_returns_the_disk_files_and_forgets_memory() {
        let mut cache: RenditionCache<u32> = RenditionCache::new();
        cache.remember("a", Rendition::Thumbnail, 1, 4);
        cache.remember("b", Rendition::Thumbnail, 2, 4);
        let files = cache.invalidate(&["a"]);
        assert_eq!(files, vec!["a-thumbnail.bin", "a-preview.bin", "a-local.bin"]);
        assert_eq!(cache.cached("a", Rendition::Thumbnail), None);
        assert_eq!(cache.cached("b", Rendition::Thumbnail), Some(2), "other assets are untouched");
        assert!(cache.invalidate(&[]).is_empty());
    }

    #[test]
    fn thumbnail_and_preview_budgets_are_separate() {
        // Budgets of 10 each: previews overflow, the grid's thumbnail stays.
        let mut cache: RenditionCache<u32> = RenditionCache::with_budgets(10, 10);
        cache.remember("t", Rendition::Thumbnail, 1, 6);
        cache.remember("p1", Rendition::Preview, 2, 6);
        cache.remember("p2", Rendition::Preview, 3, 6);
        assert_eq!(cache.cached("t", Rendition::Thumbnail), Some(1), "a swipe through previews leaves thumbnails alone");
        assert_eq!(cache.cached("p1", Rendition::Preview), None, "the least recently used preview went");
        assert_eq!(cache.cached("p2", Rendition::Preview), Some(3));
        assert_eq!(cache.memory_cost(Pool::Previews), 6);
    }

    #[test]
    fn least_recently_used_goes_first() {
        let mut cache: MemoryCache<u32> = MemoryCache::new(3);
        cache.insert("a".into(), 1, 1);
        cache.insert("b".into(), 2, 1);
        cache.insert("c".into(), 3, 1);
        assert_eq!(cache.get("a"), Some(1), "a is used again");
        cache.insert("d".into(), 4, 1);
        assert_eq!(cache.get("b"), None, "b was the least recently used");
        assert_eq!(cache.get("a"), Some(1));
        assert_eq!(cache.get("c"), Some(3));
        assert_eq!(cache.get("d"), Some(4));
        assert_eq!(cache.total_cost(), 3);
    }

    #[test]
    fn entry_larger_than_the_budget_is_not_kept() {
        let mut cache: MemoryCache<u32> = MemoryCache::new(10);
        cache.insert("big".into(), 1, 11);
        assert_eq!(cache.get("big"), None);
        assert_eq!(cache.total_cost(), 0);
    }

    #[test]
    fn clear_all_empties_both_pools() {
        let mut cache: RenditionCache<u32> = RenditionCache::new();
        cache.remember("a", Rendition::Thumbnail, 1, 4);
        cache.remember("a", Rendition::Preview, 2, 4);
        cache.clear_all();
        assert_eq!(cache.memory_cost(Pool::Thumbnails), 0);
        assert_eq!(cache.memory_cost(Pool::Previews), 0);
    }

    #[test]
    fn mini_bitmap_slots_are_write_once_and_never_reused() {
        use mini_bitmap::SlotTable;
        let mut table = SlotTable::default();
        let first = table.allocate("a").unwrap();
        table.publish("a", first);
        assert_eq!(table.allocate("a"), None, "write-once: a filled asset gets no new slot");
        assert_eq!(table.slot("a"), Some(first));

        table.invalidate(&["a"]);
        assert_eq!(table.slot("a"), None);
        let again = table.allocate("a").unwrap();
        assert_ne!(again, first, "the old slot may still be on screen");

        let mut known = SlotTable::from_rows([("x".to_string(), 41usize)].into_iter().collect());
        assert_eq!(known.allocate("y"), Some(42), "continues after the highest stored slot");
    }

    #[test]
    fn mini_bitmap_chunk_maths() {
        use mini_bitmap::*;
        assert_eq!(SLOT_BYTES, 36_864);
        assert_eq!(locate(0), (0, 0));
        assert_eq!(locate(8191), (0, 8191 * SLOT_BYTES));
        assert_eq!(locate(8192), (1, 0));
        assert_eq!(chunks_for_slot(0), 1);
        assert_eq!(chunks_for_slot(8192), 2);
        assert_eq!(DECODE_LONG_SIDE, 384);
    }

    #[test]
    fn mini_bitmap_aspect_fill_crops_the_longer_side() {
        // 400 × 200: scaled by 0.48 to 192 × 96, centred horizontally.
        let (x, y, w, h) = mini_bitmap::aspect_fill_rect(400, 200);
        assert!((x + 48.0).abs() < 1e-9 && y.abs() < 1e-9, "({x}, {y})");
        assert!((w - 192.0).abs() < 1e-9 && (h - 96.0).abs() < 1e-9);
        // Square: fills the square exactly.
        assert_eq!(mini_bitmap::aspect_fill_rect(50, 50), (0.0, 0.0, 96.0, 96.0));
    }

    /// Starts a download and returns its ticket number.
    fn start(ticket: Ticket) -> u64 {
        match ticket {
            Ticket::Start(number) => number,
            Ticket::Join => panic!("expected a new download"),
        }
    }
}
