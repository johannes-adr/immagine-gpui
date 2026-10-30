//! The liquid-glass selection thumb shared by the tab bar and the Jahre / Monate / Alle bar: the thumb follows the
//! finger while it is dragged (swelling like a bubble), slides to the nearest slot on release, and a plain tap
//! selects the slot under the finger. The state lives here; each bar passes in its geometry (x of the first slot,
//! slot width, slot count) and which slot is selected.

use crate::components::glass::glass_div_faded;
use crate::theme::Palette;
use crate::util::{lerp, now_ms, spring_out};
use gpui_kit::*;
use std::time::Duration;

/// Finger travel below which a press still counts as a tap on the slot under it.
const TAP_SLOP: f32 = 6.;
/// Length of the thumb animation.
const THUMB_MS: f64 = 520.;

#[derive(Clone, Copy)]
struct Drag {
    /// Thumb position in slot units (0 = first slot, fractional while between slots).
    pos: f32,
    /// Finger x minus thumb center x at the moment of the press, so grabbing does not make the thumb jump.
    grab: f32,
    start_x: f32,
    moved: bool,
}

#[derive(Default)]
pub(super) struct Slider {
    /// Where the thumb started its last move (slot units) and the bubble size then.
    from: f32,
    bubble_from: f32,
    /// Keys the animation: changing it replays the slide.
    serial: u32,
    at: f64,
    drag: Option<Drag>,
}

impl Slider {
    fn bump(&mut self) {
        self.serial += 1;
        self.at = now_ms();
    }

    /// The selection changed from outside (not by dragging): slide from slot `from`.
    pub(super) fn retarget(&mut self, from: usize) {
        self.from = from as f32;
        self.bubble_from = 0.;
        self.bump();
    }

    /// The slot highlighted right now: the one under the thumb while dragging, else the selected one.
    pub(super) fn shown(&self, sel: usize) -> usize {
        match self.drag {
            Some(d) if d.moved => d.pos.round() as usize,
            _ => sel,
        }
    }

    /// Whether the press going now has travelled beyond a tap.
    pub(super) fn moved(&self) -> bool {
        self.drag.is_some_and(|d| d.moved)
    }

    pub(super) fn begin(&mut self, sel: usize, x: f32, left: f32, slot: f32) {
        let pos = sel as f32;
        self.drag = Some(Drag { pos, grab: x - (left + (pos + 0.5) * slot), start_x: x, moved: false });
        self.bump();
    }

    /// Returns whether the thumb moved (the bar has to redraw).
    pub(super) fn drag_to(&mut self, x: f32, left: f32, slot: f32, n: usize) -> bool {
        let Some(d) = self.drag.as_mut() else { return false };
        d.moved |= (x - d.start_x).abs() > TAP_SLOP;
        if d.moved {
            d.pos = ((x - d.grab - left) / slot - 0.5).clamp(0., (n - 1) as f32);
        }
        d.moved
    }

    /// Finishes the press and returns the slot to select, or None when no press was going.
    pub(super) fn end(&mut self, sel: usize, x: f32, left: f32, slot: f32, n: usize) -> Option<usize> {
        let d = self.drag.take()?;
        let last = n - 1;
        let (from, target) = if d.moved {
            (d.pos, (d.pos.round() as usize).min(last))
        } else {
            (sel as f32, (((x - left) / slot).floor().max(0.) as usize).min(last))
        };
        self.from = from;
        self.bubble_from = 1.;
        self.bump();
        Some(target)
    }

    /// The thumb: a glass pill `h` tall that slides between slots `slot` wide. `ids` are the element id and the
    /// animation id (they must differ between bars). `k` scales the glass and the thumb color down to nothing.
    pub(super) fn thumb(&self, ids: (&'static str, &'static str), sel: usize, slot: f32, h: f32, radius: f32, p: &Palette, k: f32) -> impl IntoElement {
        // While dragging, the thumb sits at the finger and the bubble swells; on release it slides to its slot
        // while the bubble shrinks. One animation keyed by `serial` drives both, and re-rendering during the drag
        // keeps the same animation running with the new captured position.
        let (pos0, pos1, b0, b1) = match self.drag {
            Some(d) => (d.pos, d.pos, 0., 1.),
            // A rebuilt bar must start at rest, not replay the slide.
            None if now_ms() - self.at > THUMB_MS => (sel as f32, sel as f32, 0., 0.),
            None => (self.from, sel as f32, self.bubble_from, 0.),
        };
        let fill = Rgba { a: p.tab_thumb.a * k, ..p.tab_thumb };
        glass_div_faded(p.bar_tint, radius, k)
            .id((ids.0, self.serial))
            .absolute()
            .bg(fill)
            .with_animation((ids.1, self.serial), Animation::new(Duration::from_millis(THUMB_MS as u64)).with_easing(spring_out), move |el, d| {
                let pos = lerp(pos0, pos1, d);
                let b = lerp(b0, b1, d).max(0.);
                let (dw, dh) = (18. * b, 14. * b);
                el.left(px(pos * slot - dw / 2.)).top(px(-dh / 2.)).w(px(slot + dw)).h(px(h + dh))
            })
    }
}
