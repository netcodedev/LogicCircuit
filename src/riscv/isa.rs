//! RV32IM + Zicsr instruction encoding: one table drives the decoder, the
//! encoder (used by the assembler) and the disassembler.
//!
//! Formats (bit fields of the 32-bit word):
//! - R: `funct7 | rs2 | rs1 | funct3 | rd | opcode`
//! - I: `imm[11:0] | rs1 | funct3 | rd | opcode`
//! - S: `imm[11:5] | rs2 | rs1 | funct3 | imm[4:0] | opcode`
//! - B: `imm[12|10:5] | rs2 | rs1 | funct3 | imm[4:1|11] | opcode`
//! - U: `imm[31:12] | rd | opcode`
//! - J: `imm[20|10:1|11|19:12] | rd | opcode`

use std::fmt;

/// Operand layout of an instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// `add rd, rs1, rs2`
    R,
    /// `addi rd, rs1, imm12`
    I,
    /// `slli rd, rs1, shamt` (funct7 in imm[11:5])
    Shift,
    /// `lw rd, imm(rs1)`
    Load,
    /// `jalr rd, imm(rs1)`
    Jalr,
    /// `sw rs2, imm(rs1)`
    S,
    /// `beq rs1, rs2, target`
    B,
    /// `lui rd, imm20`
    U,
    /// `jal rd, target`
    J,
    /// `csrrw rd, csr, rs1`
    Csr,
    /// `csrrwi rd, csr, uimm5`
    CsrI,
    /// No operands, fixed word (`ecall`, `mret`, ...).
    Fixed,
    /// `fence` (operands ignored)
    Fence,
}

/// One instruction of the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpInfo {
    pub op: Op,
    pub mnemonic: &'static str,
    pub format: Format,
    pub opcode: u32,
    pub funct3: u32,
    /// funct7 for R and Shift, the whole word for Fixed.
    pub funct7: u32,
}

macro_rules! ops {
    ($($op:ident $name:literal $fmt:ident $opc:literal $f3:literal $f7:literal;)*) => {
        /// An instruction.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Op { $($op,)* }
        pub const OPS: &[OpInfo] = &[
            $(OpInfo { op: Op::$op, mnemonic: $name, format: Format::$fmt, opcode: $opc, funct3: $f3, funct7: $f7 },)*
        ];
    };
}

ops! {
    Lui "lui" U 0x37 0 0;
    Auipc "auipc" U 0x17 0 0;
    Jal "jal" J 0x6F 0 0;
    Jalr "jalr" Jalr 0x67 0 0;
    Beq "beq" B 0x63 0 0;
    Bne "bne" B 0x63 1 0;
    Blt "blt" B 0x63 4 0;
    Bge "bge" B 0x63 5 0;
    Bltu "bltu" B 0x63 6 0;
    Bgeu "bgeu" B 0x63 7 0;
    Lb "lb" Load 0x03 0 0;
    Lh "lh" Load 0x03 1 0;
    Lw "lw" Load 0x03 2 0;
    Lbu "lbu" Load 0x03 4 0;
    Lhu "lhu" Load 0x03 5 0;
    Sb "sb" S 0x23 0 0;
    Sh "sh" S 0x23 1 0;
    Sw "sw" S 0x23 2 0;
    Addi "addi" I 0x13 0 0;
    Slti "slti" I 0x13 2 0;
    Sltiu "sltiu" I 0x13 3 0;
    Xori "xori" I 0x13 4 0;
    Ori "ori" I 0x13 6 0;
    Andi "andi" I 0x13 7 0;
    Slli "slli" Shift 0x13 1 0x00;
    Srli "srli" Shift 0x13 5 0x00;
    Srai "srai" Shift 0x13 5 0x20;
    Add "add" R 0x33 0 0x00;
    Sub "sub" R 0x33 0 0x20;
    Sll "sll" R 0x33 1 0x00;
    Slt "slt" R 0x33 2 0x00;
    Sltu "sltu" R 0x33 3 0x00;
    Xor "xor" R 0x33 4 0x00;
    Srl "srl" R 0x33 5 0x00;
    Sra "sra" R 0x33 5 0x20;
    Or "or" R 0x33 6 0x00;
    And "and" R 0x33 7 0x00;
    Mul "mul" R 0x33 0 0x01;
    Mulh "mulh" R 0x33 1 0x01;
    Mulhsu "mulhsu" R 0x33 2 0x01;
    Mulhu "mulhu" R 0x33 3 0x01;
    Div "div" R 0x33 4 0x01;
    Divu "divu" R 0x33 5 0x01;
    Rem "rem" R 0x33 6 0x01;
    Remu "remu" R 0x33 7 0x01;
    Fence "fence" Fence 0x0F 0 0;
    Ecall "ecall" Fixed 0x73 0 0x0000_0073;
    Ebreak "ebreak" Fixed 0x73 0 0x0010_0073;
    Mret "mret" Fixed 0x73 0 0x3020_0073;
    Wfi "wfi" Fixed 0x73 0 0x1050_0073;
    Csrrw "csrrw" Csr 0x73 1 0;
    Csrrs "csrrs" Csr 0x73 2 0;
    Csrrc "csrrc" Csr 0x73 3 0;
    Csrrwi "csrrwi" CsrI 0x73 5 0;
    Csrrsi "csrrsi" CsrI 0x73 6 0;
    Csrrci "csrrci" CsrI 0x73 7 0;
}

impl Op {
    pub fn info(self) -> &'static OpInfo {
        OPS.iter().find(|i| i.op == self).unwrap()
    }

    pub fn by_mnemonic(name: &str) -> Option<Op> {
        OPS.iter().find(|i| i.mnemonic.eq_ignore_ascii_case(name)).map(|i| i.op)
    }
}

/// A decoded instruction. `imm` is the sign-extended immediate (the CSR
/// number for CSR instructions, the shift amount for shifts); for `CsrI`
/// the 5-bit immediate is in `rs1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inst {
    pub op: Op,
    pub rd: u8,
    pub rs1: u8,
    pub rs2: u8,
    pub imm: i32,
}

impl Inst {
    pub fn new(op: Op) -> Inst {
        Inst { op, rd: 0, rs1: 0, rs2: 0, imm: 0 }
    }
}

fn bits(w: u32, hi: u32, lo: u32) -> u32 {
    (w >> lo) & ((1 << (hi - lo + 1)) - 1)
}

fn sext(v: u32, width: u32) -> i32 {
    ((v << (32 - width)) as i32) >> (32 - width)
}

/// Decodes a word; `None` for an illegal instruction.
pub fn decode(w: u32) -> Option<Inst> {
    let opcode = w & 0x7F;
    let rd = bits(w, 11, 7) as u8;
    let funct3 = bits(w, 14, 12);
    let rs1 = bits(w, 19, 15) as u8;
    let rs2 = bits(w, 24, 20) as u8;
    let funct7 = bits(w, 31, 25);
    let info = OPS.iter().find(|i| {
        i.opcode == opcode
            && match i.format {
                Format::U | Format::J => true,
                Format::Fixed => w == i.funct7,
                Format::R | Format::Shift => i.funct3 == funct3 && i.funct7 == funct7,
                Format::Fence => funct3 == 0,
                _ => i.funct3 == funct3,
            }
    })?;
    let imm = match info.format {
        Format::I | Format::Load | Format::Jalr => sext(bits(w, 31, 20), 12),
        Format::Shift => rs2 as i32,
        Format::Csr | Format::CsrI => bits(w, 31, 20) as i32,
        Format::S => sext(bits(w, 31, 25) << 5 | bits(w, 11, 7), 12),
        Format::B => sext(
            bits(w, 31, 31) << 12 | bits(w, 7, 7) << 11 | bits(w, 30, 25) << 5 | bits(w, 11, 8) << 1,
            13,
        ),
        Format::U => (w & 0xFFFF_F000) as i32,
        Format::J => sext(
            bits(w, 31, 31) << 20 | bits(w, 19, 12) << 12 | bits(w, 20, 20) << 11 | bits(w, 30, 21) << 1,
            21,
        ),
        Format::R | Format::Fixed | Format::Fence => 0,
    };
    let (rd, rs1, rs2) = match info.format {
        Format::R => (rd, rs1, rs2),
        Format::I | Format::Shift | Format::Load | Format::Jalr | Format::Csr | Format::CsrI => (rd, rs1, 0),
        Format::S | Format::B => (0, rs1, rs2),
        Format::U | Format::J => (rd, 0, 0),
        Format::Fixed | Format::Fence => (0, 0, 0),
    };
    Some(Inst { op: info.op, rd, rs1, rs2, imm })
}

/// Error from `encode`: an operand out of range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodeError(pub String);

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn check(cond: bool, msg: impl FnOnce() -> String) -> Result<(), EncodeError> {
    if cond { Ok(()) } else { Err(EncodeError(msg())) }
}

/// Encodes an instruction (branch and jump immediates are byte offsets).
pub fn encode(i: &Inst) -> Result<u32, EncodeError> {
    let info = i.op.info();
    let (rd, rs1, rs2) = (i.rd as u32 & 31, i.rs1 as u32 & 31, i.rs2 as u32 & 31);
    let base = info.opcode | info.funct3 << 12;
    let imm = i.imm;
    let in_signed = |bits: u32| imm >= -(1 << (bits - 1)) && imm < (1 << (bits - 1));
    Ok(match info.format {
        Format::R => base | rd << 7 | rs1 << 15 | rs2 << 20 | info.funct7 << 25,
        Format::I | Format::Load | Format::Jalr => {
            check(in_signed(12), || format!("immediate {imm} out of range (-2048..2047)"))?;
            base | rd << 7 | rs1 << 15 | (imm as u32 & 0xFFF) << 20
        }
        Format::Shift => {
            check((0..32).contains(&imm), || format!("shift amount {imm} out of range (0..31)"))?;
            base | rd << 7 | rs1 << 15 | (imm as u32) << 20 | info.funct7 << 25
        }
        Format::Csr | Format::CsrI => {
            check((0..4096).contains(&imm), || format!("CSR number {imm} out of range"))?;
            base | rd << 7 | rs1 << 15 | (imm as u32) << 20
        }
        Format::S => {
            check(in_signed(12), || format!("offset {imm} out of range (-2048..2047)"))?;
            let u = imm as u32;
            base | (u & 0x1F) << 7 | rs1 << 15 | rs2 << 20 | (u >> 5 & 0x7F) << 25
        }
        Format::B => {
            check(in_signed(13) && imm % 2 == 0, || format!("branch offset {imm} out of range (±4 KiB, even)"))?;
            let u = imm as u32;
            base | (u >> 11 & 1) << 7
                | (u >> 1 & 0xF) << 8
                | rs1 << 15
                | rs2 << 20
                | (u >> 5 & 0x3F) << 25
                | (u >> 12 & 1) << 31
        }
        Format::U => {
            check(imm as u32 & 0xFFF == 0, || "U immediate must be a multiple of 4096".into())?;
            base | rd << 7 | (imm as u32 & 0xFFFF_F000)
        }
        Format::J => {
            check(in_signed(21) && imm % 2 == 0, || format!("jump offset {imm} out of range (±1 MiB, even)"))?;
            let u = imm as u32;
            base | rd << 7 | (u >> 12 & 0xFF) << 12 | (u >> 11 & 1) << 20 | (u >> 1 & 0x3FF) << 21 | (u >> 20 & 1) << 31
        }
        Format::Fixed => info.funct7,
        Format::Fence => base | 0x0FF0_0000,
    })
}

/// ABI register names, x0..x31.
pub const REG_NAMES: [&str; 32] = [
    "zero", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7", "s2",
    "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "t3", "t4", "t5", "t6",
];

/// Register number for `x5`, `t0` or `fp`.
pub fn reg_by_name(name: &str) -> Option<u8> {
    let name = name.to_ascii_lowercase();
    if name == "fp" {
        return Some(8);
    }
    if let Some(n) = name.strip_prefix('x').and_then(|n| n.parse::<u8>().ok()) {
        return (n < 32).then_some(n);
    }
    REG_NAMES.iter().position(|&r| r == name).map(|n| n as u8)
}

/// Machine-mode CSRs the SoC implements.
pub mod csr {
    pub const MSTATUS: u16 = 0x300;
    pub const MISA: u16 = 0x301;
    pub const MIE: u16 = 0x304;
    pub const MTVEC: u16 = 0x305;
    pub const MSCRATCH: u16 = 0x340;
    pub const MEPC: u16 = 0x341;
    pub const MCAUSE: u16 = 0x342;
    pub const MTVAL: u16 = 0x343;
    pub const MIP: u16 = 0x344;
    pub const MCYCLE: u16 = 0xB00;
    pub const MINSTRET: u16 = 0xB02;
    pub const MCYCLEH: u16 = 0xB80;
    pub const MINSTRETH: u16 = 0xB82;
    pub const CYCLE: u16 = 0xC00;
    pub const INSTRET: u16 = 0xC02;
    pub const CYCLEH: u16 = 0xC80;
    pub const INSTRETH: u16 = 0xC82;
    pub const MHARTID: u16 = 0xF14;

    pub const NAMES: &[(&str, u16)] = &[
        ("mstatus", MSTATUS),
        ("misa", MISA),
        ("mie", MIE),
        ("mtvec", MTVEC),
        ("mscratch", MSCRATCH),
        ("mepc", MEPC),
        ("mcause", MCAUSE),
        ("mtval", MTVAL),
        ("mip", MIP),
        ("mcycle", MCYCLE),
        ("minstret", MINSTRET),
        ("mcycleh", MCYCLEH),
        ("minstreth", MINSTRETH),
        ("cycle", CYCLE),
        ("instret", INSTRET),
        ("cycleh", CYCLEH),
        ("instreth", INSTRETH),
        ("mhartid", MHARTID),
    ];

    pub fn by_name(name: &str) -> Option<u16> {
        NAMES.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|&(_, c)| c)
    }

    pub fn name(csr: u16) -> Option<&'static str> {
        NAMES.iter().find(|&&(_, c)| c == csr).map(|&(n, _)| n)
    }
}

/// Renders an instruction at address `pc` (branch and jump targets as
/// absolute addresses), e.g. `addi a0, a0, 1` or `beq a0, zero, 0x40`.
pub fn disassemble(pc: u32, word: u32) -> String {
    let Some(i) = decode(word) else {
        return format!(".word 0x{word:08x}");
    };
    let info = i.op.info();
    let r = |n: u8| REG_NAMES[n as usize];
    let m = info.mnemonic;
    let target = pc.wrapping_add(i.imm as u32);
    let csr_name = |c: i32| csr::name(c as u16).map_or(format!("0x{c:03x}"), str::to_string);
    match info.format {
        Format::R => format!("{m} {}, {}, {}", r(i.rd), r(i.rs1), r(i.rs2)),
        Format::I | Format::Shift => format!("{m} {}, {}, {}", r(i.rd), r(i.rs1), i.imm),
        Format::Load | Format::Jalr => format!("{m} {}, {}({})", r(i.rd), i.imm, r(i.rs1)),
        Format::S => format!("{m} {}, {}({})", r(i.rs2), i.imm, r(i.rs1)),
        Format::B => format!("{m} {}, {}, 0x{target:x}", r(i.rs1), r(i.rs2)),
        Format::U => format!("{m} {}, 0x{:x}", r(i.rd), (i.imm as u32) >> 12),
        Format::J => format!("{m} {}, 0x{target:x}", r(i.rd)),
        Format::Csr => format!("{m} {}, {}, {}", r(i.rd), csr_name(i.imm), r(i.rs1)),
        Format::CsrI => format!("{m} {}, {}, {}", r(i.rd), csr_name(i.imm), i.rs1),
        Format::Fixed | Format::Fence => m.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_round_trip() {
        let cases = [
            (Inst { op: Op::Addi, rd: 10, rs1: 10, rs2: 0, imm: -1 }, 0xFFF5_0513),
            (Inst { op: Op::Add, rd: 1, rs1: 2, rs2: 3, imm: 0 }, 0x0031_00B3),
            (Inst { op: Op::Sw, rd: 0, rs1: 2, rs2: 1, imm: 12 }, 0x0011_2623),
            (Inst { op: Op::Beq, rd: 0, rs1: 10, rs2: 0, imm: -8 }, 0xFE05_0CE3),
            (Inst { op: Op::Jal, rd: 1, rs1: 0, rs2: 0, imm: 2048 }, 0x0010_00EF),
            (Inst { op: Op::Lui, rd: 5, rs1: 0, rs2: 0, imm: 0x12345000 }, 0x1234_52B7),
            (Inst { op: Op::Srai, rd: 5, rs1: 5, rs2: 0, imm: 3 }, 0x4032_D293),
            (Inst { op: Op::Csrrw, rd: 0, rs1: 5, rs2: 0, imm: 0x305 }, 0x3052_9073),
            (Inst { op: Op::Mret, rd: 0, rs1: 0, rs2: 0, imm: 0 }, 0x3020_0073),
            (Inst { op: Op::Divu, rd: 10, rs1: 11, rs2: 12, imm: 0 }, 0x02C5_D533),
        ];
        for (inst, word) in cases {
            assert_eq!(encode(&inst).unwrap(), word, "{inst:?}");
            assert_eq!(decode(word), Some(inst), "{word:08x}");
        }
        assert_eq!(decode(0), None);
        assert_eq!(decode(0xFFFF_FFFF), None);
    }

    #[test]
    fn disassembles() {
        assert_eq!(disassemble(0x100, 0xFE05_0CE3), "beq a0, zero, 0xf8");
        assert_eq!(disassemble(0, 0x0011_2623), "sw ra, 12(sp)");
        assert_eq!(disassemble(0, 0x3052_9073), "csrrw zero, mtvec, t0");
        assert_eq!(reg_by_name("x31"), Some(31));
        assert_eq!(reg_by_name("s11"), Some(27));
        assert_eq!(reg_by_name("x32"), None);
    }
}
