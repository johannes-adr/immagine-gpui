mod row;

use super::Todos;
use crate::components::glass::glass_div;
use crate::theme::{Palette, BLUE};
use gpui_kit::component::input::Input;
use gpui_kit::*;

pub(crate) struct Todo {
    pub(crate) id: u32,
    pub(crate) title: SharedString,
    pub(crate) done: bool,
}

impl Todos {
    pub(crate) fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        self.items.push(Todo {
            id: self.next_id,
            title: text.into(),
            done: false,
        });
        self.next_id += 1;
        self.input.update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    pub(crate) fn reminders_view(&mut self, p: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let remaining = self.items.iter().filter(|t| !t.done).count();
        let n = self.items.len();

        let rows: Vec<_> = (0..n)
            .map(|ix| self.row(ix, ix + 1 == n, p, cx))
            .collect();

        let list = div()
            .id("list")
            .flex_1()
            .overflow_y_scroll()
            .px_4()
            .child(
                glass_div(p.glass_tint, 26.)
                    .overflow_hidden()
                    .children(rows),
            );

        let header = div()
            .px_5()
            .pb_3()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div().flex().justify_end().child(
                    glass_div(p.glass_tint, 18.)
                        .id("edit")
                        .h(px(36.))
                        .px_4()
                        .flex()
                        .items_center()
                        .text_size(px(17.))
                        .text_color(rgb(BLUE))
                        .active(|s| s.opacity(0.6))
                        .child(if self.editing { "Done" } else { "Edit" })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.editing = !this.editing;
                            cx.notify();
                        })),
                ),
            )
            .child(
                div()
                    .text_size(px(34.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.label)
                    .child("Reminders"),
            )
            .child(
                div()
                    .text_size(px(15.))
                    .text_color(p.secondary)
                    .child(format!("{remaining} remaining")),
            );

        let add_btn = div()
            .id("add")
            .size(px(36.))
            .flex_none()
            .rounded_full()
            .bg(rgb(BLUE))
            .text_color(rgb(0xffffff))
            .text_size(px(22.))
            .flex()
            .items_center()
            .justify_center()
            .active(|s| s.opacity(0.6))
            .child("+")
            .on_click(cx.listener(|this, _, window, cx| this.add(window, cx)));

        let composer = div().occlude().px_4().pt_2().pb(px(self.bottom.max(10.) + 64. + 12.)).child(
            glass_div(p.glass_tint, 26.)
                .h(px(52.))
                .pl_4()
                .pr(px(8.))
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div().flex_1().child(
                        Input::new(&self.input)
                            .appearance(false)
                            .bordered(false)
                            .cleanable(false),
                    ),
                )
                .child(add_btn),
        );

        div()
            .size_full()
            .flex()
            .flex_col()
            .pt(px(self.top.max(12.)))
            .child(header)
            .child(list)
            .child(composer)
    }
}
