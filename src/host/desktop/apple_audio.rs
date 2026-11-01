//! AAC decoding, speaker and playback clock for the app's own video player on macOS and iOS (AudioToolbox).
//!
//! * `AudioConverter` turns each AAC packet into float PCM on the calling (main) thread: one packet is ~1000 frames
//!   and decodes in microseconds, so no worker is needed. The ASC from the MP4 goes in as an `esds` magic cookie.
//! * Output is an output `AudioUnit` (`DefaultOutput` on macOS, `RemoteIO` on iOS) with a render callback that pulls
//!   from a queue of PCM segments stamped with their media time. Main and audio thread talk through a channel only
//!   (no lock is shared); the callback owns the queue.
//! * The clock is the host clock (`mach_absolute_time`), not the sound card: `start_clock(media_us)` anchors
//!   `(host ticks, media_us)`, `clock_us` is the anchor plus elapsed ticks (frozen while paused). It keeps running
//!   when no sound is queued (silent videos, audio ending first). The render callback computes the media time of
//!   its first frame from `AudioTimeStamp::host_time` (the moment the frame leaves the speaker, latency included),
//!   plays on sequentially and only re-syncs when it drifted more than [`RESYNC_US`], so segment boundaries stay
//!   click-free.
//! * Output is always stereo float (mono is duplicated). More than two channels are not downmixed: `configure`
//!   returns `false` and the video plays without sound.
//! * iOS: the `AVAudioSession` category is set to `playback` and activated, so sound also plays with the silent
//!   switch on.

use super::apple_ffi::*;
use crate::host::{AudioConfig, AudioOutput, Chunk};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::mpsc::{channel, Receiver, Sender};

/// The render callback re-syncs to the clock when its own position is off by more than this.
const RESYNC_US: f64 = 30_000.;

enum Cmd {
    /// Interleaved stereo float samples that start at `ts_us`.
    Pcm { ts_us: i64, samples: Vec<f32> },
    /// Playing: `media_us` was at host tick `ticks`.
    Run { ticks: u64, media_us: i64 },
    Pause,
    /// Drop everything queued and stop.
    Reset,
}

struct Segment {
    ts_us: i64,
    samples: Vec<f32>,
}

/// Owned by the audio thread (handed to it as the callback's refcon).
struct Render {
    rx: Receiver<Cmd>,
    queue: VecDeque<Segment>,
    run: Option<(u64, i64)>,
    /// Media time (µs) of the next frame to render.
    pos: Option<f64>,
    sample_rate: f64,
    /// Frames rendered with sound in them (tests read it; they also mute the device).
    #[cfg_attr(not(test), allow(dead_code))]
    audible: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

unsafe extern "C" fn render(
    refcon: *mut c_void,
    _flags: *mut u32,
    time: *const AudioTimeStamp,
    _bus: u32,
    frames: u32,
    io: *mut AudioBufferList,
) -> OSStatus {
    let state = unsafe { &mut *(refcon as *mut Render) };
    let buffer = unsafe { &mut (*io).buffers[0] };
    let out = unsafe { std::slice::from_raw_parts_mut(buffer.data as *mut f32, frames as usize * 2) };
    out.fill(0.);
    while let Ok(cmd) = state.rx.try_recv() {
        match cmd {
            Cmd::Pcm { ts_us, samples } => state.queue.push_back(Segment { ts_us, samples }),
            Cmd::Run { ticks, media_us } => state.run = Some((ticks, media_us)),
            Cmd::Pause => {
                state.run = None;
                state.pos = None;
            }
            Cmd::Reset => {
                state.queue.clear();
                state.run = None;
                state.pos = None;
            }
        }
    }
    let Some((anchor_ticks, anchor_media)) = state.run else { return 0 };
    let now = unsafe {
        let t = &*time;
        if t.flags & kAudioTimeStampHostTimeValid != 0 { t.host_time } else { host_ticks() }
    };
    let expected = anchor_media as f64 + ticks_to_us(now as i64 - anchor_ticks as i64);
    let mut pos = match state.pos {
        Some(p) if (p - expected).abs() < RESYNC_US => p,
        _ => expected,
    };
    let per_frame = 1e6 / state.sample_rate;
    let n = frames as usize;
    let mut j = 0;
    while j < n {
        let Some(seg) = state.queue.front() else { break };
        let seg_frames = (seg.samples.len() / 2) as i64;
        let idx = ((pos - seg.ts_us as f64) / per_frame).round() as i64;
        if idx >= seg_frames {
            state.queue.pop_front();
            continue;
        }
        if idx < 0 {
            // Gap before the segment: silence until it starts.
            let gap = ((-idx) as usize).min(n - j);
            j += gap;
            pos += gap as f64 * per_frame;
            continue;
        }
        let take = ((seg_frames - idx) as usize).min(n - j);
        let from = idx as usize * 2;
        out[j * 2..(j + take) * 2].copy_from_slice(&seg.samples[from..from + take * 2]);
        j += take;
        pos += take as f64 * per_frame;
    }
    pos += (n - j) as f64 * per_frame;
    state.pos = Some(pos);
    #[cfg(test)]
    {
        let loud = out.iter().filter(|s| s.abs() > 1e-4).count() as u64 / 2;
        state.audible.fetch_add(loud, std::sync::atomic::Ordering::Relaxed);
        out.fill(0.);
    }
    0
}

/// Feeds one AAC packet to the converter.
struct PacketSource {
    data: *mut c_void,
    len: u32,
    channels: u32,
    given: bool,
    desc: AudioStreamPacketDescription,
}

/// Returned by the input proc when the single packet was already handed over ("no more input right now").
const NO_MORE_INPUT: OSStatus = 0x6e6f_6d6f;

unsafe extern "C" fn supply_packet(
    _converter: AudioConverterRef,
    packets: *mut u32,
    data: *mut AudioBufferList,
    descriptions: *mut *mut AudioStreamPacketDescription,
    user: *mut c_void,
) -> OSStatus {
    let src = unsafe { &mut *(user as *mut PacketSource) };
    if src.given {
        unsafe { *packets = 0 };
        return NO_MORE_INPUT;
    }
    src.given = true;
    unsafe {
        *packets = 1;
        (*data).number_buffers = 1;
        (*data).buffers[0] = AudioBuffer { number_channels: src.channels, data_byte_size: src.len, data: src.data };
        if !descriptions.is_null() {
            src.desc = AudioStreamPacketDescription { start_offset: 0, variable_frames_in_packet: 0, data_byte_size: src.len };
            *descriptions = &mut src.desc;
        }
    }
    0
}

/// MPEG-4 `ES_Descriptor` ("esds" payload) around an AudioSpecificConfig: the magic cookie format AudioToolbox
/// documents for AAC.
fn esds(asc: &[u8]) -> Vec<u8> {
    // Lengths use the 4-byte form (0x80 0x80 0x80 n), valid for any size below 128.
    let len = |n: usize| [0x80, 0x80, 0x80, n as u8];
    let mut dec_specific = vec![0x05];
    dec_specific.extend(len(asc.len()));
    dec_specific.extend(asc);
    let mut dec_config = vec![0x04];
    dec_config.extend(len(13 + dec_specific.len()));
    dec_config.extend([0x40, 0x15, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    dec_config.extend(&dec_specific);
    let sl = [0x06, 0x80, 0x80, 0x80, 0x01, 0x02];
    let mut es = vec![0x03];
    es.extend(len(3 + dec_config.len() + sl.len()));
    es.extend([0, 0, 0]);
    es.extend(&dec_config);
    es.extend(sl);
    es
}

struct Engine {
    converter: AudioConverterRef,
    unit: AudioUnit,
    tx: Sender<Cmd>,
    /// Kept alive for the callback; freed after the unit stopped.
    render: *mut Render,
    channels: u32,
    #[cfg_attr(not(test), allow(dead_code))]
    audible: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe {
            AudioOutputUnitStop(self.unit);
            AudioUnitUninitialize(self.unit);
            AudioComponentInstanceDispose(self.unit);
            AudioConverterDispose(self.converter);
            drop(Box::from_raw(self.render));
        }
    }
}

#[derive(Default)]
struct ClockState {
    /// Host ticks and media µs that coincide while running.
    anchor: Option<(u64, i64)>,
    /// Media position held while paused.
    frozen: Option<i64>,
    paused: bool,
}

#[derive(Default)]
pub struct AppleAudioOut {
    engine: RefCell<Option<Engine>>,
    clock: RefCell<ClockState>,
    /// Set by `configure`, so `reset` can keep the unit running.
    configured: Cell<bool>,
}

impl AppleAudioOut {
    pub fn create() -> Option<Self> {
        Some(Self::default())
    }

    #[cfg(test)]
    fn audible(&self) -> u64 {
        self.engine.borrow().as_ref().map_or(0, |e| e.audible.load(std::sync::atomic::Ordering::Relaxed))
    }

    fn send(&self, cmd: Cmd) {
        if let Some(e) = self.engine.borrow().as_ref() {
            let _ = e.tx.send(cmd);
        }
    }

    fn now_media(c: &ClockState) -> Option<i64> {
        let (ticks, media) = c.anchor?;
        Some(c.frozen.unwrap_or_else(|| media + ticks_to_us(host_ticks() as i64 - ticks as i64) as i64))
    }

    fn build(config: &AudioConfig) -> Result<Engine, String> {
        let channels = config.channels.clamp(1, 8);
        if channels > 2 {
            return Err(format!("{channels} Kanäle werden nicht unterstützt"));
        }
        let sample_rate = config.sample_rate.max(8000) as f64;
        unsafe {
            let input = AudioStreamBasicDescription {
                sample_rate,
                format_id: kAudioFormatMPEG4AAC,
                frames_per_packet: 1024,
                channels_per_frame: channels,
                ..Default::default()
            };
            let pcm = AudioStreamBasicDescription {
                sample_rate,
                format_id: kAudioFormatLinearPCM,
                format_flags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked,
                bytes_per_packet: 4 * channels,
                frames_per_packet: 1,
                bytes_per_frame: 4 * channels,
                channels_per_frame: channels,
                bits_per_channel: 32,
                reserved: 0,
            };
            let mut converter: AudioConverterRef = std::ptr::null_mut();
            let status = AudioConverterNew(&input, &pcm, &mut converter);
            if status != 0 {
                return Err(format!("AudioConverterNew {status}"));
            }
            let cookie = esds(&config.description);
            let status = AudioConverterSetProperty(converter, kAudioConverterDecompressionMagicCookie, cookie.len() as u32, cookie.as_ptr() as *const c_void);
            if status != 0 {
                AudioConverterDispose(converter);
                return Err(format!("magic cookie {status}"));
            }

            #[cfg(target_os = "ios")]
            activate_session();

            let desc = AudioComponentDescription {
                component_type: kAudioUnitType_Output,
                component_sub_type: kAudioUnitSubType_Output,
                component_manufacturer: kAudioUnitManufacturer_Apple,
                component_flags: 0,
                component_flags_mask: 0,
            };
            let component = AudioComponentFindNext(std::ptr::null_mut(), &desc);
            let mut unit: AudioUnit = std::ptr::null_mut();
            if component.is_null() || AudioComponentInstanceNew(component, &mut unit) != 0 {
                AudioConverterDispose(converter);
                return Err("kein Audioausgang".into());
            }
            let (tx, rx) = channel();
            let audible = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
            let render_state = Box::into_raw(Box::new(Render { rx, queue: VecDeque::new(), run: None, pos: None, sample_rate, audible: audible.clone() }));
            let out_format = AudioStreamBasicDescription {
                sample_rate,
                format_id: kAudioFormatLinearPCM,
                format_flags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked,
                bytes_per_packet: 8,
                frames_per_packet: 1,
                bytes_per_frame: 8,
                channels_per_frame: 2,
                bits_per_channel: 32,
                reserved: 0,
            };
            let callback = AURenderCallbackStruct { input_proc: render, input_proc_ref_con: render_state as *mut c_void };
            let ok = AudioUnitSetProperty(unit, kAudioUnitProperty_StreamFormat, kAudioUnitScope_Input, 0, &out_format as *const _ as *const c_void, size_of::<AudioStreamBasicDescription>() as u32) == 0
                && AudioUnitSetProperty(unit, kAudioUnitProperty_SetRenderCallback, kAudioUnitScope_Input, 0, &callback as *const _ as *const c_void, size_of::<AURenderCallbackStruct>() as u32) == 0
                && AudioUnitInitialize(unit) == 0
                && AudioOutputUnitStart(unit) == 0;
            let engine = Engine { converter, unit, tx, render: render_state, channels, audible };
            if !ok {
                drop(engine);
                return Err("Audioausgang lässt sich nicht starten".into());
            }
            Ok(engine)
        }
    }
}

/// Category `playback`, active: sound is not muted by the silent switch and takes over from other audio.
#[cfg(target_os = "ios")]
unsafe fn activate_session() {
    use std::ffi::c_char;
    #[link(name = "AVFoundation", kind = "framework")]
    unsafe extern "C" {
        static AVAudioSessionCategoryPlayback: *mut c_void;
    }
    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> *mut c_void;
        fn sel_registerName(name: *const c_char) -> *mut c_void;
        fn objc_msgSend();
    }
    unsafe {
        let class = objc_getClass(c"AVAudioSession".as_ptr());
        if class.is_null() {
            return;
        }
        let shared: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let set_category: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *mut *mut c_void) -> bool = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let set_active: unsafe extern "C" fn(*mut c_void, *mut c_void, bool, *mut *mut c_void) -> bool = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let session = shared(class, sel_registerName(c"sharedInstance".as_ptr()));
        if session.is_null() {
            return;
        }
        let mut error: *mut c_void = std::ptr::null_mut();
        set_category(session, sel_registerName(c"setCategory:error:".as_ptr()), AVAudioSessionCategoryPlayback, &mut error);
        set_active(session, sel_registerName(c"setActive:error:".as_ptr()), true, &mut error);
    }
}

impl AudioOutput for AppleAudioOut {
    fn configure(&self, config: AudioConfig) -> bool {
        match Self::build(&config) {
            Ok(engine) => {
                *self.engine.borrow_mut() = Some(engine);
                self.configured.set(true);
                true
            }
            Err(e) => {
                eprintln!("[audio] {e}");
                false
            }
        }
    }

    fn decode(&self, chunk: &Chunk) {
        let engine = self.engine.borrow();
        let Some(e) = engine.as_ref() else { return };
        let ch = e.channels as usize;
        let mut pcm = vec![0f32; 4096 * ch];
        let mut data = chunk.data.clone();
        let mut source = PacketSource {
            data: data.as_mut_ptr() as *mut c_void,
            len: data.len() as u32,
            channels: e.channels,
            given: false,
            desc: AudioStreamPacketDescription { start_offset: 0, variable_frames_in_packet: 0, data_byte_size: 0 },
        };
        let mut list = AudioBufferList {
            number_buffers: 1,
            buffers: [AudioBuffer { number_channels: e.channels, data_byte_size: (pcm.len() * 4) as u32, data: pcm.as_mut_ptr() as *mut c_void }],
        };
        let mut frames = 4096u32;
        let status = unsafe {
            AudioConverterFillComplexBuffer(e.converter, supply_packet, &mut source as *mut _ as *mut c_void, &mut frames, &mut list, std::ptr::null_mut())
        };
        if frames == 0 || (status != 0 && status != NO_MORE_INPUT) {
            if status != 0 && status != NO_MORE_INPUT {
                eprintln!("[audio] decode error {status}");
            }
            return;
        }
        pcm.truncate(frames as usize * ch);
        let samples = if ch == 1 { pcm.iter().flat_map(|&s| [s, s]).collect() } else { pcm };
        let _ = e.tx.send(Cmd::Pcm { ts_us: chunk.ts_us, samples });
    }

    /// Packets are decoded as they arrive, nothing is held back.
    fn queue_len(&self) -> u32 {
        0
    }

    fn reset(&self) {
        if let Some(e) = self.engine.borrow().as_ref() {
            unsafe { AudioConverterReset(e.converter) };
            let _ = e.tx.send(Cmd::Reset);
        }
        let mut c = self.clock.borrow_mut();
        c.anchor = None;
        c.frozen = None;
    }

    /// AudioToolbox returns every packet's output at once, nothing is held back.
    fn flush(&self) {}

    fn start_clock(&self, media_us: i64) {
        let mut c = self.clock.borrow_mut();
        let ticks = host_ticks();
        c.anchor = Some((ticks, media_us));
        if c.paused {
            c.frozen = Some(media_us);
            self.send(Cmd::Pause);
        } else {
            c.frozen = None;
            self.send(Cmd::Run { ticks, media_us });
        }
    }

    fn clock_us(&self) -> Option<i64> {
        Self::now_media(&self.clock.borrow())
    }

    fn set_paused(&self, paused: bool) {
        let mut c = self.clock.borrow_mut();
        if c.paused == paused {
            return;
        }
        c.paused = paused;
        if c.anchor.is_none() {
            return;
        }
        if paused {
            c.frozen = Self::now_media(&c);
            self.send(Cmd::Pause);
        } else if let Some(media_us) = c.frozen.take() {
            let ticks = host_ticks();
            c.anchor = Some((ticks, media_us));
            self.send(Cmd::Run { ticks, media_us });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::movie::Movie;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    fn config(movie: &Movie) -> AudioConfig {
        let a = movie.audio.as_ref().expect("audio track");
        AudioConfig { codec: a.codec.clone(), description: a.description.clone(), sample_rate: a.sample_rate, channels: a.channels }
    }

    #[test]
    fn esds_is_well_formed() {
        let e = esds(&[0x12, 0x08]);
        assert_eq!(e[0], 0x03);
        // ES_Descriptor length covers everything after its own header.
        assert_eq!(e[4] as usize, e.len() - 5);
    }

    #[test]
    fn clock_runs_pauses_and_resets_without_sound() {
        let out = AppleAudioOut::create().unwrap();
        assert_eq!(out.clock_us(), None);
        out.start_clock(5_000_000);
        std::thread::sleep(Duration::from_millis(120));
        let t = out.clock_us().unwrap();
        assert!((5_100_000..5_400_000).contains(&t), "{t}");
        out.set_paused(true);
        let held = out.clock_us().unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(out.clock_us(), Some(held));
        out.set_paused(false);
        std::thread::sleep(Duration::from_millis(100));
        let t2 = out.clock_us().unwrap();
        assert!(t2 >= held + 80_000 && t2 < held + 400_000, "{held} -> {t2}");
        out.reset();
        assert_eq!(out.clock_us(), None);
    }

    #[test]
    fn decodes_and_plays_the_aac_track() {
        let mut movie = Movie::open(include_bytes!("../../logic/testdata/tiny.mp4").to_vec()).unwrap();
        let out = AppleAudioOut::create().unwrap();
        if !out.configure(config(&movie)) {
            eprintln!("no audio output device here: skipping");
            return;
        }
                let id = movie.audio.as_ref().unwrap().track_id;
        let count = movie.audio.as_ref().unwrap().samples.len();
        for i in 0..count {
            out.decode(&movie.read(id, i).unwrap());
        }
        out.flush();
        out.start_clock(0);
        out.set_paused(false);
        std::thread::sleep(Duration::from_millis(600));
        let t = out.clock_us().unwrap();
        assert!((500_000..1_200_000).contains(&t), "{t}");
        let played = out.audible();
        eprintln!("audible frames after 0.6 s: {played}");
        assert!(played > 10_000, "the callback rendered no sound ({played})");
        // Pause holds the clock and the sound.
        out.set_paused(true);
        let held = out.clock_us().unwrap();
        let before = out.audible();
        std::thread::sleep(Duration::from_millis(250));
        assert_eq!(out.clock_us(), Some(held));
        assert!(out.audible() - before < 2_000, "sound continued while paused");
        // Reset forgets the clock and stops sound.
        out.reset();
        assert_eq!(out.clock_us(), None);
        let before = out.audible();
        out.set_paused(false);
        out.start_clock(1_000_000);
        std::thread::sleep(Duration::from_millis(250));
        assert!(out.audible() - before < 2_000, "old sound survived the reset");
        // Seek to the middle: sound from there plays.
        let from = crate::logic::movie::audio_start_for(&movie.audio.as_ref().unwrap().samples, 1_000_000);
        out.reset();
        for i in from..count {
            out.decode(&movie.read(id, i).unwrap());
        }
        let before = out.audible();
        out.start_clock(1_000_000);
        std::thread::sleep(Duration::from_millis(400));
        assert!(out.audible() - before > 5_000, "no sound after the seek");
    }

    #[test]
    fn more_than_two_channels_is_refused() {
        let out = AppleAudioOut::create().unwrap();
        assert!(!out.configure(AudioConfig { codec: "mp4a.40.2".into(), description: vec![0x11, 0x90], sample_rate: 48000, channels: 6 }));
    }
}
