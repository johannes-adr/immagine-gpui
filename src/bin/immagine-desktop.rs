//! Native desktop build of the app (`cargo run --bin immagine-desktop`). On wasm this binary is empty;
//! the browser entry is `host::web::entry::run`.

#[cfg(not(target_family = "wasm"))]
fn main() {
    gpui_web_demo::host::desktop::entry::run();
}

#[cfg(target_family = "wasm")]
fn main() {}
