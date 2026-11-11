//! Tests of the RISC-V circuits against the Rust reference (random vectors).

use std::path::Path;

use crate::circuit::Circuit;
use crate::cldl::CircuitBuilder;

use super::isa::{Format, OPS, decode};

pub(crate) fn builder() -> CircuitBuilder {
    let mut b = CircuitBuilder::new();
    b.load_circuits_from_path(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/circuits"))).unwrap();
    b
}

/// xorshift64
pub(crate) struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// Random 32-bit value, biased towards edge cases.
    pub fn word(&mut self) -> u32 {
        let r = self.next();
        match r % 8 {
            0 => [0, 1, u32::MAX, 0x8000_0000, 0x7FFF_FFFF, 31, 32][(r >> 8) as usize % 7],
            1 => (r >> 8) as u32 & 0xFF,
            2 => ((r >> 8) as u32 & 0xFF).wrapping_neg(),
            _ => (r >> 16) as u32,
        }
    }
}

pub(crate) fn set_bus(c: &Circuit, name: &str, width: usize, v: u64) {
    for i in 0..width {
        c.set_input(&format!("{name}.{i}"), v >> i & 1 == 1);
    }
}

pub(crate) fn bus(c: &Circuit, name: &str, width: usize) -> u64 {
    (0..width).fold(0, |acc, i| acc | (c.get_wire_value(&format!("{name}.{i}")) as u64) << i)
}

#[test]
fn adder_ks5() {
    let c = builder().build("ADDER_KS5<32>");
    let mut rng = Rng(1);
    for _ in 0..500 {
        let (a, b, cin) = (rng.word(), rng.word(), rng.next() & 1);
        set_bus(&c, "A", 32, a as u64);
        set_bus(&c, "B", 32, b as u64);
        c.set_input("Cin", cin == 1);
        c.evaluate_stable();
        let sum = a as u64 + b as u64 + cin;
        assert_eq!(bus(&c, "S", 32), sum & 0xFFFF_FFFF, "{a:x} + {b:x} + {cin}");
        assert_eq!(c.get_wire_value("Cout"), sum >> 32 == 1);
    }
}

#[test]
fn decoder_5_32() {
    let c = builder().build("DECODER_5_32");
    for en in [false, true] {
        c.set_input("Enable", en);
        for a in 0..32 {
            set_bus(&c, "A", 5, a);
            c.evaluate_stable();
            assert_eq!(bus(&c, "O", 32), if en { 1 << a } else { 0 }, "A={a}");
        }
    }
}

#[test]
fn reg_file_32() {
    let c = builder().build("REG_FILE_32<32>");
    let mut want = [0u32; 32];
    let mut rng = Rng(2);
    for _ in 0..300 {
        let (wa, v, we) = (rng.next() % 32, rng.word(), rng.next() % 4 != 0);
        set_bus(&c, "WriteAddr", 5, wa);
        set_bus(&c, "DataIn", 32, v as u64);
        c.set_input("WriteEnable", we);
        c.evaluate_stable();
        c.set_input("CLK", true);
        c.evaluate_stable();
        c.set_input("CLK", false);
        c.evaluate_stable();
        if we && wa != 0 {
            want[wa as usize] = v;
        }
        let (ra, rb) = (rng.next() % 32, rng.next() % 32);
        set_bus(&c, "ReadAddrA", 5, ra);
        set_bus(&c, "ReadAddrB", 5, rb);
        c.evaluate_stable();
        assert_eq!(bus(&c, "DataOutA", 32), want[ra as usize] as u64);
        assert_eq!(bus(&c, "DataOutB", 32), want[rb as usize] as u64);
    }
}

#[test]
fn alu_rv32() {
    let c = builder().build("ALU_RV32");
    let mut rng = Rng(3);
    for _ in 0..3000 {
        let (a, b) = (rng.word(), rng.word());
        let f = rng.next() % 8;
        let sub = rng.next() & 1 == 1;
        let arith = rng.next() & 1 == 1;
        set_bus(&c, "A", 32, a as u64);
        set_bus(&c, "B", 32, b as u64);
        set_bus(&c, "F", 3, f);
        c.set_input("Sub", sub);
        c.set_input("Arith", arith);
        c.evaluate_stable();
        let sh = b & 31;
        let want = match f {
            0 if sub => a.wrapping_sub(b),
            0 => a.wrapping_add(b),
            1 => a << sh,
            2 if sub => ((a as i32) < (b as i32)) as u32,
            3 if sub => (a < b) as u32,
            4 => a ^ b,
            5 if arith => ((a as i32) >> sh) as u32,
            5 => a >> sh,
            6 => a | b,
            7 => a & b,
            _ => continue, // SLT/SLTU need Sub
        };
        assert_eq!(bus(&c, "Result", 32) as u32, want, "F={f} sub={sub} arith={arith} {a:x} {b:x}");
        assert_eq!(c.get_wire_value("Eq"), a == b);
        if sub {
            assert_eq!(c.get_wire_value("Lt"), (a as i32) < (b as i32), "{a:x} < {b:x}");
            assert_eq!(c.get_wire_value("Ltu"), a < b);
        }
    }
}

#[test]
fn branch_rv32() {
    let c = builder().build("BRANCH_RV32");
    let mut rng = Rng(7);
    for k in 0..3000 {
        let a = rng.word();
        let b = match k % 4 {
            0 => a,
            1 => a ^ 0x8000_0000,
            _ => rng.word(),
        };
        let f = [0, 1, 4, 5, 6, 7][k % 6];
        set_bus(&c, "A", 32, a as u64);
        set_bus(&c, "B", 32, b as u64);
        set_bus(&c, "F", 3, f);
        c.evaluate_stable();
        let want = match f {
            0 => a == b,
            1 => a != b,
            4 => (a as i32) < (b as i32),
            5 => (a as i32) >= (b as i32),
            6 => a < b,
            _ => a >= b,
        };
        assert_eq!(c.get_wire_value("Cond"), want, "F={f} {a:x} {b:x}");
    }
}

#[test]
fn imm_rv32() {
    let c = builder().build("IMM_RV32");
    let mut rng = Rng(4);
    for info in OPS {
        let fmt = match info.format {
            Format::I | Format::Load | Format::Jalr | Format::Shift => "FmtI",
            Format::S => "FmtS",
            Format::B => "FmtB",
            Format::U => "FmtU",
            Format::J => "FmtJ",
            _ => continue,
        };
        for f in ["FmtI", "FmtS", "FmtB", "FmtU", "FmtJ"] {
            c.set_input(f, f == fmt);
        }
        for _ in 0..200 {
            // A random word with this opcode and funct3, funct7 where needed.
            let w = (rng.next() as u32 & !0x707F) | info.opcode | info.funct3 << 12;
            let w = if info.format == Format::Shift { w & 0x01FF_FFFF | info.funct7 << 25 } else { w };
            let Some(inst) = decode(w) else { panic!("{w:08x}") };
            set_bus(&c, "Ir", 32, w as u64);
            c.evaluate_stable();
            let want = if info.format == Format::Shift {
                // Shifts read the I immediate; only bits 4..0 count.
                ((w as i32) >> 20) as u32
            } else {
                inst.imm as u32
            };
            assert_eq!(bus(&c, "Imm", 32) as u32, want, "{} {w:08x}", info.mnemonic);
        }
    }
}

#[test]
fn control_rv32() {
    use super::isa::Op::*;
    let c = builder().build("CONTROL_RV32");
    let mut rng = Rng(5);
    let opcodes = [0x03, 0x0F, 0x13, 0x17, 0x23, 0x33, 0x37, 0x63, 0x67, 0x6F, 0x73, 0x0B, 0x2F, 0x53, 0x7F];
    let mut words: Vec<u32> = OPS.iter().map(|i| crate::riscv::isa::encode(&crate::riscv::isa::Inst::new(i.op)).unwrap()).collect();
    for _ in 0..20000 {
        let r = rng.next();
        let mut w = (r >> 16) as u32;
        w = w & !0x7F | opcodes[(r % opcodes.len() as u64) as usize];
        // Make funct7 / system words likely to be legal sometimes.
        match r >> 60 {
            0 => w &= 0x01FF_FFFF,
            1 => w = w & 0x01FF_FFFF | 0x4000_0000,
            2 => w = w & 0x01FF_FFFF | 0x0200_0000,
            3 => w = [0x0000_0073, 0x0010_0073, 0x3020_0073, 0x1050_0073][(r >> 8) as usize % 4] | (w & 0xF8F80) * ((r >> 12) & 1) as u32,
            _ => {}
        }
        words.push(w);
    }
    for w in words {
        set_bus(&c, "Ir", 32, w as u64);
        c.evaluate_stable();
        let d = decode(w);
        let get = |n: &str| c.get_wire_value(n);
        assert_eq!(get("Illegal"), d.is_none(), "{w:08x} {d:?}");
        let Some(i) = d else { continue };
        let is = |ops: &[super::isa::Op]| ops.contains(&i.op);
        let f3 = (w >> 12) & 7;
        assert_eq!(get("IsLoad"), is(&[Lb, Lh, Lw, Lbu, Lhu]), "{w:08x}");
        assert_eq!(get("IsStore"), is(&[Sb, Sh, Sw]), "{w:08x}");
        assert_eq!(get("IsBranch"), is(&[Beq, Bne, Blt, Bge, Bltu, Bgeu]));
        assert_eq!(get("IsJal"), i.op == Jal);
        assert_eq!(get("IsJalr"), i.op == Jalr);
        assert_eq!(get("IsLui"), i.op == Lui);
        assert_eq!(get("IsAuipc"), i.op == Auipc);
        let csr = is(&[Csrrw, Csrrs, Csrrc, Csrrwi, Csrrsi, Csrrci]);
        assert_eq!(get("IsCsr"), csr, "{w:08x}");
        assert_eq!(get("Ecall"), i.op == Ecall);
        assert_eq!(get("Ebreak"), i.op == Ebreak);
        assert_eq!(get("Mret"), i.op == Mret);
        assert_eq!(get("IsMulDiv"), is(&[Mul, Mulh, Mulhsu, Mulhu, Div, Divu, Rem, Remu]));
        let alu = get("IsOp") || get("IsOpImm");
        assert_eq!(bus(&c, "AluF", 3) as u32, if alu { f3 } else { 0 }, "{w:08x}");
        let sub = is(&[Beq, Bne, Blt, Bge, Bltu, Bgeu, Sub, Slt, Sltu, Slti, Sltiu]);
        // (Don't care for M instructions: the ALU result is not used.)
        if !get("IsMulDiv") {
            assert_eq!(get("AluSub"), sub, "{w:08x} {:?}", i.op);
        }
        let writes = !is(&[Sb, Sh, Sw, Beq, Bne, Blt, Bge, Bltu, Bgeu, Fence, Ecall, Ebreak, Mret, Wfi]);
        assert_eq!(get("RegWrite"), writes, "{w:08x} {:?}", i.op);
        let fmt = match w & 0x7F {
            0x03 | 0x13 | 0x67 => "FmtI",
            0x23 => "FmtS",
            0x63 => "FmtB",
            0x37 | 0x17 => "FmtU",
            0x6F => "FmtJ",
            _ => "",
        };
        for f in ["FmtI", "FmtS", "FmtB", "FmtU", "FmtJ"] {
            assert_eq!(get(f), f == fmt, "{f} {w:08x}");
        }
    }
}

#[test]
fn lsu_rv32() {
    let c = builder().build("LSU_RV32");
    let mut rng = Rng(6);
    for _ in 0..2000 {
        let (rs2, word) = (rng.word(), rng.word());
        let size = rng.next() % 3;
        let unsigned = size < 2 && rng.next() & 1 == 1;
        let addr = (rng.next() % 4) as u32 & !((1u32 << size) - 1);
        set_bus(&c, "Rs2", 32, rs2 as u64);
        set_bus(&c, "Word", 32, word as u64);
        set_bus(&c, "Size", 2, size);
        c.set_input("Unsigned", unsigned);
        set_bus(&c, "Addr", 2, addr as u64);
        c.evaluate_stable();
        let en = bus(&c, "ByteEn", 4) as u32;
        let want_en = match size {
            0 => 1 << addr,
            1 => 3 << addr,
            _ => 15,
        };
        assert_eq!(en, want_en, "size {size} addr {addr}");
        // The enabled lanes hold the value at its place.
        let data = bus(&c, "StoreData", 32) as u32;
        let mask = (0..4).filter(|k| en >> k & 1 == 1).fold(0u32, |m, k| m | 0xFF << (8 * k));
        assert_eq!(data & mask, (rs2 << (8 * addr)) & mask, "store size {size} addr {addr}");
        let v = word >> (8 * addr);
        let want = match (size, unsigned) {
            (0, false) => v as u8 as i8 as u32,
            (0, true) => v & 0xFF,
            (1, false) => v as u16 as i16 as u32,
            (1, true) => v & 0xFFFF,
            _ => word,
        };
        assert_eq!(bus(&c, "LoadData", 32) as u32, want, "load size {size} u {unsigned} addr {addr} {word:08x}");
    }
}
