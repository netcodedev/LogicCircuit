//! Runs programs on a circuit that exposes a CPU memory interface.
//!
//! The circuit drives `PC` and reads `Instruction[16]`; [`Machine`] plays
//! the instruction memory (a ROM held in Rust) and the clock. Optional ports
//! are served when the circuit has them:
//! - `InstructionNext[16]`: the word at PC+1 (CPU_16B's two-word
//!   instructions); its presence selects [`Isa::Cpu16`]
//! - `MemAddr`/`MemDataIn`/`MemDataOut`/`MemWrite`: a data RAM that answers
//!   combinationally and stores on the rising clock edge
//! - `In`: an input port, set from [`Machine::input`] if that is `Some`
//! - `IWait`/`DWait` (with `MemRead`): wait states of slow memories, see
//!   [`Machine::latency`]
//!
//! The port widths come from the circuit, so CPU_8B, COMPUTER_8B, CPU_16B,
//! COMPUTER_16B and the pipelined COMPUTER_16B_PIPE(3) all work. On the
//! pipelines one step is one clock, not one instruction.

use std::cell::Cell;

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
pub const INPUT: &str = "In";
pub const OUT: &str = "Out";
pub const CLK: &str = "CLK";
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

/// True if the circuit has the ports needed to run programs on it.
pub fn is_runnable(circuit: &Circuit) -> bool {
    let map = circuit.get_wire_map();
    map.contains_key(&format!("{PC}.0"))
        && map.contains_key(&format!("{INSTRUCTION}.0"))
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
    /// `IWait`/`DWait` inputs present.
    i_wait: bool,
    d_wait: bool,
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
#[derive(Debug, Clone)]
pub struct Machine {
    ports: Ports,
    /// Instruction memory; words past the end read as 0 (HALT).
    pub rom: Vec<u16>,
    /// Data memory (empty if the circuit has no memory ports).
    pub ram: Vec<u16>,
    /// Value presented on the `In` port; `None` leaves it to the caller.
    pub input: Option<u16>,
    /// Wait states per memory access (instruction and data memory), for
    /// circuits with `IWait`/`DWait` ports; 0 = memories answer at once.
    pub latency: usize,
    fetch_wait: WaitState,
    data_wait: WaitState,
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
            i_wait: circuit.get_wire_map().contains_key(I_WAIT),
            d_wait: circuit.get_wire_map().contains_key(D_WAIT),
        };
        let ram_words = if ports.mem_data > 0 { 1 << ports.mem_addr } else { 0 };
        Some(Self {
            ports,
            rom: Vec::new(),
            ram: vec![0; ram_words],
            input: None,
            latency: 0,
            fetch_wait: WaitState::default(),
            data_wait: WaitState::default(),
        })
    }

    /// The instruction set of the CPU behind the ports.
    pub fn isa(&self) -> Isa {
        if self.ports.next > 0 { Isa::Cpu16 } else { Isa::Cpu8 }
    }

    /// Width of the PC in bits.
    pub fn pc_width(&self) -> usize {
        self.ports.pc
    }

    pub fn pc(&self, circuit: &Circuit) -> usize {
        read_bus(circuit, PC, self.ports.pc) as usize
    }

    /// Instruction memory at `addr` (wrapping at the PC width).
    pub fn word_at(&self, addr: usize) -> u16 {
        let addr = addr & ((1 << self.ports.pc) - 1);
        self.rom.get(addr).copied().unwrap_or(0)
    }

    /// Data memory at the address the circuit drives right now.
    fn mem_addr(&self, circuit: &Circuit) -> usize {
        read_bus(circuit, MEM_ADDR, self.ports.mem_addr) as usize
    }

    /// Clears the data memory.
    pub fn clear_ram(&mut self) {
        self.ram.fill(0);
    }

    /// Presents the word(s) at the current PC, the input port and the data
    /// memory word at `MemAddr`, and settles the circuit, so all
    /// combinational signals reflect the instruction about to execute.
    pub fn fetch(&self, circuit: &Circuit) {
        let pc = self.pc(circuit);
        write_bus(circuit, INSTRUCTION, INSTRUCTION_WIDTH, self.word_at(pc) as u32);
        write_bus(circuit, INSTRUCTION_NEXT, self.ports.next, self.word_at(pc + 1) as u32);
        if self.ports.i_wait {
            circuit.set_input(I_WAIT, self.fetch_wait.waiting(Some(pc), self.latency));
        }
        if let Some(input) = self.input {
            write_bus(circuit, INPUT, self.ports.input, input as u32);
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
        let store = (!self.ram.is_empty() && read_bit(circuit, MEM_WRITE) && !self.data_waiting(circuit))
            .then(|| (self.mem_addr(circuit), read_bus(circuit, MEM_DATA_OUT, self.ports.mem_data) as u16));
        circuit.set_input(CLK, true);
        circuit.evaluate_stable();
        if let Some((addr, data)) = store {
            self.ram[addr] = data;
        }
        circuit.set_input(CLK, false);
        circuit.evaluate_stable();
        self.fetch_wait.clock(false);
        self.data_wait.clock(true);
        self.fetch(circuit);
        true
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

    /// The 16-bit computers: single cycle, two- and three-stage pipeline.
    const COMPUTERS_16: [&str; 3] = ["COMPUTER_16B", "COMPUTER_16B_PIPE", "COMPUTER_16B_PIPE3"];

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

    /// Wait states (IWait/DWait) change the number of clocks, never the
    /// result.
    #[test]
    fn slow_memories() {
        let programs = [
            (include_str!("../programs/16/multiply.asm"), 0, 1234 * 37),
            (include_str!("../programs/16/factorial.asm"), 5, 120),
            (include_str!("../programs/16/sort.asm"), 0, 0x1234),
        ];
        for (src, input, out) in programs {
            let mut last = 0;
            for latency in [0, 1, 3] {
                let (circuit, machine, clocks) = run16_slow("COMPUTER_16B_PIPE3", src, input, latency);
                assert_eq!(read_bus(&circuit, OUT, 16), out, "latency {latency}");
                if src.contains("Bubble sort") {
                    assert_eq!(machine.ram[0x100..0x108], [0, 1, 7, 42, 503, 999, 0x0FFF, 0x1234]);
                }
                assert!(clocks > last, "latency {latency}: {clocks} clocks");
                last = clocks;
            }
        }
    }
}
