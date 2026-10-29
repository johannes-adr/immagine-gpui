//! Platform-independent port of Immagine's logic (models, client, store, sync, grid maths).
//!
//! Nothing in here may import `gpui`, `web-sys` or `wasm-bindgen`, so `cargo test --lib` runs natively.
//! Browser and UI code lives elsewhere and implements the traits in `ports`.
//! Each submodule is owned by one porting agent (see `docs/immagine-port-plan.md`).

pub mod client;
pub mod crop_geometry;
pub mod display_image;
pub mod grid;
pub mod iso_date;
pub mod json;
pub mod media_duration;
pub mod models;
pub mod ports;
pub mod rendition_cache;
pub mod store;
pub mod sync;
pub mod thumbhash;
pub mod viewer_images;
