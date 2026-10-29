# Plan: Immagine (Swift/iOS) nach gpui-web (Rust/WASM)

Quelle: `../immagine` (geklont per HTTPS, `github.com/RobinCoding/immagine`), ~23k Zeilen Swift, UIKit + SwiftUI + GRDB (SQLite).
Ziel: dieselbe App als GPUI-Webapp gegen einen Immich-Server (getestet: `http://immich.server.local`, API unter `/api`, `GET /api/server/ping` antwortet `pong`).

Zugangsdaten gehören **nicht** ins Repo: nur per Umgebungsvariablen `E2E_SERVER_URL`, `E2E_EMAIL`, `E2E_PASSWORD` (wie in Robins `E2EServer.swift`).

## 1. Was übernommen wird, was nicht

| Bereich | Swift-Quelle | Web-Entscheidung |
|---|---|---|
| Modelle, ISO-Datum, JSON | `Models/`, `Support/ISODate`, `JSONCoding` | 1:1 nach Rust (`serde`) |
| Immich-Client | `Net/ImmichClient.swift` (~45 Endpunkte) | Rust, über Trait `HttpTransport` (Browser: `fetch`) |
| Datenbank | `Database/Database.swift` (GRDB/SQLite) | Trait `Store`; Start in-memory + IndexedDB-Snapshot, kein SQLite |
| Sync | `Sync/` (Delta-Sync-Stream, Album-Outbox) | Rust, läuft auf `Store` + `HttpTransport` |
| Thumbnails | `Thumbnails/` (Thumbhash, Mini-Bitmaps, Prio-Queue) | Thumbhash-Decoder + Prio-Queue als reine Logik; Pixel via `createImageBitmap`, Cache via Cache API |
| Raster-Layout, Zoom 3/5/15, Auswahl, GridDiff | `UI/Timeline/` | Layout/Diff als reine Logik, Rendering in GPUI |
| Viewer, Info-Panel, Albums, Personen, Sortiermodus, Editor | `UI/*` | GPUI-Views, in Wellen |
| Login/Keychain | `Net/Credentials`, `Keychain` | Token in `localStorage`/IndexedDB |
| **Entfällt (kein Web-Äquivalent)** | PhotoKit-Upload, BGTasks, Upload-Extension, Live Text, MapKit, Haptik | später: manueller Upload per `<input type=file>` |

## 2. Testsuite: wie sie genutzt wird

- 13 XCTest-Dateien, ~130 Tests, ~2200 Zeilen, fast alles **reine Logik** (`CoreTests`, `ModelTests`, `GeometryTests`, `UploadTests`, `DatabaseTests`, …). Das ist die Spezifikation.
- Jeder Agent portiert „seine“ Testdatei **zuerst** als Rust-`#[cfg(test)]` (gleiche Testnamen in snake_case, gleiche Eingaben/Erwartungen) und implementiert dann, bis grün.
- Damit `cargo test` nativ läuft (nicht nur wasm): `Cargo.toml` bekommt `crate-type = ["cdylib", "rlib"]`, und die ganze Logik liegt in `src/logic/` **ohne** `web-sys`/`gpui`-Importe. Wasm-Teile hängen hinter `#[cfg(target_family = "wasm")]`.
- `ImmagineUITests` (25 Tests, `E2EServer.swift`) sind Simulator-UI-Tests. Sie werden nicht portiert, sondern als **E2E-Checkliste** gegen den echten Server im Browser-Pane abgearbeitet (Welle D).

## 3. Phase 0: Gerüst (macht eine Person/Sonnet, sequentiell, bevor Haiku starten)

Ohne festes Gerüst kollidieren parallele Agenten. Phase 0 liefert:

1. `Cargo.toml`: `rlib`, `serde`, `serde_json`; `src/logic/mod.rs` mit leeren Untermodulen (ein Modul pro Agent, siehe Welle A).
2. Traits in `src/logic/ports.rs`: `HttpTransport` (async `request(method, path, headers, body) -> Response`), `Store`, `Clock`. Dazu Mock-Implementierungen für Tests.
3. Pro Welle-A-Modul eine Datei mit `todo!()`-Signaturen **und** der portierten, noch roten Testdatei (Skelett genügt; die Agenten füllen aus).
4. `serve.py`: Reverse-Proxy `/api/*` → `IMMICH_URL` (vermeidet CORS; der Immich-Server schickt vermutlich keine CORS-Header für unseren Origin). Prüfen mit `curl`.
5. `docs/immagine-port-plan.md` (dieses Dokument) + Abschnitt in `AGENTS.md`, damit Agenten die Regeln kennen.

Regel für alle Agenten: **eigene Dateien, ein Agent pro Datei**, `mod.rs`/`lib.rs` fasst nur die Phase-0-Person an, jeder Agent arbeitet in eigenem Git-Worktree und liefert einen Branch.

## 4. Welle A: reine Logik, parallel (Haiku geeignet)

Jeder Auftrag: Swift-Datei(en) + zugehörige Tests lesen → Rust-Tests portieren → implementieren → `cargo test logic::<modul>` grün. Keine UI, kein wasm.

| # | Modul (`src/logic/…`) | Swift-Quelle | Test-Spezifikation |
|---|---|---|---|
| A1 | `iso_date`, `json`, `media_duration` | `Support/ISODate`, `JSONCoding`, `MediaDuration` | `CoreTests` (ISO-Parser mit Offsets/Millisekunden, JSON-Samples) |
| A2 | `models::{asset, album, album_tree, album_op, timeline, server, sync}` | `Models/*` | `ModelTests`, `FavoritesTests` |
| A3 | `models::{person, asset_details, asset_info, asset_edit}` | `Models/Person` u. a. | `PeopleTests`, `InfoPanelTests` |
| A4 | `grid::{layout, diff, selection}`, `crop_geometry` | `TimelineGridLayout`, `CropGeometry`, GridDiff | `GeometryTests`, `GridInteractionTests` |
| A5 | `thumbhash`, `display_image`, `viewer_images`, `rendition_cache` | `ThumbhashDecoder`, `DisplayImage`, `ViewerImages` | `DisplayImageTests`, `ViewerImagesTests`, `RenditionCacheTests` |
| A6 | `client` (Request-Aufbau, Fehler, alle Endpunkte) | `Net/ImmichClient.swift` | gegen `MockTransport` mit echten JSON-Antworten; Login-Antwort, Paging, Fehlercodes 401/4xx |
| A7 | `store` (In-Memory-Implementierung von `Store`) | `Database/Database.swift` | `DatabaseTests` (Upsert, Alben, Favoriten, Papierkorb, Delta-Anwendung) |

A1–A7 sind untereinander unabhängig (nur gemeinsame Typen aus Phase 0). Wenn A2 Typen ändern muss, die andere nutzen: vorher in Phase 0 festlegen, sonst gibt es Merge-Konflikte.

## 5. Welle B: Orchestrierung und Browser-Adapter

Hängt von A ab. Zwei Gruppen parallel:

- **B1 Sync (Haiku, eng spezifiziert):** `SyncEngine`, `SyncStream` (NDJSON-Stream parsen, Acks), `AlbumOutbox` (Offline-Änderungen nachreichen). Tests: Rest von `ModelTests`/`OfflineTests` sowie neue Tests mit `MockTransport` + `MemoryStore`.
- **B2 Web-Adapter (Sonnet, wasm-lastig, Haiku-Risiko):**
  - `FetchTransport` (`web-sys`/`wasm-bindgen-futures`, Bearer-Token, Binärantworten),
  - `IdbStore` (IndexedDB-Snapshot des `Store`),
  - Bild-Dekodierung: `createImageBitmap` → Textur für GPUI (muss im vendored `gpui-pre-web` geprüft werden; GPUI-Bildpfad evtl. erweitern),
  - Thumbnail-Store: Prio-Queue (max. ~6 gleichzeitig, Abbruch außerhalb des Sichtbereichs, wie in Robins `TODOs.txt` beschrieben) + Cache API.

## 6. Welle C: UI in GPUI

Gemeinsamer Unterbau zuerst (Sonnet): `AppState`-Äquivalent (`Entity<AppState>`), Navigation (Tab-Leiste existiert), Glas-Komponenten wiederverwenden. Danach parallel, jeweils eigenes Verzeichnis unter `src/app/`:

| # | View | Schwierigkeit | Modell |
|---|---|---|---|
| C1 | Login + Einstellungen | gering | Haiku |
| C2 | **Timeline-Raster**: virtualisiertes Raster, Pinch 3/5/15, Datums-Header, Pull-to-Refresh, Thumbhash-Platzhalter, Auswahlmodus | hoch (Performance, Gesten) | Sonnet/Opus |
| C3 | **Viewer**: Wischen, Zoom, Zoom-Übergang (Muster existiert in `photos/viewer.rs`), Filmstreifen, Chrome | hoch | Sonnet/Opus |
| C4 | Alben + Ordner (`AlbumTree`), Album-Detail, Sheets | mittel | Haiku mit Review |
| C5 | Personen: Liste, Personenseite, Gesichter | mittel | Haiku mit Review |
| C6 | Info-Panel (Metadaten, Ort ohne Karte), Favoriten | gering–mittel | Haiku |
| C7 | Sortiermodus (Slidebox) | mittel | Sonnet |
| C8 | Editor (Crop/Drehen/Spiegeln über `AssetEdit`) | mittel | Sonnet |

C1, C4, C5, C6 greifen nur auf Welle A/B zu und können früh starten; C2/C3 erst, wenn Thumbnail-Store (B2) steht.

## 7. Welle D: Integration

1. E2E gegen echten Server im Browser-Pane und iOS-Simulator (WebGL2 **und** WebGPU, siehe `AGENTS.md`): Login, Sync, Raster scrollen, Viewer, Album anlegen/umbenennen/löschen, Favorit, Person umbenennen. Testdaten erzeugen und danach wieder aufräumen, nie vorhandene Fotos löschen oder ändern.
2. Performance: FPS-Badge, Sync von ~25k Assets, Speicher auf iPhone-Safari.
3. PWA: `VERSION` in `www/sw.js` hochzählen, Standalone-Viewport prüfen.
4. Optional: manueller Upload per Dateiauswahl.

## 8. Reihenfolge und Parallelität

```
Phase 0 (1 Person)
   ├─ Welle A: A1 A2 A3 A4 A5 A6 A7   (7 Haiku parallel)
   │     ├─ B1 Sync (Haiku)   ┐
   │     └─ B2 Adapter (Sonnet)┤
   │                           ├─ C1 C4 C5 C6 (Haiku), C7 C8 (Sonnet), C2 C3 (Sonnet/Opus)
   └──────────────────────────────── Welle D
```

## 9. Risiken

- **CORS / Mixed Content:** Die App läuft über `https://<LAN-IP>:8443`, Immich über `http://`. Ohne Proxy blockt Safari Mixed Content. Deshalb der Proxy in Phase 0.
- **Bild-Pipeline:** GPUI-Web muss dekodierte Browser-Bilder anzeigen können; das ist die größte technische Unbekannte und gehört vor C2/C3 geklärt (Spike am Anfang von B2).
- **Haiku-Grenzen:** gut für test-getriebene Portierung und einfache Views, schwach bei Gesten, Performance und wasm/GPUI-Interna. Deshalb C2/C3/B2 mit Sonnet oder stärker.
- **Raster-Performance:** 25k Assets brauchen Virtualisierung und zeilenweises Layout, kein Element pro Foto.
- **Lizenz/Absprache:** Das Immagine-Repo hat Robin; vor Veröffentlichung des Ports klären, ob die Übernahme so gewollt ist.

## 10. Stand Phase 0 (erledigt)

- `Cargo.toml`: `rlib` + `cdylib`, `serde`, `serde_json`, dev-dep `futures-lite`. `cargo test --lib` läuft nativ, `cargo check --target wasm32-unknown-unknown --lib` ist grün.
- `src/logic/` (Modulname `logic`, nicht `core`, wegen Namenskollision mit dem Rust-`core`-Crate): Dateien pro Agent sind angelegt, jede mit Kopfkommentar (Owner, Swift-Quelle, Test-Spezifikation). `mod.rs`-Dateien sind fertig; Agenten legen keine neuen Module an, ohne `mod.rs` mit dem Orchestrator abzustimmen.
- `src/logic/ports.rs`: `HttpTransport` (async, nicht `Send`), `Clock`, `MockTransport`, `FixedClock`, mit Test. Den `Store`-Trait definiert A7 in `store.rs`.
- `serve.py`: `/api/*` wird an `IMMICH_URL` (Standard `http://immich.server.local`) weitergereicht. Geprüft: `ping` und Login über `http://localhost:8080/api/...` funktionieren.

**Reihenfolge-Korrektur:** A6 (Client) und A7 (Store) hängen von den Modellen aus A2/A3 ab. Erst A1–A5 starten; A6/A7 starten, sobald A2/A3 gemergt sind.

## 11. Auftragsvorlage für Haiku-Agenten

> Du portierst ein Modul der iOS-App Immagine nach Rust. Datei: `src/logic/<modul>.rs` (nur diese Dateien ändern, außer anderes steht im Kopfkommentar). Lies im Kopfkommentar die Swift-Quelle und die Test-Spezifikation in `../immagine`. 1) Portiere die zugehörigen XCTest-Tests als `#[cfg(test)] mod tests` (gleiche Eingaben und Erwartungen, Namen in snake_case). 2) Implementiere, bis `cargo test --lib logic::<modul>` grün ist. 3) Keine Imports von `gpui`, `web-sys`, `wasm-bindgen`. 4) Keine neuen Abhängigkeiten ohne Rückfrage. 5) Arbeite im eigenen Worktree-Branch, committe dort und berichte: was portiert, welche Tests, was bewusst weggelassen (und warum).

## 12. Ergebnis B2-Spike: Bild-Pipeline (geprüft im Browser-Pane)

- **Funktioniert ohne eigenen Dekoder:** `gpui::img(SharedUri::from("<url>"))` lädt über GPUIs `FetchHttpClient`, dekodiert mit der `image`-Crate (JPEG ist aktiviert) und zeichnet. Getestet mit `/api/assets/<id>/thumbnail?size=preview` (Preview ~400 KB JPEG), das Foto erschien im Fenster.
- **Authentifizierung per Cookie:** `POST /api/auth/login` über den Proxy setzt `immich_access_token` (HttpOnly, Path=/). Danach hängt der Browser das Cookie an alle gleich-originalen `/api/...`-Requests an, auch an die von `img()`. Header (Bearer) kann `img()` nicht setzen, Cookies brauchen es nicht. Der Login läuft daher über `fetch` (oder `FetchTransport`), nicht über `ImmichClient` mit Token-Parameter allein.
- **Absolute URL nötig/getestet:** im Spike wurde `http://localhost:8080/...` verwendet; relative URLs sind nicht geprüft. Für die App `location.origin` aus JS durchreichen.
- **Offen:** Dekodierung von großen Originalen/Vorschauen blockiert evtl. den Hauptthread (die `image`-Crate dekodiert in WASM langsam). Für das Raster reichen `thumbnail`-Größen; für den Viewer ggf. `createImageBitmap` nachziehen. Kein eigener Prioritäts-/Abbruch-Mechanismus: `img()` lädt, was gerendert wird, die Prio-Queue (`rendition_cache`) muss davor entscheiden, welche Zellen überhaupt ein `img` bekommen.

## 13. Stand Welle B (erledigt)

- **B1 Sync** (`src/logic/sync/`): gemergt, 23 Tests. `SyncEngine<S, T, C>`: Outbox → Full Sync → Stream. Offen: `repairAlbumLinks`, Callbacks/Coalescing, parallele Detailabrufe.
- **B2 Adapter** (`src/web_adapters/`, nur wasm): `FetchTransport` (fetch, Cookies same-origin), IndexedDB-Key-Value (`idb::put/get`), `WebClock`, `selftest`/`sync_selftest` (aus der Konsole aufrufbar).
- **Gegen den echten Server geprüft** (Browser-Pane, Login per Cookie über den Proxy): Full Sync 26 377 Assets + 12 Alben in ~4,8 s, danach inkrementeller Lauf in ~50 ms; Snapshot-Roundtrip über IndexedDB (100 Assets: 74 KB) stimmt. Snapshot für alle 26k Assets noch nicht gemessen (grob ~20 MB JSON, Schreiben/Laden prüfen).
- Offen für Welle C: Thumbnail-Queue (Prio, Abbruch), Snapshot-Persistenz im App-Ablauf (Start: restore, Ende: put), Login-Screen, Raster/Viewer.

## 14. Stand Welle C, Teil 1: Login und Timeline (im Browser-Pane geprüft)

- Neues Root `src/immagine/` (`Immagine`-View) ersetzt die Demo als Startansicht (`src/app/` bleibt vorerst als Spender für Glas-/Tab-Bar-Code, `#[allow(dead_code)]`).
- **Login:** E-Mail/Passwort (maskiert), Token in `localStorage` (`immagine.token`), Cookie vom Proxy für `img()`. Das Tippen im Formular ist noch nicht im Browser-Pane getestet; der Test lief mit per `fetch` gesetztem Token.
- **Start:** gespeicherten Store-Snapshot aus IndexedDB laden, Timeline sofort zeigen, danach Sync im Hintergrund; Snapshot nur bei Änderungen zurückschreiben. 401 → Abmelden.
- **Timeline:** `uniform_list` (virtualisiert), Spalten nach Breite (3–8), `img(thumbnail)` mit `ObjectFit::Cover`. 24 942 echte Fotos, Scrollen funktioniert (37–47 FPS im Pane, Worst-Frame 56–90 ms).
- **Bekannte Probleme:**
  1. Erster Frame erst nach ~3 s (mit Snapshot) bzw. ~8 s (erster Full Sync): Sync und JSON-Parsing blockieren den Hauptthread. Lösung: in Chunks mit `yield` aufteilen oder Sync in einen Worker.
  2. GPUIs Asset-Cache behält dekodierte Thumbnails; bei langem Scrollen wächst der Speicher. Lösung: eigener LRU-Image-Cache (`rendition_cache`) / `RetainAllImageCache`-Ersatz.
  3. Schlechte Frame-Zeiten beim Dekodieren (Worst-Frame bis 90 ms).
  4. Kein Platzhalter aus Thumbhash, kein Tab-Bar, kein Viewer, keine Alben/Personen.

## 15. Stand Welle C, Teil 2: Tab-Bar und Viewer (im Browser-Pane geprüft)

- **Tab-Bar** (`src/immagine/tab_bar.rs`): Glas-Leiste mit Mediathek / Alben / Personen und animiertem Schieber; Alben und Personen zeigen noch „folgt bald“ (`Immagine::coming_soon`).
- **Viewer** (`src/immagine/viewer.rs`): Zoom aus der angetippten Kachel (Kachel-Bounds über ein `canvas` pro Kachel in `tile_bounds`), Wischen links/rechts zum Blättern (Nachbarseiten werden vorgeladen), Wischen nach unten oder „Fertig“ schließt. Jede Seite legt die schon geladene Grid-Vorschau unter das `preview`-Bild (Platzhalter ohne Sprung). Geprüft: öffnen (5 von 24 942), blättern (→ 6), Wisch-nach-unten-Schließen, Tab-Wechsel.
- **Nicht umgesetzt im Viewer:** Pinch-/Doppeltipp-Zoom, Original statt Vorschau, Video-Wiedergabe, Filmstreifen, Teilen/Favorit/Info/Löschen, Snap-Back-Animation beim Abbrechen eines Wischs, Tastatursteuerung.
- Muster für Views, die `cx` brauchen: Rückgabetyp `AnyElement` (sonst Borrow-Fehler), Klick-Handler in `uniform_list`-Closures über `cx.weak_entity()`.

## 16. Performance (Browser-Pane, WebGPU)

- **Start war nie das Problem:** Mit gespeichertem Snapshot (19,7 MB) dauert Restore 37 ms, der erste Frame ~110 ms, ein inkrementeller Sync ~300 ms. Die 3–8 s aus Abschnitt 14 waren der **erste** Full Sync (einmalig, blockiert die Seite ~4,8 s). Offen: Full Sync in Chunks mit `yield` oder im Worker.
- **Bild-Pipeline neu** (`src/immagine/thumbs.rs`, `web_adapters/image_loader.rs`): `fetch` → `createImageBitmap` → `OffscreenCanvas.getImageData` → `RenderImage` (BGRA). Der Browser dekodiert nativ statt der `image`-Crate im WASM-Hauptthread. Dazu: max. 6 gleichzeitige Ladevorgänge, neueste Frames zuerst, veraltete Anfragen (aus dem Bild gescrollt) werden verworfen, LRU mit 160-MB-Budget für dekodierte Pixel, ausgeworfene Bilder werden per `Window::drop_image` aus dem Atlas entfernt. Thumbhash-Platzhalter (`logic::thumbhash`) füllen Kacheln bis zum Bild.
- **Messung** (gleicher Browser-Pane, Foto-Raster scrollen): vorher 37–47 FPS, Worst-Frame 56–90 ms; nachher 55–61 FPS, Worst-Frame 19–33 ms, auch nach 300 schnellen Wheel-Events (~7 000 Zeilen weit). JS-Heap 11 MB. WASM-Speicher und iPhone-Verhalten nicht gemessen.
- Viewer-Seiten (Raster-Thumbnail + `preview`) laufen durch dieselbe Pipeline.

## 17. Stand Welle C, Teil 3: Alben und Personen (Haiku-Agenten, im Browser-Pane geprüft)

- **Alben** (`src/immagine/albums.rs`): Kacheln für Ordner (2×2-Collage) und Alben, Ordner-Navigation mit Zurück-Chip/Breadcrumb, Albumseite mit `photo_grid`. Geprüft: 12 Alben, „Favoriten“ geöffnet, Foto im Viewer („2 von 3“).
- **Personen** (`src/immagine/people.rs`): lädt Personen per `ImmichClient::people`, runde Porträts über die Thumb-Pipeline, Personenseite mit `person_assets`. Geprüft: 514 Personen, Person „Johannes“ mit 5 707 Fotos.
- **Fehler gefunden und behoben:** Gitterkacheln clippten nicht; Bilder mit `ObjectFit::Cover` ragten über die Kachel hinaus und wurden nur von der nächsten Zeile überdeckt (bei einer einzigen Zeile sichtbar). Fix: `overflow_hidden` an der Kachel, Bild `absolute().size_full()`.
- `log_out` setzt jetzt Alben, Personen, Viewer und Tab zurück.
- **Offen:** Alben: Favoriten-Kachel, Anlegen/Umbenennen/Löschen, Wisch-zurück, Virtualisierung der Kacheln, Cover-Berechnung bei jedem Sync. Personen: Namen/Geburtstag ändern, Zusammenführen, Ausblenden; Wisch-zurück. Beide: kein Viewer-Pinch, keine Info.
- **Prozess:** neue Worktrees brauchen `assets/*.sub.ttf` (git-ignoriert) zum Bauen.
