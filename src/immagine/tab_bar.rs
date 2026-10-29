use super::Immagine;
use crate::components::glass::glass_div;
use crate::theme::{Palette, BLUE};
use crate::util::{lerp, spring_out};
use gpui_kit::*;
use std::time::Duration;

pub(super) const TABS: [&str; 3] = ["Mediathek", "Alben", "Personen"];

impl Immagine {
    pub(super) fn select_tab(&mut self, tab: usize, cx: &mut Context<Self>) {
        if tab != self.tab {
            self.prev_tab = self.tab;
            self.tab = tab;
            self.tab_gen += 1;
            cx.notify();
        }
    }

    pub(super) fn tab_bar(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let n = TABS.len() as f32;
        let (prev, cur) = (self.prev_tab as f32, self.tab as f32);
        let thumb = glass_div(p.glass_tint, 28.)
            .id(("thumb", self.tab_gen))
            .absolute()
            .top_0()
            .h_full()
            .w(relative(1. / n))
            .bg(p.chip)
            .with_animation(
                ("thumb-anim", self.tab_gen),
                Animation::new(Duration::from_millis(520)).with_easing(spring_out),
                move |el, d| el.left(relative(lerp(prev, cur, d) / n)),
            );

        let color = |ix: usize| -> Rgba { if self.tab == ix { rgb(BLUE).into() } else { p.secondary } };
        let photo_icon = |c: Rgba| {
            div()
                .size(px(20.))
                .rounded(px(5.))
                .border_2()
                .border_color(c)
                .flex()
                .items_center()
                .justify_center()
                .child(div().size(px(6.)).rounded_full().bg(c))
        };
        let album_icon = |c: Rgba| {
            div().size(px(22.)).relative().child(div().absolute().top_0().left(px(3.)).w(px(16.)).h(px(3.)).rounded_full().bg(c)).child(
                div().absolute().top(px(5.)).left_0().size(px(20.)).rounded(px(5.)).border_2().border_color(c),
            )
        };
        let person_icon = |c: Rgba| {
            div()
                .size(px(22.))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(2.))
                .child(div().size(px(9.)).rounded_full().bg(c))
                .child(div().w(px(18.)).h(px(9.)).rounded_t(px(9.)).bg(c))
        };

        let item = |ix: usize, icon: Div| {
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
                .child(div().text_size(px(11.)).font_weight(FontWeight::MEDIUM).text_color(color(ix)).child(TABS[ix]))
                .on_click(cx.listener(move |this, _, _, cx| this.select_tab(ix, cx)))
        };

        div().absolute().left(px(20.)).right(px(20.)).bottom(px(self.bottom.max(10.))).occlude().child(
            glass_div(p.glass_tint, 32.).h(px(64.)).p(px(4.)).child(
                div()
                    .relative()
                    .size_full()
                    .flex()
                    .child(thumb)
                    .child(item(0, photo_icon(color(0))))
                    .child(item(1, album_icon(color(1))))
                    .child(item(2, person_icon(color(2)))),
            ),
        )
        .into_any_element()
    }
}