//! Image decoding by the browser: `fetch` -> `createImageBitmap` -> pixels. The browser decodes JPEG/WebP
//! natively (and off the main thread), which is far cheaper than decoding in WASM on the UI thread.

use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use crate::host::Decoded;
use web_sys::{Blob, ImageBitmap, OffscreenCanvas, OffscreenCanvasRenderingContext2d, RequestInit, RequestCredentials, Response};

fn js(error: JsValue) -> String {
    error.as_string().unwrap_or_else(|| format!("{error:?}"))
}

/// Downloads `url` (same-origin credentials, so the Immich cookie applies) and decodes it.
pub async fn decode_url(url: &str) -> Result<Decoded, String> {
    let window = web_sys::window().ok_or("no window")?;
    let init = RequestInit::new();
    init.set_credentials(RequestCredentials::SameOrigin);
    let request = web_sys::Request::new_with_str_and_init(url, &init).map_err(js)?;
    let response: Response = JsFuture::from(window.fetch_with_request(&request)).await.map_err(js)?.dyn_into().map_err(js)?;
    if !response.ok() {
        return Err(format!("HTTP {}", response.status()));
    }
    let blob: Blob = JsFuture::from(response.blob().map_err(js)?).await.map_err(js)?.dyn_into().map_err(js)?;
    let bitmap: ImageBitmap = JsFuture::from(window.create_image_bitmap_with_blob(&blob).map_err(js)?)
        .await
        .map_err(js)?
        .dyn_into()
        .map_err(js)?;
    let (width, height) = (bitmap.width(), bitmap.height());
    let canvas = OffscreenCanvas::new(width, height).map_err(js)?;
    let context: OffscreenCanvasRenderingContext2d =
        canvas.get_context("2d").map_err(js)?.ok_or("no 2d context")?.dyn_into().map_err(|_| "not a 2d context".to_string())?;
    context.draw_image_with_image_bitmap(&bitmap, 0., 0.).map_err(js)?;
    let data = context.get_image_data(0., 0., width as f64, height as f64).map_err(js)?;
    bitmap.close();
    Ok(Decoded { width, height, rgba: data.data().0 })
}
