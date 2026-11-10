//! The simulator: a built circuit of built-in gates (`GateKind`) and
//! rising-edge D flip-flops (`DFLIP_FLOP`) over numbered wires.
//!
//! Every wire is an index into one array of values. After the build the
//! gates are sorted once in topological order (flip-flop outputs cut the
//! feedback paths; any other feedback loop becomes one group that is iterated
//! until it settles), so `evaluate_stable` evaluates each gate at most once
//! per pass, and only the gates whose inputs changed. See `schedule.rs`.

use std::collections::HashMap;

mod circuit;
mod gates;
mod schedule;

pub use gates::{Gate, GateKind};

/// Index of a wire in a built circuit.
pub type WireId = u32;

/// A rising-edge D flip-flop with outputs Q and nQ (our `DFLIP_FLOP`, built
/// in instead of its two latches; the latch version stays for the NAND
/// views and counts). Starts with Q = 0, as the latch version's INIT does.
#[derive(Debug, Clone)]
pub struct Flop {
    pub id: String,
    pub d: WireId,
    pub clk: WireId,
    pub q: WireId,
    pub nq: WireId,
}

/// A gate in its compact, simulated form.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SimGate {
    pub kind: GateKind,
    pub n: u8,
    pub inputs: [WireId; 4],
    pub out: WireId,
}

/// One element in build order.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Element {
    Gate(u32),
    Flop(u32),
}

/// State of a flip-flop besides its output wires.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FlopState {
    /// Clock level seen at the last evaluation (for edge detection).
    pub last_clk: bool,
}

#[derive(Debug)]
pub struct Circuit {
    name: String,
    /// Value of every wire.
    values: Vec<std::cell::Cell<bool>>,
    gates: Vec<SimGate>,
    gate_ids: Vec<String>,
    flops: Vec<Flop>,
    flop_state: Vec<std::cell::Cell<FlopState>>,
    /// Gates and flip-flops in build order (for the evaluation during the
    /// build, before the schedule exists).
    elements: Vec<Element>,
    inputs: HashMap<String, WireId>,
    outputs: HashMap<String, WireId>,
    wire_map: HashMap<String, WireId>,
    /// Instance path -> type of every built-in gate.
    gate_kinds: HashMap<String, GateKind>,
    /// Instance path -> index of every built-in flip-flop.
    flop_index: HashMap<String, usize>,
    /// Evaluation order and change tracking, made by `finalize`.
    schedule: Option<schedule::Schedule>,
}

#[cfg(test)]
mod tests {
    use crate::cldl::CircuitBuilder;

    /// The built-in flip-flops behave like the latch version: random inputs
    /// and clocks, every wire both builds have compared after each step.
    #[test]
    fn built_in_flip_flops_match_latches() {
        let builder = CircuitBuilder::new();
        for name in ["DFLIP_FLOP", "SYNC<2>", "REGISTER_R<4>", "REG_FILE_16<4>"] {
            let fast = builder.build(name);
            let slow = builder.build_latches(name);
            assert!(fast.flop_count() > 0 || name == "DFLIP_FLOP", "{name}");
            assert_eq!(slow.flop_count(), 0);
            let mut inputs: Vec<&str> = fast.inputs.keys().map(String::as_str).collect();
            inputs.sort();
            let mut seed = 0x2545_f491_4f6c_dd1du64;
            for step in 0..400 {
                for &pin in &inputs {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    // The clock rises every fourth step, on its own (data
                    // changing right at the edge is a race in hardware too);
                    // the data changes at random in the other steps.
                    let phase = step % 4;
                    let v = if pin == "CLK" { phase == 2 || phase == 3 } else { seed & 3 == 0 };
                    if pin == "CLK" || phase != 2 {
                        fast.set_input(pin, v);
                        slow.set_input(pin, v);
                    }
                }
                fast.evaluate_stable();
                slow.evaluate_stable();
                for (key, w) in fast.wire_names() {
                    assert_eq!(fast.value(w), slow.get_wire_value(key), "{name}: {key} at step {step}");
                }
            }
        }
    }
}
