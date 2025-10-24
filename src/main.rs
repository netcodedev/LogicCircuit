use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::rc::Rc;
use uuid::Uuid;

mod cldl;
use crate::cldl::CircuitBuilder;
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


// --- Helper Functions for Mux2To1 Tests ---

/// Runs a single test case for the Mux2To1.
fn test_mux2to1_case(mux: &Circuit, a: bool, b: bool, s: bool) {
    mux.set_input("A", a);
    mux.set_input("B", b);
    mux.set_input("S", s);
    mux.evaluate_stable();
    let out = mux.get_output_unwrapped("Out");

    // Calculate expected result
    let expected_out = if s { b } else { a };

    println!(
        "  Mux(A={}, B={}, S={}) -> Out={} [Expected: {}] {}",
        a as u8, b as u8, s as u8,
        out as u8,
        expected_out as u8,
        if out == expected_out { "✅" } else { "❌ ERROR" }
    );
    assert_eq!(out, expected_out, "Mux output mismatch for A={}, B={}, S={}", a, b, s);
}
// --- End Helper Functions ---


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

    // --- Mux 2-to-1 Test ---
    println!("\nTesting 2-to-1 Multiplexer:");
    let mux = builder.build("Mux2To1");

    // Test cases (A, B, S)
    println!("Testing S=0 (Select A):");
    test_mux2to1_case(&mux, false, false, false); // S=0, A=0 -> Out=0
    test_mux2to1_case(&mux, true, false, false);  // S=0, A=1 -> Out=1
    test_mux2to1_case(&mux, false, true, false);  // S=0, A=0 -> Out=0 (B is ignored)
    test_mux2to1_case(&mux, true, true, false);   // S=0, A=1 -> Out=1 (B is ignored)

    println!("Testing S=1 (Select B):");
    test_mux2to1_case(&mux, false, false, true);  // S=1, B=0 -> Out=0
    test_mux2to1_case(&mux, true, false, true);   // S=1, B=0 -> Out=0 (A is ignored)
    test_mux2to1_case(&mux, false, true, true);   // S=1, B=1 -> Out=1
    test_mux2to1_case(&mux, true, true, true);    // S=1, B=1 -> Out=1 (A is ignored)

    println!("\nMux tests passed! ✅");
}
