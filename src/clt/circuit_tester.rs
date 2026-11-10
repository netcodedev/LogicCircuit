use pest::{Parser, iterators::Pair};
use std::fs;
use std::path::Path;

// --- Use super to access items in src/clt/mod.rs ---
use super::{
    CircuitTester,
    CltCommand,
    CltFile,
    CltParser,
    CltPinRef,
    CltPinValue,
    CltSeqStep,
    CltTestCase,
    CltTestSuite,
    CltValue,
    Rule, // Assuming Rule is imported or defined
};
// --- Import items from other modules ---
use crate::circuit::Circuit;
use crate::cldl::CircuitBuilder; // To build circuits
use crate::clt::TestSummary; // The circuit object itself

// --- Parsing Helper Functions ---

/// Parses a pest 'pin' rule from CLT grammar into CltPinRef
fn parse_clt_pin_ref(pair: Pair<Rule>) -> CltPinRef {
    assert_eq!(pair.as_rule(), Rule::pin);
    let mut _base_name = String::new();
    let mut index = None;
    let full_pin_str = pair.as_str().trim();

    if let Some(bracket_pos) = full_pin_str.find('[') {
        if full_pin_str.ends_with(']') {
            let num_str = &full_pin_str[bracket_pos + 1..full_pin_str.len() - 1];
            index = Some(
                num_str
                    .parse::<usize>()
                    .expect("CLT Grammar ensures valid number in specifier"),
            );
            _base_name = full_pin_str[..bracket_pos].trim().to_string();
        } else {
            _base_name = full_pin_str.to_string(); // Malformed, treat as name
        }
    } else {
        _base_name = full_pin_str.to_string(); // No index
    }

    if _base_name.is_empty() {
        panic!(
            "Failed to parse CLT pin, empty base_name for: {}",
            full_pin_str
        );
    }
    CltPinRef {
        base_name: _base_name,
        index,
    }
}

/// Parses a pest 'pin_value' rule from CLT grammar into CltPinValue
fn parse_clt_pin_value(pair: Pair<Rule>) -> CltPinValue {
    assert_eq!(pair.as_rule(), Rule::pin_value);
    let mut inner = pair.into_inner();
    let pin_ref = parse_clt_pin_ref(inner.next().unwrap());
    let value_str = inner.next().unwrap().as_str();
    let value = match value_str {
        "0" => CltValue::Zero,
        "1" => CltValue::One,
        _ => unreachable!("CLT Grammar ensures value is 0 or 1"),
    };
    CltPinValue { pin_ref, value }
}

/// Parses an assignment (`pin_value` or `bus_value`) into one CltPinValue
/// per bit. A bus value `A[hi:lo] v` sets A[lo] to bit 0 of v, and so on.
fn parse_clt_assignment(pair: Pair<Rule>) -> Vec<CltPinValue> {
    if pair.as_rule() == Rule::pin_value {
        return vec![parse_clt_pin_value(pair)];
    }
    assert_eq!(pair.as_rule(), Rule::bus_value);
    let mut inner = pair.into_inner();
    let pin = inner.next().unwrap().as_str();
    let number = inner.next().unwrap().as_str().replace('_', "");
    let open = pin.rfind('[').unwrap();
    let (base, range) = (&pin[..open], &pin[open + 1..pin.len() - 1]);
    let (hi, lo) = range.split_once(':').unwrap();
    let (hi, lo): (usize, usize) = (hi.trim().parse().unwrap(), lo.trim().parse().unwrap());
    assert!(hi >= lo && hi - lo < 128, "CLT: bad bus range in {pin} (at most 128 bits)");
    let width = hi - lo + 1;
    let bad = |e: String| -> ! { panic!("CLT: bad value {number} for {pin}: {e}") };
    let radix = |digits: &str, r: u32| u128::from_str_radix(digits, r).unwrap_or_else(|e| bad(e.to_string()));
    // The value as bits (two's complement for negative numbers).
    let bits: u128 = if let Some(h) = number.strip_prefix("0x").or_else(|| number.strip_prefix("0X")) {
        radix(h, 16)
    } else if let Some(b) = number.strip_prefix("0b").or_else(|| number.strip_prefix("0B")) {
        radix(b, 2)
    } else if let Some(d) = number.strip_prefix('-') {
        let m = radix(d, 10);
        if m > 1u128 << (width - 1) {
            bad(format!("does not fit {width} bits"));
        }
        m.wrapping_neg()
    } else {
        radix(&number, 10)
    };
    let unsigned_fits = width == 128 || bits >> width == 0;
    let negative = number.starts_with('-');
    if !negative && !unsigned_fits {
        bad(format!("does not fit {width} bits"));
    }
    (0..width)
        .map(|k| CltPinValue {
            pin_ref: CltPinRef { base_name: base.to_string(), index: Some(lo + k) },
            value: if bits >> k & 1 == 1 { CltValue::One } else { CltValue::Zero },
        })
        .collect()
}

/// Parses a pest 'seq_step' rule from CLT grammar into CltSeqStep
fn parse_clt_seq_step(pair: Pair<Rule>) -> CltSeqStep {
    assert_eq!(pair.as_rule(), Rule::seq_step);
    let assignments = pair.into_inner().flat_map(parse_clt_assignment).collect();
    CltSeqStep { assignments }
}

/// Parses a pest 'test_command' rule into CltCommand variants (an ASSERT of
/// a bus becomes one ASSERT per bit).
fn parse_clt_command(pair: Pair<Rule>) -> Vec<CltCommand> {
    assert_eq!(pair.as_rule(), Rule::test_command);
    let command_pair = pair.into_inner().next().unwrap(); // Get the specific command (set, eval, etc.)
    match command_pair.as_rule() {
        Rule::set_command => {
            let assignments = command_pair.into_inner().flat_map(parse_clt_assignment).collect();
            vec![CltCommand::Set(assignments)]
        }
        Rule::eval_command => vec![CltCommand::Eval],
        Rule::assert_command => {
            command_pair.into_inner().flat_map(parse_clt_assignment).map(CltCommand::Assert).collect()
        }
        Rule::seq_command => {
            let steps = command_pair
                .into_inner()
                .filter(|p| p.as_rule() == Rule::seq_step)
                .map(parse_clt_seq_step)
                .collect();
            vec![CltCommand::Seq(steps)]
        }
        _ => unreachable!(
            "Unexpected rule inside test_command: {:?}",
            command_pair.as_rule()
        ),
    }
}

/// Parses a pest 'test_case' rule into a CltTestCase struct
fn parse_clt_test_case(pair: Pair<Rule>) -> CltTestCase {
    assert_eq!(pair.as_rule(), Rule::test_case);
    let mut inner = pair.into_inner();
    // TEST "Name" { body }
    let name_str_literal = inner.next().unwrap().as_str();
    // Remove surrounding quotes from the string literal
    let name = name_str_literal[1..name_str_literal.len() - 1].to_string();
    let body_pair = inner.next().unwrap();
    assert_eq!(body_pair.as_rule(), Rule::test_body);
    let body = body_pair.into_inner().flat_map(parse_clt_command).collect();
    CltTestCase { name, body }
}

/// Parses a pest 'test_suite' rule into a CltTestSuite struct
fn parse_clt_test_suite(pair: Pair<Rule>) -> CltTestSuite {
    assert_eq!(pair.as_rule(), Rule::test_suite);
    let mut inner = pair.into_inner();
    // TESTS FOR CircuitName { test_case* }
    // Drop spaces so "ADDER< 8 >" names the definition "ADDER<8>".
    let circuit_name: String = inner.next().unwrap().as_str().chars().filter(|c| !c.is_whitespace()).collect();
    let test_cases = inner.map(parse_clt_test_case).collect();
    CltTestSuite {
        circuit_name,
        test_cases,
    }
}

/// Parses the entire content of a .clt file string into a CltFile struct
pub(super) fn parse_clt_file(file_content: &str) -> Result<CltFile, pest::error::Error<Rule>> {
    let mut file_pairs = CltParser::parse(Rule::file, file_content.trim())?;
    let file_pair = file_pairs.next().unwrap(); // Rule::file
    let suites = file_pair
        .into_inner()
        .filter(|p| p.as_rule() == Rule::test_suite) // Filter out EOI etc.
        .map(parse_clt_test_suite)
        .collect();
    Ok(CltFile { suites })
}

// --- Test Execution Logic ---

impl CircuitTester {
    /// Creates a new CircuitTester.
    pub fn new() -> Self {
        CircuitTester { nand: false }
    }

    /// Finds, parses, and runs all tests in .clt files within a directory.
    pub fn run_tests_in_dir(
        &self,
        dir_path: &Path,
        builder: &CircuitBuilder, // Pass builder to construct circuits
    ) -> Result<TestSummary, String> {
        if !dir_path.is_dir() {
            return Err(format!("Test directory not found: {:?}", dir_path));
        }
        println!("\n--- Running Tests in {:?} ---", dir_path);

        let mut summary = TestSummary::new();

        // Use walkdir crate for easier recursive traversal, or std::fs::read_dir recursively
        match self.process_directory_recursively(dir_path, builder, &mut summary) {
            Ok(_) => {
                println!("--- Test Summary ---");
                println!("  Passed: {}", summary.passed);
                println!("  Failed: {}", summary.failed);
                Ok(summary)
            }
            Err(e) => Err(e),
        }
    }

    // Helper for recursive directory processing
    fn process_directory_recursively(
        &self,
        dir_path: &Path,
        builder: &CircuitBuilder,
        summary: &mut TestSummary,
    ) -> Result<(), String> {
        for entry in fs::read_dir(dir_path)
            .map_err(|e| format!("Failed to read directory {:?}: {}", dir_path, e))?
        {
            let entry = entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;
            let path = entry.path();
            if path.is_dir() {
                self.process_directory_recursively(&path, builder, summary)?;
            } else if path.is_file() && path.extension().map_or(false, |ext| ext == "clt") {
                println!("\nFound test file: {:?}", path);
                let content = fs::read_to_string(&path)
                    .map_err(|e| format!("Failed to read test file {:?}: {}", path, e))?;

                match self.run_test_file_content(&content, builder, summary) {
                    Ok(_) => {} // Success recorded in summary
                    Err(e) => {
                        eprintln!("  ERROR processing file {:?}: {}", path, e);
                        // Decide if a file-level error counts as a test failure
                        // summary.failed += 1; // Or maybe just report error and continue
                    }
                }
            }
        }
        Ok(())
    }

    /// Parses and runs tests defined in a single .clt file content string.
    pub fn run_test_file_content(
        &self,
        file_content: &str,
        builder: &CircuitBuilder,
        summary: &mut TestSummary,
    ) -> Result<(), String> {
        let clt_file =
            parse_clt_file(file_content).map_err(|e| format!("CLT parsing error: {}", e))?;

        for suite in clt_file.suites {
            println!("  Testing Circuit: {}", suite.circuit_name);
            // Build the circuit *once* per suite
            let circuit = if self.nand {
                builder.build_nand(&suite.circuit_name)
            } else {
                builder.build(&suite.circuit_name)
            };

            for test_case in suite.test_cases {
                println!("    Test Case: {}", test_case.name);
                let result = self.execute_test_case(&circuit, &test_case);
                match result {
                    Ok(_) => {
                        println!("      Result: ✅ Passed");
                        summary.passed += 1;
                    }
                    Err(failure_msg) => {
                        println!("      Result: ❌ Failed");
                        println!("        Reason: {}", failure_msg);
                        summary.failed += 1;
                    }
                }
            }
        }
        Ok(())
    }

    /// Executes the commands within a single test case.
    fn execute_test_case(&self, circuit: &Circuit, test_case: &CltTestCase) -> Result<(), String> {
        for (cmd_index, command) in test_case.body.iter().enumerate() {
            match command {
                CltCommand::Set(assignments) => {
                    for pv in assignments {
                        let value = match pv.value {
                            CltValue::Zero => false,
                            CltValue::One => true,
                        };
                        let pin_key = pv.pin_ref.to_canonical_key();
                        // Use Circuit's set_input - assumes it handles canonical names
                        circuit.set_input(&pin_key, value);
                    }
                }
                CltCommand::Eval => {
                    circuit.evaluate_stable();
                }
                CltCommand::Assert(assertion) => {
                    let expected_value = match assertion.value {
                        CltValue::Zero => false,
                        CltValue::One => true,
                    };
                    let pin_key = assertion.pin_ref.to_canonical_key();
                    // Use Circuit's get_output_unwrapped - assumes it handles canonical names
                    // Note: This will panic if the output doesn't exist.
                    // A more robust runner might use get_output and return Err on None.
                    let actual_value = circuit.get_wire_value(&pin_key);

                    if actual_value != expected_value {
                        circuit.print_io_state();
                        return Err(format!(
                            "Assertion failed at command index {}: Expected output '{}' to be {}, but was {}",
                            cmd_index, pin_key, expected_value as u8, actual_value as u8
                        ));
                    }
                }
                CltCommand::Seq(steps) => {
                    for (_step_index, step) in steps.iter().enumerate() {
                        // Set assignments for the step
                        for pv in &step.assignments {
                            let value = match pv.value {
                                CltValue::Zero => false,
                                CltValue::One => true,
                            };
                            let pin_key = pv.pin_ref.to_canonical_key();
                            circuit.set_input(&pin_key, value);
                        }
                        // Evaluate after each step in the sequence
                        circuit.evaluate_stable();
                        // Debug print (optional)
                        // println!("      [Seq Step {} completed]", step_index);
                    }
                }
            }
        }
        Ok(()) // If all commands executed without assertion error
    }
}

impl TestSummary {
    pub fn new() -> Self {
        Default::default()
    }
}
