use include_dir::{Dir, include_dir};
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

/// e.g., "PART G1: NAND"
#[derive(Debug, Clone)]
pub struct PartDef {
    instance_name: String,
    part_type: String,
}

/// e.g., "CONNECT a -> G1.in1"
#[derive(Debug, Clone)]
pub struct ConnectionDef {
    source_pin: String,
    dest_pin: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitValue {
    Zero,
    One,
}

#[derive(Debug, Clone)]
pub struct PinValue {
    pin_name: String,
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
    inputs: Vec<String>,
    outputs: Vec<String>,
    parts: Vec<PartDef>,
    connections: Vec<ConnectionDef>,
    initialization: Vec<InitCommand>,
}

/// Helper to parse a single pin_value pair (e.g., "D 0")
fn parse_pin_value(pair: Pair<Rule>) -> PinValue {
    let mut inner = pair.into_inner();
    let pin_name = inner.next().unwrap().as_str().trim().to_string();
    let value_str = inner.next().unwrap().as_str();
    let value = match value_str {
        "0" => InitValue::Zero,
        "1" => InitValue::One,
        _ => unreachable!("Grammar ensures value is 0 or 1"),
    };
    PinValue { pin_name, value }
}

/// Helper to parse a single step (one or more pin_values)
fn parse_step(pair: Pair<Rule>) -> SeqStep {
    let assignments = pair.into_inner().map(parse_pin_value).collect();
    SeqStep { assignments }
}

/// Parses a CLDL string into our CldlDefinition struct.
fn parse_cldl(cldl_string: &str) -> Result<CldlDefinition, pest::error::Error<Rule>> {
    let mut file = CldlParser::parse(Rule::file, cldl_string.trim())?;
    let circuit_pair = file.next().unwrap().into_inner().next().unwrap();
    assert_eq!(circuit_pair.as_rule(), Rule::circuit);

    let mut inner_rules = circuit_pair.into_inner();
    let name = inner_rules.next().unwrap().as_str().trim().to_string();

    let circuit_body_pair = inner_rules.next().unwrap();
    assert_eq!(circuit_body_pair.as_rule(), Rule::circuit_body);

    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    let mut parts = Vec::new();
    let mut connections = Vec::new();
    let mut initialization = Vec::new();

    for pair in circuit_body_pair.into_inner() {
        match pair.as_rule() {
            Rule::input_def => {
                for ident in pair.into_inner() {
                    if ident.as_rule() == Rule::ident {
                        inputs.push(ident.as_str().trim().to_string());
                    }
                }
            }
            Rule::output_def => {
                for ident in pair.into_inner() {
                    if ident.as_rule() == Rule::ident {
                        outputs.push(ident.as_str().trim().to_string());
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
                let mut conn_inner = pair.into_inner();
                let source_pin = conn_inner.next().unwrap().as_str().trim().to_string();
                let dest_pin = conn_inner.next().unwrap().as_str().trim().to_string();
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
                    // Apply prefix to pin name before looking up in wire_map
                    let full_pin_name = namespace_pin_static(&pv.pin_name, instance_prefix);

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
                for (_i, step) in steps.iter().enumerate() {
                    // Set all pins for this step, applying prefix
                    for pv in &step.assignments {
                        let value = match pv.value {
                            InitValue::Zero => false,
                            InitValue::One => true,
                        };
                        let full_pin_name = namespace_pin_static(&pv.pin_name, instance_prefix);
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

fn namespace_pin_static(pin_name: &str, prefix: &str) -> String {
    if prefix.is_empty() {
        return pin_name.to_string();
    }
    if pin_name.contains('.') {
        format!("{}.{}", prefix, pin_name)
    } else {
        format!("{}.{}", prefix, pin_name)
    }
}

impl CircuitBuilder {
    /// Creates a new builder and loads all the base CLDL definitions
    /// embedded in the binary from the "src/circuits" directory.
    pub fn new() -> Self {
        let mut definitions = HashMap::new();

        println!("Loading embedded circuit definitions...");
        for file in CIRCUIT_DIR.files() {
            // Only read files with the .cldl extension
            if file.path().extension().map_or(false, |s| s == "cldl") {
                let cldl_string = file.contents_utf8().expect("Circuit file not valid UTF-8");

                match parse_cldl(cldl_string.trim()) {
                    Ok(def) => {
                        definitions.insert(def.name.clone(), def);
                    }
                    Err(e) => panic!(
                        "Failed to parse embedded CLDL from {:?}:\n{}",
                        file.path(),
                        e
                    ),
                }
            }
        }
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
        let entries = fs::read_dir(path)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            // Only read files with the .cldl extension
            if path.is_file() && path.extension().map_or(false, |s| s == "cldl") {
                let cldl_string = fs::read_to_string(&path)?;

                match parse_cldl(cldl_string.trim()) {
                    Ok(def) => {
                        println!("[Builder] Loaded file definition for '{}'", def.name);
                        self.definitions.insert(def.name.clone(), def);
                    }
                    Err(e) => eprintln!("Failed to parse CLDL from file {:?}:\n{}", path, e),
                }
            }
        }
        Ok(())
    }

    /// Builds a new `Circuit` from the named definition and runs initialization.
    pub fn build(&self, top_level_name: &str) -> Circuit {
        let mut circuit = Circuit::new(top_level_name.to_string());
        let mut wire_map: HashMap<String, Wire> = HashMap::new();
        let top_def = self
            .definitions
            .get(top_level_name)
            .expect("Top-level circuit definition not found");

        // Pass 1: Create top-level Inputs
        for input_name in &top_def.inputs {
            let w = circuit.add_input(input_name);
            wire_map.insert(input_name.clone(), w);
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

        // Pass 4: Connect top-level Outputs
        for output_name in &top_def.outputs {
            // Use get() or expect() as wire *should* exist now after instantiation
            let w = wire_map
                .get(output_name)
                .expect(&format!(
                    "Output wire '{}' not found after build",
                    output_name
                ))
                .clone();
            circuit.add_output(output_name, w);
        }

        // Pass 5: Execute Initialization Sequence (Now recursively done within instantiate_part)
        // We still need to run it for the top-level circuit itself.
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
            let in1 = self.get_or_create_wire(&format!("{}.in1", instance_name), wire_map);
            let in2 = self.get_or_create_wire(&format!("{}.in2", instance_name), wire_map);
            let out = self.get_or_create_wire(&format!("{}.out", instance_name), wire_map);

            let gate = NandGate {
                id: instance_name.to_string(),
                in1,
                in2,
                out,
            };
            circuit.add_gate(Box::new(gate));
        } else if let Some(def) = self.definitions.get(part_type) {
            // Apply internal connections first
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
            panic!("Unknown part type '{}'", part_type);
        }
    }

    /// Applies a list of connections, prefixing them with the instance name.
    fn apply_connections(
        &self,
        connections: &[ConnectionDef],
        instance_prefix: &str, // e.g., "G_XOR"
        wire_map: &mut HashMap<String, Wire>,
    ) {
        for conn in connections {
            let src_pin = self.namespace_pin(&conn.source_pin, instance_prefix);
            let dest_pin = self.namespace_pin(&conn.dest_pin, instance_prefix);

            if let Some(dest_wire) = wire_map.get(&dest_pin).cloned() {
                wire_map.insert(src_pin, dest_wire);
            } else {
                let src_wire = self.get_or_create_wire(&src_pin, wire_map);
                wire_map.insert(dest_pin, src_wire);
            }
        }
    }

    /// Prepends a prefix to a pin name.
    fn namespace_pin(&self, pin_name: &str, prefix: &str) -> String {
        if prefix.is_empty() {
            return pin_name.to_string(); // Top-level, no prefix
        }
        format!("{}.{}", prefix, pin_name)
    }

    /// Helper to get a wire from the map or create it if it doesn't exist.
    fn get_or_create_wire(&self, name: &str, map: &mut HashMap<String, Wire>) -> Wire {
        map.entry(name.to_string()).or_insert_with(wire).clone()
    }
}
