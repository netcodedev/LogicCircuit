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

/// Sets the inputs for the 4-bit adder.
/// a, b are u8 values (0-15), cin is bool.
fn set_adder4_inputs(adder: &Circuit, a: u8, b: u8, cin: bool) {
    assert!(a < 16, "Input 'a' must be < 16 for 4-bit adder");
    assert!(b < 16, "Input 'b' must be < 16 for 4-bit adder");

    adder.set_input("A0", (a & 0b0001) != 0);
    adder.set_input("A1", (a & 0b0010) != 0);
    adder.set_input("A2", (a & 0b0100) != 0);
    adder.set_input("A3", (a & 0b1000) != 0);

    adder.set_input("B0", (b & 0b0001) != 0);
    adder.set_input("B1", (b & 0b0010) != 0);
    adder.set_input("B2", (b & 0b0100) != 0);
    adder.set_input("B3", (b & 0b1000) != 0);

    adder.set_input("Cin", cin);
}

/// Gets the outputs from the 4-bit adder.
/// Returns (sum: u8, cout: bool).
fn get_adder4_outputs(adder: &Circuit) -> (u8, bool) {
    let s0 = adder.get_output("S0").expect(&format!("Output 'S0' not found in circuit 'Adder4Bit'"));
    let s1 = adder.get_output("S1").expect(&format!("Output 'S1' not found in circuit 'Adder4Bit'"));
    let s2 = adder.get_output("S2").expect(&format!("Output 'S2' not found in circuit 'Adder4Bit'"));
    let s3 = adder.get_output("S3").expect(&format!("Output 'S3' not found in circuit 'Adder4Bit'"));
    let cout = adder.get_output("Cout").expect(&format!("Output 'Cout' not found in circuit 'Adder4Bit'"));

    let sum = (s0 as u8) | ((s1 as u8) << 1) | ((s2 as u8) << 2) | ((s3 as u8) << 3);

    (sum, cout)
}

/// Formats a 4-bit number for printing.
fn format_u4(n: u8) -> String {
    format!("{:04b}", n)
}

/// Runs a single test case for the 4-bit adder.
fn test_adder4_case(adder: &Circuit, a: u8, b: u8, cin: bool) {
    set_adder4_inputs(adder, a, b, cin);
    adder.evaluate_stable();
    let (sum, cout) = get_adder4_outputs(adder);

    // Calculate expected result using standard Rust arithmetic
    let expected_result = a as u16 + b as u16 + cin as u16;
    let expected_sum = (expected_result & 0b1111) as u8;
    let expected_cout = (expected_result & 0b10000) != 0;

    println!(
        "  {} + {} + {} = Sum: {} (Cout: {}) [Expected: Sum={}, Cout={}] {}",
        format_u4(a), format_u4(b), cin as u8,
        format_u4(sum), cout as u8,
        format_u4(expected_sum), expected_cout as u8,
        if sum == expected_sum && cout == expected_cout { "✅" } else { "❌ ERROR" }
    );
    assert_eq!(sum, expected_sum, "Sum mismatch for {} + {} + {}", a, b, cin);
    assert_eq!(cout, expected_cout, "Cout mismatch for {} + {} + {}", a, b, cin);
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

    // --- 4-Bit Adder Test ---
    println!("\nTesting 4-Bit Adder:");
    let adder4 = builder.build("Adder4Bit");

    // Test cases (A + B + Cin)
    test_adder4_case(&adder4, 0, 0, false); // 0000 + 0000 + 0 = 0000 (C=0)
    test_adder4_case(&adder4, 1, 0, false); // 0001 + 0000 + 0 = 0001 (C=0)
    test_adder4_case(&adder4, 1, 1, false); // 0001 + 0001 + 0 = 0010 (C=0)
    test_adder4_case(&adder4, 2, 3, false); // 0010 + 0011 + 0 = 0101 (C=0)
    test_adder4_case(&adder4, 7, 1, false); // 0111 + 0001 + 0 = 1000 (C=0) -> Tests carry ripple
    test_adder4_case(&adder4, 8, 8, false); // 1000 + 1000 + 0 = 0000 (C=1) -> Tests carry out
    test_adder4_case(&adder4, 15, 1, false); // 1111 + 0001 + 0 = 0000 (C=1)
    test_adder4_case(&adder4, 15, 15, false); // 1111 + 1111 + 0 = 1110 (C=1)
    test_adder4_case(&adder4, 5, 5, true);  // 0101 + 0101 + 1 = 1011 (C=0) -> Tests carry in
    test_adder4_case(&adder4, 8, 7, true);  // 1000 + 0111 + 1 = 0000 (C=1) -> Tests carry in and out
    test_adder4_case(&adder4, 15, 15, true); // 1111 + 1111 + 1 = 1111 (C=1)
}