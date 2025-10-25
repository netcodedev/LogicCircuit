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


// --- Helper Functions for 4-Bit XOR Tests ---

/// Sets the inputs for the 4-bit XOR unit.
fn set_xor4b_inputs(xor_unit: &Circuit, a: u8, b: u8) {
    assert!(a < 16, "Input 'a' must be < 16 for 4-bit XOR");
    assert!(b < 16, "Input 'b' must be < 16 for 4-bit XOR");

    xor_unit.set_input("A0", (a & 0b0001) != 0);
    xor_unit.set_input("A1", (a & 0b0010) != 0);
    xor_unit.set_input("A2", (a & 0b0100) != 0);
    xor_unit.set_input("A3", (a & 0b1000) != 0);

    xor_unit.set_input("B0", (b & 0b0001) != 0);
    xor_unit.set_input("B1", (b & 0b0010) != 0);
    xor_unit.set_input("B2", (b & 0b0100) != 0);
    xor_unit.set_input("B3", (b & 0b1000) != 0);
}

/// Gets the 4-bit output from the XOR unit.
fn get_xor4b_output(xor_unit: &Circuit) -> u8 {
    let out0 = xor_unit.get_output_unwrapped("Out0");
    let out1 = xor_unit.get_output_unwrapped("Out1");
    let out2 = xor_unit.get_output_unwrapped("Out2");
    let out3 = xor_unit.get_output_unwrapped("Out3");

    (out0 as u8) | ((out1 as u8) << 1) | ((out2 as u8) << 2) | ((out3 as u8) << 3)
}

/// Formats a 4-bit number for printing.
fn format_u4(n: u8) -> String {
    format!("{:04b}", n)
}

/// Runs a single test case for the 4-bit XOR unit.
fn test_xor4b_case(xor_unit: &Circuit, a: u8, b: u8) {
    set_xor4b_inputs(xor_unit, a, b);
    xor_unit.evaluate_stable();
    let out = get_xor4b_output(xor_unit);

    // Calculate expected result using Rust's bitwise XOR
    let expected_out = a ^ b;

    println!(
        "  {} XOR {} = {} [Expected: {}] {}",
        format_u4(a), format_u4(b),
        format_u4(out),
        format_u4(expected_out),
        if out == expected_out { "✅" } else { "❌ ERROR" }
    );
    assert_eq!(out, expected_out, "XOR4B output mismatch for A={}, B={}", a, b);
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

    // --- 4-Bit XOR Unit Test ---
    println!("\nTesting 4-Bit Bitwise XOR Unit:");
    let xor4b = builder.build("XOR_4B");

    // Test cases (A, B)
    test_xor4b_case(&xor4b, 0b0000, 0b0000); // 0 ^ 0 = 0
    test_xor4b_case(&xor4b, 0b1111, 0b0000); // 15 ^ 0 = 15
    test_xor4b_case(&xor4b, 0b0000, 0b1111); // 0 ^ 15 = 15
    test_xor4b_case(&xor4b, 0b1111, 0b1111); // 15 ^ 15 = 0
    test_xor4b_case(&xor4b, 0b0101, 0b1010); // 5 ^ 10 = 15 (1111)
    test_xor4b_case(&xor4b, 0b1100, 0b1010); // 12 ^ 10 = 6 (0110)
    test_xor4b_case(&xor4b, 0b0111, 0b1110); // 7 ^ 14 = 9 (1001)
    test_xor4b_case(&xor4b, 0b1011, 0b1101); // 11 ^ 13 = 6 (0110)
    test_xor4b_case(&xor4b, 0b1001, 0b1001); // 9 ^ 9 = 0 (0000)

    println!("\nXOR_4B tests passed! ✅");
}