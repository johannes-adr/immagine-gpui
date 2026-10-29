use super::Todos;
use crate::components::glass::glass_div;
use crate::theme::{Palette, BLUE};
use crate::util::{lerp, spring_out};
use gpui_kit::*;
use std::time::Duration;

impl Todos {
    pub(crate) fn tab_bar(&self, p: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let (prev, cur) = (self.prev_tab as f32, self.tab as f32);
        let thumb = glass_div(p.glass_tint, 28.)
            .id(("thumb", self.tab_gen))
            .absolute()
            .top_0()
            .h_full()
            .w(relative(0.5))
            .bg(p.chip)
            .with_animation(
                ("thumb-anim", self.tab_gen),
                Animation::new(Duration::from_millis(520)).with_easing(spring_out),
                move |el, d| el.left(relative(0.5 * lerp(prev, cur, d))),
            );

        let item = |ix: usize, label: &'static str, icon: Div| {
            div()
                .id(("tab", ix))
                .flex_1()
                .h_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(3.))
                .active(|s| s.opacity(0.6))
                .child(icon)
                .child(
                    div()
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if self.tab == ix { rgb(BLUE).into() } else { p.secondary })
                        .child(label),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.select_tab(ix, cx)))
        };

        let bar = |color: Rgba, w: f32| div().w(px(w)).h(px(3.)).rounded_full().bg(color);
        let list_icon = |color: Rgba| {
            div().w(px(22.)).h(px(20.)).flex().flex_col().justify_between().py(px(2.)).children(
                (0..3).map(|_| {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(3.))
                        .child(div().size(px(3.)).rounded_full().bg(color))
                        .child(bar(color, 15.))
                }),
            )
        };
        let photo_icon = |color: Rgba| {
            div()
                .size(px(20.))
                .rounded(px(5.))
                .border_2()
                .border_color(color)
                .flex()
                .items_center()
                .justify_center()
                .child(div().size(px(6.)).rounded_full().bg(color))
        };
        let c0 = if self.tab == 0 { rgb(BLUE).into() } else { p.secondary };
        let c1 = if self.tab == 1 { rgb(BLUE).into() } else { p.secondary };

        div()
            .absolute()
            .left(px(20.))
            .right(px(20.))
            .bottom(px(self.bottom.max(10.)))
            .occlude()
            .child(
                glass_div(p.glass_tint, 32.).h(px(64.)).p(px(4.)).child(
                    div()
                        .relative()
                        .size_full()
                        .flex()
                        .child(thumb)
                        .child(item(0, "Reminders", list_icon(c0)))
                        .child(item(1, "Photos", photo_icon(c1))),
                ),
            )
    }
}
