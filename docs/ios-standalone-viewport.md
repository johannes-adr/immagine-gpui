# iOS-PWA: Lücke am unteren Rand (Protokoll)

Stand: 29.10.2026. Dieses Dokument protokolliert, wie sich die installierte App (Home-Bildschirm, Standalone-Modus) auf iOS verhält, was gemessen wurde, was nicht half und was am Ende funktioniert hat. Kurzfassung steht in `AGENTS.md`.

## Symptom

Wenn die App vom Home-Bildschirm gestartet wird (also ohne Safari-Leiste), endet die Fläche zu früh. Die untere Hälfte der Tab-Bar ist abgeschnitten, darunter ist ein Streifen in der Hintergrundfarbe der Seite (`#cfe3ff`). Im normalen Safari-Tab tritt das nicht auf.

## Testumgebung

| Gerät | iOS | Grafik-Backend | Punkte |
|---|---|---|---|
| iPhone 16 Pro (Simulator) | 18.4 | WebGL2 | 402×874 |
| iPhone 18 Pro (Simulator) | 27.0 | WebGPU | 402×874 |

Seite: `www/index.html` mit `viewport-fit=cover`, `apple-mobile-web-app-capable`, `apple-mobile-web-app-status-bar-style=black-translucent` und `manifest.webmanifest` (`display: standalone`). Ein echtes iPhone wurde für diese Untersuchung nicht verwendet.

## Messwerte

Die 62 pt entsprechen der oberen Safe-Area (Dynamic Island), 34 pt ist die untere.

| Situation | `innerHeight` | Insets oben/unten | Canvas | Ergebnis |
|---|---|---|---|---|
| Safari-Tab (iOS 18.4) | an sichtbare Fläche angepasst | – | passt | keine Lücke |
| Standalone, Kaltstart (beide iOS) | **812** | 62 / 34 px | 874 (CSS), Backing 1748 px | Lücke unten, Tab-Bar abgeschnitten |
| Standalone, allererster Start nach dem Hinzufügen (iOS 27) | **874** | 62 / 34 px | 874 | voll bis unten, richtig |
| Standalone mit Fix, Kaltstart (iOS 27, zweimal) | 874 | 62 / 34 px | 0..874 | keine Lücke, Seite scrollt nicht |
| Standalone mit Fix, Kaltstart (iOS 18.4) | 874 | 62 / 34 px | 0..874 | keine Lücke |

Beobachtung: Der Wert ändert sich innerhalb eines Starts nicht. Beim Kaltstart ist er schon bei der ersten Messung (etwa 70 ms nach Seitenstart) 812.

## Versuche in zeitlicher Reihenfolge

1. **Ausgangslage:** `html, body { position: fixed; inset: 0; height: 100% }` mit `100dvh`-Variante. Ergebnis: Lücke, `innerHeight` 812.
2. **`100vh` im Standalone-Modus:** keine sichtbare Wirkung. Dieser Test ist nur eingeschränkt aussagekräftig, weil der Service Worker damals noch alte Dateien auslieferte.
3. **Höhe per JavaScript aus `screen.height`:** `body` wurde 874 hoch, `html.clientHeight` blieb 812, im Bild keine Änderung.
4. **Canvas direkt gemessen:** Canvas-Rechteck 0..874, Backing-Store 1748 px (874 × 2). Zusätzlich Canvas-Hintergrund Magenta. Unter 812 pt war nur die Seitenfarbe zu sehen, kein Magenta. Damit war belegt: Der Canvas ist groß genug, wird dort aber nicht angezeigt.
5. **GPUI-Patch (`vendor/gpui-pre-web`, `resize()` ohne Wirkung):** GPUI setzte die Canvas-Größe beim Öffnen sonst per Inline-Stil auf `innerHeight`. Mit dem Patch war der Canvas 874 hoch, das GPUI-Layout passte sich an (Tab-Bar 34 pt über dem Rand), trotzdem war alles unter 812 pt leer. Wieder zurückgenommen.
6. **iOS 27:** Safari-Tab läuft mit WebGPU ohne Lücke. Standalone verhält sich wie auf iOS 18.4. Nur der allererste Start nach dem Hinzufügen hatte volle Höhe (zunächst nicht reproduzierbar, später gezielt nachgestellt: Icon löschen, neu hinzufügen, sofort starten).
7. **Recherche** (siehe unten): bekanntes WebKit-Problem. Die häufigste Lösung nimmt `position: fixed` weg und macht `html` per `lvh` höher.
8. **Fix umgesetzt und gemessen:** siehe nächster Abschnitt.

## Lösung

In `www/index.html`, nur im Standalone-Modus:

```css
@media (display-mode: standalone) {
  html { position: static; inset: auto; height: auto; min-height: 100lvh; }
  body { position: relative; inset: auto; width: 100%; height: 100lvh; min-height: 100lvh; }
}
```

Die normale Regel (`position: fixed; inset: 0; height: 100dvh`) bleibt für Safari-Tabs bestehen, damit die Seite dort nicht scrollt und keine Lücke entsteht, wenn die Safari-Leiste einklappt.

Nach dem Fix sind `innerHeight`, `html`, `body` und Canvas alle 874 pt hoch. Ein Wisch nach oben scrollte die Seite nicht.

## Offene Punkte

- Nicht auf einem echten iPhone getestet. In einem der gefundenen Projekte hat eine Lösung über `screen.height` dort die Tab-Bar abgeschnitten.
- Warum nur der allererste Start nach dem Hinzufügen die volle Höhe bekommt, ist nicht geklärt.
- Nicht ausprobiert: `apple-mobile-web-app-status-bar-style` auf `default` oder `black`.
- Der Safari-Tab wurde nach dem Fix nicht erneut angesehen; die neue Regel greift dort nicht.

## Quellen (Berichte anderer Projekte, nicht von Apple)

- WebKit-Bug 301108 wird in mehreren Projekten als Ursache genannt (Viewport um die Statusleistenhöhe zu kurz bei `viewport-fit=cover` und `black-translucent`). Die Bug-Seite selbst konnte ich nicht öffnen.
- [Voxal PR 230](https://github.com/ErwannRobin/Voxal/pull/230), [iceledrain PR 22](https://github.com/4n8rvvj9t9-rgb/iceledrain/pull/22), [japan-2026 PR 7](https://github.com/hshringeri/japan-2026/pull/7): Lösung über `100lvh` bzw. `min-height` auf `html`.
- [quenya PR 10](https://github.com/baher228/quenya/pull/10) und [quenya PR 11](https://github.com/baher228/quenya/pull/11): Anpassung an `screen.height` schob die Tab-Bar auf einem echten iPhone aus dem Bild, iOS zeichnet im Home-Screen-Modus nicht unter `innerHeight`.
- [irodr PR 147](https://github.com/azu/irodr/pull/147): Lücke nur beim ersten Start, `viewport-fit=cover` für einen Frame entfernen und wieder setzen.
- [splitkick Issue 63](https://github.com/azeitler/splitkick/issues/63): iOS 26.6, Zweifel, ob die Ursache WebKit oder die eigene Höhenberechnung ist.

## Fallstricke beim Testen

- **Service Worker:** Der echte `sw.js` liefert Cache-first. Neue Versionen erscheinen erst beim zweiten Start. Im Dev-Betrieb liefert `serve.py` deshalb einen Kill-Switch (`PWA=1` schaltet den echten ein).
- **Neustart:** Die App muss wirklich beendet werden: `xcrun simctl terminate <udid> com.apple.webapp`. Ein Wegwischen im App-Wechsler war unzuverlässig. Sonst sieht man eine fortgesetzte App mit eingefrorenen Werten (gleiche FPS-Zahlen).
- **Icon neu hinzufügen:** Die `apple-*`-Metas werden nur beim Hinzufügen gelesen.
- **Screenshots:** Das Werkzeug hinkt teils einen Schritt hinterher, darum nach einem Tipp zweimal schauen.
- **Messen in der PWA:** Es gibt keine Konsole. Ein temporäres fixes `<div>` mit `innerHeight`, `html`/`body`-Höhe und Canvas-Rechteck hat gut funktioniert.
