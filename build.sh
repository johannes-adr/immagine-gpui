#!/bin/sh
# Build the wasm module and generate JS bindings into www/pkg
set -e
RUSTFLAGS='-C link-arg=-zstack-size=8388608' cargo build --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir www/pkg target/wasm32-unknown-unknown/release/gpui_web_demo.wasm
