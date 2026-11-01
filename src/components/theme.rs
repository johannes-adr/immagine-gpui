//! Colours the components take from their host. Every component receives what it needs as parameters; `Theme` is
//! the convenient bundle for them, with iOS-like light and dark defaults. An app builds its palette from it (or
//! maps its own onto it) once per render.

use gpui_kit::*;

#[derive(Clone, Copy)]
pub(crate) struct Theme {
    /// Primary text and icons.
    pub label: Rgba,
    /// Captions, values, hints.
    pub secondary: Rgba,
    /// Hairlines between rows.
    pub separator: Rgba,
    /// Tint of glass that sits on the page (chips, fields).
    pub glass: Hsla,
    /// Tint of glass bars.
    pub bar: Hsla,
    /// Page behind grouped lists and sheets.
    pub background: Rgba,
    /// Grouped-list rows and cards.
    pub card: Rgba,
    /// Tappable text and selection.
    pub accent: Rgba,
    /// Destructive actions.
    pub destructive: Rgba,
    /// Switch "on".
    pub positive: Rgba,
    /// Faint fill of controls (switch "off", progress tracks, secondary buttons).
    pub fill: Rgba,
}

impl Theme {
    pub(crate) fn new(dark: bool) -> Self {
        let (accent, destructive, positive, fill) = (rgb(0x007aff), rgb(0xff3b30), rgb(0x34c759), rgba(0x78788033));
        if dark {
            Self {
                label: rgba(0xffffffff),
                secondary: rgba(0xebebf599),
                separator: rgba(0xffffff26),
                glass: Hsla { h: 0., s: 0., l: 1., a: 0.16 },
                bar: Hsla { h: 0., s: 0., l: 0.1, a: 0.45 },
                background: rgb(0x1c1c1e),
                card: rgb(0x2c2c2e),
                accent,
                destructive,
                positive,
                fill,
            }
        } else {
            Self {
                label: rgba(0x000000ff),
                secondary: rgba(0x3c3c4399),
                separator: rgba(0x3c3c434a),
                glass: Hsla { h: 0., s: 0., l: 1., a: 0.34 },
                bar: Hsla { h: 0., s: 0., l: 1., a: 0.62 },
                background: rgb(0xf2f2f7),
                card: rgb(0xffffff),
                accent,
                destructive,
                positive,
                fill,
            }
        }
    }
}
