//! Settings sheet, opened by the gear at the top right of Sammlungen: what the app stores and how it runs.

use super::{downloads::DOWNLOAD_DB, video_player::DISK_DB, Immagine, DB};
use crate::components::sheet::{self, Sheet};
use crate::host::{AppInfo, Current as H, Host, Pinch};
use crate::theme::Palette;
use gpui_kit::*;

#[derive(Default)]
pub(super) struct SettingsState {
    pub(super) sheet: Sheet,
    /// Bytes in the store snapshot db and in the video cache db; `None` until measured.
    store_bytes: Option<u64>,
    video_bytes: Option<u64>,
    download_bytes: Option<u64>,
    info: Option<AppInfo>,
}

/// 1 536 -> "1,5 KB".
fn size(n: u64) -> String {
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
    pub(super) fn open_settings(&mut self, cx: &mut Context<Self>) {
        self.settings.sheet.open();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let store = H::blob_size(DB).await.ok();
            let video = H::blob_size(DISK_DB).await.ok();
            let downloads = H::blob_size(DOWNLOAD_DB).await.ok();
            let info = H::app_info().await;
            let _ = this.update(cx, |t, cx| {
                t.settings.store_bytes = store;
                t.settings.video_bytes = video;
                t.settings.download_bytes = downloads;
                t.settings.info = Some(info);
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
                row(p, "Mediathek (IndexedDB)", st.store_bytes.map(size).unwrap_or_else(unknown), false),
                row(p, "Video-Cache (IndexedDB)", st.video_bytes.map(size).unwrap_or_else(unknown), false),
                row(p, "Downloads (IndexedDB)", st.download_bytes.map(size).unwrap_or_else(unknown), false),
            ],
        );
        let app = group(
            card,
            "App",
            p,
            vec![
                row(
                    p,
                    "PWA-Version",
                    match &info {
                        None => unknown(),
                        Some(i) => i.pwa_version.clone().unwrap_or_else(|| "nicht installiert".into()),
                    },
                    true,
                ),
                row(p, "Modus", info.as_ref().map_or_else(unknown, |i| if i.standalone { "Installiert".into() } else { "Browser".into() }), false),
                row(p, "Renderer", if self.webgpu { "WebGPU".into() } else { "WebGL (langsamer)".into() }, false),
                row(p, "Threads", info.as_ref().map_or_else(unknown, |i| if i.threads { "An".into() } else { "Aus".into() }), false),
            ],
        );
        let content = div()
            .px_4()
            .pb(px(self.bottom.max(16.)))
            .flex()
            .flex_col()
            .gap_6()
            .child(storage)
            .child(app)
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
