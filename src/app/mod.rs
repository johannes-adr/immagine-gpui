mod photos;
mod reminders;
mod tab_bar;

use crate::components::fps::{fps_badge, FpsMeter};
use crate::theme::Palette;
use crate::components::ease::{anim, lerp};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use photos::{Viewer, PHOTO_COUNT};
use reminders::Todo;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

pub(crate) struct Todos {
    items: Vec<Todo>,
    next_id: u32,
    editing: bool,
    input: Entity<InputState>,
    // Safe-area insets (CSS px) and color scheme reported by the page.
    top: f32,
    bottom: f32,
    dark: bool,
    tab: usize,
    prev_tab: usize,
    tab_gen: u32,
    tile_bounds: Rc<RefCell<Vec<Bounds<Pixels>>>>,
    // (photo index, pressed, generation) of the last press change, to animate it.
    press: Option<(usize, bool, u32)>,
    press_gen: u32,
    viewer: Option<Viewer>,
    meter: Rc<FpsMeter>,
    fps: u32,
    worst_ms: u32,
    webgpu: bool,
}

impl Todos {
    pub(crate) fn new(
        top: f32,
        bottom: f32,
        dark: bool,
        webgpu: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("New Reminder"));
        cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.add(window, cx);
            }
        })
        .detach();

        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this.update(cx, |t, cx| t.tick_fps(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        let seed = ["Buy oat milk", "Book dentist", "Reply to Anna", "Plan weekend trip"];
        let items = seed
            .iter()
            .enumerate()
            .map(|(i, t)| Todo {
                id: i as u32,
                title: (*t).into(),
                done: i == 0,
            })
            .collect();
        Self {
            items,
            next_id: seed.len() as u32,
            editing: false,
            input,
            top,
            bottom,
            dark,
            tab: 0,
            prev_tab: 0,
            tab_gen: 0,
            tile_bounds: Rc::new(RefCell::new(vec![Bounds::default(); PHOTO_COUNT])),
            press: None,
            press_gen: 0,
            viewer: None,
            meter: Rc::new(FpsMeter::default()),
            fps: 0,
            worst_ms: 0,
            webgpu,
        }
    }

    /// Once a second, publishes the frame rate measured by the meter.
    fn tick_fps(&mut self, cx: &mut Context<Self>) {
        let Some((fps, worst)) = self.meter.reading() else {
            return;
        };
        if (fps, worst) != (self.fps, self.worst_ms) {
            self.fps = fps;
            self.worst_ms = worst;
            self.meter.skip_next_paint();
            cx.notify();
        }
    }

    fn select_tab(&mut self, tab: usize, cx: &mut Context<Self>) {
        if tab != self.tab {
            self.prev_tab = self.tab;
            self.tab = tab;
            self.tab_gen += 1;
            cx.notify();
        }
    }
}

impl Render for Todos {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::new(self.dark);
        let (bg_from, bg_to) = if self.dark {
            (rgb(0x0b1026), rgb(0x2a1450))
        } else {
            (rgb(0xcfe3ff), rgb(0xf6dcff))
        };
        let blob = |x: f32, y: f32, size: f32, color: u32| {
            div()
                .absolute()
                .left(px(x))
                .top(px(y))
                .size(px(size))
                .rounded_full()
                .bg(rgba(color))
        };

        let content = if self.tab == 0 {
            self.reminders_view(&p, cx).into_any_element()
        } else {
            self.photos_view(&p, window, cx).into_any_element()
        };

        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(linear_gradient(
                160.,
                linear_color_stop(bg_from, 0.),
                linear_color_stop(bg_to, 1.),
            ))
            .text_color(p.label)
            // Soft color fields so the glass has something to refract.
            .child(blob(-80., 40., 280., 0x5ac8fa66))
            .child(blob(150., 260., 300., 0xff2d9266))
            .child(blob(-60., 560., 260., 0xffcc0055))
            .child(
                div()
                    .id(("content", self.tab_gen))
                    .size_full()
                    .with_animation(("content-anim", self.tab_gen), anim(380), move |el, d| {
                        el.opacity(d).left(px(lerp(24., 0., d)))
                    })
                    .child(content),
            )
            .child(self.tab_bar(&p, cx))
            .children(self.viewer_overlay(window, cx))
            .child(fps_badge(self.meter.clone(), self.top, self.fps, self.worst_ms, <crate::host::Current as crate::host::Host>::renderer_name(self.webgpu)))
    }
}
