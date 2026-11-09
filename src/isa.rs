//! Instruction set of CPU_8B: one table drives the assembler and the
//! disassembler, so a new instruction only needs a new row here (plus the
//! hardware).
//!
//! Word layout: `[15..12] opcode | [11..8] Rd | [7..4] Ra | [3..0] Rb/Imm`,
//! except JMP/BRZ, whose low byte is the jump target.

use std::collections::HashMap;
use std::fmt;

/// Operand layout of an instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// No operands: `HALT`
    None,
    /// `ADD Rd, Ra, Rb`
    RegRegReg,
    /// `ADDI Rd, Ra, imm4`
    RegRegImm,
    /// `LW Rd`
    Rd,
    /// `OUT Ra`
    Ra,
    /// `JMP addr`
    Addr,
    /// `BRZ Ra, addr` (Ra in bits 11..8)
    RaAddr,
}

#[derive(Debug, Clone, Copy)]
pub struct OpInfo {
    pub mnemonic: &'static str,
    pub opcode: u16,
    pub format: Format,
}

pub const OPS: &[OpInfo] = &[
    OpInfo { mnemonic: "HALT", opcode: 0x0, format: Format::None },
    OpInfo { mnemonic: "ADD", opcode: 0x1, format: Format::RegRegReg },
    OpInfo { mnemonic: "SUB", opcode: 0x2, format: Format::RegRegReg },
    OpInfo { mnemonic: "AND", opcode: 0x3, format: Format::RegRegReg },
    OpInfo { mnemonic: "OR", opcode: 0x4, format: Format::RegRegReg },
    OpInfo { mnemonic: "XOR", opcode: 0x5, format: Format::RegRegReg },
    OpInfo { mnemonic: "OUT", opcode: 0x6, format: Format::Ra },
    OpInfo { mnemonic: "JMP", opcode: 0x7, format: Format::Addr },
    OpInfo { mnemonic: "BRZ", opcode: 0x8, format: Format::RaAddr },
    OpInfo { mnemonic: "ADDI", opcode: 0x9, format: Format::RegRegImm },
    OpInfo { mnemonic: "SUBI", opcode: 0xA, format: Format::RegRegImm },
    OpInfo { mnemonic: "ANDI", opcode: 0xB, format: Format::RegRegImm },
    OpInfo { mnemonic: "ORI", opcode: 0xC, format: Format::RegRegImm },
    OpInfo { mnemonic: "XORI", opcode: 0xD, format: Format::RegRegImm },
    OpInfo { mnemonic: "LW", opcode: 0xF, format: Format::Rd },
];

/// Number of addressable instruction words (8-bit PC).
pub const MEMORY_WORDS: usize = 256;

fn op_by_opcode(opcode: u16) -> Option<&'static OpInfo> {
    OPS.iter().find(|op| op.opcode == opcode)
}

fn op_by_mnemonic(name: &str) -> Option<&'static OpInfo> {
    OPS.iter().find(|op| op.mnemonic.eq_ignore_ascii_case(name))
}

/// Renders one instruction word as assembly, e.g. `ADD R3, R1, R2`.
pub fn disassemble(word: u16) -> String {
    let rd = (word >> 8) & 0xF;
    let ra = (word >> 4) & 0xF;
    let rb = word & 0xF;
    let addr = word & 0xFF;
    match op_by_opcode(word >> 12) {
        None => format!(".word 0x{word:04X}"),
        Some(op) => match op.format {
            Format::None => op.mnemonic.to_string(),
            Format::RegRegReg => format!("{} R{rd}, R{ra}, R{rb}", op.mnemonic),
            Format::RegRegImm => format!("{} R{rd}, R{ra}, {rb}", op.mnemonic),
            Format::Rd => format!("{} R{rd}", op.mnemonic),
            Format::Ra => format!("{} R{ra}", op.mnemonic),
            Format::Addr => format!("{} 0x{addr:02X}", op.mnemonic),
            Format::RaAddr => format!("{} R{rd}, 0x{addr:02X}", op.mnemonic),
        },
    }
}

/// An assembled program: one word per address, plus the 1-based source line
/// each word came from (for highlighting the current line).
#[derive(Debug, Clone, Default)]
pub struct Program {
    pub words: Vec<u16>,
    pub lines: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsmError {
    /// 1-based source line.
    pub line: usize,
    pub message: String,
}

impl fmt::Display for AsmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for AsmError {}

/// One instruction (or `.word`) waiting for label resolution.
struct Pending<'a> {
    line: usize,
    mnemonic: &'a str,
    operands: Vec<&'a str>,
}

/// Assembles source text.
///
/// Syntax, one statement per line:
/// - `label:` defines a label (may share a line with an instruction)
/// - `ADD R3, R1, R2` etc., mnemonics and registers case-insensitive
/// - `.word 0x9105` emits a raw 16-bit word, so a pure hex listing is valid too
/// - numbers: decimal, `0x..` hex or `0b..` binary
/// - comments start with `;`, `#` or `//`
pub fn assemble(source: &str) -> Result<Program, AsmError> {
    let mut labels: HashMap<String, usize> = HashMap::new();
    let mut pending: Vec<Pending> = Vec::new();

    // Pass 1: strip comments, collect labels and statements.
    for (idx, raw) in source.lines().enumerate() {
        let line = idx + 1;
        let mut text = raw;
        for marker in [";", "#", "//"] {
            if let Some(pos) = text.find(marker) {
                text = &text[..pos];
            }
        }
        let mut text = text.trim();

        while let Some(colon) = text.find(':') {
            let label = text[..colon].trim();
            if !is_identifier(label) {
                return Err(err(line, format!("invalid label '{label}'")));
            }
            if labels.insert(label.to_ascii_lowercase(), pending.len()).is_some() {
                return Err(err(line, format!("label '{label}' defined twice")));
            }
            text = text[colon + 1..].trim();
        }
        if text.is_empty() {
            continue;
        }

        let (mnemonic, rest) = match text.find(char::is_whitespace) {
            Some(pos) => (&text[..pos], text[pos..].trim()),
            None => (text, ""),
        };
        let operands = if rest.is_empty() {
            Vec::new()
        } else {
            rest.split(',').map(str::trim).collect()
        };
        pending.push(Pending { line, mnemonic, operands });
    }

    if pending.len() > MEMORY_WORDS {
        return Err(err(
            pending[MEMORY_WORDS].line,
            format!("program exceeds {MEMORY_WORDS} words"),
        ));
    }

    // Pass 2: encode.
    let mut program = Program::default();
    for p in &pending {
        program.words.push(encode(p, &labels)?);
        program.lines.push(p.line);
    }
    Ok(program)
}

fn encode(p: &Pending, labels: &HashMap<String, usize>) -> Result<u16, AsmError> {
    let line = p.line;
    if p.mnemonic.eq_ignore_ascii_case(".word") {
        expect_operands(p, 1)?;
        return parse_number(p.operands[0], line, 0xFFFF).map(|v| v as u16);
    }
    let op = op_by_mnemonic(p.mnemonic)
        .ok_or_else(|| err(line, format!("unknown instruction '{}'", p.mnemonic)))?;
    let reg = |i: usize| parse_register(p.operands[i], line);
    let addr = |i: usize| -> Result<u16, AsmError> {
        let text = p.operands[i];
        match labels.get(&text.to_ascii_lowercase()) {
            Some(&a) => Ok(a as u16),
            None if is_identifier(text) => Err(err(line, format!("unknown label '{text}'"))),
            None => parse_number(text, line, 0xFF).map(|v| v as u16),
        }
    };
    let base = op.opcode << 12;
    Ok(match op.format {
        Format::None => {
            expect_operands(p, 0)?;
            base
        }
        Format::RegRegReg => {
            expect_operands(p, 3)?;
            base | reg(0)? << 8 | reg(1)? << 4 | reg(2)?
        }
        Format::RegRegImm => {
            expect_operands(p, 3)?;
            let imm = parse_number(p.operands[2], line, 0xF)? as u16;
            base | reg(0)? << 8 | reg(1)? << 4 | imm
        }
        Format::Rd => {
            expect_operands(p, 1)?;
            base | reg(0)? << 8
        }
        Format::Ra => {
            expect_operands(p, 1)?;
            base | reg(0)? << 4
        }
        Format::Addr => {
            expect_operands(p, 1)?;
            base | addr(0)?
        }
        Format::RaAddr => {
            expect_operands(p, 2)?;
            base | reg(0)? << 8 | addr(1)?
        }
    })
}

fn expect_operands(p: &Pending, n: usize) -> Result<(), AsmError> {
    if p.operands.len() == n && p.operands.iter().all(|o| !o.is_empty()) {
        Ok(())
    } else {
        Err(err(
            p.line,
            format!("{} expects {n} operand(s), got {}", p.mnemonic.to_ascii_uppercase(), p.operands.len()),
        ))
    }
}

fn parse_register(text: &str, line: usize) -> Result<u16, AsmError> {
    let bad = || err(line, format!("expected register R0..R15, got '{text}'"));
    let digits = text.strip_prefix(['R', 'r']).ok_or_else(bad)?;
    match digits.parse::<u16>() {
        Ok(n) if n < 16 => Ok(n),
        _ => Err(bad()),
    }
}

fn parse_number(text: &str, line: usize, max: u32) -> Result<u32, AsmError> {
    let lower = text.to_ascii_lowercase();
    let parsed = if let Some(hex) = lower.strip_prefix("0x") {
        u32::from_str_radix(hex, 16)
    } else if let Some(bin) = lower.strip_prefix("0b") {
        u32::from_str_radix(bin, 2)
    } else {
        lower.parse::<u32>()
    };
    match parsed {
        Ok(v) if v <= max => Ok(v),
        Ok(v) => Err(err(line, format!("value {v} out of range (max {max})"))),
        Err(_) => Err(err(line, format!("expected a number, got '{text}'"))),
    }
}

fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn err(line: usize, message: String) -> AsmError {
    AsmError { line, message }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_every_format() {
        let p = assemble(
            "start: ADDI R1, R0, 5\n ADD R3, R1, R2\n OUT R3\n BRZ R1, end\n JMP start\n LW R6\n end: HALT\n .word 0xBEEF",
        )
        .unwrap();
        assert_eq!(p.words, vec![0x9105, 0x1312, 0x6030, 0x8106, 0x7000, 0xF600, 0x0000, 0xBEEF]);
        assert_eq!(p.lines, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn disassemble_roundtrips() {
        let src = "ADD R3, R1, R2\nADDI R1, R0, 5\nOUT R3\nJMP 0x02\nBRZ R1, 0x06\nLW R6\nHALT";
        let p = assemble(src).unwrap();
        let text: Vec<String> = p.words.iter().map(|&w| disassemble(w)).collect();
        assert_eq!(text.join("\n"), src);
    }

    #[test]
    fn comments_labels_and_case() {
        let p = assemble("; header\n\nloop: subi r1, r1, 1 // dec\n  brz R1, LOOP # back").unwrap();
        assert_eq!(p.words, vec![0xA111, 0x8100]);
        assert_eq!(p.lines, vec![3, 4]);
    }

    #[test]
    fn reports_errors_with_line() {
        assert_eq!(assemble("NOP").unwrap_err().line, 1);
        assert_eq!(assemble("HALT\nADDI R1, R0, 16").unwrap_err().line, 2);
        assert_eq!(assemble("JMP nowhere").unwrap_err().message, "unknown label 'nowhere'");
        assert_eq!(assemble("ADD R1, R2").unwrap_err().line, 1);
        assert_eq!(assemble("OUT R16").unwrap_err().line, 1);
    }
}
