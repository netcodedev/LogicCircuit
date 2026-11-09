//! Runs programs on a circuit that exposes a CPU memory interface.
//!
//! The circuit drives `PC[8]` and reads `Instruction[16]`; this module plays
//! the instruction memory (a ROM held in Rust) and the clock. Anything with
//! these ports works, e.g. CPU_8B or COMPUTER_8B. A later RAM-backed setup
//! only needs a different memory behind the same ports.

use crate::circuit::Circuit;
use crate::isa::MEMORY_WORDS;

pub const PC: &str = "PC";
pub const INSTRUCTION: &str = "Instruction";
pub const CLK: &str = "CLK";
pub const HALTED: &str = "Halted";

const PC_WIDTH: usize = 8;
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

/// True if the circuit has the ports needed to run programs on it.
pub fn is_runnable(circuit: &Circuit) -> bool {
    let map = circuit.get_wire_map();
    map.contains_key(&format!("{PC}.0"))
        && map.contains_key(&format!("{INSTRUCTION}.0"))
        && map.contains_key(CLK)
}

pub fn pc(circuit: &Circuit) -> usize {
    read_bus(circuit, PC, PC_WIDTH) as usize
}

pub fn halted(circuit: &Circuit) -> bool {
    circuit
        .get_wire_map()
        .get(HALTED)
        .is_some_and(|w| w.borrow().value)
}

/// Instruction memory: `rom[pc]`, with empty cells reading as 0 (HALT).
pub fn word_at(rom: &[u16], pc: usize) -> u16 {
    rom.get(pc % MEMORY_WORDS).copied().unwrap_or(0)
}

/// Presents the word at the current PC on `Instruction` and settles the
/// circuit, so all combinational signals reflect the instruction about to
/// execute.
pub fn fetch(circuit: &Circuit, rom: &[u16]) {
    write_bus(circuit, INSTRUCTION, INSTRUCTION_WIDTH, word_at(rom, pc(circuit)) as u32);
    circuit.evaluate_stable();
}

/// Executes one instruction: fetch, one full clock cycle, fetch the next.
/// Returns `false` (and does nothing) if the CPU is halted.
pub fn step(circuit: &Circuit, rom: &[u16]) -> bool {
    fetch(circuit, rom);
    if halted(circuit) {
        return false;
    }
    circuit.set_input(CLK, true);
    circuit.evaluate_stable();
    circuit.set_input(CLK, false);
    circuit.evaluate_stable();
    fetch(circuit, rom);
    true
}

/// Runs until HALT or `max_steps`; returns the number of executed steps.
pub fn run(circuit: &Circuit, rom: &[u16], max_steps: usize) -> usize {
    let mut steps = 0;
    while steps < max_steps && step(circuit, rom) {
        steps += 1;
    }
    steps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cldl::CircuitBuilder;
    use crate::isa::assemble;
    use std::path::Path;

    fn computer() -> Circuit {
        let mut builder = CircuitBuilder::new();
        builder
            .load_circuits_from_path(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/circuits")))
            .unwrap();
        builder.build("COMPUTER_8B")
    }

    #[test]
    fn runs_example_programs_to_halt() {
        for (src, out) in [
            (include_str!("../programs/countdown.asm"), 0),
            (include_str!("../programs/fibonacci.asm"), 233),
            (include_str!("../programs/hex.asm"), 8),
        ] {
            let circuit = computer();
            let rom = assemble(src).unwrap().words;
            run(&circuit, &rom, 1000);
            assert!(halted(&circuit));
            assert_eq!(read_bus(&circuit, "Out", 8), out);
        }
    }

    #[test]
    fn empty_memory_halts_immediately() {
        let circuit = computer();
        assert_eq!(run(&circuit, &[], 10), 0);
        assert_eq!(pc(&circuit), 0);
    }
}
