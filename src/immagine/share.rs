//! Sharing assets from the viewer or the selection. The system sheet needs the complete files, so they are fetched
//! first (in pieces, the share button shows how far); stored downloads need no request. A second tap on the button,
//! leaving the photo or leaving select mode cancels before the sheet opens.

use super::downloads::{self, DOWNLOAD_DB};
use super::Immagine;
use crate::host::{Current as H, Host, ShareFile};
use crate::logic::client::ImmichClient;
use crate::logic::models::timeline::TimelineItem;
use crate::logic::store::Store;
use gpui_kit::*;
use std::collections::HashSet;

/// Largest total that is loaded into memory to be shared (web memory is 1 GiB and the bytes are copied on the way).
const SHARE_MAX_BYTES: u64 = 200_000_000;

/// Which button started a share, and so what must stay on screen for it to continue.
#[derive(Clone, PartialEq)]
pub(super) enum ShareFrom {
    /// The viewer, showing this asset.
    Viewer(String),
    Selection,
}

#[derive(Default)]
pub(super) struct ShareState {
    serial: u64,
    /// What is loading, with the fraction received.
    running: Option<(ShareFrom, f32)>,
    /// An error of the last share from the selection; shown instead of the selected count.
    pub(super) notice: Option<String>,
}

/// One asset to fetch; collected on the main thread, where the store is.
struct Job {
    id: String,
    video: bool,
    stored: bool,
    name: Option<String>,
}

impl Immagine {
    /// How far the share started by `from` is, while its files load.
    pub(super) fn share_fraction(&self, from: &ShareFrom) -> Option<f32> {
        self.share.running.as_ref().filter(|(f, _)| f == from).map(|(_, fraction)| *fraction)
    }

    /// Select mode ended: a share from it stops and its error goes.
    pub(super) fn share_selection_ended(&mut self) {
        self.share.notice = None;
        if self.share_fraction(&ShareFrom::Selection).is_some() {
            self.share.serial += 1;
            self.share.running = None;
        }
    }

    fn share_wanted(&self, serial: u64, from: &ShareFrom) -> bool {
        self.share.serial == serial
            && match from {
                ShareFrom::Viewer(id) => self.viewer_item().is_some_and(|item| item.id == *id),
                ShareFrom::Selection => self.select.on,
            }
    }

    /// Fetches `items` and opens the share sheet; a tap while the same button's share loads cancels it.
    pub(super) fn share_assets(&mut self, items: Vec<TimelineItem>, from: ShareFrom, cx: &mut Context<Self>) {
        self.share.serial += 1;
        self.share.notice = None;
        if self.share_fraction(&from).is_some() {
            self.share.running = None;
            cx.notify();
            return;
        }
        if items.is_empty() {
            return;
        }
        self.share.running = Some((from.clone(), 0.));
        cx.notify();
        let serial = self.share.serial;
        let (origin, token) = (self.origin.clone(), self.token.clone());
        let jobs: Vec<Job> = {
            let stored = self.downloads.ids.borrow();
            let store = self.engine.as_ref().map(|e| e.store());
            items
                .iter()
                .map(|item| Job {
                    id: item.id.clone(),
                    video: item.is_video(),
                    stored: stored.contains(&item.id),
                    name: store.and_then(|s| s.asset(&item.id)).and_then(|a| a.original_file_name),
                })
                .collect()
        };
        let keep_on = from.clone();
        cx.spawn(async move |this, cx| {
            let wanted = |cx: &mut AsyncApp| this.update(cx, |t, _| t.share_wanted(serial, &keep_on)).unwrap_or(false);
            let result = async {
                let count = jobs.len();
                let mut files = Vec::with_capacity(count);
                let mut used = 0u64;
                let too_big = || format!("Zu groß zum Teilen (über {} MB).", SHARE_MAX_BYTES / 1_000_000);
                for (i, job) in jobs.iter().enumerate() {
                    if !wanted(cx) {
                        return Ok(false);
                    }
                    let local = if job.stored { H::blob_get(DOWNLOAD_DB, &job.id).await.ok().flatten().filter(|b| !b.is_empty()) } else { None };
                    let bytes = match local {
                        Some(bytes) => bytes,
                        None => {
                            let url = downloads::asset_url(&origin, &job.id, job.video);
                            let fetched = downloads::fetch_ranged(cx, &url, |cx, done, total| {
                                if used + total > SHARE_MAX_BYTES {
                                    return Err(too_big());
                                }
                                let going = this
                                    .update(cx, |t, cx| {
                                        let going = t.share_wanted(serial, &keep_on);
                                        if let (true, Some(running)) = (going, t.share.running.as_mut()) {
                                            running.1 = (i as f32 + done as f32 / total as f32) / count as f32;
                                            cx.notify();
                                        }
                                        going
                                    })
                                    .unwrap_or(false);
                                Ok(going)
                            })
                            .await?;
                            let Some(bytes) = fetched else { return Ok(false) };
                            bytes
                        }
                    };
                    used += bytes.len() as u64;
                    if used > SHARE_MAX_BYTES {
                        return Err(too_big());
                    }
                    // Only while a sync has the engine the store could not name it.
                    let name = match &job.name {
                        Some(name) => Some(name.clone()),
                        None => {
                            let client = ImmichClient::new(&origin, H::transport()).map_err(|e| e.to_string())?;
                            client.asset(&job.id, &token).await.ok().and_then(|a| a.original_file_name)
                        }
                    };
                    let (name, mime) = share_name(name.as_deref(), &bytes, job.video);
                    files.push(ShareFile { name, mime, bytes });
                    let _ = this.update(cx, |t, cx| {
                        if let (true, Some(running)) = (t.share.serial == serial, t.share.running.as_mut()) {
                            running.1 = (i + 1) as f32 / count as f32;
                            cx.notify();
                        }
                    });
                }
                if !wanted(cx) {
                    return Ok(false);
                }
                unique_names(&mut files);
                // The sheet is about to open: the ring goes now, not when it is dismissed.
                let _ = this.update(cx, |t, cx| {
                    t.share.running = None;
                    cx.notify();
                });
                H::share_files(files).await.map(|()| true)
            }
            .await;
            let _ = this.update(cx, |t, cx| {
                if t.share.serial != serial {
                    return;
                }
                t.share.running = None;
                match result {
                    Ok(true) if from == ShareFrom::Selection => t.select_end(),
                    Ok(_) => {}
                    // Dismissing the sheet is reported as an error too; nothing to tell the user then.
                    Err(e) if e.contains("AbortError") => {}
                    Err(e) if from == ShareFrom::Selection => t.share.notice = Some(e),
                    Err(e) => t.status = e,
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// File name and MIME type for sharing `bytes`: the asset's own name where known, with the extension of what is
/// really sent (an edited/preview JPEG of a `.HEIC` is `.jpg`, a transcoded video `.mp4`).
fn share_name(original: Option<&str>, bytes: &[u8], video: bool) -> (String, &'static str) {
    let (ext, mime) = sniff(bytes).unwrap_or(if video { ("mp4", "video/mp4") } else { ("jpg", "image/jpeg") });
    let stem = original
        .map(|n| n.rsplit(['/', '\\']).next().unwrap_or(n))
        .map(|n| n.rsplit_once('.').map_or(n, |(stem, _)| stem))
        .filter(|s| !s.is_empty())
        .unwrap_or(if video { "video" } else { "foto" });
    (format!("{stem}.{ext}"), mime)
}

fn sniff(b: &[u8]) -> Option<(&'static str, &'static str)> {
    if b.starts_with(&[0xFF, 0xD8]) {
        Some(("jpg", "image/jpeg"))
    } else if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some(("png", "image/png"))
    } else if b.starts_with(b"GIF8") {
        Some(("gif", "image/gif"))
    } else if b.len() > 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some(("webp", "image/webp"))
    } else if b.len() > 12 && &b[4..8] == b"ftyp" {
        Some(match &b[8..12] {
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"heim" | b"heis" | b"mif1" | b"msf1" => ("heic", "image/heic"),
            b"avif" => ("avif", "image/avif"),
            b"qt  " => ("mov", "video/quicktime"),
            _ => ("mp4", "video/mp4"),
        })
    } else {
        None
    }
}

/// Two assets may have the same name (`foto.jpg`, or the same file name in different years); on a share they land in
/// one folder. The later ones become `foto-2.jpg`, `foto-3.jpg`.
fn unique_names(files: &mut [ShareFile]) {
    let mut seen = HashSet::new();
    for file in files {
        let (stem, ext) = file.name.rsplit_once('.').map_or((file.name.clone(), String::new()), |(s, e)| (s.to_string(), format!(".{e}")));
        let mut name = file.name.clone();
        let mut n = 1;
        while !seen.insert(name.to_lowercase()) {
            n += 1;
            name = format!("{stem}-{n}{ext}");
        }
        file.name = name;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn share_names_follow_the_bytes_sent() {
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let mut heic = vec![0, 0, 0, 24];
        heic.extend_from_slice(b"ftypheic0000");
        assert_eq!(share_name(Some("IMG_1.HEIC"), &jpeg, false), ("IMG_1.jpg".to_string(), "image/jpeg"));
        assert_eq!(share_name(Some("a/b.jpg"), &heic, false), ("b.heic".to_string(), "image/heic"));
        assert_eq!(share_name(Some("clip.MOV"), b"\0\0\0\x18ftypisom0000", true), ("clip.mp4".to_string(), "video/mp4"));
        assert_eq!(share_name(None, b"", true), ("video.mp4".to_string(), "video/mp4"));
    }

    #[::core::prelude::v1::test]
    fn names_of_one_share_stay_apart() {
        let file = |name: &str| ShareFile { name: name.to_string(), mime: "image/jpeg", bytes: Vec::new() };
        let mut files = vec![file("a.jpg"), file("b.jpg"), file("A.JPG"), file("a.jpg"), file("noext"), file("noext")];
        unique_names(&mut files);
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["a.jpg", "b.jpg", "A-2.JPG", "a-3.jpg", "noext", "noext-2"]);
    }
}
