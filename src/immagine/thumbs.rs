//! Image store for grid thumbnails and viewer previews.
//!
//! GPUI's own `img(url)` decodes in WASM on the UI thread and keeps every decoded image forever. This store
//! loads through the browser's decoder (`Host::fetch_image` + `Host::decode_bytes`, scaled to the cell size), keeps
//! decoded images in a byte-bounded LRU, runs at most `MAX_ACTIVE` loads at once, drops requests for tiles that
//! scrolled away before their turn, aborts running grid thumbnail loads that scrolled away, holds back new loads
//! during a fast fling, and keeps the encoded thumbnails in a persistent cache (`THUMB_CACHE_DB`). Images are
//! `RenderImage`s (BGRA) that GPUI paints through `img(Arc<RenderImage>)`.

use super::{downloads::DOWNLOAD_DB, Immagine};
use crate::logic::thumbhash;
use crate::host::{Cancel, Current as H, Decoded, Host};
use gpui_kit::*;
use image::{Frame, RgbaImage};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// Encoded `t:` thumbnails (WebP, a few KB each) persisted across launches, key `t:<id>:<thumbhash>`.
pub(crate) const THUMB_CACHE_DB: &str = "thumb-cache";
/// Entries kept in `THUMB_CACHE_DB`; beyond it the oldest inserted are deleted.
const CACHE_CAP: usize = 40_000;
/// Scroll speed (points per ms) above which no new loads start, and how fresh the speed sample must be.
const FLING_SPEED: f32 = 3.;
const FLING_FRESH_MS: f64 = 100.;
/// A held store wakes up again after this long to start the loads of the tiles that stayed visible.
const WAKE_MS: u64 = 150;
/// A gap between scroll samples longer than this was a pause, not a speed.
const SAMPLE_GAP_MS: f64 = 100.;

/// Loads running at once, and how many of them may use the network. Loads from the persistent cache or a stored
/// download are cheap (no request) and are not held back by a fling.
const MAX_ACTIVE: usize = 16;
const MAX_NET: usize = 6;
/// Decoded pixels kept in memory (RGBA = 4 bytes per pixel).
const BYTE_BUDGET: usize = 160 * 1024 * 1024;
/// A queued request not asked for again within this many frames has scrolled out of sight.
const STALE_FRAMES: u64 = 4;
/// A failed load is retried after this long (times the number of tries so far), at most `MAX_TRIES` times in all.
/// Time, not frames: views that do not render continuously (Alben) would otherwise never retry.
const RETRY_MS: f64 = 1000.;
const MAX_TRIES: u8 = 4;

struct Entry {
    image: Arc<RenderImage>,
    stamp: u64,
    bytes: usize,
    /// Short side this was decoded for (0 = full size).
    side: u32,
}

struct Job {
    key: String,
    url: String,
    /// Wanted short side (0 = full size).
    side: u32,
    frame: u64,
    seq: u64,
    /// Prefetch for tiles not on screen yet: always behind visible requests, promoted when a tile asks for it.
    low: bool,
}

/// A load in flight: its cancel token and the last frame something asked for it.
struct Running {
    cancel: Cancel,
    frame: u64,
    net: bool,
}

/// Keys of `THUMB_CACHE_DB`, kept in memory so a miss never touches IndexedDB. `known` holds the names as
/// `norm` maps them (the desktop store turns `:` into `_`), `order` the keys as the store knows them, oldest first.
#[derive(Default)]
struct CacheIndex {
    known: HashSet<String>,
    order: VecDeque<String>,
}

fn norm(key: &str) -> String {
    key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect()
}

/// An entry decoded for `have` serves a request for `want` (0 = full size) if it is at least that big.
fn enough(have: u32, want: u32) -> bool {
    have == 0 || (want != 0 && have >= want)
}

/// The bigger of two wanted sides (0 = full size is the biggest).
fn bigger(a: u32, b: u32) -> u32 {
    if a == 0 || b == 0 { 0 } else { a.max(b) }
}

/// Decode size for a tile of `cell` points: its pixel size rounded up to a bucket, so a pinch does not ask for a
/// new size every frame. Above the biggest bucket (the server thumbnail is ~250 px) the full image.
pub(super) fn side_for(cell: f32, scale: f32) -> u32 {
    let px = (cell * scale).ceil().max(1.) as u32;
    [64, 96, 128, 192].into_iter().find(|b| px <= *b).unwrap_or(0)
}

fn encode_component(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `{origin}/api/assets/{id}/thumbnail?size={size}&c={thumbhash}`: the hash is the cache key of the response (an
/// edit changes it), left out when the asset has none.
pub(super) fn thumb_url(origin: &str, id: &str, size: &str, hash: Option<&str>) -> String {
    let mut url = format!("{origin}/api/assets/{id}/thumbnail?size={size}");
    if let Some(hash) = hash.filter(|h| !h.is_empty()) {
        url.push_str("&c=");
        url.push_str(&encode_component(hash));
    }
    url
}

/// Persistent-cache key of a grid thumbnail load: `t:<id>:<encoded thumbhash>`; `None` for other renditions.
fn cache_key(key: &str, url: &str) -> Option<String> {
    key.starts_with("t:").then(|| {
        let hash = url.split_once("&c=").map_or("", |(_, h)| h.split('&').next().unwrap_or(""));
        format!("{key}:{hash}")
    })
}

pub(super) struct Thumbs {
    images: HashMap<String, Entry>,
    running: HashMap<String, Running>,
    queue: Vec<Job>,
    /// Keys whose load failed: when it happened (ms) and how often. They are retried a few times.
    failed: HashMap<String, (f64, u8)>,
    evicted: Vec<Arc<RenderImage>>,
    frame: u64,
    seq: u64,
    active: usize,
    net_active: usize,
    bytes: usize,
    view: WeakEntity<Immagine>,
    /// Assets kept in IndexedDB (shared with `Downloads`): their renditions are read from there before the network.
    local: Rc<RefCell<HashSet<String>>>,
    cache: CacheIndex,
    /// Last scroll sample (offset, time) and the speed measured from the last two.
    scroll: Option<(f32, f64)>,
    speed: f32,
    speed_at: f64,
    wake_pending: bool,
    /// The last real scroll movement was towards the top of the list (offset grew); prefetch looks that way.
    up: bool,
}

pub(super) type SharedThumbs = Rc<RefCell<Thumbs>>;

pub(super) fn new(view: WeakEntity<Immagine>, local: Rc<RefCell<HashSet<String>>>) -> SharedThumbs {
    Rc::new(RefCell::new(Thumbs {
        images: HashMap::new(),
        running: HashMap::new(),
        queue: Vec::new(),
        failed: HashMap::new(),
        evicted: Vec::new(),
        frame: 0,
        seq: 0,
        active: 0,
        net_active: 0,
        bytes: 0,
        view,
        local,
        cache: CacheIndex::default(),
        scroll: None,
        speed: 0.,
        speed_at: f64::MIN,
        wake_pending: false,
        up: false,
    }))
}

impl Thumbs {
    /// Called once per render pass that asks for images; visible entries are re-stamped, scrolled-off ones age.
    pub(super) fn begin_frame(&mut self) {
        self.frame += 1;
        // Grid thumbnails nobody asks for any more are aborted; previews and full sizes run to the end.
        for (key, r) in &self.running {
            if key.starts_with("t:") && r.frame + STALE_FRAMES < self.frame {
                r.cancel.cancel();
            }
        }
    }

    /// Reports the grid's scroll offset (once per frame). Fast movement holds back new loads (`held`).
    pub(super) fn track_scroll(&mut self, offset: f32) {
        let now = H::now_ms();
        let Some((prev, at)) = self.scroll else {
            self.scroll = Some((offset, now));
            return;
        };
        let dt = now - at;
        if dt < 1. {
            return;
        }
        // Keep the old direction while standing still.
        if offset != prev {
            self.up = offset > prev;
        }
        self.speed = if dt > SAMPLE_GAP_MS { 0. } else { (offset - prev).abs() / dt as f32 };
        self.speed_at = now;
        self.scroll = Some((offset, now));
    }

    /// Whether the user last scrolled towards the start of the list (false = down or unknown).
    pub(super) fn moving_up(&self) -> bool {
        self.up
    }

    /// Whether the job can be served without the network (a stored download or the persistent thumbnail cache).
    fn cheap(&self, key: &str, url: &str) -> bool {
        let local = key.split_once(':').is_some_and(|(_, id)| self.local.borrow().contains(id));
        local || cache_key(key, url).is_some_and(|k| self.cache.known.contains(&norm(&k)))
    }

    fn held(&self) -> bool {
        self.speed > FLING_SPEED && H::now_ms() - self.speed_at < FLING_FRESH_MS
    }

    /// Images evicted since the last call; the caller frees their atlas tiles with `Window::drop_image`.
    pub(super) fn take_evicted(&mut self) -> Vec<Arc<RenderImage>> {
        std::mem::take(&mut self.evicted)
    }

    /// Drops every decoded rendition of asset `id` (after an edit), so the next frame loads them again.
    pub(super) fn forget(&mut self, id: &str) {
        for prefix in ["t", "p", "f"] {
            if let Some(e) = self.images.remove(&format!("{prefix}:{id}")) {
                self.bytes -= e.bytes;
                self.evicted.push(e.image);
            }
        }
    }

    fn fail(&mut self, key: &str) {
        let tries = self.failed.get(key).map_or(0, |f| f.1) + 1;
        self.failed.insert(key.to_string(), (H::now_ms(), tries));
    }

    fn insert(&mut self, key: String, image: Arc<RenderImage>, bytes: usize, side: u32) {
        self.bytes += bytes;
        let stamp = self.frame;
        if let Some(old) = self.images.insert(key, Entry { image, stamp, bytes, side }) {
            self.bytes -= old.bytes;
            self.evicted.push(old.image);
        }
        while self.bytes > BYTE_BUDGET {
            // Oldest entry that is not in the last couple of frames.
            let oldest = self.images.iter().filter(|(_, e)| e.stamp + 2 < self.frame).min_by_key(|(_, e)| e.stamp).map(|(k, _)| k.clone());
            let Some(key) = oldest else { break };
            if let Some(e) = self.images.remove(&key) {
                self.bytes -= e.bytes;
                self.evicted.push(e.image);
            }
        }
    }
}

pub(super) fn render_image(width: u32, height: u32, mut rgba: Vec<u8>) -> Option<Arc<RenderImage>> {
    // GPUI paints BGRA.
    for px in rgba.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    let buffer = RgbaImage::from_raw(width, height, rgba)?;
    Some(Arc::new(RenderImage::new(vec![Frame::new(buffer)])))
}

/// The decoded image for `key` if it is already there. Unlike `get` it never starts a load, so it is safe
/// during an animation where a decode on the main thread would drop frames.
pub(super) fn peek(store: &SharedThumbs, key: &str) -> Option<Arc<RenderImage>> {
    let mut s = store.borrow_mut();
    let frame = s.frame;
    let entry = s.images.get_mut(key)?;
    entry.stamp = frame;
    Some(entry.image.clone())
}

/// The decoded image for `key` at full size, or `None` while it loads (the request is queued here).
pub(super) fn get(store: &SharedThumbs, key: &str, url: impl FnOnce() -> String, cx: &mut App) -> Option<Arc<RenderImage>> {
    get_sized(store, key, 0, url, cx)
}

/// Like `get`, decoded with the short side at least `min_side` (see `side_for`; 0 = full size). A smaller entry
/// is returned meanwhile (drawn upscaled) while the bigger one loads. During a fast fling nothing new is queued.
pub(super) fn get_sized(store: &SharedThumbs, key: &str, min_side: u32, url: impl FnOnce() -> String, cx: &mut App) -> Option<Arc<RenderImage>> {
    let have = {
        let mut s = store.borrow_mut();
        let frame = s.frame;
        let have = s.images.get_mut(key).map(|entry| {
            entry.stamp = frame;
            (entry.image.clone(), entry.side)
        });
        if let Some((image, side)) = &have {
            if enough(*side, min_side) {
                return Some(image.clone());
            }
        }
        let have = have.map(|(image, _)| image);
        if let Some(r) = s.running.get_mut(key) {
            r.frame = frame;
            return have;
        }
        if let Some(&(at, tries)) = s.failed.get(key) {
            // A failed load (network hiccup, decoder busy) is tried again a little later, a few times.
            if tries >= MAX_TRIES || H::now_ms() < at + RETRY_MS * tries as f64 {
                return have;
            }
        }
        if let Some(job) = s.queue.iter_mut().find(|j| j.key == key) {
            job.frame = frame;
            job.low = false;
            job.side = bigger(job.side, min_side);
        } else {
            let url = url();
            if s.held() && !s.cheap(key, &url) {
                let wake = !std::mem::replace(&mut s.wake_pending, true);
                drop(s);
                if wake {
                    schedule_wake(store, cx);
                }
                return have;
            }
            s.seq += 1;
            let job = Job { key: key.to_string(), url, side: min_side, frame, seq: s.seq, low: false };
            s.queue.push(job);
        }
        have
    };
    pump(store, cx);
    have
}

/// Queues low-priority loads for tiles just beyond the visible rows, so they are there when scrolled in. `jobs` are
/// `(asset id, url builder)`; the key is `t:<id>` and the url is only built for jobs that really get queued. Does
/// nothing during a fling or while a visible tile is still waiting. Called every frame while idle, which also keeps
/// the queued and running prefetch jobs alive (the staleness rule is the same as for visible requests).
pub(super) fn prefetch<'a, F: FnOnce() -> String>(
    store: &SharedThumbs,
    jobs: impl Iterator<Item = (&'a str, F)>,
    side: u32,
    cx: &mut App,
) {
    {
        let mut s = store.borrow_mut();
        if s.held() || s.queue.iter().any(|j| !j.low) {
            return;
        }
        let frame = s.frame;
        let mut key = String::new();
        for (id, url) in jobs {
            key.clear();
            key.push_str("t:");
            key.push_str(id);
            if s.images.get(&key).is_some_and(|e| enough(e.side, side)) {
                continue;
            }
            if let Some(r) = s.running.get_mut(&key) {
                r.frame = frame;
                continue;
            }
            if let Some(job) = s.queue.iter_mut().find(|j| j.key == key) {
                job.frame = frame;
                job.side = bigger(job.side, side);
                continue;
            }
            if s.failed.contains_key(&key) {
                continue;
            }
            s.seq += 1;
            let job = Job { key: key.clone(), url: url(), side, frame, seq: s.seq, low: true };
            s.queue.push(job);
        }
    }
    pump(store, cx);
}

/// Re-renders the view after `ms`, so tiles whose load failed ask for their image again.
fn schedule_retry(store: &SharedThumbs, ms: f64, cx: &mut App) {
    let view = store.borrow().view.clone();
    cx.spawn(async move |cx| {
        cx.background_executor().timer(Duration::from_millis(ms as u64)).await;
        view.update(cx, |_, cx| cx.notify()).ok();
    })
    .detach();
}

/// Notifies the view after a moment, so the loads held back during a fling start once it slowed down.
fn schedule_wake(store: &SharedThumbs, cx: &mut App) {
    let store = store.clone();
    let view = store.borrow().view.clone();
    cx.spawn(async move |cx| {
        cx.background_executor().timer(Duration::from_millis(WAKE_MS)).await;
        store.borrow_mut().wake_pending = false;
        view.update(cx, |_, cx| cx.notify()).ok();
    })
    .detach();
}

/// The tiny blurred stand-in for an asset, decoded from its thumbhash (cheap, cached like any image).
pub(super) fn placeholder(store: &SharedThumbs, id: &str, hash: &str) -> Option<Arc<RenderImage>> {
    let key = format!("h:{id}");
    let mut s = store.borrow_mut();
    let frame = s.frame;
    if let Some(entry) = s.images.get_mut(&key) {
        entry.stamp = frame;
        return Some(entry.image.clone());
    }
    let (rgba, w, h) = thumbhash::decode(hash)?;
    let image = render_image(w, h, rgba)?;
    s.insert(key, image.clone(), (w * h * 4) as usize, 0);
    Some(image)
}

fn pump(store: &SharedThumbs, cx: &mut App) {
    loop {
        let (job, cancel) = {
            let mut s = store.borrow_mut();
            if s.active >= MAX_ACTIVE {
                return;
            }
            let frame = s.frame;
            s.queue.retain(|j| j.frame + STALE_FRAMES >= frame);
            // Network loads wait for a fling to slow and for a free connection slot; cheap ones (cache, downloads) run.
            let held = s.held();
            let net_free = s.net_active < MAX_NET;
            // Visible requests before prefetch, then newest frame first (what the user is looking at now), then request order.
            let next = s
                .queue
                .iter()
                .enumerate()
                .filter(|(_, j)| s.cheap(&j.key, &j.url) || (!held && net_free))
                .max_by_key(|(_, j)| (!j.low, j.frame, std::cmp::Reverse(j.seq)))
                .map(|(i, _)| i);
            let Some(index) = next else {
                // If the fling stops dead no frame would start the waiting network loads otherwise.
                let wake = held && !s.queue.is_empty() && !std::mem::replace(&mut s.wake_pending, true);
                drop(s);
                if wake {
                    schedule_wake(store, cx);
                }
                return;
            };
            let job = s.queue.swap_remove(index);
            let net = !s.cheap(&job.key, &job.url);
            let cancel = Cancel::new();
            s.running.insert(job.key.clone(), Running { cancel: cancel.clone(), frame: job.frame, net });
            s.active += 1;
            s.net_active += net as usize;
            (job, cancel)
        };
        let store = store.clone();
        // Key `t:<id>` / `p:<id>` / `f:<id>`: a downloaded asset has its thumbnail and preview stored under the same
        // keys and its original under the bare id (the full-size rendition is decoded from it).
        let local = job.key.split_once(':').filter(|(_, id)| store.borrow().local.borrow().contains(*id)).map(|(kind, id)| {
            let blob = if kind == "f" { id.to_string() } else { job.key.clone() };
            blob
        });
        let cached = cache_key(&job.key, &job.url).filter(|k| store.borrow().cache.known.contains(&norm(k)));
        let put_key = cache_key(&job.key, &job.url);
        cx.spawn(async move |cx| {
            let result = load(&store, &job, local, cached, &cancel).await;
            let (result, fresh) = match result {
                Ok((decoded, fresh)) => {
                    // The per-pixel BGRA swap runs on a worker (on the main thread when there are none).
                    let bytes = (decoded.width * decoded.height * 4) as usize;
                    let image = cx.background_spawn(async move { render_image(decoded.width, decoded.height, decoded.rgba) }).await;
                    (image.map(|i| (i, bytes)).ok_or_else(|| "bad image".to_string()), fresh)
                }
                Err(e) => (Err(e), None),
            };
            let _ = cx.update(|app| finish(&store, &job.key, job.side, result, &cancel, app));
            // Fire and forget: the next launch finds the thumbnail without the network.
            if let (Some(key), Some(bytes)) = (put_key, fresh) {
                cache_put(&store, key, bytes).await;
            }
        })
        .detach();
    }
}

/// Where the pixels come from: a stored download, the persistent thumbnail cache, then the network. The last one
/// also hands back the encoded bytes (only for cacheable thumbnails) so the caller can persist them.
async fn load(
    store: &SharedThumbs,
    job: &Job,
    local: Option<String>,
    cached: Option<String>,
    cancel: &Cancel,
) -> Result<(Decoded, Option<Vec<u8>>), String> {
    if let Some(blob) = local {
        if let Ok(Some(bytes)) = H::blob_get(DOWNLOAD_DB, &blob).await {
            if !bytes.is_empty() {
                if let Ok(decoded) = H::decode_bytes(bytes, job.side).await {
                    return Ok((decoded, None));
                }
            }
        }
    }
    if let Some(key) = cached {
        match H::blob_get(THUMB_CACHE_DB, &key).await {
            Ok(Some(bytes)) if !bytes.is_empty() => {
                if let Ok(decoded) = H::decode_bytes(bytes, job.side).await {
                    return Ok((decoded, None));
                }
            }
            _ => {}
        }
        // Gone or unreadable: forget it and fetch again.
        store.borrow_mut().cache.known.remove(&norm(&key));
    }
    let bytes = H::fetch_image(&job.url, cancel).await?;
    if cancel.is_cancelled() {
        return Err("cancelled".into());
    }
    let keep = cache_key(&job.key, &job.url).map(|_| bytes.clone());
    let decoded = H::decode_bytes(bytes, job.side).await?;
    Ok((decoded, keep))
}

/// Persists an encoded thumbnail and trims the cache to `CACHE_CAP` entries, oldest inserted first.
async fn cache_put(store: &SharedThumbs, key: String, bytes: Vec<u8>) {
    if H::blob_put(THUMB_CACHE_DB, &key, &bytes).await.is_err() {
        return;
    }
    let evict: Vec<String> = {
        let mut s = store.borrow_mut();
        if s.cache.known.insert(norm(&key)) {
            s.cache.order.push_back(key);
        }
        let mut evict = Vec::new();
        while s.cache.known.len() > CACHE_CAP {
            let Some(old) = s.cache.order.pop_front() else { break };
            s.cache.known.remove(&norm(&old));
            evict.push(old);
        }
        evict
    };
    for old in evict {
        let _ = H::blob_delete(THUMB_CACHE_DB, &old).await;
    }
}

/// Reads which thumbnails the persistent cache holds; called once at start (until then every load goes to the network).
pub(super) fn init_cache(store: &SharedThumbs, cx: &mut App) {
    let store = store.clone();
    cx.spawn(async move |_| {
        let keys = H::blob_keys(THUMB_CACHE_DB).await.unwrap_or_default();
        let mut s = store.borrow_mut();
        for key in keys {
            if s.cache.known.insert(norm(&key)) {
                s.cache.order.push_back(key);
            }
        }
    })
    .detach();
}

/// Forgets the cached keys (logout; the caller clears the db itself).
pub(super) fn clear_cache(store: &SharedThumbs) {
    store.borrow_mut().cache = CacheIndex::default();
}

fn finish(store: &SharedThumbs, key: &str, side: u32, result: Result<(Arc<RenderImage>, usize), String>, cancel: &Cancel, cx: &mut App) {
    let mut retry = None;
    let view = {
        let mut s = store.borrow_mut();
        s.active -= 1;
        if s.running.remove(key).is_some_and(|r| r.net) {
            s.net_active -= 1;
        }
        match result {
            Ok((image, bytes)) => {
                s.failed.remove(key);
                s.insert(key.to_string(), image, bytes, side);
            }
            // An aborted load is not a failure: the tile is simply asked for again if it comes back.
            Err(_) if cancel.is_cancelled() => {}
            Err(_) => {
                s.fail(key);
                retry = Some(RETRY_MS * s.failed.get(key).map_or(1, |f| f.1) as f64 + 50.);
            }
        }
        s.view.clone()
    };
    if let Some(wait) = retry {
        schedule_retry(store, wait, cx);
    }
    pump(store, cx);
    view.update(cx, |_, cx| cx.notify()).ok();
}
