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


// --- Helper Functions for 4-Bit Register Tests ---

/// Sets the data inputs (D0-D3) for the 4-bit register.
fn set_register4_data(register: &Circuit, data: u8) {
    assert!(data < 16, "Input data must be < 16 for 4-bit register");
    register.set_input("D0", (data & 0b0001) != 0);
    register.set_input("D1", (data & 0b0010) != 0);
    register.set_input("D2", (data & 0b0100) != 0);
    register.set_input("D3", (data & 0b1000) != 0);
}

/// Gets the Q and nQ outputs from the 4-bit register.
/// Returns (q_value: u8, nq_value: u8).
fn get_register4_outputs(register: &Circuit) -> (u8, u8) {
    let q0 = register.get_output_unwrapped("Q0");
    let q1 = register.get_output_unwrapped("Q1");
    let q2 = register.get_output_unwrapped("Q2");
    let q3 = register.get_output_unwrapped("Q3");

    let nq0 = register.get_output_unwrapped("nQ0");
    let nq1 = register.get_output_unwrapped("nQ1");
    let nq2 = register.get_output_unwrapped("nQ2");
    let nq3 = register.get_output_unwrapped("nQ3");

    let q_value = (q0 as u8) | ((q1 as u8) << 1) | ((q2 as u8) << 2) | ((q3 as u8) << 3);
    let nq_value = (nq0 as u8) | ((nq1 as u8) << 1) | ((nq2 as u8) << 2) | ((nq3 as u8) << 3);

    (q_value, nq_value)
}

/// Formats a 4-bit number for printing.
fn format_u4(n: u8) -> String {
    format!("{:04b}", n)
}

/// Pulses the clock (0 -> 1 -> 0) and evaluates stability.
fn pulse_clock(circuit: &Circuit) {
    circuit.set_input("CLK", true);  // Rising edge
    circuit.evaluate_stable();
    circuit.set_input("CLK", false); // Falling edge
    circuit.evaluate_stable();
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

    // --- 4-Bit Register Test ---
    println!("\nTesting 4-Bit Register:");
    let register4 = builder.build("Register4Bit");

    // The INIT block should have already run, setting Q=0000. Verify initial state.
    println!("1. Initial State after INIT (CLK=0):");
    let (q_val, nq_val) = get_register4_outputs(&register4);
    println!("   Q = {} ({}), nQ = {} ({})", format_u4(q_val), q_val, format_u4(nq_val), nq_val);
    assert_eq!(q_val, 0b0000, "Initial Q state incorrect");
    assert_eq!(nq_val, 0b1111, "Initial nQ state incorrect"); // nQ should be NOT Q

    // Set Data input to 5 (0101), keep CLK low. Output should not change.
    println!("2. Set D = 5 (0101), CLK=0:");
    set_register4_data(&register4, 0b0101);
    register4.evaluate_stable(); // Ensure combinational logic settles if any
    let (q_val, nq_val) = get_register4_outputs(&register4);
    println!("   Q = {} ({}), nQ = {} ({})", format_u4(q_val), q_val, format_u4(nq_val), nq_val);
    assert_eq!(q_val, 0b0000, "Q changed while CLK low");
    assert_eq!(nq_val, 0b1111, "nQ changed while CLK low");

    // Pulse the clock. Q should now latch the value 5 (0101).
    println!("3. Pulse CLK (Rising Edge latches D=5):");
    pulse_clock(&register4);
    let (q_val, nq_val) = get_register4_outputs(&register4);
    println!("   Q = {} ({}), nQ = {} ({})", format_u4(q_val), q_val, format_u4(nq_val), nq_val);
    assert_eq!(q_val, 0b0101, "Q did not latch D=5 on rising edge");
    assert_eq!(nq_val, 0b1010, "nQ is not NOT(Q) after latching 5"); // Check nQ = NOT(0101) = 1010

    // Change Data input to 10 (1010), keep CLK low. Output should hold 5.
    println!("4. Set D = 10 (1010), CLK=0:");
    set_register4_data(&register4, 0b1010);
    register4.evaluate_stable();
    let (q_val, nq_val) = get_register4_outputs(&register4);
    println!("   Q = {} ({}), nQ = {} ({})", format_u4(q_val), q_val, format_u4(nq_val), nq_val);
    assert_eq!(q_val, 0b0101, "Q changed while CLK low (should hold 5)");
    assert_eq!(nq_val, 0b1010, "nQ changed while CLK low (should hold NOT 5)");

    // Pulse the clock again. Q should now latch the value 10 (1010).
    println!("5. Pulse CLK (Rising Edge latches D=10):");
    pulse_clock(&register4);
    let (q_val, nq_val) = get_register4_outputs(&register4);
    println!("   Q = {} ({}), nQ = {} ({})", format_u4(q_val), q_val, format_u4(nq_val), nq_val);
    assert_eq!(q_val, 0b1010, "Q did not latch D=10 on rising edge");
    assert_eq!(nq_val, 0b0101, "nQ is not NOT(Q) after latching 10"); // Check nQ = NOT(1010) = 0101

    println!("\nRegister tests passed! ✅");
}