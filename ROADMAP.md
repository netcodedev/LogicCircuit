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

- [ ] `src/export/verilog.rs`: strukturelles Verilog aus `flatten()`
  - [ ] ein Modul pro Top-Level-Schaltung, Ports = CLDL-Ein- und Ausgänge (Busse als Vektoren)
  - [ ] ein `wire` pro Netz; Namen aus unseren Netzschlüsseln ableiten
        (`Cpu.Datapath.Alu.FA0.sum` → gültiger Verilog-Bezeichner), damit wir
        Layout-Netze später wieder unseren Simulationsdrähten zuordnen können
  - [ ] Zellinstanzen mit Instanznamen = Hierarchiepfad (für die Rückverfolgung)
  - [ ] nicht getriebene Netze (lesen 0) an eine Tie-Low-Zelle hängen (`sky130_fd_sc_hd__conb_1`)
- [ ] Zwei Zellbibliotheken:
  - [ ] `generic`: eigene Verilog-Module `LC_NAND2`, `LC_NOT`, `LC_AND2`, … (zum Simulieren mit Icarus/Verilator)
  - [ ] `sky130`: Zuordnung auf `sky130_fd_sc_hd`-Zellen
        (NAND→`nand2_1`, NOT→`inv_1`, AND→`and2_1`, AND3→`and3_1`, OR→`or2_1`,
        NOR→`nor2_1`, NOR4→`nor4_1`, XOR→`xor2_1`, XNOR→`xnor2_1`;
        Mehrbit-Blöcke wie `AND_4B` bitweise auflösen)
- [ ] CLI: `export-verilog <NAME> [--level nand|basic] [--cells generic|sky130] [-o DATEI]`
- [ ] Browser: Button „Verilog herunterladen“ für die aktuelle Ebene (läuft komplett in WASM)
- [ ] **Gegenprobe:** aus den `.clt`-Tests eine Verilog-Testbench erzeugen
      (`export-testbench`) und den exportierten Code mit Icarus Verilog
      laufen lassen; Ergebnisse müssen mit unserem Simulator übereinstimmen
- [ ] Rust-Tests: Export ist deterministisch, jede Zelle hat alle Pins verbunden

## Phase 2 – Platzierung und Verdrahtung (CLI)

- [ ] Verzeichnis `chip/` mit OpenLane-Konfiguration (`config.json`) pro Design
- [ ] Synthese überspringen bzw. Netzliste unverändert übernehmen, damit Yosys
      unsere Struktur nicht wegoptimiert (vorab gemappte Netzliste, `keep`-Attribute,
      prüfen, dass die Zellzahl gleich bleibt)
- [ ] CLI: `chip build <NAME>` = Verilog exportieren → Konfiguration schreiben →
      OpenLane (Docker/Nix) aufrufen → Ergebnisse nach `chip/<NAME>/run/` legen
- [ ] **Einstellbare Lagenzahl:** `--max-layer met1..met5` (OpenLane
      `RT_MAX_LAYER`). sky130 hat `li1` plus `met1`–`met5`; weniger Lagen ergeben
      größere Fläche und längere Leitungen
- [ ] Kenngrößen aus den Reports einsammeln: Fläche, Zellzahl, Transistorzahl,
      Leitungslänge pro Lage, Via-Anzahl, kritischer Pfad / maximale Taktfrequenz
- [ ] CLI: `chip report <NAME>` gibt diese Kennzahlen als Tabelle aus
- [ ] Experiment dokumentieren: dieselbe Schaltung mit 2, 3, 4, 5 Lagen vergleichen

## Phase 3 – Layout in eigenes Format umwandeln

- [ ] Parser für DEF (Platzierung, Verdrahtung, Netznamen) und LEF (Zellgrößen, Pins)
- [ ] GDS lesen (z. B. Crate `gds21`), um die Zellinnereien zu bekommen:
      Diffusion, Polysilizium, `li1`, Kontakte – daraus sind die Transistoren erkennbar
- [ ] Eigenes kompaktes Layout-Format (JSON zum Debuggen, binär für große Designs):
      Lagen mit Höhe/Dicke, Rechtecke pro Lage, Vias, Zellinstanzen, Netz-ID je Form
- [ ] Netz-IDs auf unsere Simulationsschlüssel zurückführen (über die Namen aus Phase 1)
- [ ] CLI: `chip extract <NAME> -o layout.bin`
- [ ] Rust-Tests: jede Zelle der Netzliste ist im Layout, jedes Netz ist verbunden

## Phase 4 – 3D-Anzeige im Browser

- [ ] Neuer Viewer-Modus „Chip 3D“ (three.js, Version pinnen)
- [ ] Layout-Datei laden: zur Build-Zeit einbetten und per Drag-and-drop
- [ ] Geometrie pro Lage zusammenfassen (Instanced/Merged Meshes), damit
      Zehntausende Formen flüssig laufen
- [ ] Bedienung: Orbit/Zoom, Lagen einzeln ein-/ausblenden, „Explode“-Regler
      zieht die Lagen auseinander, Schnitt in Z-Richtung
- [ ] Hover/Klick auf Leitung oder Zelle → Netzname, Hierarchiepfad, aktueller Wert
- [ ] **Live-Signale:** Simulation (Programm-Panel) läuft weiter; Netze auf 1 leuchten im Chip
- [ ] Netz hervorheben: ein Signal durch alle Lagen und Vias verfolgen
- [ ] Kennzahlen aus Phase 2 im Panel anzeigen

## Phase 5 – Ausgabe in der CLI

- [ ] `chip render <NAME> [--layers met1,met2] [--format svg|png] [-o DATEI]`:
      Draufsicht pro Lage oder übereinander, gleiche Farben wie im Browser
- [ ] `chip render --iso`: isometrische Ansicht als Bild (Lagen versetzt gestapelt)
- [ ] `chip view <NAME>`: startet `viz-serve` und öffnet den 3D-Viewer mit diesem Layout
- [ ] Alle Kennzahlen auch als JSON (`--json`) für Skripte

## Phase 6 – Optional / später

- [ ] Eigenen Platzierer und Router in Rust (Labyrinth-Router mit
      Negotiated Congestion auf einem 3D-Gitter) – läuft dann auch komplett im Browser
- [ ] Transistoransicht ohne PDK: jedes NAND als CMOS-Zelle mit 4 Transistoren
      selbst zeichnen (Lehrmodus)
- [ ] TinyTapeout prüfen: Passt `COMPUTER_8B` (oder eine kleinere Variante)
      in die verfügbaren Kacheln? Dann ließe sich der Chip tatsächlich fertigen
- [ ] Größenvergleich: NAND-only-Mapping gegen optimierte Zellen (Fläche, Transistoren, Takt)
