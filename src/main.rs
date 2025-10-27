use std::path::Path;

mod circuit;
mod cldl;
use crate::{circuit::Circuit, cldl::CircuitBuilder};

// --- Helper Functions for 4-Bit XOR Tests ---

/// Sets the inputs for the 4-bit XOR unit.
fn set_xor4b_inputs(xor_unit: &Circuit, a: u8, b: u8) {
    assert!(a < 16, "Input 'a' must be < 16 for 4-bit XOR");
    assert!(b < 16, "Input 'b' must be < 16 for 4-bit XOR");

    xor_unit.set_input("A.0", (a & 0b0001) != 0);
    xor_unit.set_input("A.1", (a & 0b0010) != 0);
    xor_unit.set_input("A.2", (a & 0b0100) != 0);
    xor_unit.set_input("A.3", (a & 0b1000) != 0);

    xor_unit.set_input("B.0", (b & 0b0001) != 0);
    xor_unit.set_input("B.1", (b & 0b0010) != 0);
    xor_unit.set_input("B.2", (b & 0b0100) != 0);
    xor_unit.set_input("B.3", (b & 0b1000) != 0);
}

/// Gets the 4-bit output from the XOR unit.
fn get_xor4b_output(xor_unit: &Circuit) -> u8 {
    let out0 = xor_unit.get_output_unwrapped("Out.0");
    let out1 = xor_unit.get_output_unwrapped("Out.1");
    let out2 = xor_unit.get_output_unwrapped("Out.2");
    let out3 = xor_unit.get_output_unwrapped("Out.3");

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