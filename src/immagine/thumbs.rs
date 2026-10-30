//! Image store for grid thumbnails and viewer previews.
//!
//! GPUI's own `img(url)` decodes in WASM on the UI thread and keeps every decoded image forever. This store
//! loads through the browser's decoder (`Host::decode_url`), keeps decoded images in a byte-bounded
//! LRU, runs at most `MAX_ACTIVE` loads at once, and drops requests for tiles that scrolled away before their
//! turn. Images are `RenderImage`s (BGRA) that GPUI paints through `img(Arc<RenderImage>)`.

use super::Immagine;
use crate::logic::thumbhash;
use crate::host::{Current as H, Decoded, Host};
use gpui_kit::*;
use image::{Frame, RgbaImage};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

const MAX_ACTIVE: usize = 6;
/// Decoded pixels kept in memory (RGBA = 4 bytes per pixel).
const BYTE_BUDGET: usize = 160 * 1024 * 1024;
/// A queued request not asked for again within this many frames has scrolled out of sight.
const STALE_FRAMES: u64 = 4;
/// A failed load is retried after this many frames, at most `MAX_TRIES` times in all.
const RETRY_FRAMES: u64 = 40;
const MAX_TRIES: u8 = 4;

struct Entry {
    image: Arc<RenderImage>,
    stamp: u64,
    bytes: usize,
}

struct Job {
    key: String,
    url: String,
    frame: u64,
    seq: u64,
}

pub(super) struct Thumbs {
    images: HashMap<String, Entry>,
    pending: HashSet<String>,
    queue: Vec<Job>,
    /// Keys whose load failed: the frame it happened in and how often. They are retried a few times.
    failed: HashMap<String, (u64, u8)>,
    evicted: Vec<Arc<RenderImage>>,
    frame: u64,
    seq: u64,
    active: usize,
    bytes: usize,
    view: WeakEntity<Immagine>,
}

pub(super) type SharedThumbs = Rc<RefCell<Thumbs>>;

pub(super) fn new(view: WeakEntity<Immagine>) -> SharedThumbs {
    Rc::new(RefCell::new(Thumbs {
        images: HashMap::new(),
        pending: HashSet::new(),
        queue: Vec::new(),
        failed: HashMap::new(),
        evicted: Vec::new(),
        frame: 0,
        seq: 0,
        active: 0,
        bytes: 0,
        view,
    }))
}

impl Thumbs {
    /// Called once per render pass that asks for images; visible entries are re-stamped, scrolled-off ones age.
    pub(super) fn begin_frame(&mut self) {
        self.frame += 1;
    }

    /// Images evicted since the last call; the caller frees their atlas tiles with `Window::drop_image`.
    pub(super) fn take_evicted(&mut self) -> Vec<Arc<RenderImage>> {
        std::mem::take(&mut self.evicted)
    }

    fn insert(&mut self, key: String, image: Arc<RenderImage>, bytes: usize) {
        self.bytes += bytes;
        let stamp = self.frame;
        if let Some(old) = self.images.insert(key, Entry { image, stamp, bytes }) {
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

fn render_image(width: u32, height: u32, mut rgba: Vec<u8>) -> Option<Arc<RenderImage>> {
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

/// The decoded image for `key`, or `None` while it loads (the request is queued here).
pub(super) fn get(store: &SharedThumbs, key: &str, url: impl FnOnce() -> String, cx: &mut App) -> Option<Arc<RenderImage>> {
    {
        let mut s = store.borrow_mut();
        let frame = s.frame;
        if let Some(entry) = s.images.get_mut(key) {
            entry.stamp = frame;
            return Some(entry.image.clone());
        }
        if s.pending.contains(key) {
            return None;
        }
        if let Some(&(at, tries)) = s.failed.get(key) {
            // A failed load (network hiccup, decoder busy) is tried again a little later, a few times.
            if tries >= MAX_TRIES || frame < at + RETRY_FRAMES {
                return None;
            }
        }
        if let Some(job) = s.queue.iter_mut().find(|j| j.key == key) {
            job.frame = frame;
        } else {
            s.seq += 1;
            let job = Job { key: key.to_string(), url: url(), frame, seq: s.seq };
            s.queue.push(job);
        }
    }
    pump(store, cx);
    None
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
    s.insert(key, image.clone(), (w * h * 4) as usize);
    Some(image)
}

fn pump(store: &SharedThumbs, cx: &mut App) {
    loop {
        let job = {
            let mut s = store.borrow_mut();
            if s.active >= MAX_ACTIVE {
                return;
            }
            let frame = s.frame;
            s.queue.retain(|j| j.frame + STALE_FRAMES >= frame);
            // Newest frame first (what the user is looking at now), then request order.
            let next = s.queue.iter().enumerate().max_by_key(|(_, j)| (j.frame, std::cmp::Reverse(j.seq))).map(|(i, _)| i);
            let Some(index) = next else { return };
            let job = s.queue.swap_remove(index);
            s.pending.insert(job.key.clone());
            s.active += 1;
            job
        };
        let store = store.clone();
        cx.spawn(async move |cx| {
            let result = H::decode_url(&job.url).await;
            let _ = cx.update(|app| finish(&store, job.key, result, app));
        })
        .detach();
    }
}

fn finish(store: &SharedThumbs, key: String, result: Result<Decoded, String>, cx: &mut App) {
    let view = {
        let mut s = store.borrow_mut();
        s.active -= 1;
        s.pending.remove(&key);
        match result.ok().and_then(|d| render_image(d.width, d.height, d.rgba).map(|i| (i, (d.width * d.height * 4) as usize))) {
            Some((image, bytes)) => {
                s.failed.remove(&key);
                s.insert(key, image, bytes)
            }
            None => {
                let frame = s.frame;
                let tries = s.failed.get(&key).map_or(0, |f| f.1) + 1;
                s.failed.insert(key, (frame, tries));
            }
        }
        s.view.clone()
    };
    pump(store, cx);
    view.update(cx, |_, cx| cx.notify()).ok();
}
