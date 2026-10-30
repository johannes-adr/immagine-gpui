//! An iOS-style Reminders + Photos demo written with GPUI and compiled to WebAssembly.
//!
//! Layout follows the nesting of the UI: `app` is the root view (tab switching, background),
//! with the `tab_bar`, the `reminders` tab and the `photos` tab (grid and full-screen viewer)
//! below it; `components` holds reusable building blocks (glass, generated photos, FPS badge).

#[allow(dead_code)]
mod app;
mod components;
pub mod logic;
mod theme;
mod util;
pub mod host;
mod immagine;
