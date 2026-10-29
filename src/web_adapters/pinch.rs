//! Two-finger pinch from touch pointer events. GPUI's web platform delivers single-pointer input only,
//! so this listens on the window (capture phase) and reports the distance ratio between the two touches.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::PointerEvent;

#[derive(Clone, Copy, Debug)]
pub enum Pinch {
    /// A second finger touched down.
    Begin,
    /// `ratio` is this move's change of the finger distance (1.0 = unchanged); `x`/`y` the midpoint in CSS px.
    Move { ratio: f32, x: f32, y: f32 },
    /// One of the two fingers lifted.
    End,
}

pub struct PinchHandle {
    listeners: Vec<(&'static str, Closure<dyn FnMut(PointerEvent)>)>,
}

impl Drop for PinchHandle {
    fn drop(&mut self) {
        if let Some(window) = web_sys::window() {
            for (name, closure) in &self.listeners {
                let _ = window.remove_event_listener_with_callback_and_bool(name, closure.as_ref().unchecked_ref(), true);
            }
        }
    }
}

#[derive(Default)]
struct Touches {
    points: HashMap<i32, (f64, f64)>,
    distance: Option<f64>,
}

impl Touches {
    fn two(&self) -> Option<((f64, f64), (f64, f64))> {
        (self.points.len() == 2).then(|| {
            let mut it = self.points.values();
            (*it.next().unwrap(), *it.next().unwrap())
        })
    }
}

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

pub fn install(on_pinch: impl Fn(Pinch) + 'static) -> Option<PinchHandle> {
    let window = web_sys::window()?;
    let on_pinch = Rc::new(on_pinch);
    let touches = Rc::new(RefCell::new(Touches::default()));
    let mut listeners: Vec<(&'static str, Closure<dyn FnMut(PointerEvent)>)> = Vec::new();

    let mut add = |name: &'static str, handler: Box<dyn FnMut(PointerEvent)>| {
        let closure = Closure::wrap(handler);
        let _ = window.add_event_listener_with_callback_and_bool(name, closure.as_ref().unchecked_ref(), true);
        listeners.push((name, closure));
    };

    {
        let (t, cb) = (touches.clone(), on_pinch.clone());
        add("pointerdown", Box::new(move |e: PointerEvent| {
            if e.pointer_type() != "touch" {
                return;
            }
            let mut t = t.borrow_mut();
            t.points.insert(e.pointer_id(), (e.client_x() as f64, e.client_y() as f64));
            if let Some((a, b)) = t.two() {
                t.distance = Some(dist(a, b).max(1.));
                drop(t);
                cb(Pinch::Begin);
            }
        }));
    }
    {
        let (t, cb) = (touches.clone(), on_pinch.clone());
        add("pointermove", Box::new(move |e: PointerEvent| {
            if e.pointer_type() != "touch" {
                return;
            }
            let mut t = t.borrow_mut();
            let Some(p) = t.points.get_mut(&e.pointer_id()) else { return };
            *p = (e.client_x() as f64, e.client_y() as f64);
            if let (Some((a, b)), Some(last)) = (t.two(), t.distance) {
                let d = dist(a, b).max(1.);
                t.distance = Some(d);
                drop(t);
                cb(Pinch::Move { ratio: (d / last) as f32, x: ((a.0 + b.0) / 2.) as f32, y: ((a.1 + b.1) / 2.) as f32 });
            }
        }));
    }
    for name in ["pointerup", "pointercancel"] {
        let (t, cb) = (touches.clone(), on_pinch.clone());
        add(name, Box::new(move |e: PointerEvent| {
            if e.pointer_type() != "touch" {
                return;
            }
            let mut t = t.borrow_mut();
            let was_pinching = t.points.len() == 2;
            t.points.remove(&e.pointer_id());
            if was_pinching {
                t.distance = None;
                drop(t);
                cb(Pinch::End);
            }
        }));
    }
    Some(PinchHandle { listeners })
}
