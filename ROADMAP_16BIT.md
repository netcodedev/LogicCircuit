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

Endgültig festgelegt am 2026-11-09 (Phase 3). Felder: `op` = Bits 15..12,
`Rd` = 11..8, `Rs` = 7..4, `fn` = 3..0, `imm8` = 7..0. Rechenbefehle wirken
auf `Rd` (`Rd ← Rd op Rs`). Register-Operanden von SYS-Befehlen stehen
immer im Rd-Feld. Reservierte Codes wirken wie NOP (LONG mit reserviertem
fn überspringt trotzdem zwei Wörter).

| op | Befehl | Kodierung | Wirkung | Flags |
|----|--------|-----------|---------|-------|
| 0 | HALT | `0 - - 0` | PC bleibt stehen | |
| 0 | NOP | `0 - - 1` | | |
| 0 | RET | `0 - - 2` | PC ← mem[SP]; SP ← SP+1 | |
| 0 | EI, DI | `0 1 - 1`, `0 2 - 1` | Interrupts an/aus (NOP-Varianten; nur CPU_16B_SOC) | |
| 0 | RETI | `0 1 - 2` | RET, stellt Flags wieder her, Interrupts an (RET-Variante; nur CPU_16B_SOC) | ZNCV |
| 0 | JR Rd | `0 Rd - 3` | PC ← Rd | |
| 0 | CALLR Rd | `0 Rd - 4` | SP ← SP−1; mem[SP] ← PC+1; PC ← Rd | |
| 0 | PUSH Rd | `0 Rd - 5` | SP ← SP−1; mem[SP] ← Rd | |
| 0 | POP Rd | `0 Rd - 6` | Rd ← mem[SP]; SP ← SP+1 | |
| 0 | OUT Rd | `0 Rd - 7` | Out ← Rd | |
| 0 | IN Rd | `0 Rd - 8` | Rd ← In | |
| 1 | ADD SUB ADC SBC | `1 Rd Rs 0..3` | Rd ← Rd ± Rs (+C) | ZNCV |
| 1 | AND OR XOR BIC | `1 Rd Rs 4..7` | BIC: Rd ← Rd AND NOT Rs | ZN |
| 1 | MOV NOT | `1 Rd Rs 8, 9` | Rd ← Rs bzw. NOT Rs | ZN |
| 1 | NEG | `1 Rd Rs 10` | Rd ← −Rs | ZNCV |
| 1 | CMP | `1 Rd Rs 11` | Rd − Rs, ohne Rückschreiben | ZNCV |
| 1 | TEST | `1 Rd Rs 12` | Rd AND Rs, ohne Rückschreiben | ZN |
| 2 | SHL SHR ROL ROR SAR | `2 Rd Rs k` | Rd ← Rd schieben um Rs[3..0]; k = 0, 1, 2, 3, 5 | ZN |
| 2 | … mit Immediate | `2 Rd n 8+k` | um n (0..15) aus dem Rs-Feld | ZN |
| 3 | LW Rd, [Rs+off] | `3 Rd Rs off4` | Rd ← mem[Rs + off4] | |
| 4 | SW Rd, [Rs+off] | `4 Rd Rs off4` | mem[Rs + off4] ← Rd | |
| 5 | Bcc off | `5 cond off8` | wenn cond: PC ← PC+1+off8 (vorzeichenbehaftet) | |
| 6 | LI Rd, imm16 | `6 Rd - 0` + Wort | Rd ← imm16 | |
| 6 | JMP addr16 | `6 - - 1` + Wort | PC ← addr16 | |
| 6 | CALL addr16 | `6 - - 2` + Wort | SP ← SP−1; mem[SP] ← PC+2; PC ← addr16 | |
| 7 | MULL MULH DIVU MODU | `7 Rd Rs 0..3` | Rd ← low/high Wort von Rd·Rs, Rd / Rs, Rd mod Rs (vorzeichenlos; /0: 0xFFFF bzw. Rd). Nur CPU_16B_SOC (MULDIV_16B, 16 Takte), sonst NOP | ZN |
| 8 | MOVI Rd, imm8 | `8 Rd imm8` | Rd ← imm8 (ohne Vorzeichen) | |
| 9 | ADDI Rd, imm8 | `9 Rd imm8` | Rd ← Rd + imm8 (vorzeichenbehaftet, −128..127) | ZNCV |
| 10 | CMPI Rd, imm8 | `A Rd imm8` | Rd − imm8 (vorzeichenbehaftet), ohne Rückschreiben | ZNCV |
| 11 | ANDI ORI XORI | `B/C/D Rd imm8` | imm8 ohne Vorzeichen | ZN |
| 14 | LUI Rd, imm8 | `E Rd imm8` | Rd ← imm8 ≪ 8 (mit ORI zusammen eine 16-Bit-Konstante) | |
| 15 | – | | reserviert | |

**Bedingungen** (`cond`, wie ARM): 0 EQ, 1 NE, 2 CS/HS (≥ ohne Vorzeichen),
3 CC/LO (<), 4 MI, 5 PL, 6 VS, 7 VC, 8 HI, 9 LS, 10 GE, 11 LT, 12 GT, 13 LE,
14 AL (immer), 15 NV (nie). C nach SUB/CMP: 1 = kein Borgen.

**Flags:** Z und N kommen aus dem Wert, der zurückgeschrieben würde (ALU
oder Schieber); C und V aus dem Addierer. MOVI, LUI, LI, LW, POP und IN
lassen die Flags unverändert, damit zwischen CMP und Bcc Konstanten geladen
werden können.

**Steuersignale** (`CONTROL_UNIT_16B`, 774 NANDs): RegWrite, ASrc
(0 Rd, 1 Imm), BSrc (0 Rs, 1 Imm, 2 zweites Wort), ImmSel (0 imm8, 1 imm8
vorzeichenerweitert, 2 imm8≪8, 3 off4), AluS (ADD/AND/OR/XOR), InvB, ZeroA,
CinOne, CinCarry, WbSel (0 ALU, 1 Schieber, 2 Speicher, 3 In), LoadZN, LoadCV,
MemWrite, MemAddrSel (0 ALU, 1 SP, 2 SP−1), MemDataSel (0 Rd,
1 Rücksprungadresse), TargetSel (0 zweites Wort, 1 Rd, 2 Speicher), Jump,
BranchInstr, Step2, SpInc, SpDec, OutEnable, Halt, ShLeft/ShRot/ShArith/ShImm
(direkt aus fn).

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
| `PC_16B` | `circuits/` | 1.676 | |
| `SP_16B` | `circuits/` | 1.009 | |
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
      ALU und sind nicht der kritische Pfad). *Revidiert 2026-11-10:* PC und SP
      lagen doch auf dem kritischen Pfad, beide nutzen jetzt `ADDER_CLA_16B`
      (Phase 7)
- [x] Barrel-Shifter `SHIFTER_16B` aus vier `SHIFT_STAGE<16, S>` (nur rechts;
      links durch Bitumkehr davor und danach). Kein Carry-Ausgang (Schieben
      setzt nur Z/N)
- [x] `FLAGS_16B`: Z/N und C/V getrennt ladbar, Reset
- [x] `PC_16B`: `Return = PC + 1 + Step2` (Rücksprungadresse für CALL), Sprung-
      ziel `PC + 1 + Offset` aus eigenem Addierer, `Jump` lädt `Target`, `Branch`
      wählt als letzter Mux vor dem Register; Reset (Stand Phase 7)
- [x] `SP_16B`: `Next = SP − Dec + Inc`, Reset auf 0
- [x] `HEX_DISPLAY_16B` (4× `HEX_7SEG`, `Seg3..Seg0`)
- [ ] Multiplizierer: zurückgestellt (Phase-0-Entscheidung)

## Phase 3 – ISA und Steuerwerk

- [x] Bitbelegung final, Steuersignale je Opcode/Funktionscode (Tabelle oben,
      2026-11-09). Änderungen gegenüber Phase 0: LUI setzt das untere Byte auf 0
      (LUI + ORI statt LUI mit Erhalt), SYS-Register im Rd-Feld,
      Schiebe-fn 0 SHL, 1 SHR, 2 ROL, 3 ROR, 5 SAR (Steuerbits direkt aus fn)
- [x] `CONTROL_UNIT_16B` (774 NANDs, Tiefe 33): Opcode-Decoder plus je ein
      Decoder für die fn von SYS, ALU und LONG, ODER-Bäume aus `OR_WIDE<G>`.
      Tests: 58 Opcode/fn-Kombinationen (jede Instruktion und reservierte Codes) mit allen Steuersignalen
- [x] `COND_16B` (74 NANDs): alle 16 Bedingungen × 16 Flag-Kombinationen
- [x] `.clt` (427 + 173), `cargo test`, `verify-verilog` 80 Suiten in 4 Varianten grün

## Phase 4 – CPU_16B und COMPUTER_16B

- [x] `CPU_16B` (**20.091 NANDs**, Tiefe 104 NAND-Stufen): Ports
      `Instruction[16]`, `InstructionNext[16]`, `MemDataIn[16]`, `In[16]`, `CLK`,
      `Reset` → `PC[16]`, `MemAddr[16]`, `MemDataOut[16]`, `MemWrite`, `Out[16]`,
      `Halted`, `SP[16]`, `FlagZ/N/C/V`. Synchroner Reset für PC, SP, Flags, Out;
      sperrt Register- und Speicherschreiben. Alle Bauteile mit Registern vor
      dem Steuerwerk deklariert, `HaltNot` zuletzt (INIT-Reihenfolge)
- [x] `COMPUTER_16B` (**20.755 NANDs**) mit `HEX_DISPLAY_16B` (`Seg3..Seg0`)
- [x] Programmablauf-Tests (`circuits/CPU_16B.clt`): sechs Programme,
      erzeugt mit einem unabhängigen Referenzmodell der ISA (Einmal-Skript im
      Scratchpad, nicht im Repo). Das Harness liefert die Wörter bei PC und PC+1
      und beantwortet Speicherzugriffe; geprüft werden nach jedem Takt PC und
      jede Änderung von Out, SP und Flags, dazu Adresse und Daten jedes
      Speicherzugriffs:
      Arithmetik/Logik/Flags inkl. 32-Bit-ADC/SBC (31 Schritte), Schieben und
      Rotieren (17), Schleife und alle Arten bedingter Sprünge (47), Speicher,
      Stack, verschachteltes CALL/RET, CALLR, JR (34), IN/NOP (4), Fibonacci bis
      46.368 (122). Gegenprobe: ein absichtlich falsches ASSERT schlägt fehl
- [x] Aufteilung (NANDs): Registerfile 12.308 (61 %), PC 1.096, ALU 1.091,
      Schieber 890, SP 839, Steuerwerk 774, Ausgaberegister 596, fünf 4:1-Muxe
      je 384, Anzeige 664
- [x] **2026-11-09:** `cargo test` (39), `test` (427), `-d circuits test` (181), `verify-verilog` 82 Suiten in 4 Varianten grün (CPU_16B: 6.558 Prüfungen je Variante)

## Phase 5 – Software und Viewer

- [x] `src/isa.rs`: `Isa::Cpu8`/`Isa::Cpu16`, gemeinsamer erster Durchlauf
      (Kommentare, Labels, `.word`); 16-Bit-Assembler und -Disassembler in
      `src/isa/cpu16.rs`, 64K Wörter. Syntax: `ADD R1, R2`, `SHL R1, R2` bzw.
      `SHL R1, 3`, `LW R1, [R2+3]`, Sprungziele als Adressen/Labels (der
      Assembler rechnet den Versatz aus, Fehler bei mehr als −128..127),
      negative Zahlen, Labels überall wo Zahlen stehen dürfen.
      Pseudobefehle `INC`, `DEC`, `SUBI`, `BRA` = `BAL`, `BHS`/`BLO`.
      LI/JMP/CALL sind immer zwei Wörter (Labeladressen hängen nicht vom Wert
      ab); das Listing zeigt das zweite Wort als `; 0x1234`
- [x] `src/machine.rs`: `Machine` mit ROM, RAM und Eingabeport; Breiten aus den
      Ports. `InstructionNext` vorhanden ⇒ 16-Bit-ISA; RAM (2^Breite von
      `MemAddr`) nur mit Speicherports: `MemDataIn = ram[MemAddr]` vor jedem
      Takt, Schreiben bei steigender Flanke, wenn `MemWrite`
- [x] CLI `run`: ohne `--top` läuft ein Programm aus einem Ordner `16` auf
      COMPUTER_16B, sonst auf COMPUTER_8B; `--input` setzt `In`; Trace, Out und
      Anzeige mit der Breite der Ports (vier Ziffern bei COMPUTER_16B)
- [x] Viewer: Beispiele und Assembler passend zur gebauten CPU, Adressen
      vierstellig; ein Programm der anderen ISA wird beim Wechsel ersetzt.
      Der Eingabeport bleibt in der Hand der Seitenleiste. Geprüft über die
      WASM-Schnittstelle in Node (beide CPUs, Fibonacci bis HALT); die Seite
      selbst nur lokal prüfbar (CDN im Container gesperrt)
- [x] Beispielprogramme `programs/16/`: `fibonacci` (bis 46.368, Ende über das
      Carry), `multiply` (1234 × 37 per Unterprogramm, Register auf dem Stack),
      `factorial` (`n!` von `In`, rekursiv; 8! = 40.320), `sort` (Bubblesort von
      acht Wörtern im RAM). Rust-Tests laufen alle bis HALT und prüfen Out bzw.
      den RAM, dazu ein Test für SW/LW, PUSH/POP und IN
- [x] Laufzeit (Release, `run`): COMPUTER_16B ≈ 200–400 Befehle/s,
      COMPUTER_8B ≈ 1,6 kHz – 2,5-mal so viele Gatter und zwei Auswertungen
      pro Fetch (Speicherwort hängt von `MemAddr` ab)
- [x] **2026-11-09:** `cargo test` (45), `test` (427), `-d circuits test` (181), `verify-verilog` 82 Suiten in 4 Varianten grün

## Phase 6 – Chip und Vergleich

- [x] `chip build COMPUTER_16B` (Basis-Blöcke, sky130, 10-ns-Takt, 2026-11-09),
      `chip extract`, `chip report`, isometrische Renders und Chip-3D-Ansicht
      im Viewer geprüft (Lauf `basic-met5`: 6.478 Zellen platziert, alle 6.550
      Netze verbunden)
- [x] Lagen-Experiment COMPUTER_16B:

  | Lauf | Die | Leitung | Vias | DRC | fmax | Laufzeit |
  |---|---|---|---|---|---|---|
  | met5, 40 % | 329 µm | 115,2 mm | 34.219 | 0 | 95 MHz | 102 s |
  | met3, 40 % | 329 µm | 116,4 mm | 34.106 | 0 | 96 MHz | 121 s |
  | met5, 70 % | 251 µm | 103,8 mm | 33.975 | 0 | 96 MHz | 122 s |
  | met5 + `--optimize` | 329 µm | 118,4 mm | 34.552 | 0 | 100 MHz | 104 s |
  | met2, 40 % | 329 µm | – | – | 267 | – | abgebrochen nach 110 min |

  Anders als COMPUTER_8B (DRC-frei schon mit met2) reichen für COMPUTER_16B
  **zwei Lagen nicht**: Der Detail-Router blieb nach 58 Iterationen bei 267
  Verletzungen stehen. Ab met3 ist alles DRC-frei; met4/met5 tragen nur 3 %
  der Leitung. 70 % Auslastung spart 42 % Fläche bei gleichem Takt.

- [x] Vergleich mit COMPUTER_8B (beide `basic-met5`, 40 %, gleiches Image):

  | | COMPUTER_8B | COMPUTER_16B | Faktor |
  |---|---|---|---|
  | NANDs (Simulator) | 8.064 | 20.755 | 2,6× |
  | Zellen (ohne Füller/Taps) | 2.213 | 6.539 | 3,0× |
  | Transistoren | 13.852 | 38.281 | 2,8× |
  | Flipflops | | 292 | |
  | Logikfläche | 14.704 µm² | 41.658 µm² | 2,8× |
  | Die | 199 × 199 µm | 329 × 329 µm | 2,7× |
  | Leitung | 31,7 mm | 115,2 mm | 3,6× |
  | Vias | 11.516 | 34.219 | 3,0× |
  | fmax | 98 MHz | 95 MHz | 0,97× |

  Ergebnis: Fläche und Transistoren wachsen wie die Logik (≈ 2,8×), die
  Leitung etwas stärker (3,6×, längere Busse im größeren Die). Der Takt bleibt
  fast gleich, weil die ALU den Kogge-Stone-Addierer nutzt.
  Kritischer Pfad von COMPUTER_16B: Flags → `COND_16B` → `BranchAnd` →
  Inkrement-Mux im PC → **16-Bit-Ripple-Addierer des PC** → Rücksprungadresse
  → `MemDataOut` (≈ 6,5 ns davon im Ripple-Addierer). Bei COMPUTER_8B endet
  der längste Pfad im Registerfile.
  Mit dem eigenen Router (3 Lagen, ohne PDK-Regeln) sah das anders aus
  (6,4× Fläche): Er musste für die 16-Bit-Busse die Auslastung auf 43 %
  senken; OpenROAD verteilt die Leitung dagegen problemlos auf met1/met2
  (91 %).
- [x] PC-Addierer als Kogge-Stone und Rücksprungadresse aus eigenem
      Inkrementierer – erledigt in Phase 7

## Phase 7 – Takt: kritischen Pfad entschärfen (2026-11-10)

Ohne ISA-Änderung, nur Umbau von `PC_16B` und `SP_16B`.

- [x] `PC_16B`: Inkrementierer (`PC + 1 + Step2`) und Sprung-Addierer
      (`PC + 1 + Offset`) sind getrennte `ADDER_CLA_16B` und hängen nur am
      PC-Register und am Befehl. `Jump` wählt zuerst zwischen Inkrement und
      `Target`, das späte `Branch` (Flags → `COND_16B` → `BranchAnd`) steuert
      nur noch den letzten Mux vor dem Register. Der Ausgang heißt jetzt
      `Return` (= `PC + 1 + Step2`) und hängt nicht mehr von `Branch` ab
- [x] `SP_16B`: `ADDER_CLA_16B` statt Ripple (SP − 1 → `MemAddr` bei
      PUSH/CALL war nach dem PC-Umbau der kritische Pfad)
- [x] Ergebnis (`chip build COMPUTER_16B`, basic, met5, 40 %, 10-ns-Takt):

  | Stand | NANDs | Tiefe | Transistoren | Die | fmax | kritischer Pfad |
  |---|---|---|---|---|---|---|
  | vorher | 20.755 | – | 38.281 | 329 µm | 95 MHz | Flags → Branch → PC-Ripple → `MemDataOut` |
  | PC umgebaut | 21.335 | 97 | 40.103 | 336 µm | 109 MHz | Decoder → SP-Ripple → `MemAddr` |
  | + SP mit CLA | 21.505 | 82 | 40.710 | 339 µm | **132 MHz** | Decoder → ImmMux → BMux → ALU → WbMux → `WbZero` → Flags |

  +750 NANDs (+3,6 %) und +6 % Transistoren für +39 % Takt. Der Pfad läuft
  jetzt wie erwartet durch den Datenpfad. Größte Einzelposten darauf: das
  Steuersignal `ImmSel` (schwach getriebenes `nor4` mit 16 Mux-Lasten,
  ≈ 1,1 ns) und der Kogge-Stone-Addierer der ALU (≈ 1,5 ns).
- [x] Hinweis: Befehle und Daten kommen ohne externe Verzögerung an
      (`Instruction` ist Startpunkt mit 0 ns). Mit echtem ROM/RAM läge deren
      Zugriffszeit zusätzlich im Takt – nächster Hebel ist eine Pipeline
      (Befehlsregister, synchrones RAM) – siehe Phase 8

## Phase 8 – Speichermodell und 2-stufige Pipeline (2026-11-10)

- [x] **Speichermodell im Timing** (`chip build --mem-delay NS`, Laufname
      `…-mem3`; `MEM_DELAY` in `chip/flow/pnr.tcl`): Befehls- und Datenspeicher
      liegen außerhalb des Chips und antworten NS nach ihrer Adresse.
      `apply_mem_delay` liest die Ankunftszeit von `PC` bzw. `MemAddr` an den
      Pins (Periode − Slack bei Output-Delay 0) und setzt darauf den
      Input-Delay von `Instruction`/`InstructionNext` bzw. `MemDataIn`; damit
      ist der ganze Pfad PC → ROM → Befehl → … → `MemAddr` → RAM → … im Takt.
      Die Datenports bekommen außerdem einen `buf_4` als Treiber.
- [x] **Erkenntnis:** OpenSTA rechnet Eingangsports mit idealem Treiber. Bei
      der Einzeltakt-CPU treiben `Instruction`-Pins direkt Hunderte Lasten
      (Registerfile-Adressen, Steuerwerk) – das war in den 132 MHz aus
      Phase 7 kostenlos. Ein Flipflop als Treiber (Pipeline) braucht dafür
      ohne Puffer 6,4 ns.
- [x] `CPU_16B_PIPE` / `COMPUTER_16B_PIPE`: Fetch | Execute, gleiche Ports,
      gleiche ISA, gleicher Datenpfad.
  - `IR_16B`: Befehlsregister (Wort und Folgewort). Bit 0 invertiert
    gespeichert, damit „gelöscht“ NOP (0x0001) statt HALT (0x0000) ist;
    Flush (genommener Sprung) und Reset laden NOP. Beide Hälften sind
    `REGISTER_R`: ein einfaches `REGISTER` setzt in seinem INIT den
    `Load`-Eingang, der hier mit dem PC-Enable verbunden ist – der PC zählte
    beim Bauen schon einmal hoch.
  - `PC_FETCH_16B`: Fetch-PC. Während ein Befehl ausgeführt wird, zeigt der PC
    schon dahinter, daher Sprungziel = PC + Offset und Rücksprungadresse = PC
    (kein eigenes PC-Register für die Execute-Stufe). Ein Vor-Decoder
    (Opcode 6) überspringt das zweite Wort von LI/JMP/CALL.
  - Keine Daten-Hazards (Register, Flags und Speicher nur in Execute);
    genommene Sprünge kosten einen Takt, ebenso der erste Takt nach Reset.
  - Tests: `IR_16B.clt`, `PC_FETCH_16B.clt`, `CPU_16B_PIPE.clt` (dieselben
    sechs Programme wie `CPU_16B.clt`, aus dem Simulator erzeugt; Endzustand,
    `Out`, SP, Flags und alle Speicherschreibzugriffe gleich wie bei
    `CPU_16B`), `COMPUTER_16B_PIPE.clt`; Rust-Tests laufen alle Programme auf
    beiden Rechnern. `verify-verilog`: 86 Suiten in 4 Varianten grün.
  - 22.680 NANDs (+1.175 gegenüber COMPUTER_16B: `IR_16B` 1.160).
- [x] Takte pro Programm (Einzeltakt → Pipeline): Arithmetik 31 → 32,
      Schleifen 47 → 62, Speicher/CALL 34 → 45, Fibonacci 122 → 146,
      factorial.asm 488 → 595 (≈ +20 %).
- [x] Ergebnis (basic, met5, 40 %, `--mem-delay 3`):

  | | COMPUTER_16B | COMPUTER_16B_PIPE |
  |---|---|---|
  | fmax ohne `--optimize` | 59 MHz | 54 MHz |
  | fmax mit `--optimize` | 57 MHz | **71 MHz** |
  | Takte pro Befehl (Programme oben) | 1,0 | ≈ 1,2 |
  | Befehle pro µs (bester Lauf) | ≈ 59 | ≈ 59 |
  | Transistoren | 40.710 | 42.368 |

  Ohne Puffer ist die Pipeline langsamer (Fanout des Befehlsregisters, s. o.);
  mit `--optimize` puffert OpenROAD es, und der Takt steigt um ≈ 20 % – das
  frisst der verlorene Takt pro genommenem Sprung aber wieder auf. Netto
  kein Gewinn, weil der RAM-Zugriff weiter mitten in Execute liegt: der
  kritische Pfad ist jetzt Befehlsregister → SP/ALU → `MemAddr` (9,6 ns) →
  RAM (3 ns) → `MemDataIn` → `TargetMux` → PC (RET).
- [x] Nächster Hebel: Speicherstufe (Fetch | Execute | Memory) mit
      registrierter Speicheradresse bzw. synchronem RAM. Kostet einen
      Forwarding-Pfad bzw. einen Wartetakt bei „Laden, dann sofort benutzen“
      und macht RET/POP zweitaktig. Alternativ Sprungvorhersage
      (z. B. rückwärts = genommen), um den Sprung-Takt zurückzuholen. Speicherstufe: Phase 9

## Phase 9 – Speicherstufe: 3-stufige Pipeline (2026-11-10)

- [x] `CPU_16B_PIPE3` / `COMPUTER_16B_PIPE3`: Fetch | Execute | Memory, gleiche
      Ports, gleiche ISA.
  - Execute rechnet wie bisher und legt Ergebnis, Speicheradresse,
    Speicherdaten und Steuerbits (Rd, RegWrite, MemWrite, Load, Ret) in
    Pipeline-Register (`ResultM`, `AddrM`, `DataM`, `CtrlM`).
  - Memory: `MemAddr`, `MemDataOut` und `MemWrite` kommen direkt aus diesen
    Registern (stabil ab Taktbeginn, wie bei einem synchronen SRAM); das
    Registerfile wird erst hier geschrieben (Ergebnis oder geladenes Wort).
  - `HAZARD_16B`: Forwarding von `ResultM` an Rd/Rs der Execute-Stufe; bei
    „Laden, dann sofort benutzen“ (LW/POP) ein Wartetakt (PC und Befehls-
    register halten, das Steuerwerk sieht NOP). R0 wird nie weitergereicht.
  - Flags, SP und `Out` werden in Execute geschrieben, brauchen also nichts.
  - RET: Ziel kommt aus dem Speicher – in Execute wird das geholte Wort
    verworfen, in Memory der PC aus `MemDataIn` geladen (zwei Takte).
  - `Halted` erst, wenn die Speicherstufe nichts mehr schreibt (ein SW direkt
    vor HALT findet noch statt).
  - Tests: `HAZARD_16B.clt` (15 Fälle), `CPU_16B_PIPE3.clt` (die sechs
    Programme aus `CPU_16B.clt` plus ein Hazard-Programm: Laden und sofort
    benutzen, Weiterreichen in Store-Daten und Sprungziel, RET, Schreiben
    auf R0, SW vor HALT; Endzustand gleich wie bei `CPU_16B`),
    `COMPUTER_16B_PIPE3.clt`; Rust-Test `pipeline_hazards` auf allen drei
    Rechnern. `verify-verilog`: 89 Suiten in 4 Varianten grün.
  - 25.442 NANDs (+3.937 gegenüber COMPUTER_16B: Pipeline-Register ≈ 2.100,
    Befehlsregister 1.196, Forwarding-Muxe 256, Hazard 79).
- [x] Takte (Einzeltakt → 3 Stufen): factorial 488 → 610, fibonacci 168 → 193,
      multiply 65 → 79, sort 320 → 401 (≈ 1,22 Takte pro Befehl).
- [x] Ergebnis (basic, met5, 40 %, `--mem-delay 3`):

  | | COMPUTER_16B | _PIPE (2 Stufen) | _PIPE3 (3 Stufen) |
  |---|---|---|---|
  | fmax ohne `--optimize` | 59 MHz | 54 MHz | 62 MHz |
  | fmax mit `--optimize` | 57 MHz | 71 MHz | **102 MHz** |
  | Takte pro Befehl | 1,0 | ≈ 1,2 | ≈ 1,22 |
  | Befehle pro µs (bester Lauf) | ≈ 59 | ≈ 59 | **≈ 84** (+42 %) |
  | Transistoren | 40.710 | 42.368 | 46.748 |
  | Die | 339 µm | 345 µm | 362 µm |

  Der Speicher liegt jetzt in keinem langen Pfad mehr: Ohne Speicher-
  verzögerung (`basic-met5`) ergibt sich genau derselbe Takt wie mit 3 ns.
  Kritischer Pfad (optimiert, 9,6 ns): Befehlsregister → `HAZARD_16B` →
  Stall-Gatter vor dem Steuerwerk (≈ 1,2 ns) → Steuerwerk → `ImmMux` →
  `BMux` → ALU → `WbMux` → `WbZero` → Flags.
- [ ] Mögliche nächste Schritte: Stall nicht vor dem Steuerwerk, sondern nur
      an den Schreibsignalen ausblenden (spart ≈ 1 ns); Z-Flag nicht aus dem
      Ergebnis, sondern parallel aus dem Addierer; Sprungvorhersage.
