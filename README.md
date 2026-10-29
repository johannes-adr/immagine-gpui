# GPUI web demo: Reminders + Photos with liquid glass

A small iOS-style todo and photo app built with GPUI, compiled to WebAssembly.

- `src/lib.rs`: the app (reminders list, glass tab bar, photo grid, zoom-to-fullscreen viewer, FPS badge).
- `vendor/gpui-pre`, `vendor/gpui-pre-wgpu`: GPUI patched with a liquid-glass renderer pass
  (`Window::paint_glass`, shader in `vendor/gpui-pre-wgpu/src/glass.wgsl`). Wired via `[patch.crates-io]`.
- `www/index.html`: loader. `serve.py`: HTTP on 8080 and HTTPS on 8443 (self-signed cert for your LAN IP).

## Run

```sh
./serve.sh
```

Checks the tools (installs `wasm-bindgen-cli` if missing), builds the wasm app and serves `www/` on `0.0.0.0`:
HTTPS on `:8443` (self-signed cert for your LAN IP, use this on the phone) and HTTP on `:8080`. It frees the
ports first. `SKIP_BUILD=1 ./serve.sh` serves without rebuilding; `IMMICH_URL=...` points the `/api` proxy at
your Immich server. Manual steps: `./build.sh` then `python3 serve.py`.

Needs nightly Rust (see `rust-toolchain.toml`).

## Fonts

The web platform ships no fonts. Put `Arial.ttf` and `Arial-Bold.ttf` (or any fonts, edit the
`include_bytes!` paths in `src/lib.rs`) into `assets/`. They are git-ignored because system fonts
are not redistributable; IBM Plex Sans (OFL) is a good open replacement.

## Notes

- WebGPU needs iOS 26+ Safari; older iOS uses the WebGL2 path (the glass shader works on both).
- Glass does not honor GPUI clipping yet.
