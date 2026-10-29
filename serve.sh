#!/bin/sh
# One command for everything: check/install tools, build the wasm app, then serve www/ on 0.0.0.0
# (HTTPS :8443 with a self-signed cert for the LAN IP, plain HTTP :8080).
#   ./serve.sh            build + serve        SKIP_BUILD=1 ./serve.sh   serve only
# IMMICH_URL=... overrides the Immich server; PWA=1 enables the real service worker.
set -e
cd "$(dirname "$0")"
WB_VERSION=0.2.129

if [ -z "$SKIP_BUILD" ]; then
  command -v cargo >/dev/null || { echo "Rust fehlt: https://rustup.rs"; exit 1; }
  rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true
  if ! command -v wasm-bindgen >/dev/null || ! wasm-bindgen --version | grep -q "$WB_VERSION"; then
    echo "Installiere wasm-bindgen-cli $WB_VERSION ..."
    cargo install wasm-bindgen-cli --version "$WB_VERSION" --locked
  fi
  command -v wasm-opt >/dev/null || echo "Hinweis: 'brew install binaryen brotli' macht das Bundle kleiner (optional)."
  [ -f assets/Arial.ttf ] && [ -f assets/Arial-Bold.ttf ] || { echo "assets/Arial.ttf und Arial-Bold.ttf fehlen (siehe README, Abschnitt Fonts)"; exit 1; }
  [ -f assets/Arial.sub.ttf ] || ./subset-fonts.sh
  ./build.sh
fi

for port in 8080 8443; do
  pids=$(lsof -ti tcp:$port 2>/dev/null) && kill $pids 2>/dev/null || true
done
exec python3 serve.py
