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

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(cmds: &[CltPinValue]) -> Vec<(String, bool)> {
        cmds.iter().map(|p| (p.pin_ref.to_canonical_key(), p.value == CltValue::One)).collect()
    }

    #[test]
    fn bus_values_expand_to_bits() {
        let f = parse_clt(
            "TESTS FOR X { TEST \"t\" { SET A[3:0] 0xA, B[5:4] -1, C 1; ASSERT S[2:0] 0b011; SEQ D[1:0] 2 -> CLK 1; } }",
        )
        .unwrap();
        let body = &f.suites[0].test_cases[0].body;
        let CltCommand::Set(set) = &body[0] else { panic!() };
        let want = [("A.0", false), ("A.1", true), ("A.2", false), ("A.3", true), ("B.4", true), ("B.5", true), ("C", true)];
        assert_eq!(bits(set), want.map(|(k, v)| (k.to_string(), v)));
        let asserts: Vec<_> = body[1..4].iter().map(|c| match c {
            CltCommand::Assert(p) => (p.pin_ref.to_canonical_key(), p.value == CltValue::One),
            _ => panic!(),
        }).collect();
        assert_eq!(asserts, [("S.0".into(), true), ("S.1".into(), true), ("S.2".into(), false)]);
        let CltCommand::Seq(steps) = &body[4] else { panic!() };
        assert_eq!(bits(&steps[0].assignments), [("D.0".into(), false), ("D.1".into(), true)]);
    }

    #[test]
    fn wide_bus_values() {
        let f = parse_clt("TESTS FOR X { TEST \"t\" { SET W[127:0] 0x8000_0000_0000_0000_0000_0000_0000_0001; } }").unwrap();
        let CltCommand::Set(set) = &f.suites[0].test_cases[0].body[0] else { panic!() };
        let ones: Vec<String> = bits(set).into_iter().filter(|(_, v)| *v).map(|(k, _)| k).collect();
        assert_eq!(set.len(), 128);
        assert_eq!(ones, ["W.0", "W.127"]);
    }

    #[test]
    #[should_panic]
    fn bus_value_must_fit() {
        let _ = parse_clt("TESTS FOR X { TEST \"t\" { SET A[3:0] 0x10; } }");
    }
}
