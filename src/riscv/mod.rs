//! The RISC-V SoC (`SOC_RV32`, circuits in `circuits/riscv/`), separate from
//! the 8- and 16-bit computers: instruction set ([`isa`]), assembler
//! ([`asm`]) and instruction-level simulator ([`sim`]).

pub mod asm;
pub mod cli;
pub mod isa;
pub mod machine;
pub mod sim;
#[cfg(test)]
mod cpu_tests;
#[cfg(test)]
mod tests;

/// The SoC's memory map.
pub mod map {
    /// RAM from address 0 (program and data); addresses wrap around it.
    pub const RAM_SIZE: usize = 1 << 20;
    /// I/O registers at 0x1000_0000: 16 words, repeated every 64 bytes up
    /// to 0x1FFF_FFFF (IO_RV32 decodes only address bits 31:28 and 5:2).
    pub const IO_BASE: u32 = 0x1000_0000;
    pub const IO_MASK: u32 = 0x3F;

    /// Offsets of the I/O registers (32 bits each).
    pub const GPIO_OUT: u32 = 0x00;
    pub const GPIO_IN: u32 = 0x04;
    /// Value of the 8-digit hex display.
    pub const DISPLAY: u32 = 0x08;
    /// Write: send a byte. Read: the received byte (and clear "received").
    pub const UART_DATA: u32 = 0x0C;
    /// Bit 0: sending, bit 1: a byte was received.
    pub const UART_STATUS: u32 = 0x10;
    /// Bit 0: interrupt (machine external) when a byte was received.
    pub const UART_CTRL: u32 = 0x14;
    pub const UART_CTRL_RX_IRQ: u32 = 1;
    pub const UART_CTRL_MASK: u32 = 1;
    /// Clocks per UART bit - 1 (16 bits; 0 after reset).
    pub const UART_DIV: u32 = 0x18;
    /// Machine timer (as a CLINT): 64-bit `mtime` counting clocks and
    /// `mtimecmp`; the timer interrupt is pending while mtime >= mtimecmp.
    pub const MTIME: u32 = 0x20;
    pub const MTIMEH: u32 = 0x24;
    pub const MTIMECMP: u32 = 0x28;
    pub const MTIMECMPH: u32 = 0x2C;

    pub fn is_io(addr: u32) -> bool {
        addr >> 28 == 1
    }
}
