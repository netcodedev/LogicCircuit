use pest_derive::Parser;

mod circuit_tester;

/// Parses the content of a .clt file.
pub fn parse_clt(content: &str) -> Result<CltFile, String> {
    circuit_tester::parse_clt_file(content).map_err(|e| e.to_string())
}

// --- CLT AST (Intermediate Structs) ---

/// Represents a pin reference from CLT, potentially indexed.
/// e.g., "Clk", "Data[2]", "RegA.Q[0]"
#[derive(Debug, Clone, PartialEq, Eq, Hash)] // Added PartialEq, Eq, Hash for potential use in HashMaps
pub struct CltPinRef {
    pub base_name: String, // Full path *before* index
    pub index: Option<usize>,
}

impl CltPinRef {
    /// Generates the canonical string representation used internally.
    /// Matches the format used by CircuitBuilder's wire_map.
    /// e.g., "A" -> "A", "Data[2]" -> "Data.2", "RegA.Q[0]" -> "RegA.Q.0"
    pub fn to_canonical_key(&self) -> String {
        match self.index {
            Some(idx) => format!("{}.{}", self.base_name, idx),
            None => self.base_name.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CltValue {
    Zero,
    One,
}

/// Represents a pin assignment or assertion value.
/// e.g., "D 0", "BusA[1] 1"
#[derive(Debug, Clone)]
pub struct CltPinValue {
    pub pin_ref: CltPinRef,
    pub value: CltValue,
}

/// Represents a step within a SEQ command (one or more assignments).
#[derive(Debug, Clone)]
pub struct CltSeqStep {
    pub assignments: Vec<CltPinValue>,
}

/// Represents a single command within a test case.
#[derive(Debug, Clone)]
pub enum CltCommand {
    Set(Vec<CltPinValue>),
    Eval,
    Assert(CltPinValue), // Assert takes a single pin value
    Seq(Vec<CltSeqStep>),
}

/// Represents a single named test case.
#[derive(Debug, Clone)]
pub struct CltTestCase {
    pub name: String, // Name from the string literal
    pub body: Vec<CltCommand>,
}

/// Represents a collection of test cases for a specific circuit.
#[derive(Debug, Clone)]
pub struct CltTestSuite {
    pub circuit_name: String, // Name of the circuit being tested
    pub test_cases: Vec<CltTestCase>,
}

/// Represents the entire content of a parsed .clt file.
#[derive(Debug, Clone)]
pub struct CltFile {
    pub suites: Vec<CltTestSuite>,
}

// --- CLT Parser ---

#[derive(Parser)]
#[grammar = "./clt/clt.pest"] // Point to the CLT grammar file
pub struct CltParser;

/// This struct handles testing circuits
pub struct CircuitTester {
    /// Build every circuit from NAND gates only (`CircuitBuilder::build_nand`)
    /// instead of with the built-in gates.
    pub nand: bool,
}

// Simple struct to hold test results
#[derive(Debug, Default)]
pub struct TestSummary {
    pub passed: u32,
    pub failed: u32,
}
