//! Video playback as a DOM `<video>` element above the canvas: the browser decodes and streams (HTTP range
//! requests through the same-origin proxy, auth by cookie), GPUI has no video element of its own.

use wasm_bindgen::JsCast;
use web_sys::{HtmlElement, HtmlVideoElement};

pub struct VideoHandle {
    element: HtmlVideoElement,
}

impl Drop for VideoHandle {
    fn drop(&mut self) {
        let _ = self.element.pause();
        self.element.remove();
    }
}

/// Shows `url` between `top` and `bottom` px of the page (so the viewer's own buttons stay visible).
pub fn show(url: &str, top: f32, bottom: f32) -> Option<VideoHandle> {
    let document = web_sys::window()?.document()?;
    let element: HtmlVideoElement = document.create_element("video").ok()?.dyn_into().ok()?;
    element.set_src(url);
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
    Some(VideoHandle { element })
}
