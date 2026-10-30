import io
import http.client, http.server, os, socket, socketserver, ssl, subprocess, threading, urllib.parse

# PORT=8090 python3 serve.py runs a second server (e.g. from a worktree) next to the default one.
HTTP_PORT = int(os.environ.get("PORT", 8080))
HTTPS_PORT = HTTP_PORT + 363
CERT_DIR = "certs"

# Dev default: serve a service worker that deletes its caches and unregisters itself, so every reload
# gets fresh files. `PWA=1 python3 serve.py` serves the real, caching sw.js instead.
PWA = os.environ.get("PWA") == "1"
COI = os.environ.get("COI") != "0"
KILL_SW = b"""self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (e) => e.waitUntil((async () => {
  for (const k of await caches.keys()) await caches.delete(k);
  await self.registration.unregister();
  for (const c of await self.clients.matchAll()) c.navigate(c.url);
})()));
"""

# Same-origin proxy for the Immich API: /api/* is forwarded to IMMICH_URL, so the page needs no CORS
# and the https page can reach an http server (no mixed content).
IMMICH = urllib.parse.urlsplit(os.environ.get("IMMICH_URL", "http://immich.server.local"))
HOP = {"connection", "keep-alive", "transfer-encoding", "te", "trailer", "upgrade", "proxy-authorization", "proxy-authenticate", "host"}

class H(http.server.SimpleHTTPRequestHandler):
    # Keep-alive: static files and proxied responses with a length carry Content-Length; the close-delimited
    # proxy path sets close_connection itself.
    protocol_version = "HTTP/1.1"
    extensions_map = {**http.server.SimpleHTTPRequestHandler.extensions_map, ".wasm": "application/wasm"}

    def proxy(self):
        body = self.rfile.read(int(self.headers.get("Content-Length") or 0)) or None
        headers = {k: v for k, v in self.headers.items() if k.lower() not in HOP | {"content-length", "accept-encoding"}}
        conn_cls = http.client.HTTPSConnection if IMMICH.scheme == "https" else http.client.HTTPConnection
        conn = conn_cls(IMMICH.netloc, timeout=60)
        try:
            conn.request(self.command, self.path, body, headers)
            r = conn.getresponse()
            self.send_response(r.status)
            for k, v in r.getheaders():
                if k.lower() not in HOP | {"content-length"}:
                    self.send_header(k, v)
            length = r.getheader("Content-Length")
            chunked = "chunked" in (r.getheader("Transfer-Encoding") or "").lower()
            if length is not None and not chunked:
                # Known length: forward it and keep the client connection alive (a grid fires hundreds of thumbnail requests).
                self.send_header("Content-Length", length)
                self.end_headers()
                if self.command != "HEAD":
                    while chunk := r.read(65536):
                        self.wfile.write(chunk)
            else:
                # Close-delimited body: lets large originals/videos stream without buffering.
                self.send_header("Connection", "close")
                self.end_headers()
                self.close_connection = True
                while chunk := r.read(65536):
                    self.wfile.write(chunk)
        except (OSError, http.client.HTTPException) as e:
            self.close_connection = True
            if not self.wfile.closed:
                try:
                    self.send_error(502, f"Immich proxy: {e}")
                except OSError:
                    pass
        finally:
            conn.close()

    def do_POST(self): self.proxy() if self.path.startswith("/api/") else self.send_error(405)
    do_PUT = do_PATCH = do_DELETE = do_POST

    def do_GET(self):
        if self.path.startswith("/api/"):
            return self.proxy()
        super().do_GET()

    def do_HEAD(self):
        if self.path.startswith("/api/"):
            return self.proxy()
        super().do_HEAD()

    def send_head(self):
        if not PWA and self.path.split("?")[0] == "/sw.js":
            self.send_response(200)
            self.send_header("Content-Type", "text/javascript")
            self.send_header("Content-Length", str(len(KILL_SW)))
            self.end_headers()
            self.wfile.write(KILL_SW)
            return None
        # Serve precompressed <file>.br / <file>.gz (written by build.sh) when the client accepts them.
        path = self.translate_path(self.path)
        accept = self.headers.get("Accept-Encoding", "")
        if os.path.isfile(path):
            for enc, ext in (("br", ".br"), ("gzip", ".gz")):
                if enc in accept and os.path.isfile(path + ext):
                    f = open(path + ext, "rb")
                    self.send_response(200)
                    self.send_header("Content-Type", self.guess_type(path))
                    self.send_header("Content-Encoding", enc)
                    self.send_header("Content-Length", str(os.fstat(f.fileno()).st_size))
                    self.send_header("Vary", "Accept-Encoding")
                    self.end_headers()
                    return f
        rng = self.headers.get("Range", "")
        if rng.startswith("bytes=") and "," not in rng and os.path.isfile(path):
            # Single byte range of a static file (the video player streams by range); small files only, read whole.
            size = os.path.getsize(path)
            first, _, last = rng[6:].partition("-")
            try:
                start = int(first) if first else max(size - int(last), 0)
                end = min(int(last), size - 1) if first and last else size - 1
            except ValueError:
                start, end = 0, -1
            if start > end or start >= size:
                self.send_response(416)
                self.send_header("Content-Range", f"bytes */{size}")
                self.send_header("Content-Length", "0")
                self.end_headers()
                return None
            with open(path, "rb") as f:
                f.seek(start)
                data = f.read(end - start + 1)
            self.send_response(206)
            self.send_header("Content-Type", self.guess_type(path))
            self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Accept-Ranges", "bytes")
            self.end_headers()
            return io.BytesIO(data)
        return super().send_head()

    def end_headers(self):
        # Static files are never cached in dev. Proxied /api/ responses keep Immich's own Cache-Control (thumbnails
        # are cacheable for a day there), so the browser cache behaves like in production.
        if not self.path.startswith("/api/"):
            self.send_header("Cache-Control", "no-store")
        # Cross-origin isolation: SharedArrayBuffer, so the wasm threads (GPUI background executor) run.
        # COI=0 leaves it out: same build, GPUI falls back to one thread (for A/B comparisons).
        if COI:
            self.send_header("Cross-Origin-Opener-Policy", "same-origin")
            self.send_header("Cross-Origin-Embedder-Policy", "require-corp")
        super().end_headers()

class Server(socketserver.ThreadingMixIn, http.server.HTTPServer):
    allow_reuse_address = True
    daemon_threads = True

def lan_ip():
    # Connecting a UDP socket sends nothing; it just picks the outbound interface.
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        s.connect(("10.255.255.255", 1))
        return s.getsockname()[0]
    except OSError:
        return "127.0.0.1"
    finally:
        s.close()

def ensure_cert(ip):
    """Self-signed cert valid for localhost and the Mac's LAN IP, regenerated if the IP changes."""
    os.makedirs(CERT_DIR, exist_ok=True)
    cert, key, stamp = (f"{CERT_DIR}/{n}" for n in ("cert.pem", "key.pem", "ip.txt"))
    if os.path.exists(cert) and os.path.exists(stamp) and open(stamp).read() == ip:
        return cert, key
    subprocess.run([
        "openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "365",
        "-keyout", key, "-out", cert, "-subj", "/CN=gpui-web-demo",
        "-addext", f"subjectAltName=DNS:localhost,IP:127.0.0.1,IP:{ip}",
        "-addext", "extendedKeyUsage=serverAuth",
    ], check=True, capture_output=True)
    open(stamp, "w").write(ip)
    return cert, key

def serve(server):
    threading.Thread(target=server.serve_forever, daemon=True).start()

handler = lambda *a: H(*a, directory="www")
ip = lan_ip()
cert, key = ensure_cert(ip)

http_server = Server(("0.0.0.0", HTTP_PORT), handler)
https_server = Server(("0.0.0.0", HTTPS_PORT), handler)
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.load_cert_chain(cert, key)
https_server.socket = ctx.wrap_socket(https_server.socket, server_side=True)

serve(http_server)
print("Service worker: " + ("real (PWA=1)" if PWA else "disabled (set PWA=1 to enable)"))
print(f"Immich proxy: /api -> {IMMICH.geturl()}")
print(f"Desktop:http://localhost:{HTTP_PORT}")
print(f"iPhone:  https://{ip}:{HTTPS_PORT}   (same Wi-Fi; accept the certificate warning)", flush=True)
try:
    https_server.serve_forever()
except KeyboardInterrupt:
    pass
