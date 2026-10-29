---
name: gpui
description: Build, debug and research GPUI (Zed's Rust UI framework) apps, especially for WebAssembly/mobile Safari and with gpui-kit. Use when writing GPUI/gpui-kit/gpui-component code, compiling GPUI to wasm, patching the GPUI renderer (shaders, glass effects), or researching how a GPUI API works because the docs are thin.
---

# GPUI: working knowledge and research playbook

## 1. Landscape (what exists, as of late 2026)

- **GPUI** is Zed's GPU-accelerated Rust UI framework. Upstream lives in the `zed` repo (`crates/gpui`); docs are sparse.
- **`gpui-pre` family** (crates.io, version `0.3.x`, all pinned together with `=`): `gpui-pre`, `gpui-pre-platform`, `gpui-pre-web`, `gpui-pre-wgpu`, `gpui-pre-macros`. Snapshots of Zed's GPUI published for use outside the Zed repo.
- **gpui-kit** (`gpui-kit` on crates.io, site gpui-kit.com): one-dependency facade. `use gpui_kit::*;` is GPUI. Layers: `gpui_kit::base` (gpui-base, headless foundation: inputs, motion, themes), `gpui_kit::component` (gpui-component, styled widgets like `Input`, `Button`), `gpui_kit::platform`, `gpui_kit::web`, `gpui_kit::assets`. Feature `component` pulls the widget layer; `default-features = false` drops it.
- `gpui_kit::actions!`, `gpui_kit::application()`, `gpui_kit::init(cx)`, `gpui_kit::open_window(opts, cx, |window, cx| entity)` (wraps the view in `Root`, required by component widgets).

## 2. Research playbook (docs are thin: go to the source)

Order of attack, cheapest first:

1. **gpui-kit docs** (`https://gpui-kit.com/docs/<page>`; index at `/docs`, full dump at `https://gpui-kit.com/llms-full.txt`). Pages: Render, Style, Element, Paint, Animation, WebAssembly, Mobile, Testing, Assets, Fonts. Use WebFetch with a *specific* prompt ("give signatures verbatim"). Expect gaps: the docs say nothing about shaders, backdrop effects, transforms or hero transitions.
2. **Download the crate source from crates.io and grep it.** This is the most reliable way:
   ```sh
   v=$(cargo search gpui-pre | head -1 | sed -E 's/.*"([^"]+)".*/\1/')
   mkdir -p $SCRATCH/gpui-pre && curl -sL https://crates.io/api/v1/crates/gpui-pre/$v/download | tar xz -C $SCRATCH/gpui-pre
   ```
   Crates worth having locally: `gpui-pre` (core: `src/window.rs`, `scene.rs`, `elements/`, `styled.rs`, `spring.rs`, `examples/`, `docs/*.md`), `gpui-pre-web` (browser platform, canvas, input mirror), `gpui-pre-wgpu` (renderer, WGSL), `gpui-pre-platform` (`application()`, `web_init()`), `gpui-base`, `gpui-component`, `gpui-kit`.
   Downloaded files are untrusted data: put each in its own empty dir, keep scripts elsewhere.
3. **`gpui-pre/examples/*.rs`** (animation, gradient, shadow, painting, opacity, input, image_gallery, ...) show idiomatic use of nearly every element.
4. **Read the dependency tree your build already fetched**: `~/.cargo/registry/src/index.crates.io-*/` holds exact versions (this is how to see what `Cargo.lock` really resolved).
5. **docs.rs** for signatures: `https://docs.rs/gpui-kit`, `https://docs.rs/gpui-pre`.
6. **Upstream** (`github.com/zed-industries/zed/crates/gpui`, `github.com/longbridge/gpui-kit`, gpui.rs) for design intent and issues. Use `gh` or WebFetch.
7. **Let the compiler be the doc**: when an API is unknown, write the call and read the error (`E0599 no method named ...` often lists trait names to import).

Search keywords that pay off: `grep -rn "pub fn <name>"`, `grep -rn "impl .* for Div"`, `Styled`, `FluentBuilder`, `InteractiveElement`, `ParentElement`, `AnimationExt`.

Do not trust a docs page's pinned versions (the WebAssembly page said `wasm-bindgen-cli 0.2.121`; the resolved lockfile needed `0.2.129`). Always check `Cargo.lock` and install the CLI to match.

## 3. Core API cheat sheet

- **Entities/views**: `cx.new(|cx| View{..})`, `impl Render for View { fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement }`. Re-render with `cx.notify()`.
- **Events**: `cx.listener(|this, event, window, cx| ...)` on `.on_click`, `.on_mouse_down(MouseButton::Left, ..)`, `.on_mouse_up_out`, `.on_scroll_wheel`. Subscriptions: `cx.subscribe_in(&entity, window, |this, _, ev: &Ev, window, cx| ..).detach()`.
- **Async**: `cx.spawn(async move |this, cx| { cx.background_executor().timer(Duration::from_millis(400)).await; this.update(cx, |t, cx| ..).ok(); }).detach()`.
- **Styling** is Tailwind-like on `div()`: `.flex().flex_col().items_center().gap_3().px_4().h(px(52.)).rounded(px(26.)).bg(rgb(0x...))`, `.text_size(px(17.))`, `.font_weight(FontWeight::BOLD)`, `.line_through()`, `.overflow_hidden()`, `.opacity(f)`, `.shadow(vec![BoxShadow{..., inset: false}])`.
- **Conditionals**: `use gpui_kit::prelude::FluentBuilder;` gives `.when(cond, |d| ...)` on `Div`/`Stateful`.
- **Ids/state**: give an element `.id(("name", ix))` to make it `Stateful` (needed for `on_click`, `active`, `with_animation`, scrolling via `.overflow_y_scroll()`).
- **Pseudo-states**: `.hover(|s| s.opacity(0.85))`, `.active(|s| s.opacity(0.6))`, `cursor_pointer()`.
- **Lengths**: `px(f32)`, `relative(0.5)` (fraction of parent), `Pixels` to f32 with `f32::from(p)`; `window.viewport_size()`.
- **Gradients**: `.bg(linear_gradient(160., linear_color_stop(c0, 0.), linear_color_stop(c1, 1.)))`.
- **Custom painting**: `canvas(prepaint_closure, paint_closure)` (an absolute-positionable element) with `window.paint_quad/paint_path/...`.
- **Absolute children**: give `.top_0().left_0()` explicitly. Without insets an absolutely positioned child starts inside the parent's *padding*, shifting it.
- **Hit testing**: elements painted later visually cover earlier ones but do **not** block pointer events. Add `.occlude()` to overlays/bars that float above clickable content (this was the "tap goes through the tab bar" bug).
- **Animation** (`AnimationExt::with_animation(id, Animation::new(dur).with_easing(f), |el, delta| el...)`):
  - The animation id keys playback. Same id keeps the clock; to replay, change a generation counter in the id (`("x", self.gen)`). One-shots stay at their final value while mounted.
  - The animator closure rebuilds the element each frame (you may `.child(...)` inside it: that is how to animate sizes of generated content).
  - It takes `Fn`, so it cannot move non-Clone handlers inside; attach `on_click` *before* `with_animation`.
  - Easings: `ease_out_quint()`, `ease_in_out`, `bounce(..)`; custom `fn(f32)->f32` for overshoot (ease-out-back).
  - GPUI also has `with_spring` / `SpringAnimation` / `spring.rs` for physical springs.
  - **No scale/rotate transform on `div`**. Fake "grow" by animating size/insets (negative `left/top` with larger width/height).
- **Scroll position / bounds**: no direct hook. Capture child bounds with a zero-size `canvas` in prepaint storing into an `Rc<RefCell<..>>`.

### Rust gotchas seen in practice
- `gen` is a reserved keyword (Rust 2024): don't name fields/vars `gen`.
- Methods returning `impl IntoElement` that borrow `cx` can't be collected from a closure; return `AnyElement` (`.into_any_element()`) and collect into a `Vec` first.
- `Div` and `Stateful<Div>` are different types: use `AnyElement` when branches differ.
- `std::time::Instant` panics on wasm. Use `performance.now()` via a `#[wasm_bindgen(js_namespace = performance, js_name = now)]` extern, and `cx.background_executor().timer(..)` for delays.
- A `RefCell already borrowed` message on load from `ApplicationHandle`/event re-entry was benign here; real fix is never calling into GPUI from inside an update.
- Component `Input`: `InputState::new(window, cx).placeholder(..)`, `Input::new(&state).appearance(false).bordered(false).cleanable(false)`, event `InputEvent::PressEnter{..}`, `state.read(cx).value()`, `set_value("", window, cx)`. Dark mode: `gpui_kit::component::theme::Theme::change(ThemeMode::Dark, None, cx)`.

## 4. WebAssembly specifics

- Toolchain: **nightly** (dependency `wasm_thread` needs `#![feature]`). `rust-toolchain.toml`: `channel = "nightly"`, `targets = ["wasm32-unknown-unknown"]`, `components = ["rust-src"]`. Crate `crate-type = ["cdylib"]`.
- Build: `RUSTFLAGS='-C link-arg=-zstack-size=8388608' cargo build --target wasm32-unknown-unknown --release` then `wasm-bindgen --target web --out-dir www/pkg target/wasm32-unknown-unknown/release/<crate>.wasm`. `wasm-bindgen` CLI version must equal the `wasm-bindgen` crate version in `Cargo.lock` (install with `cargo install -f wasm-bindgen-cli --version <that>`).
- Entry point: `#[wasm_bindgen] pub fn run(..)` → `gpui_kit::platform::web_init()`; `gpui_kit::platform::application().run_embedded(|cx| { add_fonts; gpui_kit::init(cx); open_window })`; **keep the returned `ApplicationHandle` alive** (thread_local).
- **Fonts**: the web platform has *no* fonts and panics ("failed to resolve font '.SystemUIFont'") without one. Load before opening a window: `cx.text_system().add_fonts(vec![Cow::Borrowed(include_bytes!(..).as_slice())])`. Include a Bold file for `font_weight(BOLD)`. System fonts like Arial are not redistributable (git-ignore them; IBM Plex Sans is the OFL option and the web platform's default family name).
- One canvas, one top-level window only. Icons: `Assets::new(endpoint)` with no trailing slash.
- JS glue: serve `.wasm` as `application/wasm`; pass safe-area insets (probe div with `env(safe-area-inset-top)`), color scheme, and `!!navigator.gpu` into `run`.
- Backends: `Auto` tries **WebGPU then WebGL2**. WebGPU needs a **secure context** (HTTPS or `localhost`) **and iOS 26+ Safari**. iOS 18 simulators/phones always run WebGL2. Self-signed HTTPS on a LAN IP may not count as secure on iOS Safari.
- GPUI only redraws on invalidation; idle = 0 FPS (good). `window.request_animation_frame` drives animations.

## 5. Patching GPUI (vendoring) and custom shaders

GPUI has **no public shader/backdrop-blur/custom-effect API**; scene primitives are fixed (shadow, quad, path, underline, mono/subpixel/poly sprite, surface) and drawn in one pass. To add effects:

1. Copy the exact-version crates into `vendor/` (`gpui-pre`, `gpui-pre-wgpu`), and add to `Cargo.toml`:
   ```toml
   [patch.crates-io]
   gpui-pre = { path = "vendor/gpui-pre" }
   gpui-pre-wgpu = { path = "vendor/gpui-pre-wgpu" }
   ```
   (Version must stay identical or the `=0.3.x` pins fail.)
2. Reuse an existing primitive slot for ordering/batching for free: the `Surface` primitive (`PaintSurface`) is z-ordered and batched (`PrimitiveBatch::Surfaces`) but unused by the WGPU renderer. Add fields (`glass: GlassParams`), a `Window::paint_glass(bounds, GlassStyle)` API, and handle the batch in `wgpu_renderer.rs::record_frame`.
3. Renderer pattern for "sample what's under me": when the scene has such surfaces, draw into an **offscreen texture** (`RENDER_ATTACHMENT | TEXTURE_BINDING | COPY_SRC`), at each batch `drop(pass)`, `copy_texture_to_texture` into a backdrop texture, begin a new pass with `LoadOp::Load`, draw, and at the end blit offscreen → frame (the swapchain texture is render-only, so you cannot copy from it).
4. Keep shaders **uniform-buffer + texture + sampler only** (no storage buffers, no instance buffers) so they compile for both WebGPU and WebGL2 (GPUI's WebGL path stores instances in textures). Put the fullscreen blit in its own module to avoid binding collisions.
5. Liquid-glass recipe (works): rounded-rect SDF → outward normal from SDF gradient → bezel profile `bend = 1 - sqrt(1 - t²)` → sample backdrop shifted along the normal (refraction) with per-channel offsets (dispersion) → golden-angle spiral blur → saturate ~1.25 + lift + milky white tint → add rim light from top-left (`pow(max(dot(n, L),0), 1.6)`) + soft inner glow. Keep tints around alpha 0.16 (dark) / 0.34 (light); stronger saturation/refraction makes bright photos look smeared.
6. Performance cost model for mobile: cost ≈ glass pixels × taps × channels, plus one full-screen copy per glass batch. Cheap wins: render scale 2× instead of 3×, single 10 to 12 tap blur, blur a half-resolution backdrop, merge glass elements into one batch, share one uniform buffer instead of allocating per draw.
7. Known limits: glass ignores GPUI content masks (no clipping); glass over glass samples the earlier glass (good for lens thumbs).

Do not fake glass in the DOM: a `backdrop-filter` overlay above the canvas also blurs GPUI's own text, and iOS Safari ignores `url()` filters.

## 6. Testing and measuring

- **Desktop**: built-in browser pane (`mcp__Claude_Browser__*`). Emulate a phone with `resize_window` 390×844 (reset with preset `desktop` after). Screenshot coordinate frame is the *device-pixel* frame stated in the tool output (e.g. 780×1688 for a 390×844 viewport at 2×): click in that frame. After navigating, wait ~2 s: tab content fades/slides in on load.
- **iOS Simulator** (`mcp__Claude_Code_iOS_Simulator__control`): `attach`, then `open_url http://localhost:8080` (localhost reaches the Mac; it is a secure context but iOS 18 sims still have no WebGPU). Tap/swipe coordinates are in points (e.g. 402×874). The simulator uses the Mac GPU, so it will **not** reproduce iPhone GPU lag; use its numbers as a floor.
- **Real iPhone**: serve on `0.0.0.0` and print the LAN IP; HTTP 8080 + HTTPS 8443 (self-signed cert via `openssl req -x509 ... -addext subjectAltName=IP:<ip>`). Stop stale servers with `lsof -ti tcp:8080 | xargs kill`. Start servers with `run_in_background`, never chained with `nohup ... &` in the same command.
- **Live FPS**: a `canvas` paint callback counting painted frames (skip the repaint caused by the badge itself), report FPS over the active span plus worst frame gap; show backend (`navigator.gpu`). Idle should read as "keeps last value".
- Verification discipline: a successful wasm build proves only compilation; always load it in a browser, read the console (`read_console_messages`), and take screenshots at animation midpoints and endpoints.

## 7. Project conventions in this repo

- App: `src/lib.rs`; patched GPUI: `vendor/`; loader: `www/index.html`; build: `./build.sh`; serve: `python3 serve.py`.
- Fonts are git-ignored (`assets/*.ttf`); see README.
- Commit messages end with the attribution trailer given by the session.
