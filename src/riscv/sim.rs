//! Instruction-level simulator for the RISC-V SoC: RV32IM + Zicsr in
//! machine mode, with the SoC's memory map (see [`super::map`]). It is the
//! reference the circuit is checked against: same program, same final
//! registers, memory and I/O.
//!
//! Deviations from a full RV32 machine-mode hart, shared with the circuit:
//! `ebreak` halts the machine (instead of a breakpoint trap); `wfi` is a
//! `nop`; `fence` is a `nop`; `mcycle` counts instructions here (clocks in
//! the circuit). Misaligned loads, stores and jump targets trap.

use std::collections::VecDeque;

use super::isa::{Inst, Op, csr, decode};
use super::map;

/// Exception causes (`mcause` with the top bit clear).
pub mod cause {
    pub const MISALIGNED_FETCH: u32 = 0;
    pub const ILLEGAL: u32 = 2;
    pub const MISALIGNED_LOAD: u32 = 4;
    pub const MISALIGNED_STORE: u32 = 6;
    pub const ECALL_M: u32 = 11;
    /// Interrupts (top bit set).
    pub const TIMER_INT: u32 = 0x8000_0007;
    pub const EXTERNAL_INT: u32 = 0x8000_000B;
}

const MSTATUS_MIE: u32 = 1 << 3;
const MSTATUS_MPIE: u32 = 1 << 7;
/// MPP is fixed to machine mode.
const MSTATUS_MPP: u32 = 3 << 11;
const MIP_MTIP: u32 = 1 << 7;
const MIP_MEIP: u32 = 1 << 11;
/// misa: RV32, I and M.
pub const MISA: u32 = 0x4000_0000 | 1 << 8 | 1 << 12;

/// The SoC's peripherals as the instruction-level simulator sees them:
/// registers without timing (the UART sends and receives at once).
#[derive(Debug, Clone, Default)]
pub struct Io {
    pub gpio_out: u32,
    pub gpio_in: u32,
    pub display: u32,
    /// Bytes sent on the UART.
    pub uart_out: Vec<u8>,
    /// Bytes waiting to be received.
    pub uart_in: VecDeque<u8>,
    pub uart_ctrl: u32,
    pub uart_div: u32,
    pub mtime: u64,
    pub mtimecmp: u64,
}

impl Io {
    fn read(&mut self, offset: u32) -> u32 {
        match offset {
            map::GPIO_OUT => self.gpio_out,
            map::GPIO_IN => self.gpio_in,
            map::DISPLAY => self.display,
            map::UART_DATA => self.uart_in.pop_front().map_or(0, u32::from),
            map::UART_STATUS => (!self.uart_in.is_empty() as u32) << 1,
            map::UART_CTRL => self.uart_ctrl,
            map::UART_DIV => self.uart_div,
            map::MTIME => self.mtime as u32,
            map::MTIMEH => (self.mtime >> 32) as u32,
            map::MTIMECMP => self.mtimecmp as u32,
            map::MTIMECMPH => (self.mtimecmp >> 32) as u32,
            _ => 0,
        }
    }

    fn write(&mut self, offset: u32, v: u32) {
        let lo = |x: u64| x & !0xFFFF_FFFF | v as u64;
        let hi = |x: u64| x & 0xFFFF_FFFF | (v as u64) << 32;
        match offset {
            map::GPIO_OUT => self.gpio_out = v,
            map::DISPLAY => self.display = v,
            map::UART_DATA => self.uart_out.push(v as u8),
            map::UART_CTRL => self.uart_ctrl = v & map::UART_CTRL_MASK,
            map::UART_DIV => self.uart_div = v & 0xFFFF,
            map::MTIME => self.mtime = lo(self.mtime),
            map::MTIMEH => self.mtime = hi(self.mtime),
            map::MTIMECMP => self.mtimecmp = lo(self.mtimecmp),
            map::MTIMECMPH => self.mtimecmp = hi(self.mtimecmp),
            _ => {}
        }
    }

    /// Pending interrupt bits for `mip`.
    pub fn mip(&self) -> u32 {
        let timer = if self.mtime >= self.mtimecmp { MIP_MTIP } else { 0 };
        let rx = self.uart_ctrl & map::UART_CTRL_RX_IRQ != 0 && !self.uart_in.is_empty();
        timer | if rx { MIP_MEIP } else { 0 }
    }
}

#[derive(Debug, Clone)]
pub struct Rv32 {
    pub x: [u32; 32],
    pub pc: u32,
    /// RAM, `map::RAM_SIZE` bytes from address 0.
    pub ram: Vec<u8>,
    pub io: Io,
    pub halted: bool,
    pub mstatus: u32,
    pub mie: u32,
    pub mtvec: u32,
    pub mscratch: u32,
    pub mepc: u32,
    pub mcause: u32,
    pub mtval: u32,
    pub mcycle: u64,
    pub minstret: u64,
    /// Set by a CSR write to minstret(h): the write replaces the increment.
    instret_written: bool,
}

/// Result of an access: the value, or the exception cause.
type Access<T> = Result<T, u32>;

impl Rv32 {
    /// A machine with `program` (little-endian words) loaded at address 0.
    pub fn new(program: &[u32]) -> Rv32 {
        let mut ram = vec![0u8; map::RAM_SIZE];
        for (i, w) in program.iter().enumerate() {
            ram[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        Rv32 {
            x: [0; 32],
            pc: 0,
            ram,
            io: Io::default(),
            halted: false,
            mstatus: MSTATUS_MPP,
            mie: 0,
            mtvec: 0,
            mscratch: 0,
            mepc: 0,
            mcause: 0,
            mtval: 0,
            mcycle: 0,
            minstret: 0,
            instret_written: false,
        }
    }

    /// Word of RAM at a (word-aligned) byte address.
    pub fn ram_word(&self, addr: u32) -> u32 {
        let a = (addr as usize % map::RAM_SIZE) & !3;
        u32::from_le_bytes(self.ram[a..a + 4].try_into().unwrap())
    }

    fn load(&mut self, addr: u32, size: u32) -> Access<u32> {
        if addr % size != 0 {
            return Err(cause::MISALIGNED_LOAD);
        }
        if map::is_io(addr) {
            let word = self.io.read(addr & map::IO_MASK & !3);
            return Ok(word >> (8 * (addr & 3)));
        }
        let a = addr as usize % map::RAM_SIZE;
        Ok((0..size as usize).rev().fold(0, |v, i| v << 8 | self.ram[a + i] as u32))
    }

    fn store(&mut self, addr: u32, size: u32, value: u32) -> Access<()> {
        if addr % size != 0 {
            return Err(cause::MISALIGNED_STORE);
        }
        if map::is_io(addr) {
            // I/O registers take whole words: a narrower store writes its
            // bytes in their lanes and 0 in the others, as IO_RV32 does.
            let value = if size == 4 { value } else { value & ((1 << (8 * size)) - 1) };
            self.io.write(addr & map::IO_MASK & !3, value << (8 * (addr & 3)));
            return Ok(());
        }
        let a = addr as usize % map::RAM_SIZE;
        for i in 0..size as usize {
            self.ram[a + i] = (value >> (8 * i)) as u8;
        }
        Ok(())
    }

    fn read_csr(&self, n: u16) -> Option<u32> {
        Some(match n {
            csr::MSTATUS => self.mstatus,
            csr::MISA => MISA,
            csr::MIE => self.mie,
            csr::MTVEC => self.mtvec,
            csr::MSCRATCH => self.mscratch,
            csr::MEPC => self.mepc,
            csr::MCAUSE => self.mcause,
            csr::MTVAL => self.mtval,
            csr::MIP => self.io.mip(),
            csr::MCYCLE | csr::CYCLE => self.mcycle as u32,
            csr::MCYCLEH | csr::CYCLEH => (self.mcycle >> 32) as u32,
            csr::MINSTRET | csr::INSTRET => self.minstret as u32,
            csr::MINSTRETH | csr::INSTRETH => (self.minstret >> 32) as u32,
            csr::MHARTID => 0,
            _ => return None,
        })
    }

    /// Writes a CSR; false if it is read-only or does not exist.
    fn write_csr(&mut self, n: u16, v: u32) -> bool {
        let lo = |x: u64| x & !0xFFFF_FFFF | v as u64;
        let hi = |x: u64| x & 0xFFFF_FFFF | (v as u64) << 32;
        match n {
            csr::MSTATUS => self.mstatus = v & (MSTATUS_MIE | MSTATUS_MPIE) | MSTATUS_MPP,
            csr::MISA | csr::MIP => {}
            csr::MIE => self.mie = v & (MIP_MTIP | MIP_MEIP),
            csr::MTVEC => self.mtvec = v & !3,
            csr::MSCRATCH => self.mscratch = v,
            csr::MEPC => self.mepc = v & !3,
            csr::MCAUSE => self.mcause = v & 0x8000_000F,
            csr::MTVAL => self.mtval = v,
            csr::MCYCLE => self.mcycle = lo(self.mcycle),
            csr::MCYCLEH => self.mcycle = hi(self.mcycle),
            csr::MINSTRET => {
                self.minstret = lo(self.minstret);
                self.instret_written = true;
            }
            csr::MINSTRETH => {
                self.minstret = hi(self.minstret);
                self.instret_written = true;
            }
            _ => return false,
        }
        true
    }

    /// Enters the trap handler.
    fn trap(&mut self, cause: u32, epc: u32, tval: u32) {
        self.mepc = epc;
        self.mcause = cause;
        self.mtval = tval;
        let mie = self.mstatus & MSTATUS_MIE != 0;
        self.mstatus = MSTATUS_MPP | if mie { MSTATUS_MPIE } else { 0 };
        self.pc = self.mtvec;
    }

    /// The interrupt to take now, if any (timer before external).
    fn pending_interrupt(&self) -> Option<u32> {
        if self.mstatus & MSTATUS_MIE == 0 {
            return None;
        }
        let p = self.io.mip() & self.mie;
        if p & MIP_MEIP != 0 {
            Some(cause::EXTERNAL_INT)
        } else if p & MIP_MTIP != 0 {
            Some(cause::TIMER_INT)
        } else {
            None
        }
    }

    /// Executes one instruction (or takes an interrupt); false if halted.
    pub fn step(&mut self) -> bool {
        if self.halted {
            return false;
        }
        self.mcycle = self.mcycle.wrapping_add(1);
        self.io.mtime = self.io.mtime.wrapping_add(1);
        if let Some(c) = self.pending_interrupt() {
            self.trap(c, self.pc, 0);
            return true;
        }
        let pc = self.pc;
        let word = match self.load(pc, 4) {
            Ok(w) => w,
            Err(_) => {
                self.trap(cause::MISALIGNED_FETCH, pc, pc);
                return true;
            }
        };
        let Some(inst) = decode(word) else {
            self.trap(cause::ILLEGAL, pc, word);
            return true;
        };
        self.instret_written = false;
        match self.execute(inst, pc, word) {
            Ok(()) if self.instret_written => {}
            Ok(()) => self.minstret = self.minstret.wrapping_add(1),
            Err((cause, tval)) => self.trap(cause, pc, tval),
        }
        true
    }

    /// Runs until halted or `max` steps; returns the steps taken.
    pub fn run(&mut self, max: usize) -> usize {
        (0..max).take_while(|_| self.step()).count()
    }

    fn execute(&mut self, i: Inst, pc: u32, word: u32) -> Result<(), (u32, u32)> {
        let a = self.x[i.rs1 as usize];
        let b = self.x[i.rs2 as usize];
        let imm = i.imm as u32;
        let mut next = pc.wrapping_add(4);
        let mut rd: Option<u32> = None;
        let jump = |target: u32| if target % 4 != 0 { Err((cause::MISALIGNED_FETCH, target)) } else { Ok(target) };
        use Op::*;
        match i.op {
            Lui => rd = Some(imm),
            Auipc => rd = Some(pc.wrapping_add(imm)),
            Jal => {
                next = jump(pc.wrapping_add(imm))?;
                rd = Some(pc.wrapping_add(4));
            }
            Jalr => {
                next = jump(a.wrapping_add(imm) & !1)?;
                rd = Some(pc.wrapping_add(4));
            }
            Beq | Bne | Blt | Bge | Bltu | Bgeu => {
                let taken = match i.op {
                    Beq => a == b,
                    Bne => a != b,
                    Blt => (a as i32) < (b as i32),
                    Bge => (a as i32) >= (b as i32),
                    Bltu => a < b,
                    _ => a >= b,
                };
                if taken {
                    next = jump(pc.wrapping_add(imm))?;
                }
            }
            Lb | Lh | Lw | Lbu | Lhu => {
                let addr = a.wrapping_add(imm);
                let size = match i.op {
                    Lb | Lbu => 1,
                    Lh | Lhu => 2,
                    _ => 4,
                };
                let v = self.load(addr, size).map_err(|c| (c, addr))?;
                rd = Some(match i.op {
                    Lb => v as u8 as i8 as u32,
                    Lh => v as u16 as i16 as u32,
                    Lbu => v & 0xFF,
                    Lhu => v & 0xFFFF,
                    _ => v,
                });
            }
            Sb | Sh | Sw => {
                let addr = a.wrapping_add(imm);
                let size = match i.op {
                    Sb => 1,
                    Sh => 2,
                    _ => 4,
                };
                self.store(addr, size, b).map_err(|c| (c, addr))?;
            }
            Addi => rd = Some(a.wrapping_add(imm)),
            Slti => rd = Some(((a as i32) < i.imm) as u32),
            Sltiu => rd = Some((a < imm) as u32),
            Xori => rd = Some(a ^ imm),
            Ori => rd = Some(a | imm),
            Andi => rd = Some(a & imm),
            Slli => rd = Some(a << (imm & 31)),
            Srli => rd = Some(a >> (imm & 31)),
            Srai => rd = Some(((a as i32) >> (imm & 31)) as u32),
            Add => rd = Some(a.wrapping_add(b)),
            Sub => rd = Some(a.wrapping_sub(b)),
            Sll => rd = Some(a << (b & 31)),
            Slt => rd = Some(((a as i32) < (b as i32)) as u32),
            Sltu => rd = Some((a < b) as u32),
            Xor => rd = Some(a ^ b),
            Srl => rd = Some(a >> (b & 31)),
            Sra => rd = Some(((a as i32) >> (b & 31)) as u32),
            Or => rd = Some(a | b),
            And => rd = Some(a & b),
            Mul => rd = Some(a.wrapping_mul(b)),
            Mulh => rd = Some(((a as i32 as i64 * b as i32 as i64) >> 32) as u32),
            Mulhsu => rd = Some(((a as i32 as i64 * b as i64) >> 32) as u32),
            Mulhu => rd = Some(((a as u64 * b as u64) >> 32) as u32),
            Div => {
                rd = Some(if b == 0 {
                    u32::MAX
                } else {
                    (a as i32).wrapping_div(b as i32) as u32
                })
            }
            Divu => rd = Some(a.checked_div(b).unwrap_or(u32::MAX)),
            Rem => rd = Some(if b == 0 { a } else { (a as i32).wrapping_rem(b as i32) as u32 }),
            Remu => rd = Some(if b == 0 { a } else { a % b }),
            Fence | Wfi => {}
            Ecall => return Err((cause::ECALL_M, 0)),
            Ebreak => {
                self.halted = true;
                return Ok(());
            }
            Mret => {
                let mpie = self.mstatus & MSTATUS_MPIE != 0;
                self.mstatus = MSTATUS_MPP | MSTATUS_MPIE | if mpie { MSTATUS_MIE } else { 0 };
                next = self.mepc;
            }
            Csrrw | Csrrs | Csrrc | Csrrwi | Csrrsi | Csrrci => {
                let n = i.imm as u16;
                let src = if matches!(i.op, Csrrwi | Csrrsi | Csrrci) { i.rs1 as u32 } else { a };
                let illegal = Err((cause::ILLEGAL, word));
                let Some(old) = self.read_csr(n) else { return illegal };
                let (write, new) = match i.op {
                    Csrrw | Csrrwi => (true, src),
                    Csrrs | Csrrsi => (i.rs1 != 0, old | src),
                    _ => (i.rs1 != 0, old & !src),
                };
                // Read-only CSRs (0xC00.., 0xF00..) trap on a write.
                if write && (n >> 10 == 3 || !self.write_csr(n, new)) {
                    return illegal;
                }
                rd = Some(old);
            }
        }
        if let Some(v) = rd
            && i.rd != 0
        {
            self.x[i.rd as usize] = v;
        }
        self.pc = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::riscv::asm::assemble;

    fn run(src: &str) -> Rv32 {
        let p = assemble(src).unwrap_or_else(|e| panic!("{e}"));
        let mut m = Rv32::new(&p.words);
        m.run(100_000);
        assert!(m.halted, "no ebreak");
        m
    }

    #[test]
    fn arithmetic_and_branches() {
        let m = run("li a0, 10\n li a1, 0\nloop: add a1, a1, a0\n addi a0, a0, -1\n bnez a0, loop\n ebreak");
        assert_eq!(m.x[11], 55);
        let m = run("li a0, -7\n li a1, 2\n div a2, a0, a1\n rem a3, a0, a1\n divu a4, a0, zero\n \
                     mulh a5, a0, a0\n mul a6, a0, a0\n srai a7, a0, 1\n srli t0, a0, 28\n sltu t1, a1, a0\n ebreak");
        assert_eq!(m.x[12] as i32, -3);
        assert_eq!(m.x[13] as i32, -1);
        assert_eq!(m.x[14], u32::MAX);
        assert_eq!((m.x[15], m.x[16]), (0, 49));
        assert_eq!(m.x[17] as i32, -4);
        assert_eq!((m.x[5], m.x[6]), (0xF, 1));
        let m = run("li a0, 0x80000000\n li a1, -1\n div a2, a0, a1\n rem a3, a0, a1\n ebreak");
        assert_eq!((m.x[12], m.x[13]), (0x8000_0000, 0));
    }

    #[test]
    fn memory_and_io() {
        let m = run("li t0, 0x1000\n li t1, -2\n sb t1, 1(t0)\n lb a0, 1(t0)\n lbu a1, 1(t0)\n lw a2, 0(t0)\n \
                     sh t1, 2(t0)\n lhu a3, 2(t0)\n li t2, 0x10000000\n sw a3, 8(t2)\n li t3, 'A'\n sw t3, 12(t2)\n ebreak");
        assert_eq!((m.x[10] as i32, m.x[11], m.x[12], m.x[13]), (-2, 0xFE, 0xFE00, 0xFFFE));
        assert_eq!(m.io.display, 0xFFFE);
        assert_eq!(m.io.uart_out, b"A");
        assert_eq!(m.ram_word(0x1000), 0xFFFE_FE00);
    }

    #[test]
    fn traps_and_csrs() {
        let m = run("la t0, handler\n csrw mtvec, t0\n li a0, 1\n ecall\n addi a0, a0, 10\n \
                     lw a1, 2(zero)\n ebreak\n\
                     handler: csrr t1, mepc\n addi t1, t1, 4\n csrw mepc, t1\n csrr a2, mcause\n addi a0, a0, 100\n mret");
        assert_eq!(m.x[10], 211); // ecall + misaligned load: two traps
        assert_eq!(m.mcause, cause::MISALIGNED_LOAD);
        assert_eq!(m.mtval, 2);
        assert_eq!(m.x[12], cause::MISALIGNED_LOAD);
        let m = run("csrr a0, misa\n ebreak");
        assert_eq!(m.x[10], MISA);
        let m = run("la t0, h\n csrw mtvec, t0\n csrw cycle, zero\n ebreak\n h: csrr a0, mcause\n ebreak");
        assert_eq!(m.x[10], cause::ILLEGAL);
    }

    #[test]
    fn timer_interrupt() {
        let m = run("la t0, h\n csrw mtvec, t0\n li t1, 0x10000000\n li t2, 50\n sw t2, 0x28(t1)\n sw zero, 0x2C(t1)\n \
                     li t3, 128\n csrw mie, t3\n csrsi mstatus, 8\n loop: addi a0, a0, 1\n beqz a1, loop\n ebreak\n \
                     h: li a1, 1\n li t2, -1\n sw t2, 0x2C(t1)\n mret");
        assert_eq!(m.x[11], 1);
        assert!(m.x[10] > 10 && m.x[10] < 50, "{}", m.x[10]);
        assert_eq!(m.mcause, cause::TIMER_INT);
    }
}
