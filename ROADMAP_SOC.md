# Roadmap: vom 16-Bit-Kern zum eigenen Board (SoC)

Fernziel: der eigene Chip auf einem selbst entworfenen Motherboard, mit
verlötetem Speicher, GPIO, echten 7-Segment-Anzeigen, UART und Ethernet.
Ausgangspunkt ist `CPU_16B_PIPE3` (3-stufige Pipeline, siehe
`ROADMAP_16BIT.md`). Die ersten Schritte laufen im Simulator und brauchen
noch keine Hardware.

## Rahmenbedingungen (Stand 2026-11-10)

- **Pins:** SDRAM braucht ~38 Pins (13 Adresse, 2 Bank, 16 Daten,
  CLK/CKE/CS/RAS/CAS/WE/DQM). TinyTapeout hat 24 I/Os, größere Shuttles
  eher ~38. Für einen ersten Chip realistischer: QSPI-Flash (Programm) und
  QSPI-PSRAM (Daten) mit zusammen ~10 Pins; SDRAM erst mit genug Pins.
- **Caches:** In unserer NAND-Welt kostet ein Bit ~35 NANDs (das
  Registerfile mit 32 Byte hat ~12.000). Ein L1 mit 256 Byte wären ~70.000
  NANDs – echte Caches brauchen SRAM-Makros (z. B. OpenRAM für sky130), die
  der Chip-Flow noch nicht platzieren kann.
- **Ethernet:** nicht selbst bauen, sondern einen W5500 per SPI anschließen
  (TCP/IP im Baustein). Ein eigener MAC mit PHY wäre ein eigenes Projekt.
- **Zwischenschritt FPGA:** dasselbe System erst auf einem FPGA-Board mit
  SDRAM, 7-Segment-Anzeigen und GPIO laufen lassen (Verilog-Export gibt es),
  bevor ein Chip gefertigt wird.

## Phasen

### Phase 1 – Wartefähiger Speicherbus (2026-11-10)
- [x] `IWait`/`DWait` an `CPU_16B_PIPE3` (aktiv-high, damit unverbundene
      Eingänge „bereit“ bedeuten und alles Bisherige gleich bleibt), Ausgang
      `MemRead` (LW, POP, RET in der Speicherstufe)
- [x] `IWait`: Befehl noch nicht da → PC hält, Execute bekommt eine Blase;
      ein Sprung lädt den PC trotzdem
- [x] `DWait` während `MemRead`/`MemWrite`: die ganze Pipeline steht
      („Freeze“: Steuerwerk sieht NOP wie beim Load-Use-Stall, die
      Pipeline-Register und das Registerfile schreiben nicht, `MemAddr`,
      `MemDataOut`, `MemWrite` bleiben stehen). 24.801 NANDs (+~50)
- [x] Runner: `Machine::latency` bzw. `run --mem-latency N` = N Wartetakte
      pro Zugriff. Ein fertig geholtes Befehlswort bleibt gültig, solange der
      PC steht; ein fertiger Datenzugriff ist mit der Taktflanke verbraucht.
      Fibonacci: 193 / 362 / 700 Takte bei 0 / 1 / 3 Wartetakten
- [x] Tests: Rust-Test `slow_memories` (0, 1, 3 Wartetakte: gleiche
      Ergebnisse, mehr Takte); `CPU_16B_PIPE3.clt` und
      `COMPUTER_16B_PIPE3.clt` enthalten je ein Programm zusätzlich mit 2
      Wartetakten pro Zugriff (`IWait`/`DWait` im Testharness)

### Phase 2 – Gemeinsamer Speicher (2026-11-10)
- [x] `CPU_16B_SOC` / `COMPUTER_16B_SOC`: ein Speicherbus für Befehle und
      Daten (von Neumann): `Addr`, `DataIn`, `DataOut`, `Write`, `Wait`,
      dazu `Fetch` (1 = Befehlsholen). Die Speicherstufe hat Vorrang; in dem
      Takt bekommt Execute eine Blase (wie bei `IWait`). `Wait` während eines
      Zugriffs der Speicherstufe hält die ganze Pipeline
- [x] Kein `InstructionNext` mehr: Erreicht LI/JMP/CALL die Execute-Stufe,
      wartet der Befehl einen Takt (`NeedNext`, Steuerwerk sieht NOP), bis
      das Folgewort in `IrNext` steht. Der PC zeigt dann hinter das zweite
      Wort, Rücksprungadresse und Sprungziel bleiben wie bei der Pipeline.
      Der Fetch-Vor-Decoder fällt weg
- [x] `IR_WORD_16B` (Befehlsregister für ein Wort, aus `IR_16B` gelöst;
      `IR_16B` nutzt es, gleiche NAND-Zahl)
- [x] 25.649 NANDs (COMPUTER_16B_SOC; COMPUTER_16B_PIPE3: 25.442 vor Phase 1)
- [x] Runner: ein gemeinsamer Speicher (`rom` = Inhalt ab Adresse 0,
      geschriebene Wörter in `ram`), Latenz pro Buszugriff. Takte
      gegenüber PIPE3 (getrennte Speicher, ohne Latenz): factorial 610 → 689,
      sort 401 → 478, multiply 79 → 87, fibonacci 193 → 193 – jeder
      Speicherzugriff blockiert einen Fetch, LI/JMP/CALL kosten einen Takt
- [x] Tests: alle Rust-Programmtests auch auf COMPUTER_16B_SOC, mit 0/1/3
      Wartetakten, plus ein Programm, das Code in den Speicher schreibt und
      anspringt; `CPU_16B_SOC.clt`/`COMPUTER_16B_SOC.clt` (Testharness spielt
      den Speicher, prüft jeden Buszugriff; Endzustand gleich `CPU_16B`; ein
      Programm zusätzlich mit 2 Wartetakten)

### Phase 3 – Memory-mapped I/O
- [ ] Adressseite 0xF0xx für Peripherie (Stack wächst von 0xFFFF nach unten,
      darf also nicht unter 0xF100 wachsen)
- [ ] GPIO (16 Ausgänge, 16 Eingänge), Anzeige-Register für die
      4-stellige 7-Segment-Anzeige, UART-Sender mit Baudraten-Teiler
- [ ] `SOC_16B`: CPU + Adressdecoder + Peripherie, Pins für externen
      Speicher, GPIO, Anzeige, UART
- [ ] Runner dekodiert die UART-Leitung und gibt den Text aus

### Phase 4 – Speicher-Controller
- [ ] QSPI-Controller (Flash lesen, PSRAM lesen/schreiben), Modell im Runner
- [ ] später ggf. SDRAM-Controller (Init, Refresh, Bänke, CAS-Latenz)

### Phase 5 – Caches
- [ ] L1-I (direkt abgebildet), dann L1-D (write-through)
- [ ] SRAM-Makros im Chip-Flow (OpenRAM) statt Flipflops

### Phase 6 – System
- [ ] Boot-ROM: kopiert beim Reset das Programm aus dem Flash
- [ ] Interrupts (UART, Timer, Netzwerk), Timer
- [ ] SPI-Master, W5500-Anbindung, UART-Empfänger
- [ ] Multiplexte 7-Segment-Ansteuerung für echte Anzeigen

### Phase 7 – Hardware
- [ ] FPGA-Prototyp mit echten Peripherien
- [ ] IO-Pads, Pegel (1,8 V Kern / 3,3 V Board), Takt aus Quarz, Reset
- [ ] Board-Design (KiCad), Tapeout-Weg auswählen
