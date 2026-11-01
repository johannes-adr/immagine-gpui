//! Grouped lists in the iOS settings style: an uppercase caption above a rounded card whose rows are separated by
//! hairlines.
//!
//! ```ignore
//! list::Group::new(&theme, "Storage")
//!     .row("Used", "1,5 GB")
//!     .action("clear", "Clear cache", theme.accent, cx.listener(|this, _, _, cx| this.clear(cx)))
//!     .toggle("fps", "Show FPS", on, cx.listener(|this, _, _, cx| this.flip(cx)))
//!     .build()
//! ```

use crate::components::button::dim;
use crate::components::theme::Theme;
use gpui_kit::*;

/// Row height (tall enough to tap).
pub(crate) const ROW_H: f32 = 48.;
/// Corner radius of the card.
pub(crate) const RADIUS: f32 = 22.;

/// The small uppercase caption above a group or section.
pub(crate) fn caption(theme: &Theme, text: &str) -> Div {
    div().px_4().text_size(px(13.)).text_color(theme.secondary).child(text.to_uppercase())
}

/// A caption with plain elements (cards) below it.
pub(crate) fn section(theme: &Theme, title: &str, children: Vec<Div>) -> Div {
    div().flex().flex_col().gap_2().child(caption(theme, title)).children(children)
}

/// A caption above a card of rows. Each `row`/`action`/`toggle`/`raw` call appends one row; the group puts the
/// hairline between them.
pub(crate) struct Group {
    theme: Theme,
    caption: String,
    rows: Vec<AnyElement>,
}

impl Group {
    pub(crate) fn new(theme: &Theme, caption: impl Into<String>) -> Self {
        Self { theme: *theme, caption: caption.into(), rows: Vec::new() }
    }

    /// The hairline above every row but the first.
    fn line(&self, row: Div) -> Div {
        if self.rows.is_empty() { row } else { row.border_t_1().border_color(self.theme.separator) }
    }

    fn base(&self, h: Div) -> Div {
        h.min_h(px(ROW_H)).px_4().flex().flex_row().items_center().justify_between().gap_3().text_size(px(17.))
    }

    /// Label on the left, value (secondary color) on the right.
    pub(crate) fn row(mut self, label: impl Into<SharedString>, value: impl Into<SharedString>) -> Self {
        let row = self.line(self.base(div())).child(div().child(label.into())).child(div().text_color(self.theme.secondary).child(value.into()));
        self.rows.push(row.into_any_element());
        self
    }

    /// A tappable row with a colored label.
    pub(crate) fn action(mut self, id: impl Into<ElementId>, label: impl Into<SharedString>, color: Rgba, on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        let row = self.line(self.base(div())).id(id).justify_start().text_color(color).active(dim).child(label.into()).on_click(on_click);
        self.rows.push(row.into_any_element());
        self
    }

    /// A row with a switch on the right.
    pub(crate) fn toggle(mut self, id: impl Into<ElementId>, label: impl Into<SharedString>, on: bool, on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        let track = div().w(px(51.)).h(px(31.)).rounded_full().p(px(2.)).flex();
        let track = if on { track.justify_end().bg(self.theme.positive) } else { track.bg(self.theme.fill) };
        let row = self.line(self.base(div())).id(id).active(dim).child(label.into()).child(track.child(div().size(px(27.)).rounded_full().bg(rgb(0xffffff)))).on_click(on_click);
        self.rows.push(row.into_any_element());
        self
    }

    /// Any element as a row; it brings its own hairline if it is not the first.
    pub(crate) fn raw(mut self, row: impl IntoElement) -> Self {
        self.rows.push(row.into_any_element());
        self
    }

    pub(crate) fn build(self) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(caption(&self.theme, &self.caption))
            .child(div().rounded(px(RADIUS)).bg(self.theme.card).overflow_hidden().flex().flex_col().children(self.rows))
    }
}
