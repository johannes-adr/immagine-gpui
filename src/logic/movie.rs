//! MP4 demuxing for the native video player: the file's tracks, the codec configuration WebCodecs needs and the
//! encoded samples. Pure logic (no platform imports): the decoders live behind `host::Host`.
//!
//! Supported: progressive and fragmented MP4/MOV with H.264 (`avc1`) video and optional AAC (`mp4a`) audio.
//! Other codecs (HEVC, VP9, ...) report an error so the caller can say so.
//!
//! Streaming: only `ftyp` + `moov` are needed to know every sample's time, size and byte position
//! (`Movie::open_header`); the sample bytes are fetched later by range into a [`ByteCache`] and turned into
//! [`Chunk`]s with `Movie::next_chunk`. [`HeaderScan`] finds the two boxes with as few range requests as possible.

use mp4::{MediaType, Mp4Reader, Mp4Track, TrackType};
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Seek};

/// One encoded sample's timing, kept for the whole file so seeking and feeding need no further parsing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SampleMeta {
    /// Presentation time (decode time plus composition offset), microseconds.
    pub pts_us: i64,
    pub dur_us: i64,
    pub key: bool,
}

/// An encoded sample ready for a decoder.
#[derive(Clone, Debug)]
pub struct Chunk {
    pub key: bool,
    pub ts_us: i64,
    pub dur_us: i64,
    pub data: Vec<u8>,
}

pub struct VideoInfo {
    pub track_id: u32,
    pub width: u32,
    pub height: u32,
    /// WebCodecs codec string, e.g. `avc1.64001f`.
    pub codec: String,
    /// `avcC` record (length-prefixed NAL units in the samples, parameter sets here).
    pub description: Vec<u8>,
    pub samples: Vec<SampleMeta>,
    /// Byte position and size of every sample (empty for fragmented files, which are read through the `mp4` crate).
    pub locs: Vec<SampleLoc>,
}

pub struct AudioInfo {
    pub track_id: u32,
    /// WebCodecs codec string, e.g. `mp4a.40.2`.
    pub codec: String,
    /// AudioSpecificConfig.
    pub description: Vec<u8>,
    pub sample_rate: u32,
    pub channels: u32,
    pub samples: Vec<SampleMeta>,
    pub locs: Vec<SampleLoc>,
}

/// Where a sample lives in the file.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SampleLoc {
    pub offset: u64,
    pub size: u32,
}

const AAC_RATES: [u32; 13] = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350];

fn us(ticks: i64, timescale: u32) -> i64 {
    (ticks as i128 * 1_000_000 / timescale.max(1) as i128) as i64
}

/// Outcome of asking for a sample.
pub enum Fetched {
    Ready(Chunk),
    /// The sample's bytes are not in the cache yet.
    Missing,
    /// No such sample.
    End,
}

/// Merge byte ranges closer than this when planning requests (skipping the gap is cheaper than a round trip).
pub const MERGE_GAP: u64 = 256 * 1024;
/// Request size limits: a request is padded to `MIN_REQUEST` and cut at `MAX_REQUEST`.
pub const MIN_REQUEST: u64 = 1024 * 1024;
pub const MAX_REQUEST: u64 = 4 * 1024 * 1024;
/// How much of the file's start is fetched to find `moov` (usually in there for faststart files).
pub const HEAD_REQUEST: u64 = 256 * 1024;
/// A `moov` larger than this is not streamed (the demuxer would hold it in memory several times).
const MAX_MOOV: u64 = 128 * 1024 * 1024;

/// Errors that mean "not worth downloading the whole file", the rest ("Fallback: ...", parse trouble) may be
/// a quirk of the streaming path only.
pub fn worth_full_download(error: &str) -> bool {
    !(error.starts_with("Dieses Videoformat") || error.starts_with("Keine Videospur"))
}

/// Loaded byte spans of the file, keyed by start. Spans never overlap (new data wins), touching ones stay apart.
#[derive(Default)]
pub struct ByteCache {
    spans: BTreeMap<u64, Vec<u8>>,
}

impl ByteCache {
    pub fn new() -> ByteCache {
        ByteCache::default()
    }

    pub fn insert(&mut self, start: u64, data: Vec<u8>) {
        if data.is_empty() {
            return;
        }
        let end = start + data.len() as u64;
        let hit: Vec<u64> = self.spans.range(..end).filter(|(s, v)| **s + v.len() as u64 > start).map(|(s, _)| *s).collect();
        for key in hit {
            let mut v = self.spans.remove(&key).unwrap();
            if key + v.len() as u64 > end {
                let tail = v.split_off((end - key) as usize);
                self.spans.insert(end, tail);
            }
            if key < start {
                v.truncate((start - key) as usize);
                self.spans.insert(key, v);
            }
        }
        self.spans.insert(start, data);
    }

    /// Byte count held.
    pub fn len(&self) -> u64 {
        self.spans.values().map(|v| v.len() as u64).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// The span containing `pos`: (its start, bytes).
    fn span_at(&self, pos: u64) -> Option<(u64, &Vec<u8>)> {
        self.spans.range(..=pos).next_back().filter(|(s, v)| **s + v.len() as u64 > pos).map(|(s, v)| (*s, v))
    }

    /// Whether `[offset, offset + len)` is fully cached (also across touching spans).
    pub fn has(&self, offset: u64, len: u64) -> bool {
        self.first_gap(offset, offset + len).is_none()
    }

    /// `[offset, offset + len)` as one buffer, `None` unless all of it is cached.
    pub fn read(&self, offset: u64, len: u64) -> Option<Vec<u8>> {
        let end = offset + len;
        let mut out = Vec::with_capacity(len as usize);
        let mut pos = offset;
        while pos < end {
            let (start, bytes) = self.span_at(pos)?;
            let from = (pos - start) as usize;
            let to = ((end - start) as usize).min(bytes.len());
            out.extend_from_slice(&bytes[from..to]);
            pos = start + to as u64;
        }
        Some(out)
    }

    /// The first stretch of `[start, end)` that is not cached.
    pub fn first_gap(&self, start: u64, end: u64) -> Option<(u64, u64)> {
        let mut pos = start;
        while pos < end {
            match self.span_at(pos) {
                Some((s, v)) => pos = s + v.len() as u64,
                None => {
                    let next = self.spans.range(pos..).next().map_or(end, |(s, _)| *s);
                    return Some((pos, next.min(end)));
                }
            }
        }
        None
    }

    /// Drops the spans farthest from `center` (a file position) until at most `keep` bytes are left; the span
    /// holding `center` always stays.
    pub fn evict_far(&mut self, center: u64, keep: u64) {
        let mut total = self.len();
        if total <= keep {
            return;
        }
        let dist = |s: u64, len: usize| {
            let (a, b) = (s, s + len as u64);
            if center < a { a - center } else if center >= b { center - b + 1 } else { 0 }
        };
        let mut order: Vec<(u64, u64, usize)> = self.spans.iter().map(|(s, v)| (dist(*s, v.len()), *s, v.len())).collect();
        order.sort_unstable_by(|a, b| b.cmp(a));
        for (d, start, len) in order {
            if total <= keep || d == 0 {
                break;
            }
            self.spans.remove(&start);
            total -= len as u64;
        }
    }

    /// Drops spans that end at or before `pos` (data behind the playhead).
    pub fn evict_before(&mut self, pos: u64) {
        let old: Vec<u64> = self.spans.iter().filter(|(s, v)| **s + v.len() as u64 <= pos).map(|(s, _)| *s).collect();
        for key in old {
            self.spans.remove(&key);
        }
    }
}

/// Merges `(start, end)` ranges (end exclusive) that are at most `gap` apart.
pub fn merge_ranges(mut ranges: Vec<(u64, u64)>, gap: u64) -> Vec<(u64, u64)> {
    ranges.sort_unstable();
    let mut out: Vec<(u64, u64)> = Vec::new();
    for (s, e) in ranges {
        match out.last_mut() {
            Some(last) if s <= last.1 + gap => last.1 = last.1.max(e),
            _ => out.push((s, e)),
        }
    }
    out
}

/// The next request to make for the wanted ranges: the first uncached stretch, padded to `MIN_REQUEST`, cut at
/// `MAX_REQUEST`, never beyond `total`. `(start, end)` with `end` exclusive.
pub fn next_request(wanted: &[(u64, u64)], cache: &ByteCache, total: u64) -> Option<(u64, u64)> {
    for &(s, e) in wanted {
        if let Some((gs, ge)) = cache.first_gap(s, e.min(total)) {
            let end = ge.min(gs + MAX_REQUEST).max(gs + MIN_REQUEST).min(total);
            return (end > gs).then_some((gs, end));
        }
    }
    None
}

/// Finds `ftyp` and `moov` in a file whose bytes arrive by range: call `step` after every insert into the cache
/// until it says `Done`. Walks the top-level boxes (32-bit, 64-bit and "to the end" sizes), jumping over `mdat`
/// by requesting only the next header, so `moov` after `mdat` costs two small extra requests.
pub struct HeaderScan {
    total: u64,
    pos: u64,
    ftyp: Option<(u64, u64)>,
    moov: Option<(u64, u64)>,
}

#[derive(Debug)]
pub enum Scan {
    /// Fetch `[start, end)` and call `step` again.
    Fetch { start: u64, end: u64 },
    /// `(offset, size)` of both boxes, all their bytes are in the cache.
    Done { ftyp: (u64, u64), moov: (u64, u64) },
}

impl HeaderScan {
    pub fn new(total: u64) -> HeaderScan {
        HeaderScan { total, pos: 0, ftyp: None, moov: None }
    }

    pub fn step(&mut self, cache: &ByteCache) -> Result<Scan, String> {
        loop {
            if let (Some(ftyp), Some(moov)) = (self.ftyp, self.moov) {
                for (off, size) in [ftyp, moov] {
                    if let Some((gs, ge)) = cache.first_gap(off, off + size) {
                        return Ok(Scan::Fetch { start: gs, end: ge });
                    }
                }
                return Ok(Scan::Done { ftyp, moov });
            }
            if self.pos + 8 > self.total {
                return Err("Fallback: ftyp oder moov fehlt".into());
            }
            let want = 16.min(self.total - self.pos);
            let Some(h) = cache.read(self.pos, want) else {
                return Ok(Scan::Fetch { start: self.pos, end: (self.pos + 64 * 1024).min(self.total) });
            };
            let size32 = u32::from_be_bytes(h[0..4].try_into().unwrap()) as u64;
            let name: [u8; 4] = h[4..8].try_into().unwrap();
            let (header, size) = match size32 {
                0 => (8, self.total - self.pos),
                1 => {
                    if h.len() < 16 {
                        return Err("Fallback: abgeschnittene Box".into());
                    }
                    (16, u64::from_be_bytes(h[8..16].try_into().unwrap()))
                }
                n => (8, n),
            };
            if size < header || self.pos + size > self.total {
                return Err("Fallback: ungültige Boxgröße".into());
            }
            match &name {
                b"ftyp" => self.ftyp = Some((self.pos, size)),
                b"moov" => {
                    if size > MAX_MOOV {
                        return Err("Fallback: moov zu groß".into());
                    }
                    self.moov = Some((self.pos, size));
                }
                b"moof" => return Err("Fallback: fragmentiertes MP4".into()),
                _ => {}
            }
            self.pos += size;
        }
    }
}

/// A box rewritten with a plain 32-bit header (the `mp4` crate does not follow 64-bit or zero sizes reliably).
fn plain_box(raw: &[u8]) -> Result<Vec<u8>, String> {
    let header = if raw.len() >= 8 && u32::from_be_bytes(raw[0..4].try_into().unwrap()) == 1 { 16 } else { 8 };
    if raw.len() < header {
        return Err("Fallback: abgeschnittene Box".into());
    }
    let payload = &raw[header..];
    let total = 8 + payload.len() as u64;
    if total > u32::MAX as u64 {
        return Err("Fallback: Box zu groß".into());
    }
    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(&(total as u32).to_be_bytes());
    out.extend_from_slice(&raw[4..8]);
    out.extend_from_slice(payload);
    Ok(out)
}

/// Sample table of one track from its `stbl` boxes, without reading any sample.
fn sample_table(track: &Mp4Track) -> Result<(Vec<SampleMeta>, Vec<SampleLoc>), String> {
    let stbl = &track.trak.mdia.minf.stbl;
    let timescale = track.timescale();
    let n = stbl.stsz.sample_count as usize;
    let size_of = |i: usize| -> Result<u32, String> {
        if stbl.stsz.sample_size != 0 { Ok(stbl.stsz.sample_size) } else { stbl.stsz.sample_sizes.get(i).copied().ok_or_else(|| "MP4: stsz zu kurz".to_string()) }
    };
    let chunks: Vec<u64> = if let Some(c) = &stbl.co64 {
        c.entries.clone()
    } else if let Some(c) = &stbl.stco {
        c.entries.iter().map(|x| *x as u64).collect()
    } else {
        return Err("MP4: stco fehlt".into());
    };

    let mut locs = Vec::with_capacity(n);
    let entries = &stbl.stsc.entries;
    for (ei, entry) in entries.iter().enumerate() {
        let first = entry.first_chunk as usize;
        if first == 0 {
            return Err("MP4: stsc ungültig".into());
        }
        let end = entries.get(ei + 1).map_or(chunks.len() + 1, |e| e.first_chunk as usize);
        for chunk in first..end {
            let Some(&base) = chunks.get(chunk - 1) else { break };
            let mut offset = base;
            for _ in 0..entry.samples_per_chunk {
                if locs.len() >= n {
                    break;
                }
                let size = size_of(locs.len())?;
                locs.push(SampleLoc { offset, size });
                offset += size as u64;
            }
        }
    }
    if locs.len() != n {
        return Err("MP4: Sample-Tabellen passen nicht zusammen".into());
    }

    let mut metas = Vec::with_capacity(n);
    let mut stts = stbl.stts.entries.iter().flat_map(|e| std::iter::repeat_n(e.sample_delta, e.sample_count as usize));
    let mut ctts = stbl.ctts.iter().flat_map(|c| c.entries.iter()).flat_map(|e| std::iter::repeat_n(e.sample_offset, e.sample_count as usize));
    let mut sync = stbl.stss.as_ref().map(|s| s.entries.iter().copied().peekable());
    let mut dts: i64 = 0;
    for i in 0..n {
        let delta = stts.next().ok_or("MP4: stts zu kurz")?;
        let offset = ctts.next().unwrap_or(0);
        let id = i as u32 + 1;
        let key = match sync.as_mut() {
            None => true,
            Some(it) => {
                while it.next_if(|s| *s < id).is_some() {}
                it.next_if(|s| *s == id).is_some()
            }
        };
        metas.push(SampleMeta { pts_us: us(dts + offset as i64, timescale).max(0), dur_us: us(delta as i64, timescale), key });
        dts += delta as i64;
    }
    Ok((metas, locs))
}

/// Track and codec information, samples not filled in yet.
struct Parsed {
    duration_us: i64,
    video: VideoInfo,
    audio: Option<AudioInfo>,
}

fn parse_info<R: Read + Seek>(reader: &Mp4Reader<R>) -> Result<Parsed, String> {
    let duration_us = reader.duration().as_micros() as i64;
    let mut video_id = None;
    let mut audio_id = None;
    for (id, track) in reader.tracks() {
        match track.track_type() {
            Ok(TrackType::Video) if video_id.is_none() => {
                if !matches!(track.media_type(), Ok(MediaType::H264)) {
                    return Err("Dieses Videoformat (nicht H.264) wird noch nicht unterstützt.".into());
                }
                video_id = Some(*id);
            }
            Ok(TrackType::Audio) if audio_id.is_none() && matches!(track.media_type(), Ok(MediaType::AAC)) => audio_id = Some(*id),
            _ => {}
        }
    }
    let video_id = video_id.ok_or("Keine Videospur gefunden.")?;

    let video = {
        let track = &reader.tracks()[&video_id];
        let avc1 = track.trak.mdia.minf.stbl.stsd.avc1.as_ref().ok_or("avc1 fehlt")?;
        let avcc = &avc1.avcc;
        let mut description = vec![
            avcc.configuration_version,
            avcc.avc_profile_indication,
            avcc.profile_compatibility,
            avcc.avc_level_indication,
            0xFC | (avcc.length_size_minus_one & 3),
            0xE0 | avcc.sequence_parameter_sets.len() as u8,
        ];
        for nal in &avcc.sequence_parameter_sets {
            description.extend_from_slice(&(nal.bytes.len() as u16).to_be_bytes());
            description.extend_from_slice(&nal.bytes);
        }
        description.push(avcc.picture_parameter_sets.len() as u8);
        for nal in &avcc.picture_parameter_sets {
            description.extend_from_slice(&(nal.bytes.len() as u16).to_be_bytes());
            description.extend_from_slice(&nal.bytes);
        }
        VideoInfo {
            track_id: video_id,
            width: avc1.width as u32,
            height: avc1.height as u32,
            codec: format!("avc1.{:02x}{:02x}{:02x}", avcc.avc_profile_indication, avcc.profile_compatibility, avcc.avc_level_indication),
            description,
            samples: Vec::new(),
            locs: Vec::new(),
        }
    };

    let audio = audio_id.and_then(|id| {
        let track = &reader.tracks()[&id];
        let mp4a = track.trak.mdia.minf.stbl.stsd.mp4a.as_ref()?;
        let spec = &mp4a.esds.as_ref()?.es_desc.dec_config.dec_specific;
        let (aot, freq, chan) = (spec.profile, spec.freq_index, spec.chan_conf);
        let description = vec![(aot << 3) | (freq >> 1), ((freq & 1) << 7) | (chan << 3)];
        Some(AudioInfo {
            track_id: id,
            codec: format!("mp4a.40.{aot}"),
            description,
            sample_rate: AAC_RATES.get(freq as usize).copied().unwrap_or(mp4a.samplerate.value() as u32),
            channels: chan.max(1) as u32,
            samples: Vec::new(),
            locs: Vec::new(),
        })
    });
    Ok(Parsed { duration_us, video, audio })
}

pub struct Movie {
    /// Only for fragmented files, which are demuxed through the `mp4` crate from a complete download.
    reader: Option<Mp4Reader<Cursor<Vec<u8>>>>,
    /// The whole file when `open` was given all of it.
    own: ByteCache,
    pub duration_us: i64,
    pub video: VideoInfo,
    pub audio: Option<AudioInfo>,
}

impl Movie {
    /// From the complete file. Fragmented files are indexed by reading every sample (and kept for `read`); others
    /// get the same sample table as `open_header` and keep the bytes as a one-span cache (see `take_cache`).
    pub fn open(bytes: Vec<u8>) -> Result<Movie, String> {
        let size = bytes.len() as u64;
        let mut parsed = {
            let reader = Mp4Reader::read_header(Cursor::new(&bytes[..]), size).map_err(|e| format!("MP4: {e}"))?;
            if reader.is_fragmented() || reader.moov.mvex.is_some() {
                None
            } else {
                let mut parsed = parse_info(&reader)?;
                Self::fill_tables(&reader, &mut parsed)?;
                Some(parsed)
            }
        };
        match parsed.take() {
            Some(p) => {
                let mut own = ByteCache::new();
                own.insert(0, bytes);
                Ok(Movie { reader: None, own, duration_us: p.duration_us, video: p.video, audio: p.audio })
            }
            None => Self::open_fragmented(bytes),
        }
    }

    /// Streaming entry: `ftyp` and `moov` boxes exactly as they are in the file (their offsets inside the file do
    /// not matter, the sample positions come from the tables). Fails for fragmented files.
    pub fn open_header(ftyp: &[u8], moov: &[u8]) -> Result<Movie, String> {
        let mut head = plain_box(ftyp)?;
        head.extend_from_slice(&plain_box(moov)?);
        let size = head.len() as u64;
        let reader = Mp4Reader::read_header(Cursor::new(&head[..]), size).map_err(|e| format!("MP4: {e}"))?;
        if reader.moov.mvex.is_some() {
            return Err("Fallback: fragmentiertes MP4".into());
        }
        let mut parsed = parse_info(&reader)?;
        Self::fill_tables(&reader, &mut parsed)?;
        Ok(Movie { reader: None, own: ByteCache::new(), duration_us: parsed.duration_us, video: parsed.video, audio: parsed.audio })
    }

    fn fill_tables<R: Read + Seek>(reader: &Mp4Reader<R>, parsed: &mut Parsed) -> Result<(), String> {
        (parsed.video.samples, parsed.video.locs) = sample_table(&reader.tracks()[&parsed.video.track_id])?;
        if let Some(a) = parsed.audio.as_mut() {
            (a.samples, a.locs) = sample_table(&reader.tracks()[&a.track_id])?;
        }
        Ok(())
    }

    fn open_fragmented(bytes: Vec<u8>) -> Result<Movie, String> {
        let size = bytes.len() as u64;
        let mut reader = Mp4Reader::read_header(Cursor::new(bytes), size).map_err(|e| format!("MP4: {e}"))?;
        let mut parsed = parse_info(&reader)?;
        parsed.video.samples = Self::index(&mut reader, parsed.video.track_id)?;
        if let Some(a) = parsed.audio.as_mut() {
            a.samples = Self::index(&mut reader, a.track_id)?;
        }
        Ok(Movie { reader: Some(reader), own: ByteCache::new(), duration_us: parsed.duration_us, video: parsed.video, audio: parsed.audio })
    }

    /// Timing and keyframe flag of every sample of `track_id` (reads each sample once; only the metadata is kept).
    fn index(reader: &mut Mp4Reader<Cursor<Vec<u8>>>, track_id: u32) -> Result<Vec<SampleMeta>, String> {
        let timescale = reader.tracks()[&track_id].timescale();
        let count = reader.sample_count(track_id).map_err(|e| format!("MP4: {e}"))?;
        let mut samples = Vec::with_capacity(count as usize);
        for id in 1..=count {
            let Some(s) = reader.read_sample(track_id, id).map_err(|e| format!("MP4: {e}"))? else { break };
            samples.push(SampleMeta {
                pts_us: us(s.start_time as i64 + s.rendering_offset as i64, timescale).max(0),
                dur_us: us(s.duration as i64, timescale),
                key: s.is_sync,
            });
        }
        Ok(samples)
    }

    /// Fragmented movie read through the `mp4` crate (no byte cache involved).
    pub fn is_legacy(&self) -> bool {
        self.reader.is_some()
    }

    /// Hands out the bytes `open` kept (the whole file).
    pub fn take_cache(&mut self) -> ByteCache {
        std::mem::take(&mut self.own)
    }

    fn locate(&self, track_id: u32, index: usize) -> Option<(SampleMeta, SampleLoc)> {
        if track_id == self.video.track_id {
            Some((*self.video.samples.get(index)?, *self.video.locs.get(index)?))
        } else {
            let a = self.audio.as_ref().filter(|a| a.track_id == track_id)?;
            Some((*a.samples.get(index)?, *a.locs.get(index)?))
        }
    }

    /// The sample built from cached bytes.
    pub fn next_chunk(&mut self, track_id: u32, index: usize, cache: &ByteCache) -> Fetched {
        if let Some(reader) = self.reader.as_mut() {
            let meta = if track_id == self.video.track_id {
                self.video.samples.get(index).copied()
            } else {
                self.audio.as_ref().filter(|a| a.track_id == track_id).and_then(|a| a.samples.get(index).copied())
            };
            let Some(meta) = meta else { return Fetched::End };
            return match reader.read_sample(track_id, index as u32 + 1).ok().flatten() {
                Some(s) => Fetched::Ready(Chunk { key: meta.key, ts_us: meta.pts_us, dur_us: meta.dur_us, data: s.bytes.to_vec() }),
                None => Fetched::End,
            };
        }
        let Some((meta, loc)) = self.locate(track_id, index) else { return Fetched::End };
        match cache.read(loc.offset, loc.size as u64) {
            Some(data) => Fetched::Ready(Chunk { key: meta.key, ts_us: meta.pts_us, dur_us: meta.dur_us, data }),
            None => Fetched::Missing,
        }
    }

    /// The encoded sample `index` (0-based) of `track_id`, from the bytes `open` kept.
    pub fn read(&mut self, track_id: u32, index: usize) -> Option<Chunk> {
        let own = std::mem::take(&mut self.own);
        let result = self.next_chunk(track_id, index, &own);
        self.own = own;
        match result {
            Fetched::Ready(c) => Some(c),
            _ => None,
        }
    }

    /// Byte ranges (merged, `end` exclusive) of the samples to decode next: video from `next_video` and audio
    /// from `next_audio` up to presentation time `horizon_us`, video at most `max_bytes`.
    pub fn wanted(&self, next_video: usize, next_audio: usize, horizon_us: i64, max_bytes: u64) -> Vec<(u64, u64)> {
        let mut ranges = Vec::new();
        let mut bytes = 0u64;
        for (i, (meta, loc)) in self.video.samples.iter().zip(&self.video.locs).enumerate().skip(next_video) {
            if i > next_video && (meta.pts_us > horizon_us || bytes > max_bytes) {
                break;
            }
            bytes += loc.size as u64;
            ranges.push((loc.offset, loc.offset + loc.size as u64));
        }
        if let Some(a) = &self.audio {
            for (i, (meta, loc)) in a.samples.iter().zip(&a.locs).enumerate().skip(next_audio) {
                if i > next_audio && meta.pts_us > horizon_us {
                    break;
                }
                ranges.push((loc.offset, loc.offset + loc.size as u64));
            }
        }
        merge_ranges(ranges, MERGE_GAP)
    }

    /// Lowest file offset of the samples from time `t_us` on (everything before it can be dropped from the cache).
    pub fn offset_at(&self, t_us: i64) -> u64 {
        let v = self.video.samples.partition_point(|s| s.pts_us < t_us);
        let mut low = self.video.locs.get(v).map_or(u64::MAX, |l| l.offset);
        if let Some(a) = &self.audio {
            let i = a.samples.partition_point(|s| s.pts_us < t_us);
            low = low.min(a.locs.get(i).map_or(u64::MAX, |l| l.offset));
        }
        low
    }
}

/// Index of the keyframe to start decoding from to show the picture at `t_us`: the last keyframe whose
/// presentation time is not after it (the first sample when none is).
pub fn video_start_for(samples: &[SampleMeta], t_us: i64) -> usize {
    samples.iter().rposition(|s| s.key && s.pts_us <= t_us).unwrap_or(0)
}

/// Index of the first audio sample that still sounds at `t_us`.
pub fn audio_start_for(samples: &[SampleMeta], t_us: i64) -> usize {
    samples.partition_point(|s| s.pts_us + s.dur_us <= t_us).min(samples.len().saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny() -> Movie {
        Movie::open(include_bytes!("testdata/tiny.mp4").to_vec()).expect("demux")
    }

    #[test]
    fn reads_tracks_and_codec_config() {
        let m = tiny();
        assert_eq!((m.video.width, m.video.height), (64, 48));
        assert!(m.video.codec.starts_with("avc1."), "{}", m.video.codec);
        assert_eq!(m.video.description[0], 1);
        assert_eq!(m.video.samples.len(), 30);
        let a = m.audio.as_ref().expect("audio track");
        assert_eq!(a.codec, "mp4a.40.2");
        assert_eq!(a.description.len(), 2);
        assert_eq!(a.sample_rate, 44100);
        assert!((m.duration_us - 2_000_000).abs() < 100_000, "{}", m.duration_us);
    }

    #[test]
    fn first_sample_is_a_keyframe_and_reads_back() {
        let mut m = tiny();
        let id = m.video.track_id;
        let c = m.read(id, 0).unwrap();
        assert!(c.key && !c.data.is_empty());
        assert!(m.read(id, 30).is_none());
    }

    #[test]
    fn seeking_picks_keyframe_before_and_audio_sample_at() {
        let m = tiny();
        let i = video_start_for(&m.video.samples, 1_000_000);
        assert!(m.video.samples[i].key && m.video.samples[i].pts_us <= 1_000_000);
        assert!(i > 0);
        let a = m.audio.unwrap().samples;
        let j = audio_start_for(&a, 1_000_000);
        assert!(a[j].pts_us + a[j].dur_us > 1_000_000 && a[j].pts_us <= 1_000_000);
    }

    const TAIL: &[u8] = include_bytes!("testdata/tail.mp4");
    const TINY: &[u8] = include_bytes!("testdata/tiny.mp4");

    /// A "server": answers range requests from `file` and counts them.
    struct Server<'a> {
        file: &'a [u8],
        requests: Vec<(u64, u64)>,
    }

    impl Server<'_> {
        fn get(&mut self, cache: &mut ByteCache, start: u64, end: u64) {
            let end = end.min(self.file.len() as u64);
            self.requests.push((start, end));
            cache.insert(start, self.file[start as usize..end as usize].to_vec());
        }
    }

    /// Opens through `HeaderScan` with a first request of `head` bytes, like the player does.
    fn open_streamed(server: &mut Server, head: u64) -> (Movie, ByteCache) {
        let total = server.file.len() as u64;
        let mut cache = ByteCache::new();
        server.get(&mut cache, 0, head);
        let mut scan = HeaderScan::new(total);
        loop {
            match scan.step(&cache).expect("scan") {
                Scan::Fetch { start, end } => server.get(&mut cache, start, end),
                Scan::Done { ftyp, moov } => {
                    let movie = Movie::open_header(&cache.read(ftyp.0, ftyp.1).unwrap(), &cache.read(moov.0, moov.1).unwrap()).expect("open_header");
                    return (movie, cache);
                }
            }
        }
    }

    #[test]
    fn sample_table_matches_reading_every_sample() {
        for bytes in [TINY, TAIL] {
            let new = Movie::open(bytes.to_vec()).unwrap();
            let old = Movie::open_fragmented(bytes.to_vec()).unwrap();
            assert_eq!(new.video.samples, old.video.samples);
            assert_eq!(new.audio.as_ref().unwrap().samples, old.audio.as_ref().unwrap().samples);
            assert_eq!(new.video.locs.len(), new.video.samples.len());
            // Every location holds exactly the bytes the old reader returned.
            let (mut new, mut old) = (new, old);
            for (id, n) in [(new.video.track_id, new.video.samples.len()), (new.audio.as_ref().unwrap().track_id, new.audio.as_ref().unwrap().samples.len())] {
                for i in 0..n {
                    assert_eq!(new.read(id, i).unwrap().data, old.read(id, i).unwrap().data, "track {id} sample {i}");
                }
            }
        }
    }

    #[test]
    fn header_only_parsing_and_on_demand_reads() {
        for (bytes, head) in [(TINY, 4096u64), (TAIL, 4096)] {
            let mut server = Server { file: bytes, requests: Vec::new() };
            let (mut movie, mut cache) = open_streamed(&mut server, head);
            let mut full = Movie::open(bytes.to_vec()).unwrap();
            assert_eq!(movie.video.samples, full.video.samples);
            assert!(cache.len() < bytes.len() as u64 || bytes.len() as u64 <= head + 4096);
            // The cache holds no sample yet beyond the head, so reads report `Missing` until fetched.
            let vid = movie.video.track_id;
            let last = movie.video.samples.len() - 1;
            assert!(matches!(movie.next_chunk(vid, last, &ByteCache::new()), Fetched::Missing));
            let total = bytes.len() as u64;
            let mut guard = 0;
            loop {
                let wanted = movie.wanted(0, 0, i64::MAX, u64::MAX);
                let Some((s, e)) = next_request(&wanted, &cache, total) else { break };
                server.get(&mut cache, s, e);
                guard += 1;
                assert!(guard < 50);
            }
            for i in 0..=last {
                let Fetched::Ready(c) = movie.next_chunk(vid, i, &cache) else { panic!("sample {i} missing") };
                assert_eq!(c.data, full.read(vid, i).unwrap().data);
            }
            assert!(matches!(movie.next_chunk(vid, last + 1, &cache), Fetched::End));
        }
    }

    #[test]
    fn moov_after_mdat_costs_few_requests_and_only_needed_bytes() {
        let mut server = Server { file: TAIL, requests: Vec::new() };
        let (_movie, cache) = open_streamed(&mut server, 1024);
        // Head, one header probe past mdat, then the exact moov.
        assert!(server.requests.len() <= 3, "{:?}", server.requests);
        assert_eq!(server.requests.last().unwrap(), &(22486, 22486 + 2391));
        assert!(cache.first_gap(22486, 22486 + 2391).is_none());
    }

    #[test]
    fn scan_handles_64_bit_and_zero_sized_boxes() {
        // ftyp, a 64-bit free box, a zero-size (to the end) mdat holding moov-less data: no moov -> error.
        let mut f = Vec::new();
        f.extend_from_slice(&[0, 0, 0, 16, b'f', b't', b'y', b'p', b'i', b's', b'o', b'm', 0, 0, 0, 0]);
        f.extend_from_slice(&[0, 0, 0, 1, b'f', b'r', b'e', b'e', 0, 0, 0, 0, 0, 0, 0, 24, 1, 2, 3, 4, 5, 6, 7, 8]);
        f.extend_from_slice(&[0, 0, 0, 0, b'm', b'd', b'a', b't', 9, 9, 9, 9]);
        let mut cache = ByteCache::new();
        cache.insert(0, f.clone());
        assert!(HeaderScan::new(f.len() as u64).step(&cache).unwrap_err().starts_with("Fallback"));
        // The same with a moov (any bytes) between: both boxes are located, 64-bit header included.
        let mut g = f[..16].to_vec();
        g.extend_from_slice(&[0, 0, 0, 1, b'm', b'o', b'o', b'v', 0, 0, 0, 0, 0, 0, 0, 20, 1, 2, 3, 4]);
        let mut cache = ByteCache::new();
        cache.insert(0, g.clone());
        match HeaderScan::new(g.len() as u64).step(&cache).unwrap() {
            Scan::Done { ftyp, moov } => assert_eq!((ftyp, moov), ((0, 16), (16, 20))),
            _ => panic!("expected Done"),
        }
        assert_eq!(plain_box(&g[16..]).unwrap(), [0, 0, 0, 12, b'm', b'o', b'o', b'v', 1, 2, 3, 4]);
    }

    #[test]
    fn fragmented_scan_falls_back() {
        let mut f = vec![0, 0, 0, 8, b'f', b't', b'y', b'p'];
        f.extend_from_slice(&[0, 0, 0, 8, b'm', b'o', b'o', b'f']);
        let mut cache = ByteCache::new();
        cache.insert(0, f.clone());
        assert!(HeaderScan::new(f.len() as u64).step(&cache).is_err());
    }

    #[test]
    fn byte_cache_overlaps_gaps_and_eviction() {
        let mut c = ByteCache::new();
        c.insert(10, vec![1; 10]);
        c.insert(30, vec![3; 10]);
        assert_eq!(c.first_gap(10, 40), Some((20, 30)));
        c.insert(15, vec![2; 20]); // overlaps both
        assert_eq!(c.read(10, 5).unwrap(), vec![1; 5]);
        assert_eq!(c.read(15, 20).unwrap(), vec![2; 20]);
        assert_eq!(c.read(35, 5).unwrap(), vec![3; 5]);
        assert!(c.has(10, 30));
        assert!(!c.has(9, 2));
        c.insert(5, vec![0; 5]); // touching: read across both spans
        assert_eq!(c.read(5, 10).unwrap().len(), 10);
        c.evict_before(36);
        assert!(!c.has(10, 1) && c.has(36, 4));
    }

    #[test]
    fn evict_far_keeps_the_nearest_spans_within_budget() {
        let mut c = ByteCache::new();
        for i in 0..10u64 {
            c.insert(i * 100, vec![0; 100]);
        }
        c.evict_far(450, 300);
        assert_eq!(c.len(), 300);
        assert!(c.has(400, 100), "span at the center stays");
        assert!(c.has(300, 100) && c.has(500, 100));
        assert!(!c.has(0, 1) && !c.has(900, 1));
        c.evict_far(450, 0);
        assert!(c.has(400, 100) && c.len() == 100, "the center span is never dropped");
    }

    #[test]
    fn requests_are_merged_padded_and_capped() {
        assert_eq!(merge_ranges(vec![(500_000, 600_000), (0, 100), (200_000, 300_000)], MERGE_GAP), vec![(0, 600_000)]);
        assert_eq!(merge_ranges(vec![(0, 10), (10_000_000, 10_000_010)], MERGE_GAP).len(), 2);
        let cache = ByteCache::new();
        assert_eq!(next_request(&[(100, 200)], &cache, 50_000_000), Some((100, 100 + MIN_REQUEST)));
        assert_eq!(next_request(&[(0, 100_000_000)], &cache, 50_000_000), Some((0, MAX_REQUEST)));
        assert_eq!(next_request(&[(100, 200)], &cache, 500), Some((100, 500)));
    }
}
