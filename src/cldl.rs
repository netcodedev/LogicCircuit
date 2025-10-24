use std::{collections::HashMap, fs, path::Path, fmt::Write};
use include_dir::{include_dir, Dir};

use pest::Parser;
// Import LogicGate trait
use crate::{wire, Circuit, NandGate, Wire, LogicGate}; // <-- ADDED LogicGate

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

/// Represents the entire definition of one CIRCUIT block.
#[derive(Debug, Clone)]
pub struct CldlDefinition {
    name: String,
    inputs: Vec<String>,
    outputs: Vec<String>,
    parts: Vec<PartDef>,
    connections: Vec<ConnectionDef>,
}

/// Parses a CLDL string into our CldlDefinition struct.
fn parse_cldl(cldl_string: &str) -> Result<CldlDefinition, pest::error::Error<Rule>> {
    let mut file = CldlParser::parse(Rule::file, cldl_string.trim())?;
    let circuit_pair = file.next().unwrap().into_inner().next().unwrap();
    
    let mut inner_rules = circuit_pair.into_inner();
    let name = inner_rules.next().unwrap().as_str().trim().to_string();

    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    let mut parts = Vec::new();
    let mut connections = Vec::new();

    for pair in inner_rules {
        match pair.as_rule() {
            Rule::input_def => {
                for ident in pair.into_inner() {
                    inputs.push(ident.as_str().trim().to_string());
                }
            }
            Rule::output_def => {
                for ident in pair.into_inner() {
                    outputs.push(ident.as_str().trim().to_string());
                }
            }
            Rule::part_def => {
                let mut part_inner = pair.into_inner();
                let instance_name = part_inner.next().unwrap().as_str().trim().to_string();
                let part_type = part_inner.next().unwrap().as_str().trim().to_string();
                parts.push(PartDef { instance_name, part_type });
            }
            Rule::connect_def => {
                let mut conn_inner = pair.into_inner();
                let source_pin = conn_inner.next().unwrap().as_str().trim().to_string();
                let dest_pin = conn_inner.next().unwrap().as_str().trim().to_string();
                connections.push(ConnectionDef { source_pin, dest_pin });
            }
            rule => {
                // This should not print WHITESPACE or COMMENT if they are silent.
                println!("  [Parser] Ignoring rule: {:?}", rule); // Removed debug
            }
        }
    }

    Ok(CldlDefinition { name, inputs, outputs, parts, connections })
}


// --- Circuit Builder ---

/// This struct holds all known circuit definitions and can build them.
pub struct CircuitBuilder {
    /// A map from circuit name (e.g., "XOR") to its parsed definition.
    definitions: HashMap<String, CldlDefinition>,
}

// Helper function for DOT export
// This needs to parse "Parent.Child.Pin" into (Parent.Child, Pin)
fn parse_dot_pin_path(full_pin_path: &str) -> (String, Option<String>) {
    let parts: Vec<&str> = full_pin_path.split('.').collect();
    if parts.len() > 1 {
        let pin_name = parts.last().unwrap().to_string();
        let node_name = parts[0..parts.len() - 1].join(".");
        (node_name, Some(pin_name))
    } else {
        (full_pin_path.to_string(), None)
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
                    },
                    Err(e) => panic!("Failed to parse embedded CLDL from {:?}:\n{}", file.path(), e),
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
            return Err(std::io::Error::new(std::io::ErrorKind::NotFound, 
                format!("Circuit directory not found at: {:?}", path)));
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
                    },
                    Err(e) => eprintln!("Failed to parse CLDL from file {:?}:\n{}", path, e),
                }
            }
        }
        Ok(())
    }


    /// Builds a new `Circuit` from the named definition.
    pub fn build(&self, top_level_name: &str) -> Circuit {
        let mut circuit = Circuit::new(top_level_name.to_string());
        
        let mut wire_map: HashMap<String, Wire> = HashMap::new();
        
        let top_def = self.definitions.get(top_level_name)
            .expect("Top-level circuit definition not found");

        // --- Pass 1: Create wires for top-level Inputs ---
        for input_name in &top_def.inputs {
            let w = circuit.add_input(input_name);
            wire_map.insert(input_name.clone(), w);
        }

        // --- Pass 2: Handle Connections (Wire Aliasing) ---
        self.apply_connections(&top_def.connections, "", &mut wire_map);

        // --- Pass 3: Instantiate all parts recursively ---
        for part in &top_def.parts {
            self.instantiate_part(
                &part.part_type,
                &part.instance_name,
                &mut circuit,
                &mut wire_map
            );
        }
        
        // --- Pass 4: Connect top-level Outputs ---
        for output_name in &top_def.outputs {
            let w = self.get_or_create_wire(output_name, &mut wire_map);
            circuit.add_output(output_name, w);
        }

        circuit
    }

    /// Recursively instantiates a part and all its sub-parts.
    fn instantiate_part(
        &self,
        part_type: &str,
        instance_name: &str, // e.g., "G_XOR"
        circuit: &mut Circuit,
        wire_map: &mut HashMap<String, Wire>,
    ) {
        if part_type == "NAND" {
            // --- Base Case: Primitive Gate ---
            let in1 = self.get_or_create_wire(&format!("{}.in1", instance_name), wire_map);
            let in2 = self.get_or_create_wire(&format!("{}.in2", instance_name), wire_map);
            let out = self.get_or_create_wire(&format!("{}.out", instance_name), wire_map);
            
            // --- MODIFIED: Pass the instance_name as the ID ---
            let gate = NandGate {
                id: instance_name.to_string(),
                in1, 
                in2, 
                out 
            };
            circuit.add_gate(Box::new(gate));

        } else if let Some(def) = self.definitions.get(part_type) {
            // --- Recursive Case: Composite Gate (Subcircuit) ---
            
            // 1. Apply all *internal* connections for this subcircuit.
            self.apply_connections(&def.connections, instance_name, wire_map);

            // 2. Instantiate all *sub-parts* of this part.
            for sub_part in &def.parts {
                self.instantiate_part(
                    &sub_part.part_type,
                    &format!("{}.{}", instance_name, sub_part.instance_name),
                    circuit,
                    wire_map,
                );
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
    
    /// Exports the hierarchical definition of a circuit to the DOT graph language.
    pub fn export_to_dot(&self, top_level_name: &str) -> String {
        let mut dot_string = String::new();
        let top_def = self.definitions.get(top_level_name)
            .expect("Top-level circuit definition not found");

        writeln!(dot_string, "digraph G {{").unwrap();
        writeln!(dot_string, "  rankdir=LR;").unwrap(); // Left-to-right layout
        writeln!(dot_string, "  node [fontname=\"Helvetica\"];").unwrap();
        writeln!(dot_string, "  edge [fontname=\"Helvetica\"];").unwrap();
        writeln!(dot_string, "  compound=true;").unwrap(); // Allow edges to/from subgraphs

        // Recursively build the graph
        self.build_dot_recursive(top_def, "", &mut dot_string);

        writeln!(dot_string, "}}").unwrap();
        dot_string
    }

    /// Recursive helper for building the DOT string.
    fn build_dot_recursive(
        &self,
        def: &CldlDefinition,
        prefix: &str,
        dot_string: &mut String
    ) {
        // Create a subgraph cluster for this component
        let cluster_id = if prefix.is_empty() { def.name.clone() } else { prefix.to_string() };
        writeln!(dot_string, "  subgraph \"cluster_{}\" {{", cluster_id).unwrap();
        writeln!(dot_string, "    label = \"{}\";", def.name).unwrap();
        writeln!(dot_string, "    style = \"filled\";").unwrap();
        writeln!(dot_string, "    color = \"lightgrey\";").unwrap();
        // --- MODIFIED: Default node shape is now record ---
        writeln!(dot_string, "    node [shape=record, style=filled, fillcolor=white];").unwrap();


        // Define "dummy" nodes for top-level inputs and outputs within this cluster
        for input_name in &def.inputs {
            let input_node_id = format!("{}_in_{}", cluster_id, input_name);
            // --- MODIFIED: Use record shape for consistency, but simple label ---
            writeln!(dot_string, "    \"{}\" [label=\"{}\", shape=invhouse, height=0.3, width=0.3, fixedsize=true];", input_node_id, input_name).unwrap();
        }
        for output_name in &def.outputs {
            let output_node_id = format!("{}_out_{}", cluster_id, output_name);
            // --- MODIFIED: Use record shape for consistency, but simple label ---
            writeln!(dot_string, "    \"{}\" [label=\"{}\", shape=house, height=0.3, width=0.3, fixedsize=true];", output_node_id, output_name).unwrap();
        }

        // --- NEW: List of all primitive gates ---
        let primitives = ["NAND", "NOT", "AND", "OR", "XOR", "XNOR", "NOR"];

        // Define Parts (Nodes) and recurse for subcircuits
        for part in &def.parts {
            let part_dot_id = format!("{}.{}", cluster_id, part.instance_name);
            
            // --- MODIFIED: Use record shape for primitives ---
            if primitives.contains(&part.part_type.as_str()) {
                // Primitive gates are nodes with ports
                let label = if part.part_type == "NOT" {
                    // Format: {<port_id>port_label | ...}
                    format!("{{{}}}", part.instance_name)
                } else {
                    // All other 2-input primitives
                    format!("{{{}}}", part.instance_name)
                };
                writeln!(dot_string, "    \"{}\" [label=\"{}\"];", part_dot_id, label).unwrap();

            } else if let Some(sub_def) = self.definitions.get(&part.part_type) {
                // Recursive case: This part is a subgraph
                self.build_dot_recursive(sub_def, &part_dot_id, dot_string);
            }
        }

        // --- MODIFIED: Connection Logic ---
        for conn in &def.connections {
            let (src_node_name, src_pin_opt) = parse_dot_pin_path(&conn.source_pin);
            let (dest_node_name, dest_pin_opt) = parse_dot_pin_path(&conn.dest_pin);

            // 1. Find the full DOT ID for the SOURCE node
            let src_dot_id = match src_node_name.as_str() {
                _ if def.inputs.contains(&src_node_name) => {
                    // It's a connection FROM a cluster input
                    format!("\"{}\"", format!("{}_in_{}", cluster_id, src_node_name))
                },
                _ => { // It's a connection FROM a part's output
                    let node_id = format!("{}.{}", cluster_id, src_node_name);
                    let port_id = src_pin_opt.as_deref().unwrap_or("out"); // Default to "out"
                    format!("\"{}\":{}", node_id, port_id)
                }
            };

            // 2. Find the full DOT ID for the DESTINATION node
            let dest_dot_id = match dest_node_name.as_str() {
                _ if def.outputs.contains(&dest_node_name) => {
                    // It's a connection TO a cluster output
                    format!("\"{}\"", format!("{}_out_{}", cluster_id, dest_node_name))
                },
                _ => { // It's a connection TO a part's input
                    let node_id = format!("{}.{}", cluster_id, dest_node_name);
                    // The pin name *is* the port name (e.g., "in1", "in2", "in")
                    let port_id = dest_pin_opt.as_deref().unwrap_or("in"); 
                    format!("\"{}\":{}", node_id, port_id)
                }
            };
            
            // 3. Use the destination pin name as the label
            let edge_label = dest_pin_opt.unwrap_or_default(); 
            
            // 4. Write the edge
            writeln!(dot_string, "  {} -> {} [label=\"{}\"];", src_dot_id, dest_dot_id, edge_label).unwrap();
        }

        writeln!(dot_string, "  }}").unwrap(); // End of subgraph cluster
    }
}

