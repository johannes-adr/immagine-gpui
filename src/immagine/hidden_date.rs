//! "Fotos anzeigen ab": a per-device date before which photos are not shown anywhere in the app.
//!
//! The filter lives in `MemoryStore` (`set_hidden_before`): every query (timeline, favorites, albums and their
//! counts and covers, people pages, range views, mini bitmaps) skips the rows, but the rows stay in the store and
//! keep syncing, so lowering or clearing the date brings everything back at once, offline, without a resync.
//! Thumbnails are only requested for tiles that are shown, so hidden photos cost no image traffic.
//! This module holds the setting (`immagine.hidden_before`, `YYYY-MM-DD`), applies it to the store and the lists
//! derived from it, and draws the settings rows with a small stepper picker (no keyboard, no native date picker).

use super::Immagine;
use crate::host::{Current as H, Host};
use crate::logic::hidden_before::{Field, Ymd, MONTHS};
use crate::logic::ports::Clock;
use crate::logic::store::Store;
use crate::theme::{Palette, BLUE};
use gpui_kit::*;
use std::rc::Rc;

pub(super) const HIDDEN_KEY: &str = "immagine.hidden_before";

fn today() -> Ymd {
    Ymd::from_millis(H::clock().now_ms())
}

pub(super) struct HiddenState {
    /// The applied limit; `None` = show everything.
    pub(super) date: Option<Ymd>,
    /// Picker expanded, and the date it edits (applied only by "Übernehmen").
    open: bool,
    draft: Ymd,
    /// Photos the limit hides right now (cached: a scan of every row, done when something changes).
    pub(super) count: usize,
}

impl Default for HiddenState {
    fn default() -> Self {
        let date = H::setting_get(HIDDEN_KEY).and_then(|raw| Ymd::parse(raw.trim()));
        HiddenState { date, open: false, draft: date.unwrap_or_else(today), count: 0 }
    }
}

impl Immagine {
    /// The store's cutoff for the current setting (epoch ms, 0 = no limit).
    pub(super) fn hidden_cutoff(&self) -> i64 {
        self.settings.hidden.date.map_or(0, |d| d.cutoff_millis())
    }

    /// Opens the settings sheet's picker with the current value and measures what is hidden.
    pub(super) fn hidden_on_open(&mut self) {
        let h = &mut self.settings.hidden;
        h.open = false;
        h.draft = h.date.unwrap_or_else(today);
        self.hidden_recount();
    }

    pub(super) fn hidden_recount(&mut self) {
        self.settings.hidden.count = self.engine.as_ref().map_or(0, |e| e.store().hidden_count());
    }

    /// After a sync, which ran with the engine away from the UI: the setting may have changed meanwhile, and the
    /// store may have new rows.
    pub(super) fn hidden_after_sync(&mut self) {
        let cutoff = self.hidden_cutoff();
        let Some(engine) = self.engine.as_mut() else {
            return;
        };
        if engine.store().hidden_before() != cutoff {
            engine.store_mut().set_hidden_before(cutoff);
            self.hidden_rebuild();
        } else {
            self.hidden_recount();
        }
    }

    /// Saves and applies a new limit (`None` = all). One rebuild of everything derived from the store.
    fn set_hidden_date(&mut self, date: Option<Ymd>, cx: &mut Context<Self>) {
        match date {
            Some(d) => H::setting_set(HIDDEN_KEY, &d.format()),
            None => H::setting_remove(HIDDEN_KEY),
        }
        let h = &mut self.settings.hidden;
        h.date = date;
        h.open = false;
        h.draft = date.unwrap_or_else(today);
        let cutoff = self.hidden_cutoff();
        // While a sync runs the engine is away; `hidden_after_sync` applies the cutoff when it returns.
        if let Some(engine) = self.engine.as_mut() {
            engine.store_mut().set_hidden_before(cutoff);
            self.hidden_rebuild();
        }
        cx.notify();
    }

    /// Rebuilds the lists derived from the store after the cutoff changed: timeline items, albums (counts,
    /// covers, the open album page), range views, selection, search and person results; the grids start at the
    /// newest photo again so no stale scroll geometry is left.
    fn hidden_rebuild(&mut self) {
        let Some(engine) = self.engine.as_ref() else {
            return;
        };
        let mut items = engine.store().fetch_timeline();
        items.reverse();
        self.items = Rc::new(items);
        self.albums.refresh(engine.store());
        self.hidden_recount();
        *self.range_cache.borrow_mut() = None;
        self.select_end();
        self.timeline_placed.set(false);
        self.years_placed.set(false);
        self.months_placed.set(false);
    }

    /// Server-side results (search, a person's photos) are fetched, not stored: ask again.
    fn hidden_refetch(&mut self, cx: &mut Context<Self>) {
        if self.search.open {
            self.start_search(cx);
        }
        if let Some(id) = self.people.open.as_ref().map(|page| page.person.id.clone()) {
            self.load_person_photos(id, cx);
        }
    }

    fn apply_hidden_draft(&mut self, date: Option<Ymd>, cx: &mut Context<Self>) {
        self.set_hidden_date(date, cx);
        self.hidden_refetch(cx);
    }

    /// The rows of the settings group "Anzeige": the value row, the picker (when open) and the hint line.
    pub(super) fn hidden_rows(&self, p: &Palette, cx: &mut Context<Self>) -> Vec<Div> {
        let h = &self.settings.hidden;
        let separator = |d: Div| d.border_t_1().border_color(p.separator);
        let mut rows = Vec::new();

        let value = h.date.map_or_else(|| "Alle".to_string(), |d| d.german());
        rows.push(
            div().child(
                div()
                    .id("hidden-row")
                    .min_h(px(48.))
                    .px_4()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .text_size(px(17.))
                    .active(|s| s.opacity(0.6))
                    .child(div().child("Fotos anzeigen ab"))
                    .child(div().text_color(if h.open { rgb(BLUE) } else { p.secondary }).child(value))
                    .on_click(cx.listener(|this, _, window, cx| {
                        if window.tap_caught_fling() {
                            return;
                        }
                        let h = &mut this.settings.hidden;
                        h.open = !h.open;
                        if h.open {
                            h.draft = h.date.unwrap_or_else(today);
                        }
                        cx.notify();
                    })),
            ),
        );

        if h.open {
            let draft = h.draft;
            let column = |label: &'static str, field: Field, text: String, width: f32, cx: &mut Context<Self>| -> Div {
                let step = |up: bool, cx: &mut Context<Self>| {
                    let delta: i32 = if up { 1 } else { -1 };
                    div()
                        .id(("hidden-step", (match field { Field::Day => 0, Field::Month => 2, Field::Year => 4 }) + usize::from(up)))
                        .w_full()
                        .h(px(44.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .active(|s| s.opacity(0.4))
                        .child(crate::components::icon::chevron(if up { crate::components::icon::Dir::Up } else { crate::components::icon::Dir::Down }, rgb(BLUE)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if window.tap_caught_fling() {
                                return;
                            }
                            let h = &mut this.settings.hidden;
                            h.draft = h.draft.stepped(field, delta, today());
                            cx.notify();
                        }))
                };
                div()
                    .w(px(width))
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(div().text_size(px(12.)).text_color(p.secondary).child(label))
                    .child(step(true, cx))
                    .child(div().h(px(32.)).flex().items_center().text_size(px(20.)).child(text))
                    .child(step(false, cx))
            };
            let mut presets = div().flex().flex_row().flex_wrap().justify_center().gap_2();
            for (i, years) in [1, 2, 5, 10].into_iter().enumerate() {
                let label = if years == 1 { "vor 1 Jahr".to_string() } else { format!("vor {years} Jahren") };
                presets = presets.child(
                    div()
                        .id(("hidden-preset", i))
                        .px_3()
                        .h(px(32.))
                        .flex()
                        .items_center()
                        .rounded_full()
                        .bg(p.separator)
                        .text_size(px(15.))
                        .active(|s| s.opacity(0.6))
                        .child(label)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if window.tap_caught_fling() {
                                return;
                            }
                            this.settings.hidden.draft = Ymd::years_ago(today(), years);
                            cx.notify();
                        })),
                );
            }
            let button = |id: &'static str, label: &'static str, color: Rgba, bold: bool| {
                let d = div()
                    .id(id)
                    .flex_1()
                    .min_h(px(44.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(17.))
                    .text_color(color)
                    .active(|s| s.opacity(0.6))
                    .child(label);
                if bold { d.font_weight(FontWeight::SEMIBOLD) } else { d }
            };
            let picker = div()
                .px_4()
                .py_2()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_center()
                        .gap_4()
                        .child(column("Tag", Field::Day, draft.day.to_string(), 70., cx))
                        .child(column("Monat", Field::Month, MONTHS[(draft.month - 1) as usize].to_string(), 130., cx))
                        .child(column("Jahr", Field::Year, draft.year.to_string(), 80., cx)),
                )
                .child(presets)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .child(button("hidden-clear", "Alle anzeigen", rgb(BLUE), false).on_click(cx.listener(|this, _, window, cx| {
                            if !window.tap_caught_fling() {
                                this.apply_hidden_draft(None, cx);
                            }
                        })))
                        .child(button("hidden-apply", "Übernehmen", rgb(BLUE), true).on_click(cx.listener(move |this, _, window, cx| {
                            if !window.tap_caught_fling() {
                                let date = Ymd::clamped(this.settings.hidden.draft.year, this.settings.hidden.draft.month as i64, this.settings.hidden.draft.day as i64, today());
                                this.apply_hidden_draft(Some(date), cx);
                            }
                        }))),
                );
            rows.push(separator(div()).child(picker));
        }

        let hint = if h.date.is_none() {
            "Alle Fotos werden angezeigt.".to_string()
        } else if h.count == 1 {
            "1 Foto ausgeblendet. Nur auf diesem Gerät, nichts wird gelöscht.".to_string()
        } else {
            format!("{} Fotos ausgeblendet. Nur auf diesem Gerät, nichts wird gelöscht.", h.count)
        };
        rows.push(separator(div()).px_4().py_2().text_size(px(13.)).text_color(p.secondary).child(hint));
        rows
    }
}
