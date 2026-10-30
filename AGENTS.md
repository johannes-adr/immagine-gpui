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
- If `serve.py` says "Address already in use", an old server holds the port: `lsof -ti tcp:8080 | xargs kill`. `PORT=8090 python3 serve.py` runs a second one (e.g. from a worktree; HTTPS is `PORT+363`).
- `assets/Arial.ttf` and `assets/Arial-Bold.ttf` are git-ignored but `include_bytes!`'d in `src/web.rs`; the build fails without them. The web platform ships no fonts and panics when none are loaded before the window opens.
- `build.sh` also runs `wasm-opt -O3` (binaryen) and precompresses wasm/js to `.br`/`.gz`; `brew install binaryen brotli`. Do not use `wasm-opt --all-features` (produces wasm browsers reject: "unknown import kind"); the explicit `--enable-*` list incl. `--enable-threads` is required.
- `assets/*.sub.ttf` (what the build embeds) are Latin subsets of the Arial files, made by `./subset-fonts.sh` (needs fonttools); they are git-ignored like the originals.
- Rebuild after any Rust or `vendor/` change; `www/index.html` changes need no rebuild.

## Architecture

**App (`src/`), organised by the nesting of the UI.**

```
src/
  lib.rs              module list only
  host/               THE platform seam (see "Platform rule" below)
    mod.rs              `Host` trait, shared types (`Decoded`, `Pinch`), `Current` chosen by cfg
    web/                wasm only: fetch, IndexedDB, localStorage, createImageBitmap, DOM video,
                        pointer events, selftest, `entry::run` (wasm_bindgen entry: loads fonts, opens the window)
    desktop/            non-wasm: ureq, files, `image` crate, `entry::run` (native window)
  bin/immagine-desktop.rs  native entry, calls `host::desktop::entry::run`
  theme.rs            colors, glass tints, Palette (light/dark)
  util.rs             lerp, anim, spring_out, now_ms (delegates to `Host`)
  components/         reusable building blocks
    glass.rs            glass_div (liquid glass via Window::paint_glass)
    photo.rs            generated "photo" (gradient, sun, hills)
    fps.rs              FpsMeter and the FPS badge
  immagine/           the Immich photo app (root view `Immagine`, platform independent)
  app/                older Reminders + Photos demo (`Todos`), not wired into `run` any more
  logic/              pure logic, no gpui/platform imports
```

**Platform rule (hard):** platform dependencies enter the crate only through `src/host/`.
- Outside `src/host/` there is no `web_sys`, `js_sys`, `wasm_bindgen`, `wasm_bindgen_futures`, `ureq`, `std::fs`/`std::env` for app data, and no `#[cfg(target_family/target_os ...)]`. The one exception is `src/bin/immagine-desktop.rs`, which needs the cfg to compile as an empty `main` on wasm.
- `immagine/`, `app/`, `components/`, `util.rs` and `logic/` reach the platform through `host::Current` and the `Host` trait only (`use crate::host::{Current as H, Host}`). `logic/` imports not even that: it uses the traits in `logic/ports.rs`.
- Missing capability? Add a method to `Host` and implement it in **both** `host/web` and `host/desktop` (return `None`/no-op where a platform cannot do it, and say so in the module docs). Never `cfg` around a call site.
- Platform crates belong in the matching `[target.'cfg(...)'.dependencies]` block of `Cargo.toml`.
- Quick check: `grep -rnE 'cfg\((not\()?target_(family|os)|web_sys|wasm_bindgen|js_sys|ureq' src --include='*.rs' | grep -v '^src/host/'` must print only `src/bin/immagine-desktop.rs`.
- Renderer-level platform differences (WGPU vs Metal glass) live in `vendor/`, not in `src/`.

All tab code is `impl Todos` split across the child modules of `app`; the struct's private fields are visible to every descendant module, so methods that cross module boundaries are `pub(crate)`. `www/index.html` passes safe-area insets, color scheme and `!!navigator.gpu` to `run`.

**GPUI is vendored and patched** (`vendor/`, wired in `Cargo.toml` via `[patch.crates-io]`; versions must stay `0.3.10` because `gpui-kit` pins `=0.3.10`). GPUI has no backdrop blur or custom-shader hook, so the liquid-glass look is a renderer feature added here:

- `vendor/gpui-pre` (`scene.rs`, `window.rs`): adds `GlassParams`/`GlassStyle` and `Window::paint_glass(bounds, style)`. It reuses the `PaintSurface` primitive (already z-ordered and batched, previously a no-op on WGPU) with a new `glass` field.
- `vendor/gpui-pre-wgpu`: `glass.rs` + `glass.wgsl` + `glass_blit.wgsl`, hooked into `wgpu_renderer.rs` (`render_frame` creates/resizes the glass resources, `record_frame` handles the `Surfaces` batch). When a frame contains glass, everything is drawn into an offscreen texture; the backdrop is copied **once per frame** at the first glass batch; the offscreen texture is blitted to the real frame at the end.
- The glass shader uses only a uniform buffer, texture and sampler so it runs on WebGPU **and** WebGL2. iOS before 26 has no WebGPU, so WebGL2 is the path real iPhones usually take; check both.
- `vendor/gpui-pre-web` (`window.rs`): `MAX_RENDER_DPR = 2.0` caps the canvas backing-store scale (3x phones otherwise draw 2.25x the pixels).

Glass semantics to remember: later glass elements do not see content painted between glass batches; glass ignores GPUI content masks (no clipping when partly scrolled out); children of `glass_div` paint after the glass and stay crisp. The glass layer is an absolute `canvas` and needs explicit `top_0().left_0()`, otherwise it is offset by the parent's padding.

## Threads (web and desktop)

- The threading abstraction is GPUI's own: `cx.background_spawn(...)` / `cx.background_executor()`. Desktop: a native thread pool. Web: `vendor/gpui-pre-web/src/dispatcher.rs` starts `hardwareConcurrency` Web Workers via `wasm_thread` that share the wasm memory; results come back to the main thread through a mailbox woken by `Atomics.waitAsync`. It is platform-neutral gpui, so `immagine/` and `logic/` may use it without a `Host` method.
- Requirements, all in place: `build.sh` compiles with `+atomics,+bulk-memory,+mutable-globals`, `-Z build-std=std,panic_abort` and explicit linker args (`--shared-memory --import-memory --max-memory=1 GiB` plus the `__tls_*`/`__wasm_init_tls` exports; this nightly does not add them itself, without them the memory silently stays unshared). The page must be `crossOriginIsolated`: `serve.py` sends `COOP: same-origin` + `COEP: require-corp`, `sw.js` adds them to the responses it serves (static hosts get threads from the second launch).
- Without isolation (`COI=0 python3 serve.py`, same build) GPUI logs `falling back to single-threaded dispatcher` and the app runs on one thread. Check with `crossOriginIsolated` in the console and the 10 `using deprecated parameters for the initialization function` warnings (one per worker init).
- Verified: Chrome (browser pane), iOS 27 Safari (WebGPU) and iOS 18.4 Safari (prerequisites + app boots to login, WebGL2). Safari has no `COEP: credentialless`, so cross-origin subresources need CORS (our `fetch` calls do) or a CORP header.
- **How to offload work** (from a view method; the closure body runs on a worker, everything else on the main thread):
  ```rust
  self.serial += 1;
  let serial = self.serial;
  cx.spawn(async move |this, cx| {
      // Moved in by value; must be `Send + 'static`. Returns the result to the awaiting main-thread task.
      let parsed = cx.background_spawn(async move { parse(&bytes) }).await;
      let _ = this.update(cx, |t, cx| {
          if t.serial == serial { t.apply(parsed); cx.notify(); } // drop stale results
      });
  })
  .detach(); // a dropped `Task` is cancelled: `.detach()` it or keep it in a field
  ```
- **What may go into `background_spawn`:** pure Rust CPU work on owned, `Send` data: JSON parsing (`logic::json`, sync pages), store serialization (`MemoryStore` to/from bytes), `logic::range_groups`, grouping/sorting the timeline. `logic/` functions are the natural unit: they have no gpui or platform imports.
- **What may not:** anything holding `Rc`, `RefCell`, entities, `Window`/`App`/`Context`, or `Host` handles. `Host` calls (fetch, IndexedDB, localStorage, `createImageBitmap`, video, gestures) wrap JS objects that exist only on the main thread; `await` them in `cx.spawn`, then hand the bytes to `background_spawn`. Data held in `Rc<Vec<..>>` must be cloned into an owned `Vec` (or the field changed to `Arc`) before it can move; for very large data measure whether the clone costs more than the work.
- **Locks:** never block on the main thread. A contended `std::sync::Mutex::lock`/`Condvar`/`thread::join` there traps on the web (`memory.atomic.wait` is forbidden on the main thread) and stalls the UI on desktop. Pass data by value and return results through the task instead of sharing state.
- **It is not guaranteed to be parallel:** with the single-thread fallback (no isolation, old Safari) `background_spawn` runs on the main thread between frames. Split very long jobs (tens of ms) into chunks or yield, so the fallback does not freeze the UI.
- **Desktop I/O:** blocking I/O (ureq, `std::fs`) stays in `host/desktop/blocking.rs`, not in `background_spawn`; blocking the executor's pool threads starves other background tasks.
- **Timers:** `cx.background_executor().timer(d).await` inside `cx.spawn` only waits; it does not move the task off the main thread.
- Rendering and layout stay on the main thread, so threads only help once CPU work is moved off it. Boot times are unchanged (warm: first frame ~130-180 ms either way); the first cold load compiles the module per worker.

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

## Immagine port

Porting Robin's Immagine (`../immagine`, Swift/iOS, clone of github.com/RobinCoding/immagine) to this app; plan and agent split in `docs/immagine-port-plan.md`.

- **Platform seam:** `src/host/mod.rs` defines the `Host` trait (transport, clock, origin, settings, blob store, image decoding, video, touch gestures). `host::Current` is chosen by cfg: `host/web/` (wasm: fetch, IndexedDB, localStorage, createImageBitmap, DOM video, pointer events, `entry::run`) or `host/desktop/` (everything else: `ureq`, files under `IMMAGINE_DATA_DIR` / the user data dir, `image` crate; no video, no touch gestures). `immagine/`, `app/`, `logic/` use only `Host`; no `web_sys`/`wasm_bindgen` outside `host/web`. New platform feature = new `Host` method + both impls.
- Desktop build: `cargo run --bin immagine-desktop` (server from `IMMICH_URL`, default `http://immich.server.local`; 402x874 window). Glass on macOS/iOS comes from a Metal pass in the vendored `vendor/gpui-pre-apple` (`metal_renderer.rs` + `glass.metal`, which is `glass.wgsl` run through `naga` with the bindings set by hand: keep both in sync when the look changes). With glass in the frame the scene is drawn offscreen, copied once into a backdrop texture at the first glass batch and blitted to the drawable; `PaintSurface.image_buffer` is `Option` (None = glass). Glass look flag: `set_glass_variant(n)` (0 = original, 1 = Apple-like, the default, with edge lensing, clearer middle, specular rim, drop shadow; one shader, branch on `Params.extra.x`). Default is 1; `?glass=0` (web) / `IMMAGINE_GLASS=0` (desktop) selects the original. After editing `glass.wgsl` run `vendor/gpui-pre-apple/regen-glass.sh` (needs `cargo install naga-cli`). Light/dark follows `Window::appearance()` (also live). Desktop gaps: video playback and single-finger viewer drag (that comes from `Host::install_gestures`, which is `None` here).
- Platform-independent logic lives in `src/logic/` and must not import `gpui`, `web-sys` or `wasm-bindgen`. Test it natively with `cargo test --lib` (the crate is `rlib` + `cdylib`).
- The module is called `logic`, not `core`, to avoid clashing with the `core` crate.
- `serve.py` proxies `/api/*` to `IMMICH_URL` (default `http://immich.server.local`), so the page is same-origin. Credentials for E2E come from the environment (`E2E_SERVER_URL`, `E2E_EMAIL`, `E2E_PASSWORD`), never from files in the repo.

- Full knowledge dump of the port (architecture, Immich API, GPUI-web findings, performance numbers, gotchas, open items): `docs/immagine-wissensdump.md`.
- **Simulator Safari keeps old ES modules** across reloads of the same tab: after a rebuild it may fail with `LinkError: import function ./gpui_web_demo_bg.js:__wbg_... must be callable` (stale JS glue, new wasm). `serve.py` now sends `Cache-Control: no-store`; if it still happens, `xcrun simctl terminate booted com.apple.mobilesafari` and reopen the URL (add `?x=N`).
- Typing the Immich password into the app's login form is left to the user (the agent does not enter real passwords); to test the logged-in app in the browser pane, set `localStorage["immagine.token"]` from a `fetch('/api/auth/login')` response, the cookie comes with it. In the simulator there is no console, so the user logs in by hand once (the token persists in that Safari's localStorage).
- **Direct to Immich (no `serve.py`):** the login screen has a server field; the choice is saved as `immagine.server` and `Host::origin()` returns it (web: falls back to the page origin, desktop: falls back to `IMMICH_URL`). All requests carry the bearer token; videos on another origin are fetched into a blob URL. Needs CORS on the reverse proxy in front of Immich, snippet and limits in `docs/direct-immich.md`. The service worker ignores other-origin requests. Not yet tested against a CORS-enabled server.

## Search

- `src/immagine/search.rs`: opened by the lupe button at the right of the range bar (`tab_bar.rs`, "range-search"); while open, `search_bar` (glass field + close circle, rides above the keyboard via `Window::visual_viewport_bounds`) replaces the tab/range bars. Tap handlers call `focus_search` (`Window::request_virtual_keyboard`): the web platform only summons the iOS keyboard for taps on the text itself, so taps on the pill or the lupe must request it explicitly (`Immagine::search`, `open_search`/`close_search`; shown instead of the timeline while `search.open` on tab 0). Text field with 400 ms debounce (Enter searches at once), modes **Kontext** (`ImmichClient::smart_search`, `POST /api/search/smart`, CLIP, relevance order) and **Dateiname** (`filename_search`, `POST /api/search/metadata` with `originalFileName`), media filter Alle/Fotos/Videos. Results are not stored; a `serial` drops stale answers. Not yet done (Immich has them): filters for people, places, camera, date, favorites, and the explore/suggestions page.
- Immich reference for server DTOs and the mobile app: sparse-clone `github.com/immich-app/immich` (`git clone --depth 1 --filter=blob:none --sparse`, then `git sparse-checkout set --skip-checks server/src/dtos/search.dto.ts mobile/lib/models/search`) into a scratch dir; search DTOs are in `server/src/dtos/search.dto.ts`.
- **Mediathek range views:** `range_mode` (0 Jahre, 1 Monate, 2 Alle, set by the range bar in `tab_bar.rs`) picks the view of tab 0. Jahre/Monate (`immagine/range_views.rs`) are derived from the in-memory timeline by `logic/range_groups.rs` (no `/timeline/buckets` calls; items carry `date_bucket`), cached by `Rc` identity in `range_cache`. Both are fixed-row-height `uniform_list`s laid out like `photo_grid` (spacer rows + shifted box). Month rows show the busiest day big plus up to three more days. Tapping a year jumps to Monate (`months_jump`), a day jumps to Alle (`timeline_jump`, handled in `photo_grid`). `sync_range_bar` never flips back to the tab bar while `range_mode != 2`; the Mediathek circle sets mode 2 and the tab bar.
- **Glass selection thumb** (`immagine/slider.rs`): one `Slider` (drag, bubble, slide, tap) shared by the tab bar (`Bar::Tabs`) and the Jahre/Monate/Alle bar (`Bar::Range`); each bar passes geometry via `Bar::geometry` and an `input_layer(view, bar, bounded)`. Do not add a second thumb implementation. A glass with `k <= 0` paints nothing (`glass_div_with`): a faded glass would otherwise still blit the backdrop snapshot over earlier glass (the tab bar's invisible thumb wiped the range bar's left part during the swap).

## Timeline grid zoom

- Levels `ZOOM_LEVELS = [1, 3, 5, 15]` (`logic/grid/layout.rs`). Default 3 columns below 700 pt width, 5 above (`grid_default`, follows the width until the user zooms; the width is not final at the first frame). `grid_cols` is the settled layout (photos per row), `grid_f` the live column count that sets the cell size. While pinching only the cell size changes (rows keep `grid_cols` photos, centered, so nothing reflows and the grid scales about the screen middle); reflowing on every column-count step flickered badly. Pinch input: touch via `Host::install_gestures`; desktop `PinchEvent` and ctrl+wheel through `grid_input` (capture-phase window listeners; gpui negates the browser's `deltaY`, so pinch-out is dy > 0). On release `grid_settle` picks the level (next level only past `SNAP_AT` of the way in log scale, else springs back); `grid_tick` (from `render`) animates `grid_f` there, fades the photos out at the end, switches `grid_cols` and fades them back in. The anchor photo stays centered through a pixel-exact `set_offset` while zooming (`timeline_jump` every frame).
