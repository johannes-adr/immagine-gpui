//! Settings sheet, opened by the gear at the top right of Sammlungen: what the app stores and how it runs.

use super::{DEBUG_KEY, downloads::DOWNLOAD_DB, thumbs::{self, THUMB_CACHE_DB}, video_player::DISK_DB, Immagine, DB};
use crate::components::rubber_band::RubberBand;
use crate::components::sheet::{self, Sheet};
use crate::host::{AppInfo, Current as H, Host, Pinch};
use crate::theme::{Palette, BLUE};
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
    /// The content list below the title strip and its own overscroll band (the page's band stays put behind
    /// the scrim).
    scroll: ScrollHandle,
    pub(super) rubber: RubberBand,
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

fn row(p: &Palette, label: &str, value: String, first: bool) -> Div {
    let line = if first { div() } else { div().border_t_1().border_color(p.separator) };
    line
        .min_h(px(48.))
        .px_4()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .text_size(px(17.))
        .child(div().child(label.to_string()))
        .child(div().text_color(p.secondary).child(value))
}

fn group(card: Rgba, caption: &str, p: &Palette, rows: Vec<Div>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().px_4().text_size(px(13.)).text_color(p.secondary).child(caption.to_uppercase()))
        .child(div().rounded(px(22.)).bg(card).overflow_hidden().flex().flex_col().children(rows))
}

impl Immagine {
    /// A touch scroll step while the sheet is shown (from the root's `rubber_band::input`). The content scrolls
    /// like any `overflow_y_scroll` div (GPUI turns the pan into scroll events at the finger); this only adds the
    /// sheet's own rubber band, and swallows the steps while the title strip drags the sheet, so the list does
    /// not scroll along with it.
    pub(super) fn settings_scroll_event(&mut self, dy: f32, phase: TouchPhase, window: &mut Window) -> (bool, bool) {
        if self.settings.sheet.dragging() {
            window.cancel_touch_momentum();
            return (true, false);
        }
        let scroll = self.settings.scroll.clone();
        self.settings.rubber.event(dy, phase, Some(&scroll), window)
    }

    pub(super) fn open_settings(&mut self, cx: &mut Context<Self>) {
        self.settings.sheet.open();
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

    /// Raw single-finger events (the touch path of the sheet's drag).
    pub(super) fn settings_pinch(&mut self, event: Pinch, cx: &mut Context<Self>) {
        let sheet = &self.settings.sheet;
        match event {
            Pinch::Down { y, .. } if sheet.hit(y) => sheet.grab(y),
            Pinch::Drag { y, .. } => sheet.drag_to(y),
            Pinch::Up { y, .. } => {
                if y >= 0. {
                    sheet.drag_to(y);
                }
                sheet.release();
            }
            _ => return,
        }
        cx.notify();
    }

    pub(super) fn settings_overlay(&self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let sheet = &self.settings.sheet;
        if !sheet.visible() {
            return None;
        }
        let (background, card) = if self.dark { (rgb(0x1c1c1e), rgb(0x2c2c2e)) } else { (rgb(0xf2f2f7), rgb(0xffffff)) };
        let st = &self.settings;
        let unknown = || "…".to_string();
        let info = st.info.clone();
        let storage = group(
            card,
            "Speicher",
            p,
            vec![
                row(
                    p,
                    "Belegt / Kapazität",
                    match info.as_ref().map(|i| (i.storage_used, i.storage_quota)) {
                        None => unknown(),
                        Some((Some(used), Some(quota))) => format!("{} / {}", size(used), size(quota)),
                        Some((Some(used), None)) => size(used),
                        Some(_) => "unbekannt".into(),
                    },
                    true,
                ),
                row(p, "Mediathek (Sync-Stand)", usage_text(st.store), false),
                // Counted in assets (bare keys); the bytes include their stored thumbnails and previews.
                row(p, "Downloads", usage_text(st.downloads.map(|u| Usage { count: st.dl_parts.map_or(u.count, |d| d.0), ..u })), false),
                row(p, "  davon Thumbnails", st.dl_parts.map_or_else(unknown, |(_, t, _)| t.to_string()), false),
                row(p, "  davon Vorschauen", st.dl_parts.map_or_else(unknown, |(_, _, pr)| pr.to_string()), false),
                row(p, "Thumbnail-Cache", usage_text(st.thumbs), false),
                div().border_t_1().border_color(p.separator).child(
                    div()
                    .id("settings-clear-thumbs")
                    .min_h(px(48.))
                    .px_4()
                    .flex()
                    .items_center()
                    .text_size(px(17.))
                    .text_color(rgb(BLUE))
                    .active(|s| s.opacity(0.6))
                    .child("Thumbnail-Cache leeren")
                    .on_click(cx.listener(|this, _, _, cx| this.clear_thumb_cache(cx))),
                ),
                row(p, "Video-Cache", usage_text(st.video), false),
            ],
        );
        let native = info.as_ref().is_some_and(|i| i.native);
        let mut app_rows = Vec::new();
        if native {
            app_rows.push(row(p, "Modus", "Native App".into(), true));
            app_rows.push(row(p, "Renderer", "Metal".into(), false));
        } else {
            app_rows.push(row(
                p,
                "PWA-Version",
                match &info {
                    None => unknown(),
                    Some(i) => i.pwa_version.clone().unwrap_or_else(|| "nicht installiert".into()),
                },
                true,
            ));
            app_rows.push(row(p, "Modus", info.as_ref().map_or_else(unknown, |i| if i.standalone { "Installiert".into() } else { "Browser".into() }), false));
            app_rows.push(row(p, "Renderer", if self.webgpu { "WebGPU".into() } else { "WebGL (langsamer)".into() }, false));
        }
        app_rows.push(row(p, "Threads", info.as_ref().map_or_else(unknown, |i| if i.threads { "An".into() } else { "Aus".into() }), false));
        let app = group(card, "App", p, app_rows);
        let debug = self.debug;
        let toggle = div().border_t_1().border_color(p.separator).child(
            div()
                .id("settings-debug")
                .min_h(px(48.))
                .px_4()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .text_size(px(17.))
                .active(|s| s.opacity(0.6))
                .child("FPS anzeigen")
                .child(
                    {
                        let track = div().w(px(51.)).h(px(31.)).rounded_full().p(px(2.)).flex();
                        if debug { track.justify_end() } else { track }
                    }
                        .bg(if debug { rgb(0x34c759) } else { rgb(0x78788033) })
                        .child(div().size(px(27.)).rounded_full().bg(rgb(0xffffff))),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.debug = !this.debug;
                    H::setting_set(DEBUG_KEY, if this.debug { "1" } else { "0" });
                    cx.notify();
                })),
        );
        let debug_group = group(card, "Debug", p, vec![div().child(toggle)]);
        let account = group(
            card,
            "Konto",
            p,
            vec![div().child(
                div()
                    .id("settings-logout")
                    .min_h(px(48.))
                    .px_4()
                    .flex()
                    .items_center()
                    .text_size(px(17.))
                    .text_color(rgb(0xff3b30))
                    .active(|s| s.opacity(0.6))
                    .child("Abmelden")
                    .on_click(cx.listener(|this, _, _, cx| this.log_out(cx))),
            )],
        );
        let inner = div()
            .px_4()
            .pt_2()
            .pb(px(self.bottom.max(16.)))
            .flex()
            .flex_col()
            .gap_6()
            .child(storage)
            .child(app)
            .child(debug_group)
            .child(account);
        // Scrolls below the title strip (absolute with top + bottom = a definite height, so `max_offset` is the
        // overflow); the sheet's own rubber band gives it the iOS overscroll.
        let content = div()
            .id("settings-scroll")
            .absolute()
            .top(px(sheet::HEADER))
            .left_0()
            .right_0()
            .bottom_0()
            .overflow_y_scroll()
            .track_scroll(&st.scroll)
            .child(st.rubber.shift(inner))
            .into_any_element();
        Some(sheet::overlay(
            sheet,
            |this: &Immagine| &this.settings.sheet,
            cx,
            window,
            self.top.max(12.) + 10.,
            (background, p.glass_tint, p.label),
            "Einstellungen",
            content,
            |this, cx| this.close_settings(cx),
        ))
    }
}
