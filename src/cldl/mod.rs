use std::collections::HashMap;
use include_dir::{Dir, include_dir};

mod circuit_builder;


#[cfg(debug_assertions)]
const _GRAMMAR: &str = include_str!("cldl.pest");

// Statically include the circuits directory at compile time
static CIRCUIT_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/src/circuits");

#[derive(pest_derive::Parser)]
#[grammar = "./cldl/cldl.pest"]
pub struct CldlParser;

// --- CLDL AST (Intermediate Structs) ---

/// Represents an identifier that might define a bus width.
/// e.g., "A" (width=None) or "Data[4]" (width=Some(4))
#[derive(Debug, Clone)]
pub struct IoPinDef {
    name: String,
    width: Option<usize>,
}

/// Represents a pin reference, potentially indexed.
/// e.g., "Clk" (index=None), "Data[2]" (index=Some(2)), "RegA.Q[0]" (base_name="RegA.Q", index=Some(0))
#[derive(Debug, Clone)]
pub struct PinRef {
    base_name: String, // Full path *before* index, e.g., "A", "G1.out", "RegA.Q"
    index: Option<usize>,
}

impl PinRef {
    /// Generates the canonical string representation used in wire_map keys.
    /// e.g., "A" -> "A", "Data[2]" -> "Data.2", "RegA.Q[0]" -> "RegA.Q.0"
    fn to_wire_map_key(&self) -> String {
        match self.index {
            Some(idx) => format!("{}.{}", self.base_name, idx),
            None => self.base_name.clone(),
        }
    }
}


/// e.g., "PART G1: NAND"
#[derive(Debug, Clone)]
pub struct PartDef {
    instance_name: String,
    part_type: String,
}

/// e.g., "CONNECT a -> G1.in1" or "CONNECT BusA[0] -> G1.in1"
#[derive(Debug, Clone)]
pub struct ConnectionDef {
    source_pin: PinRef,
    dest_pin: PinRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitValue {
    Zero,
    One,
}

/// e.g., "D 0" or "BusA[1] 1"
#[derive(Debug, Clone)]
pub struct PinValue {
    pin_ref: PinRef,
    value: InitValue,
}

#[derive(Debug, Clone)]
pub struct SeqStep {
    assignments: Vec<PinValue>,
}

#[derive(Debug, Clone)]
pub enum InitCommand {
    Set(Vec<PinValue>),
    Eval,
    Seq(Vec<SeqStep>),
}

/// Represents the entire definition of one CIRCUIT block.
#[derive(Debug, Clone)]
pub struct CldlDefinition {
    name: String,
    inputs: Vec<IoPinDef>,
    outputs: Vec<IoPinDef>,
    parts: Vec<PartDef>,
    connections: Vec<ConnectionDef>,
    initialization: Vec<InitCommand>,
}

/// This struct holds all known circuit definitions and can build them.
pub struct CircuitBuilder {
    /// A map from circuit name (e.g., "XOR") to its parsed definition.
    definitions: HashMap<String, CldlDefinition>,
}