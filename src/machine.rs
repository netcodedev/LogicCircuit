//! Runs programs on a circuit that exposes a CPU memory interface.
//!
//! The circuit drives `PC` and reads `Instruction[16]`; [`Machine`] plays
//! the instruction memory (a ROM held in Rust) and the clock. Optional ports
//! are served when the circuit has them:
//! - `InstructionNext[16]`: the word at PC+1 (CPU_16B's two-word
//!   instructions); its presence selects [`Isa::Cpu16`]
//! - `MemAddr`/`MemDataIn`/`MemDataOut`/`MemWrite`: a data RAM that answers
//!   combinationally and stores on the rising clock edge
//! - `In` and `GpioIn` (SOC_16B): input ports, set from [`Machine::input`] if
//!   that is `Some`
//! - `IWait`/`DWait` (with `MemRead`): wait states of slow memories, see
//!   [`Machine::latency`]
//!
//! Instead of `Instruction` a circuit may have one memory bus for
//! instructions and data (`Addr`, `DataIn`, `DataOut`, `Write`, `Fetch`,
//! `Wait`, see CPU_16B_SOC); the machine then plays that one memory.
//!
//! The port widths come from the circuit, so CPU_8B, COMPUTER_8B, CPU_16B,
//! COMPUTER_16B, the pipelined COMPUTER_16B_PIPE(3) and COMPUTER_16B_SOC all
//! work. On the pipelines one step is one clock, not one instruction.

use std::cell::Cell;
use std::collections::VecDeque;

use crate::circuit::Circuit;
use crate::isa::Isa;

pub const PC: &str = "PC";
pub const INSTRUCTION: &str = "Instruction";
pub const INSTRUCTION_NEXT: &str = "InstructionNext";
pub const MEM_ADDR: &str = "MemAddr";
pub const MEM_DATA_IN: &str = "MemDataIn";
pub const MEM_DATA_OUT: &str = "MemDataOut";
pub const MEM_WRITE: &str = "MemWrite";
pub const MEM_READ: &str = "MemRead";
pub const I_WAIT: &str = "IWait";
pub const D_WAIT: &str = "DWait";
pub const ADDR: &str = "Addr";
pub const DATA_IN: &str = "DataIn";
pub const DATA_OUT: &str = "DataOut";
pub const WRITE: &str = "Write";
pub const FETCH: &str = "Fetch";
pub const WAIT: &str = "Wait";
/// UART transmit line of SOC_16B.
pub const TX: &str = "Tx";
/// UART receive line of SOC_16B (idles at 1).
pub const RX: &str = "Rx";
/// Byte enables of a memory-bus write (11 word, 01 low byte, 10 high byte).
pub const BYTE_EN: &str = "ByteEn";
/// Baud divider of the UART inside SOC_16B (clocks per bit - 1). A real
/// receiver is told the baud rate; the simulated one reads it here.
const UART_DIV: &str = "Io.Uart.Div";
pub const INPUT: &str = "In";
pub const GPIO_IN: &str = "GpioIn";
pub const OUT: &str = "Out";
pub const CLK: &str = "CLK";
pub const RESET: &str = "Reset";
/// Clocks [`Machine::reset`] holds `Reset`: SOC_16B synchronizes it (two
/// clocks late) and resets on the next edge.
const RESET_CLOCKS: usize = 3;
pub const HALTED: &str = "Halted";

const INSTRUCTION_WIDTH: usize = 16;

/// Reads an output or internal bus (`name.0` = LSB) as a number.
/// Missing bits read as 0.
pub fn read_bus(circuit: &Circuit, name: &str, width: usize) -> u32 {
    let map = circuit.get_wire_map();
    (0..width).fold(0, |acc, i| {
        let bit = map
            .get(&format!("{name}.{i}"))
            .is_some_and(|w| w.borrow().value);
        acc | (bit as u32) << i
    })
}

/// Sets a top-level input bus without evaluating.
pub fn write_bus(circuit: &Circuit, name: &str, width: usize, value: u32) {
    for i in 0..width {
        circuit.set_input(&format!("{name}.{i}"), (value >> i) & 1 == 1);
    }
}

/// Width of bus `name` (number of consecutive `name.i` wires), 0 if absent.
pub fn bus_width(circuit: &Circuit, name: &str) -> usize {
    let map = circuit.get_wire_map();
    (0..).take_while(|i| map.contains_key(&format!("{name}.{i}"))).count()
}

/// True if the circuit has the ports needed to run programs on it: PC, CLK
/// and either an instruction port or a memory bus (`Addr`/`DataIn`).
pub fn is_runnable(circuit: &Circuit) -> bool {
    let map = circuit.get_wire_map();
    map.contains_key(&format!("{PC}.0"))
        && (map.contains_key(&format!("{INSTRUCTION}.0"))
            || map.contains_key(&format!("{ADDR}.0")) && map.contains_key(&format!("{DATA_IN}.0")))
        && map.contains_key(CLK)
}

/// Value of a single-bit wire; false if it does not exist.
pub fn read_bit(circuit: &Circuit, name: &str) -> bool {
    circuit.get_wire_map().get(name).is_some_and(|w| w.borrow().value)
}

pub fn halted(circuit: &Circuit) -> bool {
    read_bit(circuit, HALTED)
}

/// Port widths of a runnable circuit (0 = port absent).
#[derive(Debug, Clone, Copy)]
struct Ports {
    pc: usize,
    next: usize,
    mem_addr: usize,
    mem_data: usize,
    input: usize,
    gpio_in: usize,
    /// `IWait`/`DWait` inputs present.
    i_wait: bool,
    d_wait: bool,
    /// Width of `Addr` of a memory bus (CPU_16B_SOC), 0 = separate ports.
    bus: usize,
    /// `Tx` output (UART) present.
    tx: bool,
    /// `Rx` input (UART) present.
    rx: bool,
    /// Width of `ByteEn` (2 on CPU_16B_SOC, 0 = word writes only).
    byte_en: usize,
}

/// Sender for the `Rx` line: frames of [`Machine::uart_in`] (8N1) at the
/// bit time the circuit's UART is set to, one idle bit time between bytes.
#[derive(Debug, Clone, Default)]
struct UartTx {
    /// Frame (start bit in bit 0 .. stop bit in bit 9), clocks since it
    /// began, clocks per bit.
    frame: Option<(u16, usize, usize)>,
}

/// Receiver for the UART line `Tx` (8N1, LSB first), sampled once per
/// clock in the middle of each bit.
#[derive(Debug, Clone, Default)]
struct UartRx {
    /// Clocks since the start bit began; None while the line is idle.
    clocks: Option<usize>,
    /// Clocks per bit of the current frame.
    period: usize,
    frame: u16,
}

/// Wait states of one memory port. An access starts when the circuit
/// presents a new request; the port then answers "wait" for `latency`
/// clocks and delivers afterwards. Between clock edges [`Self::waiting`]
/// may be asked any number of times.
#[derive(Debug, Clone, Default)]
struct WaitState {
    request: Cell<Option<usize>>,
    left: Cell<usize>,
}

impl WaitState {
    /// True while the access to `request` (None = no access) is not done.
    fn waiting(&self, request: Option<usize>, latency: usize) -> bool {
        if request != self.request.get() {
            self.request.set(request);
            self.left.set(if request.is_some() { latency } else { 0 });
        }
        self.left.get() > 0
    }

    /// Clock edge. A finished access is used up (`consume`, data memory:
    /// the memory stage always moves on) or kept as long as the request
    /// stays the same (instruction memory: the fetch may be held, e.g.
    /// by a stall, and the word stays valid).
    fn clock(&self, consume: bool) {
        if self.left.get() > 0 {
            self.left.set(self.left.get() - 1);
        } else if consume {
            self.request.set(None);
        }
    }
}

/// Memories, input port and clock around one runnable circuit.
///
/// With a memory bus (`Addr`, `DataIn`, `DataOut`, `Write`, `Wait`, `Fetch`)
/// there is one memory for instructions and data: [`Self::rom`] is its
/// initial content from address 0, words written by the circuit go to
/// [`Self::ram`] and read back from there.
#[derive(Debug, Clone)]
pub struct Machine {
    ports: Ports,
    /// Instruction memory; words past the end read as 0 (HALT).
    pub rom: Vec<u16>,
    /// Data memory (empty if the circuit has no memory ports).
    pub ram: Vec<u16>,
    /// Value presented on the `In` port; `None` leaves it to the caller.
    pub input: Option<u16>,
    /// Wait states per memory access, for circuits with `IWait`/`DWait` or
    /// `Wait` ports; 0 = memories answer at once.
    pub latency: usize,
    fetch_wait: WaitState,
    data_wait: WaitState,
    /// Bus only: words of `ram` the circuit has written.
    written: Vec<bool>,
    /// Bytes received on the UART line `Tx`, if the circuit has one.
    pub uart: Vec<u8>,
    uart_rx: UartRx,
    /// Bytes still to send to the circuit on its `Rx` line.
    pub uart_in: VecDeque<u8>,
    /// Clocks the line stays idle before the first byte (time for the
    /// program to set up its UART, as a terminal would only type later).
    pub uart_in_delay: usize,
    uart_tx: UartTx,
}

impl Machine {
    /// `None` if the circuit lacks PC/Instruction/CLK.
    pub fn new(circuit: &Circuit) -> Option<Self> {
        if !is_runnable(circuit) {
            return None;
        }
        let ports = Ports {
            pc: bus_width(circuit, PC),
            next: bus_width(circuit, INSTRUCTION_NEXT),
            mem_addr: bus_width(circuit, MEM_ADDR),
            mem_data: bus_width(circuit, MEM_DATA_IN),
            input: bus_width(circuit, INPUT),
            gpio_in: bus_width(circuit, GPIO_IN),
            i_wait: circuit.get_wire_map().contains_key(I_WAIT),
            d_wait: circuit.get_wire_map().contains_key(D_WAIT),
            bus: bus_width(circuit, ADDR),
            tx: circuit.get_wire_map().contains_key(TX),
            rx: circuit.get_wire_map().contains_key(RX),
            byte_en: bus_width(circuit, BYTE_EN),
        };
        if ports.rx {
            circuit.set_input(RX, true); // idle line
        }
        let ram_words = match ports {
            Ports { bus: b @ 1.., .. } => 1 << b,
            Ports { mem_data: 1.., mem_addr, .. } => 1 << mem_addr,
            _ => 0,
        };
        Some(Self {
            ports,
            rom: Vec::new(),
            ram: vec![0; ram_words],
            input: None,
            latency: 0,
            fetch_wait: WaitState::default(),
            data_wait: WaitState::default(),
            written: vec![false; if ports.bus > 0 { ram_words } else { 0 }],
            uart: Vec::new(),
            uart_rx: UartRx::default(),
            uart_in: VecDeque::new(),
            uart_in_delay: 100,
            uart_tx: UartTx::default(),
        })
    }

    /// Level of the `Rx` line during the current clock.
    fn rx_level(&self) -> bool {
        match self.uart_tx.frame {
            Some((frame, clocks, period)) if clocks < 10 * period => (frame >> (clocks / period)) & 1 == 1,
            _ => true,
        }
    }

    /// Clock edge for the `Rx` sender: next clock of the frame (plus one
    /// idle bit time after the stop bit), then the next byte.
    fn clock_uart_in(&mut self, circuit: &Circuit) {
        match &mut self.uart_tx.frame {
            Some((_, clocks, period)) if *clocks + 1 < 11 * *period => *clocks += 1,
            None if self.uart_in_delay > 0 => self.uart_in_delay -= 1,
            _ => {
                self.uart_tx.frame = self.uart_in.pop_front().map(|byte| {
                    let period = read_bus(circuit, UART_DIV, 16) as usize + 1;
                    (1 << 9 | (byte as u16) << 1, 0, period)
                });
            }
        }
    }

    /// Samples the UART line after a clock edge (the line only changes on
    /// edges). A falling edge starts a frame; bit k (0 = start, 1..8 data,
    /// 9 = stop) is read `period / 2` clocks into its time slot.
    fn sample_uart(&mut self, circuit: &Circuit) {
        let tx = read_bit(circuit, TX);
        let rx = &mut self.uart_rx;
        let clocks = match rx.clocks {
            None if tx => return,
            None => {
                rx.period = read_bus(circuit, UART_DIV, 16) as usize + 1;
                rx.frame = 0;
                0
            }
            Some(c) => c + 1,
        };
        rx.clocks = Some(clocks);
        let half = rx.period / 2;
        if clocks < half || (clocks - half) % rx.period != 0 {
            return;
        }
        match (clocks - half) / rx.period {
            k @ 1..=8 => rx.frame |= (tx as u16) << (k - 1),
            9 => {
                // A missing stop bit (framing error) still delivers the byte.
                self.uart.push(rx.frame as u8);
                rx.clocks = None;
            }
            _ => {}
        }
    }

    /// The instruction set of the CPU behind the ports.
    pub fn isa(&self) -> Isa {
        if self.ports.next > 0 || self.ports.bus > 0 { Isa::Cpu16 } else { Isa::Cpu8 }
    }

    /// Width of the PC in bits.
    pub fn pc_width(&self) -> usize {
        self.ports.pc
    }

    pub fn pc(&self, circuit: &Circuit) -> usize {
        read_bus(circuit, PC, self.ports.pc) as usize
    }

    /// Instruction memory at `addr` (wrapping at the PC width); with a
    /// memory bus the shared memory, including words written since.
    pub fn word_at(&self, addr: usize) -> u16 {
        let addr = addr & ((1 << self.ports.pc) - 1);
        if self.written.get(addr) == Some(&true) {
            return self.ram[addr];
        }
        self.rom.get(addr).copied().unwrap_or(0)
    }

    /// Data memory at the address the circuit drives right now.
    fn mem_addr(&self, circuit: &Circuit) -> usize {
        read_bus(circuit, MEM_ADDR, self.ports.mem_addr) as usize
    }

    /// Clears the data memory.
    pub fn clear_ram(&mut self) {
        self.ram.fill(0);
        self.written.fill(false);
    }

    /// Presents the word(s) at the current PC, the input port and the data
    /// memory word at `MemAddr`, and settles the circuit, so all
    /// combinational signals reflect the instruction about to execute.
    pub fn fetch(&self, circuit: &Circuit) {
        if self.ports.bus > 0 {
            return self.serve_bus(circuit);
        }
        let pc = self.pc(circuit);
        write_bus(circuit, INSTRUCTION, INSTRUCTION_WIDTH, self.word_at(pc) as u32);
        write_bus(circuit, INSTRUCTION_NEXT, self.ports.next, self.word_at(pc + 1) as u32);
        if self.ports.i_wait {
            circuit.set_input(I_WAIT, self.fetch_wait.waiting(Some(pc), self.latency));
        }
        if let Some(input) = self.input {
            write_bus(circuit, INPUT, self.ports.input, input as u32);
            write_bus(circuit, GPIO_IN, self.ports.gpio_in, input as u32);
        }
        circuit.evaluate_stable();
        if self.ram.is_empty() {
            return;
        }
        // The address does not depend on the data read (CPU_16B), so this
        // settles after one round; the loop only guards other designs.
        for _ in 0..4 {
            let addr = self.mem_addr(circuit);
            write_bus(circuit, MEM_DATA_IN, self.ports.mem_data, self.ram[addr] as u32);
            if self.ports.d_wait {
                circuit.set_input(D_WAIT, self.data_wait.waiting(self.data_request(circuit), self.latency));
            }
            circuit.evaluate_stable();
            if self.mem_addr(circuit) == addr {
                break;
            }
        }
    }

    /// Memory bus: presents the word at `Addr` (instruction or data) and the
    /// wait state of the current access, and settles the circuit.
    fn serve_bus(&self, circuit: &Circuit) {
        if self.ports.rx {
            circuit.set_input(RX, self.rx_level());
        }
        if let Some(input) = self.input {
            write_bus(circuit, INPUT, self.ports.input, input as u32);
            write_bus(circuit, GPIO_IN, self.ports.gpio_in, input as u32);
        }
        circuit.evaluate_stable();
        // Addr comes from registers (PC or the memory stage), so this settles
        // after one round; the loop only guards other designs.
        for _ in 0..4 {
            let addr = read_bus(circuit, ADDR, self.ports.bus) as usize;
            write_bus(circuit, DATA_IN, 16, self.word_at(addr) as u32);
            let request = addr | (read_bit(circuit, WRITE) as usize) << 31 | (read_bit(circuit, FETCH) as usize) << 30;
            circuit.set_input(WAIT, self.data_wait.waiting(Some(request), self.latency));
            circuit.evaluate_stable();
            if read_bus(circuit, ADDR, self.ports.bus) as usize == addr {
                break;
            }
        }
    }

    /// The data access the circuit requests right now (address, plus a bit
    /// for writes), None if it neither reads nor writes.
    fn data_request(&self, circuit: &Circuit) -> Option<usize> {
        let write = read_bit(circuit, MEM_WRITE);
        (write || read_bit(circuit, MEM_READ)).then(|| self.mem_addr(circuit) | (write as usize) << 31)
    }

    /// True while the circuit waits for the data memory.
    fn data_waiting(&self, circuit: &Circuit) -> bool {
        self.ports.d_wait && read_bit(circuit, D_WAIT)
    }

    /// Executes one instruction (one clock on the pipelines): fetch, one full
    /// clock cycle (storing to memory on the rising edge if `MemWrite` is set
    /// and the memory is not waiting), fetch the next.
    /// Returns `false` (and does nothing) if the CPU is halted.
    pub fn step(&mut self, circuit: &Circuit) -> bool {
        self.fetch(circuit);
        if halted(circuit) {
            return false;
        }
        let store = if self.ports.bus > 0 {
            (read_bit(circuit, WRITE) && !read_bit(circuit, WAIT)).then(|| {
                let addr = read_bus(circuit, ADDR, self.ports.bus) as usize;
                let data = read_bus(circuit, DATA_OUT, 16) as u16;
                // SB writes one byte (ByteEn 01 or 10), the rest a word.
                let mask = match (self.ports.byte_en, read_bus(circuit, BYTE_EN, 2)) {
                    (0, _) | (_, 3) => 0xFFFF,
                    (_, be) => if be == 1 { 0x00FF } else { 0xFF00 },
                };
                (addr, self.word_at(addr) & !mask | data & mask)
            })
        } else {
            (!self.ram.is_empty() && read_bit(circuit, MEM_WRITE) && !self.data_waiting(circuit))
                .then(|| (self.mem_addr(circuit), read_bus(circuit, MEM_DATA_OUT, self.ports.mem_data) as u16))
        };
        // A finished fetch stays valid while the PC stays; any other bus
        // access is used up by the clock edge.
        let fetching = self.ports.bus > 0 && read_bit(circuit, FETCH);
        circuit.set_input(CLK, true);
        circuit.evaluate_stable();
        if let Some((addr, data)) = store {
            self.ram[addr] = data;
            if let Some(w) = self.written.get_mut(addr) {
                *w = true;
            }
        }
        circuit.set_input(CLK, false);
        circuit.evaluate_stable();
        self.fetch_wait.clock(false);
        self.data_wait.clock(!fetching);
        if self.ports.tx {
            self.sample_uart(circuit);
        }
        if self.ports.rx {
            self.clock_uart_in(circuit);
        }
        self.fetch(circuit);
        true
    }

    /// Power-on reset, as a reset circuit on a board does it: holds `Reset`
    /// for a few clocks with the inputs applied, then releases it. Nothing
    /// is written to memory meanwhile. Does nothing without a `Reset` input.
    pub fn reset(&mut self, circuit: &Circuit) {
        if !circuit.get_wire_map().contains_key(RESET) {
            return;
        }
        circuit.set_input(RESET, true);
        for _ in 0..RESET_CLOCKS {
            self.fetch(circuit);
            for clk in [true, false] {
                circuit.set_input(CLK, clk);
                circuit.evaluate_stable();
            }
        }
        circuit.set_input(RESET, false);
        self.fetch(circuit);
    }

    /// Runs until HALT or `max_steps`; returns the number of executed steps.
    pub fn run(&mut self, circuit: &Circuit, max_steps: usize) -> usize {
        let mut steps = 0;
        while steps < max_steps && self.step(circuit) {
            steps += 1;
        }
        steps
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cldl::CircuitBuilder;
    use std::path::Path;

    fn build(name: &str) -> (Circuit, Machine) {
        let mut builder = CircuitBuilder::new();
        builder
            .load_circuits_from_path(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/circuits")))
            .unwrap();
        let circuit = builder.build(name);
        let machine = Machine::new(&circuit).unwrap();
        (circuit, machine)
    }

    #[test]
    fn runs_example_programs_to_halt() {
        for (src, out) in [
            (include_str!("../programs/countdown.asm"), 0),
            (include_str!("../programs/fibonacci.asm"), 233),
            (include_str!("../programs/hex.asm"), 8),
        ] {
            let (circuit, mut machine) = build("COMPUTER_8B");
            assert_eq!(machine.isa(), Isa::Cpu8);
            machine.rom = machine.isa().assemble(src).unwrap().words;
            machine.run(&circuit, 1000);
            assert!(halted(&circuit));
            assert_eq!(read_bus(&circuit, OUT, 8), out);
        }
    }

    #[test]
    fn empty_memory_halts_immediately() {
        let (circuit, mut machine) = build("COMPUTER_8B");
        assert_eq!(machine.run(&circuit, 10), 0);
        assert_eq!(machine.pc(&circuit), 0);
    }

    /// The 16-bit computers: single cycle, two- and three-stage pipeline,
    /// three stages with one shared memory, and that with peripherals.
    const COMPUTERS_16: [&str; 5] =
        ["COMPUTER_16B", "COMPUTER_16B_PIPE", "COMPUTER_16B_PIPE3", "COMPUTER_16B_SOC", "SOC_16B"];

    /// Runs a 16-bit program on `top` to HALT and returns the circuit and
    /// machine.
    fn run16(top: &str, src: &str, input: u16) -> (Circuit, Machine) {
        let (circuit, machine, _) = run16_slow(top, src, input, 0);
        (circuit, machine)
    }

    /// Like [`run16`] with `latency` wait states per memory access; also
    /// returns the number of clocks.
    fn run16_slow(top: &str, src: &str, input: u16, latency: usize) -> (Circuit, Machine, usize) {
        let (circuit, mut machine) = build(top);
        assert_eq!(machine.isa(), Isa::Cpu16);
        assert_eq!(machine.ram.len(), 1 << 16);
        machine.rom = machine.isa().assemble(src).unwrap().words;
        machine.input = Some(input);
        machine.latency = latency;
        machine.reset(&circuit);
        let clocks = machine.run(&circuit, 1_000_000);
        assert!(halted(&circuit), "no HALT");
        (circuit, machine, clocks)
    }

    #[test]
    fn runs_16bit_example_programs_to_halt() {
        let programs = [
            (include_str!("../programs/16/fibonacci.asm"), 0, 46368),
            (include_str!("../programs/16/multiply.asm"), 0, 1234 * 37),
            (include_str!("../programs/16/factorial.asm"), 8, 40320),
            (include_str!("../programs/16/sort.asm"), 0, 0x1234),
        ];
        for top in COMPUTERS_16 {
            for (src, input, out) in programs {
                let (circuit, machine) = run16(top, src, input);
                assert_eq!(read_bus(&circuit, OUT, 16), out, "{top}");
                if src.contains("Bubble sort") {
                    assert_eq!(machine.ram[0x100..0x108], [0, 1, 7, 42, 503, 999, 0x0FFF, 0x1234]);
                }
            }
        }
    }

    #[test]
    fn data_memory_and_stack() {
        let src = "LI R1, 0x1000\n MOVI R2, 7\n SW R2, [R1+3]\n LW R3, [R1+3]\n\
                   PUSH R3\n LI R4, 0xBEEF\n PUSH R4\n POP R5\n POP R6\n\
                   IN R7\n ADD R7, R6\n OUT R7\n HALT";
        for top in COMPUTERS_16 {
            let (circuit, machine) = run16(top, src, 100);
            assert_eq!(machine.ram[0x1003], 7);
            // The stack grows down from 0xFFFF; popped words stay in memory.
            assert_eq!(machine.ram[0xFFFF], 7);
            assert_eq!(machine.ram[0xFFFE], 0xBEEF);
            assert_eq!(read_bus(&circuit, OUT, 16), 107);
        }
    }

    /// Back-to-back dependencies that the three-stage pipeline has to
    /// forward or stall for (see HAZARD_16B); all computers must agree.
    #[test]
    fn pipeline_hazards() {
        let src = "LI R1, 0x2000\n MOVI R2, 5\n SW R2, [R1]\n\
                   LW R3, [R1]\n ADD R3, R3\n ADDI R3, 1\n SW R3, [R1+1]\n\
                   LW R4, [R1+1]\n OUT R4\n PUSH R4\n POP R5\n\
                   LI R6, target\n SW R6, [R1+2]\n LW R7, [R1+2]\n JR R7\n HALT\n\
                   target: CMP R5, R4\n BEQ ok\n HALT\n\
                   ok: ADD R0, R5\n MOV R8, R0\n ADD R8, R5\n SW R8, [R1+3]\n HALT";
        for top in COMPUTERS_16 {
            let (circuit, machine) = run16(top, src, 0);
            assert_eq!(machine.ram[0x2000..0x2004], [5, 11, 0x0012, 11], "{top}");
            assert_eq!(read_bus(&circuit, OUT, 16), 11, "{top}");
        }
    }

    /// Wait states (IWait/DWait, or Wait on the memory bus) change the
    /// number of clocks, never the result.
    #[test]
    fn slow_memories() {
        let programs = [
            (include_str!("../programs/16/multiply.asm"), 0, 1234 * 37),
            (include_str!("../programs/16/factorial.asm"), 5, 120),
            (include_str!("../programs/16/sort.asm"), 0, 0x1234),
        ];
        for top in ["COMPUTER_16B_PIPE3", "COMPUTER_16B_SOC"] {
            for (src, input, out) in programs {
                let mut last = 0;
                for latency in [0, 1, 3] {
                    let (circuit, machine, clocks) = run16_slow(top, src, input, latency);
                    assert_eq!(read_bus(&circuit, OUT, 16), out, "{top}, latency {latency}");
                    if src.contains("Bubble sort") {
                        assert_eq!(machine.ram[0x100..0x108], [0, 1, 7, 42, 503, 999, 0x0FFF, 0x1234]);
                    }
                    assert!(clocks > last, "{top}, latency {latency}: {clocks} clocks");
                    last = clocks;
                }
            }
        }
    }

    /// With one memory for instructions and data, a program can write code
    /// and run it.
    #[test]
    fn shared_memory_runs_written_code() {
        // Writes "MOVI R2, 0x42; OUT R2; HALT" to 0x0100 and jumps there.
        let src = "LI R1, 0x0100\n LI R3, 0x8242\n SW R3, [R1]\n LI R3, 0x0207\n SW R3, [R1+1]\n\
                   MOVI R3, 0\n SW R3, [R1+2]\n JR R1";
        let (circuit, machine) = run16("COMPUTER_16B_SOC", src, 0);
        assert_eq!(machine.word_at(0x0100), 0x8242);
        assert_eq!(read_bus(&circuit, OUT, 16), 0x42);
    }

    /// SOC_16B peripherals: display, UART (decoded by the machine) and GPIO,
    /// also with slow memory.
    #[test]
    fn soc_peripherals() {
        for latency in [0, 2] {
            let (circuit, machine, _) =
                run16_slow("SOC_16B", include_str!("../programs/soc/hello.asm"), 0xBEEF, latency);
            assert_eq!(machine.uart, b"Hello!", "latency {latency}");
            assert_eq!(read_bus(&circuit, "GpioOut", 16), 0xBEEF);
            assert_eq!(read_bus(&circuit, "Io.Display", 16), 0xC0DE);
            // I/O writes stay inside the chip.
            assert_eq!(machine.ram[0xF002], 0);
        }
        // OUT also writes the display, so the programs of programs/16 show
        // their result on SOC_16B too.
        let (circuit, _) = run16("SOC_16B", include_str!("../programs/16/fibonacci.asm"), 0);
        assert_eq!(read_bus(&circuit, "Io.Display", 16), 46368);
    }

    /// The instruction-level simulator (isa::sim16) is the reference for the
    /// circuits: same Out, SP, flags and data memory on the example programs
    /// and the benchmarks.
    #[test]
    fn isa_simulator_matches_circuit() {
        use crate::isa::sim16::{BENCHMARKS, Cpu16};
        let mut programs: Vec<(&str, u16)> = vec![
            (include_str!("../programs/16/fibonacci.asm"), 0),
            (include_str!("../programs/16/multiply.asm"), 0),
            (include_str!("../programs/16/factorial.asm"), 6),
            (include_str!("../programs/16/sort.asm"), 0),
        ];
        programs.extend(BENCHMARKS.iter().map(|&(_, src, _)| (src, 0)));
        programs.push((MULDIV_TEST, 0));
        programs.push((BYTE_TEST, 0));
        for top in ["COMPUTER_16B", "SOC_16B"] {
            for &(src, input) in &programs {
                // Only CPU_16B_SOC has the multiply/divide unit and LB/SB.
                let uses_muldiv = ["MULL", "MULH", "DIVU", "MODU", "LB ", "SB "].iter().any(|m| src.contains(m));
                if top == "COMPUTER_16B" && uses_muldiv {
                    continue;
                }
                let words = Isa::Cpu16.assemble(src).unwrap().words;
                let (circuit, machine) = run16(top, src, input);
                let mut cpu = Cpu16::new(&words);
                cpu.input = input;
                cpu.run(1_000_000);
                assert_eq!(read_bus(&circuit, OUT, 16), cpu.out as u32, "{top}");
                assert_eq!(read_bus(&circuit, "Cpu.SP", 16), cpu.sp as u32, "{top}");
                let flags = ["Cpu.FlagZ", "Cpu.FlagN", "Cpu.FlagC", "Cpu.FlagV"].map(|f| read_bit(&circuit, f));
                assert_eq!(flags, [cpu.flags.z, cpu.flags.n, cpu.flags.c, cpu.flags.v], "{top}");
                if top == "COMPUTER_16B" {
                    // Data memory above the program (Harvard: the program is not in RAM).
                    assert_eq!(machine.ram[words.len()..], cpu.mem[words.len()..]);
                } else {
                    assert!((0..cpu.mem.len()).all(|a| machine.word_at(a) == cpu.mem[a]), "{top}: memory");
                }
            }
        }
    }

    /// MULL/MULH/DIVU/MODU with forwarding, a load-use operand, division by
    /// zero and the Z flag. Out = 51111.
    const MULDIV_TEST: &str = "LI R1, 1234\n MOVI R2, 37\n MOV R3, R1\n MULL R3, R2\n\
        LI R4, 0xFFFF\n MOV R5, R4\n MULH R5, R4\n LI R6, 12345\n MOVI R7, 100\n MOV R8, R6\n\
        DIVU R8, R7\n MODU R6, R7\n ADD R8, R6\n MOVI R9, 7\n DIVU R9, R0\n LI R10, 0x1000\n\
        SW R3, [R10]\n LW R11, [R10]\n MULL R11, R2\n MODU R1, R1\n BEQ ok\n HALT\n\
        ok: ADD R8, R5\n ADD R8, R9\n ADD R8, R11\n OUT R8\n HALT";

    /// Timer interrupts every `period` clocks while the main program adds
    /// 0x8000 `rounds` times into R2:R1 with ADD/ADC. The handler changes
    /// the flags on purpose: only if RETI restores C (and the handler saves
    /// its registers) is R2 = rounds / 2 at the end. The period must be
    /// longer than the handler (which takes longer with slow memory).
    fn interrupt_test(period: u16, rounds: u16) -> String {
        format!(
            "JMP main\n\
             isr: PUSH R7\n PUSH R8\n INC R9\n LI R7, 0xF000\n SW R9, [R7+2]\n MOVI R8, 3\n SW R8, [R7+7]\n\
             CMPI R9, 0x55\n POP R8\n POP R7\n RETI\n\
             main: LI R7, 0xF000\n LI R8, {}\n SW R8, [R7+6]\n MOVI R8, 3\n SW R8, [R7+7]\n\
             MOVI R1, 0\n MOVI R2, 0\n LI R3, 0x8000\n LI R4, {rounds}\n MOVI R9, 0\n EI\n\
             loop: ADD R1, R3\n ADC R2, R0\n DEC R4\n BNE loop\n DI\n OUT R2\n HALT",
            period - 1
        )
    }

    /// Timer interrupts on SOC_16B: handler entry/return, saved flags and
    /// registers, also with slow memory.
    #[test]
    fn soc_timer_interrupts() {
        for latency in [0, 1, 3] {
            let (circuit, _, _) = run16_slow("SOC_16B", &interrupt_test(150, 128), 0, latency);
            assert_eq!(read_bus(&circuit, OUT, 16), 64, "latency {latency}");
            let ticks = read_bus(&circuit, "Io.Display", 16);
            assert!(ticks >= 2, "latency {latency}: only {ticks} interrupts");
            let (circuit, _, _) = run16_slow("SOC_16B", include_str!("../programs/soc/timer.asm"), 0, latency);
            assert_eq!(read_bus(&circuit, OUT, 16), 8, "timer.asm, latency {latency}");
            assert_eq!(read_bus(&circuit, "Io.Display", 16), 8, "timer.asm, latency {latency}");
        }
    }

    /// SB/LB at even and odd byte addresses, a byte store next to a word,
    /// load-use on LB and an LB right after SB. Out = 0x9BF8.
    const BYTE_TEST: &str = "LI R3, 0x2001\n LI R1, 0x1234\n LI R4, 0x1000\n SW R1, [R4]\n\
        MOVI R2, 0x48\n SB R2, [R3+1]\n MOVI R2, 0x9A\n SB R2, [R3]\n LB R5, [R3]\n ADD R5, R5\n\
        LB R6, [R3+1]\n LW R7, [R4]\n LW R8, [R4+1]\n LB R9, [R3+7]\n ADD R7, R8\n ADD R7, R5\n\
        ADD R7, R6\n ADD R7, R9\n OUT R7\n HALT";

    /// Runs `src` on SOC_16B with `text` arriving on the UART receive line.
    fn run_soc_uart(src: &str, text: &str, latency: usize) -> (Circuit, Machine) {
        let (circuit, mut machine) = build("SOC_16B");
        machine.rom = Isa::Cpu16.assemble(src).unwrap().words;
        machine.input = Some(0);
        machine.latency = latency;
        machine.uart_in.extend(text.bytes());
        machine.reset(&circuit);
        machine.run(&circuit, 1_000_000);
        assert!(halted(&circuit), "no HALT");
        (circuit, machine)
    }

    /// UART receive (polled) and LB/SB: programs/soc/echo.asm.
    #[test]
    fn soc_uart_echo() {
        for latency in [0, 1] {
            let (_, machine) = run_soc_uart(include_str!("../programs/soc/echo.asm"), "Hello, SoC!\n", latency);
            assert_eq!(String::from_utf8_lossy(&machine.uart), "!COS ,OLLEH", "latency {latency}");
        }
    }

    /// UART receive interrupt: the handler stores each byte (SB) and counts
    /// them; the main program sums the bytes once four have arrived.
    #[test]
    fn soc_uart_receive_interrupt() {
        let src = "JMP main\n\
            isr: PUSH R1\n PUSH R2\n LI R1, 0xF000\n LW R2, [R1+8]\n SB R2, [R10]\n INC R10\n POP R2\n POP R1\n RETI\n\
            main: LI R1, 0xF000\n MOVI R2, 1\n SW R2, [R1+5]\n SW R2, [R1+9]\n LI R10, 0x2000\n EI\n\
            wait: LI R3, 0x2004\n CMP R10, R3\n BNE wait\n DI\n\
            LI R4, 0x2000\n MOVI R5, 0\n MOVI R6, 4\n sum: LB R7, [R4]\n ADD R5, R7\n INC R4\n DEC R6\n BNE sum\n\
            OUT R5\n HALT";
        let (circuit, machine) = run_soc_uart(src, "LC16", 0);
        assert_eq!(read_bus(&circuit, OUT, 16), (b'L' + b'C' + b'1' + b'6') as u32);
        assert_eq!(machine.word_at(0x1000), u16::from_le_bytes([b'L', b'C']));
    }
}
