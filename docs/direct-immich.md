# PWA ohne Python-Server (direkt zu Immich)

Die App speichert die beim Login eingegebene Server-Adresse (`localStorage["immagine.server"]`) und schickt alle
API-Requests dorthin, mit `Authorization: Bearer <token>` (Bilder per `fetch`, Videos als Blob-URL, weil `<video src>`
keinen Header senden kann). Ohne gespeicherte Adresse gilt die Origin der Seite (der `serve.py`-Proxy).

Immich selbst sendet keine CORS-Header, der Reverse Proxy davor muss sie setzen (openresty/nginx, `server`-Block von Immich):

```nginx
set $cors_origin "";
if ($http_origin ~* ^https://(deine-pwa-domain\.example)$) { set $cors_origin $http_origin; }

add_header Access-Control-Allow-Origin  $cors_origin always;
add_header Access-Control-Allow-Headers "Authorization, Content-Type, Range" always;
add_header Access-Control-Allow-Methods "GET, POST, PUT, PATCH, DELETE, OPTIONS" always;
add_header Access-Control-Expose-Headers "Content-Length, Content-Range" always;
add_header Vary Origin always;

location / {
    if ($request_method = OPTIONS) { return 204; }
    # ... bestehendes proxy_pass zu Immich
}
```

Randbedingungen:
- Die PWA kommt per HTTPS (Service Worker, WebGPU), also muss auch Immich per HTTPS erreichbar sein (sonst Mixed Content).
- Die PWA-Dateien (`www/`) kann jeder statische Host liefern; die Shell steht dank Service Worker auch offline bereit.
