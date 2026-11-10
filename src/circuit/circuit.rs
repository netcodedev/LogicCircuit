use std::cell::Cell;
use std::collections::HashMap;

use super::schedule::Schedule;
use super::{Circuit, Element, Flop, FlopState, Gate, GateKind, SimGate, WireId};

/// Most passes `evaluate_stable` makes before it reports an oscillation.
pub(super) const MAX_ITERATIONS: u32 = 1000;

impl Circuit {
    /// Creates a new, empty circuit with a name and `wires` wires (all 0).
    pub fn new(name: String, wires: usize) -> Self {
        Circuit {
            name,
            values: (0..wires).map(|_| Cell::new(false)).collect(),
            gates: Vec::new(),
            gate_ids: Vec::new(),
            flops: Vec::new(),
            flop_state: Vec::new(),
            elements: Vec::new(),
            inputs: HashMap::new(),
            outputs: HashMap::new(),
            wire_map: HashMap::new(),
            gate_kinds: HashMap::new(),
            flop_index: HashMap::new(),
            schedule: None,
        }
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    /// Number of gates the circuit was built from (built-in gates, see
    /// `GateKind`; NAND gates only for `CircuitBuilder::build_nand`).
    /// Built-in flip-flops are counted by `flop_count`.
    pub fn gate_count(&self) -> usize {
        self.gates.len()
    }

    /// Number of built-in flip-flops (`DFLIP_FLOP`, see `Flop`).
    pub fn flop_count(&self) -> usize {
        self.flops.len()
    }

    /// Gates and flip-flops added so far.
    pub fn element_count(&self) -> usize {
        self.elements.len()
    }

    /// Adds a built-in gate and remembers its type by instance path.
    pub fn add_builtin(&mut self, gate: Gate) {
        assert!(self.schedule.is_none(), "circuit already finalized");
        let mut inputs = [0; 4];
        inputs[..gate.inputs.len()].copy_from_slice(&gate.inputs);
        self.gate_kinds.insert(gate.id.clone(), gate.kind);
        self.elements.push(Element::Gate(self.gates.len() as u32));
        self.gates.push(SimGate { kind: gate.kind, n: gate.inputs.len() as u8, inputs, out: gate.out });
        self.gate_ids.push(gate.id);
    }

    /// Adds a built-in flip-flop (Q = 0, nQ = 1).
    pub fn add_flop(&mut self, flop: Flop) {
        assert!(self.schedule.is_none(), "circuit already finalized");
        self.values[flop.q as usize].set(false);
        self.values[flop.nq as usize].set(true);
        self.flop_index.insert(flop.id.clone(), self.flops.len());
        self.elements.push(Element::Flop(self.flops.len() as u32));
        self.flops.push(flop);
        self.flop_state.push(Cell::new(FlopState::default()));
    }

    /// Type of the built-in gate at instance path `id` (e.g. "Cpu.Alu.G3").
    pub fn gate_kind(&self, id: &str) -> Option<GateKind> {
        self.gate_kinds.get(id).copied()
    }

    /// The built-in flip-flop at instance path `id` (e.g. "Pc.Reg.DFF[3]").
    pub fn flop(&self, id: &str) -> Option<&Flop> {
        self.flop_index.get(id).map(|&i| &self.flops[i])
    }

    /// Registers wire `wire` as a named circuit input.
    pub fn register_input(&mut self, name: &str, wire: WireId) {
        self.inputs.insert(name.to_string(), wire);
    }

    pub fn add_output(&mut self, name: &str, wire: WireId) {
        self.outputs.insert(name.to_string(), wire);
    }

    pub fn set_input(&self, name: &str, value: bool) {
        if let Some(&wire) = self.inputs.get(name) {
            self.write(wire, value);
        } else {
            eprintln!("Warning: Input '{}' not found.", name);
        }
    }

    #[allow(dead_code)]
    pub fn get_output(&self, name: &str) -> Option<bool> {
        self.outputs.get(name).map(|&w| self.value(w))
    }

    /// Value of the wire `name`; panics if there is none.
    pub fn get_wire_value(&self, name: &str) -> bool {
        self.wire_value(name).unwrap_or_else(|| panic!("Value '{}' not found", name))
    }

    /// Value of the wire `name` (any wire key, e.g. "Cpu.Pc.Q.3").
    pub fn wire_value(&self, name: &str) -> Option<bool> {
        self.wire_map.get(name).map(|&w| self.value(w))
    }

    /// True if the circuit has a wire `name`.
    pub fn has_wire(&self, name: &str) -> bool {
        self.wire_map.contains_key(name)
    }

    /// Index of the wire `name`.
    pub fn wire_id(&self, name: &str) -> Option<WireId> {
        self.wire_map.get(name).copied()
    }

    /// Value of wire `id`.
    #[inline]
    pub fn value(&self, id: WireId) -> bool {
        self.values[id as usize].get()
    }

    pub fn set_wire_map(&mut self, wire_map: HashMap<String, WireId>) {
        self.wire_map = wire_map;
    }

    /// All wire keys with their wire.
    pub fn wire_names(&self) -> impl Iterator<Item = (&str, WireId)> {
        self.wire_map.iter().map(|(k, &w)| (k.as_str(), w))
    }

    /// Sets wire `id` (an initialization step during the build).
    pub(crate) fn set_wire(&self, id: WireId, value: bool) {
        self.write(id, value);
    }

    /// Writes a wire; true if its value changed. A change marks the gates
    /// and flip-flops that read the wire for the next evaluation.
    #[inline]
    pub(super) fn write(&self, id: WireId, value: bool) -> bool {
        let cell = &self.values[id as usize];
        if cell.get() == value {
            return false;
        }
        cell.set(value);
        if let Some(s) = &self.schedule {
            s.mark(id);
        }
        true
    }

    /// Output value of gate `g` for the current wire values.
    #[inline]
    pub(super) fn eval_gate(&self, g: &SimGate) -> bool {
        let v = |i: usize| self.values[g.inputs[i] as usize].get();
        match g.kind {
            GateKind::Nand => !(v(0) && v(1)),
            GateKind::Not => !v(0),
            GateKind::And => v(0) && v(1),
            GateKind::And3 => v(0) && v(1) && v(2),
            GateKind::Or => v(0) || v(1),
            GateKind::Nor => !(v(0) || v(1)),
            GateKind::Nor4 => !(v(0) || v(1) || v(2) || v(3)),
            GateKind::Xor => v(0) ^ v(1),
            GateKind::Xnor => !(v(0) ^ v(1)),
        }
    }

    pub(super) fn sim_gate(&self, i: u32) -> &SimGate {
        &self.gates[i as usize]
    }

    pub(super) fn sim_gates(&self) -> &[SimGate] {
        &self.gates
    }

    pub(super) fn sim_flops(&self) -> &[Flop] {
        &self.flops
    }

    pub(super) fn wire_count(&self) -> usize {
        self.values.len()
    }

    /// Clock check of flip-flop `f`: its new Q if the clock rose since the
    /// last check (D at this moment), otherwise None. Remembers the clock.
    #[inline]
    pub(super) fn clock_flop(&self, f: u32) -> Option<bool> {
        let flop = &self.flops[f as usize];
        let clk = self.value(flop.clk);
        let state = &self.flop_state[f as usize];
        let rose = clk && !state.get().last_clk;
        state.set(FlopState { last_clk: clk });
        rose.then(|| self.value(flop.d))
    }

    /// Sets the outputs of flip-flop `f` to `q`; true if Q changed.
    #[inline]
    pub(super) fn set_flop(&self, f: u32, q: bool) -> bool {
        let flop = &self.flops[f as usize];
        self.write(flop.nq, !q);
        self.write(flop.q, q)
    }

    /// Evaluates the gates and flip-flops from index `first` on (in build
    /// order) once; true if any output changed.
    fn sweep_from(&self, first: usize) -> bool {
        let mut changed = false;
        for e in &self.elements[first.min(self.elements.len())..] {
            match *e {
                Element::Gate(g) => {
                    let g = &self.gates[g as usize];
                    changed |= self.write(g.out, self.eval_gate(g));
                }
                Element::Flop(f) => {
                    if let Some(q) = self.clock_flop(f) {
                        changed |= self.set_flop(f, q);
                    }
                }
            }
        }
        changed
    }

    /// Evaluates until no wire changes: gates in topological order, then the
    /// flip-flops whose clock changed, and again while a flip-flop changed.
    /// Before `finalize` (during the build) this sweeps all elements in
    /// build order instead.
    pub fn evaluate_stable(&self) {
        match &self.schedule {
            Some(s) => s.run(self),
            None => self.evaluate_stable_from(0),
        }
    }

    /// Sweeps the gates and flip-flops from index `first` on (in build
    /// order) until nothing changes; used during the build, where an
    /// initialization only evaluates its own part.
    pub fn evaluate_stable_from(&self, first: usize) {
        let mut iterations = 0;
        while self.sweep_from(first) {
            iterations += 1;
            if iterations > MAX_ITERATIONS {
                eprintln!(
                    "Warning: Circuit {} did not stabilize after {} iterations. Possible oscillation detected.",
                    self.name, MAX_ITERATIONS
                );
                break;
            }
        }
    }

    /// Ends the build: sorts the gates for `evaluate_stable` and marks
    /// everything for the first evaluation.
    pub fn finalize(&mut self) {
        if self.schedule.is_none() {
            self.schedule = Some(Schedule::new(self));
        }
    }

    pub fn print_io_state(&self) {
        println!("[Circuit] IO State for {}:", self.name);
        for (title, map) in [("Inputs", &self.inputs), ("Outputs", &self.outputs)] {
            let mut sorted: Vec<_> = map.iter().collect();
            sorted.sort_by_key(|(name, _)| *name);
            println!("  {title}: ");
            for (name, &wire) in sorted {
                println!("    {}={} ", name, self.value(wire) as u8);
            }
            println!();
        }
    }
}
