//! A glass button that morphs into a glass card (and back): the rectangle and corner radius move from the
//! button to the card, the content fades in during the last 45 % and a dimmed scrim behind closes on tap. While
//! it morphs the caller hides the button itself (`Morph::active`), the card takes its place.
//!
//! Usage: keep one `Morph` (a `Copy` value) in the view, call `open(now_ms())` from the button's click handler
//! and `close(now_ms())` + `Morph::finish_after` from the scrim / the content's actions, replace the button with
//! an empty box of its size while `active()`, and add `overlay(...)` to the page while it returns `Some`.
//! The anchor can be anywhere (top right menu, bottom right confirmation): both rectangles are given in
//! screen coordinates.

use crate::components::button;
use crate::components::glass::glass_div_with;
use crate::components::scrim::scrim;
use crate::components::ease::lerp;
use gpui_kit::*;
use std::time::Duration;

/// Tint of a card that carries text (dark, nearly opaque).
pub(crate) const DIALOG_TINT: Hsla = Hsla { h: 0., s: 0., l: 0.12, a: 0.94 };

/// Length of the morph from the button to the card (and back).
pub(crate) const MORPH_MS: f64 = 340.;

/// Rectangle as (left, top, width, height).
pub(crate) type Rect = (f32, f32, f32, f32);

/// When the morph started and whether it is opening (else closing); `None` = at rest, only the button shows.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub(crate) struct Morph(Option<(f64, bool)>);

impl Morph {
    /// Button hidden, card (or its animation) shown.
    pub(crate) fn active(&self) -> bool {
        self.0.is_some()
    }

    /// Opening or fully open (not closing).
    pub(crate) fn is_open(&self) -> bool {
        matches!(self.0, Some((_, true)))
    }

    /// Starts opening; does nothing while active.
    pub(crate) fn open(&mut self, now: f64) -> bool {
        if self.0.is_some() {
            return false;
        }
        self.0 = Some((now, true));
        true
    }

    /// Starts closing; true when it did (so the caller schedules `finish_after`).
    pub(crate) fn close(&mut self, now: f64) -> bool {
        if !self.is_open() {
            return false;
        }
        self.0 = Some((now, false));
        true
    }

    /// Back to rest at once (the action ran, or the host view went away).
    pub(crate) fn reset(&mut self) {
        self.0 = None;
    }

    /// Eased progress, 0 = button, 1 = card; `None` at rest. The second value says the animation still runs.
    pub(crate) fn t(&self, now: f64) -> Option<(f32, bool)> {
        let (start, open) = self.0?;
        let e = ((now - start) / MORPH_MS).clamp(0., 1.) as f32;
        let eased = 1. - (1. - e).powi(3);
        Some((if open { eased } else { 1. - eased }, e < 1.))
    }

    /// After the closing animation: clears the state `field` points to (if it still closes) and redraws.
    pub(crate) fn finish_after<V: 'static>(cx: &mut Context<V>, field: fn(&mut V) -> Option<&mut Morph>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(MORPH_MS as u64 + 40)).await;
            this.update(cx, |t, cx| {
                if let Some(m) = field(t).filter(|m| matches!(m.0, Some((_, false)))) {
                    m.reset();
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }
}

/// Look of one morph.
pub(crate) struct Spec {
    /// Element id of the card; the scrim is `(id, 1)`.
    pub id: &'static str,
    pub anchor: Rect,
    pub card: Rect,
    pub tint: Hsla,
    pub anchor_radius: f32,
    pub card_radius: f32,
}

/// Scrim and morphing card. `content` is laid out in a box of the card's size at its top left and fades in over
/// the second half of the morph (it is dropped until then); its click handlers should only act while
/// `morph.is_open()`. `on_scrim` runs when the dimmed page is tapped. `None` while at rest.
pub(crate) fn overlay<V: 'static>(
    morph: Morph,
    spec: Spec,
    now: f64,
    window: &mut Window,
    content: AnyElement,
    on_scrim: fn(&mut V, &mut Context<V>),
    cx: &mut Context<V>,
) -> Option<AnyElement> {
    let (t, running) = morph.t(now)?;
    if running {
        window.request_animation_frame();
    }
    let (x0, y0, w0, h0) = spec.anchor;
    let (x1, y1, bw, bh) = spec.card;
    let alpha = ((t - 0.55) / 0.45).clamp(0., 1.);
    let mut card = glass_div_with(spec.tint, lerp(spec.anchor_radius, spec.card_radius, t), 1., true)
        .id(spec.id)
        .occlude()
        .absolute()
        .left(px(lerp(x0, x1, t)))
        .top(px(lerp(y0, y1, t)))
        .w(px(lerp(w0, bw, t)))
        .h(px(lerp(h0, bh, t)))
        .overflow_hidden();
    if alpha > 0. {
        card = card.child(div().absolute().left_0().top_0().w(px(bw)).h(px(bh)).opacity(alpha).child(content));
    }
    Some(
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                scrim((spec.id, 1usize), 0.35 * t)
                    // Without it a tap also reaches the photo layer and toggles the viewer's controls.
                    .occlude()
                    .on_click(cx.listener(move |this, _, _, cx| on_scrim(this, cx))),
            )
            .child(card)
            .into_any_element(),
    )
}

/// Content of a confirmation card: `message` above one full-width destructive button. `on_confirm` is attached
/// only when given (pass it while `Morph::is_open`, so a tap during the animation does nothing).
pub(crate) fn confirm_content(
    id: &'static str,
    message: impl Into<SharedString>,
    label: impl Into<SharedString>,
    text: Rgba,
    destructive: Rgba,
    on_confirm: Option<impl Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
) -> AnyElement {
    let mut confirm = button::filled(id, rgba(0xffffff1f), 54., 27., label).text_size(px(20.)).text_color(destructive);
    if let Some(f) = on_confirm {
        confirm = confirm.on_click(f);
    }
    div()
        .size_full()
        .p(px(20.))
        .flex()
        .flex_col()
        .justify_between()
        .child(div().text_size(px(17.)).text_color(text).child(message.into()))
        .child(confirm)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn open_then_close_then_rest() {
        let mut m = Morph::default();
        assert!(!m.active() && m.t(0.).is_none());
        assert!(m.open(100.) && m.is_open());
        assert!(!m.open(120.), "already active");
        assert_eq!(m.t(100.), Some((0., true)));
        assert_eq!(m.t(100. + MORPH_MS), Some((1., false)));
        assert!(m.close(1000.) && m.active() && !m.is_open());
        assert!(!m.close(1010.), "already closing");
        assert_eq!(m.t(1000.), Some((1., true)));
        assert_eq!(m.t(1000. + MORPH_MS), Some((0., false)));
        m.reset();
        assert!(!m.active());
    }
}
