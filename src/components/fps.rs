use crate::util::now_ms;
use gpui_kit::*;
use std::cell::Cell;
use std::rc::Rc;

/// Counts frames the renderer actually paints, so the FPS readout is the app's real frame rate.
#[derive(Default)]
pub(crate) struct FpsMeter {
    frames: Cell<u32>,
    first: Cell<f64>,
    last: Cell<f64>,
    worst_gap: Cell<f64>,
    // Set before the readout repaints itself so that repaint is not counted.
    skip: Cell<bool>,
}

impl FpsMeter {
    /// Once a second, returns the frame rate over the span in which frames were painted,
    /// so a half-second animation reads as its real rate, not diluted by idle time.
    /// `None` while idle: the caller keeps showing the last measurement.
    pub(crate) fn reading(&self) -> Option<(u32, u32)> {
        let frames = self.frames.replace(0);
        let (first, last, worst) = (self.first.get(), self.last.get(), self.worst_gap.replace(0.));
        if frames < 3 || last <= first {
            return None;
        }
        let fps = ((frames - 1) as f64 / ((last - first) / 1000.)).round() as u32;
        Some((fps, worst.round() as u32))
    }

    /// The next paint is the readout repainting itself; do not count it.
    pub(crate) fn skip_next_paint(&self) {
        self.skip.set(true);
    }

    fn record_paint(&self) {
        if self.skip.replace(false) {
            return;
        }
        let now = now_ms();
        if self.frames.get() == 0 {
            self.first.set(now);
        } else {
            self.worst_gap.set(self.worst_gap.get().max(now - self.last.get()));
        }
        self.last.set(now);
        self.frames.set(self.frames.get() + 1);
    }
}

/// The FPS readout in the top-left corner. Its `canvas` paint callback counts painted frames.
pub(crate) fn fps_badge(meter: Rc<FpsMeter>, top: f32, fps: u32, worst_ms: u32, webgpu: bool) -> impl IntoElement {
    div()
        .absolute()
        .left(px(20.))
        .top(px(top.max(12.) + 7.))
        .h(px(22.))
        .px_2()
        .rounded_full()
        .flex()
        .items_center()
        .bg(rgba(0x000000a0))
        .text_color(rgb(0xffffff))
        .text_size(px(12.))
        .child(canvas(|_, _, _| (), move |_, _, _, _| meter.record_paint()))
        .child(format!(
            "{} FPS · {} ms worst · {}",
            fps,
            worst_ms,
            if webgpu { "WebGPU" } else { "WebGL2" }
        ))
}
