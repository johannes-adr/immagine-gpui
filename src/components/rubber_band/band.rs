//! Rubber band (iOS-style overscroll) state machine. Pure math: the UI feeds it scroll deltas, the scroll position
//! of the list underneath and the clock, and reads back the offset past the end.
//!
//! Only a finger moves the band; the coasting that follows a fling is cancelled at an end and never adds to it.
//! A fling that runs into an end gives one small bounce instead.

/// How far content can be pulled past its ends.
const MAX_OVERSCROLL: f32 = 150.;
/// Spring-back duration, bounce duration to its peak.
const BOUNCE_MS: f64 = 380.;
const KICK_MS: f64 = 150.;

/// Phase of a touch scroll gesture (cancelled counts as ended).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Phase {
    Started,
    Moved,
    Ended,
}

#[derive(Clone, Copy, Default, PartialEq)]
enum Mode {
    #[default]
    Rest,
    /// The finger drags the band.
    Drag,
    /// Easing back to rest from `from`, started at `at`.
    Back { at: f64, from: f32 },
    /// A fling ran into the end: a small bounce of height `amp`, then back.
    Kick { at: f64, amp: f32 },
}

/// What the UI has to do after an event.
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct Reaction {
    /// The event was used up by the band; the list must not see it.
    pub used: bool,
    /// Stop the synthesized momentum of the current fling.
    pub cancel_momentum: bool,
    /// The band changed; redraw.
    pub notify: bool,
}

/// Offset past the end (positive pulls the content down) and what drives it.
#[derive(Clone, Copy, Default)]
pub(super) struct RubberBand {
    value: f32,
    down: bool,
    mode: Mode,
}

impl RubberBand {
    /// Current offset in pixels (0 at rest).
    pub fn value(&self) -> f32 {
        self.value
    }

    /// One touch scroll event, before the list sees it. `offset`/`max` are the list's scroll offset (<= 0) and its
    /// maximum scroll distance; `active` is false while another layer (e.g. the photo viewer) owns the gesture.
    pub fn on_scroll(&mut self, dy: f32, phase: Phase, offset: f32, max: f32, now: f64, active: bool) -> Reaction {
        if phase == Phase::Started {
            self.down = true;
        }
        // Momentum is synthesized after the finger lifted, so it arrives while `down` is already false.
        let finger = self.down;
        if phase == Phase::Ended {
            self.down = false;
        }
        if !active {
            return Reaction::default();
        }
        let outward = (offset >= -0.5 && dy > 0.) || (offset <= -max + 0.5 && dy < 0.);
        let mut r = Reaction::default();
        if finger {
            if self.value != 0. || (outward && dy != 0.) {
                // Pushing on gets harder the farther out it is; coming back is one to one.
                let moving_out = self.value == 0. || (self.value > 0.) == (dy > 0.);
                let step = if moving_out { dy * 0.5 / (1. + self.value.abs() / 90.) } else { dy };
                let next = self.value + step;
                self.value = if self.value != 0. && (next > 0.) != (self.value > 0.) { 0. } else { next.clamp(-MAX_OVERSCROLL, MAX_OVERSCROLL) };
                // Dragging takes over a band that was springing back. Pushed all the way in, the band is at rest
                // again: the finger scrolls the list from here, and a fling it ends with must coast on.
                self.mode = if self.value == 0. { Mode::Rest } else { Mode::Drag };
                r.used = true;
            }
            if !self.down && self.value != 0. {
                // The finger lifted with the band out: spring back. The fling it started is judged event by event
                // below: pushing on is cancelled, coming back toward the content just scrolls it.
                self.mode = Mode::Back { at: now, from: self.value };
                r.used = true;
            }
        } else if self.value != 0. || self.mode != Mode::Rest {
            // Coasting while the band is out or springing: pushing farther out is cancelled, coasting back toward
            // the content goes through to the list.
            let out = match self.mode {
                Mode::Back { from, .. } => from,
                Mode::Kick { amp, .. } => amp,
                _ => self.value,
            };
            if dy != 0. && (dy > 0.) == (out > 0.) {
                r.cancel_momentum = true;
                r.used = true;
            }
        } else if dy != 0. && (offset + dy > 0.5 || offset + dy < -max - 0.5) {
            // A fling reaches (or is at) an end with this step: one small bounce, scaled by its speed. The list still
            // takes the step, which it clamps at the end.
            r.cancel_momentum = true;
            self.mode = Mode::Kick { at: now, amp: (dy * 6.).abs().clamp(20., 90.) * dy.signum() };
            r.notify = true;
        }
        r.notify |= r.used;
        r
    }

    /// Advances the spring-back and bounce from the clock. Returns true while it needs more frames.
    pub fn tick(&mut self, now: f64) -> bool {
        match self.mode {
            Mode::Rest => return false,
            Mode::Drag if self.down => return false,
            Mode::Drag if self.value == 0. => {
                self.mode = Mode::Rest;
                return false;
            }
            Mode::Drag => self.mode = Mode::Back { at: now, from: self.value },
            Mode::Kick { at, amp } if now - at >= KICK_MS => self.mode = Mode::Back { at: at + KICK_MS, from: amp },
            _ => {}
        }
        // Done is decided by the clock, not by the value: a bounce starts at 0.
        let done;
        (self.value, done) = match self.mode {
            Mode::Kick { at, amp } => {
                let t = (((now - at) / KICK_MS) as f32).clamp(0., 1.);
                (amp * (1. - (1. - t).powi(2)), false)
            }
            Mode::Back { at, from } => {
                let t = ((now - at) / BOUNCE_MS).clamp(0., 1.) as f32;
                (from * (1. - t).powi(3), t >= 1.)
            }
            _ => (self.value, self.value == 0.),
        };
        if done {
            self.value = 0.;
            self.mode = Mode::Rest;
        }
        !done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX: f32 = 2000.;

    /// Pull the band out at the top, push it back in with the same finger, scroll on into the list and fling:
    /// the fling must reach the list and must not be cancelled.
    #[test]
    fn fling_after_band_pushed_back_in_coasts_on() {
        let mut b = RubberBand::default();
        let mut t = 0.;
        b.on_scroll(10., Phase::Started, 0., MAX, t, true);
        for _ in 0..5 {
            t += 16.;
            b.on_scroll(10., Phase::Moved, 0., MAX, t, true);
            b.tick(t);
        }
        assert!(b.value() > 0.);
        // Back in, then into the list (offset moves away from the top).
        let mut offset = 0.;
        for _ in 0..10 {
            t += 16.;
            let r = b.on_scroll(-20., Phase::Moved, offset, MAX, t, true);
            if !r.used {
                offset -= 20.;
            }
            b.tick(t);
        }
        assert_eq!(b.value(), 0.);
        assert!(offset < 0.);
        t += 16.;
        b.on_scroll(-20., Phase::Ended, offset, MAX, t, true);
        b.tick(t);
        // Momentum toward the content.
        for _ in 0..10 {
            t += 16.;
            let r = b.on_scroll(-15., Phase::Moved, offset, MAX, t, true);
            assert!(!r.used && !r.cancel_momentum);
            offset -= 15.;
            b.tick(t);
        }
        assert_eq!(b.value(), 0.);
    }

    /// A band released while out springs back to rest; a fling outward is cancelled meanwhile.
    #[test]
    fn released_band_springs_back() {
        let mut b = RubberBand::default();
        let mut t = 0.;
        b.on_scroll(30., Phase::Started, 0., MAX, t, true);
        t += 16.;
        b.on_scroll(30., Phase::Ended, 0., MAX, t, true);
        assert!(b.value() > 0.);
        t += 16.;
        let r = b.on_scroll(10., Phase::Moved, 0., MAX, t, true);
        assert!(r.cancel_momentum);
        while b.tick(t) {
            t += 16.;
        }
        assert_eq!(b.value(), 0.);
    }
}
