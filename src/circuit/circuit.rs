use std::collections::HashMap;

use crate::circuit::{wire, Circuit, LogicGate, Wire};

impl Circuit {
    /// Creates a new, empty circuit with a name.
    pub fn new(name: String) -> Self {
        Circuit {
            name,
            gates: Vec::new(),
            inputs: HashMap::new(),
            outputs: HashMap::new(),
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
        self.get_output(name).expect(&format!("Output '{}' not found in circuit '{}'", name, self.name))
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
}