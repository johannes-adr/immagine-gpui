#!/bin/sh
# Build the wasm module, generate JS bindings into www/pkg, optimize and precompress.
# Needs: wasm-bindgen CLI (matching Cargo.lock), wasm-opt (binaryen), brotli.
set -e
RUSTFLAGS='-C link-arg=-zstack-size=8388608' cargo build --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir www/pkg target/wasm32-unknown-unknown/release/gpui_web_demo.wasm
W=www/pkg/gpui_web_demo_bg.wasm
if command -v wasm-opt >/dev/null; then
  # Explicit features: --all-features emits wasm that browsers reject ("unknown import kind").
  wasm-opt -O3 --enable-threads --enable-bulk-memory --enable-bulk-memory-opt --enable-sign-ext \
    --enable-mutable-globals --enable-nontrapping-float-to-int --enable-multivalue \
    --enable-reference-types --enable-simd --strip-debug --strip-producers "$W" -o "$W.opt" && mv "$W.opt" "$W"
fi
# serve.py picks these up (Content-Encoding: br / gzip).
for f in "$W" www/pkg/gpui_web_demo.js; do
  command -v brotli >/dev/null && brotli -f -q 11 -o "$f.br" "$f"
  gzip -9 -f -k "$f"
done
ls -l "$W"*
