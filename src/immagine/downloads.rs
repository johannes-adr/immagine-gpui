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
        let base = format!("{}/api/assets/{id}", self.origin);
        // A video is stored as the H.264 version the player streams (the original may be HEVC, which it cannot play).
        let url = if item.is_video() { format!("{base}/video/playback") } else { format!("{base}/original?edited=true") };
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = async {
                // The grid's and the viewer's renditions, so the photo shows offline too (best effort).
                for (key, size) in [("t", "thumbnail"), ("p", "preview")] {
                    if let Ok(bytes) = H::fetch_bytes(&format!("{base}/thumbnail?size={size}")).await {
                        let _ = H::blob_put(DOWNLOAD_DB, &format!("{key}:{id}"), &bytes).await;
                    }
                }
                let first = H::fetch_range(&url, 0, CHUNK - 1).await?;
                let total = first.total.filter(|_| first.partial);
                let mut bytes = first.bytes;
                // A server that ignored `Range` sent everything at once.
                while let Some(total) = total.filter(|t| (bytes.len() as u64) < *t) {
                    let progress = bytes.len() as f32 / total as f32;
                    let _ = this.update(cx, |t, cx| {
                        t.downloads.busy.insert(id.clone(), progress);
                        cx.notify();
                    });
                    let start = bytes.len() as u64;
                    let piece = H::fetch_range(&url, start, (start + CHUNK).min(total) - 1).await?;
                    if piece.bytes.is_empty() {
                        return Err("Download unvollständig".to_string());
                    }
                    bytes.extend(piece.bytes);
                }
                H::blob_put(DOWNLOAD_DB, &id, &bytes).await
            }
            .await;
            let _ = this.update(cx, |t, cx| {
                t.downloads.busy.remove(&id);
                match result {
                    Ok(()) => {
                        t.downloads.ids.borrow_mut().insert(id);
                    }
                    Err(e) => t.status = format!("Download fehlgeschlagen: {e}"),
                }
                cx.notify();
            });
        })
        .detach();
    }
}
