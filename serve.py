import http.server, os, socket, socketserver, ssl, subprocess, threading

HTTP_PORT, HTTPS_PORT = 8080, 8443
CERT_DIR = "certs"

class H(http.server.SimpleHTTPRequestHandler):
    extensions_map = {**http.server.SimpleHTTPRequestHandler.extensions_map, ".wasm": "application/wasm"}
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
print(f"Desktop: http://localhost:{HTTP_PORT}")
print(f"iPhone:  https://{ip}:{HTTPS_PORT}   (same Wi-Fi; accept the certificate warning)", flush=True)
try:
    https_server.serve_forever()
except KeyboardInterrupt:
    pass
