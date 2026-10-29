use gpui_kit::*;

pub(crate) const BLUE: u32 = 0x007aff;
pub(crate) const RED: u32 = 0xff3b30;
pub(crate) const GLASS_TINT_DARK: Hsla = Hsla { h: 0., s: 0., l: 1., a: 0.16 };
pub(crate) const GLASS_TINT_LIGHT: Hsla = Hsla { h: 0., s: 0., l: 1., a: 0.34 };

/// Palette that tracks the system appearance, modeled on iOS semantic colors.
pub(crate) struct Palette {
    pub(crate) label: Rgba,
    pub(crate) secondary: Rgba,
    pub(crate) separator: Rgba,
    pub(crate) glass_tint: Hsla,
    pub(crate) chip: Rgba,
    pub(crate) bar_tint: Hsla,
    pub(crate) tab_inactive: Rgba,
    pub(crate) tab_thumb: Rgba,
}

impl Palette {
    pub(crate) fn new(dark: bool) -> Self {
        if dark {
            Self {
                label: rgba(0xffffffff),
                secondary: rgba(0xebebf599),
                separator: rgba(0xffffff26),
                glass_tint: GLASS_TINT_DARK,
                chip: rgba(0xffffff33),
                bar_tint: Hsla { h: 0., s: 0., l: 0.1, a: 0.45 },
                tab_inactive: rgba(0xffffffd9),
                tab_thumb: rgba(0xffffff33),
            }
        } else {
            Self {
                label: rgba(0x000000ff),
                secondary: rgba(0x3c3c4399),
                separator: rgba(0x3c3c434a),
                glass_tint: GLASS_TINT_LIGHT,
                chip: rgba(0x0000001a),
                bar_tint: Hsla { h: 0., s: 0., l: 1., a: 0.62 },
                tab_inactive: rgba(0x000000b3),
                tab_thumb: rgba(0xffffffcc),
            }
        }
    }
}
