# IndexedDB (Web-Host) — Wissen und Grenzen

Stand: Notizen aus der Entwicklung, Quota-Zahlen sind grobe Richtwerte aus dem Gedächtnis und **nicht geprüft**;
sie hängen von Browserversion und Gerät ab. Das tatsächliche Quota liefert `navigator.storage.estimate()`.

## Was es ist
Die Datenbank des Browsers: asynchroner Key-Value-Speicher für beliebige Werte inkl. großer Binärdaten, pro Origin
(Domain + Port), überlebt Neustarts. Bei uns: `src/host/web/idb.rs` (eine Datenbank pro `db`-Name, ein Object Store
`kv`; `put`, `get`, `clear`), erreichbar nur über `Host::blob_get/blob_put/blob_clear`. Auf dem Desktop sind das
Dateien (`host/desktop/store.rs`).

Genutzt für:
- `immagine` / Store-Snapshot (`SNAPSHOT_KEY`): die Timeline offline und schneller Start.
- `video-cache` (`DISK_DB` in `immagine/video_player.rs`): Spans (1–4 MB) bei Videos größer als `MEM_BUDGET`,
  Key = Startposition in der Datei; wird beim Öffnen eines Players mit `blob_clear` geleert.

## Größe (Richtwerte)
Kein festes Limit, sondern Quota pro Origin nach freiem Speicher:
- Chrome/Edge: bis ca. 60 % des Datenträgers.
- Firefox: ca. 10 % des Datenträgers.
- Safari (iOS/macOS): im Browser grob 1 GB, danach Nachfrage; installierte PWA (Home-Bildschirm) deutlich mehr
  (bis ca. 60 %).

## Zuverlässigkeit
- Standard ist "best effort": der Browser darf bei Speicherdruck räumen, dann das ganze Origin auf einmal.
- Safari löscht Daten von Websites, die 7 Tage nicht benutzt wurden (ITP); gilt nicht für zum Home-Bildschirm
  hinzugefügte PWAs.
- `navigator.storage.persist()` fordert dauerhaften Speicher an (Chrome/Firefox: Heuristik oder Nachfrage, Safari
  eingeschränkt). Wir rufen es noch nicht auf.
- Jeder `put` kopiert die Bytes und schreibt den ganzen Wert; gut für Stücke von 1–4 MB, schlecht für
  Gigabyte-Werte am Stück.
- Schreibfehler (Quota) dürfen nie fatal sein: der Code behandelt sie als "nicht gecached" (Video-Cache liest
  dann wieder vom Server; Snapshot-Fehler werden ignoriert).

## Eignung
- Als Cache (neu ladbar): gut. Die Originale liegen auf dem Immich-Server, ein Verlust ist unkritisch.
- Als einzige Kopie: nicht verlässlich. Für Dauerhaftigkeit: installierte PWA + `storage.persist()`.
- Für große Offline-Dateien besser: OPFS (Origin Private File System), gleicher Quota-Pool, aber Lesen/Schreiben mit
  Offsets ohne Komplettkopie. Auf Safari ist der effiziente Zugriff (`createSyncAccessHandle`) nur in Web Workern
  möglich (wir haben Worker, aber es wäre ein eigener Umbau).

## Mögliche nächste Schritte
- `navigator.storage.persist()` beim Start anfordern.
- Plattencache auf einen Anteil von `storage.estimate()` begrenzen.
- Bei echten Offline-Kopien ganzer Videos/Fotos: OPFS statt IndexedDB prüfen.
