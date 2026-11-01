use crate::components::theme::Theme;
use gpui_kit::*;

pub(crate) const BLUE: u32 = 0x007aff;
pub(crate) const RED: u32 = 0xff3b30;
pub(crate) const GLASS_TINT_DARK: Hsla = Hsla { h: 0., s: 0., l: 1., a: 0.16 };

/// The app's palette: the components' `Theme` (system appearance, iOS semantic colors) plus a few app-only colors.
pub(crate) struct Palette {
    pub(crate) label: Rgba,
    pub(crate) secondary: Rgba,
    pub(crate) separator: Rgba,
    pub(crate) glass_tint: Hsla,
    pub(crate) chip: Rgba,
    pub(crate) bar_tint: Hsla,
    pub(crate) tab_inactive: Rgba,
    pub(crate) tab_thumb: Rgba,
    /// The same colors as the components take them.
    pub(crate) ui: Theme,
}

impl Palette {
    pub(crate) fn new(dark: bool) -> Self {
        let ui = Theme::new(dark);
        let (chip, tab_inactive, tab_thumb) = if dark {
            (rgba(0xffffff33), rgba(0xffffffd9), rgba(0xffffff33))
        } else {
            (rgba(0x0000001a), rgba(0x000000b3), rgba(0xffffffcc))
        };
        Self { label: ui.label, secondary: ui.secondary, separator: ui.separator, glass_tint: ui.glass, bar_tint: ui.bar, chip, tab_inactive, tab_thumb, ui }
    }
}
