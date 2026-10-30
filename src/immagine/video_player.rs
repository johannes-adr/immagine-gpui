//! The app's own video player: streams the file by HTTP range, demuxes it in Rust (`logic::movie`), feeds the platform's
//! decoders (`Host::video_decoder` / `Host::audio_output`), keeps the playback clock and hands the current picture
//! to GPUI as an image. There is no `<video>` element and no browser controls; the controls are drawn by the viewer.
//!
//! The picture is chosen by the clock: the audio output's clock when the movie has sound (so picture follows
//! sound), else the wall clock. `tick` runs once per rendered frame while the player is busy.

use super::thumbs::render_image;
use crate::host::{AudioConfig, AudioOut, AudioOutput, Current as H, Decoded, Host, VideoConfig, VideoDecode, VideoDecoder};
use crate::logic::movie::{audio_start_for, next_request, video_start_for, worth_full_download, ByteCache, Fetched, HeaderScan, Movie, Scan, HEAD_REQUEST};
use crate::util::now_ms;
use gpui_kit::*;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;

/// Decoded pictures waiting for their time. More just holds memory (a 1280 px picture is ~3.7 MB).
const MAX_PENDING: usize = 8;
/// Encoded samples handed to the decoder at once.
const MAX_DECODER_QUEUE: u32 = 6;
/// Sound is decoded this far ahead of the clock.
const AUDIO_AHEAD_US: i64 = 2_000_000;
/// Read-ahead: sample bytes are requested this far (media time) and this many video bytes past the feed position.
const READ_AHEAD_US: i64 = 10_000_000;
const READ_AHEAD_BYTES: u64 = 8 * 1024 * 1024;
/// Cached bytes older than this behind the playhead are dropped.
const KEEP_BEHIND_US: i64 = 30_000_000;
/// Consecutive failed range requests before the player gives up, and the pause between tries.
const MAX_FAILURES: u32 = 3;
const RETRY_MS: f64 = 1000.;
/// After a seek the player keeps rendering this long so the first picture of the new position shows up.
const SETTLE_MS: f64 = 600.;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Status {
    Loading,
    Ready,
    Failed(String),
}

/// The opened movie with the bytes loaded while opening it.
struct Opened {
    movie: Movie,
    cache: ByteCache,
    /// File size; with everything in `cache` (full download) no further requests are made.
    total: u64,
}

struct RangeDone {
    serial: u64,
    start: u64,
    result: Result<Vec<u8>, String>,
}

/// What the decoder callbacks and the loading tasks hand to the player (they run outside of `tick`).
#[derive(Default)]
struct Inbox {
    opened: Option<Result<Opened, String>>,
    frames: Vec<(i64, Decoded)>,
    ranges: Vec<RangeDone>,
}

fn load_error(e: impl std::fmt::Display) -> String {
    format!("Das Video konnte nicht geladen werden ({e}).")
}

/// Whole file download, then the same table-driven demuxing (fragmented files read through the `mp4` crate).
async fn open_full(bytes: Vec<u8>, cx: &AsyncApp) -> Result<Opened, String> {
    let total = bytes.len() as u64;
    cx.background_spawn(async move {
        let mut movie = Movie::open(bytes)?;
        let cache = movie.take_cache();
        Ok(Opened { movie, cache, total })
    })
    .await
}

/// Probes the head with a range request, reads only `ftyp` + `moov` and leaves the rest to on-demand ranges. Any
/// server or file that cannot do that (no 206, unknown size, `moov` unreachable, fragmented) is downloaded whole.
async fn open_movie(url: &str, cx: &AsyncApp) -> Result<Opened, String> {
    let first = H::fetch_range(url, 0, HEAD_REQUEST - 1).await.map_err(load_error)?;
    let total = match (first.partial, first.total) {
        (true, Some(total)) => total,
        _ => return open_full(first.bytes, cx).await,
    };
    let mut cache = ByteCache::new();
    cache.insert(0, first.bytes);
    let mut scan = HeaderScan::new(total);
    for _ in 0..8 {
        match scan.step(&cache) {
            Ok(Scan::Fetch { start, end }) => match H::fetch_range(url, start, end - 1).await {
                Ok(r) if r.partial => cache.insert(start, r.bytes),
                Ok(r) => return open_full(r.bytes, cx).await,
                Err(e) => return Err(load_error(e)),
            },
            Ok(Scan::Done { ftyp, moov }) => {
                let (Some(ftyp), Some(moov)) = (cache.read(ftyp.0, ftyp.1), cache.read(moov.0, moov.1)) else { break };
                match cx.background_spawn(async move { Movie::open_header(&ftyp, &moov) }).await {
                    Ok(movie) => return Ok(Opened { movie, cache, total }),
                    Err(e) if !worth_full_download(&e) => return Err(e),
                    Err(_) => break,
                }
            }
            Err(e) if !worth_full_download(&e) => return Err(e),
            Err(_) => break,
        }
    }
    drop(cache);
    open_full(H::fetch_bytes(url).await.map_err(load_error)?, cx).await
}

pub(super) struct Player {
    inbox: Rc<RefCell<Inbox>>,
    pub(super) status: Status,
    app: AsyncApp,
    url: String,
    movie: Option<Movie>,
    /// Loaded file bytes; samples are cut from here, missing ones are requested by range.
    cache: ByteCache,
    total: u64,
    /// Tags the one request in flight (`None` = idle); a seek bumps it so the answer no longer counts as awaited.
    serial: u64,
    inflight: Option<u64>,
    failures: u32,
    retry_at: f64,
    /// The decoder feed is blocked on bytes that have not arrived.
    starved: bool,
    /// Playback waits for data (starved with nothing left to show): the clock stands still.
    pub(super) buffering: bool,
    decoder: Option<VideoDecoder>,
    audio: Option<AudioOut>,
    /// The movie has a sound track the platform can decode: the audio clock drives playback.
    has_audio: bool,
    next_video: usize,
    next_audio: usize,
    pending: VecDeque<(i64, Decoded)>,
    shown: Option<(i64, Arc<RenderImage>)>,
    /// Replaced pictures; the next `tick` frees their atlas tiles.
    garbage: Vec<Arc<RenderImage>>,
    /// The user wants playback (false = paused).
    playing: bool,
    /// The clock runs: the first picture at or after `target_us` has been decoded.
    started: bool,
    /// Where playback (re)starts from, microseconds.
    target_us: i64,
    base_wall: f64,
    base_media: i64,
    ended: bool,
    /// `flush` was sent to the decoder after the last sample.
    flushed: bool,
    audio_flushed: bool,
    settle_until: f64,
    pub(super) duration_us: i64,
    pub(super) aspect: f32,
    /// Position shown by the controls, updated by `tick`.
    pub(super) position_us: i64,
    /// Scrubbing: whether playback resumes when the finger lifts, and when the last preview seek happened.
    scrub_resume: Option<bool>,
    last_scrub: f64,
}

impl Player {
    /// Starts downloading `url` and playing it as soon as it is ready. Call from the tap that asked for playback
    /// (the audio output can only be unlocked there).
    pub(super) fn open<T: 'static>(url: String, cx: &mut Context<T>) -> Player {
        let inbox = Rc::new(RefCell::new(Inbox::default()));
        let task_inbox = inbox.clone();
        let task_url = url.clone();
        cx.spawn(async move |this, cx| {
            let opened = open_movie(&task_url, cx).await;
            task_inbox.borrow_mut().opened = Some(opened);
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
        Player {
            inbox,
            status: Status::Loading,
            app: cx.to_async(),
            url,
            cache: ByteCache::new(),
            total: 0,
            serial: 0,
            inflight: None,
            failures: 0,
            retry_at: 0.,
            starved: false,
            buffering: false,
            movie: None,
            decoder: None,
            audio: H::audio_output(),
            has_audio: false,
            next_video: 0,
            next_audio: 0,
            pending: VecDeque::new(),
            shown: None,
            garbage: Vec::new(),
            playing: true,
            started: false,
            target_us: 0,
            base_wall: 0.,
            base_media: 0,
            ended: false,
            flushed: false,
            audio_flushed: false,
            settle_until: 0.,
            duration_us: 0,
            aspect: 0.,
            position_us: 0,
            scrub_resume: None,
            last_scrub: 0.,
        }
    }

    pub(super) fn frame(&self) -> Option<Arc<RenderImage>> {
        self.shown.as_ref().map(|(_, image)| image.clone())
    }

    pub(super) fn is_playing(&self) -> bool {
        self.playing
    }

    fn attach(&mut self, result: Result<Opened, String>) {
        let Opened { movie, cache, total } = match result {
            Ok(opened) => opened,
            Err(e) => {
                self.status = Status::Failed(e);
                return;
            }
        };
        let sink = self.inbox.clone();
        let config = VideoConfig {
            codec: movie.video.codec.clone(),
            description: movie.video.description.clone(),
            width: movie.video.width,
            height: movie.video.height,
        };
        let Some(decoder) = H::video_decoder(config, move |ts, picture| sink.borrow_mut().frames.push((ts, picture))) else {
            self.status = Status::Failed("Auf diesem Gerät lassen sich Videos noch nicht abspielen.".into());
            return;
        };
        if let (Some(audio), Some(track)) = (self.audio.as_ref(), movie.audio.as_ref()) {
            self.has_audio = audio.configure(AudioConfig {
                codec: track.codec.clone(),
                description: track.description.clone(),
                sample_rate: track.sample_rate,
                channels: track.channels,
            });
        }
        self.duration_us = movie.duration_us.max(movie.video.samples.last().map_or(0, |s| s.pts_us + s.dur_us));
        self.aspect = movie.video.width as f32 / movie.video.height.max(1) as f32;
        self.decoder = Some(decoder);
        self.cache = cache;
        self.total = total;
        self.movie = Some(movie);
        self.next_video = 0;
        self.next_audio = 0;
        self.settle_until = now_ms() + SETTLE_MS;
        self.status = Status::Ready;
    }

    /// Media position in microseconds.
    fn clock(&self) -> i64 {
        if !self.started {
            return self.target_us;
        }
        match (&self.audio, self.has_audio) {
            (Some(audio), true) => audio.clock_us().unwrap_or(self.target_us),
            _ if self.playing && !self.buffering => self.base_media + ((now_ms() - self.base_wall) * 1000.) as i64,
            _ => self.base_media,
        }
    }

    fn end_us(&self) -> i64 {
        self.movie.as_ref().and_then(|m| m.video.samples.iter().map(|s| s.pts_us + s.dur_us).max()).unwrap_or(self.duration_us)
    }

    /// Advances the player to now: takes in what arrived, starts the clock, picks the picture, feeds the decoders.
    /// Returns whether it needs another frame (playing, or waiting for pictures).
    pub(super) fn tick(&mut self, window: &mut Window) -> bool {
        for old in self.garbage.drain(..) {
            let _ = window.drop_image(old);
        }
        let (opened, frames, ranges) = {
            let mut inbox = self.inbox.borrow_mut();
            (inbox.opened.take(), std::mem::take(&mut inbox.frames), std::mem::take(&mut inbox.ranges))
        };
        if let Some(result) = opened {
            self.attach(result);
        }
        self.pending.extend(frames);
        self.take_ranges(ranges);
        if self.status != Status::Ready {
            return false;
        }
        if let Some(e) = self.decoder.as_ref().and_then(|d| d.error()) {
            self.status = Status::Failed(format!("Das Video lässt sich nicht dekodieren ({e})."));
            return false;
        }
        let now = now_ms();

        if !self.started && self.playing {
            let have_next = self.pending.iter().any(|(ts, _)| *ts >= self.target_us);
            if have_next || self.video_done() {
                self.started = true;
                self.base_wall = now;
                self.base_media = self.target_us;
                if let (Some(audio), true) = (&self.audio, self.has_audio) {
                    audio.start_clock(self.target_us);
                    audio.set_paused(false);
                }
            }
        }

        let at = self.clock();
        let mut due = None;
        while self.pending.front().is_some_and(|(ts, _)| *ts <= at) {
            due = self.pending.pop_front();
        }
        if let Some((ts, picture)) = due {
            if let Some(image) = render_image(picture.width, picture.height, picture.rgba) {
                if let Some((_, old)) = self.shown.replace((ts, image)) {
                    self.garbage.push(old);
                }
            }
        }
        self.position_us = at.clamp(0, self.duration_us);

        if self.started && self.playing && self.video_done() && self.pending.is_empty() && at >= self.end_us() {
            self.playing = false;
            self.ended = true;
            self.base_media = self.end_us();
            if let (Some(audio), true) = (&self.audio, self.has_audio) {
                audio.set_paused(true);
            }
        }

        self.feed(at);
        self.update_buffering(now, at);
        self.request_more(now, at);
        self.playing || self.buffering || now < self.settle_until || self.decoder.as_ref().is_some_and(|d| d.queue_len() > 0)
    }

    /// Stores arrived ranges (always valid: bytes are addressed by file position) and settles the request.
    fn take_ranges(&mut self, ranges: Vec<RangeDone>) {
        for done in ranges {
            let awaited = self.inflight == Some(done.serial);
            match done.result {
                Ok(bytes) => {
                    self.cache.insert(done.start, bytes);
                    if awaited {
                        self.failures = 0;
                    }
                }
                Err(e) if awaited => {
                    self.failures += 1;
                    self.retry_at = now_ms() + RETRY_MS;
                    if self.failures >= MAX_FAILURES {
                        self.status = Status::Failed(load_error(e));
                    }
                }
                Err(_) => {}
            }
            if awaited {
                self.inflight = None;
            }
        }
    }

    /// Keeps one range request going for the bytes the decoders need next, and drops what is far behind.
    fn request_more(&mut self, now: f64, at: i64) {
        let Some(movie) = self.movie.as_ref() else { return };
        if movie.is_legacy() || self.status != Status::Ready {
            return;
        }
        self.cache.evict_before(movie.offset_at(at - KEEP_BEHIND_US));
        if self.inflight.is_some() || now < self.retry_at {
            return;
        }
        let horizon = at.max(self.target_us) + READ_AHEAD_US;
        let wanted = movie.wanted(self.next_video, if self.has_audio { self.next_audio } else { usize::MAX }, horizon, READ_AHEAD_BYTES);
        let Some((start, end)) = next_request(&wanted, &self.cache, self.total) else { return };
        self.serial += 1;
        let serial = self.serial;
        self.inflight = Some(serial);
        let (url, inbox) = (self.url.clone(), self.inbox.clone());
        self.app
            .spawn(async move |cx| {
                let result = H::fetch_range(&url, start, end - 1).await.and_then(|r| {
                    // A server that starts ignoring Range mid-way answers with the whole file; take the slice.
                    if r.partial { Ok(r.bytes) } else { r.bytes.get(start as usize..(end as usize).min(r.bytes.len())).map(|b| b.to_vec()).ok_or_else(|| "kein Range".to_string()) }
                });
                inbox.borrow_mut().ranges.push(RangeDone { serial, start, result });
                cx.refresh();
            })
            .detach();
    }

    /// Buffering = the decoder feed waits for bytes and nothing is left to show. The clock stops while it lasts
    /// (audio is paused; the wall clock freezes in `clock`) and resumes when data is there.
    fn update_buffering(&mut self, now: f64, at: i64) {
        let dry = self.starved && self.pending.is_empty() && self.decoder.as_ref().is_none_or(|d| d.queue_len() == 0);
        if dry == self.buffering {
            return;
        }
        self.buffering = dry;
        if !self.started || !self.playing {
            return;
        }
        if dry {
            self.base_media = at;
        } else {
            self.base_wall = now;
        }
        if let (Some(audio), true) = (&self.audio, self.has_audio) {
            audio.set_paused(dry);
        }
    }

    fn video_done(&self) -> bool {
        let n = self.movie.as_ref().map_or(0, |m| m.video.samples.len());
        self.next_video >= n && self.decoder.as_ref().is_none_or(|d| d.queue_len() == 0)
    }

    fn feed(&mut self, at: i64) {
        let (Some(movie), Some(decoder)) = (self.movie.as_mut(), self.decoder.as_ref()) else { return };
        let video = movie.video.track_id;
        let count = movie.video.samples.len();
        self.starved = false;
        while self.next_video < count && decoder.queue_len() < MAX_DECODER_QUEUE && self.pending.len() < MAX_PENDING {
            match movie.next_chunk(video, self.next_video, &self.cache) {
                Fetched::Ready(chunk) => decoder.decode(&chunk),
                Fetched::Missing => {
                    self.starved = true;
                    break;
                }
                Fetched::End => {
                    self.next_video = count;
                    break;
                }
            }
            self.next_video += 1;
        }
        if self.next_video >= count && !self.flushed {
            self.flushed = true;
            decoder.flush();
        }
        if let (Some(audio), true) = (self.audio.as_ref(), self.has_audio) {
            let track = movie.audio.as_ref().map(|a| (a.track_id, a.samples.len()));
            if let Some((id, count)) = track {
                let horizon = at.max(self.target_us) + AUDIO_AHEAD_US;
                while self.next_audio < count && audio.queue_len() < 12 {
                    if movie.audio.as_ref().and_then(|a| a.samples.get(self.next_audio)).is_some_and(|s| s.pts_us > horizon) {
                        break;
                    }
                    match movie.next_chunk(id, self.next_audio, &self.cache) {
                        Fetched::Ready(chunk) => audio.decode(&chunk),
                        Fetched::Missing => break,
                        Fetched::End => {
                            self.next_audio = count;
                            break;
                        }
                    }
                    self.next_audio += 1;
                }
                if self.next_audio >= count && !self.audio_flushed {
                    self.audio_flushed = true;
                    audio.flush();
                }
            }
        }
    }

    /// Jumps to `t_us`: decoding restarts at the keyframe before it and the pictures before `t_us` are dropped.
    pub(super) fn seek(&mut self, t_us: i64) {
        let t_us = t_us.clamp(0, self.duration_us.max(0));
        self.target_us = t_us;
        self.base_media = t_us;
        self.position_us = t_us;
        self.started = false;
        self.ended = false;
        self.flushed = false;
        self.audio_flushed = false;
        self.settle_until = now_ms() + SETTLE_MS;
        self.pending.clear();
        self.inbox.borrow_mut().frames.clear();
        // The request in flight (if any) still fills the cache, but is no longer awaited: the next tick asks for
        // the bytes of the new position.
        self.serial += 1;
        self.inflight = None;
        self.starved = false;
        self.buffering = false;
        if let (Some(movie), Some(decoder)) = (self.movie.as_ref(), self.decoder.as_ref()) {
            decoder.reset();
            self.next_video = video_start_for(&movie.video.samples, t_us);
            if let Some(audio) = self.audio.as_ref().filter(|_| self.has_audio) {
                audio.reset();
                self.next_audio = movie.audio.as_ref().map_or(0, |a| audio_start_for(&a.samples, t_us));
            }
        }
    }

    pub(super) fn toggle_play(&mut self) {
        if self.status != Status::Ready {
            return;
        }
        if self.ended {
            self.seek(0);
            self.playing = true;
            if let (Some(audio), true) = (&self.audio, self.has_audio) {
                audio.set_paused(false);
            }
            return;
        }
        if self.playing {
            self.base_media = self.clock();
            self.playing = false;
            if let (Some(audio), true) = (&self.audio, self.has_audio) {
                audio.set_paused(true);
            }
        } else {
            self.playing = true;
            self.base_wall = now_ms();
            if let (Some(audio), true, false) = (&self.audio, self.has_audio, self.buffering) {
                audio.set_paused(false);
            }
        }
    }

    /// A finger went down on the scrubber: playback pauses while it moves.
    pub(super) fn scrub_begin(&mut self) {
        if self.scrub_resume.is_none() {
            self.scrub_resume = Some(self.playing && !self.ended);
            if self.playing {
                self.toggle_play();
            }
        }
    }

    /// The finger is at `fraction` (0..1) of the movie. Previews at most every 120 ms; `scrub_end` makes it final.
    pub(super) fn scrub_to(&mut self, fraction: f32) {
        self.position_us = (fraction.clamp(0., 1.) as f64 * self.duration_us as f64) as i64;
        let now = now_ms();
        if now - self.last_scrub > 120. {
            self.last_scrub = now;
            self.seek(self.position_us);
        }
    }

    pub(super) fn scrub_end(&mut self, fraction: f32) {
        let Some(resume) = self.scrub_resume.take() else { return };
        self.seek((fraction.clamp(0., 1.) as f64 * self.duration_us as f64) as i64);
        if resume {
            self.toggle_play();
        }
    }

    pub(super) fn scrubbing(&self) -> bool {
        self.scrub_resume.is_some()
    }
}

/// `m:ss` (or `h:mm:ss`) of a position in microseconds.
pub(super) fn fmt_time(us: i64) -> String {
    let s = (us.max(0) / 1_000_000) as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

#[cfg(test)]
mod tests {
    use super::fmt_time;

    #[test]
    fn formats_positions() {
        assert_eq!(fmt_time(0), "0:00");
        assert_eq!(fmt_time(65_900_000), "1:05");
        assert_eq!(fmt_time(3_725_000_000), "1:02:05");
        assert_eq!(fmt_time(-5), "0:00");
    }
}
