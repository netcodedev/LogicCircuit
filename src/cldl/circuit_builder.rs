use std::{collections::HashMap, fs, path::Path, fmt::Write};

use include_dir::{Dir, DirEntry};
use pest::{Parser, iterators::Pair};

use crate::{
    circuit::{Circuit, NandGate, Wire, wire},
    cldl::{
        CIRCUIT_DIR, CldlDefinition, CldlParser, ConnectionDef, InitCommand, InitValue, IoPinDef,
        PartDef, PinRef, PinValue, SeqStep,
    },
};

use super::{CircuitBuilder, Rule};

impl CircuitBuilder {
    const PRIMITIVE_GATES: &[&str] = &[
        "NAND", "NOT", "AND", "OR", "NOR", "XOR", "XNOR", 
        "AND3", "NOR4", "MUX_2_1", "DFLIP_FLOP", "ADDER_4B", "REGISTER_4B"
    ];
    /// Creates a new builder and loads all the base CLDL definitions recursively.
    pub fn new() -> Self {
        let mut definitions = HashMap::new();
        println!("Loading embedded circuit definitions recursively...");

        fn load_recursive(
            dir: &'static Dir<'static>,
            definitions: &mut HashMap<String, CldlDefinition>,
        ) {
            for entry in dir.entries() {
                match entry {
                    DirEntry::Dir(sub_dir) => {
                        load_recursive(sub_dir, definitions);
                    }
                    DirEntry::File(file) => {
                        if file.path().extension().map_or(false, |s| s == "cldl") {
                            let cldl_string =
                                file.contents_utf8().expect("Circuit file not valid UTF-8");
                            match parse_cldl(cldl_string.trim()) {
                                Ok(def) => {
                                    if definitions.contains_key(&def.name) {
                                        eprintln!(
                                            "Warning: Duplicate circuit definition found for '{}'. Overwriting.",
                                            def.name
                                        );
                                    }
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
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |s| s == "cldl") {
                let cldl_string = fs::read_to_string(&path)?;
                match parse_cldl(cldl_string.trim()) {
                    Ok(def) => {
                        if self.definitions.contains_key(&def.name) {
                            eprintln!(
                                "Warning: Overwriting definition for '{}' from file.",
                                def.name
                            );
                        }
                        self.definitions.insert(def.name.clone(), def);
                    }
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
        let top_def = self.definitions.get(top_level_name).unwrap_or_else(|| {
            panic!(
                "Top-level circuit definition '{}' not found",
                top_level_name
            )
        });

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
        self.apply_connections(top_def, "", &mut wire_map);

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
                        let w = wire_map
                            .get(&wire_name)
                            .unwrap_or_else(|| {
                                panic!(
                                    "Output wire bit '{}' not found after build in {}",
                                    wire_name, top_level_name
                                )
                            })
                            .clone();
                        circuit.add_output(&wire_name, w); // Add individual outputs to circuit
                    }
                }
                None => {
                    // Single wire output
                    let w = wire_map
                        .get(&output_def.name)
                        .unwrap_or_else(|| {
                            panic!(
                                "Output wire '{}' not found after build in {}",
                                output_def.name, top_level_name
                            )
                        })
                        .clone();
                    circuit.add_output(&output_def.name, w);
                }
            }
        }

        // Pass 5: Execute Top-Level Init
        if !top_def.initialization.is_empty() {
            execute_initialization(&circuit, &wire_map, &top_def.initialization, "");
        }
        circuit.set_wire_map(wire_map);
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
                in1,
                in2,
                out,
            };
            circuit.add_gate(Box::new(gate));
        } else if let Some(def) = self.definitions.get(part_type) {
            // Apply internal connections for the sub-circuit first
            self.apply_connections(def, instance_name, wire_map);
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
            panic!(
                "Unknown part type '{}' requested by part '{}'",
                part_type, instance_name
            );
        }
    }

    /// Finds the width of a pin *base_name* (e.g., "A" or "G1.in") within the context of `def`.
    /// Returns `Some(width)` for a bus, `None` for a single-bit pin.
    fn get_pin_width(&self, base_name: &str, def: &CldlDefinition) -> Option<usize> {
        // Case 1: Is it an IO pin of the current definition? (e.g., "A")
        if !base_name.contains('.') {
            if let Some(pin) = def.inputs.iter().find(|p| p.name == base_name) {
                return pin.width;
            }
            if let Some(pin) = def.outputs.iter().find(|p| p.name == base_name) {
                return pin.width;
            }
            // Not an IO pin, must be an internal single wire
            return None;
        }

        // Case 2: It's a pin on a sub-part (e.g., "G1.in")
        // We need to find the part instance name and the pin name on that part.
        // `rfind` handles nested names like "Datapath.ALU.A" -> part="Datapath.ALU", pin="A"
        if let Some(dot_pos) = base_name.rfind('.') {
            let (part_instance_name, pin_name) = base_name.split_at(dot_pos);
            let pin_name = &pin_name[1..]; // remove leading '.'

            // Find the PartDef for "Datapath.ALU" in the *current* definition
            if let Some(part_def) = def.parts.iter().find(|p| p.instance_name == part_instance_name) {
                // Found PartDef { instance_name: "Datapath.ALU", part_type: "ALU_4B" }
                // Now find the definition for "ALU_4B"
                if let Some(sub_def) = self.definitions.get(&part_def.part_type) {
                    // Search the sub-definition's IO pins
                    if let Some(pin) = sub_def.inputs.iter().find(|p| p.name == pin_name) {
                        return pin.width;
                    }
                    if let Some(pin) = sub_def.outputs.iter().find(|p| p.name == pin_name) {
                        return pin.width;
                    }
                    // Pin not found on sub-def IO
                    eprintln!("Warning: Pin '{}' not found on inputs or outputs of part type '{}' (for instance '{}')",
                        pin_name, part_def.part_type, part_instance_name);
                    return None;
                } else {
                    // No definition found for this part_type.
                    // Is it "NAND"?
                    if part_def.part_type == "NAND" {
                        // "in1", "in2", "out" are all single-bit
                        return None;
                    }
                    // Unknown part type, assume single-bit pins
                    eprintln!("Warning: No CLDL definition found for part type '{}' (instance '{}'). Assuming single-bit pins.",
                        part_def.part_type, part_instance_name);
                    return None;
                }
            } else {
                // Part instance not found in definition
                eprintln!("Warning: Part instance '{}' (from pin base_name '{}') not found in definition '{}'.",
                    part_instance_name, base_name, def.name);
                return None;
            }
        }

        // Default: not found or malformed, assume single wire
        None
    }

    /// Helper to alias two wires in the map.
    /// `src_wire_key` will be aliased to `dest_wire_key`.
    fn alias_wires(
        &self,
        wire_map: &mut HashMap<String, Wire>,
        src_wire_key: &str,
        dest_wire_key: &str,
    ) {
        if let Some(dest_wire) = wire_map.get(dest_wire_key).cloned() {
            // Dest exists, make Src point to it
            wire_map.insert(src_wire_key.to_string(), dest_wire);
        } else {
            // Dest doesn't exist, get/create Src and make Dest point to it
            let src_wire = self.get_or_create_wire(src_wire_key, wire_map);
            wire_map.insert(dest_wire_key.to_string(), src_wire);
        }
    }

    /// Applies connections from a definition, handling single-bit, bus-to-bus, and tee connections.
    fn apply_connections(
        &self,
        def: &CldlDefinition,
        instance_prefix: &str,
        wire_map: &mut HashMap<String, Wire>,
    ) {
        for conn in &def.connections {
            // Apply prefix to base names (e.g., "A" -> "MyPart.A")
            let src_pin_ref = namespace_pin_ref(&conn.source_pin, instance_prefix);
            let dest_pin_ref = namespace_pin_ref(&conn.dest_pin, instance_prefix);

            match (src_pin_ref.index, dest_pin_ref.index) {
                (Some(_), Some(_)) => {
                    // Case 1: Single-bit to Single-bit (e.g., A[0] -> B[1])
                    let src_wire_key = src_pin_ref.to_wire_map_key();
                    let dest_wire_key = dest_pin_ref.to_wire_map_key();
                    self.alias_wires(wire_map, &src_wire_key, &dest_wire_key);
                }

                (None, None) => {
                    // Case 2: Bus-to-Bus (A -> B) OR Single-to-Single (Clk -> G1.clk)
                    // Get width using the *local* base names from the connection def
                    let src_width = self.get_pin_width(&conn.source_pin.base_name, def);
                    let dest_width = self.get_pin_width(&conn.dest_pin.base_name, def);

                    match (src_width, dest_width) {
                        (Some(w1), Some(w2)) => {
                            // Bus-to-Bus (A -> B)
                            if w1 != w2 {
                                panic!(
                                    "Bus width mismatch in '{}': {} (width {}) -> {} (width {})",
                                    def.name, src_pin_ref.base_name, w1, dest_pin_ref.base_name, w2
                                );
                            }
                            // Connect all bits
                            for i in 0..w1 {
                                // Use the *namespaced* base_name for the key
                                let src_wire_key = format!("{}.{}", src_pin_ref.base_name, i);
                                let dest_wire_key = format!("{}.{}", dest_pin_ref.base_name, i);
                                self.alias_wires(wire_map, &src_wire_key, &dest_wire_key);
                            }
                        }
                        (None, None) => {
                            // Single-to-Single (Clk -> G1.clk)
                            let src_wire_key = src_pin_ref.to_wire_map_key();
                            let dest_wire_key = dest_pin_ref.to_wire_map_key();
                            self.alias_wires(wire_map, &src_wire_key, &dest_wire_key);
                        }
                        (Some(w), None) => {
                            // Bus-to-Single (A -> B) - Error
                            panic!(
                                "Cannot connect bus {} (width {}) to single pin {} in {}",
                                src_pin_ref.base_name, w, dest_pin_ref.base_name, def.name
                            );
                        }
                        (None, Some(w)) => {
                            // Single-to-Bus (A -> B) - "Tee" connection
                            // Connect single source (A) to all bits of destination bus (B[0]...B[w-1])
                            let src_wire_key = src_pin_ref.to_wire_map_key();
                            for i in 0..w {
                                // Use the *namespaced* base_name for the key
                                let dest_wire_key = format!("{}.{}", dest_pin_ref.base_name, i);
                                self.alias_wires(wire_map, &src_wire_key, &dest_wire_key);
                            }
                        }
                    }
                }

                (Some(_), None) => {
                    // Case 3: Single-bit-to-Bus (e.g., A[0] -> B) - "Tee" connection
                    // OR Single-bit-to-Single-pin (e.g., A[0] -> B)
                    // Get width using the *local* base name from the connection def
                    let dest_width = self.get_pin_width(&conn.dest_pin.base_name, def);
                    let src_wire_key = src_pin_ref.to_wire_map_key(); // e.g., "MyPart.A.0"

                    if let Some(w) = dest_width {
                        // Tee A[0] to B[0]...B[w-1]
                        for i in 0..w {
                            // Use the *namespaced* base_name for the key
                            let dest_wire_key = format!("{}.{}", dest_pin_ref.base_name, i);
                            self.alias_wires(wire_map, &src_wire_key, &dest_wire_key);
                        }
                    } else {
                        // Single-bit-to-Single-pin (A[0] -> B)
                        let dest_wire_key = dest_pin_ref.to_wire_map_key(); // e.g., "MyPart.B"
                        self.alias_wires(wire_map, &src_wire_key, &dest_wire_key);
                    }
                }

                (None, Some(_)) => {
                    // Case 4: (e.g., A -> B[0])
                    // Source has no index, Dest has an index.
                    // Check if source is a bus or a single pin.
                    // Get width using the *local* base name from the connection def
                    let src_width = self.get_pin_width(&conn.source_pin.base_name, def);
                    
                    if src_width.is_some() {
                        // Source IS a bus (e.g., BusA -> B[0]) - Error
                        panic!(
                            "Cannot connect bus {} to single bit {} in {}",
                            src_pin_ref.base_name,
                            dest_pin_ref.to_wire_map_key(),
                            def.name
                        );
                    } else {
                        // Source is a single pin (e.g., FA0.sum -> S[0]) - OK
                        let src_wire_key = src_pin_ref.to_wire_map_key(); // "prefix.FA0.sum"
                        let dest_wire_key = dest_pin_ref.to_wire_map_key(); // "prefix.S.0"
                        self.alias_wires(wire_map, &src_wire_key, &dest_wire_key);
                    }
                }
            }
        }
    }

    /// Helper to get a wire from the map or create it if it doesn't exist, using canonical key.
    fn get_or_create_wire(
        &self,
        canonical_wire_name: &str,
        map: &mut HashMap<String, Wire>,
    ) -> Wire {
        map.entry(canonical_wire_name.to_string())
            .or_insert_with(wire)
            .clone()
    }

    /// Exports the hierarchical definition of a circuit to the DOT graph language.
    pub fn export_to_dot(&self, top_level_name: &str) -> String {
        let mut dot_string = String::new();
        let top_def = self.definitions.get(top_level_name)
            .expect("Top-level circuit definition not found");

        writeln!(dot_string, "digraph G {{").unwrap();
        writeln!(dot_string, "  rankdir=LR;").unwrap(); // Left-to-right layout
        writeln!(dot_string, "  splines=ortho;").unwrap(); // Use straight 90-degree lines
        writeln!(dot_string, "  node [fontname=\"Helvetica\"];").unwrap();
        writeln!(dot_string, "  edge [fontname=\"Helvetica\"];").unwrap();
        writeln!(dot_string, "  compound=true;").unwrap(); // Allow edges to/from subgraphs

        // Recursively build the graph
        self.build_dot_recursive(top_def, "", &mut dot_string);

        writeln!(dot_string, "}}").unwrap();
        dot_string
    }

    /// Recursive helper for building the DOT string using clusters and ports.
    fn build_dot_recursive(
        &self,
        def: &CldlDefinition,
        prefix: &str, // The unique instance path, e.g., "Datapath.ALU.Adder"
        dot_string: &mut String
    ) {
        // --- Create a subgraph cluster for this component ---
        let cluster_dot_id = if prefix.is_empty() { 
            "cluster_TOP".to_string() 
        } else { 
            format!("cluster_{}", prefix) 
        };
        
        writeln!(dot_string, "  subgraph \"{}\" {{", cluster_dot_id).unwrap();
        writeln!(dot_string, "    label = \"{}\\n({})\\n\";", prefix, def.name).unwrap(); // Label cluster with instance AND type
        writeln!(dot_string, "    style = \"filled\";").unwrap();
        writeln!(dot_string, "    color = \"lightgrey\";").unwrap();
        writeln!(dot_string, "    node [shape=box, style=filled, fillcolor=white];").unwrap();

        // --- Define "dummy" nodes for top-level inputs and outputs within this cluster ---
        // These nodes are *inside* the cluster and represent its interface
        for input_def in &def.inputs {
            let node_id = format!("\"{}_PIN_{}\"", prefix, input_def.name); // Unique ID
            let label = if let Some(w) = input_def.width { format!("{}[{}]", input_def.name, w) } else { input_def.name.clone() };
            writeln!(dot_string, "    {} [label=\"{}\", shape=invhouse];", node_id, label).unwrap();
        }
        for output_def in &def.outputs {
            let node_id = format!("\"{}_PIN_{}\"", prefix, output_def.name); // Unique ID
            let label = if let Some(w) = output_def.width { format!("{}[{}]", output_def.name, w) } else { output_def.name.clone() };
            writeln!(dot_string, "    {} [label=\"{}\", shape=house];", node_id, label).unwrap();
        }

        // --- Define Parts (Nodes) and recurse for subcircuits ---
        for part in &def.parts {
            // Create the unique dot ID for this part instance
            let part_dot_id = if prefix.is_empty() { 
                part.instance_name.clone() 
            } else { 
                format!("{}.{}", prefix, part.instance_name) 
            };

            // --- THIS IS THE NEW LOGIC ---
            // Check if it's a primitive first.
            if Self::PRIMITIVE_GATES.contains(&part.part_type.as_str()) {
                // Primitive gates are nodes, do not recurse.
                writeln!(dot_string, "    \"{}\" [label=\"{}\\n({})\"];", part_dot_id, part.instance_name, part.part_type).unwrap();
            } 
            // If not primitive, *then* check if it's a composite we can recurse into.
            else if let Some(sub_def) = self.definitions.get(&part.part_type) {
                // Recursive case: This part is a subgraph
                self.build_dot_recursive(sub_def, &part_dot_id, dot_string);
            } 
            // Fallback for unknown parts (e.g., composite buses)
            else {
                // Treat as a primitive/black box
                 writeln!(dot_string, "    \"{}\" [label=\"{}\\n({})\", style=dashed];", part_dot_id, part.instance_name, part.part_type).unwrap();
            }
        }

        // --- Connections ---
        // This part connects all the nodes defined above.
        for conn in &def.connections {
            // Get the dot node ID for the source pin
            let (src_node_dot_id, _src_label) = 
                self.get_dot_node_id_for_pin(&conn.source_pin, prefix, def);
            
            // Get the dot node ID for the destination pin
            let (dest_node_dot_id, _dest_label) = 
                self.get_dot_node_id_for_pin(&conn.dest_pin, prefix, def);
            
            // Create a label for the edge itself
            let edge_label = format!(" {}", conn.source_pin.to_wire_map_key().replace(prefix, "").trim_start_matches('.'));
            
            // Add lhead/ltail for subclusters
            let lhead = if dest_node_dot_id.starts_with("\"cluster_") { format!(", lhead={}", dest_node_dot_id) } else { "".to_string() };
            let ltail = if src_node_dot_id.starts_with("\"cluster_") { format!(", ltail={}", src_node_dot_id) } else { "".to_string() };

            writeln!(dot_string, "  {} -> {} [label=\"{}\"{}{}];", 
                src_node_dot_id, dest_node_dot_id, edge_label, ltail, lhead).unwrap();
        }

        writeln!(dot_string, "  }}").unwrap(); // End of subgraph cluster
    }

    /// Helper for DOT export: Finds the DOT node ID for a given PinRef.
    /// A PinRef can be an IO pin ("A") or a part pin ("G1.in1").
    /// Returns the Node's DOT ID (which may be a cluster or a node) and a pin label.
    fn get_dot_node_id_for_pin(
        &self,
        pin_ref: &PinRef,
        prefix: &str, // The instance prefix, e.g., "Datapath.ALU"
        def: &CldlDefinition, // Pass in the current definition
    ) -> (String, String) {
        let base_name = &pin_ref.base_name;
        let parts: Vec<&str> = base_name.split('.').collect();

        if parts.len() > 1 {
            // It's a part pin, e.g., "G1.in1" or "Mux.A"
            let part_instance_name = parts[0..parts.len()-1].join(".");
            let pin_name = parts.last().unwrap().to_string();
            
            // Full DOT ID of the part instance
            let part_dot_id = if prefix.is_empty() { 
                part_instance_name.clone() 
            } else { 
                format!("{}.{}", prefix, part_instance_name) 
            };

            // Check if this part is a subgraph (composite) or a primitive node
            // --- FIX for E0277 ---
            // Find the part's type (e.g., "AND") from the *current* definition's part list
            let part_type = def.parts.iter()
                .find(|p| p.instance_name == part_instance_name)
                .map_or("", |p| p.part_type.as_str());

            let is_subgraph = self.definitions.contains_key(part_type) 
                                && !Self::PRIMITIVE_GATES.contains(&part_type);
            // --- END FIX ---

            if is_subgraph {
                // It's a subgraph (cluster). We connect to its *internal* IO pin node.
                let pin_node_id = format!("\"{}_PIN_{}\"", part_dot_id, pin_name);
                (pin_node_id, pin_name)
            } else {
                // It's a primitive node. We connect to the node itself.
                (format!("\"{}\"", part_dot_id), pin_name)
            }

        } else {
            // It's an IO pin of the *current* cluster, e.g., "A" or "Cout"
            let node_id = format!("\"{}_PIN_{}\"", prefix, base_name);
            (node_id, base_name.clone())
        }
    }
}

/// --- Parses a pest 'pin' rule into our PinRef struct ---
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
            Rule::bus_specifier => {}
            _ => unreachable!("Unexpected rule inside pin: {:?}", inner_pair.as_rule()),
        }
    }

    let mut index = None; // Reset

    if let Some(bracket_pos) = full_pin_str.find('[') {
        if full_pin_str.ends_with(']') {
            let num_str = &full_pin_str[bracket_pos + 1..full_pin_str.len() - 1];
            index = Some(
                num_str
                    .parse::<usize>()
                    .expect("Grammar ensures valid number in specifier"),
            );
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

/// Parses an io_ident rule into IoPinDef
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
                width = Some(
                    num_str
                        .parse::<usize>()
                        .expect("Grammar ensures valid number"),
                );
            }
            _ => unreachable!(
                "Unexpected rule inside io_ident: {:?}",
                inner_pair.as_rule()
            ),
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
    let name = circuit_inner_rules
        .next()
        .unwrap()
        .as_str()
        .trim()
        .to_string();

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

/// Helper function to execute initialization using the wire_map
fn execute_initialization(
    circuit: &Circuit,
    wire_map: &HashMap<String, Wire>,
    commands: &[InitCommand],
    instance_prefix: &str,
) {
    let component_name = if instance_prefix.is_empty() {
        circuit.get_name()
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
                    let full_pin_name =
                        namespace_pin_ref(&pv.pin_ref, instance_prefix).to_wire_map_key();

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
                        let full_pin_name =
                            namespace_pin_ref(&pv.pin_ref, instance_prefix).to_wire_map_key();
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

/// This function adds the prefix *before* the index part.
/// e.g., prefix="LATCH_M", pin_ref={base="Data", index=0} -> {base="LATCH_M.Data", index=0}
/// e.g., prefix="LATCH_M", pin_ref={base="G1.out", index=None} -> {base="LATCH_M.G1.out", index=None}
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


