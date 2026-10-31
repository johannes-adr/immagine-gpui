//! Pinch and raw single-touch events from GPUI's touch observer (`gpui::set_touch_observer`, vendored in
//! `vendor/gpui-pre`). Only touch screens produce them (iOS); on macOS nothing ever arrives, mouse and
//! trackpad input reach the app through GPUI itself. Mirrors `host/web/pinch.rs`.

use crate::host::Pinch;
use gpui_kit::{TouchEvent, TouchId, TouchPhase};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Removes the observer when dropped.
pub struct GestureHandle;

impl Drop for GestureHandle {
    fn drop(&mut self) {
        gpui_kit::set_touch_observer(None);
    }
}

#[derive(Default)]
struct Touches {
    points: HashMap<TouchId, (f32, f32)>,
    distance: Option<f32>,
}

impl Touches {
    fn two(&self) -> Option<((f32, f32), (f32, f32))> {
        (self.points.len() == 2).then(|| {
            let mut it = self.points.values();
            (*it.next().unwrap(), *it.next().unwrap())
        })
    }
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

pub fn install(on_event: impl Fn(Pinch) + 'static) -> GestureHandle {
    let touches = Rc::new(RefCell::new(Touches::default()));
    gpui_kit::set_touch_observer(Some(Rc::new(move |e: &TouchEvent| {
        let pos = (f32::from(e.position.x), f32::from(e.position.y));
        let mut t = touches.borrow_mut();
        match e.phase {
            TouchPhase::Started => {
                t.points.insert(e.id, pos);
                if let Some((a, b)) = t.two() {
                    t.distance = Some(dist(a, b).max(1.));
                    drop(t);
                    on_event(Pinch::Begin);
                } else if t.points.len() == 1 {
                    drop(t);
                    on_event(Pinch::Down { x: pos.0, y: pos.1 });
                }
            }
            TouchPhase::Moved => {
                let Some(p) = t.points.get_mut(&e.id) else { return };
                *p = pos;
                if let (Some((a, b)), Some(last)) = (t.two(), t.distance) {
                    let d = dist(a, b).max(1.);
                    t.distance = Some(d);
                    drop(t);
                    on_event(Pinch::Move { ratio: d / last, x: (a.0 + b.0) / 2., y: (a.1 + b.1) / 2. });
                } else if t.points.len() == 1 {
                    drop(t);
                    on_event(Pinch::Drag { x: pos.0, y: pos.1 });
                }
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                let count = t.points.len();
                if t.points.remove(&e.id).is_none() {
                    return;
                }
                if count == 2 {
                    t.distance = None;
                    drop(t);
                    on_event(Pinch::End);
                } else if count == 1 {
                    drop(t);
                    on_event(if e.phase == TouchPhase::Cancelled { Pinch::Up { x: -1., y: -1. } } else { Pinch::Up { x: pos.0, y: pos.1 } });
                }
            }
        }
    })));
    GestureHandle
}
