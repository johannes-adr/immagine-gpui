//! Downloads: the original of a photo kept in IndexedDB (db `downloads`, key = asset id). The viewer's button
//! stores one, the grid marks the stored ones.

use super::Immagine;
use crate::host::{Current as H, Host};
use gpui_kit::*;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

pub(super) const DOWNLOAD_DB: &str = "downloads";
/// The original arrives in pieces of this size, so the button can show progress.
const CHUNK: u64 = 2 * 1024 * 1024;

#[derive(Default)]
pub(super) struct Downloads {
    /// Ids stored in the db; shared with the grid's row builder.
    pub(super) ids: Rc<RefCell<HashSet<String>>>,
    /// Ids being fetched right now, with the fraction received (0 while the size is not known yet).
    pub(super) busy: HashMap<String, f32>,
}

impl Immagine {
    /// Reads which assets are stored; called once at start.
    pub(super) fn load_downloads(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let keys = H::blob_keys(DOWNLOAD_DB).await.unwrap_or_default();
            let _ = this.update(cx, |t, cx| {
                // Keys with a colon are the stored thumbnails and previews of a download.
                t.downloads.ids.borrow_mut().extend(keys.into_iter().filter(|k| !k.contains(':')));
                // Jobs continue only now, so assets stored earlier are skipped instead of fetched again.
                t.jobs_load(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Tapping the button of a stored photo removes it again; of a plain one, fetches and stores its original.
    /// Does nothing while that photo's download is on its way.
    pub(super) fn download_current(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.viewer_item() else { return };
        let id = item.id.clone();
        if self.downloads.busy.contains_key(&id) {
            return;
        }
        if self.downloads.ids.borrow_mut().remove(&id) {
            cx.notify();
            cx.spawn(async move |_, _| {
                for key in [id.clone(), format!("t:{id}"), format!("p:{id}")] {
                    let _ = H::blob_delete(DOWNLOAD_DB, &key).await;
                }
            })
            .detach();
            return;
        }
        self.downloads.busy.insert(id.clone(), 0.);
        let origin = self.origin.clone();
        let video = item.is_video();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = fetch_and_store(&this, cx, &origin, &id, video, |_| true).await;
            let _ = this.update(cx, |t, cx| {
                if let Err(e) = result {
                    t.status = format!("Download fehlgeschlagen: {e}");
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// The file of an asset: a video is the H.264 version the player streams (the original may be HEVC, which it cannot
/// play), a photo its original.
pub(super) fn asset_url(origin: &str, id: &str, video: bool) -> String {
    let base = format!("{origin}/api/assets/{id}");
    if video { format!("{base}/video/playback") } else { format!("{base}/original?edited=true") }
}

/// Downloads `url` in pieces. `piece(received, total)` runs before each further piece and can stop (`Ok(false)`, the
/// result is then `None`) or fail. A server that ignores `Range` sends everything at once, without any `piece` call.
pub(super) async fn fetch_ranged(
    cx: &mut AsyncApp,
    url: &str,
    mut piece: impl FnMut(&mut AsyncApp, u64, u64) -> Result<bool, String>,
) -> Result<Option<Vec<u8>>, String> {
    let first = H::fetch_range(url, 0, CHUNK - 1).await?;
    let total = first.total.filter(|_| first.partial);
    let mut bytes = first.bytes;
    while let Some(total) = total.filter(|t| (bytes.len() as u64) < *t) {
        if !piece(cx, bytes.len() as u64, total)? {
            return Ok(None);
        }
        let start = bytes.len() as u64;
        let next = H::fetch_range(url, start, (start + CHUNK).min(total) - 1).await?;
        if next.bytes.is_empty() {
            return Err("Download unvollständig".to_string());
        }
        bytes.extend(next.bytes);
    }
    Ok(Some(bytes))
}

/// How a download ended.
pub(super) enum Fetched {
    /// Stored with this many bytes (the original; thumbnail and preview not counted).
    Stored(u64),
    /// `keep_going` said stop between two pieces; nothing was stored.
    Stopped,
}

/// Fetches an asset's thumbnail, preview and original (a video: its H.264 playback version) and stores them in
/// the `downloads` db, reporting progress in `Downloads.busy` and adding the id to `Downloads.ids` at the end.
/// `keep_going` is asked between the pieces of the original (pausing a job stops a big video mid-way).
pub(super) async fn fetch_and_store(
    this: &WeakEntity<Immagine>,
    cx: &mut AsyncApp,
    origin: &str,
    id: &str,
    video: bool,
    keep_going: impl Fn(&Immagine) -> bool,
) -> Result<Fetched, String> {
    let base = format!("{origin}/api/assets/{id}");
    let url = asset_url(origin, id, video);
    let progress = |cx: &mut AsyncApp, value: Option<f32>| {
        let _ = this.update(cx, |t, cx| {
            match value {
                Some(v) => {
                    t.downloads.busy.insert(id.to_string(), v);
                }
                None => {
                    t.downloads.busy.remove(id);
                }
            }
            cx.notify();
        });
    };
    let go = |cx: &mut AsyncApp| this.update(cx, |t, _| keep_going(t)).unwrap_or(false);
    progress(cx, Some(0.));
    let result = async {
        // The grid's and the viewer's renditions, so the photo shows offline too (best effort).
        for (key, size) in [("t", "thumbnail"), ("p", "preview")] {
            if let Ok(bytes) = H::fetch_bytes(&format!("{base}/thumbnail?size={size}")).await {
                let _ = H::blob_put(DOWNLOAD_DB, &format!("{key}:{id}"), &bytes).await;
            }
        }
        let fetched = fetch_ranged(cx, &url, |cx, done, total| {
            if !go(cx) {
                return Ok(false);
            }
            progress(cx, Some(done as f32 / total as f32));
            Ok(true)
        })
        .await?;
        let Some(bytes) = fetched else { return Ok(Fetched::Stopped) };
        H::blob_put(DOWNLOAD_DB, id, &bytes).await?;
        Ok(Fetched::Stored(bytes.len() as u64))
    }
    .await;
    progress(cx, None);
    if let Ok(Fetched::Stored(_)) = result {
        let _ = this.update(cx, |t, cx| {
            t.downloads.ids.borrow_mut().insert(id.to_string());
            cx.notify();
        });
    }
    result
}
