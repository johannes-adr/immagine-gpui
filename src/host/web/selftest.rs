//! Browser smoke test for the adapters against a real Immich server (called from the console:
//! `await wasm.selftest(email, password)`). Credentials are never stored.

use super::{fetch::FetchTransport, idb};
use crate::logic::client::ImmichClient;
use crate::logic::store::{MemoryStore, Store};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub async fn selftest(email: String, password: String) -> Result<String, JsValue> {
    let fail = |e: String| JsValue::from_str(&e);
    let origin = web_sys::window().ok_or_else(|| fail("no window".into()))?.location().origin()?;
    let client = ImmichClient::new(&origin, FetchTransport).map_err(|e| fail(e.to_string()))?;

    let auth = client.login(&email, &password).await.map_err(|e| fail(e.to_string()))?;
    let token = auth.access_token.ok_or_else(|| fail("login returned no access token".into()))?;
    let page = client.search_assets(1, 100, &token).await.map_err(|e| fail(e.to_string()))?;
    let mut store = MemoryStore::new();
    store.upsert_assets(&page.assets.items);

    let bytes = store.snapshot().map_err(|e| fail(e.to_string()))?;
    idb::put("selftest", "store", &bytes).await.map_err(fail)?;
    let back = idb::get("selftest", "store").await.map_err(fail)?.ok_or_else(|| fail("snapshot missing".into()))?;
    let restored = MemoryStore::restore(&back).map_err(|e| fail(e.to_string()))?;

    Ok(format!(
        "server total={}, page={}, stored={}, snapshot={} bytes, restored={}",
        page.assets.total,
        page.assets.items.len(),
        store.asset_count(),
        bytes.len(),
        restored.asset_count()
    ))
}

/// Runs the real sync engine twice (full sync, then the incremental stream) against the server.
#[wasm_bindgen]
pub async fn sync_selftest(email: String, password: String) -> Result<String, JsValue> {
    use crate::logic::sync::engine::SyncEngine;
    let fail = |e: String| JsValue::from_str(&e);
    let origin = web_sys::window().ok_or_else(|| fail("no window".into()))?.location().origin()?;
    let login = ImmichClient::new(&origin, FetchTransport).map_err(|e| fail(e.to_string()))?;
    let token = login
        .login(&email, &password)
        .await
        .map_err(|e| fail(e.to_string()))?
        .access_token
        .ok_or_else(|| fail("no access token".into()))?;

    let mut engine = SyncEngine::new(MemoryStore::new(), FetchTransport, super::WebClock, &origin).map_err(|e| fail(e.to_string()))?;
    let t0 = js_sys::Date::now();
    let first = engine.sync(&token).await.map_err(|e| fail(format!("first sync: {e}")))?;
    let t1 = js_sys::Date::now();
    let second = engine.sync(&token).await.map_err(|e| fail(format!("second sync: {e}")))?;
    let t2 = js_sys::Date::now();
    Ok(format!(
        "first: full={} assets={} albums={} {}ms | second: full={} summary={:?} {}ms",
        first.full_sync,
        engine.store().asset_count(),
        engine.store().albums().len(),
        (t1 - t0) as i64,
        second.full_sync,
        second.stream.summary(),
        (t2 - t1) as i64,
    ))
}

/// Plays `url` (an MP4 reachable from the page) through the app's own demuxer and WebCodecs decoders without any
/// UI and reports how many pictures and audio blocks came out: `await wasm.video_selftest('/testdata/x.mp4')`.
#[wasm_bindgen]
pub async fn video_selftest(url: String) -> Result<String, JsValue> {
    use super::codecs::{WebAudioOut, WebVideoDecoder};
    use crate::host::{AudioConfig, AudioOutput, VideoConfig, VideoDecode};
    use crate::logic::movie::Movie;
    use std::cell::RefCell;
    use std::rc::Rc;
    let fail = |e: String| JsValue::from_str(&e);
    let bytes = super::fetch::bytes(&url).await.map_err(fail)?;
    let size = bytes.len();
    let mut movie = Movie::open(bytes).map_err(fail)?;
    let frames: Rc<RefCell<Vec<(i64, u32, u32, u64)>>> = Rc::default();
    let sink = frames.clone();
    let config = VideoConfig { codec: movie.video.codec.clone(), description: movie.video.description.clone(), width: movie.video.width, height: movie.video.height };
    let decoder = WebVideoDecoder::new(config, move |ts, d| {
        let sum: u64 = d.rgba.iter().step_by(97).map(|b| *b as u64).sum();
        sink.borrow_mut().push((ts, d.width, d.height, sum));
    })
    .ok_or_else(|| fail("no VideoDecoder".into()))?;
    let id = movie.video.track_id;
    let count = movie.video.samples.len();
    for i in 0..count {
        let chunk = movie.read(id, i).ok_or_else(|| fail(format!("sample {i}")))?;
        decoder.decode(&chunk);
    }
    decoder.flush();
    let audio = WebAudioOut::new();
    let mut audio_ok = None;
    if let (Some(out), Some(track)) = (audio.as_ref(), movie.audio.as_ref()) {
        let (aid, n) = (track.track_id, track.samples.len());
        audio_ok = Some(out.configure(AudioConfig { codec: track.codec.clone(), description: track.description.clone(), sample_rate: track.sample_rate, channels: track.channels }));
        for i in 0..n {
            if let Some(chunk) = movie.read(aid, i) {
                out.decode(&chunk);
            }
        }
        out.start_clock(0);
    }
    let started = js_sys::Date::now();
    while frames.borrow().len() < count && js_sys::Date::now() - started < 8000. {
        wasm_bindgen_futures::JsFuture::from(js_sys::Promise::new(&mut |resolve, _| {
            let _ = web_sys::window().unwrap().set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 50);
        }))
        .await?;
    }
    let f = frames.borrow();
    let first = f.first().copied();
    let ordered = f.windows(2).all(|w| w[0].0 <= w[1].0);
    let distinct = f.iter().map(|x| x.3).collect::<std::collections::HashSet<_>>().len();
    Ok(format!(
        "{size} bytes, {}x{} {} desc={}B, samples={count}, frames out={} ordered={ordered} distinct={distinct} first={first:?}, audio configured={audio_ok:?} clock={:?} err={:?}",
        movie.video.width,
        movie.video.height,
        movie.video.codec,
        movie.video.description.len(),
        f.len(),
        audio.as_ref().and_then(|a| a.clock_us()),
        decoder.error()
    ))
}

/// Like `video_selftest`, but through the streaming path: probes the head with a range request, parses only
/// `ftyp` + `moov`, then fetches sample bytes by range on demand (first the start, then a seek into the middle
/// with an empty cache) and decodes them: `await wasm.video_selftest_ranged('/x.mp4')`. Reports requests and bytes.
#[wasm_bindgen]
pub async fn video_selftest_ranged(url: String) -> Result<String, JsValue> {
    use super::codecs::WebVideoDecoder;
    use crate::host::{VideoConfig, VideoDecode};
    use crate::logic::movie::{next_request, video_start_for, ByteCache, Fetched, HeaderScan, Movie, Scan, HEAD_REQUEST};
    use std::cell::RefCell;
    use std::rc::Rc;
    let fail = |e: String| JsValue::from_str(&e);
    let mut log: Vec<String> = Vec::new();
    let mut fetched = 0u64;
    let first = super::fetch::range(&url, 0, HEAD_REQUEST - 1).await.map_err(fail)?;
    log.push(format!("req 0-{} -> partial={} total={:?} got {}", HEAD_REQUEST - 1, first.partial, first.total, first.bytes.len()));
    let (true, Some(total)) = (first.partial, first.total) else { return Err(fail(format!("server did not answer 206: {}", log.join("; ")))) };
    fetched += first.bytes.len() as u64;
    let mut cache = ByteCache::new();
    cache.insert(0, first.bytes);
    let mut scan = HeaderScan::new(total);
    let mut movie = loop {
        match scan.step(&cache).map_err(fail)? {
            Scan::Fetch { start, end } => {
                let r = super::fetch::range(&url, start, end - 1).await.map_err(fail)?;
                log.push(format!("req {start}-{} -> partial={} got {}", end - 1, r.partial, r.bytes.len()));
                fetched += r.bytes.len() as u64;
                cache.insert(start, r.bytes);
            }
            Scan::Done { ftyp, moov } => {
                break Movie::open_header(&cache.read(ftyp.0, ftyp.1).unwrap(), &cache.read(moov.0, moov.1).unwrap()).map_err(fail)?;
            }
        }
    };
    let header_requests = log.len();
    let count = movie.video.samples.len();
    let id = movie.video.track_id;
    let frames: Rc<RefCell<Vec<i64>>> = Rc::default();
    let sink = frames.clone();
    let config = VideoConfig { codec: movie.video.codec.clone(), description: movie.video.description.clone(), width: movie.video.width, height: movie.video.height };
    let decoder = WebVideoDecoder::new(config, move |ts, _| sink.borrow_mut().push(ts)).ok_or_else(|| fail("no VideoDecoder".into()))?;

    // Pass 1: from the start, in read-ahead sized steps.
    let mut next = 0usize;
    let mut sample_misses = 0;
    while next < count {
        match movie.next_chunk(id, next, &cache) {
            Fetched::Ready(c) => {
                decoder.decode(&c);
                next += 1;
            }
            Fetched::Missing => {
                sample_misses += 1;
                let wanted = movie.wanted(next, usize::MAX, i64::MAX, 4 * 1024 * 1024);
                let (s, e) = next_request(&wanted, &cache, total).ok_or_else(|| fail(format!("sample {next} missing but nothing to request")))?;
                let r = super::fetch::range(&url, s, e - 1).await.map_err(fail)?;
                log.push(format!("req {s}-{} -> partial={} got {}", e - 1, r.partial, r.bytes.len()));
                fetched += r.bytes.len() as u64;
                cache.insert(s, r.bytes);
            }
            Fetched::End => break,
        }
    }
    decoder.flush();
    let wait = |ms: i32| async move {
        wasm_bindgen_futures::JsFuture::from(js_sys::Promise::new(&mut |resolve, _| {
            let _ = web_sys::window().unwrap().set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }))
        .await
    };
    let started = js_sys::Date::now();
    while frames.borrow().len() < count && js_sys::Date::now() - started < 8000. {
        wait(50).await?;
    }
    let out1 = frames.borrow().len();

    // Pass 2: seek to the middle with an empty cache, decode ~3 s from the keyframe before it.
    let mid = movie.duration_us / 2;
    let start = video_start_for(&movie.video.samples, mid);
    let mut cache2 = ByteCache::new();
    decoder.reset();
    frames.borrow_mut().clear();
    let mut fed = 0usize;
    let mut i = start;
    let mut seek_bytes = 0u64;
    let mut seek_requests = 0;
    while i < count && movie.video.samples[i].pts_us <= mid + 3_000_000 {
        match movie.next_chunk(id, i, &cache2) {
            Fetched::Ready(c) => {
                decoder.decode(&c);
                fed += 1;
                i += 1;
            }
            Fetched::Missing => {
                let wanted = movie.wanted(i, usize::MAX, mid + 3_000_000, 4 * 1024 * 1024);
                let (s, e) = next_request(&wanted, &cache2, total).ok_or_else(|| fail(format!("seek sample {i} missing")))?;
                let r = super::fetch::range(&url, s, e - 1).await.map_err(fail)?;
                seek_requests += 1;
                seek_bytes += r.bytes.len() as u64;
                log.push(format!("seek req {s}-{} -> partial={} got {}", e - 1, r.partial, r.bytes.len()));
                cache2.insert(s, r.bytes);
            }
            Fetched::End => break,
        }
    }
    decoder.flush();
    let started = js_sys::Date::now();
    while frames.borrow().len() < fed && js_sys::Date::now() - started < 8000. {
        wait(50).await?;
    }
    let out2 = frames.borrow().len();
    Ok(format!(
        "file {total} B, {}x{} {}, samples={count}; header requests={header_requests}; pass1: {} samples fed, frames out={out1}/{count}, sample misses={sample_misses}; pass2 (seek to {}us -> keyframe #{start}): requests={seek_requests} bytes={seek_bytes}, fed={fed} frames out={out2}; total fetched in pass1={fetched} B; err={:?}\n{}",
        movie.video.width,
        movie.video.height,
        movie.video.codec,
        next,
        mid,
        decoder.error(),
        log.join("\n")
    ))
}
