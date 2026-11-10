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
  | 0xF002 | DISPLAY | lesen/schreiben, Wert der 4-stelligen Anzeige; der Befehl `OUT` schreibt es ebenfalls |
  | 0xF003 | UART_DATA | schreiben: Byte senden |
  | 0xF004 | UART_STATUS | lesen: Bit 0 = sendet noch, Bit 1 = Byte empfangen |
  | 0xF005 | UART_DIV | lesen/schreiben: Takte pro Bit − 1 |
  | 0xF006 | TIMER | lesen: Zählerstand, schreiben: Startwert (startet neu) |
  | 0xF007 | TIMER_CTRL | Bit 0 an, Bit 1 Interrupt an, Bit 2 abgelaufen (nur lesen); Schreiben quittiert |
  | 0xF008 | UART_RX | lesen: empfangenes Byte (quittiert) |
  | 0xF009 | UART_CTRL | Bit 0 = Interrupt bei empfangenem Byte |

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

**A5 – UART-Empfang und Byte-Zugriffe**
- [x] `UART_RX` (2.343 NANDs): 8N1 mit demselben Teiler wie der Sender;
      tastet jedes Bit in der Mitte ab (erstes Datenbit Div + Div/2 Takte
      nach dem Start-Bit, dann alle Div + 1). In `IO_16B`: 0xF008 `UART_RX`
      (Lesen quittiert), Status-Bit 1 in `UART_STATUS`, 0xF009 `UART_CTRL`
      (Empfangs-Interrupt); `Irq` = Timer oder Empfang. Die Register
      wiederholen sich jetzt alle 16 Wörter
- [x] `SOC_16B`: Pin `Rx` (Ruhepegel 1, über `SYNC`), `DataRead` (weder
      Fetch noch Schreiben) zum Quittieren beim Lesen
- [x] `LB`/`SB` (Opcode 15, `F Rd Rs s off3`): Byte-Adresse Rs + off3, Wort =
      Adresse / 2, gerades Byte = unteres. In `CPU_16B_SOC` lokal dekodiert
      (das Steuerwerk sieht einen reservierten Code): ALU rechnet die
      Adresse wie bei LW (A = off3), die Speicherstufe nimmt das richtige
      Byte (LB, vorzeichenlos) bzw. SB legt das Byte auf beide Hälften und
      setzt `ByteEn` (11 Wort, 01 unten, 10 oben). Assembler,
      Disassembler, `isa::sim16`
- [x] Runner: `Rx` wird bedient (`Machine::uart_in`, `run --uart-in "…\n"`,
      100 Takte Pause nach dem Reset, damit das Programm den Teiler setzen
      kann), `ByteEn` beim Schreiben. Viewer: Eingabefeld in der
      UART-Konsole, Einschalt-Reset nach dem Bauen
- [x] Beispiel `programs/soc/echo.asm`: Zeile empfangen, mit SB puffern,
      rückwärts in Großbuchstaben mit LB zurücksenden
- [x] Tests: `UART_RX.clt` (Rahmen bei drei Bitzeiten), Empfang in
      `IO_16B.clt`, BYTE-Programm in `CPU_16B_SOC.clt` (Referenz
      `isa::sim16`); Rust: Byte-Programm gegen `isa::sim16`, `soc_uart_echo`
      (0 und 1 Wartetakt), `soc_uart_receive_interrupt`. SOC_16B: 44.101
      NANDs (+3.597)

**Stand nach A1–A5 (2026-11-10)**

| | vor A1 | nach A5 |
|---|---|---|
| NANDs (SOC_16B) | 32.070 | 44.101 |
| Transistoren | 64.990¹ | 80.901 |
| Die (70 %) | 321 µm¹ | 356 µm |
| fmax (7 ns Ziel, 3 ns Speicher, `--optimize`) | 110 MHz¹ | 138 MHz |
| Takte Software-Benchmarks | 7.332 | 6.806 |
| Division (Ziffernsumme) | 2.592 Takte | 579 Takte (`DIVU`/`MODU`) |

¹ nach A2 (mit MUL/DIV-Einheit), vor A3.

Neu: Hardware-Multiplikation und -Division, Sprungvorhersage, Timer,
Interrupts, UART-Empfang, Byte-Zugriffe. Der kritische Pfad bleibt
Befehlsregister → Registerfile → Forwarding → Schieber → Write-back → Z →
Flags (jetzt über den Flag-Mux für RETI). Für TinyTapeout wären es bei 70 %
Auslastung etwa 4×2 Tiles (≈ 560 € plus Devkit), noch ohne
QSPI-Wrapper für die Pins.

### Ausbau B – Befehls-Cache (2026-11-10)

Messung vorab (`isa::sim16`, alle Benchmarks): 5.453 Befehle brauchen auf
SOC_16B 7.699 Takte. Fast alle verlorenen Takte kommen vom gemeinsamen Bus,
der jeden Takt belegt ist: 744 Speicherzugriffe und 150 zweite Befehlswörter
brauchen je einen eigenen Buszyklus. Ein reiner Vorauslese-Puffer (FIFO)
hätte kaum freie Buszyklen zum Füllen (nur während MUL/DIV und Stalls), also
~1–3 %. Ein kleiner Cache liefert Befehle dagegen ohne Bus. Trefferquote
direkt abgebildet, ein Wort pro Eintrag:

| Größe | mul | div | branch | memory | calls |
|---|---|---|---|---|---|
| 8 Wörter | 40 % | 41 % | 83 % | 96 % | 42 % |
| 16 Wörter | 62 % | 87 % | 99 % | 96 % | 87 % |
| 32 Wörter | 88 % | 98 % | 99 % | 96 % | 98 % |

- [x] `ICACHE_16` (22.784 NANDs): 16 Wörter, direkt abgebildet (Index =
      Adresse[3..0], Tag = Adresse[15..4]), in Flipflops. Lesen
      kombinatorisch an der PC-Adresse (`Hit`, `Word`). Jedes Wort, das der
      Bus dem Fetch ohne Warten liefert, kommt in den Cache (`Fill`). Ein
      Speicherzugriff, der schreibt, macht den Eintrag seiner Adresse
      ungültig (zweiter Tag-Leseport an der Busadresse, damit Stack-Zugriffe
      keine Befehle verdrängen). Ein Wort, das die Speicherstufe gerade
      schreibt (`Write` und Busadresse = PC), kommt nicht aus dem Cache, die
      Speicherung geht in Programmreihenfolge vor. Reset macht alles ungültig
- [x] `CPU_16B_SOC`: Der Fetch nimmt das Wort aus dem Cache, wenn er trifft,
      sonst vom Bus (`WordMux`); ein Treffer zählt als geholtes Wort, auch
      wenn die Speicherstufe den Bus hat. Das gilt auch für das zweite Wort
      von LI/JMP/CALL und für die Sprungvorhersage. Hält die Speicherstufe
      wegen Wartetakten die Pipeline an, wartet auch ein Treffer
- [x] Tests: `ICACHE_16.clt` (Füllen, andere Tags, Speichern an dieselbe und
      eine andere Adresse mit gleichem Index, Ersetzen, alle 16 Einträge,
      Reset); SoC-Suiten neu erzeugt (Endzustand weiter gleich
      COMPUTER_16B bzw. `isa::sim16`); Rust-Tests inkl.
      `isa_simulator_matches_circuit` und der SoC-Tests mit Wartetakten
- [x] Ergebnis SOC_16B (Takte, alle Benchmarks):

  | Benchmark | ohne Cache | mit Cache | ohne Cache, 2 Wartetakte | mit Cache, 2 Wartetakte |
  |---|---|---|---|---|
  | branch | 1.231 | 1.231 | 3.523 | 1.259 |
  | calls | 1.744 | 1.478 | 4.896 | 2.608 |
  | div | 2.396 | 2.396 | 6.478 | 2.956 |
  | div_hw | 579 | 579 | 793 | 643 |
  | memory | 980 | 886 | 3.120 | 1.334 |
  | mul | 455 | 434 | 1.323 | 828 |
  | mul_hw | 314 | 293 | 676 | 417 |
  | **gesamt** | 7.699 | 7.297 (−5 %) | 20.809 | 10.045 (−52 %) |

  Ohne Wartetakte hilft der Cache nur dort, wo Speicherzugriffe in
  Schleifen liegen (calls, memory). Mit langsamem Speicher (später QSPI)
  spart jeder Treffer die ganze Latenz: halb so viele Takte. SOC_16B:
  67.151 NANDs (+22.919)
- [x] Chip (basic, met5, 70 %, `--optimize`, `--mem-delay 3`, Ziel 7 ns):
      weiter **138 MHz** (Slack −0,23 ns statt −0,27 ns), der kritische Pfad
      liegt nicht im Cache. Die 429 × 429 µm statt 356 × 356 µm, 119.587
      Transistoren statt 80.901. `verify-verilog` 100 Suiten grün; mit
      `--x-init` scheitern wie bisher nur die Grundbaustein-Suiten, die ohne
      Reset „startet bei 0“ prüfen
- [x] Fläche: 3.351 der Bausteine im SoC sind 2:1-Multiplexer (1.094 davon
      im Cache), bisher je NOT + 2× AND + OR (≈ 22,5 µm²). Der Chip-Bau
      bildet `MUX_2_1` jetzt auf die sky130-Zelle `mux2_1` ab (≈ 11,3 µm²;
      `VerilogOptions::mux_cells`, `export-verilog`/`verify-verilog
      --mux-cells`); Simulation, Ansichten und der normale Export bleiben bei
      den Gattern. SOC_16B (gleiche Einstellungen): Logikfläche 133.105 →
      **91.674 µm² (−31 %)**, Die 429 → **359 µm**, 119.587 → 91.869
      Transistoren, **143 MHz** (Ziel 7 ns erreicht). `verify-verilog
      --mux-cells` 108 Suiten grün, `--x-init` wie bisher
- [x] Takt: Der kritische Pfad bei 7 ns lag nicht in der CPU, sondern im
      Ripple-Addierer `UART_RX.FirstWait` (Div + Div/2, 16 Volladdierer
      ≈ 5,8 ns). Die vier 16-Bit-Ripple-Addierer der Peripherie
      (`TIMER_16B.Dec`, `UART_TX/RX.CntDec`, `UART_RX.FirstWait`) sind jetzt
      `ADDER_CLA_16B` (Kogge-Stone), wie in der CPU. SOC_16B (u70, mem3,
      Ziel 5 ns): 8.820 → 9.496 exportierte Zellen, **156 MHz** (vorher 143).
      Neuer kritischer Pfad ist die Fetch-Schleife in einem Takt: PC →
      Cache-Lesemux → Tag-Vergleich → Treffer → `WordMux` → Sprungvorhersage
      → PC-Mux
- [ ] Später: SRAM-Makro statt Flipflops (ein Eintrag kostet heute ~1.400
      NANDs), größere Einträge (2–4 Wörter pro Tag) bei QSPI mit Burst;
      `MUX_4_1` auf `mux4_1` abbilden (≈ 21 µm² statt 3 × 11,3)

### Messung vor einem Pipeline-Ausbau (2026-11-10)

Zwei Werkzeuge: `bench --stalls` (`src/stalls.rs`) ordnet jeden Takt von
CPU_16B_SOC zu: Execute führt einen Befehl aus, oder der Takt geht
verloren, mit Ursache aus den internen Leitungen (`HoldX`, `IrValid`,
`Flush` und was sie auslöst). Die ausgeführten Takte sind genau die Befehle
von `isa::sim16` (Test `clocks_add_up_to_instructions_and_losses`). Der
Chip-Flow schreibt zusätzlich `endpoints.txt`: den schlechtesten Pfad zu
jedem Endpunkt (Start, Ende, Slack).

Takte SOC_16B, alle Benchmarks (5.453 Befehle):

| | ohne Wartetakte | 2 Wartetakte |
|---|---|---|
| Takte | 7.297 | 10.045 |
| Befehle ausgeführt | 5.453 (74,7 %) | 5.453 (54,3 %) |
| Sprung nicht vorhergesagt (vorwärts genommen) | 470 | 470 |
| MUL/DIV (16 Takte) | 578 | 578 |
| RET (2 Takte) | 260 | 260 |
| zweites Wort (LI/JMP/CALL) | 154 | 238 |
| JMP/CALL/JR | 130 | 130 |
| Load-Use | 109 | 105 |
| Bus belegt, Cache-Fehlschlag | 99 | 95 |
| Fehlvorhersage | 23 | 23 |
| Wartetakte Speicherstufe / Fetch | – | 1.488 / 1.198 |
| Sonstiges (Start nach Reset) | 21 | 7 |
| *vorhergesagte Sprünge (kosten 0)* | *598* | *598* |

Längste Pfade (5-ns-Lauf, Periode − Slack, je Zielregister):

| Ziel ← Start | Pfad |
|---|---|
| PC ← PC (Fetch-Schleife: Cache, Treffer, Vorhersage) | 6,42 ns |
| Ir ← CtrlM (Speicherstufe belegt Bus → kein Wort → Ir) | 6,26 ns |
| Flags Z/N ← Ir (Execute: Register, Forwarding, ALU, Z) | 6,26 ns |
| AddrM ← Ir (Execute: Adressrechnung) | 6,19 ns |
| ResultM ← Ir (Execute) | 5,96 ns |
| ICache-Valid ← CtrlM (Invalidieren beim Speichern) | 5,70 ns |
| SP ← CtrlM | 5,59 ns |
| Registerdatei ← CtrlM (Rückschreiben) | 5,50 ns |

Folgerungen:
- Die drei Stufen sind fast gleich lang (6,2–6,4 ns). Nur den Fetch zu
  teilen brächte 6,42 → 6,26 ns (+2,5 % Takt) und kostete jeden
  vorhergesagten Sprung einen Takt (+598, +8 %): lohnt sich nicht.
- Fünf Stufen (Fetch 1/2, Decode, Execute, Memory) müssten alle drei
  Pfade kürzen. Hochgerechnet aus den Zählungen kosten sie +598
  (vorhergesagte Sprünge), +470 (später entschiedene Sprünge), +130
  (JMP/CALL), +130 (RET), +23: 7.297 → ~8.650 Takte (1,59 pro Befehl).
  Laufzeit heute 7.297 × 6,42 ns = 46,8 µs; mit 5 Stufen bei 4,0 / 4,5 /
  5,0 ns 34,6 / 38,9 / 43,2 µs, also 1,35× / 1,20× / 1,08× schneller. Mit
  einem Sprungziel-Puffer im ersten Fetch-Takt entfallen die +598: 1,45×
  bei 4 ns
- Ohne neue Stufen: Return-Stack (RET im Fetch, bis −260), dynamische
  Vorhersage für Vorwärtssprünge (ein Teil der 470), JMP/CALL im Fetch
  (−130), MUL/DIV mit 2 Bit pro Takt (−289). Zusammen bis ~13 % weniger
  Takte, aber jede Vorhersage im Fetch verlängert den heute längsten Pfad
- Mit langsamem Speicher (QSPI) dominieren die Wartetakte (27 % bei 2
  Wartetakten): dort zählen Cache-Größe und Burst mehr als Stufen

### Kritische Pfade kürzen, ohne neue Stufen (2026-11-10)

Gleiches Verhalten (alle CLT-Suiten, `verify-verilog --mux-cells` und
`--x-init` für die SoC-Schaltungen, gleiche Takte in `bench`: 7.297 bzw.
10.045 mit 2 Wartetakten), nur anders angeordnete Logik:

- [x] Sprünge und Flush hängen nicht mehr an `Wait`: Der Weg CtrlM →
      Busadresse → I/O-Erkennung → `Wait` → Freeze → HoldX lief durch
      ~11 Gatter Sprunglogik bis PC und Ir. `JumpG`, `BrTaken`,
      `BrNotTaken` nehmen jetzt `NotHoldF` (Stall, NeedNext, MdWait), der
      Interrupt-Weg zum PC `IrqTakeF` (ohne Freeze). Freeze hält PC und Ir
      ohnehin an (Base = 0); nur `CtrlM` (RET in Memory, `RetMG`) und alle
      anderen Zustände behalten das volle HoldX
- [x] `PC_FETCH_16B`: Jump wählt im letzten Mux und gewinnt über Branch
      (beide waren nie gleichzeitig 1; mit `IrqTakeF` sorgt das für den
      Vektor)
- [x] Vorhersage zweimal: aus `DataIn` und aus dem Cache-Wort (Dekodierung
      und `PredAdder`/`PredAdderC`), `Hit` wählt nur noch aus; das
      Sprungziel ist so angeordnet, dass der späte `DataIn`-Addierer einen
      Mux vor dem PC durchläuft (`TargetX`, `TargetSelOr`)
- [x] `ICACHE_16`: Tag- und Adressvergleich als Baum (NOR4 + AND3) statt
      `OR_WIDE`-Kette
- [x] Execute: Der Registerwert läuft durch einen Mux vor der ALU statt
      drei (Forwarding und Quellen davor: `AOther`, `BMux`, `UseRegA/B`);
      `ALU_16B` gibt `Sum`/`Logic`/`IsSum` aus, die Summe läuft durch einen
      Mux in der ALU; das Ergebnis wird nach Ankunftszeit gewählt (Shifter
      zuletzt, dann Summe, Logik, MUL/DIV und In); `AddrMux` bekommt das
      ALU-Ergebnis zuletzt und SP − 1 aus eigenem Addierer (`SpMinus1`)
      statt über die SP-Einheit, deren `Dec` am Hold hängt
- [x] Messfehler behoben: `--mem-delay` wirkte bei SOC_16B gar nicht
      (`apply_mem_delay` kannte nur `PC`/`MemAddr`, nicht `Addr` →
      `DataIn`/`Wait`). Alle bisherigen SoC-Läufe mit „mem3“ im Namen
      rechneten also mit idealem Speicher; der Vergleich unten ist deshalb
      ohne `--mem-delay`

| SOC_16B (u70, `--optimize`, Ziel 5 ns) | vorher | nachher |
|---|---|---|
| exportierte Zellen | 9.496 | 10.194 (+7 %) |
| Logikfläche | 98.869 µm² | 102.392 µm² (+3,6 %) |
| Transistoren | 95.013 | 99.022 |
| Die | 364 µm | 373 µm |
| fmax | 156 MHz | **167 MHz (+7 %)** |
| Laufzeit Benchmarks | 46,8 µs | 43,7 µs |

Längste Pfade danach: Ir → Register → Shifter → Z → Flags 6,0 ns, Ir →
Steuerwerk (`ImmSel`, `OR_WIDE`-Kette) → Immediate → ALU → AddrM 6,0 ns,
ResultM 5,7, PC 5,6, Ir 5,3 (vorher PC 6,4, Ir 6,3, Flags 6,3). Die drei
Stufen sind jetzt ausgeglichen; weiter ginge es nur mit tieferen Umbauten
(Shifter ohne Umkehr-Muxe, Steuerbits vordekodiert in `Ir`, `OR_WIDE` als
Baum) oder mit mehr Stufen.

### Phase 4 – Speicher-Controller
- [ ] QSPI-Controller (Flash lesen, PSRAM lesen/schreiben), Modell im Runner
- [ ] später ggf. SDRAM-Controller (Init, Refresh, Bänke, CAS-Latenz)

### Phase 5 – Caches
- [x] L1-I (direkt abgebildet, 16 Wörter in Flipflops): `ICACHE_16`, siehe
      Ausbau B
- [ ] L1-D (write-through)
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
