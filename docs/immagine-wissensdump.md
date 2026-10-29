# Wissensdump: Immagine-Port nach gpui-web

Stand: 29.10.2026. Alles, was beim Portieren der iOS-App Immagine (Swift) in die GPUI-Webapp gelernt wurde. Ergänzt `AGENTS.md` (Arbeitsregeln) und `docs/immagine-port-plan.md` (Plan und Stand je Welle). Zugangsdaten stehen bewusst nirgends im Repo.

## 1. Überblick

- **Quelle:** `github.com/RobinCoding/immagine`, lokal in `../immagine`. Nativ iOS, ~23k Zeilen Swift (UIKit, SwiftUI, GRDB/SQLite), 13 XCTest-Dateien (~130 Tests, fast nur reine Logik) und 25 Simulator-UI-Tests.
- **Ziel:** Immich-Client als GPUI-Webapp (WASM), gegen deinen Server (`http://immich.server.local`, API unter `/api`; `/photos` ist nur die Weboberfläche).
- **Klonen:** SSH-Klon scheitert (kein GitHub-SSH-Key), HTTPS-Klon klappt (`gh` ist als `johannes-adr` eingeloggt).
- **Ergebnis bisher:** Login, Sync (26 377 Assets, 12 Alben in ~4,8 s, danach ~50 ms inkrementell), virtualisierte Mediathek (24 942 Fotos), Viewer (Zoom, Favorit, Info, Video), Alben, Personen (514). Läuft im Desktop-Browser-Pane und auf beiden iPhone-Simulatoren (Login-Screen), echter Test auf dem Handy steht aus.

## 2. Architektur

```
src/
  logic/            reine Logik, KEIN gpui/web-sys/wasm-bindgen, nativ testbar (cargo test --lib)
    ports.rs          HttpTransport (async, nicht Send), Clock, MockTransport, FixedClock, Method (Get/Post/Put/Patch/Delete)
    client.rs         ImmichClient<T: HttpTransport>, ~45 Endpunkte, ClientError
    store.rs          Trait Store (synchron) + MemoryStore (BTreeMaps, Timeline-Index, snapshot()/restore() als JSON)
    sync/             engine.rs (Outbox → Full Sync → Stream), stream.rs (NDJSON), outbox.rs (AlbumOps)
    models/           Asset, Album, AlbumTree, AlbumOp, Timeline, Server, Sync, Person, AssetDetails/Info/Edit
    grid/, crop_geometry, thumbhash, display_image, viewer_images, rendition_cache, iso_date, json, media_duration
  web_adapters/     nur wasm: fetch.rs (FetchTransport), idb.rs (IndexedDB kv), local_storage.rs,
                    image_loader.rs (fetch → createImageBitmap → Pixel), pinch.rs, video.rs, selftest.rs, WebClock
  immagine/         nur wasm, GPUI-Views: mod.rs (Root `Immagine`), login, timeline (photo_grid), viewer,
                    tab_bar, thumbs (Bildspeicher), albums, people
  app/              alte Reminders/Photos-Demo, nur noch Spender (#[allow(dead_code)])
```

- Das Logik-Modul heißt `logic`, nicht `core` (Namenskollision mit dem Rust-`core`-Crate). Crate-Typ ist `cdylib` + `rlib`.
- Native `cargo test --lib` kompiliert GPUI mit (~40 s beim ersten Mal), läuft danach schnell. Stand: 220 Tests grün. `cargo check --target wasm32-unknown-unknown --lib` ist der schnelle Kompilierungscheck für den UI-Teil.
- **Engine-Besitz:** `SyncEngine` wird für einen Sync aus dem `Option` in den Task *verschoben* und danach zurückgegeben. Die UI liest nie den Store, sondern eine Kopie (`items: Rc<Vec<TimelineItem>>`, `albums`). So gibt es keinen RefCell-Borrow über ein `await`.
- **Persistenz:** Nach jedem Sync mit Änderungen wird der `MemoryStore` als JSON-Snapshot in IndexedDB (`immagine`/`store`) geschrieben (~19,7 MB für 26k Assets). Start: Snapshot lesen (37 ms), Timeline sofort zeigen, dann Sync. Token in `localStorage` (`immagine.token`).

## 3. Immich-API und Authentifizierung

- `GET /api/server/ping` → `{"res":"pong"}`. Login: `POST /api/auth/login` → `accessToken` plus drei Cookies (`immich_access_token` HttpOnly, `immich_auth_type`, `immich_is_authenticated`).
- **Cookie-Auth ist der Schlüssel für Bilder:** GPUIs `img(url)` und `<video>` können keinen `Authorization`-Header setzen. Der Browser hängt das Cookie an alle gleich-originalen `/api/...`-Anfragen; deshalb läuft alles über den **same-origin Proxy** in `serve.py`. `ImmichClient` nutzt zusätzlich Bearer (Token aus `localStorage`).
- Wichtige Pfade: `POST /api/search/metadata` (Paging, `{page,size,order,type,isFavorite}`), `GET /api/assets/{id}`, `/api/assets/{id}/thumbnail?size=thumbnail|preview|fullsize`, `/api/assets/{id}/video/playback`, `/api/people`, `/api/people/{id}/thumbnail`, Album-Endpunkte (PATCH für Umbenennen/Cover), `POST /api/sync/stream` (NDJSON, Acks).
- Der Server ist Immich v3-Stil: `duration` kommt als Millisekunden-Zahl (ältere: Anzeige-String), Sync-Stream mit Reset/Complete-Events.
- Nicht jede Zahl ist, was sie scheint: Die Timeline zeigt 24 942 Fotos, der Full Sync lädt 26 377 Assets (Timeline filtert Papierkorb, Archiv, versteckte Live-Photo-Teile). `search/metadata` meldet `total` pro Seite, nicht die Gesamtzahl.

## 4. Proxy und Dev-Server (`serve.py`)

- `/api/*` wird an `IMMICH_URL` (Standard `http://immich.server.local`) weitergereicht (GET/POST/PUT/PATCH/DELETE/HEAD), Header (inkl. `Range`, mehrere `Set-Cookie`) bleiben erhalten, Antwort wird gestreamt (close-delimited).
- Wozu: kein CORS, kein Mixed Content (https-Seite → http-Immich), Cookies same-origin.
- HTTP `:8080` (localhost) und HTTPS `:8443` (selbst signiertes Zertifikat für die LAN-IP, WebGPU braucht einen sicheren Kontext). Beide hören auf `0.0.0.0`.
- Dev sendet jetzt `Cache-Control: no-store`. Service Worker ist im Dev-Modus ein Selbst-Entferner (`PWA=1` für den echten).
- Port belegt: `lsof -ti tcp:8080 | xargs kill`.

## 5. Arbeit mit parallelen Haiku-Agenten

- **Phase 0 vor den Agenten:** Gerüst (Module als leere Dateien mit Kopfkommentar: Owner, Swift-Quelle, Test-Spezifikation), Traits, Mocks, Proxy. Ohne festes Gerüst kollidieren parallele Agenten.
- **Vorlage:** Eigene Dateien pro Agent (nichts anderes anfassen), Swift-Tests zuerst nach Rust portieren (gleiche Eingaben/Erwartungen, snake_case), dann implementieren, `cargo test --lib <filter>` (nur ein Filter pro Aufruf), im Worktree-Branch committen, Bericht mit Abweichungen und Weggelassenem.
- **Abhängigkeiten ordnen:** Modelle (A2/A3) vor Client/Store (A6/A7); Sync (B1) danach; Views nach Grid/Viewer-Refactor.
- **Worktrees gleichzeitig anlegen** scheitert gelegentlich (`failed to read .../commondir`): Agenten nacheinander starten oder einen Fehlstart wiederholen.
- **Neue Worktrees haben keine git-ignorierten Dateien** (`assets/*.sub.ttf`); ohne sie baut GPUI-Code nicht. Agenten haben die Fonts selbst hineinkopiert.
- **Merge:** Branches mit disjunkten Dateien mergen konfliktfrei. Danach gemeinsam testen. Bei Verdoppelungen aufräumen (Beispiel: `ExifInfo` doppelt in A2/A3, Dauer-Hilfen doppelt in `asset.rs`/`media_duration.rs`, mit abweichendem Verhalten: Swift behält unlesbare Dauer-Strings, A2 machte `None`).
- **Gut für Haiku:** test-getriebene Portierung reiner Logik, abgegrenzte Views mit klarem Vorbild (Alben, Personen). **Schlecht:** Gesten, Performance, wasm/GPUI-Interna, Bild-Pipeline — das lief über Sonnet/selbst.
- Screenshots und Browser-Pane hinken manchmal einen Schritt hinterher; vor einem Urteil zweiten Screenshot machen.

## 6. GPUI-Web: Erkenntnisse

- **Bilder:** `img(SharedUri)` lädt über GPUIs `FetchHttpClient`, dekodiert aber per `image`-Crate im WASM-Hauptthread (JPEG/WebP ok) und **behält jedes dekodierte Bild für immer** (Asset-Cache). Für ein 25k-Raster unbrauchbar. Lösung: eigener Bildspeicher (`immagine/thumbs.rs`): `fetch` → `createImageBitmap` → `OffscreenCanvas.getImageData` → `RenderImage` (BGRA, R/B getauscht) → `img(Arc<RenderImage>)`. Browser dekodiert nativ und parallel.
  - Max. 6 Ladevorgänge gleichzeitig, neueste Frames zuerst, veraltete Anfragen (>4 Frames nicht erneuert) werden verworfen (Abbruch-Ersatz), LRU mit 160 MB Pixelbudget, Ausgeworfenes per `Window::drop_image` aus dem Atlas entfernen.
  - Thumbhash-Platzhalter (`logic::thumbhash::decode` → RGBA, **vormultipliziert**) bis das Bild da ist. Schlüssel: `t:` Thumbnail, `p:` Vorschau, `f:` fullsize, `h:` Hash, `person:` Porträt.
- **Messung (Browser-Pane, WebGPU):** vorher 37–47 FPS, schlechtester Frame 56–90 ms; nachher 55–61 FPS, 19–33 ms, auch nach ~7 000 Zeilen schnellem Scrollen. JS-Heap 11 MB. WASM-/GPU-Speicher und iPhone nicht gemessen.
- **`uniform_list`:** Closure ist `'static` und bekommt `(range, window, cx: &mut App)`; Views erreicht man über `cx.weak_entity()` (`view.update(cx, ..)`). Das Raster hat eine Extrazeile als Platz für die Tab-Bar.
- **`impl IntoElement`-Rückgaben, die `cx` einfangen, brauchen `AnyElement`** (Borrow-Fehler sonst). `with_animation`-Closures sind `'static`: nur kopierte Werte, kein `self`; Animation wird über die ID (Serial) neu gestartet. Buttons mit `cx.listener` gehören *neben* das animierte Element.
- **Kacheln müssen clippen:** `ObjectFit::Cover` allein schneidet nicht ab. Ohne `overflow_hidden` und `absolute().size_full()` ragen Bilder über die Kachel hinaus, sichtbar nur wenn keine Folgezeile darüber malt.
- **Glas (Renderer-Eigenheit):** Der Backdrop wird einmal pro Frame am *ersten* Glas-Element kopiert. Späteres Glas sieht nur den Zustand davor. Folge: Glas im Viewer spiegelte das Raster, weil die Tab-Bar zuerst gemalt wurde. Lösung: Tab-Bar nicht rendern, solange der Viewer offen ist.
- **Eingaben:** Web liefert Maus-/Pointer-Events; es gibt **kein `PinchEvent`**. Trackpad-Pinch kommt als Ctrl+Wheel, Zwei-Finger-Pinch über eigene Pointer-Listener auf `window` (`web_adapters/pinch.rs`, Capture-Phase, nur `pointerType == "touch"`). Echter Touch ist ungetestet (nur synthetische Events). `ClickEvent::click_count()`/`position()` für Doppeltipp.
- **Single-Thread-Dispatcher:** In Safari/dem Pane fehlen Threading-APIs, GPUI fällt auf einen Einzelthread-Dispatcher zurück. Alles läuft auf dem Hauptthread; langes synchrones Parsen blockiert Frames.
- **Fonts:** Latin-Subset von Arial; Zeichen wie ✓, ♡, ƒ können fehlen. Für Statusanzeigen deshalb Wörter („Favorit (an)“) statt Symbole.
- **Eingabefelder:** `InputState::set_masked(true, window, cx)` für Passwort; `InputEvent::PressEnter` per `subscribe_in`.
- **`gen`** ist in Edition 2024 reserviert (`serial` nehmen).

## 7. Browser-Adapter

- `FetchTransport`: `RequestInit` mit `credentials: same-origin`, Header über `Headers`, Body als `Uint8Array`, Antwort `array_buffer`. web-sys-APIs haben `&self`-Setter in der aktuellen Version.
- IndexedDB: ein Objektstore `kv`, Promise-Wrapper über `success/error/complete`-Events; `put` wartet auf `complete` (dauerhaft), nicht nur auf den Request. `localStorage` taugt für 20 MB nicht.
- `Response.headers()` ist iterierbar (`js_sys::try_iter`), liefert `[k, v]`-Arrays.
- Video: DOM-`<video>` (fixed, `z-index: 5`, zwischen Kopf und Fußleiste), damit Fertig/Chips im Canvas erreichbar bleiben. Auth per Cookie, Range-Anfragen laufen durch den Proxy (spielt, 1280×720 geprüft). Handle-`Drop` entfernt das Element; beim Schließen/Blättern verwerfen.
- Zeitstempel für Profiling: `web_adapters::mark(label)` loggt `[perf] label @ms` in die Konsole.
- Selbsttests aus der Konsole: `const m = await import('/pkg/gpui_web_demo.js'); await m.default(); await m.sync_selftest(email, pw)` (Credentials nie speichern).

## 8. Performance-Befunde

- Der Start war nie langsam: Snapshot-Restore 37 ms, erster Frame ~110 ms, inkrementeller Sync ~300 ms. Langsam ist nur der **erste Full Sync** (einmalig ~4,8 s, blockiert die Seite, weil JSON-Parsing auf dem Hauptthread läuft). Idee: in Chunks mit `yield` oder in einen Worker.
- Der echte Hebel waren Bild-Dekodierung (Browser statt WASM) und Cache-Verdrängung, nicht der Sync.
- Offen: Snapshot-Größe (~20 MB) bei jedem Sync mit Änderungen neu schreiben; Original/fullsize bis 48 MB RGBA pro Bild im Viewer; Alben-Kacheln nicht virtualisiert; Cover-Berechnung je Sync.

## 9. Tests und Verifikation

- Logik: `cargo test --lib` (220 Tests). Jedes Modul trägt portierte Swift-Tests.
- UI: im Browser-Pane (Chrome): Token per `fetch('/api/auth/login')` holen, in `localStorage['immagine.token']` schreiben, neu laden. Klicks per Koordinaten im Screenshot-Rahmen (800×600); `left_click_drag` für Wischen; synthetische `PointerEvent`s per JS für Pinch; synthetische `WheelEvent`s für schnelles Scrollen.
- Schreibende Tests am echten Server nur reversibel und mit Rückprüfung (Favorit an/aus, am Ende wieder Ausgangszustand, per API nachgeprüft).
- Passwörter werden vom Agenten nicht in Formulare getippt; im Simulator meldet sich die Person einmal selbst an.

## 10. iOS-Simulator und Safari

- Geräte: iPhone 16 Pro (iOS 18.4, WebGL2) und iPhone 18 Pro (iOS 27, WebGPU). Beide zeigen den Login-Screen korrekt, Safe-Area (Dynamic Island) passt. `localhost` geht direkt, Koordinaten in Punkten (402×874).
- **Safari behält alte ES-Module** im Tab: nach einem Neubau `LinkError: import function ./gpui_web_demo_bg.js:__wbg_... must be callable` (altes JS, neues wasm). Abhilfe: `xcrun simctl terminate booted com.apple.mobilesafari`, URL mit `?x=N` neu öffnen. Server sendet inzwischen `no-store`.
- Frisch gebootete Geräte brauchen ~10 s, bis Safari das erste Mal zeichnet.
- Standalone-PWA-Viewport-Bug (eine Statusleistenhöhe zu kurz) und sein CSS-Fix stehen in `AGENTS.md` und `docs/ios-standalone-viewport.md`; für Immagine noch nicht erneut geprüft.
- Handy-Test: `https://<LAN-IP>:8443` (Zertifikat bestätigen) für WebGPU, `http://<LAN-IP>:8080` für WebGL2.

## 11. Stolpersteine (Kurzliste)

- SSH-Klon scheitert, HTTPS geht.
- Neue Worktrees: `assets/*.sub.ttf` fehlen.
- `.claude/worktrees/` in `.gitignore`.
- Service-Worker-`VERSION` bei jeder Auslieferung erhöhen (aktuell v13); Cache-Name `immagine-`.
- `index.html`/Manifest: Titel und Splash heißen jetzt „Immagine“.
- `img()`-URLs müssen absolut sein (getestet mit `http://localhost:8080/...`); der Bildspeicher nutzt `location.origin`.
- `search/metadata`-Seite 1 liefert bei `size:100` nur 100 Einträge; `total` ist nicht die Gesamtzahl.
- Thumbhash in Swift reserviert 16 Nibbles, liest 5; der Port übernimmt das Verhalten.
- `date_bucket` ist UTC-Tag (Swift: Gerätezeitzone); lokale Tage muss die Anzeige mit `iso_date` umrechnen.
- `delete_assets` im Store rechnet Album-Zähler nicht neu (wie Swift).
- Info-Panel-Länderliste hat ~75 deutsche Namen; natürliche Sortierung ignoriert Umlaute.

## 12. Offen / nächste Schritte

1. Test auf dem echten Handy (WebGPU/WebGL2, Touch-Pinch, FPS, Speicher, Safe-Area, Standalone-PWA).
2. Full Sync in Chunks/Worker; Snapshot nur differenziell speichern.
3. Viewer: Teilen, Löschen/Papierkorb (Outbox hat Trash/Restore), Ort/Karte, Personen im Foto, Live Text, Wisch-hoch-Info, Snap-Back, Tastatur.
4. Alben: Favoriten-Kachel, Anlegen/Umbenennen/Löschen/Cover (`rename_album`, `set_album_thumbnail` existieren), Wisch-zurück, Virtualisierung.
5. Personen: Namen/Geburtstag, Zusammenführen, Ausblenden (Client-Methoden vorhanden).
6. Sortiermodus (Slidebox), Editor (Crop/Drehen/Spiegeln; `crop_geometry` und `AssetEdit` portiert), Einstellungen.
7. Upload per Dateiauswahl (PhotoKit-Upload, Hintergrund-Tasks, Extension entfallen im Web).
8. Alte Demo (`src/app/`) entfernen; Rechtliches mit Robin klären (Übernahme des Projekts).
