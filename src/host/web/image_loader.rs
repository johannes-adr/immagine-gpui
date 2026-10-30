//! Image loading by the browser: abortable `fetch` -> `createImageBitmap` -> (scaled) pixels. The browser decodes JPEG/WebP
//! natively (and off the main thread), which is far cheaper than decoding in WASM on the UI thread.

use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use crate::host::{Cancel, Decoded};
use web_sys::{AbortController, Blob, ImageBitmap, OffscreenCanvas, OffscreenCanvasRenderingContext2d, RequestInit, RequestCredentials, Response};

fn js(error: JsValue) -> String {
    error.as_string().unwrap_or_else(|| format!("{error:?}"))
}

/// Downloads `url` with the session token as bearer (works cross-origin; same-origin the cookie applies too).
/// `cancel` aborts the request (and the body read) through an `AbortController`.
pub async fn fetch(url: &str, cancel: &Cancel) -> Result<Vec<u8>, String> {
    if cancel.is_cancelled() {
        return Err("cancelled".into());
    }
    let window = web_sys::window().ok_or("no window")?;
    let init = RequestInit::new();
    init.set_credentials(RequestCredentials::SameOrigin);
    if let Some(token) = super::local_storage::get(super::TOKEN_KEY) {
        let headers = web_sys::Headers::new().map_err(js)?;
        headers.set("Authorization", &format!("Bearer {token}")).map_err(js)?;
        init.set_headers(&headers);
    }
    let controller = AbortController::new().map_err(js)?;
    init.set_signal(Some(&controller.signal()));
    cancel.on_cancel(move || controller.abort());
    let request = web_sys::Request::new_with_str_and_init(url, &init).map_err(js)?;
    let response: Response = JsFuture::from(window.fetch_with_request(&request)).await.map_err(js)?.dyn_into().map_err(js)?;
    if !response.ok() {
        return Err(format!("HTTP {}", response.status()));
    }
    let buffer = JsFuture::from(response.array_buffer().map_err(js)?).await.map_err(js)?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

/// Decodes encoded image bytes held in memory (see `Host::decode_bytes` for `min_side`).
pub async fn decode_bytes(bytes: &[u8], min_side: u32) -> Result<Decoded, String> {
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let blob = Blob::new_with_u8_array_sequence(&parts).map_err(js)?;
    decode_blob(&blob, min_side).await
}

async fn decode_blob(blob: &Blob, min_side: u32) -> Result<Decoded, String> {
    let window = web_sys::window().ok_or("no window")?;
    let bitmap: ImageBitmap = JsFuture::from(window.create_image_bitmap_with_blob(blob).map_err(js)?)
        .await
        .map_err(js)?
        .dyn_into()
        .map_err(js)?;
    let (src_w, src_h) = (bitmap.width(), bitmap.height());
    let short = src_w.min(src_h);
    let (width, height) = if min_side > 0 && short > min_side {
        let k = min_side as f64 / short as f64;
        (((src_w as f64 * k).round() as u32).max(1), ((src_h as f64 * k).round() as u32).max(1))
    } else {
        (src_w, src_h)
    };
    let canvas = OffscreenCanvas::new(width, height).map_err(js)?;
    let context: OffscreenCanvasRenderingContext2d =
        canvas.get_context("2d").map_err(js)?.ok_or("no 2d context")?.dyn_into().map_err(|_| "not a 2d context".to_string())?;
    if (width, height) == (src_w, src_h) {
        context.draw_image_with_image_bitmap(&bitmap, 0., 0.).map_err(js)?;
    } else {
        // No typed binding for the enum in web-sys.
        let _ = js_sys::Reflect::set(&context, &"imageSmoothingQuality".into(), &"high".into());
        context.draw_image_with_image_bitmap_and_dw_and_dh(&bitmap, 0., 0., width as f64, height as f64).map_err(js)?;
    }
    let data = context.get_image_data(0., 0., width as f64, height as f64).map_err(js)?;
    bitmap.close();
    Ok(Decoded { width, height, rgba: data.data().0 })
}
