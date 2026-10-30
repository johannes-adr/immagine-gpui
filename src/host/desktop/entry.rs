use crate::immagine::Immagine;
use gpui_kit::*;

/// Opens the app in a phone-sized window on the standard GPUI platform (Metal, Vulkan, ...).
pub fn run() {
    gpui_kit::platform::application().run(|cx| {
        gpui_kit::init(cx);
        if matches!(cx.window_appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark) {
            gpui_kit::component::theme::Theme::change(gpui_kit::component::theme::ThemeMode::Dark, None, cx);
        }
        let bounds = Bounds::centered(None, size(px(402.), px(874.)), cx);
        let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() };
        gpui_kit::open_window(options, cx, |window, cx| {
            let dark = matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark);
            cx.new(|cx| Immagine::new(0., 0., dark, false, window, cx))
        })
            .expect("failed to open window");
        cx.activate(true);
    });
}
