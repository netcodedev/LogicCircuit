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
  (Rd und SP schreiben) und `CALL`/`RET` in einem Takt gehen. Start 0xFFFF,
  der Stack wächst nach unten (`PUSH`: `SP ← SP−1; mem[SP] ← Rs`).
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

- [ ] Grammatik: `FOR`-Schleifen (verschachtelt, auch um `PART`), Ausdrücke,
      Parameterlisten `<W>` an `CIRCUIT` und Bauteiltypen
- [ ] Elaboration vor dem Planungsschritt: parametrisierte Definitionen pro
      Wert instanziieren (`REGISTER<16>`), Schleifen ausrollen
- [ ] Namen in `describe`, `nand_count`, `is_basic`, `flatten_until`, Viewer-Liste,
      Verilog-Export (`<`/`>` im Modulnamen)
- [ ] `.clt`: `TESTS FOR REGISTER<16>`
- [ ] Fehlermeldungen mit Zeile/Spalte (unbekannter Parameter, leere/negative
      Bereiche, falsche Parameteranzahl)
- [ ] Grammatik- und Builder-Tests
- [ ] Demonstration: `ADDER<W>`, `REGISTER<W>`, `MUX_2_1_BUS<W>` mit Tests für
      W = 4, 8, 16; `ADDER<8>` hat exakt so viele NANDs wie `ADDER_8B` und
      besteht dieselben Testvektoren
- [ ] alle Testläufe und `verify-verilog` (4 Varianten) grün

## Phase 2 – 16-Bit-Bausteine

- [ ] Register 16 Bit, Registerfile 16×16
- [ ] Addierer/ALU 16 Bit (Ripple vs. Carry-Lookahead per `chip build` vergleichen)
- [ ] Muxe, Barrel-Shifter (SHL/SHR/SAR/ROL/ROR)
- [ ] Flag-Register Z/C/N/V
- [ ] PC 16 Bit mit Addierer für relative Sprünge und +2
- [ ] SP-Register (±1)
- [ ] 4× `HEX_7SEG`

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
