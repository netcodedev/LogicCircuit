use include_dir::{include_dir, Dir, DirEntry};
use std::{collections::HashMap, fs, path::Path};

use pest::{Parser, iterators::Pair};
// Import LogicGate trait
use crate::{Circuit, NandGate, Wire, wire};

#[cfg(debug_assertions)]
const _GRAMMAR: &str = include_str!("cldl.pest");

#[derive(pest_derive::Parser)]
#[grammar = "cldl.pest"]
pub struct CldlParser;

// Statically include the circuits directory at compile time
static CIRCUIT_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/src/circuits");

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

// --- Parses a pest 'pin' rule into our PinRef struct ---
fn parse_pin_ref(pair: Pair<Rule>) -> PinRef {
    assert_eq!(pair.as_rule(), Rule::pin);
    // pin = @{ ident ~ ("." ~ ident)* ~ bus_specifier? }

    let mut base_name = String::new();

    let full_pin_str = pair.as_str().trim(); // Get the full matched string

    // Cloning is simpler here as we need the span later.
    for inner_pair in pair.clone().into_inner() {
        match inner_pair.as_rule() {
            Rule::ident => {
                if !base_name.is_empty() {
                    base_name.push('.');
                }
                base_name.push_str(inner_pair.as_str().trim());
            }
            Rule::bus_specifier => {
            }
            _ => unreachable!("Unexpected rule inside pin: {:?}", inner_pair.as_rule()),
        }
    }

    let mut index = None; // Reset

    if let Some(bracket_pos) = full_pin_str.find('[') {
         if full_pin_str.ends_with(']') {
            let num_str = &full_pin_str[bracket_pos + 1 .. full_pin_str.len() - 1];
             index = Some(num_str.parse::<usize>().expect("Grammar ensures valid number in specifier"));
             base_name = full_pin_str[..bracket_pos].trim().to_string();
        } else {
             // Malformed (e.g., "A[0.B")? Assume bracket is part of the name for now.
             base_name = full_pin_str.to_string();
        }
    } else {
         // No brackets found, it's just a base name
         base_name = full_pin_str.to_string();
    }

    // Final check for empty base_name
    if base_name.is_empty() {
         panic!("Failed to parse pin, empty base_name for: {}", full_pin_str);
    }

    PinRef { base_name, index }
}


/// Helper to parse a single pin_value pair (e.g., "D 0", "Bus[0] 1")
fn parse_pin_value(pair: Pair<Rule>) -> PinValue {
    // pin_value = pin ~ value
    assert_eq!(pair.as_rule(), Rule::pin_value);
    let mut inner = pair.into_inner();
    let pin_ref = parse_pin_ref(inner.next().unwrap()); // Parse the 'pin' part
    let value_str = inner.next().unwrap().as_str();
    let value = match value_str {
        "0" => InitValue::Zero,
        "1" => InitValue::One,
        _ => unreachable!("Grammar ensures value is 0 or 1"),
    };
    PinValue { pin_ref, value }
}

/// Helper to parse a single step (one or more pin_values)
fn parse_step(pair: Pair<Rule>) -> SeqStep {
    // step = pin_value ~ ("," ~ pin_value)*
    assert_eq!(pair.as_rule(), Rule::step);
    let assignments = pair.into_inner().map(parse_pin_value).collect();
    SeqStep { assignments }
}

// --- Parses an io_ident rule into IoPinDef ---
fn parse_io_ident(pair: Pair<Rule>) -> IoPinDef {
    assert_eq!(pair.as_rule(), Rule::io_ident);
    let mut name = String::new();
    let mut width = None;
    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::ident => {
                name = inner_pair.as_str().trim().to_string();
            }
            Rule::bus_specifier => {
                let num_str = inner_pair.into_inner().next().unwrap().as_str();
                width = Some(num_str.parse::<usize>().expect("Grammar ensures valid number"));
            }
            _ => unreachable!("Unexpected rule inside io_ident: {:?}", inner_pair.as_rule()),
        }
    }
    IoPinDef { name, width }
}


/// Parses a CLDL string into our CldlDefinition struct.
fn parse_cldl(cldl_string: &str) -> Result<CldlDefinition, pest::error::Error<Rule>> {
    let mut file = CldlParser::parse(Rule::file, cldl_string.trim())?;
    let circuit_pair = file.next().unwrap().into_inner().next().unwrap();
    assert_eq!(circuit_pair.as_rule(), Rule::circuit);

    let mut circuit_inner_rules = circuit_pair.into_inner();
    let name = circuit_inner_rules.next().unwrap().as_str().trim().to_string();

    let circuit_body_pair = circuit_inner_rules.next().unwrap();
    assert_eq!(circuit_body_pair.as_rule(), Rule::circuit_body);

    let mut inputs: Vec<IoPinDef> = Vec::new();
    let mut outputs: Vec<IoPinDef> = Vec::new();
    let mut parts = Vec::new();
    let mut connections = Vec::new();
    let mut initialization = Vec::new();

    for pair in circuit_body_pair.into_inner() {
        match pair.as_rule() {
            Rule::input_def => {
                // input_def = "INPUT" ~ io_ident ~ ("," ~ io_ident)* ~ ";"
                // Iterate over io_ident pairs
                for io_ident_pair in pair.into_inner() {
                    if io_ident_pair.as_rule() == Rule::io_ident {
                         inputs.push(parse_io_ident(io_ident_pair));
                    }
                }
            }
            Rule::output_def => {
                // output_def = "OUTPUT" ~ io_ident ~ ("," ~ io_ident)* ~ ";"
                for io_ident_pair in pair.into_inner() {
                     if io_ident_pair.as_rule() == Rule::io_ident {
                        outputs.push(parse_io_ident(io_ident_pair));
                     }
                }
            }
            Rule::part_def => {
                let mut part_inner = pair.into_inner();
                let instance_name = part_inner.next().unwrap().as_str().trim().to_string();
                let part_type = part_inner.next().unwrap().as_str().trim().to_string();
                parts.push(PartDef {
                    instance_name,
                    part_type,
                });
            }
            Rule::connect_def => {
                // connect_def = "CONNECT" ~ pin ~ "->" ~ pin ~ ";"
                let mut conn_inner = pair.into_inner();
                let source_pin = parse_pin_ref(conn_inner.next().unwrap());
                let dest_pin = parse_pin_ref(conn_inner.next().unwrap());
                connections.push(ConnectionDef {
                    source_pin,
                    dest_pin,
                });
            }
            Rule::init_block => {
                for init_cmd_wrapper_pair in pair.into_inner() {
                    if init_cmd_wrapper_pair.as_rule() == Rule::init_command {
                        let init_cmd_pair = init_cmd_wrapper_pair.into_inner().next().unwrap();
                        match init_cmd_pair.as_rule() {
                            Rule::set_command => {
                                let assignments = init_cmd_pair
                                    .into_inner()
                                    .filter(|p| p.as_rule() == Rule::pin_value)
                                    .map(parse_pin_value)
                                    .collect();
                                initialization.push(InitCommand::Set(assignments));
                            }
                            Rule::eval_command => {
                                initialization.push(InitCommand::Eval);
                            }
                            Rule::seq_command => {
                                let steps = init_cmd_pair
                                    .into_inner()
                                    .filter(|p| p.as_rule() == Rule::step)
                                    .map(parse_step)
                                    .collect();
                                initialization.push(InitCommand::Seq(steps));
                            }
                            rule => {
                                println!(
                                    "[Parser] Unexpected rule within init_command: {:?}",
                                    rule
                                );
                            }
                        }
                    } else {
                        println!(
                            "[Parser] Ignoring rule within init_block: {:?}",
                            init_cmd_wrapper_pair.as_rule()
                        );
                    }
                }
            }
            rule => {
                println!("[Parser] Ignoring rule: {:?}", rule);
            }
        }
    }

    Ok(CldlDefinition {
        name,
        inputs,
        outputs,
        parts,
        connections,
        initialization,
    })
}

// --- Circuit Builder ---

/// This struct holds all known circuit definitions and can build them.
pub struct CircuitBuilder {
    /// A map from circuit name (e.g., "XOR") to its parsed definition.
    definitions: HashMap<String, CldlDefinition>,
}

// Helper function to execute initialization using the wire_map
fn execute_initialization(
    circuit: &Circuit,
    wire_map: &HashMap<String, Wire>,
    commands: &[InitCommand],
    instance_prefix: &str,
) {
    let component_name = if instance_prefix.is_empty() {
        circuit.name.as_str()
    } else {
        instance_prefix
    };
    for command in commands {
        match command {
            InitCommand::Set(assignments) => {
                for pv in assignments {
                    let value = match pv.value {
                        InitValue::Zero => false,
                        InitValue::One => true,
                    };
                    // Apply prefix to the PinRef before converting to key
                    let full_pin_name = namespace_pin_ref(&pv.pin_ref, instance_prefix).to_wire_map_key();

                    if let Some(wire_to_set) = wire_map.get(&full_pin_name) {
                        wire_to_set.borrow_mut().value = value;
                    } else {
                        eprintln!(
                            "Warning: [Init] Wire '{}' not found in wire_map during initialization of {}.",
                            full_pin_name, component_name
                        );
                    }
                }
            }
            InitCommand::Eval => {
                circuit.evaluate_stable();
            }
            InitCommand::Seq(steps) => {
                for step in steps.iter() {
                    for pv in &step.assignments {
                        let value = match pv.value {
                            InitValue::Zero => false,
                            InitValue::One => true,
                        };
                        let full_pin_name = namespace_pin_ref(&pv.pin_ref, instance_prefix).to_wire_map_key();
                         if let Some(wire_to_set) = wire_map.get(&full_pin_name) {
                            wire_to_set.borrow_mut().value = value;
                         } else {
                             eprintln!(
                                "Warning: [Init Seq] Wire '{}' not found in wire_map during initialization of {}.",
                                full_pin_name, component_name
                            );
                         }
                    }
                    circuit.evaluate_stable();
                }
            }
        }
    }
}

// This function adds the prefix *before* the index part.
// e.g., prefix="LATCH_M", pin_ref={base="Data", index=0} -> {base="LATCH_M.Data", index=0}
// e.g., prefix="LATCH_M", pin_ref={base="G1.out", index=None} -> {base="LATCH_M.G1.out", index=None}
fn namespace_pin_ref(pin_ref: &PinRef, prefix: &str) -> PinRef {
    if prefix.is_empty() {
        pin_ref.clone() // Top-level, no prefix needed
    } else {
        PinRef {
            base_name: format!("{}.{}", prefix, pin_ref.base_name),
            index: pin_ref.index, // Index remains the same
        }
    }
}


impl CircuitBuilder {
    /// Creates a new builder and loads all the base CLDL definitions recursively.
    pub fn new() -> Self {
        let mut definitions = HashMap::new();
        println!("Loading embedded circuit definitions recursively...");

        fn load_recursive(dir: &'static Dir<'static>, definitions: &mut HashMap<String, CldlDefinition>) {
            for entry in dir.entries() {
                match entry {
                    DirEntry::Dir(sub_dir) => {
                        load_recursive(sub_dir, definitions);
                    }
                    DirEntry::File(file) => {
                        if file.path().extension().map_or(false, |s| s == "cldl") {
                            let cldl_string = file.contents_utf8().expect("Circuit file not valid UTF-8");
                            match parse_cldl(cldl_string.trim()) {
                                Ok(def) => {
                                    if definitions.contains_key(&def.name) {
                                        eprintln!("Warning: Duplicate circuit definition found for '{}'. Overwriting.", def.name);
                                    }
                                    definitions.insert(def.name.clone(), def);
                                },
                                Err(e) => panic!("Failed to parse embedded CLDL from {:?}:\n{}", file.path(), e),
                            }
                        }
                    }
                }
            }
        }

        load_recursive(&CIRCUIT_DIR, &mut definitions);
        if definitions.is_empty() {
            eprintln!("Warning: No embedded .cldl files found.");
        }
        Self { definitions }
    }

    /// Loads additional circuit definitions from a directory at runtime.
    /// This will overwrite any existing definitions with the same name.
    pub fn load_circuits_from_path(&mut self, path: &Path) -> std::io::Result<()> {
        if !path.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Circuit directory not found at: {:?}", path),
            ));
        }

        println!("Loading circuits from path: {:?}", path);
        for entry in fs::read_dir(path)? {
            let entry = entry?; let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |s| s == "cldl") {
                let cldl_string = fs::read_to_string(&path)?;
                match parse_cldl(cldl_string.trim()) {
                    Ok(def) => {
                         if self.definitions.contains_key(&def.name) {
                             eprintln!("Warning: Overwriting definition for '{}' from file.", def.name);
                         }
                         self.definitions.insert(def.name.clone(), def);
                    },
                    Err(e) => eprintln!("Failed to parse CLDL from file {:?}:\n{}", path, e),
                }
            } else if path.is_dir() {
                self.load_circuits_from_path(&path)?;
            }
        }
        Ok(())
    }

    /// Builds a new `Circuit` from the named definition and runs initialization.
    pub fn build(&self, top_level_name: &str) -> Circuit {
        let mut circuit = Circuit::new(top_level_name.to_string());
        let mut wire_map: HashMap<String, Wire> = HashMap::new();
        let top_def = self.definitions.get(top_level_name)
            .unwrap_or_else(|| panic!("Top-level circuit definition '{}' not found", top_level_name));

        // --- Pass 1: Handle bus inputs ---
        for input_def in &top_def.inputs {
            match input_def.width {
                Some(width) => {
                    // Create wires for each bit of the bus
                    for i in 0..width {
                        let wire_name = format!("{}.{}", input_def.name, i);
                        let w = circuit.add_input(&wire_name); // Add individual inputs to circuit
                        wire_map.insert(wire_name, w);
                    }
                }
                None => {
                    // Single wire input
                    let w = circuit.add_input(&input_def.name);
                    wire_map.insert(input_def.name.clone(), w);
                }
            }
        }

        // Pass 2: Apply top-level Connections
        self.apply_connections(&top_def.connections, "", &mut wire_map);

        // Pass 3: Instantiate all parts recursively
        for part in &top_def.parts {
            self.instantiate_part(
                &part.part_type,
                &part.instance_name,
                &mut circuit,
                &mut wire_map,
            );
        }

        //Pass 4: Handle bus outputs
        for output_def in &top_def.outputs {
             match output_def.width {
                Some(width) => {
                    // Connect wires for each bit of the bus
                    for i in 0..width {
                        let wire_name = format!("{}.{}", output_def.name, i);
                        let w = wire_map.get(&wire_name)
                           .unwrap_or_else(|| panic!("Output wire bit '{}' not found after build in {}", wire_name, top_level_name))
                           .clone();
                        circuit.add_output(&wire_name, w); // Add individual outputs to circuit
                    }
                }
                None => {
                    // Single wire output
                    let w = wire_map.get(&output_def.name)
                        .unwrap_or_else(|| panic!("Output wire '{}' not found after build in {}", output_def.name, top_level_name))
                        .clone();
                    circuit.add_output(&output_def.name, w);
                }
            }
        }

        // Pass 5: Execute Top-Level Init
        if !top_def.initialization.is_empty() {
            execute_initialization(&circuit, &wire_map, &top_def.initialization, "");
        }
        circuit
    }

    /// Recursively instantiates a part and runs its initialization if defined.
    fn instantiate_part(
        &self,
        part_type: &str,
        instance_name: &str,
        circuit: &mut Circuit,
        wire_map: &mut HashMap<String, Wire>,
    ) {
        if part_type == "NAND" {
            // Primitive gates don't have indexed pins *internally*
            // Connections *to* them might be indexed, handled by apply_connections
            let in1_wire_name = format!("{}.in1", instance_name);
            let in2_wire_name = format!("{}.in2", instance_name);
            let out_wire_name = format!("{}.out", instance_name);

            let in1 = self.get_or_create_wire(&in1_wire_name, wire_map);
            let in2 = self.get_or_create_wire(&in2_wire_name, wire_map);
            let out = self.get_or_create_wire(&out_wire_name, wire_map);
            let gate = NandGate {
                 id: instance_name.to_string(),
                in1, in2, out
            };
            circuit.add_gate(Box::new(gate));

        } else if let Some(def) = self.definitions.get(part_type) {
            // Apply internal connections for the sub-circuit first
            self.apply_connections(&def.connections, instance_name, wire_map);
            // Instantiate sub-parts recursively
            for sub_part in &def.parts {
                self.instantiate_part(
                    &sub_part.part_type,
                    &format!("{}.{}", instance_name, sub_part.instance_name),
                    circuit,
                    wire_map,
                );
            }
            // Execute initialization for this sub-part AFTER its children are built
            if !def.initialization.is_empty() {
                execute_initialization(circuit, wire_map, &def.initialization, instance_name);
            }
        } else {
            panic!("Unknown part type '{}' requested by part '{}'", part_type, instance_name);
        }
    }

    /// Applies a list of connections, handling potential bus indices.
    fn apply_connections(
        &self,
        connections: &[ConnectionDef],
        instance_prefix: &str,
        wire_map: &mut HashMap<String, Wire>,
    ) {
        for conn in connections {
            // Apply prefix to base names before generating map keys
            let src_pin_ref = namespace_pin_ref(&conn.source_pin, instance_prefix);
            let dest_pin_ref = namespace_pin_ref(&conn.dest_pin, instance_prefix);

            // Convert PinRefs to the canonical wire map key (e.g., "A.0")
            let src_wire_key = src_pin_ref.to_wire_map_key();
            let dest_wire_key = dest_pin_ref.to_wire_map_key();

            // Perform aliasing using the generated keys
            if let Some(dest_wire) = wire_map.get(&dest_wire_key).cloned() {
                wire_map.insert(src_wire_key, dest_wire);
            } else {
                let src_wire = self.get_or_create_wire(&src_wire_key, wire_map);
                wire_map.insert(dest_wire_key, src_wire);
            }
        }
    }

    /// Helper to get a wire from the map or create it if it doesn't exist, using canonical key.
    fn get_or_create_wire(&self, canonical_wire_name: &str, map: &mut HashMap<String, Wire>) -> Wire {
        map.entry(canonical_wire_name.to_string()).or_insert_with(wire).clone()
    }
}
