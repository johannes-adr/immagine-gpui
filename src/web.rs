use crate::immagine::Immagine;
use gpui_kit::*;
use std::borrow::Cow;
use std::cell::RefCell;
use wasm_bindgen::prelude::*;

thread_local! {
    static HANDLE: RefCell<Option<ApplicationHandle>> = const { RefCell::new(None) };
}

/// `top`/`bottom` are the safe-area insets in CSS px; `dark` follows the OS appearance.
#[wasm_bindgen]
pub fn run(top: f32, bottom: f32, dark: bool, webgpu: bool) {
    gpui_kit::platform::web_init();
    let handle = gpui_kit::platform::application().run_embedded(move |cx| {
        // The web platform starts with no fonts; load them before opening a window.
        cx.text_system()
            .add_fonts(vec![
                Cow::Borrowed(include_bytes!("../assets/Arial.sub.ttf").as_slice()),
                Cow::Borrowed(include_bytes!("../assets/Arial-Bold.sub.ttf").as_slice()),
            ])
            .expect("failed to load fonts");
        gpui_kit::init(cx);
        if dark {
            gpui_kit::component::theme::Theme::change(
                gpui_kit::component::theme::ThemeMode::Dark,
                None,
                cx,
            );
        }
        gpui_kit::open_window(WindowOptions::default(), cx, move |window, cx| {
            cx.new(|cx| Immagine::new(top, bottom, dark, webgpu, window, cx))
        })
        .expect("failed to open window");
    });
    HANDLE.with(|h| *h.borrow_mut() = Some(handle));
}
