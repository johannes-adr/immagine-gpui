#!/bin/sh
# Serve www/ on 0.0.0.0: HTTPS on :8443 (self-signed cert for the LAN IP) and plain HTTP on :8080.
# Frees the ports first. IMMICH_URL=... overrides the Immich server; PWA=1 enables the real service worker.
cd "$(dirname "$0")" || exit 1
for port in 8080 8443; do
  pids=$(lsof -ti tcp:$port) && kill $pids 2>/dev/null
done
exec python3 serve.py
