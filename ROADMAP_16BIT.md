# Roadmap: 16-Bit-CPU (`CPU_16B` / `COMPUTER_16B`)

Ziel: eine 16-Bit-CPU mit erweitertem Befehlssatz, gebaut aus denselben
NAND-Gattern wie `COMPUTER_8B`. Voraussetzung sind Schleifen und
Breitenparameter in CLDL, damit 16-Bit-Bausteine nicht Bit für Bit
ausgeschrieben werden müssen.

## Entscheidungen (2026-11-09)

- **Befehlsformat:** Zwei-Operanden-Format, Befehle bleiben 16 Bit:
  `[op 15..12][Rd 11..8][Rs 7..4][fn 3..0]` (`Rd = Rd op Rs`), Immediate-Befehle
  `[op][Rd][imm8]`. 16-Bit-Konstanten und -Adressen als **Zwei-Wort-Befehle**.
- **Zwei-Wort-Befehle eintaktig:** Der Runner bzw. das Testharness legt neben
  `Instruction[16]` (Wort bei PC) auch `InstructionNext[16]` (Wort bei PC+1)
  an; bei Zwei-Wort-Befehlen erhöht sich der PC um 2. Kein Mehrtakt-Steuerwerk.
- **Register:** 16 × 16 Bit, R0 ist fest 0 (wie bisher).
- **Stack-Pointer:** eigenes Hardware-Register `SP` (nicht R15), damit `POP`
  (Rd und SP schreiben) und `CALL`/`RET` in einem Takt gehen. Der Stack
  wächst nach unten (`PUSH`: `SP ← SP−1; mem[SP] ← Rs`). *Präzisiert
  2026-11-09:* Reset setzt SP auf 0, der erste PUSH schreibt also nach
  0xFFFF (gleiche Wirkung wie „Start 0xFFFF“, aber Reset auf 0 ist billiger).
- **Adressraum:** Harvard, je 64K Wörter (16 Bit, wortadressiert), 16-Bit-PC.
- **Multiplizierer:** vorerst keiner. `MULL`/`MULH` sind im ALU-Funktionscode
  reserviert; optional später als Array-Multiplizierer (~4.100 NANDs).
  Multiplikation zunächst als Software-Beispiel (Schieben + Addieren).
- **Steuerwerk:** Decoder + OR-Bäume wie bei `CONTROL_UNIT_4B`, mit
  CLDL-Schleifen gebaut; keine PLA/ROM-Tabelle.
- **Speicher:** Datenspeicher (RAM) als Rust-Modell im Runner. Ports der CPU:
  `MemAddr[16]`, `MemDataOut[16]`, `MemWrite`, `MemDataIn[16]`. Lesen
  asynchron (wie der Befehls-Fetch: Adresse anlegen, auswerten, Daten anlegen,
  auswerten), Schreiben mit der Taktflanke. Befehls-ROM bleibt im Runner.
- **I/O:** Ausgaberegister `Out[16]` (4-stellige Hex-Anzeige `Seg3..Seg0`),
  Eingang `In[16]` für `IN`.
- **Reset:** synchroner `Reset`-Eingang für PC, SP, Flags und `Out`. Die
  Register im Registerfile bekommen keinen Reset (Software initialisiert).
- **CLDL-Syntax:**
  - Bereiche wie in Rust: `FOR i IN 0..W { … }` (Ende ausgeschlossen),
    `FOR i IN 0..=15 { … }` (Ende eingeschlossen); verschachtelbar.
  - Ausdrücke in Indizes, Breiten und Grenzen: Zahlen, Parameter,
    Schleifenvariablen, `+ - *` und Klammern.
  - Indizierte Bauteile: `PART FA[i]: FULL_ADDER;`, angesprochen als
    `FA[i].Cout`; interner Instanzname `FA_3`.
  - Parameter: `CIRCUIT REGISTER<W> { INPUT Data[W]; … }` (auch `<W, N>`),
    Instanz `PART R: REGISTER<16>;`, interner Name `REGISTER<16>`.
  - Kein `IF`: Randfälle über Schleifengrenzen (`FOR i IN 1..W`).

## ISA

Feld `op` = Bits 15..12.

| op | Name | Format | Befehle |
|----|------|--------|---------|
| 0 | SYS | `[0][Rd][Rs][fn]` | fn: 0 HALT, 1 NOP, 2 RET, 3 JR Rs, 4 CALLR Rs, 5 PUSH Rs, 6 POP Rd, 7 OUT Rs, 8 IN Rd |
| 1 | ALU | `[1][Rd][Rs][fn]` | fn: 0 ADD, 1 SUB, 2 ADC, 3 SBC, 4 AND, 5 OR, 6 XOR, 7 BIC, 8 MOV, 9 NOT, 10 NEG, 11 CMP, 12 TEST, 13–15 frei (MULL/MULH) |
| 2 | SHIFT | `[2][Rd][Rs/imm4][fn]` | fn: SHL, SHR, SAR, ROL, ROR – um Registerwert oder Immediate |
| 3 | LW | `[3][Rd][Rs][off4]` | `LW Rd, [Rs+off]` |
| 4 | SW | `[4][Rd][Rs][off4]` | `SW Rd, [Rs+off]` (mem[Rs+off] ← Rd) |
| 5 | Bcc | `[5][cond][off8]` | relativ (PC+1+off, off vorzeichenbehaftet); cond: EQ NE CS CC MI PL VS VC HI LS GE LT GT LE AL |
| 6 | LONG | `[6][Rd][-][fn]` + Wort | `LI Rd, imm16`, `JMP addr16`, `CALL addr16` |
| 7 | – | | frei |
| 8–15 | OPI | `[op][Rd][imm8]` | MOVI, ADDI (imm8 vorzeichenerweitert), CMPI, ANDI, ORI, XORI, LUI (Rd ← imm8≪8 \| Rd[7..0]), 1 frei |

Die genaue Bitbelegung (fn-Nummern bei SHIFT/LONG, Reihenfolge der OPI,
Flag-Semantik von ADC/SBC/CMP) wird in Phase 3 festgezurrt und steht dann in
`src/isa.rs` als einzige Quelle.

---

## Phase 0 – Entscheidungen

- [x] Befehlsformat, Register, Adressraum, Multiplizierer, Steuerwerk,
      Speicher (siehe oben, 2026-11-09)

## Phase 1 – CLDL erweitern

- [x] Grammatik (`src/cldl/cldl.pest`): `FOR`-Schleifen (verschachtelt, auch um
      `PART`), Ausdrücke (`+ - *`, Klammern), Parameterlisten `<W>`/`<W, N>`
      an `CIRCUIT` und Bauteiltypen, Indizes an Bauteilen (`FA[i]`)
- [x] Elaboration (`src/cldl/elaborate.rs`) vor dem Planungsschritt: jede
      Datei wird zum `Template`; ohne Parameter sofort zu einer normalen
      `CldlDefinition` ausgerollt, mit Parametern beim ersten Gebrauch pro Wert
      (`CircuitBuilder::instantiate`, Name `REGISTER<16>`)
- [x] Namen: alle Nachschlagestellen (`build`, `describe`, `nand_count`,
      `is_basic`, `flatten_until`, DOT-Export) gehen über `CircuitBuilder::def`;
      `circuit_names` listet auch die benutzten Instanzen (z. B. `ADDER<8>`);
      Verilog-Modulname wird bereinigt (`ADDER<16>` → `ADDER_16_`)
- [x] `INIT`: `SET Data 0` auf einen eigenen Bus ohne Index setzt alle Bits
      (für `REGISTER<W>` ohne Schleife im INIT)
- [x] `.clt`: `TESTS FOR REGISTER<16>` (Leerzeichen erlaubt, `ADDER< 8 >`)
- [x] Fehlermeldungen mit Zeile/Spalte: unbekannter Parameter/Schleifenvariable,
      rückwärts laufender Bereich, Schleifenvariable verdeckt Parameter,
      Busbreite < 1, negativer Index, Parameterwert < 1, doppelter Parameter,
      falsche Parameteranzahl; Syntaxfehler weiter von pest
- [x] Grammatik- und Builder-Tests (`elaborate::tests`, 9 Tests)
- [x] Demonstration: `ADDER<W>`, `REGISTER<W>`, `MUX_2_1_BUS<W>` mit `.clt` für
      W = 4, 8, 16 (4 und 8 mit den Testvektoren der festen Bausteine).
      NANDs gleich: `ADDER<8>` = `ADDER_8B` = 120, `ADDER<4>` = 60,
      `REGISTER<8>` = 280, `MUX_2_1_BUS<8>` = 64; `ADDER<16>` = 240,
      `REGISTER<16>` = 560. Rust-Test vergleicht `ADDER<8>` und `ADDER_8B`
      auf 3.848 Vektoren
- [x] **2026-11-09:** `cargo test` (38), `cargo run -- test` (242),
      `cargo run -- -d circuits test` (74), `verify-verilog` alle 55 Suiten in
      allen 4 Varianten grün
- [ ] Alte feste Bausteine (`ADDER_4B/_8B`, `REGISTER_4B/_8B`, `MUX_4B/_8B_2_1`)
      durch die parametrischen ersetzen – erst nach Zustimmung

## Phase 2 – 16-Bit-Bausteine

Alle mit `.clt`-Tests und `verify-verilog` (76 Suiten, 4 Varianten) grün,
Stand 2026-11-09. „Tiefe“ = längster Pfad in NAND-Stufen zwischen
Eingängen/Flipflops und Ausgängen/Flipflops (neu in `cargo run -- nands`).

| Baustein | Ort | NANDs | Tiefe |
|---|---|---:|---:|
| `AND/OR/XOR/NOT_BUS<W>` | `src/circuits/logic/` | 2/3/4/1 je Bit | |
| `ZERO_16B` | `logic/` | 46 | |
| `MUX_4_1_BUS<W>`, `MUX_16_1_BUS<W>` | `mux/` | 384 / 1.920 (W=16) | |
| `REGISTER_R<W>` (synchroner Reset) | `memory/` | 596 (W=16) | |
| `REG_FILE_16<W>` | `memory/` | **12.308** (W=16) | 17 |
| `ADDER<16>` (Ripple) | `adder/` | 240 | 67 |
| `ADDER_CLA_16B` (Kogge-Stone) | `adder/` | 410 | 14 |
| `ALU_16B` (mit CLA) | `alu/` | 1.091 | 34 |
| `SHIFTER_16B` (Barrel) | `alu/` | 890 | 29 |
| `HEX_DISPLAY_16B` | `display/` | 664 | |
| `PC_16B` | `circuits/` | 1.096 | |
| `SP_16B` | `circuits/` | 839 | |
| `FLAGS_16B` | `circuits/` | 156 | |

- [x] Register 16 Bit (`REGISTER<16>`, `REGISTER_R<16>` mit Reset),
      Registerfile `REG_FILE_16<W>` (bei W=8 exakt `REG_FILE_16x8`, 6.188)
- [x] Muxe mit gemeinsamem Eingangsbus (`In[k*W+i]`, weil CLDL keine
      Namensverkettung kennt); bei W=8 exakt `MUX_8B_4_1`/`MUX_8B_16_1`
- [x] `ALU_16B`: Operanden-Aufbereitung `A' = ZeroA ? 0 : A`,
      `B' = InvB ? ~B : B`, Auswahl ADD/AND/OR/XOR. Damit alle
      ALU-Funktionscodes nur über Steuerbits (ADD SUB ADC SBC AND OR XOR BIC
      MOV NOT NEG CMP TEST). Flags roh: Cout (bei SUB 1 = kein Borgen), Z, N, V
- [x] **Ripple vs. Carry-Lookahead (2026-11-09):** registrierter 16-Bit-Addierer
      (`chip build`, basic, met5, 5 ns Takt): Ripple 2.750 Transistoren,
      **148 MHz**; Kogge-Stone 3.362 Transistoren (+22 %), **481 MHz**.
      Entscheidung: CLA in der ALU, Ripple in PC und SP (laufen parallel zur
      ALU und sind nicht der kritische Pfad)
- [x] Barrel-Shifter `SHIFTER_16B` aus vier `SHIFT_STAGE<16, S>` (nur rechts;
      links durch Bitumkehr davor und danach). Kein Carry-Ausgang (Schieben
      setzt nur Z/N)
- [x] `FLAGS_16B`: Z/N und C/V getrennt ladbar, Reset
- [x] `PC_16B`: `Next = PC + 1 + (Branch ? Offset : Step2)`, `Jump` lädt `Target`,
      `Next` ist zugleich Rücksprungadresse für CALL; Reset
- [x] `SP_16B`: `Next = SP − Dec + Inc`, Reset auf 0
- [x] `HEX_DISPLAY_16B` (4× `HEX_7SEG`, `Seg3..Seg0`)
- [ ] Multiplizierer: zurückgestellt (Phase-0-Entscheidung)

## Phase 3 – ISA und Steuerwerk

- [ ] Bitbelegung final, Steuersignale je Opcode/Funktionscode
- [ ] `CONTROL_UNIT_16B` mit Tests pro Opcode/Funktionscode

## Phase 4 – CPU_16B und COMPUTER_16B

- [ ] `CPU_16B` mit Reset, Datenspeicher-Ports, `InstructionNext`
- [ ] `COMPUTER_16B` mit `Seg3..Seg0`
- [ ] Programmablauf-Tests (Sprünge, CALL/RET, Speicher, Flags)

## Phase 5 – Software und Viewer

- [ ] `src/isa.rs`: ISA pro CPU wählbar, neue Formate, Zwei-Wort-Befehle, 64K
- [ ] `src/machine.rs`: Breiten aus den Ports, RAM-Modell
- [ ] CLI `run` und Viewer-Programm-Panel für COMPUTER_16B
- [ ] Beispielprogramme `programs/16/` und Rust-Tests

## Phase 6 – Chip und Vergleich

- [ ] `chip build COMPUTER_16B`, Report, 3D-Ansicht
- [ ] Vergleich mit COMPUTER_8B (NANDs, Transistoren, Fläche, fmax)
