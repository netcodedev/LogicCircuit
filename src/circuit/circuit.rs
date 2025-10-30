use std::collections::HashMap;

use crate::circuit::{Circuit, LogicGate, Wire, wire};

impl Circuit {
    /// Creates a new, empty circuit with a name.
    pub fn new(name: String) -> Self {
        Circuit {
            name,
            gates: Vec::new(),
            inputs: HashMap::new(),
            outputs: HashMap::new(),
            wire_map: HashMap::new(),
        }
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn add_gate(&mut self, gate: Box<dyn LogicGate>) {
        self.gates.push(gate);
    }

    pub fn add_input(&mut self, name: &str) -> Wire {
        let new_wire = wire();
        self.inputs.insert(name.to_string(), new_wire.clone());
        new_wire
    }

    pub fn add_output(&mut self, name: &str, wire: Wire) {
        self.outputs.insert(name.to_string(), wire);
    }

    pub fn set_input(&self, name: &str, value: bool) {
        if let Some(wire) = self.inputs.get(name) {
            wire.borrow_mut().value = value;
        } else {
            eprintln!("Warning: Input '{}' not found.", name);
        }
    }

    pub fn get_output(&self, name: &str) -> Option<bool> {
        self.outputs.get(name).map(|wire| wire.borrow().value)
    }

    pub fn get_output_unwrapped(&self, name: &str) -> bool {
        self.get_output(name).expect(&format!(
            "Output '{}' not found in circuit '{}'",
            name, self.name
        ))
    }

    pub fn set_wire_map(&mut self, wire_map: HashMap<String, Wire>) {
        self.wire_map = wire_map;
    }

    pub fn get_wire_map(&self) -> &HashMap<String, Wire> {
        &self.wire_map
    }

    pub fn evaluate_all(&self) -> bool {
        let mut changed = false;
        for gate in &self.gates {
            if gate.evaluate() {
                changed = true;
            }
        }
        changed
    }

    pub fn evaluate_stable(&self) {
        let mut iterations = 0;
        const MAX_ITERATIONS: u32 = 1000;

        while self.evaluate_all() {
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

    // --- Debugging Helper (similar to Simulator's) ---
    pub fn print_io_state(&self) {
        println!("[Circuit] IO State for {}:", self.name);
        let mut inputs_sorted: Vec<_> = self.inputs.iter().collect();
        inputs_sorted.sort_by_key(|(name, _)| *name);
        println!("  Inputs: ");
        for (name, wire) in inputs_sorted {
            println!("    {}={:?} ", name, wire);
        }
        println!();

        let mut outputs_sorted: Vec<_> = self.outputs.iter().collect();
        outputs_sorted.sort_by_key(|(name, _)| *name);
        println!("  Outputs: ");
        for (name, wire) in outputs_sorted {
            println!("    {}={:?} ", name, wire);
        }
        println!();
    }
}
