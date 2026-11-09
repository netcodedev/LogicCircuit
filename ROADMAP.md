# Roadmap: Vom Netzlisten-Export zum 3D-Chip

Ziel: Eine Schaltung (z. B. `COMPUTER_8B`) als echtes Chip-Layout erzeugen und
in 3D anzeigen – Transistoren, Metalllagen und Vias, mit einstellbarer Anzahl
an Verdrahtungslagen und live eingefärbten Signalen.

Weg: Wir exportieren unsere Netzliste als strukturelles Verilog und lassen
Platzierung und Verdrahtung von der offenen Toolchain erledigen (Yosys +
OpenROAD, SkyWater-130-nm-PDK `sky130_fd_sc_hd`). Das Ergebnis (DEF/GDS)
wandeln wir in ein eigenes Layout-Format um und zeigen es an.

## Entscheidungen (2026-11-09)

- **Toolchain:** Docker-Image `openroad/orfs`, gepinnt auf
  `26Q4-374-g2ccdcd3c2` (enthält Yosys, OpenROAD und die sky130hd-Plattform).
  Den Ablauf steuern eigene Tcl-Skripte unter `chip/flow/`, aufgerufen von
  der Rust-CLI. Kein OpenLane: das ist Python-basiert, und das Projekt
  bleibt frei von Python (nur im Container vorhanden).
- **Flipflops:** Standard ist die Abbildung von `DFLIP_FLOP` auf die echte
  Zelle `sky130_fd_sc_hd__dfxtp_1` (Timing und Clock-Tree funktionieren).
  Mit `--keep-latches` bleiben sie als eigene Gatter (NOR-Latches) erhalten,
  dann ohne verlässliches Timing.
- **Abbildungsstufe:** Standard sind Basis-Blöcke (`flatten(FlatLevel::Basic)`
  → `inv`, `and2`, `nor2`, …); mit `--level nand` wird alles aus `nand2` gebaut.
- **Lokal:** `yosys` und `iverilog` aus den Arch-Repos (für den schnellen
  Export-Test ohne Docker).

**CLI und Browser:** Die ganze Pipeline läuft über die CLI. OpenROAD lässt
sich nicht im Browser ausführen, deshalb erzeugt die CLI die Layout-Datei, und
der Browser lädt sie (eingebettet oder per Drag-and-drop). Alles, was nicht die
externe Toolchain braucht (Export, Statistiken, Darstellung), gibt es in beiden.

---

## Phase 0 – Grundlagen und Entscheidungen

- [x] **Flipflops entscheiden** (wichtigste offene Frage). Unsere Register sind
      Master-Slave-Flipflops aus NOR-SR-Latches, also kombinatorische
      Schleifen. Statische Timing-Analyse und Clock-Tree-Synthese in OpenROAD
      erkennen das nicht als Flipflop. Optionen:
  - [ ] A: `DFLIP_FLOP` beim Export auf die echte Zelle `sky130_fd_sc_hd__dfxtp_1`
        abbilden (realistisch, Timing funktioniert, aber das Flipflop ist nicht
        mehr „aus unseren Gattern“)
  - [ ] B: Latches als Gatter behalten und die Schleifen für das Timing
        abschalten (`set_disable_timing`), Taktverteilung ohne CTS (lehrreich,
        aber kein verlässliches Timing)
  - [x] Entscheidung festhalten: beides, A als Standard (siehe oben)
- [x] **Abbildungsstufe festlegen:** Welche unserer Bausteine werden zu welcher
      Standardzelle? Vorschlag: Stufe „Basis-Blöcke“ aus `flatten(FlatLevel::Basic)`
- [x] **Toolchain bereitstellen:** Image `openroad/orfs:26Q4-374-g2ccdcd3c2`
      gezogen (4,5 GB entpackt); enthält OpenROAD, Yosys und
      `flow/platforms/sky130hd` (LEF, Liberty, GDS, PDN, Tracks)
- [x] Lokal `yosys` 0.66 und `iverilog` 13.0 aus den Arch-Repos installiert (für Icarus-Gegenproben in Phase 1)
- [x] **Flow-Skript:** `chip/flow/pnr.tcl` (OpenROAD: Floorplan, Tapcells,
      Power-Grid, Platzierung, Clock-Tree, Verdrahtung, DEF/ODB, Timing) und
      `chip/flow/run.sh` (Docker-Aufruf, bis die Rust-CLI übernimmt)
- [x] **Smoke-Test** `chip/smoke/smoke.v` (registrierter Halbaddierer aus
      sky130-Zellen): läuft ohne DRC-Verstöße durch; 155 µm Leitung, 39 Vias,
      9,48 ns Slack bei 10 ns Takt. Mit `MAX_LAYER=met2` bleibt met3–met5 frei
      (151 µm, 36 Vias)
- [x] YoWASP geprüft: `@yowasp/yosys` (npm, 0.71) existiert, ~80 MB entpackt –
      im Browser machbar, aber schwer; erst bei Bedarf

**Erkenntnisse für Phase 2:**
- Das Power-Grid (`pdn.tcl`) nutzt immer met4/met5, auch wenn Signale auf
  weniger Lagen beschränkt sind. Für eine echte „N-Lagen“-Variante braucht es
  ein eigenes PDN je Lagenzahl.
- Pins liegen fest auf met2/met3; bei `MAX_LAYER` < met3 die Pin-Lagen mit absenken.
- Erwartete, harmlose Warnungen: `DRT-0349` (LEF58_ENCLOSURE),
  `PDN-1051` (leere Makro-Raster), `IFP-0028` (Core-Bereich eingerastet).

## Phase 1 – Verilog-Export

- [x] `src/export/verilog.rs`: strukturelles Verilog aus `flatten_until()`
  - [x] ein Modul pro Top-Level-Schaltung, Ports = CLDL-Ein- und Ausgänge (Busse als Vektoren)
  - [x] ein `wire` pro Netz (`n_<Schlüssel>`, `.` → `__`); Netze mit Ein-/Ausgangsbit heißen wie der Port
  - [x] Zellinstanzen `u_<Hierarchiepfad>`; Rückweg über `--names` (TSV: Verilog-Name → Schlüssel/Pfad)
  - [x] nicht getriebene Netze (lesen 0) an Tie-Low-Zellen (`conb_1` LO), weitere Ausgangsbits eines Netzes über Buffer
- [x] Zwei Zellbibliotheken:
  - [x] `generic`: `LC_*`-Module werden angehängt (Datei simuliert eigenständig). Jede
        Zelle startet mit dem Wert unseres Simulators nach dem Bauen (`INIT`) und hat
        1–3 ps Laufzeit (`DELAY`) – ohne Laufzeit schwingen über Kreuz gekoppelte
        Latches in Icarus endlos
  - [x] `sky130`: nand2, inv, and2, and3, or2, nor2, nor4, xor2, xnor2 (`_1`);
        Mehrbit-Blöcke bitweise; `DFLIP_FLOP` → `dfxtp_1` bzw. `dfxbp_1`, wenn nQ gebraucht wird
- [x] CLI: `export-verilog <NAME> [--level basic|nand] [--cells sky130|generic] [--keep-latches] [-o DATEI] [--names DATEI]`
- [x] Browser: Button „Verilog ↓“ (Zellbibliothek wählbar; Stufe NAND in der NAND-Ansicht, sonst Basis-Blöcke)
- [x] **Gegenprobe:** `verify-verilog [NAME] [--level] [--keep-latches]` erzeugt aus den
      `.clt`-Suiten Testbenches und lässt sie mit Icarus laufen (Zeitlimit 120 s pro
      Simulation). **Ergebnis 2026-11-09: alle 46 Suiten in allen vier Varianten
      (basic/nand × Flipflop-Zellen/eigene Latches) identisch zu unserem Simulator**
- [x] Rust-Tests: Export deterministisch für alle Schaltungen und Varianten,
      NAND-Stufe ohne Flipflops = exakt `nand_count` nand2-Zellen, Icarus-Probe für
      FULL_ADDER, REGISTER_4B (Latches) und COMPUTER_8B (übersprungen ohne iverilog)
- [x] **Durch OpenROAD:** `COMPUTER_8B` (Basis-Blöcke, sky130) – 2.181 Zellen
      (971 and2, 457 inv, 453 or2, 136 dfxtp, …), Die 199 × 199 µm,
      43 % Auslastung, 32 mm Leitung (met1 16,5 mm, met2 14,4 mm, met3 0,8 mm,
      met4 0,5 mm), 11.467 Vias, 0 DRC-Verstöße, 24 s. Timing: −0,16 ns Slack bei
      10 ns Takt → knapp 98 MHz

**Erkenntnisse für Phase 2:**
- OpenROADs Verilog-Leser versteht `input wire [..]` nicht – Ports ohne `wire` deklarieren.
- Ohne `repair_design` treiben Minimalzellen Netze mit bis zu 131 Pins
  (`Instruction[0]`, Registeradresse). Option `--optimize` vorsehen, die Buffer
  einfügen und Zellen vergrößern darf (ändert dann die Netzliste).
- Der kritische Pfad läuft vom Opcode-Eingang über Steuerwerk und ALU in ein Register.

## Phase 2 – Platzierung und Verdrahtung (CLI)

- [x] Läufe unter `chip/<NAME>/<run>/` (Netzliste, Namensliste, DEF/ODB, Logs, Metriken);
      der Run-Name kodiert die Optionen (`basic-met3`, `nand-met5-u70-latches-opt`).
      Statt einer Konfigurationsdatei pro Design genügen CLI-Optionen
- [x] Keine Synthese: OpenROAD liest unsere Netzliste direkt; `chip report` vergleicht
      exportierte und platzierte Zellen (Differenz = Clock-Tree-Buffer, ggf. `--optimize`)
- [x] CLI: `chip build <NAME> [--level] [--keep-latches] [--max-layer met2..met5]
      [--clock-period NS] [--util %] [--die-size UM] [--optimize]` – exportiert, startet
      OpenROAD im gepinnten Container (`chip/flow/IMAGE`), sammelt die Ergebnisse
- [x] **Einstellbare Lagenzahl:** `--max-layer` begrenzt Signale, Power-Grid und Pins.
      Unter met5 baut `pnr.tcl` ein reduziertes Power-Grid (met1-Schienen + Streifen auf
      met4 bzw. met2) und legt die Pins auf erlaubte Lagen. Minimum ist met2 (met1 allein
      kann sky130 nicht sinnvoll verdrahten; `li1` liegt in den Zellen)
- [x] Kenngrößen: Die, Logikfläche, Zellen, Transistoren (aus der CDL-Netzliste der
      Zellbibliothek gezählt), Leitung pro Lage, Vias, DRC, Slack, fmax
- [x] CLI: `chip report <NAME> [--run RUN] [--json]` – Vergleichstabelle aller Läufe,
      Leitung pro Lage, Zellliste für einen Lauf, JSON für Skripte
- [x] `--optimize`: `repair_design` + `repair_timing` (Buffer, Zellgrößen)
- [x] Lagen-Experiment (COMPUTER_8B, Basis-Blöcke, 10-ns-Takt, 2026-11-09):

  | Lauf | Die | Leitung | Vias | DRC | fmax | Laufzeit |
  |---|---|---|---|---|---|---|
  | met5, 40 % | 199 µm | 31,7 mm | 11.516 | 0 | 98 MHz | 25 s |
  | met4, 40 % | 199 µm | 31,7 mm | 11.474 | 0 | 98 MHz | 21 s |
  | met3, 40 % | 199 µm | 31,7 mm | 11.459 | 0 | 98 MHz | 25 s |
  | met2, 40 % | 199 µm | 33,2 mm | 11.478 | 0 | 96 MHz | 49 s |
  | met5, 70 % | 153 µm | 31,7 mm | 11.715 | 0 | 99 MHz | 40 s |
  | met3, 70 % | 153 µm | 31,7 mm | 11.666 | 0 | 97 MHz | 40 s |
  | met2, 70 % | 153 µm | 32,1 mm | 11.662 | 0 | 97 MHz | 171 s |
  | met5 + `--optimize` | 199 µm | 32,4 mm | 11.523 | 0 | 108 MHz | 23 s |
  | NAND-Stufe, met5 | 230 µm | 45,0 mm | 21.029 | 0 | 83 MHz | 26 s |

  Ergebnis: Der Computer ist klein genug, dass schon **2 Lagen** DRC-frei reichen; 95 %
  der Leitung liegt ohnehin auf met1/met2. Weniger Lagen kosten vor allem Rechenzeit
  des Routers (bei 70 % Auslastung 4×) und etwas Leitung/Takt. Die NAND-Abbildung
  braucht 2,1× so viele Zellen, 1,5× so viele Transistoren (20.948 statt 13.852) und
  ist 15 % langsamer. Interessant würden die Lagen erst bei deutlich größeren Designs.

## Phase 3 – Layout in eigenes Format umwandeln

- [x] Eigene Parser ohne Zusatz-Crates: `src/layout/def.rs` (Komponenten, Pins,
      Power-Grid, Netze mit Routing; generierte VIARULE-Vias, Non-Default-Rules für
      Clock-Netze, Leitungsverlängerungen, RECT-Flicken), `lef.rs` (Lagen, Breiten,
      Dicken, Vias, Makros mit Pins), `gds.rs` (Strukturen, BOUNDARY/BOX/PATH)
- [x] Zellinneres aus dem GDS: nwell, diff, tap, poly, licon, li1, mcon und met1 –
      Transistoren sind als poly über diff erkennbar (Gate = Überlappung)
- [x] Plattformdateien (Tech-LEF, Zell-LEF, Zell-GDS) werden einmalig aus dem Image
      nach `chip/.platform/sky130hd/` kopiert
- [x] Layout-Format `layout.bin`: `LCLAYOUT`, Version, JSON-Kopf (Lagenstapel mit
      Höhe/Dicke/Farbe, Masters, Instanznamen und -pfade, Netze mit Simulationsschlüssel,
      Abschnittsindex), danach Int32-Abschnitte `master_rects`, `master_polys`,
      `instances` (Master, x, y, Orientierung), `routes` (Lage, Netz, Rechteck).
      Zellgeometrie steht nur einmal pro Master (gut für Instancing in Phase 4).
      `--json` schreibt dasselbe als JSON zum Debuggen
- [x] Netze auf Simulationsschlüssel zurückgeführt (Namensliste, Port-Bits,
      Clock-Tree-Netze → Takteingang); Instanzen auf Hierarchiepfade
- [x] CLI: `chip extract <NAME> [--run RUN] [-o DATEI] [--json]`
- [x] **Prüfungen:** jede exportierte Zelle ist platziert; jedes Signalnetz verbindet
      alle seine Pins (Union-Find über überlappende Formen, Vias verbinden die Lagen
      darunter und darüber, Zellpins aus dem LEF in Platzierungsorientierung)
- [x] Ergebnis COMPUTER_8B (`basic-met5`, 2026-11-09): 2,1 MB, 6.262 Instanzen
      (2.181 aus unserer Netzliste, Rest Füll-/Tap-/Clock-Zellen), 22 Masters,
      54.654 Routing-Formen, alle 2.223 Signalnetze einem Simulationsdraht zugeordnet,
      **alle 2.181 Zellen platziert und alle 2.216 Netze verbunden**; ebenso für alle
      anderen Läufe (2–5 Lagen, 70 %, NAND-Stufe, `--optimize`). Extraktion < 0,2 s
- [x] Rust-Tests: GDS-Reals und -Elemente, LEF-Lagen/Vias/Makros, DEF inkl.
      VIARULE-Geometrie und Orientierungen, Binärformat-Rundreise, echte Extraktion
      (übersprungen ohne gebauten Lauf)

## Phase 4 – 3D-Anzeige im Browser

- [x] Viewer-Modus „Chip 3D“ (`viz/web/chip3d.js`, three.js 0.170.0 über Import-Map,
      wird erst bei Bedarf geladen); `?mode=chip&explode=..&zscale=..&cut=..&power=1`
      in der URL setzt Modus und Ansicht
- [x] Layout-Datei laden: `viz/build.sh` kopiert `chip/<NAME>/basic-met5/layout.bin`
      (sonst den neuesten Lauf) nach `viz/web/layouts/<NAME>.bin`; zusätzlich Drag-and-drop
      einer `layout.bin` (wechselt die Simulation auf das Design, wenn es bekannt ist)
- [x] Geometrie: pro Zelltyp und Lage eine zusammengeführte Geometrie (Rechtecke als
      Quader, Polygone extrudiert), gezeichnet als InstancedMesh über alle Platzierungen
      inkl. Orientierung; Routing als instanzierte Einheitsquader mit Farbe pro Instanz.
      COMPUTER_8B: rund 175 Draw-Calls für 6.262 Zellen und 55.000 Leitungsstücke
- [x] Bedienung: Orbit/Zoom/Pan, Lagen einzeln ein-/ausblenden, Explode-Regler,
      Z-Überhöhung, Schnitt von oben (Clipping-Ebene), Power-Grid ein/aus (standardmäßig
      aus – die met4/met5-Streifen verdecken sonst die Logik), F/Einpassen
- [x] Hover über Leitung oder Zelle → Netzname, Simulationsschlüssel, Lage, Wert bzw.
      Zelltyp und Hierarchiepfad in der Statuszeile
- [x] **Live-Signale:** ein Probe über alle zugeordneten Netze; Leitungen von Netzen auf 1
      werden grün, sobald die Simulation (Programm-Panel) läuft
- [x] Netz hervorheben: Klick färbt ein Netz durch alle Lagen gelb und blendet den Rest
      ab, Esc hebt es auf
- [x] Kennzahlen (Lauf, Die, Zellen, Transistoren, Leitung, Vias, fmax) im Panel – sie
      reisen im Kopf von `layout.bin` mit (`chip extract` übernimmt sie aus dem Lauf)
- [x] Ohne WebGL (z. B. im eingebetteten Browser der Claude-App) erscheint ein Hinweis;
      geprüft per Headless-Chromium mit Software-WebGL (SwiftShader)

## Phase 5 – Ausgabe in der CLI

- [x] `chip render <NAME> [--run] [--layers met1,met2|all] [--format svg|png] [-o DATEI]`:
      Draufsicht der gewählten Lagen, gleiche Farben wie im Browser
      (`src/layout/render.rs`: Polygone pro Lage → SVG-Pfade bzw. eigener
      Scanline-Rasterizer mit Alpha-Blending, PNG über das `png`-Crate)
- [x] `chip render --iso [--explode µm] [--zscale x] [--power]`: isometrische Ansicht,
      Lagen von unten nach oben gestapelt, je Form eine dunklere Unterseite als Kante
- [x] `chip view <NAME> [--run] [--port] [--no-open]`: legt das Layout unter
      `viz/web/layouts/` ab (extrahiert bei Bedarf), startet `viz-serve`, falls der Port
      frei ist, und öffnet `…/index.html?mode=chip#NAME` im Browser
- [x] Alle Kennzahlen auch als JSON: `chip report --json`
- [x] COMPUTER_8B: Draufsicht als PNG (1600 px) in < 0,1 s, isometrisch mit allen Lagen
      1,9 MB PNG bzw. 25 MB SVG

## Phase 6 – Optional / später

- [ ] Eigenen Platzierer und Router in Rust (Labyrinth-Router mit
      Negotiated Congestion auf einem 3D-Gitter) – läuft dann auch komplett im Browser
- [x] Transistoransicht ohne PDK (Ansicht „Transistoren“): jedes NAND als CMOS-Zelle –
      PMOS A/B parallel von VDD zum Ausgang, NMOS A/B in Reihe nach GND; leitende
      Transistoren leuchten live (NMOS bei Gate 1, PMOS bei Gate 0), Ausgangsknoten und
      Gate-Leitungen tragen ihren Wert. Bis 1.500 NANDs (6.000 Transistoren), darüber
      Hinweis auf eine tiefere Ebene
- [ ] TinyTapeout prüfen: Passt `COMPUTER_8B` (oder eine kleinere Variante)
      in die verfügbaren Kacheln? Dann ließe sich der Chip tatsächlich fertigen
- [ ] Größenvergleich: NAND-only-Mapping gegen optimierte Zellen (Fläche, Transistoren, Takt)
