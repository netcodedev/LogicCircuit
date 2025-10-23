use std::rc::Rc;
use std::cell::RefCell;
use std::collections::HashMap;

// --- Core Data Types ---
type Wire = Rc<RefCell<bool>>;

fn wire() -> Wire {
    Rc::new(RefCell::new(false))
}

// --- LogicGate Trait ---
/// A trait for any component that can be 'evaluated'.
/// A simulation step will just call .evaluate() on all gates.
trait LogicGate {
    /// Evaluate the gate's logic based on its inputs and update its output.
    fn evaluate(&self) -> bool;
}

// --- Primitive Gate: NAND ---
struct NandGate {
    in1: Wire,
    in2: Wire,
    out: Wire,
}

impl LogicGate for NandGate {
    fn evaluate(&self) -> bool {
        // .borrow() gets an immutable reference to the value inside the RefCell.
        let val1 = *self.in1.borrow();
        let val2 = *self.in2.borrow();
        
        let new_val = !(val1 && val2);

        // Get the old value *before* replacing it, using .replace()
        let old_val = self.out.replace(new_val);
        
        // Return true if the value changed
        new_val != old_val
    }
}

// --- Circuit Management ---

/// A 'Circuit' holds all the gates and named I/O wires.
pub struct Circuit {
    /// A list of all gates in the circuit.
    gates: Vec<Box<dyn LogicGate>>,
    
    /// Named input wires for easy external access.
    inputs: HashMap<String, Wire>,
    
    /// Named output wires for easy external access.
    outputs: HashMap<String, Wire>,
}

impl Circuit {
    /// Creates a new, empty circuit.
    pub fn new() -> Self {
        Circuit {
            gates: Vec::new(),
            inputs: HashMap::new(),
            outputs: HashMap::new(),
        }
    }

    /// Adds a gate to the circuit's evaluation list.
    pub fn add_gate(&mut self, gate: Box<dyn LogicGate>) {
        self.gates.push(gate);
    }

    /// Creates a new named input wire.
    /// Returns the wire so it can be connected to gates.
    pub fn add_input(&mut self, name: &str) -> Wire {
        let new_wire = wire();
        self.inputs.insert(name.to_string(), new_wire.clone());
        new_wire
    }

    /// Links an existing wire to a named output.
    pub fn add_output(&mut self, name: &str, wire: Wire) {
        self.outputs.insert(name.to_string(), wire);
    }

    /// Sets a named input wire's value.
    pub fn set_input(&self, name: &str, value: bool) {
        if let Some(wire) = self.inputs.get(name) {
            wire.replace(value);
        } else {
            eprintln!("Warning: Input '{}' not found.", name);
        }
    }

    /// Gets a named output wire's value.
    pub fn get_output(&self, name: &str) -> Option<bool> {
        self.outputs.get(name).map(|wire| *wire.borrow())
    }

    /// Runs one full simulation step.
    /// It evaluates every gate in the order it was added.
    /// Returns `true` if any gate's output changed, `false` otherwise.
    pub fn evaluate_all(&self) -> bool {
        let mut changed = false;
        for gate in &self.gates {
            // If any gate changes, the `changed` flag becomes true
            // and stays true.
            if gate.evaluate() {
                changed = true;
            }
        }
        changed
    }
    
    /// Repeatedly evaluates the circuit until outputs stabilize.
    /// This is necessary for combinational logic to settle.
    pub fn evaluate_stable(&self) {
        // Add a safety break to prevent infinite loops from oscillators
        // (e.g., a NOT gate's output wired to its input).
        let mut iterations = 0;
        const MAX_ITERATIONS: u32 = 1000; 

        // Keep evaluating as long as *any* gate in the circuit
        // reports a change.
        while self.evaluate_all() {
            iterations += 1;
            if iterations > MAX_ITERATIONS {
                eprintln!("Warning: Circuit did not stabilize after {} iterations. Possible oscillation detected.", MAX_ITERATIONS);
                break;
            }
        }
    }
}

// --- Composite Gate Builders ---
// These functions build complex gates *from* NANDs
// and add them to the circuit.

/// Builds a NOT gate from one NAND gate.
/// NOT(a) = NAND(a, a)
fn add_not_gate(circuit: &mut Circuit, in_a: Wire) -> Wire {
    let out_wire = wire();
    let gate = NandGate {
        in1: in_a.clone(), // Input 'a' is fanned-out to both inputs
        in2: in_a,
        out: out_wire.clone(),
    };
    circuit.add_gate(Box::new(gate));
    out_wire
}

/// Builds an AND gate from two NAND gates.
/// AND(a, b) = NOT(NAND(a, b))
fn add_and_gate(circuit: &mut Circuit, in_a: Wire, in_b: Wire) -> Wire {
    let nand_out = wire();
    let nand_gate = NandGate {
        in1: in_a,
        in2: in_b,
        out: nand_out.clone(),
    };
    circuit.add_gate(Box::new(nand_gate));
    
    // The output of the NAND is fed into a NOT gate.
    let final_out = add_not_gate(circuit, nand_out);
    final_out
}

/// Builds an OR gate from three NAND gates.
/// OR(a, b) = NAND(NOT(a), NOT(b))
fn add_or_gate(circuit: &mut Circuit, in_a: Wire, in_b: Wire) -> Wire {
    // Create NOT(a) and NOT(b)
    let not_a = add_not_gate(circuit, in_a);
    let not_b = add_not_gate(circuit, in_b);

    // NAND them together
    let final_out = wire();
    let nand_gate = NandGate {
        in1: not_a,
        in2: not_b,
        out: final_out.clone(),
    };
    circuit.add_gate(Box::new(nand_gate));
    final_out
}

/// Builds an XOR gate from four NAND gates.
/// XOR(a, b) = NAND(NAND(a, NAND(a, b)), NAND(b, NAND(a, b)))
fn add_xor_gate(circuit: &mut Circuit, in_a: Wire, in_b: Wire) -> Wire {
    // 1. Common NAND(a, b)
    let nand_ab = wire();
    let nand1 = NandGate {
        in1: in_a.clone(),
        in2: in_b.clone(),
        out: nand_ab.clone(),
    };
    circuit.add_gate(Box::new(nand1));
    
    // 2. NAND(a, NAND(a, b))
    // Notice the fan-out: 'in_a' and 'nand_ab' are cloned.
    let out2 = wire();
    let nand2 = NandGate {
        in1: in_a,
        in2: nand_ab.clone(), 
        out: out2.clone(),
    };
    circuit.add_gate(Box::new(nand2));
    
    // 3. NAND(b, NAND(a, b))
    let out3 = wire();
    let nand3 = NandGate {
        in1: in_b,
        in2: nand_ab, // Last use of nand_ab
        out: out3.clone(),
    };
    circuit.add_gate(Box::new(nand3));

    // 4. NAND(out2, out3)
    let final_out = wire();
    let nand4 = NandGate {
        in1: out2,
        in2: out3,
        out: final_out.clone(),
    };
    circuit.add_gate(Box::new(nand4));
    
    final_out
}

// --- Main Example: Build and Test a Half-Adder ---

fn main() {
    println!("Building a Half-Adder from NAND-only composite gates...");
    let mut half_adder = Circuit::new();

    // 1. Define inputs
    let in_a = half_adder.add_input("a");
    let in_b = half_adder.add_input("b");

    // 2. Build the circuit
    // SUM = XOR(a, b)
    // CARRY = AND(a, b)
    
    // We must clone the inputs because they are fanned-out
    // to both the XOR and AND gates.
    let sum_wire = add_xor_gate(&mut half_adder, in_a.clone(), in_b.clone());
    let carry_wire = add_and_gate(&mut half_adder, in_a, in_b);

    // 3. Define outputs
    half_adder.add_output("sum", sum_wire);
    half_adder.add_output("carry", carry_wire);

    // 4. Test the truth table
    println!("Testing Half-Adder Truth Table:");

    // Test: 0 + 0 = 0 (carry 0)
    half_adder.set_input("a", false);
    half_adder.set_input("b", false);
    half_adder.evaluate_stable(); // Run simulation until stable
    println!(
        "0 + 0: Sum={}, Carry={}",
        half_adder.get_output("sum").unwrap(),
        half_adder.get_output("carry").unwrap()
    );

    // Test: 1 + 0 = 1 (carry 0)
    half_adder.set_input("a", true);
    half_adder.set_input("b", false);
    half_adder.evaluate_stable();
    println!(
        "1 + 0: Sum={}, Carry={}",
        half_adder.get_output("sum").unwrap(),
        half_adder.get_output("carry").unwrap()
    );

    // Test: 0 + 1 = 1 (carry 0)
    half_adder.set_input("a", false);
    half_adder.set_input("b", true);
    half_adder.evaluate_stable();
    println!(
        "0 + 1: Sum={}, Carry={}",
        half_adder.get_output("sum").unwrap(),
        half_adder.get_output("carry").unwrap()
    );

    // Test: 1 + 1 = 0 (carry 1)
    half_adder.set_input("a", true);
    half_adder.set_input("b", true);
    half_adder.evaluate_stable();
    println!(
        "1 + 1: Sum={}, Carry={}",
        half_adder.get_output("sum").unwrap(),
        half_adder.get_output("carry").unwrap()
    );
}