//! Permanent deletion: "Papierkorb leeren" on the Papierkorb page and "Endgültig löschen" for one photo in the
//! viewer. Neither goes through the album outbox: the server cannot undo it, so there is nothing to apply first and
//! take back later. The server is asked first; only when it agrees are the rows dropped from the store.

use super::downloads::DOWNLOAD_DB;
use super::Immagine;
use crate::components::morph::{self, Morph};
use crate::components::measure;
use crate::host::{Current as H, Host};
use crate::logic::client::ImmichClient;
use crate::util::now_ms;
use gpui_kit::*;
use std::cell::Cell;
use std::rc::Rc;

/// The "Papierkorb leeren" confirmation on the Papierkorb page.
#[derive(Default)]
pub(super) struct PurgeState {
    pub(super) morph: Morph,
    /// Where the "Leeren" chip is on screen (measured every frame), the morph's anchor.
    pub(super) button: Rc<Cell<Option<morph::Rect>>>,
    busy: bool,
}

const CARD: (f32, f32) = (292., 214.);

/// `ids` = `None` empties the whole trash.
pub(super) async fn delete_forever(origin: &str, token: &str, ids: Option<Vec<String>>) -> Result<(), String> {
    let client = ImmichClient::new(origin, H::transport()).map_err(|e| e.to_string())?;
    match ids {
        None => client.empty_trash(token).await,
        Some(ids) => client.delete_assets_forever(&ids, token).await,
    }
    .map_err(|e| e.to_string())
}

impl Immagine {
    /// The server confirmed: drops the rows and their stored downloads, refreshes the album pages, stores the snapshot.
    pub(super) fn purged(&mut self, ids: &[String], cx: &mut Context<Self>) {
        use crate::logic::store::Store;
        let Some(engine) = self.engine.as_mut() else { return };
        engine.store_mut().delete_assets(ids);
        self.albums.refresh(engine.store());
        let snapshot = engine.store().snapshot().ok();
        {
            let mut stored = self.downloads.ids.borrow_mut();
            for id in ids {
                stored.remove(id);
            }
        }
        let ids = ids.to_vec();
        cx.notify();
        cx.spawn(async move |_, _| {
            for id in ids {
                for key in [id.clone(), format!("t:{id}"), format!("p:{id}")] {
                    let _ = H::blob_delete(DOWNLOAD_DB, &key).await;
                }
            }
            if let Some(bytes) = snapshot {
                let _ = H::blob_put(super::DB, super::SNAPSHOT_KEY, &bytes).await;
            }
        })
        .detach();
    }

    pub(super) fn open_purge(&mut self, cx: &mut Context<Self>) {
        if self.purge.button.get().is_some() && self.purge.morph.open(now_ms()) {
            cx.notify();
        }
    }

    pub(super) fn close_purge(&mut self, cx: &mut Context<Self>) {
        if !self.purge.morph.close(now_ms()) {
            return;
        }
        cx.notify();
        Morph::finish_after(cx, |t: &mut Self| Some(&mut t.purge.morph));
    }

    fn empty_trash(&mut self, cx: &mut Context<Self>) {
        use crate::logic::store::Store;
        if self.purge.busy {
            return;
        }
        let Some(engine) = self.engine.as_ref() else { return };
        // The server empties its whole trash, which may hold more than the store knows.
        let known: Vec<String> = engine.store().trashed_items().into_iter().map(|i| i.id).collect();
        self.purge.busy = true;
        self.purge.morph.reset();
        let (origin, token) = (self.origin.clone(), self.token.clone());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = delete_forever(&origin, &token, None).await;
            let _ = this.update(cx, |t, cx| {
                t.purge.busy = false;
                match result {
                    Ok(()) => t.purged(&known, cx),
                    Err(e) => t.status = format!("Papierkorb konnte nicht geleert werden: {e}"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Confirmation card morphing out of the "Leeren" chip.
    pub(super) fn purge_overlay(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let m = self.purge.morph;
        if !m.active() {
            return None;
        }
        let (ax, ay, aw, ah) = self.purge.button.get()?;
        let size = window.viewport_size();
        let vw: f32 = size.width.into();
        let (bw, bh) = CARD;
        let content = morph::confirm_content(
            "purge-confirm",
            "Alles im Papierkorb wird endgültig gelöscht. Das lässt sich nicht rückgängig machen.",
            "Papierkorb leeren",
            rgb(0xffffff).into(),
            rgb(0xff5a52).into(),
            m.is_open().then(|| cx.listener(|this, _, _, cx| this.empty_trash(cx))),
        );
        let spec = morph::Spec {
            id: "purge",
            anchor: (ax, ay, aw, ah),
            card: ((ax + aw - bw).max(16.).min(vw - 16. - bw), ay, bw, bh),
            tint: morph::DIALOG_TINT,
            anchor_radius: ah / 2.,
            card_radius: 34.,
        };
        morph::overlay(m, spec, now_ms(), window, content, |this: &mut Self, cx| this.close_purge(cx), cx)
    }

    /// The "Leeren" chip of the Papierkorb page.
    pub(super) fn purge_chip(&self, p: &crate::theme::Palette, cx: &mut Context<Self>) -> AnyElement {
        use crate::components::button::{self, Surface};
        if self.purge.morph.active() {
            // The chip is gone while the card morphs out of it; keep its place.
            let (w, h) = self.purge.button.get().map_or((86., 36.), |(_, _, w, h)| (w, h));
            return div().w(px(w)).h(px(h)).into_any_element();
        }
        let rect = self.purge.button.clone();
        button::chip("trash-empty", Surface::new(p.glass_tint), 36., 16., "Leeren")
            .relative()
            .text_color(rgb(crate::theme::RED))
            .text_size(px(16.))
            .active(button::dim)
            .child(measure::bounds(move |b| rect.set(Some(measure::rect(b)))))
            .on_click(cx.listener(|this, _, _, cx| this.open_purge(cx)))
            .into_any_element()
    }
}
