use super::{Immagine, Screen};
use crate::components::glass::glass_div_faded;
use crate::theme::{Palette, BLUE};
use crate::util::{anim, lerp, now_ms};
use super::icons::{glyph, Glyph};
use gpui_kit::*;
use std::time::Duration;

pub(super) const TABS: [&str; 3] = ["Mediathek", "Sammlungen", "Personen"];

/// Horizontal margin between the screen edge and the first tab slot (bar margin 20 + bar padding 4).
const SIDE: f32 = 24.;
/// Geometry of the Jahre / Monate / Alle bar: pill height, gap to the round buttons, x of its first slot.
const RANGE_H: f32 = 56.;
const RANGE_GAP: f32 = 10.;
const RANGE_LEFT: f32 = 20. + RANGE_H + RANGE_GAP + 4.;
/// Length of the swap between the tab bar and the Jahre / Monate / Alle bar.
const SWAP_MS: f64 = 420.;
/// Distance from the newest photos (px) beyond which the range bar replaces the tab bar, and below which it leaves.
const RANGE_ENTER: f32 = 180.;
const RANGE_LEAVE: f32 = 40.;
/// After tapping the Mediathek button the tab bar stays until the list has moved this far from where it was.
const LATCH_SLOP: f32 = 60.;
const RANGES: [&str; 3] = ["Jahre", "Monate", "Alle"];

/// Progress of the swap between the tab bar and the range bar (see `Immagine::morph`).
struct Morph {
    s: f32,
    tabs: f32,
    icon: f32,
    range: f32,
}

/// The two bars with a glass thumb (see `Slider`).
#[derive(Clone, Copy)]
pub(super) enum Bar {
    Tabs,
    Range,
}

impl Bar {
    /// x of the first slot, slot width and slot count.
    fn geometry(self, vw: f32) -> (f32, f32, usize) {
        match self {
            Bar::Tabs => (SIDE, slot_width(vw), TABS.len()),
            Bar::Range => (RANGE_LEFT, range_slot(vw), RANGES.len()),
        }
    }
}

fn slot_width(viewport_w: f32) -> f32 {
    (viewport_w - 2. * SIDE) / TABS.len() as f32
}

fn range_slot(viewport_w: f32) -> f32 {
    (viewport_w - 40. - 2. * (RANGE_H + RANGE_GAP) - 8.) / RANGES.len() as f32
}

impl Immagine {
    pub(super) fn select_tab(&mut self, tab: usize, cx: &mut Context<Self>) {
        if tab != self.tab {
            self.tabs_slider.retarget(self.tab);
            self.tab = tab;
            self.tab_gen += 1;
            cx.notify();
        }
    }

    /// 1 normally; while a photo closes it rises 0 to 1 in step with the grid showing through.
    fn close_k(&self) -> f32 {
        match self.viewer.as_ref().filter(|v| v.closing) {
            Some(v) => {
                let x = ((now_ms() - v.close_at) / super::viewer::CLOSE_MS as f64).clamp(0., 1.) as f32;
                1. - (1. - x).powi(5)
            }
            None => 1.,
        }
    }

    /// Swaps the tab bar for the Jahre / Monate / Alle bar when the timeline is scrolled away from the newest
    /// photos, and back when it is at the bottom again.
    pub(super) fn sync_range_bar(&mut self, viewer_open: bool, cx: &mut Context<Self>) {
        if viewer_open {
            return;
        }
        let want = if self.tab != 0 || self.screen != Screen::Timeline {
            false
        } else if self.range_mode != 2 {
            // Jahre / Monate have no "newest end" to come back to: the bar stays until the Mediathek button.
            self.range_bar
        } else {
            let handle = self.timeline_scroll.0.borrow().base_handle.clone();
            let max: f32 = handle.max_offset().y.into();
            let off: f32 = handle.offset().y.into();
            let from_bottom = max + off;
            if max <= 0. || !self.timeline_placed.get() || now_ms() - self.scroll_settle.get() < 500. {
                self.range_bar
            } else if self.range_latch.is_some_and(|at| (from_bottom - at).abs() <= LATCH_SLOP) {
                false
            } else if self.range_bar {
                from_bottom > RANGE_LEAVE
            } else {
                from_bottom > RANGE_ENTER
            }
        };
        if self.range_latch.is_some() && !(self.range_latch.is_some_and(|at| self.timeline_from_bottom().is_some_and(|f| (f - at).abs() <= LATCH_SLOP))) {
            self.range_latch = None;
        }
        if want != self.range_bar {
            self.set_range_bar(want, cx);
        }
    }

    fn timeline_from_bottom(&self) -> Option<f32> {
        let handle = self.timeline_scroll.0.borrow().base_handle.clone();
        let max: f32 = handle.max_offset().y.into();
        (max > 0.).then(|| max + f32::from(handle.offset().y))
    }

    /// A paint-time probe for the timeline's scroll position: scrolling does not necessarily re-render the view,
    /// so the check also runs after every paint of the list.
    pub(super) fn range_probe(&self, cx: &mut Context<Self>) -> Canvas<()> {
        let view = cx.weak_entity();
        canvas(
            |_, _, _| {},
            move |_, _, window, cx| {
                let view = view.clone();
                window.defer(cx, move |_, cx| {
                    view.update(cx, |t, cx| {
                        let open = t.viewer.as_ref().is_some_and(|v| !v.closing);
                        t.sync_range_bar(open, cx);
                    })
                    .ok();
                });
            },
        )
        .absolute()
        .size_full()
    }

    fn set_range_bar(&mut self, on: bool, cx: &mut Context<Self>) {
        self.range_bar = on;
        self.swap_at = now_ms();
        self.swap_gen += 1;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(SWAP_MS as u64 + 40)).await;
            this.update(cx, |_, cx| cx.notify()).ok();
        })
        .detach();
    }

    pub(super) fn select_range(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix != self.range_mode {
            self.range_slider.retarget(self.range_mode);
            self.range_mode = ix;
            cx.notify();
        }
    }

    /// The Mediathek button: back to the tab bar without moving the list; it stays until the list is scrolled on.
    fn to_newest(&mut self, cx: &mut Context<Self>) {
        self.select_range(2, cx);
        self.range_latch = self.timeline_from_bottom();
        self.set_range_bar(false, cx);
    }

    /// The morph between the bars: how far the pill has shrunk (0 = tab bar, 1 = Mediathek circle) and how
    /// visible the tab contents, the circle's icon and the rest of the range bar are. Going to the range bar the
    /// contents leave first, the pill shrinks empty and the rest arrives last; going back is the mirror image.
    fn morph(&self) -> Morph {
        let t = ((now_ms() - self.swap_at) / SWAP_MS).clamp(0., 1.) as f32;
        let e = 1. - (1. - t).powi(3);
        if self.range_bar {
            Morph {
                s: e,
                tabs: (1. - t / 0.2).clamp(0., 1.),
                icon: ((t - 0.55) / 0.25).clamp(0., 1.),
                range: ((t - 0.8) / 0.2).clamp(0., 1.),
            }
        } else {
            let s = 1. - e;
            Morph { s, tabs: (1. - 2. * s).clamp(0., 1.), icon: ((s - 0.5) * 2.).clamp(0., 1.), range: ((s - 0.5) / 0.5).clamp(0., 1.) }
        }
    }

    /// The bar(s) at the bottom: the tab bar or the range bar. While they swap, the tab bar shrinks into the
    /// Mediathek circle (or grows out of it) and the rest of the range bar fades in (or out).
    pub(super) fn bars(&self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let swapping = now_ms() - self.swap_at < SWAP_MS;
        let mut out = Vec::new();
        if swapping {
            // Keeps frames coming while the morph runs, which re-renders the view.
            out.push(
                div()
                    .id(("swap-tick", self.swap_gen))
                    .absolute()
                    .size(px(0.))
                    .with_animation(("swap-tick-anim", self.swap_gen), anim(SWAP_MS as u64), |el, _| el)
                    .into_any_element(),
            );
            out.push(self.range_bar_view(p, window, false, cx));
            out.push(self.tab_bar(p, window, cx));
        } else if self.range_bar {
            out.push(self.range_bar_view(p, window, true, cx));
        } else {
            out.push(self.tab_bar(p, window, cx));
        }
        out
    }

    fn range_bar_view(&self, p: &Palette, window: &Window, with_circle: bool, cx: &mut Context<Self>) -> AnyElement {
        const H: f32 = RANGE_H;
        let vw: f32 = window.viewport_size().width.into();
        // The circle is drawn by the shrinking tab bar while the bars swap; the rest fades in over the second part.
        let k = self.close_k() * self.morph().range;
        let bottom = self.bottom.max(10.);
        let slot = range_slot(vw);
        let thumb = self.range_slider.thumb(("range-thumb", "range-thumb-anim"), self.range_mode, slot, H - 8., 24., p, k);
        let shown = self.range_slider.shown(self.range_mode);
        let label = Rgba { a: p.label.a * k, ..p.label };
        let option = |ix: usize| {
            div()
                .flex_1()
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(18.))
                .font_weight(if shown == ix { FontWeight::BOLD } else { FontWeight::MEDIUM })
                .text_color(label)
                .child(RANGES[ix])
        };
        let round = |id: &'static str| glass_div_faded(p.bar_tint, H / 2., k).id(id).size(px(H)).flex_none().flex().items_center().justify_center();
        let hole: Rgba = if self.dark { rgb(0x000000).into() } else { rgb(0xffffff).into() };
        div()
            .id("rangebar")
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .h(px(64. + bottom + 14.))
            .occlude()
            .child(
                div()
                    .absolute()
                    .left(px(20.))
                    .right(px(20.))
                    .bottom(px(bottom + 4.))
                    .flex()
                    .items_center()
                    .gap(px(RANGE_GAP))
                    .child(if with_circle {
                        round("range-library")
                            .on_click(cx.listener(|this, _, _, cx| this.to_newest(cx)))
                            .child(photo_icon(rgb(BLUE).into(), hole))
                            .into_any_element()
                    } else {
                        div().size(px(H)).flex_none().into_any_element()
                    })
                    .child(
                        glass_div_faded(p.bar_tint, H / 2., k)
                            .flex_1()
                            .h(px(H))
                            .p(px(4.))
                            .child(
                                div()
                                    .relative()
                                    .size_full()
                                    .child(thumb)
                                    .child(div().absolute().top_0().left_0().size_full().flex().children((0..RANGES.len()).map(option))),
                            )
                            // Not while the bars swap: the tab bar's layer owns the press then.
                            .children(with_circle.then(|| input_layer(cx.weak_entity(), Bar::Range, true))),
                    )
                    .child(round("range-search").child(glyph(Glyph::Search, 26., label))),
            )
            .into_any_element()
    }

    fn slider_begin(&mut self, bar: Bar, x: f32, viewport_w: f32, cx: &mut Context<Self>) {
        let (left, slot, _) = bar.geometry(viewport_w);
        match bar {
            Bar::Tabs => self.tabs_slider.begin(self.tab, x, left, slot),
            Bar::Range => self.range_slider.begin(self.range_mode, x, left, slot),
        }
        cx.notify();
    }

    fn slider_move(&mut self, bar: Bar, x: f32, viewport_w: f32, cx: &mut Context<Self>) {
        let (left, slot, n) = bar.geometry(viewport_w);
        let slider = match bar {
            Bar::Tabs => &mut self.tabs_slider,
            Bar::Range => &mut self.range_slider,
        };
        if slider.drag_to(x, left, slot, n) {
            cx.notify();
        }
    }

    fn slider_end(&mut self, bar: Bar, x: f32, viewport_w: f32, cx: &mut Context<Self>) {
        let (left, slot, n) = bar.geometry(viewport_w);
        match bar {
            Bar::Tabs => {
                let Some(target) = self.tabs_slider.end(self.tab, x, left, slot, n) else { return };
                if target != self.tab {
                    self.tab = target;
                    self.tab_gen += 1;
                }
            }
            Bar::Range => {
                let Some(target) = self.range_slider.end(self.range_mode, x, left, slot, n) else { return };
                self.range_mode = target;
            }
        }
        cx.notify();
    }

    pub(super) fn tab_bar(&self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let slot = slot_width(window.viewport_size().width.into());
        // While a photo closes the bar appears in step with the grid showing through: the same ease-out as the
        // viewer's backdrop. Glass has no opacity, so its effects and the contents are scaled by `k`.
        let k = self.close_k();
        // Morphing into the range bar: the pill shrinks to the Mediathek circle, its contents fade out first.
        let Morph { s, tabs, icon, .. } = self.morph();
        let kc = k * tabs;
        let fade = |c: Rgba| Rgba { a: c.a * kc, ..c };
        let thumb = self.tabs_slider.thumb(("thumb", "thumb-anim"), self.tab, slot, 56., 28., p, kc);
        let shown = self.tabs_slider.shown(self.tab);

        let color = |ix: usize| -> Rgba { fade(if shown == ix { rgb(BLUE).into() } else { p.tab_inactive }) };
        // Apple's Mediathek / Sammlungen glyphs: filled shapes, with the details cut out in the screen color.
        let hole: Rgba = if self.dark { rgb(0x000000).into() } else { rgb(0xffffff).into() };
        let photo_icon = |c: Rgba| photo_icon(c, hole);
        let album_icon = |c: Rgba| {
            div()
                .size(px(26.))
                .relative()
                .child(div().absolute().top_0().left(px(5.)).w(px(16.)).h(px(3.)).rounded(px(1.5)).bg(Rgba { a: c.a * 0.45, ..c }))
                .child(div().absolute().top(px(4.5)).left(px(2.5)).w(px(21.)).h(px(3.5)).rounded(px(2.)).bg(Rgba { a: c.a * 0.7, ..c }))
                .child(div().absolute().top(px(9.)).left_0().w(px(26.)).h(px(17.)).rounded(px(4.5)).bg(c))
        };
        let person_icon = |c: Rgba| {
            div()
                .size(px(26.))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(2.))
                .child(div().size(px(11.)).rounded_full().bg(c))
                .child(div().w(px(22.)).h(px(11.)).rounded_t(px(11.)).bg(c))
        };

        // The tap target is much larger than the pill: it reaches 14 px above it and down to the screen edge, and
        // selects on press so a slightly moving finger does not lose the tap.
        let bottom = self.bottom.max(10.);
        let item = |ix: usize, icon: Div| {
            let active = shown == ix;
            div()
                .flex_1()
                .h_full()
                .flex()
                .flex_col()
                .justify_end()
                .pb(px(bottom))
                .child(
                    div()
                        .h(px(64.))
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(3.))
                        .child(icon)
                        .child(
                            div()
                                .text_size(px(11.))
                                .font_weight(if active { FontWeight::BOLD } else { FontWeight::MEDIUM })
                                .text_color(color(ix))
                                .child(TABS[ix]),
                        ),
                )
        };

        let wrapper = div().id("tabbar").absolute().left_0().right_0().bottom_0().h(px(64. + bottom + 14.)).occlude();
        let vw: f32 = window.viewport_size().width.into();
        let wrapper = wrapper
            .child(
                div().absolute().left(px(20.)).w(px(lerp(vw - 40., 56., s))).bottom(px(lerp(bottom, bottom + 4., s))).child(
                    glass_div_faded(p.bar_tint, lerp(32., 28., s), k)
                        .h(px(lerp(64., 56., s)))
                        .p(px(4.))
                        .child(div().relative().size_full().child(thumb))
                        .children((icon > 0.).then(|| {
                            let a = k * icon;
                            div().absolute().top_0().left_0().size_full().flex().items_center().justify_center().child(photo_icon(Rgba { a, ..rgb(BLUE).into() }))
                        })),
                ),
            )
            .child(
                div()
                    .absolute()
                    .left(px(24.))
                    .right(px(24.))
                    .top_0()
                    .bottom_0()
                    .flex()
                    .child(item(0, photo_icon(color(0))))
                    .child(item(1, album_icon(color(1))))
                    .child(item(2, person_icon(color(2))))
                    .child(input_layer(cx.weak_entity(), Bar::Tabs, false)),
            );
        wrapper.into_any_element()
    }
}

/// Apple's Mediathek glyph: filled shapes, with the details cut out in the `hole` color.
fn photo_icon(c: Rgba, hole: Rgba) -> Div {
    // The cut-outs fade with the icon, otherwise they stay behind as a dark shape when it is invisible.
    let hole = Rgba { a: hole.a * c.a, ..hole };
    let back = Rgba { a: c.a * 0.5, ..c };
    div()
        .size(px(26.))
        .relative()
        .child(div().absolute().top_0().left(px(4.)).size(px(22.)).rounded(px(5.)).bg(back))
        .child(div().absolute().top(px(4.)).left_0().size(px(22.)).rounded(px(5.)).bg(c))
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let (x, y): (f32, f32) = (bounds.origin.x.into(), bounds.origin.y.into());
                    let at = |px_: f32, py_: f32| point(px(x + px_), px(y + py_));
                    let mut b = PathBuilder::fill();
                    b.move_to(at(3.5, 23.));
                    b.line_to(at(8.5, 16.));
                    b.line_to(at(11.5, 19.5));
                    b.line_to(at(15., 13.5));
                    b.line_to(at(18.5, 23.));
                    b.close();
                    if let Ok(path) = b.build() {
                        window.paint_path(path, hole);
                    }
                    let mut s = PathBuilder::fill();
                    let (cx_, cy_, r) = (15.5, 10., 2.6);
                    s.move_to(at(cx_ + r, cy_));
                    for i in 1..=16 {
                        let t = i as f32 / 16. * std::f32::consts::TAU;
                        s.line_to(at(cx_ + r * t.cos(), cy_ + r * t.sin()));
                    }
                    s.close();
                    if let Ok(path) = s.build() {
                        window.paint_path(path, hole);
                    }
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
}

/// Window-level pointer listeners for a bar's slider, registered at paint time. Touch goes through the touch-drag
/// claim (the press is ours from the first moment, so the bubble can swell before any movement); mouse uses
/// press, move and release. `bounded`: only presses inside the layer count; otherwise everything at or below its
/// top edge belongs to the bar.
fn input_layer(view: WeakEntity<Immagine>, bar: Bar, bounded: bool) -> Canvas<()> {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let top: f32 = bounds.origin.y.into();
            let mine = move |p: Point<Pixels>| if bounded { bounds.contains(&p) } else { f32::from(p.y) >= top };
            let touch_view = view.clone();
            window.on_mouse_event(move |e: &TouchDragEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let w: f32 = window.viewport_size().width.into();
                let x: f32 = e.position.x.into();
                match e.phase {
                    TouchPhase::Started if mine(e.start_position) => {
                        window.prevent_default();
                        touch_view.update(cx, |t, cx| t.slider_begin(bar, x, w, cx)).ok();
                    }
                    TouchPhase::Moved => {
                        touch_view.update(cx, |t, cx| t.slider_move(bar, x, w, cx)).ok();
                    }
                    TouchPhase::Ended | TouchPhase::Cancelled => {
                        touch_view.update(cx, |t, cx| t.slider_end(bar, x, w, cx)).ok();
                    }
                    _ => {}
                }
            });
            let down_view = view.clone();
            window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble && e.button == MouseButton::Left && mine(e.position) {
                    let w: f32 = window.viewport_size().width.into();
                    down_view.update(cx, |t, cx| t.slider_begin(bar, e.position.x.into(), w, cx)).ok();
                }
            });
            let move_view = view.clone();
            window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble && e.pressed_button == Some(MouseButton::Left) {
                    let w: f32 = window.viewport_size().width.into();
                    move_view.update(cx, |t, cx| t.slider_move(bar, e.position.x.into(), w, cx)).ok();
                }
            });
            let up_view = view.clone();
            window.on_mouse_event(move |e: &MouseUpEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble && e.button == MouseButton::Left {
                    let w: f32 = window.viewport_size().width.into();
                    up_view.update(cx, |t, cx| t.slider_end(bar, e.position.x.into(), w, cx)).ok();
                }
            });
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}
