//! Instruction set of CPU_16B (see ROADMAP_16BIT.md for the full table).
//!
//! Word layout: `[15..12] op | [11..8] Rd | [7..4] Rs | [3..0] fn`, or
//! `[7..0] imm8` for the immediate forms and branches. LI, JMP and CALL take
//! their 16-bit operand from a second word.
//!
//! Assembly syntax (on top of labels, comments and `.word`, see
//! [`super::assemble`]):
//! - `ADD R1, R2` computes `R1 ← R1 + R2`; `SHL R1, R2` shifts by R2,
//!   `SHL R1, 3` by a constant
//! - `LW R1, [R2+3]`, `SW R1, [R2]`
//! - `BEQ label`: branch targets are addresses (labels or numbers), the
//!   assembler computes the offset
//! - `LI R1, 1000`, `JMP label`, `CALL label` (two words)
//! - `EI`, `DI`, `RETI`: interrupts (CPU_16B_SOC; NOP/RET elsewhere)
//! - `LB R1, [R2+3]`, `SB R1, [R2]` (op 15, off 0..7): byte at the byte
//!   address R2 + off, i.e. word (R2 + off) / 2, low byte if even; LB zero-
//!   extends (CPU_16B_SOC; NOP elsewhere)
//! - `MULL`/`MULH`/`DIVU`/`MODU R1, R2` (op 7): low/high word of the
//!   unsigned product, unsigned quotient/remainder. Only CPU_16B_SOC has the
//!   unit; the other CPUs treat op 7 as NOP
//! - pseudo instructions: `INC Rd` / `DEC Rd` (ADDI ±1), `SUBI Rd, n`
//!   (ADDI −n), `BRA` = `BAL`, `BHS` = `BCS`, `BLO` = `BCC`
//! - numbers may be negative (`-5`); a label may stand wherever a number can

use std::collections::HashMap;

use super::{err, expect_operands, is_identifier, parse_int, parse_register, parse_source, AsmError, Pending, Program};

/// Number of addressable words (16-bit PC and data address).
pub const MEMORY_WORDS: usize = 1 << 16;

/// Operand layout and encoding of one mnemonic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// op 0, `fn` in bits 3..0; `reg`: takes `Rd`.
    Sys { func: u16, reg: bool },
    /// op 1: `Rd, Rs`.
    Alu { func: u16 },
    /// op 2: `Rd, Rs` or `Rd, n`; `k` = fn[2..0].
    Shift { k: u16 },
    /// op 3/4: `Rd, [Rs+off4]`.
    Mem { op: u16 },
    /// op 7: `Rd, Rs`, multiply/divide unit (CPU_16B_SOC).
    MulDiv { func: u16 },
    /// A fixed word without operands (EI, DI, RETI).
    Fixed { word: u16 },
    /// op 15: `Rd, [Rs+off3]`, byte access (CPU_16B_SOC).
    Byte { store: bool },
    /// op 5: `target`.
    Branch { cond: u16 },
    /// op 6 plus a second word: `Rd, value` (LI) or `target`.
    Long { func: u16, reg: bool },
    /// op 8..14: `Rd, imm8` with the given value range.
    Imm { op: u16, min: i64, max: i64 },
    /// `INC`/`DEC Rd`: ADDI with a fixed value.
    Step { by: i64 },
    /// `SUBI Rd, n`: ADDI with −n.
    SubI,
}

const SYS: &[(&str, u16, bool)] = &[
    ("HALT", 0, false),
    ("NOP", 1, false),
    ("RET", 2, false),
    ("JR", 3, true),
    ("CALLR", 4, true),
    ("PUSH", 5, true),
    ("POP", 6, true),
    ("OUT", 7, true),
    ("IN", 8, true),
];

const ALU: &[&str] = &[
    "ADD", "SUB", "ADC", "SBC", "AND", "OR", "XOR", "BIC", "MOV", "NOT", "NEG", "CMP", "TEST",
];

/// Shifts by `k` (fn[2..0]); 4, 6 and 7 are reserved.
const SHIFT: &[(&str, u16)] = &[("SHL", 0), ("SHR", 1), ("ROL", 2), ("ROR", 3), ("SAR", 5)];

/// Multiply/divide (op 7, CPU_16B_SOC only; a NOP elsewhere).
const MULDIV: &[&str] = &["MULL", "MULH", "DIVU", "MODU"];

/// Interrupt control (CPU_16B_SOC): variants of NOP (EI, DI) and RET (RETI)
/// with Rd != 0, so the other CPUs run them as NOP and RET.
const FIXED: &[(&str, u16)] = &[("EI", 0x0101), ("DI", 0x0201), ("RETI", 0x0102)];

/// Branch conditions by code (first name is the one the disassembler uses).
const CONDS: &[&[&str]] = &[
    &["EQ"],
    &["NE"],
    &["CS", "HS"],
    &["CC", "LO"],
    &["MI"],
    &["PL"],
    &["VS"],
    &["VC"],
    &["HI"],
    &["LS"],
    &["GE"],
    &["LT"],
    &["GT"],
    &["LE"],
    &["AL", "RA"],
    &["NV"],
];

const LONG: &[(&str, u16, bool)] = &[("LI", 0, true), ("JMP", 1, false), ("CALL", 2, false)];

const IMM: &[(&str, u16, i64, i64)] = &[
    ("MOVI", 0x8, 0, 255),
    ("ADDI", 0x9, -128, 127),
    ("CMPI", 0xA, -128, 127),
    ("ANDI", 0xB, 0, 255),
    ("ORI", 0xC, 0, 255),
    ("XORI", 0xD, 0, 255),
    ("LUI", 0xE, 0, 255),
];

fn kind_of(mnemonic: &str) -> Option<Kind> {
    let m = mnemonic.to_ascii_uppercase();
    let m = m.as_str();
    if let Some(&(_, func, reg)) = SYS.iter().find(|s| s.0 == m) {
        return Some(Kind::Sys { func, reg });
    }
    if let Some(func) = ALU.iter().position(|&a| a == m) {
        return Some(Kind::Alu { func: func as u16 });
    }
    if let Some(&(_, k)) = SHIFT.iter().find(|s| s.0 == m) {
        return Some(Kind::Shift { k });
    }
    if let Some(func) = MULDIV.iter().position(|&a| a == m) {
        return Some(Kind::MulDiv { func: func as u16 });
    }
    if let Some(&(_, word)) = FIXED.iter().find(|s| s.0 == m) {
        return Some(Kind::Fixed { word });
    }
    if let Some(&(_, func, reg)) = LONG.iter().find(|s| s.0 == m) {
        return Some(Kind::Long { func, reg });
    }
    if let Some(&(_, op, min, max)) = IMM.iter().find(|s| s.0 == m) {
        return Some(Kind::Imm { op, min, max });
    }
    match m {
        "LW" => return Some(Kind::Mem { op: 3 }),
        "SW" => return Some(Kind::Mem { op: 4 }),
        "LB" => return Some(Kind::Byte { store: false }),
        "SB" => return Some(Kind::Byte { store: true }),
        "INC" => return Some(Kind::Step { by: 1 }),
        "DEC" => return Some(Kind::Step { by: -1 }),
        "SUBI" => return Some(Kind::SubI),
        _ => {}
    }
    let cond = m.strip_prefix('B')?;
    CONDS
        .iter()
        .position(|names| names.contains(&cond))
        .map(|c| Kind::Branch { cond: c as u16 })
}

/// Words a statement occupies.
fn size_of(p: &Pending) -> usize {
    match kind_of(p.mnemonic) {
        Some(Kind::Long { .. }) => 2,
        _ => 1,
    }
}

/// Assembles CPU_16B source; see the module docs for the syntax.
pub fn assemble(source: &str) -> Result<Program, AsmError> {
    let parsed = parse_source(source)?;

    // Statement index -> address (one past the end for trailing labels).
    let mut addrs = Vec::with_capacity(parsed.statements.len() + 1);
    let mut addr = 0;
    for p in &parsed.statements {
        addrs.push(addr);
        addr += size_of(p);
    }
    addrs.push(addr);
    if let Some(p) = parsed.statements.iter().zip(&addrs[1..]).find(|&(_, &end)| end > MEMORY_WORDS) {
        return Err(err(p.0.line, format!("program exceeds {MEMORY_WORDS} words")));
    }
    let labels: HashMap<String, usize> = parsed.labels.iter().map(|(name, &idx)| (name.clone(), addrs[idx])).collect();

    let mut program = Program::default();
    for (p, &addr) in parsed.statements.iter().zip(&addrs) {
        for word in encode(p, addr, &labels)? {
            program.words.push(word);
            program.lines.push(p.line);
        }
    }
    Ok(program)
}

fn encode(p: &Pending, addr: usize, labels: &HashMap<String, usize>) -> Result<Vec<u16>, AsmError> {
    let line = p.line;
    // A label or a number in `min..=max`.
    let value = |text: &str, min: i64, max: i64| -> Result<i64, AsmError> {
        match labels.get(&text.to_ascii_lowercase()) {
            Some(&a) if (min..=max).contains(&(a as i64)) => Ok(a as i64),
            Some(&a) => Err(err(line, format!("label '{text}' = {a} out of range ({min}..{max})"))),
            None if is_identifier(text) => Err(err(line, format!("unknown label '{text}'"))),
            None => parse_int(text, line, min, max),
        }
    };
    let reg = |i: usize| parse_register(p.operands[i], line);

    if p.mnemonic.eq_ignore_ascii_case(".word") {
        expect_operands(p, 1)?;
        return Ok(vec![value(p.operands[0], -0x8000, 0xFFFF)? as u16]);
    }
    let kind = kind_of(p.mnemonic).ok_or_else(|| err(line, format!("unknown instruction '{}'", p.mnemonic)))?;
    let imm8 = |op: u16, rd: u16, v: i64| vec![op << 12 | rd << 8 | (v as u16 & 0xFF)];
    Ok(match kind {
        Kind::Sys { func, reg: false } => {
            expect_operands(p, 0)?;
            vec![func]
        }
        Kind::Sys { func, reg: true } => {
            expect_operands(p, 1)?;
            vec![reg(0)? << 8 | func]
        }
        Kind::Alu { func } => {
            expect_operands(p, 2)?;
            vec![0x1000 | reg(0)? << 8 | reg(1)? << 4 | func]
        }
        Kind::Shift { k } => {
            expect_operands(p, 2)?;
            let rd = reg(0)?;
            match parse_register(p.operands[1], line) {
                Ok(rs) => vec![0x2000 | rd << 8 | rs << 4 | k],
                Err(_) => vec![0x2000 | rd << 8 | (value(p.operands[1], 0, 15)? as u16) << 4 | 8 | k],
            }
        }
        Kind::Mem { op } => {
            expect_operands(p, 2)?;
            let (rs, off) = parse_address(p.operands[1], line, &value)?;
            vec![op << 12 | reg(0)? << 8 | rs << 4 | off]
        }
        Kind::MulDiv { func } => {
            expect_operands(p, 2)?;
            vec![0x7000 | reg(0)? << 8 | reg(1)? << 4 | func]
        }
        Kind::Fixed { word } => {
            expect_operands(p, 0)?;
            vec![word]
        }
        Kind::Byte { store } => {
            expect_operands(p, 2)?;
            let (rs, off) = parse_address(p.operands[1], line, &value)?;
            if off > 7 {
                return Err(err(line, format!("byte offset {off} out of range (0..7)")));
            }
            vec![0xF000 | reg(0)? << 8 | rs << 4 | (store as u16) << 3 | off]
        }
        Kind::Branch { cond } => {
            expect_operands(p, 1)?;
            let target = value(p.operands[0], 0, MEMORY_WORDS as i64 - 1)?;
            let offset = target - (addr as i64 + 1);
            if !(-128..=127).contains(&offset) {
                return Err(err(line, format!("branch target {target} too far ({offset} words, max -128..127)")));
            }
            vec![0x5000 | cond << 8 | (offset as u16 & 0xFF)]
        }
        Kind::Long { func, reg: true } => {
            expect_operands(p, 2)?;
            vec![0x6000 | reg(0)? << 8 | func, value(p.operands[1], -0x8000, 0xFFFF)? as u16]
        }
        Kind::Long { func, reg: false } => {
            expect_operands(p, 1)?;
            vec![0x6000 | func, value(p.operands[0], 0, 0xFFFF)? as u16]
        }
        Kind::Imm { op, min, max } => {
            expect_operands(p, 2)?;
            imm8(op, reg(0)?, value(p.operands[1], min, max)?)
        }
        Kind::Step { by } => {
            expect_operands(p, 1)?;
            imm8(0x9, reg(0)?, by)
        }
        Kind::SubI => {
            expect_operands(p, 2)?;
            imm8(0x9, reg(0)?, -value(p.operands[1], -127, 128)?)
        }
    })
}

/// Parses `[Rs]`, `[Rs+off]` or `[Rs + off]` with `off` in 0..15.
fn parse_address(
    text: &str,
    line: usize,
    value: &dyn Fn(&str, i64, i64) -> Result<i64, AsmError>,
) -> Result<(u16, u16), AsmError> {
    let inner = text
        .strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .ok_or_else(|| err(line, format!("expected [Rs] or [Rs+off], got '{text}'")))?;
    match inner.split_once('+') {
        Some((rs, off)) => Ok((parse_register(rs.trim(), line)?, value(off.trim(), 0, 15)? as u16)),
        None => Ok((parse_register(inner.trim(), line)?, 0)),
    }
}

/// Renders the instruction `word` at address `addr` (`next` is the word
/// after it). Returns the text and the number of words the instruction
/// occupies. Reserved codes are shown as `.word`.
pub fn disassemble(addr: u16, word: u16, next: u16) -> (String, usize) {
    let op = word >> 12;
    let rd = (word >> 8) & 0xF;
    let rs = (word >> 4) & 0xF;
    let func = word & 0xF;
    let imm = word & 0xFF;
    let raw = || (format!(".word 0x{word:04X}"), 1);
    if let Some(&(name, _)) = FIXED.iter().find(|f| f.1 == word) {
        return (name.to_string(), 1);
    }
    match op {
        0 => match SYS.iter().find(|s| s.1 == func) {
            Some(&(name, _, true)) => (format!("{name} R{rd}"), 1),
            Some(&(name, _, false)) => (name.to_string(), 1),
            None => raw(),
        },
        1 => match ALU.get(func as usize) {
            Some(name) => (format!("{name} R{rd}, R{rs}"), 1),
            None => raw(),
        },
        2 => match SHIFT.iter().find(|s| s.1 == func & 7) {
            Some(&(name, _)) if func & 8 != 0 => (format!("{name} R{rd}, {rs}"), 1),
            Some(&(name, _)) => (format!("{name} R{rd}, R{rs}"), 1),
            None => raw(),
        },
        3 | 4 => {
            let name = if op == 3 { "LW" } else { "SW" };
            match func {
                0 => (format!("{name} R{rd}, [R{rs}]"), 1),
                off => (format!("{name} R{rd}, [R{rs}+{off}]"), 1),
            }
        }
        15 => {
            let name = if func & 8 != 0 { "SB" } else { "LB" };
            match func & 7 {
                0 => (format!("{name} R{rd}, [R{rs}]"), 1),
                off => (format!("{name} R{rd}, [R{rs}+{off}]"), 1),
            }
        }
        7 => match MULDIV.get(func as usize) {
            Some(name) => (format!("{name} R{rd}, R{rs}"), 1),
            None => raw(),
        },
        5 => {
            let target = addr.wrapping_add(1).wrapping_add(imm as i8 as u16);
            (format!("B{} 0x{target:04X}", CONDS[rd as usize][0]), 1)
        }
        6 => match LONG.iter().find(|s| s.1 == func) {
            Some(&(name, _, true)) => (format!("{name} R{rd}, 0x{next:04X}"), 2),
            Some(&(name, _, false)) => (format!("{name} 0x{next:04X}"), 2),
            None => (format!(".word 0x{word:04X}"), 2),
        },
        _ => match IMM.iter().find(|s| s.1 == op) {
            Some(&(name, _, min, _)) if min < 0 => (format!("{name} R{rd}, {}", imm as i8), 1),
            Some(&(name, ..)) => (format!("{name} R{rd}, 0x{imm:02X}"), 1),
            None => raw(),
        },
    }
}

/// One line per word, as the instructions run from address 0: the second
/// word of LI/JMP/CALL is shown as `; 0x1234`.
pub fn listing(words: &[u16]) -> Vec<String> {
    let mut lines = Vec::with_capacity(words.len());
    while lines.len() < words.len() {
        let addr = lines.len();
        let next = words.get(addr + 1).copied().unwrap_or(0);
        let (text, size) = disassemble(addr as u16, words[addr], next);
        lines.push(text);
        if size == 2 && addr + 1 < words.len() {
            lines.push(format!("; 0x{next:04X}"));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_every_kind() {
        let p = assemble(
            "start: HALT\n NOP\n RET\n JR R3\n CALLR R4\n PUSH R5\n POP R6\n OUT R7\n IN R8\n\
             ADD R1, R2\n TEST R15, R14\n SHL R1, R2\n SAR R3, 15\n LW R1, [R2+3]\n SW R4, [R5]\n\
             BEQ start\n BRA end\n LI R9, -1\n JMP start\n CALL end\n\
             MOVI R1, 255\n ADDI R2, -128\n CMPI R3, 5\n ANDI R4, 0x0F\n ORI R5, 1\n XORI R6, 2\n LUI R7, 0x12\n\
             INC R1\n DEC R2\n SUBI R3, 3\n BLO start\n end: .word 0xBEEF",
        )
        .unwrap();
        assert_eq!(
            p.words,
            vec![
                0x0000, 0x0001, 0x0002, 0x0303, 0x0404, 0x0505, 0x0606, 0x0707, 0x0808, // SYS
                0x1120, 0x1FEC, 0x2120, 0x23FD, 0x3123, 0x4450, // ALU, shift, memory
                0x50F0, 0x5E11, 0x6900, 0xFFFF, 0x6001, 0x0000, 0x6002, 0x0022, // branches, long
                0x81FF, 0x9280, 0xA305, 0xB40F, 0xC501, 0xD602, 0xE712, // immediates
                0x9101, 0x92FF, 0x93FD, 0x53DE, 0xBEEF, // pseudo, .word
            ]
        );
        assert_eq!(p.lines[17..19], [18, 18]);
    }

    #[test]
    fn disassemble_roundtrips() {
        let src = "HALT\nJR R3\nADD R1, R2\nNEG R4, R5\nROR R1, R2\nSHL R6, 7\nLW R1, [R2+3]\nSW R4, [R5]\n\
                   BNE 0x0000\nLI R9, 0x1234\nCALL 0x0000\nMOVI R1, 0xFF\nADDI R2, -5\nCMPI R3, 100\nLUI R7, 0x12\nRET";
        let p = assemble(src).unwrap();
        let mut lines = listing(&p.words);
        lines.retain(|l| !l.starts_with(';'));
        assert_eq!(lines.join("\n"), src);
    }

    #[test]
    fn reserved_codes_show_as_words() {
        assert_eq!(disassemble(0, 0x0009, 0).0, ".word 0x0009");
        assert_eq!(disassemble(0, 0x700F, 0).0, ".word 0x700F");
        assert_eq!(disassemble(0, 0x6003, 0), (".word 0x6003".to_string(), 2));
        // Backward branch from 0x0010 to 0x0000.
        assert_eq!(disassemble(0x10, 0x5EEF, 0).0, "BAL 0x0000");
    }

    #[test]
    fn reports_errors_with_line() {
        assert_eq!(assemble("ADD R1").unwrap_err().line, 1);
        assert_eq!(assemble("NOP\nADDI R1, 128").unwrap_err().line, 2);
        assert_eq!(assemble("LW R1, R2").unwrap_err().line, 1);
        assert_eq!(assemble("LW R1, [R2+16]").unwrap_err().line, 1);
        assert_eq!(assemble("SHL R1, 16").unwrap_err().line, 1);
        assert_eq!(assemble("JMP nowhere").unwrap_err().message, "unknown label 'nowhere'");
        let far = format!("BEQ far\n{}far: HALT", ".word 0\n".repeat(200));
        assert!(assemble(&far).unwrap_err().message.contains("too far"));
    }
}
