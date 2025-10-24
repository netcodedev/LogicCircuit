use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::Path;
use std::rc::Rc;
use uuid::Uuid;

mod cldl;
use crate::cldl::CircuitBuilder;

const EXPORT_DLATCH: bool = false;
/// A struct to hold the wire's value and its unique ID
pub struct WireData {
    id: Uuid,
    value: bool,
}

impl std::fmt::Debug for WireData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state_char = if self.value { '●' } else { '○' }; // ● for on, ○ for off
        write!(f, "({} {})", state_char, self.id)
    }
}

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
struct NandGate {
    id: String,
    in1: Wire,
    in2: Wire,
    out: Wire,
}

impl fmt::Debug for NandGate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NandGate")
            .field("id", &self.id)
            .field("in1", &self.in1)
            .field("in2", &self.in2)
            .field("out", &self.out)
            .finish()
    }
}

impl LogicGate for NandGate {
    fn evaluate(&self) -> bool {
        let val1 = self.in1.borrow().value;
        let val2 = self.in2.borrow().value;

        let new_val = !(val1 && val2);

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
            wire.borrow_mut().value = value;
        } else {
            eprintln!("Warning: Input '{}' not found.", name);
        }
    }

    pub fn get_output(&self, name: &str) -> Option<bool> {
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
                eprintln!(
                    "Warning: Circuit {} did not stabilize after {} iterations. Possible oscillation detected.",
                    self.name, MAX_ITERATIONS
                );
                break;
            }
        }
    }
}

fn main() {
    let mut builder = CircuitBuilder::new();

    let args: Vec<String> = std::env::args().collect();
    if let Some(path_str) = args.get(1) {
        let path = Path::new(path_str);
        if let Err(e) = builder.load_circuits_from_path(path) {
            eprintln!("Error loading custom circuits: {}", e);
        }
    }
    // --- D Flip-Flop Test ---
    println!("\nTesting D Flip-Flop:");
    let dff = builder.build("DFlipFlop");

    // Now start the actual test sequence. CLK is already low. D is 0.
    println!("1. Initial State (CLK=0, D=0) after priming:");
    println!(
        "   Q={}, nQ={}",
        dff.get_output("Q").unwrap(),
        dff.get_output("nQ").unwrap()
    );

    // Set D=1, but keep CLK=0. Q should not change (still 0).
    println!("2. Set D=1 (CLK=0):");
    dff.set_input("D", true);
    dff.evaluate_stable();
    println!(
        "   Q={}, nQ={}",
        dff.get_output("Q").unwrap(),
        dff.get_output("nQ").unwrap()
    );

    // Pulse the clock: 0 -> 1 (Rising Edge)
    // The Master latch (which captured D=1 just before edge) value is copied to Slave.
    // Q should become 1.
    println!("3. Clock Rising Edge (CLK=1):");
    dff.set_input("CLK", true);
    dff.evaluate_stable();
    println!(
        "   Q={}, nQ={}",
        dff.get_output("Q").unwrap(),
        dff.get_output("nQ").unwrap()
    );

    // Change D while CLK is high. Q should *not* change (still 1).
    println!("4. Change D while CLK is High (CLK=1, D=0):");
    dff.set_input("D", false);
    dff.evaluate_stable();
    println!(
        "   Q={}, nQ={}",
        dff.get_output("Q").unwrap(),
        dff.get_output("nQ").unwrap()
    );

    // Pulse the clock: 1 -> 0 (Falling Edge)
    // The Master latch opens (capturing D=0), but the Slave latch closes,
    // *holding* the previous Q=1.
    println!("5. Clock Falling Edge (CLK=0):");
    dff.set_input("CLK", false);
    dff.evaluate_stable();
    println!(
        "   Q={}, nQ={}",
        dff.get_output("Q").unwrap(),
        dff.get_output("nQ").unwrap()
    );

    // Pulse the clock: 0 -> 1 (Rising Edge)
    // The Master latch (which holds 0) copies its value to the Slave.
    // Q should now update to 0.
    println!("6. Next Rising Edge (CLK=1):");
    dff.set_input("CLK", true);
    dff.evaluate_stable();
    println!(
        "   Q={}, nQ={}",
        dff.get_output("Q").unwrap(),
        dff.get_output("nQ").unwrap()
    );

    // --- Export DLatch_AND to DOT file ---
    if EXPORT_DLATCH {
        println!("\nExporting DLatch to dot file...");
        let dot_content = builder.export_to_dot("DLatch");
        match fs::write("DLatch.dot", dot_content) {
            Ok(_) => println!("Successfully wrote DLatch.dot"),
            Err(e) => eprintln!("Error writing .dot file: {}", e),
        }
    }
}
