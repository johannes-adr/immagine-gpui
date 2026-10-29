use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

const BLUE: u32 = 0x007aff;
const RED: u32 = 0xff3b30;
const GLASS_TINT_DARK: Hsla = Hsla { h: 0., s: 0., l: 1., a: 0.16 };
const GLASS_TINT_LIGHT: Hsla = Hsla { h: 0., s: 0., l: 1., a: 0.34 };

struct Todo {
    id: u32,
    title: SharedString,
    done: bool,
}

#[cfg(target_family = "wasm")]
mod js {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = performance, js_name = now)]
        pub fn now() -> f64;
    }
}

fn now_ms() -> f64 {
    #[cfg(target_family = "wasm")]
    return js::now();
    #[cfg(not(target_family = "wasm"))]
    0.
}

/// Counts frames the renderer actually paints, so the FPS readout is the app's real frame rate.
#[derive(Default)]
struct FpsMeter {
    frames: Cell<u32>,
    first: Cell<f64>,
    last: Cell<f64>,
    worst_gap: Cell<f64>,
    // Set before the readout repaints itself so that repaint is not counted.
    skip: Cell<bool>,
}

const PHOTO_COUNT: usize = 24;
const PRESS_GROW: f32 = 5.;
const TILE_GAP: f32 = 6.;

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn anim(ms: u64) -> Animation {
    Animation::new(Duration::from_millis(ms)).with_easing(ease_out_quint())
}

/// A generated "photo": sky gradient, sun and two hills, all sized in px.
fn photo(ix: usize, w: f32, h: f32, radius: f32) -> Div {
    let hue = ((ix * 47) % 360) as f32 / 360.;
    let wrap = |v: f32| v.rem_euclid(1.);
    let side = w.min(h);
    let var = (ix % 3) as f32;
    div()
        .relative()
        .overflow_hidden()
        .w(px(w))
        .h(px(h))
        .rounded(px(radius))
        .bg(linear_gradient(
            180.,
            linear_color_stop(hsla(hue, 0.75, 0.62, 1.), 0.),
            linear_color_stop(hsla(wrap(hue + 0.12), 0.8, 0.38, 1.), 1.),
        ))
        .child(
            div()
                .absolute()
                .left(px(w * (0.15 + 0.3 * var)))
                .top(px(h * 0.14))
                .size(px(side * 0.26))
                .rounded_full()
                .bg(hsla(wrap(hue + 0.1), 0.9, 0.88, 0.9)),
        )
        .child(
            div()
                .absolute()
                .left(px(-w * 0.25))
                .top(px(h * 0.62))
                .size(px(w * 1.2))
                .rounded_full()
                .bg(hsla(wrap(hue - 0.05), 0.55, 0.24, 0.85)),
        )
        .child(
            div()
                .absolute()
                .left(px(w * 0.3))
                .top(px(h * 0.74))
                .size(px(w * 1.0))
                .rounded_full()
                .bg(hsla(wrap(hue + 0.02), 0.5, 0.16, 0.9)),
        )
}

/// Liquid glass: a div whose backdrop is blurred and refracted by the renderer's glass pass.
/// Children are painted after the glass, so they stay crisp on top of it.
fn glass_div(tint: Hsla, radius: f32) -> Div {
    let style = GlassStyle {
        corner_radius: px(radius),
        blur: px(12.),
        bezel: px((radius * 0.7).clamp(10., 22.)),
        refraction: px((radius * 0.5).clamp(6., 14.)),
        dispersion: 0.06,
        light: 1.0,
        tint,
    };
    div().relative().rounded(px(radius)).child(
        canvas(|_, _, _| (), move |bounds, _, window, _| window.paint_glass(bounds, style))
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
    )
}

/// Ease-out with a small overshoot, like a light iOS spring.
fn spring_out(t: f32) -> f32 {
    let (c1, c3) = (1.3_f32, 2.3_f32);
    1. + c3 * (t - 1.).powi(3) + c1 * (t - 1.).powi(2)
}

struct Viewer {
    ix: usize,
    from: Bounds<Pixels>,
    closing: bool,
    serial: u32,
}

struct Todos {
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

/// Palette that tracks the system appearance, modeled on iOS semantic colors.
struct Palette {
    label: Rgba,
    secondary: Rgba,
    separator: Rgba,
    glass_tint: Hsla,
    chip: Rgba,
    shadow: Hsla,
}

impl Palette {
    fn new(dark: bool) -> Self {
        if dark {
            Self {
                label: rgba(0xffffffff),
                secondary: rgba(0xebebf599),
                separator: rgba(0xffffff26),
                glass_tint: GLASS_TINT_DARK,
                chip: rgba(0xffffff33),
                shadow: hsla(0., 0., 0., 0.35),
            }
        } else {
            Self {
                label: rgba(0x000000ff),
                secondary: rgba(0x3c3c4399),
                separator: rgba(0x3c3c434a),
                glass_tint: GLASS_TINT_LIGHT,
                chip: rgba(0x0000001a),
                shadow: hsla(0., 0., 0., 0.12),
            }
        }
    }
}

impl Todos {
    fn new(
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

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    /// Liquid glass with the palette's tint; see [`glass_div`].
    fn glass(&self, p: &Palette, radius: f32) -> Div {
        glass_div(p.glass_tint, radius)
    }

    /// Once a second, publishes the frame rate over the span in which frames were painted,
    /// so a half-second animation reads as its real rate, not diluted by idle time.
    fn tick_fps(&mut self, cx: &mut Context<Self>) {
        let m = &self.meter;
        let frames = m.frames.replace(0);
        let (first, last, worst) = (m.first.get(), m.last.get(), m.worst_gap.replace(0.));
        if frames < 3 || last <= first {
            return; // idle: keep showing the last measurement
        }
        let fps = ((frames - 1) as f64 / ((last - first) / 1000.)).round() as u32;
        let worst = worst.round() as u32;
        if (fps, worst) != (self.fps, self.worst_ms) {
            self.fps = fps;
            self.worst_ms = worst;
            self.meter.skip.set(true);
            cx.notify();
        }
    }

    fn fps_badge(&self) -> impl IntoElement {
        let meter = self.meter.clone();
        div()
            .absolute()
            .left(px(20.))
            .top(px(self.top.max(12.) + 7.))
            .h(px(22.))
            .px_2()
            .rounded_full()
            .flex()
            .items_center()
            .bg(rgba(0x000000a0))
            .text_color(rgb(0xffffff))
            .text_size(px(12.))
            .child(canvas(
                |_, _, _| (),
                move |_, _, _, _| {
                    if meter.skip.replace(false) {
                        return;
                    }
                    let now = now_ms();
                    if meter.frames.get() == 0 {
                        meter.first.set(now);
                    } else {
                        meter.worst_gap.set(meter.worst_gap.get().max(now - meter.last.get()));
                    }
                    meter.last.set(now);
                    meter.frames.set(meter.frames.get() + 1);
                },
            ))
            .child(format!(
                "{} FPS · {} ms worst · {}",
                self.fps,
                self.worst_ms,
                if self.webgpu { "WebGPU" } else { "WebGL2" }
            ))
    }

    fn select_tab(&mut self, tab: usize, cx: &mut Context<Self>) {
        if tab != self.tab {
            self.prev_tab = self.tab;
            self.tab = tab;
            self.tab_gen += 1;
            cx.notify();
        }
    }

    fn tab_bar(&self, p: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
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
                self.glass(p, 32.).h(px(64.)).p(px(4.)).child(
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

    fn open_photo(&mut self, ix: usize, cx: &mut Context<Self>) {
        let from = self.tile_bounds.borrow()[ix];
        let from = Bounds::new(
            point(from.origin.x - px(PRESS_GROW), from.origin.y - px(PRESS_GROW)),
            size(from.size.width + px(2. * PRESS_GROW), from.size.height + px(2. * PRESS_GROW)),
        );
        self.press = None;
        self.press_gen += 1;
        self.viewer = Some(Viewer { ix, from, closing: false, serial: self.press_gen });
        cx.notify();
    }

    fn close_photo(&mut self, cx: &mut Context<Self>) {
        if let Some(v) = self.viewer.as_mut() {
            if v.closing {
                return;
            }
            v.closing = true;
            self.press_gen += 1;
            v.serial = self.press_gen;
            cx.notify();
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(420)).await;
                this.update(cx, |this, cx| {
                    if this.viewer.as_ref().is_some_and(|v| v.closing) {
                        this.viewer = None;
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
    }

    fn photos_view(&self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let vw: f32 = window.viewport_size().width.into();
        let cell = ((vw - TILE_GAP * 4.) / 3.).floor();

        let tiles: Vec<_> = (0..PHOTO_COUNT)
            .map(|ix| {
                let store = self.tile_bounds.clone();
                let tracker = canvas(
                    move |bounds, _, _| store.borrow_mut()[ix] = bounds,
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full();

                let base = div()
                    .id(("tile", ix))
                    .absolute()
                    .child(tracker);
                let tile: AnyElement = match self.press {
                    Some((pix, pressed, serial)) if pix == ix => base
                        .with_animation(("tile-anim", serial), anim(260), move |el, d| {
                            let g = PRESS_GROW * if pressed { d } else { 1. - d };
                            el.left(px(-g))
                                .top(px(-g))
                                .child(photo(ix, cell + 2. * g, cell + 2. * g, 4. + g))
                        })
                        .into_any_element(),
                    _ => base.child(photo(ix, cell, cell, 4.)).into_any_element(),
                };
                div()
                    .id(("cell", ix))
                    .relative()
                    .flex_none()
                    .size(px(cell))
                    .child(tile)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.press_gen += 1;
                            this.press = Some((ix, true, this.press_gen));
                            cx.notify();
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            if this.press.is_some_and(|(i, pressed, _)| i == ix && pressed) {
                                this.press_gen += 1;
                                this.press = Some((ix, false, this.press_gen));
                                cx.notify();
                            }
                        }),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.open_photo(ix, cx)))
                    .into_any_element()
            })
            .collect();

        div()
            .size_full()
            .flex()
            .flex_col()
            .pt(px(self.top.max(12.)))
            .child(
                div()
                    .px_5()
                    .pt(px(48.))
                    .pb_3()
                    .text_size(px(34.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.label)
                    .child("Photos"),
            )
            .child(
                div()
                    .id("photos")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(TILE_GAP))
                            .p(px(TILE_GAP))
                            .children(tiles),
                    )
                    .child(div().h(px(self.bottom.max(10.) + 90.))),
            )
    }

    fn viewer_overlay(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let v = self.viewer.as_ref()?;
        let (ix, from, closing) = (v.ix, v.from, v.closing);
        let vs = window.viewport_size();
        let (vw, vh): (f32, f32) = (vs.width.into(), vs.height.into());
        let (fx, fy): (f32, f32) = (from.origin.x.into(), from.origin.y.into());
        let (fw, fh): (f32, f32) = (from.size.width.into(), from.size.height.into());
        let (top, bottom) = (self.top, self.bottom);

        Some(
            div()
                .id(("viewer", v.serial))
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .occlude()
                .on_click(cx.listener(|this, _, _, cx| this.close_photo(cx)))
                .with_animation(("viewer-anim", v.serial), anim(if closing { 380 } else { 420 }), move |el, d| {
                    let t = if closing { 1. - d } else { d };
                    el.child(
                        div()
                            .absolute()
                            .size_full()
                            .bg(rgba(0x000000ff))
                            .opacity(t),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(lerp(fx, 0., t)))
                            .top(px(lerp(fy, 0., t)))
                            .child(photo(ix, lerp(fw, vw, t), lerp(fh, vh, t), lerp(4., 0., t))),
                    )
                    .child(
                        glass_div(GLASS_TINT_DARK, 18.)
                            .absolute()
                            .top(px(top.max(12.) + 8.))
                            .right(px(16.))
                            .opacity(t)
                            .h(px(36.))
                            .px_4()
                            .flex()
                            .items_center()
                            .text_color(rgb(0xffffff))
                            .text_size(px(17.))
                            .child("Done"),
                    )
                    .child(
                        div()
                            .absolute()
                            .bottom(px(bottom.max(10.) + 16.))
                            .w_full()
                            .flex()
                            .justify_center()
                            .opacity(t)
                            .text_color(rgb(0xffffff))
                            .text_size(px(15.))
                            .child(format!("Photo {}", ix + 1)),
                    )
                })
                .into_any_element(),
        )
    }

    fn row(&self, ix: usize, last: bool, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
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

impl Todos {
    fn reminders_view(&mut self, p: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
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
                self.glass(p, 26.)
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
                    self.glass(p, 18.)
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
            self.glass(p, 26.)
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
            .child(self.fps_badge())
    }
}

#[cfg(target_family = "wasm")]
mod web {
    use super::*;
    use std::borrow::Cow;
    use std::cell::RefCell;
    use wasm_bindgen::prelude::*;

    thread_local! {
        static HANDLE: RefCell<Option<ApplicationHandle>> = const { RefCell::new(None) };
    }

    /// `top`/`bottom` are the safe-area insets in CSS px; `dark` follows the OS appearance.
    #[wasm_bindgen]
    pub fn run(top: f32, bottom: f32, dark: bool, webgpu: bool) {
        gpui_kit::platform::web_init();
        let handle = gpui_kit::platform::application().run_embedded(move |cx| {
            // The web platform starts with no fonts; load them before opening a window.
            cx.text_system()
                .add_fonts(vec![
                    Cow::Borrowed(include_bytes!("../assets/Arial.ttf").as_slice()),
                    Cow::Borrowed(include_bytes!("../assets/Arial-Bold.ttf").as_slice()),
                ])
                .expect("failed to load fonts");
            gpui_kit::init(cx);
            if dark {
                gpui_kit::component::theme::Theme::change(
                    gpui_kit::component::theme::ThemeMode::Dark,
                    None,
                    cx,
                );
            }
            gpui_kit::open_window(WindowOptions::default(), cx, move |window, cx| {
                cx.new(|cx| Todos::new(top, bottom, dark, webgpu, window, cx))
            })
            .expect("failed to open window");
        });
        HANDLE.with(|h| *h.borrow_mut() = Some(handle));
    }
}
