use crate::app::Todos;
use crate::theme::{Palette, BLUE, RED};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

impl Todos {
    pub(crate) fn row(&self, ix: usize, last: bool, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let todo = &self.items[ix];
        let id = todo.id;
        let done = todo.done;

        let check = div()
            .id(("check", id))
            .size(px(24.))
            .flex_none()
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .border_2()
            .border_color(if done { rgba(BLUE << 8 | 0xff) } else { p.secondary })
            .when(done, |d| {
                d.bg(rgb(BLUE))
                    .child(div().size(px(10.)).rounded_full().bg(rgb(0xffffff)))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(t) = this.items.iter_mut().find(|t| t.id == id) {
                    t.done = !t.done;
                }
                cx.notify();
            }));

        let delete = div()
            .id(("del", id))
            .size(px(22.))
            .flex_none()
            .rounded_full()
            .bg(rgb(RED))
            .text_color(rgb(0xffffff))
            .flex()
            .items_center()
            .justify_center()
            .active(|s| s.opacity(0.6))
            .child("−")
            .on_click(cx.listener(move |this, _, _, cx| {
                this.items.retain(|t| t.id != id);
                cx.notify();
            }));

        div()
            .id(("row", id))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .h(px(52.))
                    .when(self.editing, |d| d.child(delete))
                    .child(check)
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(17.))
                            .text_color(if done { p.secondary } else { p.label })
                            .when(done, |d| d.line_through())
                            .child(todo.title.clone()),
                    ),
            )
            .when(!last, |d| {
                d.child(div().ml(px(52.)).h(px(0.5)).bg(p.separator))
            })
            .into_any_element()
    }
}
