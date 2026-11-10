# Roadmap: 32-Bit-RISC-V-SoC (`SOC_RV32`)

Ziel: ein 32-Bit-SoC mit dem Befehlssatz RISC-V RV32IM, gebaut aus denselben
Grundgattern wie `SOC_16B`, getrennt vom 8/16-Bit-Ökosystem (eigene
Schaltungen in `circuits/riscv/`, eigene Werkzeuge in `src/riscv/`, eigene
Programme in `programs/riscv/`). Gemeinsam genutzt werden nur die
breitenunabhängigen Grundbausteine in `src/circuits/` (`ADDER<W>`,
`REGISTER_R<W>`, `MUX_*_BUS<W>`, `PREFIX_LEVEL<W,D>`, ...). Der spätere
Ausbau auf 64 Bit (RV64I) wird mitgedacht: Bausteine mit Breitenparameter,
wo es geht.

## Entscheidungen (Phase 0, 2026-11-10)

Mit Ben besprochen und so angenommen:

1. **Befehlssatz:** RV32IM + Zicsr, ohne C-Erweiterung (16-Bit-Befehle
   machen Fetch und Decoder deutlich komplizierter; später optional).
2. **Register:** 32 × 32 Bit (volles RV32I, kein RV32E), x0 fest 0.
3. **Pipeline:** drei Stufen wie `SOC_16B` (Fetch | Execute | Memory) mit
   Forwarding, Load-Use-Stall und später Befehls-Cache. Das
   5-Stufen-Experiment des 16-Bit-SoC hat sich nicht gelohnt.
4. **Speicher:** ein gemeinsamer Bus (von Neumann) mit Byte-Adressen und
   Byte-Enables (`ByteEn[4]`). Fehlausgerichtete Zugriffe lösen eine
   Ausnahme aus (erlaubt laut Spezifikation, spart die Zerlegung in Hardware).
5. **Privilegien:** nur Machine-Mode mit minimalen CSRs (`mstatus`, `mie`,
   `mip`, `mtvec`, `mepc`, `mcause`, `mtval`, `mscratch`, `mcycle`,
   `minstret`, `misa`, `mhartid`) und einem Timer nach CLINT-Art
   (`mtime`/`mtimecmp`).
6. **Speicherkarte:** RAM ab `0x0000_0000`, Peripherie ab `0x1000_0000`
   (GPIO, Anzeige, UART, Timer), siehe unten.
7. **Toolchain:** eigener Assembler in Rust (läuft auch im Browser) und
   LLVM/Clang + lld als externe Abhängigkeit für C/Rust-Programme
   (`--target=riscv32 -march=rv32im_zicsr`).
8. **Chip-Flow:** zuerst weiter sky130 (direkter Vergleich mit `SOC_16B`),
   GF180MCU (wafer.space) als eigene spätere Phase.
9. **Namen:** `CPU_RV32` / `SOC_RV32` in `circuits/riscv/`.

Weitere Festlegungen (2026-11-10):

- **Halt:** `ebreak` hält die Maschine an (Ausgang `Halted`), statt eine
  Breakpoint-Ausnahme auszulösen – wie `HALT` beim 16-Bit-SoC.
- `wfi` und `fence` wirken wie `nop`.
- **Reset-Vektor** 0, `mtvec` nur direkt (Modus-Bits fest 0).
- `mcause` speichert Bit 31 und die Bits 3..0 (WLRL), `mepc`/`mtvec`
  Bits 31..2.
- Der Befehlssimulator zählt `mcycle` und `mtime` pro Befehl, die
  Schaltung pro Takt; Vergleiche lassen zeitabhängige Werte aus.

## Speicherkarte

| Adresse | Register | Zugriff |
|---|---|---|
| `0x0000_0000` | RAM (1 MiB im Runner, wiederholt sich) | lesen/schreiben |
| `0x1000_0000` | GPIO_OUT | lesen/schreiben |
| `0x1000_0004` | GPIO_IN | lesen |
| `0x1000_0008` | DISPLAY (8-stellige Hex-Anzeige) | lesen/schreiben |
| `0x1000_000C` | UART_DATA: schreiben sendet ein Byte, lesen holt das empfangene | lesen/schreiben |
| `0x1000_0010` | UART_STATUS: Bit 0 sendet gerade, Bit 1 Byte empfangen | lesen |
| `0x1000_0014` | UART_CTRL: Bit 0 Interrupt (extern) bei Empfang | lesen/schreiben |
| `0x1000_0018` | UART_DIV: Takte pro Bit - 1 (16 Bit, nach Reset 0) | lesen/schreiben |
| `0x1000_0020` / `_0024` | `mtime` (64 Bit, zählt Takte) | lesen/schreiben |
| `0x1000_0028` / `_002C` | `mtimecmp`; Timer-Interrupt, solange `mtime >= mtimecmp` | lesen/schreiben |

Die I/O-Register wiederholen sich alle 64 Byte im Bereich `0x1000_0000`–`0x1FFF_FFFF`
(IO_RV32 dekodiert nur die Adressbits 31..28 und 5..2). Ein Byte- oder
Halbwort-Store schreibt seine Bytes in ihre Spuren und 0 in die übrigen.

## Phasen

### Phase 1 – Schneller Simulator (PR #6, 2026-11-10)
- [x] Flache Draht-Arrays, Flipflops als eingebaute Bausteine, topologische
      Sortierung: `SOC_16B` ~100 Hz → ~10 kHz

### Phase 2 – Werkzeuge in Rust (`src/riscv/`)
- [x] Befehlstabelle für Decoder, Encoder und Disassembler (`isa.rs`)
- [x] Assembler mit Pseudo-Befehlen und Direktiven (`asm.rs`), geprüft
      gegen LLVM (`clang --target=riscv32`), falls installiert
- [x] Befehlssimulator RV32IM + Zicsr mit Traps, Interrupts und der
      Peripherie der Speicherkarte (`sim.rs`)

### Phase 3 – Grundbausteine 32 Bit
- [x] Gemeinsam (mit Breitenparameter, in `src/circuits/`): `ADDER_KS5<W>`
      (Kogge-Stone, 5 Präfixstufen), `REG_FILE_32<W>` (x0 fest 0),
      `MUX_32_1_BUS<W>`, `DECODER_3_8`, `DECODER_5_32`
- [x] RISC-V-spezifisch (`circuits/riscv/`): `ALU_RV32` mit
      `SHIFTER_RV32`, `IMM_RV32`, `CONTROL_RV32` (Decoder, erkennt alle
      illegalen Befehle), `CSR_RV32`, `LSU_RV32` (Byte-Spuren),
      `MULDIV_RV32` (1 Bit pro Takt, 32 Schritte, Vorzeichen über Beträge)
- [x] Rust-Tests mit Zufallsvektoren gegen die Referenz (`src/riscv/tests.rs`)

### Phase 4 – CPU_RV32
- [x] Drei Stufen (Fetch \| Execute \| Memory), Forwarding aus der
      Memory-Stufe, Load-Use-Stall, ein Bus mit `Wait`
- [x] Traps (illegal, ecall, fehlausgerichtete Sprünge/Loads/Stores),
      Timer- und externer Interrupt, `mret`, `ebreak` hält an
- [x] M-Erweiterung über `MULDIV_RV32` (Execute wartet 32 Takte)
- [x] 101.020 NANDs (Registerdatei 50 %, CSRs 18 %, MulDiv 12 %),
      Logiktiefe 131 NAND-Stufen

### Phase 5 – SOC_RV32, Runner, Tests
- [x] `IO_RV32` (GPIO, Anzeige, UART mit `UART_TX`/`UART_RX`, `mtime`/
      `mtimecmp`), `SOC_RV32` mit Synchronizern und 8-stelliger Anzeige:
      123.293 NANDs
- [x] Runner `src/riscv/machine.rs` (RAM mit Byte-Enables, Wartezyklen,
      UART-Leitungen), `riscv asm/run/bench` in der CLI
- [x] Vergleich mit dem Befehlssimulator (`src/riscv/cpu_tests.rs`):
      Register, RAM, UART, Anzeige und GPIO gleich – gezielte Programme,
      Zufallsprogramme, langsamer Speicher, Timer- und UART-Interrupts
- [x] Programme in `programs/riscv/` (Assembler) und C über clang + lld
      (`programs/riscv/c/`: `crt0.s`, `link.ld`, `soc.h`)

### Phase 6 – Benchmarks, Viewer, Chip-Flow
- [x] `riscv bench` (`programs/riscv/bench/`), ohne Wartezyklen; das
      Ergebnis jedes Programms stimmt mit dem Befehlssimulator überein.
      Teuer sind der gemeinsame Bus (Loads/Stores halten den Fetch an) und
      die 33 Takte pro Multiplikation/Division.

| Programm | Befehle | Takte | CPI |
|---|---:|---:|---:|
| branch | 21.949 | 26.938 | 1,23 |
| calls | 20.713 | 31.563 | 1,52 |
| memory | 6.666 | 19.208 | 2,88 |
| muldiv | 6.904 | 112.262 | 16,26 |
| sieve (C) | 52.382 | 77.261 | 1,47 |
| sort | 7.897 | 13.730 | 1,74 |

- [x] Viewer: SOC_RV32 mit Beispielprogrammen, Listing mit
      Byte-Adressen, UART, Anzeige, GPIO
- [x] Simulation: SOC_RV32 läuft nativ mit etwa 6–18 kHz, im Browser
      mit etwa 1 kHz
- [ ] Chip-Flow (sky130) und Vergleich mit `SOC_16B`
- [x] `.clt`-Tests für alle RISC-V-Bausteine, `CPU_RV32` und `SOC_RV32`
      (für `verify-verilog`), aus dem Simulator erzeugt mit
      `cargo test --release --lib riscv::clt_gen -- --ignored`; CLT kann
      dafür ganze Busse setzen und prüfen (`SET A[31:0] 0x1234_5678`,
      `ASSERT S[31:0] -5`, `F[2:0] 0b101`)

### Phase 7 – Kürzerer kritischer Pfad
Gemessen als Logiktiefe in NAND-Stufen (wie `nands`), also ohne echte
Gatterlaufzeiten; ein sky130-Build steht noch aus.

| Schritt | Längster Pfad | `SOC_RV32` |
|---|---:|---:|
| Ausgangslage | 131 | 123.293 NANDs |
| Fehlausrichtung aus (rs1 + imm)[1:0], Branch über imm[1], `minstret` lädt bei Retire, CSR-Writes ohne ALU-Wartezeit | 98 | 123.329 |
| `MUX_2_1` als NAND-NAND (2 statt 4 Stufen, betrifft alle Schaltungen) | 84 | 100.945 |
| Eigener Vergleicher `BRANCH_RV32` aus rs1/rs2, Immediate-Formate direkt aus den Opcode-Bits, späte Branch-Exception am Ende der Exception-Kette | 69 | 102.206 |

- [x] Ergebnis: 131 → 69 Stufen (−47 %), 17 % weniger NANDs; `SOC_16B`
      sinkt durch den neuen Mux von 85 auf 64 Stufen
- [x] Die längsten Pfade sind jetzt ausgeglichen (65–69 Stufen): Decode →
      Load-Use-Stall → `XGo` → CSR-Schreiben, Branch-Vergleich → Redirect →
      `PC`, Trap → `mstatus`
- Shifter (17 Stufen) und Registerdatei-Lesen (14) liegen nicht mehr auf
  den längsten Pfaden; ein Umbau dort würde NANDs kosten, ohne den Takt zu
  heben

### Später
- [ ] Befehls-Cache, Sprungvorhersage
- [ ] Schnellerer Multiplizierer (z. B. 4 Bit pro Takt oder Baum)
- [ ] GF180MCU-Flow (wafer.space)
- [ ] RV64I
