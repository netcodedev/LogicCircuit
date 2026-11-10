//! Runs programs on `CPU_RV32` or `SOC_RV32`: [`Machine`] plays the
//! memory on the bus (`Addr`, `DataIn`, `DataOut`, `ByteEn`, `Write`,
//! `Fetch`, `Wait`), the clock, and on the SoC the UART line and the GPIO
//! inputs. One [`Machine::step`] is one clock.
//!
//! The memory is byte-addressed RAM of [`map::RAM_SIZE`] bytes from address
//! 0 that repeats over the whole address space (on the CPU alone the I/O
//! addresses are RAM too). A word access reads the aligned word at
//! `Addr AND NOT 3`; a store writes the lanes in `ByteEn` on the rising
//! clock edge when `Wait` is 0.

use std::cell::Cell;
use std::collections::VecDeque;

use crate::circuit::Circuit;

use super::map;

pub const ADDR: &str = "Addr";
pub const DATA_IN: &str = "DataIn";
pub const DATA_OUT: &str = "DataOut";
pub const BYTE_EN: &str = "ByteEn";
pub const WRITE: &str = "Write";
pub const FETCH: &str = "Fetch";
pub const WAIT: &str = "Wait";
pub const PC: &str = "PC";
pub const HALTED: &str = "Halted";
pub const CLK: &str = "CLK";
pub const RESET: &str = "Reset";
pub const TX: &str = "Tx";
pub const RX: &str = "Rx";
pub const GPIO_IN: &str = "GpioIn";
pub const GPIO_OUT: &str = "GpioOut";
/// The display register and the UART's clocks per bit inside SOC_RV32.
const DISPLAY: &str = "Io.Display";
const UART_DIV: &str = "Io.Uart.Div";
/// Clocks [`Machine::reset`] holds `Reset` (SOC_RV32 synchronizes it).
const RESET_CLOCKS: usize = 3;

/// Reads a bus (`name.0` = LSB) as a number; missing bits read 0.
pub fn read_bus(c: &Circuit, name: &str, width: usize) -> u64 {
    (0..width).fold(0, |acc, i| acc | (c.wire_value(&format!("{name}.{i}")).unwrap_or(false) as u64) << i)
}

/// Sets a top-level input bus without evaluating.
pub fn write_bus(c: &Circuit, name: &str, width: usize, value: u64) {
    for i in 0..width {
        c.set_input(&format!("{name}.{i}"), value >> i & 1 == 1);
    }
}

fn read_bit(c: &Circuit, name: &str) -> bool {
    c.wire_value(name).unwrap_or(false)
}

/// True for circuits with the bus of CPU_RV32 (32-bit `Addr` and `DataIn`,
/// `ByteEn[4]`, `PC`, `CLK`).
pub fn is_rv32(c: &Circuit) -> bool {
    c.has_wire("Addr.31") && c.has_wire("DataIn.31") && c.has_wire("ByteEn.3") && c.has_wire("PC.31") && c.has_wire(CLK)
}

/// Receiver for the UART line `Tx` (8N1, LSB first), sampled once per clock
/// in the middle of each bit.
#[derive(Debug, Clone, Default)]
struct UartRx {
    clocks: Option<usize>,
    period: usize,
    frame: u16,
}

/// Wait states: an access (a new request) waits `latency` clocks.
#[derive(Debug, Clone, Default)]
struct WaitState {
    request: Cell<Option<u64>>,
    left: Cell<usize>,
}

impl WaitState {
    fn waiting(&self, request: u64, latency: usize) -> bool {
        if Some(request) != self.request.get() {
            self.request.set(Some(request));
            self.left.set(latency);
        }
        self.left.get() > 0
    }

    /// Clock edge: one wait state less; a finished access other than a
    /// fetch is used up (the next one waits again, even at the same address).
    fn clock(&self, fetch: bool) {
        if self.left.get() > 0 {
            self.left.set(self.left.get() - 1);
        } else if !fetch {
            self.request.set(None);
        }
    }
}

#[derive(Debug, Clone)]
pub struct Machine {
    /// RAM, [`map::RAM_SIZE`] bytes from address 0.
    pub ram: Vec<u8>,
    /// Wait states per memory access (0 = answers at once).
    pub latency: usize,
    wait: WaitState,
    /// GPIO input pins (SoC); None leaves them to the caller (the viewer's
    /// switches).
    pub gpio_in: Option<u32>,
    /// Bytes received on `Tx`.
    pub uart: Vec<u8>,
    uart_rx: UartRx,
    /// Bytes still to send on `Rx`, after `uart_in_delay` idle clocks.
    pub uart_in: VecDeque<u8>,
    pub uart_in_delay: usize,
    /// Frame being sent on `Rx`: bits, clocks since it began, clocks per bit.
    uart_tx: Option<(u16, usize, usize)>,
    /// "Cpu." on the SoC, "" on the CPU alone.
    cpu: &'static str,
    soc: bool,
    /// Clocks run since the reset.
    pub clocks: u64,
}

impl Machine {
    /// A machine with `program` (little-endian words) at address 0; None if
    /// the circuit does not have CPU_RV32's bus.
    pub fn new(c: &Circuit, program: &[u32]) -> Option<Machine> {
        if !is_rv32(c) {
            return None;
        }
        let mut ram = vec![0u8; map::RAM_SIZE];
        for (i, w) in program.iter().enumerate() {
            ram[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        let soc = c.has_wire("Cpu.PC.0");
        if c.has_wire(RX) {
            c.set_input(RX, true);
        }
        Some(Machine {
            ram,
            latency: 0,
            wait: WaitState::default(),
            gpio_in: None,
            uart: Vec::new(),
            uart_rx: UartRx::default(),
            uart_in: VecDeque::new(),
            uart_in_delay: 100,
            uart_tx: None,
            cpu: if soc { "Cpu." } else { "" },
            soc,
            clocks: 0,
        })
    }

    pub fn word(&self, addr: u32) -> u32 {
        let a = (addr as usize % map::RAM_SIZE) & !3;
        u32::from_le_bytes(self.ram[a..a + 4].try_into().unwrap())
    }

    pub fn pc(&self, c: &Circuit) -> u32 {
        read_bus(c, PC, 32) as u32
    }

    pub fn halted(&self, c: &Circuit) -> bool {
        read_bit(c, HALTED)
    }

    /// Register `x[i]` of the register file.
    pub fn reg(&self, c: &Circuit, i: usize) -> u32 {
        if i == 0 { 0 } else { read_bus(c, &format!("{}RegFile.R_{i}.Q", self.cpu), 32) as u32 }
    }

    pub fn regs(&self, c: &Circuit) -> [u32; 32] {
        std::array::from_fn(|i| self.reg(c, i))
    }

    /// SoC: the display register and GPIO outputs.
    pub fn display(&self, c: &Circuit) -> u32 {
        read_bus(c, DISPLAY, 32) as u32
    }

    pub fn gpio_out(&self, c: &Circuit) -> u32 {
        read_bus(c, GPIO_OUT, 32) as u32
    }

    /// SoC: the UART's clocks per bit - 1.
    pub fn uart_div(&self, c: &Circuit) -> u32 {
        read_bus(c, UART_DIV, 16) as u32
    }

    /// A CSR's value as CSR_RV32 reads it (by its decoded address).
    pub fn csr_reg(&self, c: &Circuit, part: &str, width: usize) -> u64 {
        read_bus(c, &format!("{}Csr.{part}.Q", self.cpu), width)
    }

    fn rx_level(&self) -> bool {
        match self.uart_tx {
            Some((frame, clocks, period)) if clocks < 10 * period => frame >> (clocks / period) & 1 == 1,
            _ => true,
        }
    }

    /// Presents the memory word at `Addr`, the wait state and the inputs,
    /// and settles the circuit.
    pub fn serve(&self, c: &Circuit) {
        if self.soc {
            c.set_input(RX, self.rx_level());
            if let Some(v) = self.gpio_in {
                write_bus(c, GPIO_IN, 32, v as u64);
            }
        }
        c.evaluate_stable();
        // Addr comes from registers, so one round settles; the loop guards.
        for _ in 0..4 {
            let addr = read_bus(c, ADDR, 32) as u32;
            write_bus(c, DATA_IN, 32, self.word(addr) as u64);
            let write = read_bit(c, WRITE);
            let fetch = read_bit(c, FETCH);
            let access = write || fetch || read_bit(c, "Read");
            let request = addr as u64 | (write as u64) << 32 | (fetch as u64) << 33;
            c.set_input(WAIT, access && self.wait.waiting(request, self.latency));
            c.evaluate_stable();
            if read_bus(c, ADDR, 32) as u32 == addr {
                break;
            }
        }
    }

    /// One clock: store on the rising edge, then serve the next access.
    /// Returns false (and does nothing) once the CPU is halted.
    pub fn step(&mut self, c: &Circuit) -> bool {
        self.serve(c);
        if self.halted(c) {
            return false;
        }
        self.clock(c);
        true
    }

    /// One clock even when halted (the UART may still be sending).
    pub fn clock(&mut self, c: &Circuit) {
        self.serve(c);
        let store = (read_bit(c, WRITE) && !read_bit(c, WAIT))
            .then(|| (read_bus(c, ADDR, 32) as u32, read_bus(c, DATA_OUT, 32) as u32, read_bus(c, BYTE_EN, 4)));
        let fetch = read_bit(c, FETCH);
        c.set_input(CLK, true);
        c.evaluate_stable();
        if let Some((addr, data, en)) = store {
            let a = (addr as usize % map::RAM_SIZE) & !3;
            for b in 0..4 {
                if en >> b & 1 == 1 {
                    self.ram[a + b] = (data >> (8 * b)) as u8;
                }
            }
        }
        c.set_input(CLK, false);
        c.evaluate_stable();
        self.wait.clock(fetch);
        self.clocks += 1;
        if self.soc {
            self.sample_uart(c);
            self.clock_uart_in(c);
        }
    }

    /// Power-on reset: holds `Reset` for a few clocks, then releases it.
    pub fn reset(&mut self, c: &Circuit) {
        c.set_input(RESET, true);
        for _ in 0..RESET_CLOCKS {
            self.serve(c);
            for clk in [true, false] {
                c.set_input(CLK, clk);
                c.evaluate_stable();
            }
        }
        c.set_input(RESET, false);
        if self.soc {
            // The synchronizer releases the CPU two clocks later.
            for _ in 0..2 {
                self.serve(c);
                for clk in [true, false] {
                    c.set_input(CLK, clk);
                    c.evaluate_stable();
                }
            }
        }
        self.clocks = 0;
        self.serve(c);
    }

    /// Runs until halted or `max` clocks; returns the clocks run.
    pub fn run(&mut self, c: &Circuit, max: u64) -> u64 {
        let start = self.clocks;
        while self.clocks - start < max && self.step(c) {}
        self.clocks - start
    }

    fn clock_uart_in(&mut self, c: &Circuit) {
        match &mut self.uart_tx {
            Some((_, clocks, period)) if *clocks + 1 < 11 * *period => *clocks += 1,
            None if self.uart_in_delay > 0 => self.uart_in_delay -= 1,
            _ => {
                self.uart_tx = self.uart_in.pop_front().map(|byte| {
                    let period = read_bus(c, UART_DIV, 16) as usize + 1;
                    (1 << 9 | (byte as u16) << 1, 0, period)
                });
            }
        }
    }

    fn sample_uart(&mut self, c: &Circuit) {
        let tx = read_bit(c, TX);
        let rx = &mut self.uart_rx;
        let clocks = match rx.clocks {
            None if tx => return,
            None => {
                rx.period = read_bus(c, UART_DIV, 16) as usize + 1;
                rx.frame = 0;
                0
            }
            Some(n) => n + 1,
        };
        rx.clocks = Some(clocks);
        let half = rx.period / 2;
        if clocks < half || (clocks - half) % rx.period != 0 {
            return;
        }
        match (clocks - half) / rx.period {
            k @ 1..=8 => rx.frame |= (tx as u16) << (k - 1),
            9 => {
                self.uart.push(rx.frame as u8);
                rx.clocks = None;
            }
            _ => {}
        }
    }
}
