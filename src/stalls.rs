//! Where the clocks of a CPU_16B_SOC go: for every clock, whether the
//! execute stage runs an instruction and, if not, why.
//!
//! The execute stage does nothing in a clock when it holds (`HoldX`: memory
//! wait, load-use stall, multiply/divide, second word of a two-word
//! instruction, interrupt entry) or when `Ir` holds a bubble (`IrValid` = 0).
//! A bubble comes from the clock before, when `Ir` loaded no word
//! (`Flush`): a jump from execute or memory dropped the fetched word, or the
//! fetch got no word because the memory stage had the bus and the cache
//! missed. The reason is taken from the internal wires on that clock.

use crate::circuit::Circuit;
use crate::machine::{Machine, read_bit};

/// Why the execute stage ran no instruction in a clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Loss {
    /// The memory stage waits for the external memory (`Wait`).
    MemWait,
    /// A loaded value is needed by the next instruction.
    LoadUse,
    /// MULL/MULH/DIVU/MODU run for 16 clocks.
    MulDiv,
    /// LI/JMP/CALL wait for their second word.
    SecondWord,
    /// The memory stage had the bus and the word at PC was not cached.
    BusBusy,
    /// The fetch got no word because the memory had wait states.
    FetchWait,
    /// JMP, CALL, JR, CALLR: the target is known in execute.
    Jump,
    /// RET: in execute (word dropped) and again in memory (target from the stack).
    Ret,
    /// A branch taken in execute that the fetch did not predict.
    BranchNotPredicted,
    /// A branch predicted taken that was not taken.
    Mispredicted,
    /// Interrupt entry.
    Interrupt,
    /// A bubble whose cause was not seen (start after reset).
    Other,
}

impl Loss {
    pub const ALL: [Loss; 12] = [
        Loss::LoadUse,
        Loss::BusBusy,
        Loss::SecondWord,
        Loss::Ret,
        Loss::Jump,
        Loss::BranchNotPredicted,
        Loss::Mispredicted,
        Loss::MulDiv,
        Loss::MemWait,
        Loss::FetchWait,
        Loss::Interrupt,
        Loss::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Loss::MemWait => "memory wait",
            Loss::LoadUse => "load-use stall",
            Loss::MulDiv => "MUL/DIV",
            Loss::SecondWord => "second word",
            Loss::BusBusy => "bus busy, cache miss",
            Loss::FetchWait => "fetch wait",
            Loss::Jump => "jump (JMP/CALL/JR)",
            Loss::Ret => "RET",
            Loss::BranchNotPredicted => "branch not predicted",
            Loss::Mispredicted => "misprediction",
            Loss::Interrupt => "interrupt",
            Loss::Other => "other",
        }
    }
}

/// Clock counts of one run.
#[derive(Clone, Debug, Default)]
pub struct Profile {
    pub clocks: usize,
    /// Clocks in which execute ran an instruction.
    pub executed: usize,
    /// Branches the fetch predicted taken (they cost no clock today).
    pub predicted: usize,
    pub lost: std::collections::BTreeMap<Loss, usize>,
}

impl Profile {
    pub fn add(&mut self, other: &Profile) {
        self.clocks += other.clocks;
        self.executed += other.executed;
        self.predicted += other.predicted;
        for (k, v) in &other.lost {
            *self.lost.entry(*k).or_default() += v;
        }
    }

    pub fn lost(&self, loss: Loss) -> usize {
        self.lost.get(&loss).copied().unwrap_or(0)
    }
}

/// Wire names of the CPU inside `circuit`: the prefix under which
/// CPU_16B_SOC's `HoldX` lives ("" for the CPU itself, "Cpu." for SOC_16B).
pub fn cpu_prefix(circuit: &Circuit) -> Option<String> {
    ["", "Cpu."]
        .into_iter()
        .find(|p| circuit.has_wire(&format!("{p}HoldX.out")) && circuit.has_wire(&format!("{p}IrValid.Q.0")))
        .map(str::to_string)
}

/// Runs like [`Machine::run`] (until HALT or `max_steps`) and counts where
/// the clocks go. `None` if the circuit has no CPU_16B_SOC.
pub fn run_profiled(m: &mut Machine, circuit: &Circuit, max_steps: usize) -> Option<Profile> {
    let p = cpu_prefix(circuit)?;
    let bit = |name: &str| read_bit(circuit, &format!("{p}{name}"));
    let mut prof = Profile::default();
    // Reason for the bubble that Ir loads on the coming edge.
    let mut pending: Option<Loss> = None;
    while prof.clocks < max_steps {
        // The state before the edge: what execute does in this clock.
        let loss = if bit("HoldX.out") {
            Some(if bit("FreezeAnd.out") {
                Loss::MemWait
            } else if bit("Hazard.Stall") {
                Loss::LoadUse
            } else if bit("MdWait.out") {
                Loss::MulDiv
            } else if bit("NeedAnd.out") {
                Loss::SecondWord
            } else {
                Loss::Interrupt
            })
        } else if !bit("IrValid.Q.0") {
            Some(pending.unwrap_or(Loss::Other))
        } else {
            None
        };
        // What Ir loads on the edge.
        if bit("Flush.out") {
            pending = Some(if bit("IrqTake.Out") {
                Loss::Interrupt
            } else if bit("Misp.out") {
                Loss::Mispredicted
            } else if bit("BranchAnd.out") {
                Loss::BranchNotPredicted
            } else if bit("RetAnd.Out") || bit("CtrlM.Q.7") {
                Loss::Ret
            } else if bit("JumpAnd.out") {
                Loss::Jump
            } else if bit("AccessOr.out") {
                Loss::BusBusy
            } else {
                Loss::FetchWait
            });
        } else if bit("AdvanceAnd.Out") {
            pending = None;
        }
        if bit("PredJump.out") {
            prof.predicted += 1;
        }
        if !m.step(circuit) {
            break;
        }
        prof.clocks += 1;
        match loss {
            Some(l) => *prof.lost.entry(l).or_default() += 1,
            None => prof.executed += 1,
        }
    }
    Some(prof)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cldl::CircuitBuilder;
    use crate::isa::Isa;
    use crate::isa::sim16::Cpu16;
    use std::path::Path;

    /// Every clock is either an executed instruction or one lost clock, and
    /// the executed ones are exactly the instructions of the
    /// instruction-level simulator, also with wait states.
    #[test]
    fn clocks_add_up_to_instructions_and_losses() {
        let mut b = CircuitBuilder::new();
        b.load_circuits_from_path(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/circuits"))).unwrap();
        let src = include_str!("../programs/bench/calls.asm");
        let words = Isa::Cpu16.assemble(src).unwrap().words;
        let mut cpu = Cpu16::new(&words);
        cpu.run(1_000_000);
        for latency in [0, 2] {
            let circuit = b.build("SOC_16B");
            let mut m = Machine::new(&circuit).unwrap();
            m.rom = words.clone();
            m.input = Some(0);
            m.latency = latency;
            m.reset(&circuit);
            let p = run_profiled(&mut m, &circuit, 1_000_000).unwrap();
            assert!(crate::machine::halted(&circuit));
            assert_eq!(p.executed as u64, cpu.steps, "latency {latency}");
            assert_eq!(p.executed + p.lost.values().sum::<usize>(), p.clocks);
            assert!(p.lost(Loss::Ret) > 0 && p.lost(Loss::SecondWord) > 0);
            if latency > 0 {
                assert!(p.lost(Loss::MemWait) + p.lost(Loss::FetchWait) > 0);
            }
        }
    }
}
