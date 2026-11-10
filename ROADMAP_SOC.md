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

### Phase 3 – Memory-mapped I/O (2026-11-10)
- [x] Adressseite 0xF0xx für Peripherie (Stack wächst von 0xFFFF nach unten,
      darf also nicht unter 0xF100 wachsen); die Register wiederholen sich
      alle 8 Wörter:

  | Adresse | Register | Zugriff |
  |---|---|---|
  | 0xF000 | GPIO_OUT | lesen/schreiben, treibt `GpioOut` |
  | 0xF001 | GPIO_IN | lesen (`GpioIn`-Pins) |
  | 0xF002 | DISPLAY | lesen/schreiben, Wert der 4-stelligen Anzeige |
  | 0xF003 | UART_DATA | schreiben: Byte senden |
  | 0xF004 | UART_STATUS | lesen: Bit 0 = sendet noch |
  | 0xF005 | UART_DIV | lesen/schreiben: Takte pro Bit − 1 |
  | 0xF006 | TIMER | lesen: Zählerstand, schreiben: Startwert (startet neu) |
  | 0xF007 | TIMER_CTRL | Bit 0 an, Bit 1 Interrupt an, Bit 2 abgelaufen (nur lesen); Schreiben quittiert |

- [x] `UART_TX` (2.330 NANDs): 8N1, Teiler-Register (nach Reset 0 = ein Takt
      pro Bit; 50 MHz und 433 ergeben 115200 Baud), Zähler, Schieberegister
- [x] `IO_16B` (4.506 NANDs): Adressdecoder, GPIO- und Anzeige-Register,
      UART, Lese-Mux. Lesen kombinatorisch, Schreiben mit der Taktflanke,
      nie Wartezustände
- [x] `SOC_16B` (30.288 NANDs): `CPU_16B_SOC` + `IO_16B` +
      `HEX_DISPLAY_16B`. Speicherbus-Pins wie `COMPUTER_16B_SOC`; Zugriffe auf
      die I/O-Seite beantwortet der Chip selbst (`Write` bleibt außen 0,
      `Wait` wird ignoriert). `IO_16B` wird vor der CPU gebaut, sonst würden
      seine INIT-Takte die fertige CPU laufen lassen
- [x] Runner: liest die `Tx`-Leitung jeden Takt und dekodiert Bytes
      (`Machine::uart`; den Teiler liest er aus `Io.Uart.Div`, wie ein echter
      Empfänger die Baudrate kennt), `--input` setzt auch `GpioIn`; `run`
      zeigt `GpioOut` und den UART-Text. Beispiel `programs/soc/hello.asm`
      (Standard-Top `SOC_16B` für Programme in `programs/soc/`): zeigt
      0xC0DE, sendet „Hello!“, spiegelt GPIO
- [x] Tests: `UART_TX.clt` (Rahmen Bit für Bit, Teiler 0 und 1),
      `IO_16B.clt` (Decoder, Register, Status), `SOC_16B.clt` (I/O-Programm
      mit `GpioOut`/Anzeige/`Tx` nach jedem Takt, auch mit 2 Wartetakten);
      Rust-Test `soc_peripherals`, alle Programmtests auch auf `SOC_16B`

### Zwischenschritt – Würde der Chip funktionieren? (2026-11-10)
- [x] Prüfung am Layout `SOC_16B/basic-met5-u70-opt` (OpenSTA, typische
      Ecke tt_025C_1v80): mit aus dem Layout extrahierten Parasiten
      (OpenRCX) −0,18 ns bei 10 ns, also ≈ 98 MHz (der Flow meldet 100 MHz mit
      geschätzten Parasiten). Hold zwischen Registern erfüllt (+0,53 ns);
      Hold-Verletzungen nur vom `Reset`-Pin (Eingangsverzögerung 0 ns
      angenommen). ~190 knappe Flanken-/Lastverletzungen an den Lese-Muxen
      von `IO_16B`. Langsame/schnelle Ecken nicht geprüft: das Docker-Image
      enthält nur die typische Bibliothek
- [x] `SYNC<W>` (`src/circuits/memory`): Zwei-Flipflop-Synchronisierer.
      `SOC_16B` synchronisiert `Reset`, `GpioIn` und `In` (zwei Takte Verzug,
      Reset mindestens drei Takte halten); der Speicherbus bleibt synchron zu
      `CLK`. 32.070 NANDs
- [x] Runner: `Machine::reset` (Einschalt-Reset: drei Takte `Reset` mit
      angelegten Eingängen, ohne Speicherschreiben), `run` und die Tests rufen
      ihn auf
- [x] `verify-verilog --x-init`: jede Zelle startet „unbekannt“ wie nach dem
      Einschalten; eine Suite besteht nur, wenn ihre Tests zuerst
      zurücksetzen. `SOC_16B`, `COMPUTER_16B_SOC`, `CPU_16B_SOC`,
      `CPU_16B_PIPE3`, `UART_TX` bestehen: nach dem Reset ist alles definiert
- [ ] Parasiten-Extraktion und Hold-Bericht fest in `chip/flow/pnr.tcl`
- [ ] Langsame/schnelle Ecken (Bibliotheken ss/ff aus dem sky130-PDK)
- [ ] Signoff (DRC mit Magic/KLayout, LVS, Antennen) – übernimmt bei einer
      Einreichung der TinyTapeout-Flow

### Ausbau A – schneller rechnen, mehr können (2026-11-10)

Reihenfolge: Benchmarks, MUL/DIV, Sprungvorhersage und kritischer Pfad,
Timer und Interrupts, UART-Empfang und Byte-Zugriffe. Ziel-CPU ist die
SoC-Linie (`CPU_16B_SOC`, `SOC_16B`).

**A1 – Befehlssatz-Simulator und Benchmarks**
- [x] `isa::sim16::Cpu16`: führt den CPU_16B-Befehlssatz direkt in Rust aus
      (ein gemeinsamer Speicher, zählt Befehle). Referenz für die Schaltungen:
      Rust-Test `isa_simulator_matches_circuit` vergleicht `Out`, SP, Flags
      und Datenspeicher mit COMPUTER_16B auf allen Beispielprogrammen und
      Benchmarks
- [x] Benchmarks `programs/bench/`: `mul` (Software-Multiplikation),
      `div` (Software-Division, Ziffernsumme), `branch` (Bits zählen),
      `memory` (füllen, kopieren, summieren), `calls` (rekursives fib(9))
- [x] `cargo run -- bench [--top A,B] [--mem-latency N] [--isa-only]`:
      Befehle im Simulator, Takte auf jedem Rechner, Ergebnis muss stimmen
- [x] Ausgangswerte (Takte, in Klammern Takte pro Befehl):

  | Benchmark | Befehle | COMPUTER_16B | COMPUTER_16B_PIPE3 | SOC_16B |
  |---|---|---|---|---|
  | branch | 1.146 | 1.146 (1,00) | 1.388 (1,21) | 1.390 (1,21) |
  | calls | 925 | 925 (1,00) | 1.308 (1,41) | 1.744 (1,89) |
  | div | 1.989 | 1.989 (1,00) | 2.557 (1,29) | 2.592 (1,30) |
  | memory | 778 | 778 (1,00) | 1.016 (1,31) | 1.118 (1,44) |
  | mul | 358 | 358 (1,00) | 434 (1,21) | 488 (1,36) |
  | **gesamt** | 5.196 | 5.196 (1,00) | 6.703 (1,29) | 7.332 (1,41) |

  SOC_16B mit 2 Wartetakten pro Speicherzugriff: 19.866 Takte (3,82).
  Bei `calls` kosten RET (zwei Takte), Zwei-Wort-CALL und Stack-Zugriffe
  auf dem gemeinsamen Bus am meisten.

**A2 – Multiplikation und Division in Hardware**
- [x] `MULDIV_16B` (3.704 NANDs): mehrtaktig, ein Bit pro Takt, 16 Schritte;
      Multiplikation durch Schieben und Addieren, Division restaurierend
      (Schieben und Subtrahieren), beide über denselben Kogge-Stone-Addierer
      und das 32-Bit-Schieberegister Hi:Lo. Division durch 0: Quotient
      0xFFFF, Rest = Dividend
- [x] Befehle (Opcode 7, bisher reserviert): `MULL`, `MULH` (unteres/oberes
      Wort des vorzeichenlosen Produkts), `DIVU`, `MODU`; Z/N aus dem
      Ergebnis. Assembler, Disassembler, `isa::sim16`
- [x] `CPU_16B_SOC`: der Befehl startet die Einheit in Execute und wartet
      dort (`MdWait`, Steuerwerk sieht NOP, PC und Befehlsregister halten),
      bis `Done`; dann schreibt er das Ergebnis über den freien Eingang 2
      des Write-back-Muxes. Operanden kommen über das Forwarding, ein
      Load-Use-Stall verzögert den Start. Die anderen CPUs behandeln
      Opcode 7 weiter als NOP. SOC_16B: 35.947 NANDs (+3.877)
- [x] Tests: `MULDIV_16B.clt` (10 Fälle inkl. Division durch 0, Takt für
      Takt), MD-Programm in `CPU_16B_SOC.clt` (Referenz: `isa::sim16`), Rust:
      `isa_simulator_matches_circuit` jetzt auch auf SOC_16B (inkl.
      Speicher), Benchmarks `mul_hw`, `div_hw` (`bench` zeigt „–“ auf CPUs
      ohne Einheit)
- [x] Ergebnis SOC_16B: `div` 2.592 → `div_hw` 593 Takte (4,4×), `mul` 488
      → `mul_hw` 326 Takte (1,5×; kleine Faktoren, Schleife und Ladebefehle
      bleiben). Ein MUL/DIV kostet 18 Takte

**A3 – Sprungvorhersage und kritischer Pfad**
- [x] Kritischer Pfad: Das Steuerwerk dekodiert das Befehlsregister jetzt
      direkt; beim Warten (`HoldE` = Load-Use-Stall, `Wait`, `NeedNext`,
      `MdWait`) werden nur die Wirkungen gesperrt (Register- und
      Speicherschreiben, Flags, SP, `Out`, Sprünge und Branches, je ein AND
      mit NOT HoldE). Vorher lag die Hazard- und Warte-Logik vor dem
      Steuerwerk, vor dem langen Pfad Decoder → ALU → Flags
- [x] Statische Sprungvorhersage im Fetch: ein geholter Bcc mit negativem
      Offset (Schleife) oder BRA (nicht BNV) gilt als genommen; der Fetch
      springt sofort (`PredAdder`: PC + 1 + Offset) und merkt sich die
      Folgeadresse (`AltPc`) und die Vorhersage (`PredBit`). In Execute:
      vorhergesagt und genommen kostet nichts, vorhergesagt und nicht
      genommen lädt `AltPc` (ein Takt), genommen ohne Vorhersage wie bisher.
      `PC_FETCH_16B` gibt dafür `Next` (PC + 1 + Step2) aus
- [x] SOC_16B: 37.308 NANDs (+1.361). Tests: Vorhersage-Programm in
      `CPU_16B_SOC.clt` (Schleifen mit falscher Vorhersage am Ende, BRA,
      BNV rückwärts, LI am Sprungziel; auch mit 2 Wartetakten), alle
      SoC-Suiten neu erzeugt; `verify-verilog` 97 Suiten in 4 Varianten
      und `--x-init` grün
- [x] Ergebnis SOC_16B (Takte): branch 1.390 → 1.231, memory 1.118 → 980,
      div 2.592 → 2.396, mul 488 → 455, calls 1.744 → 1.744 (RET/CALL, keine
      Vorhersage), fibonacci-Test 146 → 124; Software-Benchmarks gesamt
      7.332 → 6.806 (−7 %)
- [x] Takt (chip build, basic, met5, 70 %, `--optimize`, `--mem-delay 3`,
      Zielperiode 7 ns): vorher 110 MHz (Pfad `CtrlM` → Bus → I/O-Decoder →
      Wait → Freeze → Hold → Steuerwerk-Eingang → ALU → Flags), nachher
      **138 MHz (+25 %)** (Pfad Befehlsregister → Registerfile → Forwarding →
      Schieber → Write-back → Z → Flags). Mit den weniger Takten ≈ 1,35× so
      schnell

**A4 – Timer und Interrupts**
- [x] `TIMER_16B` (1.867 NANDs) in `IO_16B`: 0xF006 `TIMER` (Startwert,
      Zählerstand), 0xF007 `TIMER_CTRL` (an, Interrupt an, abgelaufen;
      Schreiben quittiert). Zählt pro Takt herunter, bei 0 neu laden und
      `Expired` setzen: alle Startwert + 1 Takte. `Irq` = abgelaufen und
      Interrupt an (Pegel)
- [x] Interrupts in `CPU_16B_SOC` (Eingang `Irq`, in `SOC_16B` vom Timer):
      steht `Irq` an, sind Interrupts erlaubt (`Ie`) und liegt in Execute
      ein echter Befehl (`IrValid`), der gerade laufen könnte (kein HALT,
      kein Halten, kein NeedNext, kein MUL/DIV), wird statt dieses Befehls
      ein `CALL 0x0002` ausgeführt – Rücksprungadresse ist die Adresse des
      unterbrochenen Befehls (`PcE`), die Flags gehen nach `FlagSave`, `Ie`
      wird gelöscht. Programme beginnen deshalb mit einem `JMP` (zwei Wörter)
- [x] Neue Befehle als Varianten bestehender Kodierungen, damit das
      Steuerwerk unverändert bleibt: `EI` = NOP mit Rd = 1 (0x0101), `DI` =
      NOP mit Rd = 2 (0x0201), `RETI` = RET mit Rd = 1 (0x0102, stellt die
      Flags wieder her und erlaubt Interrupts). Auf den anderen CPUs NOP
      bzw. RET. Assembler, Disassembler, `isa::sim16` (mit `irq`-Eingang)
- [x] Beispiel `programs/soc/timer.asm`: Handler zählt Ticks auf der
      Anzeige, das Hauptprogramm wartet mit CMPI/BLT auf 8
- [x] Tests: `TIMER_16B.clt`, Timer in `IO_16B.clt`, Interrupt-Programm in
      `SOC_16B.clt` (32-Bit-Addition mit ADD/ADC, während der Handler die
      Flags verändert); Rust `soc_timer_interrupts` mit 0, 1 und 3
      Wartetakten (Gegenprobe: ohne Flag-Wiederherstellung kommt mit 3
      Wartetakten 59 statt 64 heraus), `isa::sim16`-Test für Eintritt und
      RETI. SOC_16B: 40.504 NANDs (+3.196)
- [ ] Bekannt: ist der Handler länger als die Timer-Periode, kommt das
      Hauptprogramm nicht mehr voran (der nächste Interrupt ist schon
      fällig, bevor nach RETI ein Befehl läuft). Die Periode muss zum
      Handler und zur Speicherlatenz passen

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
