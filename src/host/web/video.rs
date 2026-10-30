//! Video playback as a DOM `<video>` element above the canvas: the browser decodes and streams (HTTP range
//! requests through the same-origin proxy, auth by cookie), GPUI has no video element of its own.
//! A server on another origin cannot be authorized by a `<video src>` (no cookie, no header), so there the
//! file is fetched with the bearer token and played from a blob URL (it has to download first).

use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Blob, HtmlElement, HtmlVideoElement, RequestInit, Response};

pub struct VideoHandle {
    element: HtmlVideoElement,
    blob_url: Rc<RefCell<Option<String>>>,
    closed: Rc<std::cell::Cell<bool>>,
}

impl Drop for VideoHandle {
    fn drop(&mut self) {
        self.closed.set(true);
        let _ = self.element.pause();
        self.element.remove();
        if let Some(url) = self.blob_url.borrow_mut().take() {
            let _ = web_sys::Url::revoke_object_url(&url);
        }
    }
}

async fn fetch_blob_url(url: &str, token: &str) -> Option<String> {
    let window = web_sys::window()?;
    let headers = web_sys::Headers::new().ok()?;
    headers.set("Authorization", &format!("Bearer {token}")).ok()?;
    let init = RequestInit::new();
    init.set_headers(&headers);
    let request = web_sys::Request::new_with_str_and_init(url, &init).ok()?;
    let response: Response = JsFuture::from(window.fetch_with_request(&request)).await.ok()?.dyn_into().ok()?;
    if !response.ok() {
        return None;
    }
    let blob: Blob = JsFuture::from(response.blob().ok()?).await.ok()?.dyn_into().ok()?;
    web_sys::Url::create_object_url_with_blob(&blob).ok()
}

/// Shows `url` between `top` and `bottom` px of the page (so the viewer's own buttons stay visible).
pub fn show(url: &str, top: f32, bottom: f32) -> Option<VideoHandle> {
    let document = web_sys::window()?.document()?;
    let element: HtmlVideoElement = document.create_element("video").ok()?.dyn_into().ok()?;
    let blob_url = Rc::new(RefCell::new(None));
    let closed = Rc::new(std::cell::Cell::new(false));
    let cross_origin = !url.starts_with(&super::origin_of_page());
    match super::local_storage::get(super::TOKEN_KEY).filter(|_| cross_origin) {
        Some(token) => {
            let (url, element, blob_url, closed) = (url.to_string(), element.clone(), blob_url.clone(), closed.clone());
            wasm_bindgen_futures::spawn_local(async move {
                let Some(object_url) = fetch_blob_url(&url, &token).await else { return };
                if closed.get() {
                    let _ = web_sys::Url::revoke_object_url(&object_url);
                    return;
                }
                element.set_src(&object_url);
                let _ = element.play();
                *blob_url.borrow_mut() = Some(object_url);
            });
        }
        None => element.set_src(url),
    }
    element.set_controls(true);
    element.set_autoplay(true);
    let _ = element.set_attribute("playsinline", "");
    let style = element.unchecked_ref::<HtmlElement>().style();
    let _ = style.set_property("position", "fixed");
    let _ = style.set_property("left", "0");
    let _ = style.set_property("right", "0");
    let _ = style.set_property("top", &format!("{top}px"));
    let _ = style.set_property("bottom", &format!("{bottom}px"));
    let _ = style.set_property("width", "100%");
    let _ = style.set_property("height", &format!("calc(100% - {}px)", top + bottom));
    let _ = style.set_property("object-fit", "contain");
    let _ = style.set_property("background", "#000");
    let _ = style.set_property("z-index", "5");
    document.body()?.append_child(&element).ok()?;
    Some(VideoHandle { element, blob_url, closed })
}
