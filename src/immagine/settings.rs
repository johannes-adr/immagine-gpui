//! Settings sheet, opened by the gear at the top right of Sammlungen: what the app stores and how it runs.

use super::{DEBUG_KEY, downloads::DOWNLOAD_DB, thumbs::{self, THUMB_CACHE_DB}, video_player::DISK_DB, Immagine, DB};
use crate::components::list::Group;
use crate::components::sheet::{self, Sheet};
use crate::host::{AppInfo, Current as H, Host};
use crate::theme::Palette;
use gpui_kit::*;

#[derive(Default)]
pub(super) struct SettingsState {
    pub(super) sheet: Sheet,
    /// Per-category usage; `None` until measured.
    store: Option<Usage>,
    video: Option<Usage>,
    thumbs: Option<Usage>,
    downloads: Option<Usage>,
    /// Downloads: bare keys (originals/videos), `t:` thumbnails, `p:` previews.
    dl_parts: Option<(usize, usize, usize)>,
    info: Option<AppInfo>,
    pub(super) hidden: super::hidden_date::HiddenState,
}

/// Entry count and bytes of one blob db.
#[derive(Clone, Copy)]
struct Usage {
    count: usize,
    bytes: u64,
}

async fn usage(db: &str) -> (Option<Usage>, Vec<String>) {
    let keys = H::blob_keys(db).await.unwrap_or_default();
    let bytes = H::blob_size(db).await.ok();
    (bytes.map(|bytes| Usage { count: keys.len(), bytes }), keys)
}

fn usage_text(u: Option<Usage>) -> String {
    match u {
        None => "…".into(),
        Some(u) => format!("{} · {}", u.count, size(u.bytes)),
    }
}

/// 1 536 -> "1,5 KB".
pub(super) fn size(n: u64) -> String {
    let n = n as f64;
    let (value, unit) = if n < 1024. {
        return format!("{} B", n as u64);
    } else if n < 1024. * 1024. {
        (n / 1024., "KB")
    } else if n < 1024. * 1024. * 1024. {
        (n / (1024. * 1024.), "MB")
    } else {
        (n / (1024. * 1024. * 1024.), "GB")
    };
    format!("{value:.1} {unit}").replace('.', ",")
}

impl Immagine {
    pub(super) fn open_settings(&mut self, cx: &mut Context<Self>) {
        self.settings.sheet.open();
        self.hidden_on_open();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let (store, _) = usage(DB).await;
            let (video, _) = usage(DISK_DB).await;
            let (thumbs, _) = usage(THUMB_CACHE_DB).await;
            let (downloads, keys) = usage(DOWNLOAD_DB).await;
            let thumb_n = keys.iter().filter(|k| k.starts_with("t:")).count();
            let preview_n = keys.iter().filter(|k| k.starts_with("p:")).count();
            let bare_n = keys.iter().filter(|k| !k.contains(':')).count();
            let info = H::app_info().await;
            let _ = this.update(cx, |t, cx| {
                t.settings.store = store;
                t.settings.video = video;
                t.settings.thumbs = thumbs;
                t.settings.downloads = downloads;
                t.settings.dl_parts = Some((bare_n, thumb_n, preview_n));
                t.settings.info = Some(info);
                cx.notify();
            });
        })
        .detach();
    }

    /// Empties the rebuildable thumbnail cache and re-measures it.
    fn clear_thumb_cache(&mut self, cx: &mut Context<Self>) {
        thumbs::clear_cache(&self.thumbs);
        cx.spawn(async move |this, cx| {
            let _ = H::blob_clear(THUMB_CACHE_DB).await;
            let (thumbs, _) = usage(THUMB_CACHE_DB).await;
            let _ = this.update(cx, |t, cx| {
                t.settings.thumbs = thumbs;
                cx.notify();
            });
        })
        .detach();
    }

    fn close_settings(&mut self, cx: &mut Context<Self>) {
        self.settings.sheet.close();
        cx.notify();
    }

    pub(super) fn settings_overlay(&self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let sheet = &self.settings.sheet;
        if !sheet.visible() {
            return None;
        }
        let ui = &p.ui;
        let st = &self.settings;
        let unknown = || "…".to_string();
        let info = st.info.clone();
        let storage = Group::new(ui, "Speicher")
            .row(
                "Belegt / Kapazität",
                match info.as_ref().map(|i| (i.storage_used, i.storage_quota)) {
                    None => unknown(),
                    Some((Some(used), Some(quota))) => format!("{} / {}", size(used), size(quota)),
                    Some((Some(used), None)) => size(used),
                    Some(_) => "unbekannt".into(),
                },
            )
            .row("Mediathek (Sync-Stand)", usage_text(st.store))
            // Counted in assets (bare keys); the bytes include their stored thumbnails and previews.
            .row("Downloads", usage_text(st.downloads.map(|u| Usage { count: st.dl_parts.map_or(u.count, |d| d.0), ..u })))
            .row("  davon Thumbnails", st.dl_parts.map_or_else(unknown, |(_, t, _)| t.to_string()))
            .row("  davon Vorschauen", st.dl_parts.map_or_else(unknown, |(_, _, pr)| pr.to_string()))
            .row("Thumbnail-Cache", usage_text(st.thumbs))
            .action("settings-clear-thumbs", "Thumbnail-Cache leeren", ui.accent, cx.listener(|this, _, _, cx| this.clear_thumb_cache(cx)))
            .row("Video-Cache", usage_text(st.video))
            .build();
        let display = self.hidden_rows(p, cx).into_iter().fold(Group::new(ui, "Anzeige"), |g, row| g.raw(row)).build();
        let native = info.as_ref().is_some_and(|i| i.native);
        let mut app = Group::new(ui, "App");
        if native {
            app = app.row("Modus", "Native App").row("Renderer", "Metal");
        } else {
            app = app
                .row(
                    "PWA-Version",
                    match &info {
                        None => unknown(),
                        Some(i) => i.pwa_version.clone().unwrap_or_else(|| "nicht installiert".into()),
                    },
                )
                .row("Modus", info.as_ref().map_or_else(unknown, |i| if i.standalone { "Installiert".into() } else { "Browser".into() }))
                .row("Renderer", if self.webgpu { "WebGPU" } else { "WebGL (langsamer)" });
        }
        let app = app.row("Threads", info.as_ref().map_or_else(unknown, |i| if i.threads { "An".into() } else { "Aus".into() })).build();
        let debug_group = Group::new(ui, "Debug")
            .toggle(
                "settings-debug",
                "FPS anzeigen",
                self.debug,
                cx.listener(|this, _, _, cx| {
                    this.debug = !this.debug;
                    H::setting_set(DEBUG_KEY, if this.debug { "1" } else { "0" });
                    cx.notify();
                }),
            )
            .build();
        let account = Group::new(ui, "Konto").action("settings-logout", "Abmelden", ui.destructive, cx.listener(|this, _, _, cx| this.log_out(cx))).build();
        let inner = div()
            .px_4()
            .pt_2()
            .pb(px(self.bottom.max(16.)))
            .flex()
            .flex_col()
            .gap_6()
            .child(display)
            .child(storage)
            .child(app)
            .child(debug_group)
            .child(account);
        Some(sheet::overlay(
            sheet,
            |this: &Immagine| &this.settings.sheet,
            cx,
            window,
            sheet::Spec { id: "settings", title: "Einstellungen", top_margin: self.top.max(12.) + 10., theme: ui },
            inner,
            |this, cx| this.close_settings(cx),
        ))
    }
}
