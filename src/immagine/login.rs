use super::Immagine;
use crate::components::glass::glass_div;
use crate::theme::{Palette, BLUE, RED};
use gpui_kit::component::input::Input;
use gpui_kit::*;

impl Immagine {
    pub(super) fn login_view(&self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let field = |input: &Entity<gpui_kit::component::input::InputState>| {
            glass_div(p.glass_tint, 16.).h(px(48.)).px_4().flex().items_center().child(
                div().flex_1().child(Input::new(input).appearance(false).bordered(false).cleanable(false)),
            )
        };
        let button = div()
            .id("login")
            .h(px(48.))
            .rounded(px(16.))
            .bg(rgb(BLUE))
            .text_color(rgb(0xffffff))
            .text_size(px(17.))
            .font_weight(FontWeight::SEMIBOLD)
            .flex()
            .items_center()
            .justify_center()
            .active(|s| s.opacity(0.6))
            .child(if self.logging_in { "Anmelden …" } else { "Anmelden" })
            .on_click(cx.listener(|this, _, _, cx| this.log_in(cx)));

        div()
            .size_full()
            .flex()
            .flex_col()
            .justify_center()
            .px(px(28.))
            .gap_3()
            .pt(px(self.top))
            // The centered form shrinks to the space above the keyboard, so the fields and the button stay visible.
            .pb(px(Self::keyboard_height(window)))
            .child(div().text_size(px(34.)).font_weight(FontWeight::BOLD).child("Immagine"))
            .child(div().pb_3())
            .child(field(&self.server))
            .child(field(&self.email))
            .child(field(&self.password))
            .children(self.login_error.clone().map(|e| div().text_size(px(14.)).text_color(rgb(RED)).child(e)))
            .child(button)
    }
}
