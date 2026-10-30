//! Background jobs (downloading a selection) and the jobs sheet, opened by the button beside the gear in
//! Sammlungen. Each job is stored in the blob db `jobs` (key = job id, JSON from `logic::jobs`) after every few
//! items and on every state change, so pausing survives a restart and running jobs continue when the app starts
//! again. Only one job works at a time (the oldest running one), one item after the other; an installed PWA
//! only runs while it is open.

use super::downloads::{fetch_and_store, Fetched};
use super::settings::size;
use super::Immagine;
use crate::components::glass::glass_div;
use crate::components::rubber_band::RubberBand;
use crate::components::sheet::{self, Sheet};
use crate::host::{Current as H, Host, Pinch};
use crate::logic::jobs::{self, Job, JobItem, JobState, Outcome};
use crate::theme::{Palette, BLUE};
use crate::util::now_ms;
use gpui_kit::*;
use std::time::Duration;

pub(super) const JOBS_DB: &str = "jobs";
/// A running job is written back at most this often (and always when it changes state).
const SAVE_EVERY_MS: f64 = 1500.;

#[derive(Default)]
pub(super) struct JobsState {
    pub(super) list: Vec<Job>,
    /// The runner loop is alive.
    running: bool,
    /// Bumped at logout: a runner from before stops.
    serial: u32,
    /// The job whose running time is being counted and since when (`now_ms`).
    tick: Option<(u64, f64)>,
    last_save: f64,
    pub(super) sheet: Sheet,
    scroll: ScrollHandle,
    pub(super) rubber: RubberBand,
}

impl JobsState {
    fn get(&self, id: u64) -> Option<&Job> {
        self.list.iter().find(|j| j.id == id)
    }

    fn get_mut(&mut self, id: u64) -> Option<&mut Job> {
        self.list.iter_mut().find(|j| j.id == id)
    }

    pub(super) fn active(&self) -> usize {
        self.list.iter().filter(|j| !j.state.finished()).count()
    }

    /// Running time of a job including the stretch that is counting now.
    fn active_ms(&self, job: &Job) -> f64 {
        job.active_ms + self.tick.filter(|t| t.0 == job.id).map_or(0., |t| now_ms() - t.1)
    }

    /// Adds the time since the last tick to the job being counted and restarts the tick (or stops counting when
    /// that job no longer runs).
    fn account(&mut self) {
        let Some((id, since)) = self.tick else { return };
        let now = now_ms();
        let running = match self.get_mut(id) {
            Some(job) => {
                job.active_ms += (now - since).max(0.);
                job.state == JobState::Running
            }
            None => false,
        };
        self.tick = running.then_some((id, now));
    }

    /// Clears everything (logout); a runner still going stops at its next step.
    pub(super) fn reset(&mut self) {
        self.list.clear();
        self.serial += 1;
        self.running = false;
        self.tick = None;
        self.sheet.close();
    }
}

fn save(job: &Job, cx: &mut Context<Immagine>) {
    let (key, bytes) = (job.key(), job.to_bytes());
    cx.spawn(async move |_, _| {
        let _ = H::blob_put(JOBS_DB, &key, &bytes).await;
    })
    .detach();
}

fn forget(ids: Vec<u64>, cx: &mut Context<Immagine>) {
    if ids.is_empty() {
        return;
    }
    cx.spawn(async move |_, _| {
        for id in ids {
            let _ = H::blob_delete(JOBS_DB, &id.to_string()).await;
        }
    })
    .detach();
}

impl Immagine {
    /// Reads the stored jobs and continues the running ones. Called once the stored downloads are known, so items
    /// stored earlier count as skipped instead of being fetched again.
    pub(super) fn jobs_load(&mut self, cx: &mut Context<Self>) {
        let serial = self.jobs.serial;
        cx.spawn(async move |this, cx| {
            let keys = H::blob_keys(JOBS_DB).await.unwrap_or_default();
            let mut stored = Vec::new();
            for key in keys {
                if let Ok(Some(bytes)) = H::blob_get(JOBS_DB, &key).await {
                    if let Some(job) = Job::from_bytes(&bytes) {
                        stored.push(job);
                    }
                }
            }
            let _ = this.update(cx, |t, cx| {
                if t.jobs.serial != serial {
                    return;
                }
                for job in stored {
                    if t.jobs.get(job.id).is_none() {
                        t.jobs.list.push(job);
                    }
                }
                let gone = jobs::prune(&mut t.jobs.list);
                forget(gone, cx);
                t.jobs_pump(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Starts a download job for `ids` (assets already stored are skipped when the job reaches them).
    pub(super) fn jobs_download(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        let video: std::collections::HashSet<&str> = self.items.iter().filter(|i| i.is_video()).map(|i| i.id.as_str()).collect();
        let items: Vec<JobItem> = ids.into_iter().map(|id| JobItem { video: video.contains(id.as_str()), id }).collect();
        if items.is_empty() {
            return;
        }
        let (id, _) = jobs::next_ids(&self.jobs.list);
        let job = Job::new(id, items);
        save(&job, cx);
        self.jobs.list.push(job);
        self.jobs_pump(cx);
        cx.notify();
    }

    fn jobs_set_state(&mut self, id: u64, state: JobState, cx: &mut Context<Self>) {
        self.jobs.account();
        let (_, seq) = jobs::next_ids(&self.jobs.list);
        let Some(job) = self.jobs.get_mut(id) else { return };
        if job.state.finished() {
            return;
        }
        if state.finished() {
            job.finish(state, seq);
        } else {
            job.state = state;
        }
        let job = job.clone();
        save(&job, cx);
        let gone = jobs::prune(&mut self.jobs.list);
        forget(gone, cx);
        self.jobs_pump(cx);
        cx.notify();
    }

    /// Runs the oldest running job item by item until no job runs any more. Only one loop at a time.
    fn jobs_pump(&mut self, cx: &mut Context<Self>) {
        if self.jobs.running || jobs::runnable(&self.jobs.list).is_none() {
            return;
        }
        self.jobs.running = true;
        let serial = self.jobs.serial;
        let origin = self.origin.clone();
        cx.spawn(async move |this, cx| {
            loop {
                let next = this
                    .update(cx, |t, _| {
                        if t.jobs.serial != serial {
                            return None;
                        }
                        let id = jobs::runnable(&t.jobs.list)?;
                        if t.jobs.tick.is_none_or(|tick| tick.0 != id) {
                            t.jobs.account();
                            t.jobs.tick = Some((id, now_ms()));
                        }
                        let item = t.jobs.get(id)?.current()?.clone();
                        let stored = t.downloads.ids.borrow().contains(&item.id);
                        Some((id, item, stored))
                    })
                    .ok()
                    .flatten();
                let Some((job_id, item, stored)) = next else { break };
                let outcome = if stored {
                    Outcome::Skipped
                } else {
                    let still = move |t: &Immagine| t.jobs.serial == serial && t.jobs.get(job_id).is_some_and(|j| j.state == JobState::Running);
                    match fetch_and_store(&this, cx, &origin, &item.id, item.video, still).await {
                        Ok(Fetched::Stored(n)) => Outcome::Stored(n),
                        // Paused or cancelled half-way: that item starts over when the job resumes.
                        Ok(Fetched::Stopped) => continue,
                        Err(e) => Outcome::Failed(e),
                    }
                };
                let _ = this.update(cx, |t, cx| {
                    if t.jobs.serial != serial {
                        return;
                    }
                    t.jobs.account();
                    let (_, seq) = jobs::next_ids(&t.jobs.list);
                    let last_save = t.jobs.last_save;
                    let Some(job) = t.jobs.get_mut(job_id) else { return };
                    // A job cancelled while its last item was on the way keeps that item uncounted.
                    if job.state.finished() || job.current().is_none_or(|c| c.id != item.id) {
                        return;
                    }
                    let finished = job.record(outcome, seq);
                    let now = now_ms();
                    if finished || job.state != JobState::Running || now - last_save > SAVE_EVERY_MS {
                        let job = job.clone();
                        t.jobs.last_save = now;
                        save(&job, cx);
                    }
                    if finished {
                        let gone = jobs::prune(&mut t.jobs.list);
                        forget(gone, cx);
                    }
                    cx.notify();
                });
            }
            let _ = this.update(cx, |t, cx| {
                if t.jobs.serial == serial {
                    t.jobs.running = false;
                    t.jobs.account();
                    // A job resumed in the moment the loop ended.
                    t.jobs_pump(cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    // MARK: - Sheet

    /// The round button beside the gear; a blue badge counts the unfinished jobs.
    pub(super) fn jobs_button(&self, p: &Palette, cx: &mut Context<Self>) -> Stateful<Div> {
        let active = self.jobs.active();
        glass_div(p.glass_tint, 18.)
            .id("albums-jobs")
            .relative()
            .size(px(36.))
            .flex()
            .items_center()
            .justify_center()
            .active(|s| s.opacity(0.6))
            .child(super::icons::glyph(super::icons::Glyph::Jobs, 20., p.label))
            .children((active > 0).then(|| {
                div()
                    .absolute()
                    .top(px(-4.))
                    .right(px(-4.))
                    .min_w(px(18.))
                    .h(px(18.))
                    .px(px(4.))
                    .rounded_full()
                    .bg(rgb(BLUE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(11.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(0xffffff))
                    .child(active.to_string())
            }))
            .on_click(cx.listener(|this, _, _, cx| this.open_jobs(cx)))
    }

    pub(super) fn open_jobs(&mut self, cx: &mut Context<Self>) {
        self.jobs.sheet.open();
        cx.notify();
        // Running times count up while the sheet is open.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let open = this
                    .update(cx, |t, cx| {
                        if t.jobs.running {
                            cx.notify();
                        }
                        t.jobs.sheet.visible()
                    })
                    .unwrap_or(false);
                if !open {
                    break;
                }
            }
        })
        .detach();
    }

    fn close_jobs(&mut self, cx: &mut Context<Self>) {
        self.jobs.sheet.close();
        cx.notify();
    }

    /// See `settings_scroll_event`.
    pub(super) fn jobs_scroll_event(&mut self, dy: f32, phase: TouchPhase, window: &mut Window) -> (bool, bool) {
        if self.jobs.sheet.dragging() {
            window.cancel_touch_momentum();
            return (true, false);
        }
        let scroll = self.jobs.scroll.clone();
        self.jobs.rubber.event(dy, phase, Some(&scroll), window)
    }

    /// Raw single-finger events (the touch path of the sheet's drag).
    pub(super) fn jobs_pinch(&mut self, event: Pinch, cx: &mut Context<Self>) {
        let sheet = &self.jobs.sheet;
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

    fn job_card(&self, job: &Job, p: &Palette, card: Rgba, cx: &mut Context<Self>) -> Div {
        let total = job.total();
        let fraction = if total == 0 { 1. } else { job.handled() as f32 / total as f32 };
        let (state, state_color) = match job.state {
            JobState::Running => ("Läuft", rgb(BLUE)),
            JobState::Paused => ("Pausiert", rgb(0xff9500)),
            JobState::Done => ("Fertig", rgb(0x34c759)),
            JobState::Cancelled => ("Abgebrochen", p.secondary),
        };
        let mut counts = format!("{} von {} heruntergeladen", job.done + job.skipped, total);
        if job.skipped > 0 {
            counts += &format!(" · {} schon vorhanden", job.skipped);
        }
        if job.failed > 0 {
            counts += &format!(" · {} fehlgeschlagen", job.failed);
        }
        let ms = self.jobs.active_ms(job);
        let mut data = format!("{} · Laufzeit {}", size(job.bytes), jobs::duration_text(ms));
        if let Some(rate) = job.rate() {
            data += &format!(" · {}/s", size(rate as u64));
        }
        let current = (job.state == JobState::Running)
            .then(|| job.current())
            .flatten()
            .and_then(|item| self.downloads.busy.get(&item.id).map(|f| (item.video, *f)))
            .map(|(video, f)| format!("Aktuell: {} {} %", if video { "Video" } else { "Foto" }, (f * 100.).round() as u32));
        let line = |text: String, color: Rgba| div().text_size(px(14.)).text_color(color).child(text);
        let id = job.id;
        let button = |key: &'static str, label: &'static str, color: Rgba| {
            div()
                .id((key, id as usize))
                .flex_1()
                .h(px(40.))
                .rounded(px(12.))
                .bg(rgba(0x7878801f))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color)
                .active(|s| s.opacity(0.6))
                .child(label)
        };
        let actions = (!job.state.finished()).then(|| {
            let toggle = if job.state == JobState::Running {
                button("job-pause", "Pausieren", rgb(BLUE)).on_click(cx.listener(move |this, _, _, cx| this.jobs_set_state(id, JobState::Paused, cx)))
            } else {
                button("job-resume", "Fortsetzen", rgb(BLUE)).on_click(cx.listener(move |this, _, _, cx| this.jobs_set_state(id, JobState::Running, cx)))
            };
            let cancel = button("job-cancel", "Abbrechen", rgb(0xff3b30)).on_click(cx.listener(move |this, _, _, cx| this.jobs_set_state(id, JobState::Cancelled, cx)));
            div().pt_1().flex().flex_row().gap_2().child(toggle).child(cancel)
        });
        let waiting = job.state == JobState::Running && jobs::runnable(&self.jobs.list) != Some(job.id);
        div()
            .rounded(px(22.))
            .bg(card)
            .p_4()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .items_center()
                    .child(div().text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).child(format!("Download · {total} Objekte")))
                    .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(state_color).child(if waiting { "Wartet" } else { state })),
            )
            .child(
                div()
                    .h(px(6.))
                    .w_full()
                    .rounded_full()
                    .bg(rgba(0x78788033))
                    .child(div().h_full().w(relative(fraction)).rounded_full().bg(state_color)),
            )
            .child(line(counts, p.label))
            .child(line(data, p.secondary))
            .children(current.map(|c| line(c, p.secondary)))
            .children(job.last_error.clone().filter(|_| job.failed > 0).map(|e| line(format!("Letzter Fehler: {e}"), rgb(0xff3b30))))
            .children(actions)
    }

    pub(super) fn jobs_overlay(&self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let sheet = &self.jobs.sheet;
        if !sheet.visible() {
            return None;
        }
        let (background, card) = if self.dark { (rgb(0x1c1c1e), rgb(0x2c2c2e)) } else { (rgb(0xf2f2f7), rgb(0xffffff)) };
        let (active, finished) = jobs::listed(&self.jobs.list);
        let caption = |text: &str| div().px_4().text_size(px(13.)).text_color(p.secondary).child(text.to_uppercase());
        let section = |title: &str, cards: Vec<Div>| div().flex().flex_col().gap_2().child(caption(title)).children(cards);
        let mut inner = div().px_4().pt_2().pb(px(self.bottom.max(16.))).flex().flex_col().gap_6();
        if active.is_empty() && finished.is_empty() {
            inner = inner.child(
                div()
                    .pt(px(40.))
                    .flex()
                    .justify_center()
                    .text_size(px(15.))
                    .text_color(p.secondary)
                    .child("Keine Jobs. Wähle in der Mediathek Fotos aus und lade sie herunter."),
            );
        }
        if !active.is_empty() {
            let cards = active.iter().map(|j| self.job_card(j, p, card, cx)).collect();
            inner = inner.child(section("Aktiv", cards));
        }
        if !finished.is_empty() {
            let cards = finished.iter().map(|j| self.job_card(j, p, card, cx)).collect();
            inner = inner.child(section("Abgeschlossen", cards));
        }
        let content = div()
            .id("jobs-scroll")
            .absolute()
            .top(px(sheet::HEADER))
            .left_0()
            .right_0()
            .bottom_0()
            .overflow_y_scroll()
            .track_scroll(&self.jobs.scroll)
            .child(self.jobs.rubber.shift(inner))
            .into_any_element();
        Some(sheet::overlay(
            sheet,
            |this: &Immagine| &this.jobs.sheet,
            cx,
            window,
            self.top.max(12.) + 10.,
            (background, p.glass_tint, p.label),
            "Jobs",
            content,
            |this, cx| this.close_jobs(cx),
        ))
    }
}
