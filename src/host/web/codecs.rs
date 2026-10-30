//! Video and audio decoding for the app's own player: the browser's WebCodecs `VideoDecoder`/`AudioDecoder`
//! (hardware accelerated, off the main thread) and a WebAudio `AudioContext` as speaker and playback clock.
//! There is no `<video>` element: Rust demuxes the MP4, feeds encoded samples in here, takes RGBA pictures out
//! and GPUI paints them.
//!
//! WebCodecs is not in `web-sys`' stable API, so the few classes needed are declared by hand below.

use crate::host::{AudioConfig, AudioOutput, Chunk, Decoded, VideoConfig, VideoDecode};
use js_sys::{Float32Array, Object, Reflect, Uint8Array};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{AudioBuffer, AudioBufferSourceNode, AudioContext, OffscreenCanvas, OffscreenCanvasRenderingContext2d};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = VideoDecoder)]
    type JsVideoDecoder;
    #[wasm_bindgen(constructor, catch, js_class = "VideoDecoder")]
    fn new(init: &Object) -> Result<JsVideoDecoder, JsValue>;
    #[wasm_bindgen(method, catch)]
    fn configure(this: &JsVideoDecoder, config: &Object) -> Result<(), JsValue>;
    #[wasm_bindgen(method, catch)]
    fn decode(this: &JsVideoDecoder, chunk: &JsValue) -> Result<(), JsValue>;
    #[wasm_bindgen(method, catch)]
    fn reset(this: &JsVideoDecoder) -> Result<(), JsValue>;
    #[wasm_bindgen(method)]
    fn flush(this: &JsVideoDecoder) -> js_sys::Promise;
    #[wasm_bindgen(method)]
    fn close(this: &JsVideoDecoder);
    #[wasm_bindgen(method, getter, js_name = decodeQueueSize)]
    fn decode_queue_size(this: &JsVideoDecoder) -> u32;

    #[wasm_bindgen(js_name = VideoFrame)]
    type JsVideoFrame;
    #[wasm_bindgen(method, getter, js_name = displayWidth)]
    fn display_width(this: &JsVideoFrame) -> u32;
    #[wasm_bindgen(method, getter, js_name = displayHeight)]
    fn display_height(this: &JsVideoFrame) -> u32;
    #[wasm_bindgen(method, getter)]
    fn timestamp(this: &JsVideoFrame) -> f64;
    #[wasm_bindgen(method, js_name = close)]
    fn close_frame(this: &JsVideoFrame);

    /// A 2d context, for `drawImage` with a `VideoFrame` (not among `web-sys`' typed overloads).
    type Ctx2d;
    #[wasm_bindgen(method, catch, js_name = drawImage)]
    fn draw_frame(this: &Ctx2d, frame: &JsVideoFrame, x: f64, y: f64, w: f64, h: f64) -> Result<(), JsValue>;

    #[wasm_bindgen(js_name = AudioDecoder)]
    type JsAudioDecoder;
    #[wasm_bindgen(constructor, catch, js_class = "AudioDecoder")]
    fn new_audio(init: &Object) -> Result<JsAudioDecoder, JsValue>;
    #[wasm_bindgen(method, catch, js_name = configure)]
    fn configure_audio(this: &JsAudioDecoder, config: &Object) -> Result<(), JsValue>;
    #[wasm_bindgen(method, catch, js_name = decode)]
    fn decode_audio(this: &JsAudioDecoder, chunk: &JsValue) -> Result<(), JsValue>;
    #[wasm_bindgen(method, catch, js_name = reset)]
    fn reset_audio(this: &JsAudioDecoder) -> Result<(), JsValue>;
    #[wasm_bindgen(method, js_name = close)]
    fn close_audio(this: &JsAudioDecoder);
    #[wasm_bindgen(method, getter, js_name = decodeQueueSize)]
    fn audio_queue_size(this: &JsAudioDecoder) -> u32;

    /// `AudioBuffer`, for the live `getChannelData` view (`web-sys` returns a copy).
    type JsAudioBuffer;
    #[wasm_bindgen(method, catch, js_name = getChannelData)]
    fn channel_view(this: &JsAudioBuffer, channel: u32) -> Result<Float32Array, JsValue>;

    #[wasm_bindgen(js_name = AudioData)]
    type JsAudioData;
    #[wasm_bindgen(method, getter, js_name = numberOfFrames)]
    fn number_of_frames(this: &JsAudioData) -> u32;
    #[wasm_bindgen(method, getter, js_name = numberOfChannels)]
    fn number_of_channels(this: &JsAudioData) -> u32;
    #[wasm_bindgen(method, getter, js_name = sampleRate)]
    fn sample_rate(this: &JsAudioData) -> f32;
    #[wasm_bindgen(method, getter, js_name = timestamp)]
    fn audio_timestamp(this: &JsAudioData) -> f64;
    #[wasm_bindgen(method, catch, js_name = copyTo)]
    fn copy_to(this: &JsAudioData, dest: &Float32Array, options: &Object) -> Result<(), JsValue>;
    #[wasm_bindgen(method, js_name = close)]
    fn close_data(this: &JsAudioData);
}

fn object(fields: &[(&str, JsValue)]) -> Object {
    let o = Object::new();
    for (k, v) in fields {
        let _ = Reflect::set(&o, &JsValue::from_str(k), v);
    }
    o
}

/// `new EncodedVideoChunk({type, timestamp, duration, data})` (or `EncodedAudioChunk`, named by `class`).
fn encoded_chunk(class: &str, chunk: &Chunk) -> Result<JsValue, JsValue> {
    let init = object(&[
        ("type", if chunk.key { "key" } else { "delta" }.into()),
        ("timestamp", (chunk.ts_us as f64).into()),
        ("duration", (chunk.dur_us as f64).into()),
        // Copied out of wasm memory: WebCodecs rejects views of shared (SharedArrayBuffer) memory.
        ("data", Uint8Array::from(chunk.data.as_slice()).into()),
    ]);
    let ctor: js_sys::Function = Reflect::get(&js_sys::global(), &JsValue::from_str(class))?.dyn_into()?;
    Reflect::construct(&ctor, &js_sys::Array::of1(&init))
}

fn describe(e: &JsValue) -> String {
    e.as_string().or_else(|| Reflect::get(e, &"message".into()).ok().and_then(|m| m.as_string())).unwrap_or_else(|| format!("{e:?}"))
}

/// Longest side of the pictures handed to GPUI: more pixels than the screen shows only cost copy time.
const MAX_SIDE: f64 = 1280.;

pub struct WebVideoDecoder {
    decoder: JsVideoDecoder,
    config: VideoConfig,
    error: Rc<RefCell<Option<String>>>,
    _output: Closure<dyn FnMut(JsVideoFrame)>,
    _failed: Closure<dyn FnMut(JsValue)>,
}

impl WebVideoDecoder {
    pub fn new(config: VideoConfig, on_frame: impl Fn(i64, Decoded) + 'static) -> Option<Self> {
        // Reused canvas; `willReadFrequently` keeps it in CPU memory so `getImageData` does not stall on the GPU.
        let scratch: RefCell<Option<(OffscreenCanvas, OffscreenCanvasRenderingContext2d)>> = RefCell::new(None);
        let output = Closure::<dyn FnMut(JsVideoFrame)>::new(move |frame: JsVideoFrame| {
            let (fw, fh) = (frame.display_width().max(1), frame.display_height().max(1));
            let scale = (MAX_SIDE / fw.max(fh) as f64).min(1.);
            let (w, h) = (((fw as f64 * scale).round() as u32).max(1), ((fh as f64 * scale).round() as u32).max(1));
            let mut scratch = scratch.borrow_mut();
            if scratch.as_ref().is_none_or(|(c, _)| c.width() != w || c.height() != h) {
                *scratch = OffscreenCanvas::new(w, h).ok().and_then(|canvas| {
                    let options = object(&[("willReadFrequently", true.into())]);
                    let context = canvas.get_context_with_context_options("2d", &options).ok()??.dyn_into().ok()?;
                    Some((canvas, context))
                });
            }
            let ts = frame.timestamp() as i64;
            if let Some((_, context)) = scratch.as_ref() {
                let drawn = context.unchecked_ref::<Ctx2d>().draw_frame(&frame, 0., 0., w as f64, h as f64).is_ok();
                if drawn {
                    if let Ok(data) = context.get_image_data(0., 0., w as f64, h as f64) {
                        on_frame(ts, Decoded { width: w, height: h, rgba: data.data().0 });
                    }
                }
            }
            frame.close_frame();
        });
        let error = Rc::new(RefCell::new(None));
        let failed = {
            let error = error.clone();
            Closure::<dyn FnMut(JsValue)>::new(move |e: JsValue| *error.borrow_mut() = Some(describe(&e)))
        };
        let init = object(&[("output", output.as_ref().clone()), ("error", failed.as_ref().clone())]);
        let decoder = JsVideoDecoder::new(&init).ok()?;
        let this = Self { decoder, config, error, _output: output, _failed: failed };
        this.configure();
        Some(this)
    }

    fn configure(&self) {
        let c = &self.config;
        let options = object(&[
            ("codec", c.codec.as_str().into()),
            ("codedWidth", c.width.into()),
            ("codedHeight", c.height.into()),
            ("description", Uint8Array::from(c.description.as_slice()).into()),
            ("hardwareAcceleration", "no-preference".into()),
            ("optimizeForLatency", true.into()),
        ]);
        if let Err(e) = self.decoder.configure(&options) {
            *self.error.borrow_mut() = Some(describe(&e));
        }
    }
}

impl VideoDecode for WebVideoDecoder {
    fn decode(&self, chunk: &Chunk) {
        match encoded_chunk("EncodedVideoChunk", chunk).and_then(|c| self.decoder.decode(&c)) {
            Ok(()) => {}
            Err(e) => *self.error.borrow_mut() = Some(describe(&e)),
        }
    }

    fn queue_len(&self) -> u32 {
        self.decoder.decode_queue_size()
    }

    fn reset(&self) {
        let _ = self.decoder.reset();
        self.configure();
    }

    fn flush(&self) {
        // The promise rejects when a seek resets the decoder meanwhile; that is expected.
        let promise = self.decoder.flush();
        wasm_bindgen_futures::spawn_local(async move {
            let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
        });
    }

    fn error(&self) -> Option<String> {
        self.error.borrow().clone()
    }
}

impl Drop for WebVideoDecoder {
    fn drop(&mut self) {
        self.decoder.close();
    }
}

/// Speaker state shared with the decoder's output callback.
struct Speaker {
    context: AudioContext,
    /// Context time and media position (µs) that coincide: the playback clock.
    anchor: Option<(f64, i64)>,
    /// Decoded audio that arrived before the clock started.
    stash: Vec<(i64, AudioBuffer)>,
    /// Scheduled sources with the context time they end at.
    playing: Vec<(f64, AudioBufferSourceNode)>,
}

impl Speaker {
    fn schedule(&mut self, ts_us: i64, buffer: &AudioBuffer) {
        let Some((anchor_t, anchor_us)) = self.anchor else {
            self.stash.push((ts_us, buffer.clone()));
            return;
        };
        let now = self.context.current_time();
        let when = anchor_t + (ts_us - anchor_us) as f64 / 1e6;
        let end = when + buffer.duration();
        if end <= now {
            return;
        }
        let (start, offset) = if when < now { (now, now - when) } else { (when, 0.) };
        let Ok(source) = self.context.create_buffer_source() else { return };
        source.set_buffer(Some(buffer));
        if source.connect_with_audio_node(&self.context.destination()).is_err() {
            return;
        }
        let _ = source.start_with_when_and_grain_offset(start, offset);
        self.playing.retain(|(done, _)| *done > now);
        self.playing.push((end, source));
    }
}

/// Audio held back for `decodeAudioData` (browsers without WebCodecs `AudioDecoder`, Safari before 26): AAC frames
/// wrapped in ADTS headers, decoded in blocks of about `BLOCK_US`.
#[derive(Default)]
struct Block {
    bytes: Vec<u8>,
    first_us: i64,
    dur_us: i64,
}

const BLOCK_US: i64 = 500_000;

/// The 7-byte ADTS header for an AAC frame of `len` bytes, from the 2-byte AudioSpecificConfig.
fn adts_header(asc: &[u8], len: usize) -> [u8; 7] {
    let (a, b) = (asc.first().copied().unwrap_or(0x12), asc.get(1).copied().unwrap_or(0x10));
    let (profile, freq, chan) = ((a >> 3).saturating_sub(1) & 3, ((a & 7) << 1) | (b >> 7), (b >> 3) & 0xF);
    let full = len + 7;
    [0xFF, 0xF1, (profile << 6) | (freq << 2) | (chan >> 2), ((chan & 3) << 6) | ((full >> 11) & 3) as u8, (full >> 3) as u8, (((full & 7) << 5) | 0x1F) as u8, 0xFC]
}

pub struct WebAudioOut {
    speaker: Rc<RefCell<Speaker>>,
    /// No `AudioDecoder` here: decode with `decodeAudioData` instead.
    legacy: std::cell::Cell<bool>,
    block: RefCell<Block>,
    /// Bumped by `reset` so blocks still decoding from before a seek are dropped.
    epoch: Rc<std::cell::Cell<u32>>,
    decoder: RefCell<Option<JsAudioDecoder>>,
    config: RefCell<Option<AudioConfig>>,
    closures: RefCell<Vec<Closure<dyn FnMut(JsAudioData)>>>,
    failed: Closure<dyn FnMut(JsValue)>,
}

impl WebAudioOut {
    pub fn new() -> Option<Self> {
        let context = AudioContext::new().ok()?;
        // iOS mutes WebAudio while the ring/silent switch is on silent, unless the page declares it plays media
        // (`navigator.audioSession`, iOS 16.4+).
        if let Some(session) = web_sys::window()
            .and_then(|w| Reflect::get(&w.navigator(), &"audioSession".into()).ok())
            .filter(|s| s.is_object())
        {
            let _ = Reflect::set(&session, &"type".into(), &"playback".into());
        }
        // Created inside the user's tap: resuming and playing a silent buffer here is what unlocks sound on iOS.
        let _ = context.resume();
        if let (Ok(buffer), Ok(source)) = (context.create_buffer(1, 1, 22050.), context.create_buffer_source()) {
            source.set_buffer(Some(&buffer));
            let _ = source.connect_with_audio_node(&context.destination());
            let _ = source.start();
        }
        Some(Self {
            speaker: Rc::new(RefCell::new(Speaker { context, anchor: None, stash: Vec::new(), playing: Vec::new() })),
            legacy: std::cell::Cell::new(false),
            block: RefCell::new(Block::default()),
            epoch: Rc::new(std::cell::Cell::new(0)),
            decoder: RefCell::new(None),
            config: RefCell::new(None),
            closures: RefCell::new(Vec::new()),
            failed: Closure::new(|e: JsValue| web_sys::console::warn_1(&format!("AudioDecoder: {}", describe(&e)).into())),
        })
    }

    fn make_decoder(&self, config: &AudioConfig) -> Option<JsAudioDecoder> {
        let speaker = self.speaker.clone();
        let output = Closure::<dyn FnMut(JsAudioData)>::new(move |data: JsAudioData| {
            let (frames, channels) = (data.number_of_frames(), data.number_of_channels());
            let ts = data.audio_timestamp() as i64;
            let mut s = speaker.borrow_mut();
            if let Ok(buffer) = s.context.create_buffer(channels, frames, data.sample_rate()) {
                let mut ok = true;
                for c in 0..channels {
                    // `getChannelData` is a live view of the buffer's samples; `copyTo` fills it in place.
                    let Ok(plane) = buffer.unchecked_ref::<JsAudioBuffer>().channel_view(c) else { ok = false; break };
                    let options = object(&[("planeIndex", c.into()), ("format", "f32-planar".into())]);
                    if let Err(e) = data.copy_to(&plane, &options) {
                        web_sys::console::warn_1(&format!("[audio] AudioData.copyTo: {}", describe(&e)).into());
                        ok = false;
                    }
                }
                if ok {
                    s.schedule(ts, &buffer);
                }
            }
            data.close_data();
        });
        let init = object(&[("output", output.as_ref().clone()), ("error", self.failed.as_ref().clone())]);
        let decoder = JsAudioDecoder::new_audio(&init).ok()?;
        self.closures.borrow_mut().push(output);
        let options = object(&[
            ("codec", config.codec.as_str().into()),
            ("sampleRate", config.sample_rate.into()),
            ("numberOfChannels", config.channels.into()),
            ("description", Uint8Array::from(config.description.as_slice()).into()),
        ]);
        decoder.configure_audio(&options).ok()?;
        Some(decoder)
    }
}

impl AudioOutput for WebAudioOut {
    fn configure(&self, config: AudioConfig) -> bool {
        if !Reflect::has(&js_sys::global(), &"AudioDecoder".into()).unwrap_or(false) {
            self.legacy.set(true);
            *self.config.borrow_mut() = Some(config);
            return true;
        }
        let decoder = self.make_decoder(&config);
        let ok = decoder.is_some();
        *self.decoder.borrow_mut() = decoder;
        *self.config.borrow_mut() = Some(config);
        ok
    }

    fn decode(&self, chunk: &Chunk) {
        if self.legacy.get() {
            let full = {
                let config = self.config.borrow();
                let mut block = self.block.borrow_mut();
                if block.bytes.is_empty() {
                    block.first_us = chunk.ts_us;
                }
                let asc = config.as_ref().map_or(&[][..], |c| c.description.as_slice());
                block.bytes.extend_from_slice(&adts_header(asc, chunk.data.len()));
                block.bytes.extend_from_slice(&chunk.data);
                block.dur_us += chunk.dur_us;
                block.dur_us >= BLOCK_US
            };
            if full {
                self.flush();
            }
            return;
        }
        if let (Some(d), Ok(c)) = (self.decoder.borrow().as_ref(), encoded_chunk("EncodedAudioChunk", chunk)) {
            let _ = d.decode_audio(&c);
        }
    }

    fn queue_len(&self) -> u32 {
        self.decoder.borrow().as_ref().map_or(0, |d| d.audio_queue_size())
    }

    fn flush(&self) {
        if !self.legacy.get() {
            return;
        }
        let block = std::mem::take(&mut *self.block.borrow_mut());
        if block.bytes.is_empty() {
            return;
        }
        let array = Uint8Array::from(block.bytes.as_slice());
        let context = self.speaker.borrow().context.clone();
        let Ok(promise) = context.decode_audio_data(&array.buffer()) else { return };
        let (speaker, epoch, expected) = (self.speaker.clone(), self.epoch.clone(), self.epoch.get());
        wasm_bindgen_futures::spawn_local(async move {
            match wasm_bindgen_futures::JsFuture::from(promise).await {
                Ok(buffer) if epoch.get() == expected => {
                    if let Ok(buffer) = buffer.dyn_into::<AudioBuffer>() {
                        speaker.borrow_mut().schedule(block.first_us, &buffer);
                    }
                }
                Ok(_) => {}
                Err(e) => web_sys::console::warn_1(&format!("decodeAudioData: {}", describe(&e)).into()),
            }
        });
    }

    fn reset(&self) {
        self.epoch.set(self.epoch.get().wrapping_add(1));
        *self.block.borrow_mut() = Block::default();
        {
            let mut s = self.speaker.borrow_mut();
            for (_, source) in s.playing.drain(..) {
                let _ = source.stop();
            }
            s.stash.clear();
            s.anchor = None;
        }
        // A reset decoder is unconfigured, and output queued before the reset must not survive: start a new one.
        if let Some(old) = self.decoder.borrow_mut().take() {
            old.close_audio();
        }
        self.closures.borrow_mut().clear();
        let config = self.config.borrow_mut().take();
        if let Some(config) = config {
            self.configure(config);
        }
    }

    fn start_clock(&self, media_us: i64) {
        let mut s = self.speaker.borrow_mut();
        let now = s.context.current_time();
        web_sys::console::log_1(&format!("[audio] clock start at {media_us} us, context {:?}, {} Hz", s.context.state(), s.context.sample_rate()).into());
        let _ = s.context.resume();
        s.anchor = Some((now, media_us));
        let stash = std::mem::take(&mut s.stash);
        for (ts, buffer) in stash {
            s.schedule(ts, &buffer);
        }
    }

    fn clock_us(&self) -> Option<i64> {
        let s = self.speaker.borrow();
        s.anchor.map(|(t, us)| us + ((s.context.current_time() - t) * 1e6) as i64)
    }

    fn set_paused(&self, paused: bool) {
        let s = self.speaker.borrow();
        let _ = if paused { s.context.suspend() } else { s.context.resume() };
    }
}

impl Drop for WebAudioOut {
    fn drop(&mut self) {
        let mut s = self.speaker.borrow_mut();
        for (_, source) in s.playing.drain(..) {
            let _ = source.stop();
        }
        if let Some(d) = self.decoder.borrow_mut().take() {
            d.close_audio();
        }
        let _ = s.context.close();
    }
}
