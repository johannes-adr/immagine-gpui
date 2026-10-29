import http.client, http.server, os, socket, socketserver, ssl, subprocess, threading, urllib.parse

HTTP_PORT, HTTPS_PORT = 8080, 8443
CERT_DIR = "certs"

# Dev default: serve a service worker that deletes its caches and unregisters itself, so every reload
# gets fresh files. `PWA=1 python3 serve.py` serves the real, caching sw.js instead.
PWA = os.environ.get("PWA") == "1"
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
            # Close-delimited body: lets large originals/videos stream without buffering.
            self.send_header("Connection", "close")
            self.end_headers()
            self.close_connection = True
            while chunk := r.read(65536):
                self.wfile.write(chunk)
        except (OSError, http.client.HTTPException) as e:
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
        return super().send_head()

    def end_headers(self):
        self.send_header("Cache-Control", "no-cache")
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
