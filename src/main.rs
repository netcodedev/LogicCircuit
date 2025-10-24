use std::path::Path;
use std::rc::Rc;
use std::cell::RefCell;
use std::collections::HashMap;
use uuid::Uuid;
use std::fs;
use std::fmt;

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

    // --- D Latch Test ---
    println!("\nTesting D Latch:");
    let d_latch = builder.build("DLatch");

    println!("1. Enable Latch to Set (Store=1, Data=1):");
    d_latch.set_input("Store", true);
    d_latch.set_input("Data", true);
    d_latch.evaluate_stable();
    println!(
        "   Q={}, nQ={}",
        d_latch.get_output("Q").unwrap(),
        d_latch.get_output("nQ").unwrap()
    );

    // Test 2: Hold State (E=0)
    // Latch closes. Q should "remember" its 1 state, even though D changes to 0.
    println!("2. Hold State (Store=0, Data=0):");
    d_latch.set_input("Store", false);
    d_latch.set_input("Data", false);
    d_latch.evaluate_stable();
    println!(
        "   Q={}, nQ={}",
        d_latch.get_output("Q").unwrap(),
        d_latch.get_output("nQ").unwrap()
    );

    // Test 3: Enable Latch to Reset (E=1, D=0)
    // Latch becomes transparent. Q should follow D and become 0.
    println!("3. Enable Latch to Reset (Store=1, Data=0):");
    d_latch.set_input("Store", true);
    d_latch.set_input("Data", false);
    d_latch.evaluate_stable();
    println!(
        "   Q={}, nQ={}",
        d_latch.get_output("Q").unwrap(),
        d_latch.get_output("nQ").unwrap()
    );

    // Test 4: Hold State (E=0)
    // Latch closes. Q should "remember" its 0 state, even though D changes to 1.
    println!("4. Hold State (Store=0, Data=1):");
    d_latch.set_input("Store", false);
    d_latch.set_input("Data", true);
    d_latch.evaluate_stable();
    // println!("STATE: {:?}", d_latch);
    println!(
        "   Q={}, nQ={}",
        d_latch.get_output("Q").unwrap(),
        d_latch.get_output("nQ").unwrap()
    );

    // Second cycle
    println!("1. Enable Latch to Set (Store=1, Data=1):");
    d_latch.set_input("Store", true);
    d_latch.set_input("Data", true);
    d_latch.evaluate_stable();
    println!(
        "   Q={}, nQ={}",
        d_latch.get_output("Q").unwrap(),
        d_latch.get_output("nQ").unwrap()
    );

    // Test 2: Hold State (E=0)
    // Latch closes. Q should "remember" its 1 state, even though D changes to 0.
    println!("2. Hold State (Store=0, Data=0):");
    d_latch.set_input("Store", false);
    d_latch.set_input("Data", false);
    d_latch.evaluate_stable();
    println!(
        "   Q={}, nQ={}",
        d_latch.get_output("Q").unwrap(),
        d_latch.get_output("nQ").unwrap()
    );

    // Test 3: Enable Latch to Reset (E=1, D=0)
    // Latch becomes transparent. Q should follow D and become 0.
    println!("3. Enable Latch to Reset (Store=1, Data=0):");
    d_latch.set_input("Store", true);
    d_latch.set_input("Data", false);
    d_latch.evaluate_stable();
    println!(
        "   Q={}, nQ={}",
        d_latch.get_output("Q").unwrap(),
        d_latch.get_output("nQ").unwrap()
    );

    // Test 4: Hold State (E=0)
    // Latch closes. Q should "remember" its 0 state, even though D changes to 1.
    println!("4. Hold State (Store=0, Data=1):");
    d_latch.set_input("Store", false);
    d_latch.set_input("Data", true);
    d_latch.evaluate_stable();
    // println!("STATE: {:?}", d_latch);
    println!(
        "   Q={}, nQ={}",
        d_latch.get_output("Q").unwrap(),
        d_latch.get_output("nQ").unwrap()
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

