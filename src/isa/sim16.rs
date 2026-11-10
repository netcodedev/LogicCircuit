//! Instruction-level simulator of the CPU_16B instruction set: executes one
//! instruction per step in plain Rust, thousands of times faster than the
//! gate-level circuits. It is the reference the circuits are tested against
//! and runs the benchmarks (`bench`), where it counts instructions.
//!
//! One memory of 64K words holds program and data (as on CPU_16B_SOC); on
//! the Harvard CPUs the same programs behave alike as long as they do not
//! store into their own code. It models the instruction set of
//! CPU_16B_SOC, including MULL/MULH/DIVU/MODU, EI/DI/RETI, interrupts and
//! LB/SB (NOP, NOP/RET and NOP on the other CPUs).

use super::cpu16::MEMORY_WORDS;

/// Condition flags.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Flags {
    pub z: bool,
    pub n: bool,
    pub c: bool,
    pub v: bool,
}

impl Flags {
    /// Branch condition `cond` (0 EQ .. 15 NV, as ARM).
    fn holds(self, cond: u16) -> bool {
        let Flags { z, n, c, v } = self;
        match cond {
            0 => z,
            1 => !z,
            2 => c,
            3 => !c,
            4 => n,
            5 => !n,
            6 => v,
            7 => !v,
            8 => c && !z,
            9 => !c || z,
            10 => n == v,
            11 => n != v,
            12 => !z && n == v,
            13 => z || n != v,
            14 => true,
            _ => false,
        }
    }
}

/// CPU state and memory.
#[derive(Debug, Clone)]
pub struct Cpu16 {
    /// R0 always reads 0.
    pub regs: [u16; 16],
    pub pc: u16,
    pub sp: u16,
    pub flags: Flags,
    pub mem: Vec<u16>,
    /// Value read by `IN`.
    pub input: u16,
    /// Output register written by `OUT`.
    pub out: u16,
    pub halted: bool,
    /// Executed instructions (an interrupt entry counts as one).
    pub steps: u64,
    /// Interrupts enabled (EI/DI/RETI).
    pub ie: bool,
    /// Flags saved at the interrupt entry, restored by RETI.
    pub saved_flags: Flags,
    /// Interrupt request line, checked before every instruction.
    pub irq: bool,
}

impl Cpu16 {
    /// State after reset with `program` at address 0.
    pub fn new(program: &[u16]) -> Self {
        let mut mem = vec![0; MEMORY_WORDS];
        mem[..program.len()].copy_from_slice(program);
        Self {
            regs: [0; 16],
            pc: 0,
            sp: 0,
            flags: Flags::default(),
            mem,
            input: 0,
            out: 0,
            halted: false,
            steps: 0,
            ie: false,
            saved_flags: Flags::default(),
            irq: false,
        }
    }

    fn reg(&self, r: u16) -> u16 {
        self.regs[r as usize]
    }

    fn set_reg(&mut self, r: u16, v: u16) {
        if r != 0 {
            self.regs[r as usize] = v;
        }
    }

    fn read(&self, addr: u16) -> u16 {
        self.mem[addr as usize]
    }

    fn write(&mut self, addr: u16, v: u16) {
        self.mem[addr as usize] = v;
    }

    fn push(&mut self, v: u16) {
        self.sp = self.sp.wrapping_sub(1);
        self.write(self.sp, v);
    }

    fn pop(&mut self) -> u16 {
        let v = self.read(self.sp);
        self.sp = self.sp.wrapping_add(1);
        v
    }

    fn set_zn(&mut self, v: u16) {
        self.flags.z = v == 0;
        self.flags.n = v & 0x8000 != 0;
    }

    /// a + b + cin with carry and signed overflow, as ALU_16B computes it.
    fn add(a: u16, b: u16, cin: bool) -> (u16, bool, bool) {
        let sum = a as u32 + b as u32 + cin as u32;
        let r = sum as u16;
        let v = (!(a ^ b) & (a ^ r)) & 0x8000 != 0;
        (r, sum > 0xFFFF, v)
    }

    /// Arithmetic with all four flags.
    fn arith(&mut self, a: u16, b: u16, cin: bool) -> u16 {
        let (r, c, v) = Self::add(a, b, cin);
        self.set_zn(r);
        self.flags.c = c;
        self.flags.v = v;
        r
    }

    fn shift(v: u16, amount: u16, k: u16) -> Option<u16> {
        let n = (amount & 15) as u32;
        Some(match k {
            0 => v << n,
            1 => v >> n,
            2 => v.rotate_left(n),
            3 => v.rotate_right(n),
            5 => ((v as i16) >> n) as u16,
            _ => return None,
        })
    }

    /// Executes one instruction; false (and nothing changes) once halted.
    pub fn step(&mut self) -> bool {
        if self.halted {
            return false;
        }
        // Interrupt entry instead of the next instruction (as CPU_16B_SOC):
        // CALL 0x0002 with the flags saved and Ie cleared.
        if self.irq && self.ie {
            self.push(self.pc);
            self.saved_flags = self.flags;
            self.ie = false;
            self.pc = 0x0002;
            self.steps += 1;
            return true;
        }
        let word = self.read(self.pc);
        let next = self.read(self.pc.wrapping_add(1));
        let (op, rd, rs, func) = (word >> 12, (word >> 8) & 15, (word >> 4) & 15, word & 15);
        let imm = word & 0xFF;
        let simm = imm as i8 as i16 as u16;
        let mut pc = self.pc.wrapping_add(1);
        match op {
            0 => match func {
                0 => {
                    self.halted = true;
                    return false;
                }
                // EI / DI: NOP with Rd = 1 / 2
                1 if rd == 1 => self.ie = true,
                1 if rd == 2 => self.ie = false,
                2 => {
                    pc = self.pop();
                    // RETI: RET with Rd = 1
                    if rd == 1 {
                        self.flags = self.saved_flags;
                        self.ie = true;
                    }
                }
                3 => pc = self.reg(rd),
                4 => {
                    self.push(pc);
                    pc = self.reg(rd);
                }
                5 => self.push(self.reg(rd)),
                6 => {
                    let v = self.pop();
                    self.set_reg(rd, v);
                }
                7 => self.out = self.reg(rd),
                8 => self.set_reg(rd, self.input),
                _ => {} // NOP and reserved
            },
            1 => {
                let (a, b) = (self.reg(rd), self.reg(rs));
                let c = self.flags.c;
                let r = match func {
                    0 => Some(self.arith(a, b, false)),
                    1 => Some(self.arith(a, !b, true)),
                    2 => Some(self.arith(a, b, c)),
                    3 => Some(self.arith(a, !b, c)),
                    10 => Some(self.arith(0, !b, true)),
                    11 => {
                        self.arith(a, !b, true);
                        None
                    }
                    4..=9 | 12 => {
                        let r = match func {
                            4 => a & b,
                            5 => a | b,
                            6 => a ^ b,
                            7 => a & !b,
                            8 => b,
                            9 => !b,
                            _ => a & b,
                        };
                        self.set_zn(r);
                        (func != 12).then_some(r)
                    }
                    _ => None,
                };
                if let Some(r) = r {
                    self.set_reg(rd, r);
                }
            }
            2 => {
                let amount = if func & 8 != 0 { rs } else { self.reg(rs) };
                if let Some(r) = Self::shift(self.reg(rd), amount, func & 7) {
                    self.set_zn(r);
                    self.set_reg(rd, r);
                }
            }
            3 => {
                let v = self.read(self.reg(rs).wrapping_add(func));
                self.set_reg(rd, v);
            }
            4 => self.write(self.reg(rs).wrapping_add(func), self.reg(rd)),
            5 => {
                if self.flags.holds(rd) {
                    pc = pc.wrapping_add(simm);
                }
            }
            6 => {
                pc = self.pc.wrapping_add(2);
                match func {
                    0 => self.set_reg(rd, next),
                    1 => pc = next,
                    2 => {
                        self.push(pc);
                        pc = next;
                    }
                    _ => {}
                }
            }
            7 if func < 4 => {
                let (a, b) = (self.reg(rd) as u32, self.reg(rs) as u32);
                let r = match func {
                    0 => (a * b) as u16,
                    1 => ((a * b) >> 16) as u16,
                    // Division by zero as MULDIV_16B computes it.
                    2 => a.checked_div(b).unwrap_or(0xFFFF) as u16,
                    _ => a.checked_rem(b).unwrap_or(a) as u16,
                };
                self.set_zn(r);
                self.set_reg(rd, r);
            }
            8 => self.set_reg(rd, imm),
            9 => {
                let r = self.arith(self.reg(rd), simm, false);
                self.set_reg(rd, r);
            }
            10 => {
                self.arith(self.reg(rd), !simm, true);
            }
            11..=13 => {
                let a = self.reg(rd);
                let r = match op {
                    11 => a & imm,
                    12 => a | imm,
                    _ => a ^ imm,
                };
                self.set_zn(r);
                self.set_reg(rd, r);
            }
            14 => self.set_reg(rd, imm << 8),
            // LB / SB: byte address Rs + off3, word = address / 2
            15 => {
                let ba = self.reg(rs).wrapping_add(func & 7);
                let (addr, shift) = (ba >> 1, (ba & 1) * 8);
                let word = self.read(addr);
                if func & 8 == 0 {
                    self.set_reg(rd, (word >> shift) & 0xFF);
                } else {
                    let byte = (self.reg(rd) & 0xFF) << shift;
                    self.write(addr, word & !(0xFF << shift) | byte);
                }
            }
            _ => {} // reserved: NOP
        }
        self.pc = pc;
        self.steps += 1;
        true
    }

    /// Runs until HALT or `max_steps`; returns the number of executed
    /// instructions.
    pub fn run(&mut self, max_steps: u64) -> u64 {
        let start = self.steps;
        while self.steps - start < max_steps && self.step() {}
        self.steps - start
    }
}

/// The benchmarks (programs/bench) with the results their headers state.
#[cfg(test)]
pub(crate) const BENCHMARKS: [(&str, &str, u16); 7] = [
    ("mul", include_str!("../../programs/bench/mul.asm"), 120),
    ("div", include_str!("../../programs/bench/div.asm"), 66),
    ("branch", include_str!("../../programs/bench/branch.asm"), 80),
    ("memory", include_str!("../../programs/bench/memory.asm"), 3384),
    ("calls", include_str!("../../programs/bench/calls.asm"), 34),
    ("mul_hw", include_str!("../../programs/bench/mul_hw.asm"), 120),
    ("div_hw", include_str!("../../programs/bench/div_hw.asm"), 66),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isa::Isa;

    /// SHIFTER_16B against the instruction-level shift for every amount and
    /// operation, on words that exercise every bit and the sign.
    #[test]
    fn shifter_circuit_matches_isa_shifts() {
        use crate::cldl::CircuitBuilder;
        use crate::machine::{read_bus, write_bus};
        let b = CircuitBuilder::new();
        let c = b.build("SHIFTER_16B");
        // (k of Cpu16::shift, Left, Rot, Arith)
        let ops = [(0, 1, 0, 0), (1, 0, 0, 0), (2, 1, 1, 0), (3, 0, 1, 0), (5, 0, 0, 1)];
        let mut words = vec![0x0000u16, 0xFFFF, 0x8000, 0x0001, 0x8421, 0xB00F, 0x7FFE, 0x5A5A];
        let mut x = 0x1234u16;
        for _ in 0..24 {
            x = x.wrapping_mul(25173).wrapping_add(13849);
            words.push(x);
        }
        for (k, left, rot, arith) in ops {
            c.set_input("Left", left == 1);
            c.set_input("Rot", rot == 1);
            c.set_input("Arith", arith == 1);
            for &v in &words {
                for n in 0..16u16 {
                    write_bus(&c, "A", 16, v as u32);
                    write_bus(&c, "Amt", 4, n as u32);
                    c.evaluate_stable();
                    let want = Cpu16::shift(v, n, k).unwrap();
                    assert_eq!(read_bus(&c, "Out", 16) as u16, want, "k={k} v={v:#06x} n={n}");
                }
            }
        }
    }

    fn run(src: &str, input: u16) -> Cpu16 {
        let mut cpu = Cpu16::new(&Isa::Cpu16.assemble(src).unwrap().words);
        cpu.input = input;
        cpu.run(1_000_000);
        assert!(cpu.halted, "no HALT");
        cpu
    }

    #[test]
    fn runs_example_programs() {
        for (src, input, out) in [
            (include_str!("../../programs/16/fibonacci.asm"), 0, 46368),
            (include_str!("../../programs/16/multiply.asm"), 0, 1234 * 37),
            (include_str!("../../programs/16/factorial.asm"), 8, 40320),
            (include_str!("../../programs/16/sort.asm"), 0, 0x1234),
        ] {
            assert_eq!(run(src, input).out, out);
        }
    }

    #[test]
    fn benchmarks_compute_their_results() {
        for (name, src, out) in BENCHMARKS {
            let cpu = run(src, 0);
            assert_eq!(cpu.out, out, "{name}");
            assert!(cpu.steps < 5000, "{name}: {} instructions, too slow for the gate-level runs", cpu.steps);
        }
    }

    #[test]
    fn interrupt_entry_and_reti() {
        // Handler at 0x0002 counts in R9; main sets C, gets interrupted
        // between SUB and ADC and must still see C.
        let src = "JMP main\n INC R9\n CMPI R9, 0\n RETI\n\
                   main: EI\n MOVI R1, 5\n MOVI R2, 3\n SUB R1, R2\n ADC R3, R0\n OUT R3\n HALT";
        let mut cpu = Cpu16::new(&Isa::Cpu16.assemble(src).unwrap().words);
        while cpu.regs[1] != 2 {
            cpu.step(); // up to and including SUB (C = 1: no borrow)
        }
        cpu.irq = true;
        cpu.step(); // entry
        assert_eq!((cpu.pc, cpu.ie), (0x0002, false));
        cpu.irq = false;
        cpu.run(1000);
        assert_eq!(cpu.regs[9], 1);
        assert_eq!(cpu.out, 1, "C lost across the interrupt");
        assert!(cpu.ie);
    }

    #[test]
    fn flags_and_carry_chain() {
        // 32-bit add (0x0001FFFF + 1) and subtract back, then signed overflow.
        let cpu = run(
            "LI R8, 0xFFFF\n MOVI R9, 1\n MOVI R10, 1\n ADD R8, R10\n ADC R9, R0\n\
             SUB R8, R10\n SBC R9, R0\n OUT R9\n LI R7, 0x7FFF\n ADDI R7, 1\n HALT",
            0,
        );
        assert_eq!(cpu.out, 1);
        assert_eq!(cpu.regs[8], 0xFFFF);
        assert!(cpu.flags.v && cpu.flags.n && !cpu.flags.z);
    }
}
