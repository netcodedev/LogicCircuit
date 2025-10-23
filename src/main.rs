use std::path::Path;
use std::rc::Rc;
use std::cell::RefCell;
use std::collections::HashMap;
use uuid::Uuid; // Import uuid

mod cldl;
use crate::cldl::CircuitBuilder;

// --- Core Data Types ---

/// A struct to hold the wire's value and its unique ID
pub struct WireData {
    id: Uuid,
    value: bool,
}

impl std::fmt::Debug for WireData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state_char = if self.value { '●' } else { '○' }; // ● for on, ○ for off
        // Write the formatted string: (○ 123e4567-...)
        write!(f, "({} {})", state_char, self.id)
    }
}

// A 'Wire' is now a shared pointer to our WireData struct
type Wire = Rc<RefCell<WireData>>;

/// Creates a new, unique wire with a v4 UUID.
fn wire() -> Wire {
    Rc::new(RefCell::new(WireData {
        id: Uuid::new_v4(),
        value: false,
    }))
}

// --- LogicGate Trait ---
pub trait LogicGate: std::fmt::Debug {
    fn evaluate(&self) -> bool;
}

// --- Primitive Gate: NAND ---
#[derive(Debug)]
struct NandGate {
    in1: Wire,
    in2: Wire,
    out: Wire,
}

impl LogicGate for NandGate {
    fn evaluate(&self) -> bool {
        // Read the .value field from the WireData
        let val1 = self.in1.borrow().value;
        let val2 = self.in2.borrow().value;
        
        let new_val = !(val1 && val2);

        // Get the old value and update the new one
        // This is a clearer pattern than .replace()
        let old_val = self.out.borrow().value;
        self.out.borrow_mut().value = new_val;
        
        let changed = new_val != old_val;

        changed
    }
}

// --- Circuit Management ---
#[derive(Debug)]
pub struct Circuit {
    name: String, // Name for debugging, e.g., "HalfAdder"
    gates: Vec<Box<dyn LogicGate>>,
    inputs: HashMap<String, Wire>,
    outputs: HashMap<String, Wire>,
}

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
            // Update the .value field
            wire.borrow_mut().value = value;
        } else {
            eprintln!("Warning: Input '{}' not found.", name);
        }
    }

    pub fn get_output(&self, name: &str) -> Option<bool> {
        // Read the .value field
        self.outputs.get(name).map(|wire| wire.borrow().value)
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
                eprintln!("Warning: Circuit {} did not stabilize after {} iterations. Possible oscillation detected.", self.name, MAX_ITERATIONS);
                break;
            }
        }
    }
}

fn main() {
    println!("Initializing CircuitBuilder...");
    let mut builder = CircuitBuilder::new();

    let args: Vec<String> = std::env::args().collect();
    if let Some(path_str) = args.get(1) {
        let path = Path::new(path_str);
        if let Err(e) = builder.load_circuits_from_path(path) {
            eprintln!("Error loading custom circuits: {}", e);
        }
    }

    // --- SR Latch Test ---
    println!("\nTesting SR Latch (Active-Low):");
    let latch = builder.build("SRLatch");

    // Start in a stable state.
    // We must "Set" or "Reset" it first to get a known value.
    // Let's Reset it: nS=1, nR=0
    println!("1. Resetting Latch (nS=1, nR=0):");
    latch.set_input("nS", true);
    latch.set_input("nR", false);
    latch.evaluate_stable();
    println!(
        "   Q={}, nQ={}",
        latch.get_output("Q").unwrap(),
        latch.get_output("nQ").unwrap()
    );

    // Test 1: Hold after Reset (nS=1, nR=1)
    // The latch should "remember" its 0 state.
    println!("2. Holding State (nS=1, nR=1):");
    latch.set_input("nS", true);
    latch.set_input("nR", true);
    latch.evaluate_stable();
    println!(
        "   Q={}, nQ={}",
        latch.get_output("Q").unwrap(),
        latch.get_output("nQ").unwrap()
    );

    // Test 2: Set the Latch (nS=0, nR=1)
    println!("3. Setting Latch (nS=0, nR=1):");
    latch.set_input("nS", false);
    latch.set_input("nR", true);
    latch.evaluate_stable();
    println!(
        "   Q={}, nQ={}",
        latch.get_output("Q").unwrap(),
        latch.get_output("nQ").unwrap()
    );

    // Test 3: Hold after Set (nS=1, nR=1)
    // The latch should "remember" its 1 state.
    println!("4. Holding State (nS=1, nR=1):");
    latch.set_input("nS", true);
    latch.set_input("nR", true);
    latch.evaluate_stable();
    println!(
        "   Q={}, nQ={}",
        latch.get_output("Q").unwrap(),
        latch.get_output("nQ").unwrap()
    );
    
    // Test 4: Invalid State (nS=0, nR=0)
    // Both outputs should go to 1.
    println!("5. Invalid State (nS=0, nR=0):");
    latch.set_input("nS", false);
    latch.set_input("nR", false);
    latch.evaluate_stable();
    println!(
        "   Q={}, nQ={}",
        latch.get_output("Q").unwrap(),
        latch.get_output("nQ").unwrap()
    );

}
