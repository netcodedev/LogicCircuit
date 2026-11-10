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

### Phase 2 – Gemeinsamer Speicher
- [ ] `CPU_16B_SOC`: ein Speicherbus für Befehle und Daten (von-Neumann),
      Speicherstufe hat Vorrang vor dem Fetch
- [ ] Kein `InstructionNext` mehr: das zweite Wort von LI/JMP/CALL wird in
      einem eigenen Fetch geholt
- [ ] Runner: ein gemeinsamer Speicher mit Latenz; Programme und Daten im
      selben Adressraum

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
