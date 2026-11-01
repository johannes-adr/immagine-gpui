//! H.264 decoding for the app's own video player on macOS and iOS: VideoToolbox (hardware where the device has it,
//! software in the simulator) behind [`VideoDecode`].
//!
//! Design (same contract as `host/web/codecs.rs`):
//!
//! * A dedicated decode thread owns the `VTDecompressionSession`. `decode`/`flush`/`reset` on the main thread only
//!   send a message (unbounded channel, never blocks).
//! * The session is synchronous (no async flag), so VideoToolbox outputs in *decode* order. The thread converts each
//!   BGRA picture to RGBA (scaled so the longest side is at most [`MAX_SIDE`]) and puts it into a reorder buffer
//!   keyed by pts. A picture is released when the buffer holds more than the stream's reorder depth (read from the
//!   SPS: `max_num_reorder_frames`, 0 for POC type 2 / baseline, else the DPB size), or when it directly follows
//!   the previous released picture (no gap, so no earlier picture can still come). `flush` releases everything.
//! * Released pictures go into a mailbox (`Mutex<VecDeque>`) that [`VideoDecode::queue_len`] drains into `on_frame`
//!   on the main thread (`try_lock`, the main thread never waits). `queue_len` = messages not yet processed plus
//!   pictures still in the mailbox, so the player keeps ticking until the last picture was delivered.
//! * `reset` bumps a generation counter: queued samples and pictures of older generations are dropped, the thread
//!   discards its reorder buffer and waits for the next keyframe.
//! * `kVTInvalidSessionErr` (iOS invalidates sessions of backgrounded apps) drops the session; the thread skips
//!   samples until the next keyframe and builds a new session there. Other failures, including bad data, are
//!   reported through `error()`.

#![allow(non_upper_case_globals)]

use super::apple_ffi::*;
use crate::host::{Chunk, Decoded, VideoConfig, VideoDecode};
use std::collections::{BTreeMap, VecDeque};
use std::ffi::c_void;
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// Longest side of the pictures handed to GPUI: more pixels than the screen shows only cost copy time.
const MAX_SIDE: usize = 1280;

enum Msg {
    Sample { serial: u32, chunk: Chunk },
    Flush { serial: u32 },
    Reset,
}

struct Frame {
    pts: i64,
    dur: i64,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

#[derive(Default)]
struct Shared {
    serial: AtomicU32,
    /// Messages sent and not yet fully processed (a sample counts until its pictures are in the reorder buffer or
    /// the mailbox).
    in_flight: AtomicI64,
    mailbox: Mutex<VecDeque<(u32, i64, Decoded)>>,
    error: Mutex<Option<String>>,
}

pub struct AppleVideoDecoder {
    tx: Sender<Msg>,
    shared: Arc<Shared>,
    on_frame: Box<dyn Fn(i64, Decoded)>,
}

impl AppleVideoDecoder {
    pub fn create(config: VideoConfig, on_frame: impl Fn(i64, Decoded) + 'static) -> Option<Self> {
        let shared = Arc::new(Shared::default());
        let (tx, rx) = channel();
        let worker_shared = shared.clone();
        std::thread::Builder::new()
            .name("video-decode".into())
            .spawn(move || Worker::new(config, worker_shared).run(rx))
            .ok()?;
        Some(Self { tx, shared, on_frame: Box::new(on_frame) })
    }

    fn send(&self, msg: Msg) {
        self.shared.in_flight.fetch_add(1, Ordering::SeqCst);
        if self.tx.send(msg).is_err() {
            self.shared.in_flight.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Moves finished pictures of the current generation to `on_frame`. Returns how many are still in the mailbox
    /// (only non-zero when the lock was busy).
    fn deliver(&self) -> i64 {
        let serial = self.shared.serial.load(Ordering::SeqCst);
        let batch: Vec<_> = match self.shared.mailbox.try_lock() {
            Ok(mut m) => m.drain(..).collect(),
            Err(_) => return 1,
        };
        for (g, ts, picture) in batch {
            if g == serial {
                (self.on_frame)(ts, picture);
            }
        }
        0
    }
}

impl VideoDecode for AppleVideoDecoder {
    fn decode(&self, chunk: &Chunk) {
        self.send(Msg::Sample { serial: self.shared.serial.load(Ordering::SeqCst), chunk: chunk.clone() });
    }

    fn queue_len(&self) -> u32 {
        // Read the counter first: the thread pushes pictures before it decrements, so a counter of 0 guarantees
        // that the drain below sees everything.
        let pending = self.shared.in_flight.load(Ordering::SeqCst).max(0);
        let left = self.deliver();
        (pending + left) as u32
    }

    fn reset(&self) {
        self.shared.serial.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut m) = self.shared.mailbox.try_lock() {
            m.clear();
        }
        self.send(Msg::Reset);
    }

    fn flush(&self) {
        self.send(Msg::Flush { serial: self.shared.serial.load(Ordering::SeqCst) });
    }

    fn error(&self) -> Option<String> {
        self.shared.error.try_lock().ok().and_then(|e| e.clone())
    }
}

// ---- decode thread ------------------------------------------------------------------------------------------

/// State the VideoToolbox output callback writes to (it may run on any thread).
#[derive(Default)]
struct CbCtx {
    out: Mutex<Vec<Frame>>,
    failure: Mutex<Option<OSStatus>>,
}

struct Session {
    raw: VTDecompressionSessionRef,
    format: CMFormatDescriptionRef,
    ctx: *const CbCtx,
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            VTDecompressionSessionWaitForAsynchronousFrames(self.raw);
            VTDecompressionSessionInvalidate(self.raw);
            CFRelease(self.raw as CFTypeRef);
            CFRelease(self.format);
            drop(Arc::from_raw(self.ctx));
        }
    }
}

struct Params {
    nal_len: i32,
    sps: Vec<Vec<u8>>,
    pps: Vec<Vec<u8>>,
}

/// Splits an `avcC` record into its NAL length size and parameter sets.
fn parse_avcc(avcc: &[u8]) -> Result<Params, String> {
    let bad = || "ungültiger avcC-Eintrag".to_string();
    if avcc.len() < 7 || avcc[0] != 1 {
        return Err(bad());
    }
    let take = |at: &mut usize, count: usize| -> Result<Vec<Vec<u8>>, String> {
        let mut v = Vec::new();
        for _ in 0..count {
            let len = u16::from_be_bytes([*avcc.get(*at).ok_or_else(bad)?, *avcc.get(*at + 1).ok_or_else(bad)?]) as usize;
            v.push(avcc.get(*at + 2..*at + 2 + len).ok_or_else(bad)?.to_vec());
            *at += 2 + len;
        }
        Ok(v)
    };
    let mut at = 6;
    let sps = take(&mut at, (avcc[5] & 0x1f) as usize)?;
    let pps_count = *avcc.get(at).ok_or_else(bad)? as usize;
    at += 1;
    let pps = take(&mut at, pps_count)?;
    if sps.is_empty() || pps.is_empty() {
        return Err(bad());
    }
    Ok(Params { nal_len: (avcc[4] & 3) as i32 + 1, sps, pps })
}

/// MSB-first bit reader over an RBSP (emulation prevention bytes already removed).
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn bit(&mut self) -> Option<u32> {
        let byte = *self.data.get(self.pos / 8)?;
        let b = (byte >> (7 - self.pos % 8)) & 1;
        self.pos += 1;
        Some(b as u32)
    }

    fn bits(&mut self, n: u32) -> Option<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 1) | self.bit()?;
        }
        Some(v)
    }

    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.bit()? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        Some(((1u64 << zeros) - 1 + self.bits(zeros)? as u64) as u32)
    }

    fn se(&mut self) -> Option<i32> {
        let k = self.ue()?;
        Some(if k % 2 == 1 { k.div_ceil(2) as i32 } else { -((k / 2) as i32) })
    }

    fn skip_scaling_list(&mut self, size: usize) -> Option<()> {
        let (mut last, mut next) = (8i32, 8i32);
        for _ in 0..size {
            if next != 0 {
                next = (last + self.se()? + 256) % 256;
            }
            if next != 0 {
                last = next;
            }
        }
        Some(())
    }

    fn skip_hrd(&mut self) -> Option<()> {
        let cpb = self.ue()? + 1;
        self.bits(8)?;
        for _ in 0..cpb {
            self.ue()?;
            self.ue()?;
            self.bit()?;
        }
        self.bits(20)?;
        Some(())
    }
}

fn unescape(nal: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(nal.len());
    let mut zeros = 0;
    for &b in nal {
        if zeros >= 2 && b == 3 {
            zeros = 0;
            continue;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        out.push(b);
    }
    out
}

/// How many pictures can precede an earlier one in output order, from the SPS. `None` if it cannot be parsed.
fn reorder_depth(sps: &[u8]) -> Option<usize> {
    let rbsp = unescape(sps.get(1..)?);
    let mut b = Bits { data: &rbsp, pos: 0 };
    let profile = b.bits(8)?;
    b.bits(8)?;
    let level = b.bits(8)?;
    b.ue()?;
    let mut chroma = 1;
    if [100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135].contains(&profile) {
        chroma = b.ue()?;
        if chroma == 3 {
            b.bit()?;
        }
        b.ue()?;
        b.ue()?;
        b.bit()?;
        if b.bit()? == 1 {
            for i in 0..(if chroma != 3 { 8 } else { 12 }) {
                if b.bit()? == 1 {
                    b.skip_scaling_list(if i < 6 { 16 } else { 64 })?;
                }
            }
        }
    }
    let _ = chroma;
    b.ue()?;
    let poc_type = b.ue()?;
    match poc_type {
        0 => {
            b.ue()?;
        }
        1 => {
            b.bit()?;
            b.se()?;
            b.se()?;
            for _ in 0..b.ue()? {
                b.se()?;
            }
        }
        // Output order equals decode order.
        _ => return Some(0),
    }
    b.ue()?;
    b.bit()?;
    let width_mbs = b.ue()? + 1;
    let height_map_units = b.ue()? + 1;
    let frame_mbs_only = b.bit()?;
    if frame_mbs_only == 0 {
        b.bit()?;
    }
    b.bit()?;
    if b.bit()? == 1 {
        for _ in 0..4 {
            b.ue()?;
        }
    }
    let height_mbs = height_map_units * (2 - frame_mbs_only);
    if b.bit()? == 1 {
        // VUI
        if b.bit()? == 1 && b.bits(8)? == 255 {
            b.bits(32)?;
        }
        if b.bit()? == 1 {
            b.bit()?;
        }
        if b.bit()? == 1 {
            b.bits(4)?;
            if b.bit()? == 1 {
                b.bits(24)?;
            }
        }
        if b.bit()? == 1 {
            b.ue()?;
            b.ue()?;
        }
        if b.bit()? == 1 {
            b.bits(32)?;
            b.bits(32)?;
            b.bit()?;
        }
        let nal_hrd = b.bit()? == 1;
        if nal_hrd {
            b.skip_hrd()?;
        }
        let vcl_hrd = b.bit()? == 1;
        if vcl_hrd {
            b.skip_hrd()?;
        }
        if nal_hrd || vcl_hrd {
            b.bit()?;
        }
        b.bit()?;
        if b.bit()? == 1 {
            b.bit()?;
            b.ue()?;
            b.ue()?;
            b.ue()?;
            b.ue()?;
            return Some(b.ue()? as usize);
        }
    }
    if profile == 66 {
        return Some(0);
    }
    // No bitstream restriction: a decoder must assume the DPB size (H.264 table A-1, MaxDpbMbs by level).
    let max_dpb_mbs: u32 = match level {
        9 | 10 => 396,
        11 => 900,
        12 | 13 | 20 => 2376,
        21 => 4752,
        22 | 30 => 8100,
        31 => 18000,
        32 => 20480,
        40 | 41 => 32768,
        42 => 34816,
        50 => 110400,
        _ => 184320,
    };
    Some((max_dpb_mbs / (width_mbs * height_mbs).max(1)).clamp(1, 16) as usize)
}

/// Converts a locked-in BGRA pixel buffer to RGBA, box-filtering down so the longest side is at most [`MAX_SIDE`].
unsafe fn convert(image: CVImageBufferRef) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        if CVPixelBufferLockBaseAddress(image, kCVPixelBufferLock_ReadOnly) != 0 {
            return None;
        }
        let (w, h) = (CVPixelBufferGetWidth(image), CVPixelBufferGetHeight(image));
        let stride = CVPixelBufferGetBytesPerRow(image);
        let base = CVPixelBufferGetBase(image);
        let result = if base.is_null() || w == 0 || h == 0 {
            None
        } else {
            let src = std::slice::from_raw_parts(base, stride * (h - 1) + w * 4);
            Some(scale_bgra_to_rgba(src, stride, w, h))
        };
        CVPixelBufferUnlockBaseAddress(image, kCVPixelBufferLock_ReadOnly);
        result
    }
}

#[allow(non_snake_case)]
unsafe fn CVPixelBufferGetBase(image: CVImageBufferRef) -> *const u8 {
    unsafe { CVPixelBufferGetBaseAddress(image) }
}

pub(crate) fn scale_bgra_to_rgba(src: &[u8], stride: usize, w: usize, h: usize) -> (u32, u32, Vec<u8>) {
    let k = (MAX_SIDE as f64 / w.max(h) as f64).min(1.);
    let (dw, dh) = (((w as f64 * k).round() as usize).max(1), ((h as f64 * k).round() as usize).max(1));
    let mut out = vec![255u8; dw * dh * 4];
    if (dw, dh) == (w, h) {
        for y in 0..h {
            let row = &src[y * stride..y * stride + w * 4];
            for (d, s) in out[y * w * 4..(y + 1) * w * 4].chunks_exact_mut(4).zip(row.chunks_exact(4)) {
                d[0] = s[2];
                d[1] = s[1];
                d[2] = s[0];
            }
        }
        return (w as u32, h as u32, out);
    }
    // Up to 2x2 taps spread over the source footprint of each destination pixel.
    let taps = |d: usize, dn: usize, sn: usize| -> ([usize; 2], usize) {
        let (a, b) = (d * sn / dn, ((d + 1) * sn / dn).max(d * sn / dn + 1));
        let n = (b - a).min(2);
        let mut t = [0; 2];
        for (i, slot) in t.iter_mut().enumerate().take(n) {
            *slot = (a + (b - a) * (2 * i + 1) / (2 * n)).min(sn - 1);
        }
        (t, n)
    };
    let xs: Vec<_> = (0..dw).map(|x| taps(x, dw, w)).collect();
    for y in 0..dh {
        let (ys, yn) = taps(y, dh, h);
        for (x, (xt, xn)) in xs.iter().enumerate() {
            let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
            for &sy in &ys[..yn] {
                for &sx in &xt[..*xn] {
                    let p = sy * stride + sx * 4;
                    b += src[p] as u32;
                    g += src[p + 1] as u32;
                    r += src[p + 2] as u32;
                }
            }
            let n = (yn * xn) as u32;
            let d = (y * dw + x) * 4;
            out[d] = (r / n) as u8;
            out[d + 1] = (g / n) as u8;
            out[d + 2] = (b / n) as u8;
        }
    }
    (dw as u32, dh as u32, out)
}

unsafe extern "C" fn on_output(
    refcon: *mut c_void,
    source_frame_refcon: *mut c_void,
    status: OSStatus,
    _info_flags: u32,
    image: CVImageBufferRef,
    pts: CMTime,
    _duration: CMTime,
) {
    let ctx = unsafe { &*(refcon as *const CbCtx) };
    if status != 0 {
        *ctx.failure.lock().unwrap_or_else(|e| e.into_inner()) = Some(status);
        return;
    }
    if image.is_null() {
        return; // dropped frame
    }
    if let Some((width, height, rgba)) = unsafe { convert(image) } {
        let frame = Frame { pts: pts.as_micros(), dur: source_frame_refcon as usize as i64, width, height, rgba };
        ctx.out.lock().unwrap_or_else(|e| e.into_inner()).push(frame);
    }
}

fn describe(status: OSStatus) -> String {
    let name = match status {
        kVTVideoDecoderBadDataErr => "fehlerhafte Videodaten",
        kVTVideoDecoderUnsupportedDataFormatErr => "Videoformat nicht unterstützt",
        kVTVideoDecoderMalfunctionErr => "Decoder ausgefallen",
        kVTInvalidSessionErr => "Sitzung ungültig",
        _ => "VideoToolbox",
    };
    format!("{name}, OSStatus {status}")
}

struct Worker {
    shared: Arc<Shared>,
    params: Result<Params, String>,
    depth: usize,
    session: Option<Session>,
    /// The next sample must be a keyframe (start, after a reset, after the session was lost).
    need_key: bool,
    failed: bool,
    held: BTreeMap<i64, Frame>,
    last_end: Option<i64>,
    /// Generation of the message being processed: tags the pictures it releases.
    serial: u32,
}

impl Worker {
    fn new(config: VideoConfig, shared: Arc<Shared>) -> Self {
        let params = parse_avcc(&config.description);
        let depth = params.as_ref().ok().and_then(|p| reorder_depth(&p.sps[0])).unwrap_or(16);
        Self { shared, params, depth, session: None, need_key: true, failed: false, held: BTreeMap::new(), last_end: None, serial: 0 }
    }

    fn run(mut self, rx: Receiver<Msg>) {
        while let Ok(msg) = rx.recv() {
            match msg {
                Msg::Sample { serial, chunk } => {
                    if serial == self.shared.serial.load(Ordering::SeqCst) {
                        self.serial = serial;
                        self.decode(&chunk);
                    }
                }
                Msg::Flush { serial } => {
                    if serial == self.shared.serial.load(Ordering::SeqCst) {
                        self.serial = serial;
                        if let Some(s) = &self.session {
                            unsafe {
                                VTDecompressionSessionFinishDelayedFrames(s.raw);
                                VTDecompressionSessionWaitForAsynchronousFrames(s.raw);
                            }
                        }
                        self.collect();
                        self.release(true);
                    }
                }
                Msg::Reset => {
                    if let Some(s) = &self.session {
                        unsafe { VTDecompressionSessionWaitForAsynchronousFrames(s.raw) };
                        let ctx = unsafe { &*s.ctx };
                        ctx.out.lock().unwrap_or_else(|e| e.into_inner()).clear();
                        *ctx.failure.lock().unwrap_or_else(|e| e.into_inner()) = None;
                    }
                    self.held.clear();
                    self.last_end = None;
                    self.need_key = true;
                    self.failed = false;
                    *self.shared.error.lock().unwrap_or_else(|e| e.into_inner()) = None;
                }
            }
            self.shared.in_flight.fetch_sub(1, Ordering::SeqCst);
        }
    }

    fn fail(&mut self, message: String) {
        self.failed = true;
        *self.shared.error.lock().unwrap_or_else(|e| e.into_inner()) = Some(message);
    }

    fn create_session(&mut self) -> Result<(), String> {
        let params = self.params.as_ref().map_err(|e| e.clone())?;
        unsafe {
            let sets: Vec<*const u8> = params.sps.iter().chain(&params.pps).map(|s| s.as_ptr()).collect();
            let sizes: Vec<usize> = params.sps.iter().chain(&params.pps).map(|s| s.len()).collect();
            let mut format: CMFormatDescriptionRef = std::ptr::null();
            let status = CMVideoFormatDescriptionCreateFromH264ParameterSets(
                std::ptr::null(),
                sets.len(),
                sets.as_ptr(),
                sizes.as_ptr(),
                params.nal_len,
                &mut format,
            );
            if status != 0 || format.is_null() {
                return Err(format!("Videoformat nicht lesbar ({})", describe(status)));
            }
            let pixel_format = kCVPixelFormatType_32BGRA;
            let number = CFNumberCreate(std::ptr::null(), kCFNumberSInt32Type, &pixel_format as *const i32 as *const c_void);
            let keys = [kCVPixelBufferPixelFormatTypeKey];
            let values = [number];
            let attributes = CFDictionaryCreate(
                std::ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                &kCFTypeDictionaryKeyCallBacks as *const u8 as *const c_void,
                &kCFTypeDictionaryValueCallBacks as *const u8 as *const c_void,
            );
            let ctx = Arc::into_raw(Arc::new(CbCtx::default()));
            let record = VTDecompressionOutputCallbackRecord { callback: on_output, refcon: ctx as *mut c_void };
            let mut raw: VTDecompressionSessionRef = std::ptr::null_mut();
            let status = VTDecompressionSessionCreate(std::ptr::null(), format, std::ptr::null(), attributes, &record, &mut raw);
            CFRelease(attributes);
            CFRelease(number);
            if status != 0 || raw.is_null() {
                CFRelease(format);
                drop(Arc::from_raw(ctx));
                return Err(describe(status));
            }
            self.session = Some(Session { raw, format, ctx });
        }
        Ok(())
    }

    /// One sample through the session. `Ok(false)`: the session was invalid (it is dropped).
    fn submit(&mut self, chunk: &Chunk) -> Result<bool, String> {
        let session = self.session.as_ref().ok_or("keine Sitzung")?;
        unsafe {
            let mut block: CMBlockBufferRef = std::ptr::null();
            let len = chunk.data.len();
            let mut status = CMBlockBufferCreateWithMemoryBlock(
                std::ptr::null(),
                std::ptr::null_mut(),
                len,
                std::ptr::null(),
                std::ptr::null(),
                0,
                len,
                kCMBlockBufferAssureMemoryNowFlag,
                &mut block,
            );
            if status == 0 {
                status = CMBlockBufferReplaceDataBytes(chunk.data.as_ptr() as *const c_void, block, 0, len);
            }
            if status != 0 {
                if !block.is_null() {
                    CFRelease(block);
                }
                return Err(describe(status));
            }
            let timing = CMSampleTimingInfo {
                duration: CMTime::micros(chunk.dur_us),
                presentation_time_stamp: CMTime::micros(chunk.ts_us),
                decode_time_stamp: CMTime::INVALID,
            };
            let mut sample: CMSampleBufferRef = std::ptr::null();
            let status = CMSampleBufferCreateReady(std::ptr::null(), block, session.format, 1, 1, &timing, 1, &len, &mut sample);
            CFRelease(block);
            if status != 0 || sample.is_null() {
                return Err(describe(status));
            }
            let mut info = 0u32;
            let status = VTDecompressionSessionDecodeFrame(session.raw, sample, 0, chunk.dur_us.max(0) as usize as *mut c_void, &mut info);
            CFRelease(sample);
            match status {
                0 => Ok(true),
                kVTInvalidSessionErr => Ok(false),
                s => Err(describe(s)),
            }
        }
    }

    fn decode(&mut self, chunk: &Chunk) {
        if self.failed {
            return;
        }
        if self.need_key && !chunk.key {
            return;
        }
        for attempt in 0..2 {
            if self.session.is_none() {
                if let Err(e) = self.create_session() {
                    return self.fail(format!("Video-Decoder lässt sich nicht starten ({e})"));
                }
            }
            self.need_key = false;
            match self.submit(chunk) {
                Ok(true) => break,
                // The system took the session away: build a new one (this sample is a keyframe on the second try
                // only if it is one; otherwise wait for the next keyframe).
                Ok(false) => {
                    self.session = None;
                    self.held.clear();
                    self.last_end = None;
                    if !chunk.key || attempt == 1 {
                        self.need_key = true;
                        return;
                    }
                }
                Err(e) => return self.fail(format!("Das Video lässt sich nicht dekodieren ({e})")),
            }
        }
        self.collect();
        self.release(false);
    }

    /// Takes the pictures the callback produced into the reorder buffer; reports callback failures.
    fn collect(&mut self) {
        let Some(session) = &self.session else { return };
        let ctx = unsafe { &*session.ctx };
        let frames = std::mem::take(&mut *ctx.out.lock().unwrap_or_else(|e| e.into_inner()));
        let failure = ctx.failure.lock().unwrap_or_else(|e| e.into_inner()).take();
        for f in frames {
            self.held.insert(f.pts, f);
        }
        match failure {
            Some(kVTInvalidSessionErr) => {
                self.session = None;
                self.need_key = true;
            }
            Some(status) => self.fail(format!("Das Video lässt sich nicht dekodieren ({})", describe(status))),
            None => {}
        }
    }

    fn release(&mut self, all: bool) {
        loop {
            let Some((&pts, frame)) = self.held.iter().next() else { break };
            let tol = (frame.dur / 4).max(2);
            let contiguous = self.last_end.is_some_and(|end| pts <= end + tol);
            if !(all || contiguous || self.held.len() > self.depth) {
                break;
            }
            let frame = self.held.remove(&pts).expect("just seen");
            self.last_end = Some(frame.pts + frame.dur);
            let picture = Decoded { width: frame.width, height: frame.height, rgba: frame.rgba };
            self.shared.mailbox.lock().unwrap_or_else(|e| e.into_inner()).push_back((self.serial, frame.pts, picture));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::movie::{video_start_for, Movie};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    type Got = Rc<RefCell<Vec<(i64, Decoded)>>>;

    fn decoder(movie: &Movie) -> (AppleVideoDecoder, Got) {
        let got: Got = Rc::new(RefCell::new(Vec::new()));
        let sink = got.clone();
        let config = VideoConfig {
            codec: movie.video.codec.clone(),
            description: movie.video.description.clone(),
            width: movie.video.width,
            height: movie.video.height,
        };
        (AppleVideoDecoder::create(config, move |ts, p| sink.borrow_mut().push((ts, p))).expect("decoder"), got)
    }

    /// Feeds samples `from..` the way the player does (a few queued at a time), flushes and waits until all
    /// pictures are out.
    fn run(movie: &mut Movie, dec: &AppleVideoDecoder, from: usize) {
        let id = movie.video.track_id;
        let n = movie.video.samples.len();
        for i in from..n {
            while dec.queue_len() >= 6 {
                std::thread::sleep(Duration::from_millis(1));
            }
            dec.decode(&movie.read(id, i).expect("sample"));
        }
        dec.flush();
        let t = Instant::now();
        while dec.queue_len() > 0 {
            assert!(t.elapsed() < Duration::from_secs(20), "decoder did not finish; error: {:?}", dec.error());
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn check_pictures(got: &Got, expect_pts: &[i64], w: u32, h: u32) {
        let got = got.borrow();
        let pts: Vec<i64> = got.iter().map(|(t, _)| *t).collect();
        assert_eq!(pts.len(), expect_pts.len(), "frame count; got {pts:?}");
        assert!(pts.windows(2).all(|w| w[0] < w[1]), "pts not strictly increasing: {pts:?}");
        assert_eq!(pts, expect_pts);
        for (_, p) in got.iter() {
            assert_eq!((p.width, p.height), (w, h));
            assert_eq!(p.rgba.len(), (w * h * 4) as usize);
        }
        let lum: Vec<u64> = got.iter().map(|(_, p)| p.rgba.chunks_exact(4).map(|px| px[0] as u64 + px[1] as u64 + px[2] as u64).sum::<u64>()).collect();
        assert!(lum.iter().all(|&l| l > 0), "black picture");
        assert!(got.iter().all(|(_, p)| p.rgba.chunks_exact(4).all(|px| px[3] == 255)));
    }

    fn sorted_pts(movie: &Movie, from: usize) -> Vec<i64> {
        let mut v: Vec<i64> = movie.video.samples[from..].iter().map(|s| s.pts_us).collect();
        v.sort();
        v
    }

    #[test]
    fn decodes_tiny_in_presentation_order() {
        let mut movie = Movie::open(include_bytes!("../../logic/testdata/tiny.mp4").to_vec()).unwrap();
        let (dec, got) = decoder(&movie);
        run(&mut movie, &dec, 0);
        assert_eq!(dec.error(), None);
        let expect = sorted_pts(&movie, 0);
        check_pictures(&got, &expect, 64, 48);
    }

    #[test]
    fn decodes_tail_moov_file() {
        let mut movie = Movie::open(include_bytes!("../../logic/testdata/tail.mp4").to_vec()).unwrap();
        let (dec, got) = decoder(&movie);
        run(&mut movie, &dec, 0);
        let expect = sorted_pts(&movie, 0);
        check_pictures(&got, &expect, 64, 48);
    }

    #[test]
    fn reset_then_decode_from_a_keyframe() {
        let mut movie = Movie::open(include_bytes!("../../logic/testdata/tiny.mp4").to_vec()).unwrap();
        let (dec, got) = decoder(&movie);
        let id = movie.video.track_id;
        // Some samples, then an abrupt seek: whatever was queued must not show up afterwards.
        for i in 0..8 {
            dec.decode(&movie.read(id, i).unwrap());
        }
        dec.reset();
        got.borrow_mut().clear();
        let start = video_start_for(&movie.video.samples, 1_000_000);
        assert!(start > 0);
        run(&mut movie, &dec, start);
        assert_eq!(dec.error(), None);
        let expect = sorted_pts(&movie, start);
        check_pictures(&got, &expect, 64, 48);
        // And twice more, to see that a second reset works after a completed run.
        dec.reset();
        got.borrow_mut().clear();
        run(&mut movie, &dec, 0);
        check_pictures(&got, &sorted_pts(&movie, 0), 64, 48);
    }

    #[test]
    fn garbage_is_reported_not_fatal_to_the_process() {
        let movie = Movie::open(include_bytes!("../../logic/testdata/tiny.mp4").to_vec()).unwrap();
        let (dec, _got) = decoder(&movie);
        dec.decode(&Chunk { key: true, ts_us: 0, dur_us: 33_000, data: vec![0, 0, 0, 5, 1, 2, 3, 4, 5, 6, 7] });
        dec.flush();
        let t = Instant::now();
        while dec.queue_len() > 0 && t.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(2));
        }
        eprintln!("garbage error: {:?}", dec.error());
    }

    #[test]
    fn bad_avcc_is_an_error() {
        let movie = Movie::open(include_bytes!("../../logic/testdata/tiny.mp4").to_vec()).unwrap();
        let config = VideoConfig { codec: "avc1".into(), description: vec![1, 2], width: 64, height: 48 };
        let dec = AppleVideoDecoder::create(config, |_, _| {}).unwrap();
        dec.decode(&Chunk { key: true, ts_us: 0, dur_us: 1, data: vec![0; 8] });
        let t = Instant::now();
        while dec.error().is_none() && t.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(dec.error().is_some());
        drop(movie);
    }

    #[test]
    fn reads_reorder_depth_from_the_sps() {
        let movie = Movie::open(include_bytes!("../../logic/testdata/tiny.mp4").to_vec()).unwrap();
        let params = parse_avcc(&movie.video.description).unwrap();
        // x264 writes bitstream_restriction: 2 reorder frames for this clip (ffprobe: has_b_frames=2).
        assert_eq!(reorder_depth(&params.sps[0]), Some(2));
    }

    #[test]
    fn scales_big_pictures_down() {
        let (w, h) = (2560usize, 1440usize);
        let src: Vec<u8> = (0..w * h * 4).map(|i| if i % 4 == 3 { 255 } else { 90 }).collect();
        let (dw, dh, rgba) = scale_bgra_to_rgba(&src, w * 4, w, h);
        assert_eq!((dw, dh), (1280, 720));
        assert_eq!(rgba.len(), 1280 * 720 * 4);
        assert!(rgba.chunks_exact(4).all(|p| p == [90, 90, 90, 255]));
    }

    /// Long clip made by hand (`IMMAGINE_TEST_CLIP=/path/clip.mp4 cargo test --lib long_clip -- --nocapture`).
    #[test]
    fn long_clip_from_env() {
        let Ok(path) = std::env::var("IMMAGINE_TEST_CLIP") else { return };
        let mut movie = Movie::open(std::fs::read(path).unwrap()).unwrap();
        let (dec, got) = decoder(&movie);
        let t = Instant::now();
        run(&mut movie, &dec, 0);
        eprintln!("{} frames in {:?}, error {:?}", got.borrow().len(), t.elapsed(), dec.error());
        let (w, h) = (movie.video.width as usize, movie.video.height as usize);
        let k = (MAX_SIDE as f64 / w.max(h) as f64).min(1.);
        let (ew, eh) = (((w as f64 * k).round()) as u32, ((h as f64 * k).round()) as u32);
        check_pictures(&got, &sorted_pts(&movie, 0), ew, eh);
    }
}
