# AGENTS.md

Guidance for coding agents (Claude Code and others) working in this repository. `CLAUDE.md` imports this file.

An iOS-style Reminders + Photos demo written with GPUI (via `gpui-kit`) and compiled to WebAssembly. There are no tests and no lint config; verification is done by loading the app in a browser or the iOS Simulator.

## Commands

```sh
cargo install wasm-bindgen-cli --version 0.2.129   # must match the wasm-bindgen version in Cargo.lock
./build.sh                                         # cargo build (wasm32, release) + wasm-bindgen into www/pkg/
python3 serve.py                                   # dev: service worker disabled (kill-switch). PWA=1 python3 serve.py for the real one
# http://localhost:8080 and https://<LAN-IP>:8443 (prints both)
```

- Needs **nightly** Rust (`rust-toolchain.toml`); stable fails inside the `wasm_thread` dependency.
- If `serve.py` says "Address already in use", an old server holds the port: `lsof -ti tcp:8080 | xargs kill`.
- `assets/Arial.ttf` and `assets/Arial-Bold.ttf` are git-ignored but `include_bytes!`'d in `src/web.rs`; the build fails without them. The web platform ships no fonts and panics when none are loaded before the window opens.
- `build.sh` also runs `wasm-opt -O3` (binaryen) and precompresses wasm/js to `.br`/`.gz`; `brew install binaryen brotli`. Do not use `wasm-opt --all-features` (produces wasm browsers reject: "unknown import kind"); the explicit `--enable-*` list incl. `--enable-threads` is required.
- `assets/*.sub.ttf` (what the build embeds) are Latin subsets of the Arial files, made by `./subset-fonts.sh` (needs fonttools); they are git-ignored like the originals.
- Rebuild after any Rust or `vendor/` change; `www/index.html` changes need no rebuild.

## Architecture

**App (`src/`), organised by the nesting of the UI.**

```
src/
  lib.rs              module list only
  web.rs              wasm entry: run(top, bottom, dark, webgpu) loads fonts, opens the window,
                      keeps the ApplicationHandle in a thread-local
  theme.rs            colors, glass tints, Palette (light/dark)
  util.rs             lerp, anim, spring_out, now_ms
  components/         reusable building blocks
    glass.rs            glass_div (liquid glass via Window::paint_glass)
    photo.rs            generated "photo" (gradient, sun, hills)
    fps.rs              FpsMeter and the FPS badge
  app/                root view `Todos`: state, tab switching, background, Render
    tab_bar.rs          floating glass tab bar
    reminders/          Reminders tab: header, list, composer (mod.rs), one row (row.rs)
    photos/             Photos tab: grid with press-grow (mod.rs), full-screen zoom overlay (viewer.rs)
```

All tab code is `impl Todos` split across the child modules of `app`; the struct's private fields are visible to every descendant module, so methods that cross module boundaries are `pub(crate)`. `www/index.html` passes safe-area insets, color scheme and `!!navigator.gpu` to `run`.

**GPUI is vendored and patched** (`vendor/`, wired in `Cargo.toml` via `[patch.crates-io]`; versions must stay `0.3.10` because `gpui-kit` pins `=0.3.10`). GPUI has no backdrop blur or custom-shader hook, so the liquid-glass look is a renderer feature added here:

- `vendor/gpui-pre` (`scene.rs`, `window.rs`): adds `GlassParams`/`GlassStyle` and `Window::paint_glass(bounds, style)`. It reuses the `PaintSurface` primitive (already z-ordered and batched, previously a no-op on WGPU) with a new `glass` field.
- `vendor/gpui-pre-wgpu`: `glass.rs` + `glass.wgsl` + `glass_blit.wgsl`, hooked into `wgpu_renderer.rs` (`render_frame` creates/resizes the glass resources, `record_frame` handles the `Surfaces` batch). When a frame contains glass, everything is drawn into an offscreen texture; the backdrop is copied **once per frame** at the first glass batch; the offscreen texture is blitted to the real frame at the end.
- The glass shader uses only a uniform buffer, texture and sampler so it runs on WebGPU **and** WebGL2. iOS before 26 has no WebGPU, so WebGL2 is the path real iPhones usually take; check both.
- `vendor/gpui-pre-web` (`window.rs`): `MAX_RENDER_DPR = 2.0` caps the canvas backing-store scale (3x phones otherwise draw 2.25x the pixels).

Glass semantics to remember: later glass elements do not see content painted between glass batches; glass ignores GPUI content masks (no clipping when partly scrolled out); children of `glass_div` paint after the glass and stay crisp. The glass layer is an absolute `canvas` and needs explicit `top_0().left_0()`, otherwise it is offset by the parent's padding.

## GPUI patterns used here

- `with_animation` playback is keyed by element id; replay by changing a generation counter in the id (`tab_gen`, `press_gen`/`serial`). Animator closures are `'static` and cannot borrow `self`, so copy values into them.
- Photo viewer zoom: each grid tile has an absolute `canvas` that writes its bounds into a shared `Rc<RefCell<Vec<Bounds>>>`; `open_photo` reads them and `viewer_overlay` lerps from that rect to the full viewport. Closing flips the animation direction and a spawned timer clears `viewer` afterwards.
- Row builders that capture `cx` return `AnyElement` (an `impl IntoElement` return borrows `cx` and fails to compile).
- `gen` is a reserved keyword in edition 2024, hence `serial`.
- Tab bar and composer call `.occlude()` so pointer events do not reach the photo grid underneath.
- The FPS badge counts frames actually painted (a `canvas` paint callback) over the active span, so it reads 0/stale when the app is idle; the app does not redraw when nothing animates.

## PWA / startup

- `www/manifest.webmanifest`, `www/icons/`, `www/sw.js`. The service worker precaches the shell, JS glue, wasm and icons (cache-first). The files are not content-hashed: **bump `VERSION` in `www/sw.js` whenever any precached file changes** (every rebuild of the wasm), otherwise installed clients keep the old app. A new worker activates immediately but the running page keeps the old files until the next launch.
- `www/index.html` starts the wasm `fetch` in parallel with the JS import, shows a CSS launch screen (`#splash`) until the first frame, and logs `[boot]` timings to the console.
- `serve.py` serves `.br`/`.gz` siblings with `Content-Encoding`. Rebuild with `build.sh` (not just cargo) so they are not stale. The release profile uses fat LTO, 1 codegen unit and `strip`, so a release build takes longer.
- Service workers need a secure context (localhost or trusted HTTPS); on the iPhone with the self-signed cert they may not register. The built-in browser pane refuses to register one, so SW behavior needs a real browser.

## iOS / browser quirks

- `www/index.html` pins `html, body` to `position: fixed; inset: 0; height: 100dvh`. Without it, Safari scrolls the document and collapses its toolbar, shifting the canvas and leaving a gap.
- Safari WebGPU is secure-context only (`https` or `localhost`); `serve.py` generates a self-signed cert for the current LAN IP into `certs/` (git-ignored).
- 60 FPS ceilings are browser/display limits, not GPUI: the web platform draws on `requestAnimationFrame` and `with_max_fps` is unset.
- For iOS Simulator testing, `localhost` works directly; tap coordinates are in points (402x874 for the iPhone 16 Pro). In the desktop browser pane, click coordinates follow the screenshot's reported frame, not CSS pixels.
- **Standalone PWA (installed from the Home Screen) viewport is one status-bar height short (WebKit bug 301108 per community reports).** In the iOS 18.4 and iOS 27 simulators `innerHeight` was 812 on an 874 pt screen (62 pt = top inset), and nothing `position: fixed` paints below it, so a blank band showed at the bottom. Only the very first launch right after adding the icon got the full 874; every later cold launch got 812. Failed: `100vh`/`100dvh`/`screen.height` on fixed `html`/`body`, un-pinning GPUI's canvas size in `resize()` (canvas was 874 tall and still not painted).
  **Fix that works (verified on both simulators over repeated cold launches, no scrolling):** in `@media (display-mode: standalone)` drop the fixed pinning: `html { position: static; height: auto; min-height: 100lvh }`, `body { position: relative; height: 100lvh; min-height: 100lvh }`. Then `innerHeight`, `html`, `body` and canvas are all 874. Not tested on a real iPhone. Full log of measurements, failed attempts and sources: `docs/ios-standalone-viewport.md`. Debugging tip: show `innerHeight`, `html`/`body` heights and the canvas rect in a temporary fixed `<div>`.

## Dev workflow notes

- `python3 serve.py` serves a self-unregistering `sw.js` so reloads are never stale; `PWA=1 python3 serve.py` serves the real caching service worker. With the real one, a new `VERSION` in `www/sw.js` only shows up on the second launch (cache-first), so bump it and launch twice.
- Testing the installed PWA in the simulator: open `http://localhost:8080` in Safari, Share, "Zum Home-Bildschirm", Hinzufügen (the share sheet needs a swipe up to reveal it; the name field is focused, so tapping the wrong spot types into it). Quit the app with `xcrun simctl terminate booted com.apple.webapp` (the web clip is not a listed app and swiping it away is unreliable). Delete a stale Home Screen icon by long-pressing it, then Remove/Löschen; `apple-*` metas are only read when adding.
- Simulators: iOS 18.4 (iPhone 16 Pro, WebGL2) and iOS 27 (iPhone 18 Pro, WebGPU) are both installed; pass `device` to the simulator tool. On iOS 26+ Safari the share sheet is behind the page-menu button (left of the URL) > Teilen > Mehr anzeigen > Zum Home-Bildschirm. The screenshot tool can lag one step behind a tap, so take a second screenshot before concluding.
- A screenshot that looks frozen (same FPS badge numbers after relaunch) means the app was resumed, not restarted.
- Debugging layout inside the PWA has no console: add a temporary fixed `<div>` that prints `innerHeight`, `screen.height` and the canvas rect, and remove it afterwards.
