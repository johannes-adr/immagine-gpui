// Precaches the app shell and the wasm bundle so launches never wait for the network.
// Bump VERSION whenever any precached file changes (the files are not content-hashed).
const VERSION = "v38";
const CACHE = "immagine-" + VERSION;
const ASSETS = [
  "./",
  "./index.html",
  "./manifest.webmanifest",
  "./pkg/gpui_web_demo.js",
  "./pkg/gpui_web_demo_bg.wasm",
  "./icons/icon-180.png",
  "./icons/icon-192.png",
  "./icons/icon-512.png",
];

self.addEventListener("install", (e) => {
  e.waitUntil(
    caches.open(CACHE)
      .then((c) => c.addAll(ASSETS.map((u) => new Request(u, { cache: "reload" }))))
      .then(() => self.skipWaiting()),
  );
});

self.addEventListener("activate", (e) => {
  e.waitUntil(
    caches.keys()
      .then((keys) => Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k))))
      .then(() => self.clients.claim()),
  );
});

self.addEventListener("fetch", (e) => {
  if (e.request.method !== "GET") return;
  // The Immich API may live on another origin (reached directly, with CORS): never touch it.
  if (new URL(e.request.url).origin !== self.location.origin) return;
  e.respondWith(
    caches.match(e.request, { ignoreSearch: true }).then((hit) => hit || fetch(e.request)).then(isolate),
  );
});

// COOP/COEP make the page crossOriginIsolated (SharedArrayBuffer, wasm threads). Added here too so a
// static host that does not send them still gets threads from the second launch on.
function isolate(r) {
  if (r.status === 0 || r.headers.get("Cross-Origin-Embedder-Policy")) return r;
  const headers = new Headers(r.headers);
  headers.set("Cross-Origin-Opener-Policy", "same-origin");
  headers.set("Cross-Origin-Embedder-Policy", "require-corp");
  return new Response(r.body, { status: r.status, statusText: r.statusText, headers });
}
