use std::{cell::RefCell, collections::HashMap, fs, path::Path, fmt::Write, rc::Rc};

use include_dir::{Dir, DirEntry};

use crate::{
    circuit::{Circuit, Gate, GateKind, Wire, wire},
    cldl::{
        CIRCUIT_DIR, CldlDefinition, FlatBlock, FlatCircuit, FlatLevel, FlatPin, InitCommand,
        InitValue, IoPinDef, PinRef,
        elaborate::{parse_template, split_instance_name},
    },
};

use super::CircuitBuilder;

impl CircuitBuilder {
    const PRIMITIVE_GATES: &[&str] = &[
        "NAND", "NOT", "AND", "OR", "NOR", "XOR", "XNOR", 
        "AND3", "NOR4", "MUX_2_1", "DFLIP_FLOP", "ADDER_4B", "REGISTER_4B"
    ];
    /// Creates a new builder and loads all the base CLDL definitions recursively.
    pub fn new() -> Self {
        let mut builder = CircuitBuilder {
            definitions: RefCell::new(HashMap::new()),
            templates: HashMap::new(),
        };
        println!("Loading embedded circuit definitions recursively...");

        fn load_recursive(dir: &'static Dir<'static>, builder: &mut CircuitBuilder) {
            for entry in dir.entries() {
                match entry {
                    DirEntry::Dir(sub_dir) => {
                        load_recursive(sub_dir, builder);
                    }
                    DirEntry::File(file) => {
                        if file.path().extension().map_or(false, |s| s == "cldl") {
                            let cldl_string =
                                file.contents_utf8().expect("Circuit file not valid UTF-8");
                            if let Err(e) = builder.add_source(cldl_string, category_of(file.path())) {
                                eprintln!("Failed to parse embedded CLDL file {:?}:\n{}", file.path(), e);
                            }
                        }
                    }
                }
            }
        }

        load_recursive(&CIRCUIT_DIR, &mut builder);
        if builder.definitions.get_mut().is_empty() {
            eprintln!("Warning: No embedded .cldl files found.");
        }
        // The built-in gates as circuits of their own (one gate each), so
        // they can be built, tested and shown like any definition.
        for kind in GateKind::ALL {
            if kind != GateKind::Nand {
                builder
                    .add_source(&kind.wrapper_source(), Some("primitive".to_string()))
                    .expect("built-in gate definition");
            }
        }
        builder
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
                if let Err(e) = self.add_source(&cldl_string, category_of(&path)) {
                    eprintln!("Failed to parse CLDL from file {:?}:\n{}", path, e);
                }
            } else if path.is_dir() {
                self.load_circuits_from_path(&path)?;
            }
        }
        Ok(())
    }

    /// Parses a single CLDL source string and adds (or overwrites) its
    /// definition. Useful where there is no filesystem, e.g. in WASM.
    /// Returns the circuit's name (without parameters for a parametric one).
    pub fn load_cldl_str(&mut self, cldl_string: &str) -> Result<String, String> {
        self.add_source(cldl_string, None)
    }

    /// Parses `source` and stores it: an ordinary definition is elaborated
    /// right away (unrolling its loops), a parametric one is kept as a
    /// template and elaborated per parameter value on first use.
    fn add_source(&mut self, source: &str, category: Option<String>) -> Result<String, String> {
        let mut template = parse_template(source)?;
        template.category = category;
        let name = template.name.clone();
        let definitions = self.definitions.get_mut();
        // Instances of an older version of this template are stale now.
        definitions.retain(|k, _| split_instance_name(k).is_none_or(|(base, _)| base != name));
        if template.params.is_empty() {
            let def = template.elaborate(&[])?;
            if definitions.contains_key(&name) {
                eprintln!("Warning: Overwriting definition for '{}'.", name);
            }
            definitions.insert(name.clone(), Rc::new(def));
            self.templates.remove(&name);
        } else {
            if self.templates.contains_key(&name) || definitions.contains_key(&name) {
                eprintln!("Warning: Overwriting definition for '{}'.", name);
            }
            definitions.remove(&name);
            self.templates.insert(name.clone(), template);
        }
        Ok(name)
    }

    /// Looks up a definition by name. Names with parameter values such as
    /// `REGISTER<16>` are elaborated from their template on first use.
    ///
    /// Panics if elaborating a template fails (the message names the source
    /// position); use `instantiate` to get the error instead.
    pub(super) fn def(&self, name: &str) -> Option<Rc<CldlDefinition>> {
        self.instantiate(name).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Like `def`, but reports elaboration errors (unknown parameter, wrong
    /// number of parameter values, backwards range, ...) as `Err`.
    /// `Ok(None)` means there is no such definition or template.
    pub fn instantiate(&self, name: &str) -> Result<Option<Rc<CldlDefinition>>, String> {
        if let Some(def) = self.definitions.borrow().get(name) {
            return Ok(Some(def.clone()));
        }
        let Some((base, args)) = split_instance_name(name) else {
            return Ok(None);
        };
        let Some(template) = self.templates.get(base) else {
            return Ok(None);
        };
        let def = Rc::new(template.elaborate(&args)?);
        // Store under the canonical name (`REGISTER<16>`), and under the
        // spelling asked for if it differs (`REGISTER< 16 >`).
        let mut definitions = self.definitions.borrow_mut();
        definitions.insert(def.name.clone(), def.clone());
        definitions.insert(name.to_string(), def.clone());
        Ok(Some(def))
    }

    /// Elaborates `name` and every part type below it, returning the first
    /// error. Useful to surface template errors before building.
    pub fn check(&self, name: &str) -> Result<(), String> {
        let def = self
            .instantiate(name)?
            .ok_or_else(|| format!("Unknown circuit '{name}'"))?;
        for part in &def.parts {
            if GateKind::from_name(&part.part_type).is_none() {
                if self.instantiate(&part.part_type)?.is_none() {
                    return Err(format!(
                        "{}: unknown part type '{}' (part '{}')",
                        def.name, part.part_type, part.instance_name
                    ));
                }
                self.check(&part.part_type)?;
            }
        }
        Ok(())
    }

    /// Names of the parametric definitions (templates), sorted, e.g. `ADDER`
    /// for `CIRCUIT ADDER<W>`, with their parameter names.
    pub fn template_names(&self) -> Vec<(String, Vec<String>)> {
        let mut names: Vec<_> = self.templates.values().map(|t| (t.name.clone(), t.params.clone())).collect();
        names.sort();
        names
    }


    /// Builds a new `Circuit` from the named definition and runs initialization.
    ///
    /// Wiring is resolved in two passes. First a *planning* pass walks the whole
    /// hierarchy and records every connection as a union in an equivalence
    /// structure over wire keys (`BuildPlan`), without creating any `Wire`.
    /// Then a *materialization* pass creates exactly one shared `Wire` per
    /// equivalence class and binds gates, inputs and outputs to it.
    ///
    /// Resolving all aliases before any `Wire` exists is what makes wiring
    /// order-independent and fan-out-safe: a single source pin can drive any
    /// number of loads, and a connection can span hierarchy boundaries, without
    /// an earlier connection being able to sever a later one.
    ///
    /// Built-in gates (`GateKind`) become one simulated gate each.
    pub fn build(&self, top_level_name: &str) -> Circuit {
        self.build_with(top_level_name, &|_| true)
    }

    /// Like `build`, but every built-in gate is replaced by its CLDL
    /// definition from NAND gates (`GateKind::nand_definition`), so the
    /// circuit consists of NAND gates only. Behaves the same as `build`;
    /// used to check the NAND definitions against the built-in gates.
    pub fn build_nand(&self, top_level_name: &str) -> Circuit {
        self.build_with(top_level_name, &|_| false)
    }

    /// Builds with the built-in gates for which `native(name)` is true; the
    /// others are expanded into NAND gates (NAND itself always stays).
    fn build_with(&self, top_level_name: &str, native: &dyn Fn(&str) -> bool) -> Circuit {
        let mut circuit = Circuit::new(top_level_name.to_string());
        let top_def = self.def(top_level_name).unwrap_or_else(|| {
            panic!(
                "Top-level circuit definition '{}' not found",
                top_level_name
            )
        });

        // === Planning pass: collect wire equivalence classes, gates and inits ===
        let mut plan = self.plan_circuit(&top_def, native);

        // === Materialization pass: one shared Wire per equivalence class ===
        let wire_map = plan.materialize();

        // Register circuit inputs (bus-aware) using the shared wires.
        for input_def in &top_def.inputs {
            match input_def.width {
                Some(width) => {
                    for i in 0..width {
                        let key = format!("{}.{}", input_def.name, i);
                        circuit.register_input(&key, wire_map[&key].clone());
                    }
                }
                None => {
                    circuit.register_input(&input_def.name, wire_map[&input_def.name].clone());
                }
            }
        }

        // Register circuit outputs (bus-aware).
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

        // Replay the recorded build events in their original DFS order: a gate
        // is added when reached, and a part's initialization runs immediately
        // after its own subtree of gates (before any later sibling gate exists).
        // Preserving this interleaving matters for parts whose init pulses a pin
        // that a *later*-built gate also drives (e.g. a register initialised to 0
        // before the write decoder that feeds its Load line is in place).
        for event in &plan.events {
            match event {
                BuildEvent::Gate { id, kind, inputs, out } => {
                    circuit.add_builtin(Gate {
                        id: id.clone(),
                        kind: *kind,
                        inputs: inputs.iter().map(|k| wire_map[k].clone()).collect(),
                        out: wire_map[out].clone(),
                    });
                }
                BuildEvent::Init { prefix, commands } => {
                    execute_initialization(&circuit, &wire_map, commands, prefix);
                }
            }
        }

        // Store the full wire map so downstream code (tests) can probe any wire.
        circuit.set_wire_map(wire_map);

        circuit
    }

    /// Runs the planning pass for a top-level definition. Built-in gates for
    /// which `native` is false are planned as their NAND definition.
    fn plan_circuit(&self, top_def: &CldlDefinition, native: &dyn Fn(&str) -> bool) -> BuildPlan {
        let mut plan = BuildPlan::default();

        // Register the top-level IO pins so they always resolve to a wire, even
        // if left unconnected.
        self.register_io_keys(top_def, "", &mut plan);

        // Top-level connections.
        self.apply_connections(top_def, "", &mut plan);

        // Recursively plan every part.
        for part in &top_def.parts {
            self.plan_part(&part.part_type, &part.instance_name, &mut plan, native);
        }

        // Top-level initialization runs last, after every part is planned.
        if !top_def.initialization.is_empty() {
            plan.events.push(BuildEvent::Init {
                prefix: String::new(),
                commands: top_def.initialization.clone(),
            });
        }
        plan
    }

    /// Category of a definition: the directory it was loaded from (e.g.
    /// "adder", "memory"), None for definitions loaded from a string.
    pub fn category(&self, name: &str) -> Option<String> {
        self.def(name).and_then(|d| d.category.clone())
    }

    /// True for the built-in gates (NOT, AND, ...) and the other definitions
    /// loaded from the `primitive` directory (AND_4B, XOR_8B, ...): the basic
    /// blocks of `FlatLevel::Basic`.
    pub fn is_basic(&self, name: &str) -> bool {
        GateKind::from_name(name).is_some()
            || self.def(name).is_some_and(|d| d.category.as_deref() == Some("primitive"))
    }

    /// Flattens the named circuit down to `level`: parts are expanded until
    /// they are NAND gates (`FlatLevel::Nand`, built-in gates replaced by
    /// their NAND definitions) or basic blocks (`FlatLevel::Basic`, see
    /// `is_basic`). Nets come from the same planning pass as building, so at
    /// the basic level they are exactly the wires that are simulated.
    pub fn flatten(&self, name: &str, level: FlatLevel) -> Option<FlatCircuit> {
        self.flatten_until(name, &|part_type: &str| level == FlatLevel::Basic && self.is_basic(part_type))
    }

    /// Like `flatten`, but parts are kept as blocks wherever `stop(part_type)`
    /// is true (NAND gates always are). Used e.g. to keep flip-flops whole
    /// for cell mapping. A built-in gate for which `stop` is false is
    /// expanded into its NAND definition.
    pub fn flatten_until(&self, name: &str, stop: &dyn Fn(&str) -> bool) -> Option<FlatCircuit> {
        let top_def = self.def(name)?;
        let mut plan = self.plan_circuit(&top_def, stop);

        // Number the equivalence classes in key order so the result is
        // deterministic. A net's key is its shortest member (ties: smallest),
        // which favours the most top-level name.
        let mut keys: Vec<String> = plan.union.ids.keys().cloned().collect();
        keys.sort_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
        let mut net_of_root: HashMap<usize, usize> = HashMap::new();
        let mut nets: Vec<String> = Vec::new();
        for key in &keys {
            let id = plan.union.ids[key];
            let root = plan.union.find(id);
            net_of_root.entry(root).or_insert_with(|| {
                nets.push(key.clone());
                nets.len() - 1
            });
        }
        let mut net = |key: &str| {
            let id = plan.union.ids[key];
            net_of_root[&plan.union.find(id)]
        };
        let bits = |base: &str, width: Option<usize>| -> Vec<String> {
            match width {
                Some(w) => (0..w).map(|i| format!("{base}.{i}")).collect(),
                None => vec![base.to_string()],
            }
        };

        let mut found = Vec::new();
        self.collect_blocks(&top_def, "", stop, &mut found);
        let blocks = found
            .into_iter()
            .map(|(id, kind)| {
                let pins: Vec<(String, bool, Option<usize>)> = match self.def(&kind) {
                    _ if kind == "NAND" => vec![
                        ("in1".into(), false, None),
                        ("in2".into(), false, None),
                        ("out".into(), true, None),
                    ],
                    Some(def) => def
                        .inputs
                        .iter()
                        .map(|p| (p.name.clone(), false, p.width))
                        .chain(def.outputs.iter().map(|p| (p.name.clone(), true, p.width)))
                        .collect(),
                    None => Vec::new(),
                };
                let pins = pins
                    .into_iter()
                    .map(|(pin, output, width)| FlatPin {
                        nets: bits(&format!("{id}.{pin}"), width).iter().map(|k| net(k)).collect(),
                        name: pin,
                        output,
                        width,
                    })
                    .collect();
                FlatBlock { id, kind, pins }
            })
            .collect();

        let io_bits = |pins: &[IoPinDef]| -> Vec<String> {
            pins.iter().flat_map(|p| bits(&p.name, p.width)).collect()
        };
        let inputs = io_bits(&top_def.inputs).into_iter().map(|k| { let n = net(&k); (k, n) }).collect();
        let outputs = io_bits(&top_def.outputs).into_iter().map(|k| { let n = net(&k); (k, n) }).collect();

        let aliases = keys.iter().map(|k| (k.clone(), net(k))).collect();

        Some(FlatCircuit { nets, blocks, inputs, outputs, aliases })
    }

    /// Collects (instance path, type) of every part that is not expanded
    /// further (NAND, or `stop` is true), in build (DFS) order.
    fn collect_blocks(&self, def: &CldlDefinition, prefix: &str, stop: &dyn Fn(&str) -> bool, out: &mut Vec<(String, String)>) {
        for part in &def.parts {
            let path = if prefix.is_empty() {
                part.instance_name.clone()
            } else {
                format!("{prefix}.{}", part.instance_name)
            };
            let keep = part.part_type == "NAND" || stop(&part.part_type);
            // A built-in gate opens into its NAND definition, not into its
            // one-gate wrapper.
            let sub = match GateKind::from_name(&part.part_type) {
                Some(kind) => kind.nand_definition().and_then(|n| self.def(n)),
                None => self.def(&part.part_type),
            };
            match sub {
                Some(sub) if !keep => self.collect_blocks(&sub, &path, stop, out),
                _ => out.push((path, part.part_type.clone())),
            }
        }
    }

    /// Recursively records a part's structure into `plan` as an ordered stream
    /// of build events (gates in DFS order, each part's init right after its own
    /// subtree). No `Wire` is created here — that happens later in
    /// `BuildPlan::materialize`.
    fn plan_part(&self, part_type: &str, instance_name: &str, plan: &mut BuildPlan, native: &dyn Fn(&str) -> bool) {
        if let Some(kind) = GateKind::from_name(part_type) {
            if let Some(nand_def) = kind.nand_definition().filter(|_| !native(part_type)) {
                // Expanded into NAND gates: the NAND definition has the same
                // pins, so the connections to this instance stay the same.
                return self.plan_part(nand_def, instance_name, plan, native);
            }
            // A built-in gate. Its pins are plain (never indexed)
            // internally; indexed connections *to* it are handled by callers.
            let key = |pin: &str| format!("{}.{}", instance_name, pin);
            let inputs: Vec<String> = kind.inputs().iter().map(|p| key(p)).collect();
            let out = key(kind.output());
            for k in inputs.iter().chain(std::iter::once(&out)) {
                plan.register(k);
            }
            plan.events.push(BuildEvent::Gate {
                id: instance_name.to_string(),
                kind,
                inputs,
                out,
            });
        } else if let Some(def) = self.def(part_type) {
            // Make this part's IO pins resolvable even when unconnected.
            self.register_io_keys(&def, instance_name, plan);
            // Internal connections of the sub-circuit.
            self.apply_connections(&def, instance_name, plan);
            // Recurse into sub-parts.
            for sub_part in &def.parts {
                self.plan_part(
                    &sub_part.part_type,
                    &format!("{}.{}", instance_name, sub_part.instance_name),
                    plan,
                    native,
                );
            }
            // Record initialization AFTER its children (post-order).
            if !def.initialization.is_empty() {
                plan.events.push(BuildEvent::Init {
                    prefix: instance_name.to_string(),
                    commands: def.initialization.clone(),
                });
            }
        } else {
            panic!(
                "Unknown part type '{}' requested by part '{}'",
                part_type, instance_name
            );
        }
    }

    /// Registers every IO pin of `def` (bus-expanded, namespaced by `prefix`)
    /// as a wire key so it resolves to a wire even if left unconnected.
    fn register_io_keys(&self, def: &CldlDefinition, prefix: &str, plan: &mut BuildPlan) {
        for pin in def.inputs.iter().chain(def.outputs.iter()) {
            let base = if prefix.is_empty() {
                pin.name.clone()
            } else {
                format!("{}.{}", prefix, pin.name)
            };
            match pin.width {
                Some(width) => {
                    for i in 0..width {
                        plan.register(&format!("{}.{}", base, i));
                    }
                }
                None => plan.register(&base),
            }
        }
    }

    /// Finds the width of a pin *base_name* (e.g., "A" or "G1.in") within the context of `def`.
    /// Returns `Some(width)` for a bus, `None` for a single-bit pin.
    pub(super) fn get_pin_width(&self, base_name: &str, def: &CldlDefinition) -> Option<usize> {
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
                if let Some(sub_def) = self.def(&part_def.part_type) {
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

    /// Records that two wire keys are the same electrical node.
    ///
    /// Both keys are merged into one equivalence class in the plan. Because this
    /// is a symmetric union rather than a map overwrite, the order in which
    /// connections are processed does not matter, and a source pin may be
    /// aliased to any number of destinations (fan-out) without an earlier alias
    /// being severed by a later one.
    fn alias_wires(&self, plan: &mut BuildPlan, src_wire_key: &str, dest_wire_key: &str) {
        plan.alias(src_wire_key, dest_wire_key);
    }

    /// Applies connections from a definition, handling single-bit, bus-to-bus, and tee connections.
    fn apply_connections(
        &self,
        def: &CldlDefinition,
        instance_prefix: &str,
        plan: &mut BuildPlan,
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
                    self.alias_wires(plan, &src_wire_key, &dest_wire_key);
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
                                self.alias_wires(plan, &src_wire_key, &dest_wire_key);
                            }
                        }
                        (None, None) => {
                            // Single-to-Single (Clk -> G1.clk)
                            let src_wire_key = src_pin_ref.to_wire_map_key();
                            let dest_wire_key = dest_pin_ref.to_wire_map_key();
                            self.alias_wires(plan, &src_wire_key, &dest_wire_key);
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
                                self.alias_wires(plan, &src_wire_key, &dest_wire_key);
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
                            self.alias_wires(plan, &src_wire_key, &dest_wire_key);
                        }
                    } else {
                        // Single-bit-to-Single-pin (A[0] -> B)
                        let dest_wire_key = dest_pin_ref.to_wire_map_key(); // e.g., "MyPart.B"
                        self.alias_wires(plan, &src_wire_key, &dest_wire_key);
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
                        self.alias_wires(plan, &src_wire_key, &dest_wire_key);
                    }
                }
            }
        }
    }

    /// Exports the hierarchical definition of a circuit to the DOT graph language.
    pub fn export_to_dot(&self, top_level_name: &str) -> String {
        let mut dot_string = String::new();
        let top_def = self.def(top_level_name)
            .expect("Top-level circuit definition not found");

        writeln!(dot_string, "digraph G {{").unwrap();
        writeln!(dot_string, "  rankdir=LR;").unwrap(); // Left-to-right layout
        writeln!(dot_string, "  splines=ortho;").unwrap(); // Use straight 90-degree lines
        writeln!(dot_string, "  node [fontname=\"Helvetica\"];").unwrap();
        writeln!(dot_string, "  edge [fontname=\"Helvetica\"];").unwrap();
        writeln!(dot_string, "  compound=true;").unwrap(); // Allow edges to/from subgraphs

        // Recursively build the graph
        self.build_dot_recursive(&top_def, "", &mut dot_string);

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
            else if let Some(sub_def) = self.def(&part.part_type) {
                // Recursive case: This part is a subgraph
                self.build_dot_recursive(&sub_def, &part_dot_id, dot_string);
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

            let is_subgraph = self.def(part_type).is_some() 
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

/// The name of the directory containing `path`, used as a definition's category.
fn category_of(path: &Path) -> Option<String> {
    path.parent()?.file_name()?.to_str().map(str::to_string)
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



/// Union-find over wire-key strings.
///
/// Every wire key seen during the planning pass is interned to an integer, and
/// connections union the two keys' classes. All keys that end up in the same
/// class denote the same electrical node and will share a single `Wire`.
#[derive(Default)]
struct WireUnion {
    ids: HashMap<String, usize>,
    parent: Vec<usize>,
}

impl WireUnion {
    /// Returns the id for `key`, assigning a fresh singleton class on first use.
    fn intern(&mut self, key: &str) -> usize {
        if let Some(&id) = self.ids.get(key) {
            return id;
        }
        let id = self.parent.len();
        self.parent.push(id);
        self.ids.insert(key.to_string(), id);
        id
    }

    /// Finds the class representative for `x`, compressing the path on the way.
    fn find(&mut self, x: usize) -> usize {
        let mut root = x;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        let mut cur = x;
        while self.parent[cur] != cur {
            let next = self.parent[cur];
            self.parent[cur] = root;
            cur = next;
        }
        root
    }

    /// Merges the classes of `a` and `b` (interning either if unseen).
    fn union(&mut self, a: &str, b: &str) {
        let ia = self.intern(a);
        let ib = self.intern(b);
        let ra = self.find(ia);
        let rb = self.find(ib);
        if ra != rb {
            self.parent[ra] = rb;
        }
    }
}

/// One step of circuit construction, recorded in DFS order during planning and
/// replayed after wires are materialized.
enum BuildEvent {
    /// Add a built-in gate wired to the given (still unresolved) wire keys.
    Gate {
        id: String,
        kind: GateKind,
        inputs: Vec<String>,
        out: String,
    },
    /// Run an initialization block for the part at `prefix`.
    Init {
        prefix: String,
        commands: Vec<InitCommand>,
    },
}

/// The resolved structure of a circuit, accumulated during the planning pass
/// before any `Wire` is created.
#[derive(Default)]
struct BuildPlan {
    /// Equivalence classes over wire keys.
    union: WireUnion,
    /// Gate-add and init steps, in the exact order they must be replayed.
    events: Vec<BuildEvent>,
}

impl BuildPlan {
    /// Ensures `key` has a wire, even if it is never connected to anything.
    fn register(&mut self, key: &str) {
        self.union.intern(key);
    }

    /// Records that `a` and `b` are the same electrical node.
    fn alias(&mut self, a: &str, b: &str) {
        self.union.union(a, b);
    }

    /// Creates exactly one shared `Wire` per equivalence class and returns a map
    /// from every registered key to its class's wire.
    fn materialize(&mut self) -> HashMap<String, Wire> {
        let keys: Vec<String> = self.union.ids.keys().cloned().collect();
        let mut class_wire: HashMap<usize, Wire> = HashMap::new();
        let mut wire_map: HashMap<String, Wire> = HashMap::with_capacity(keys.len());
        for key in keys {
            let id = self.union.ids[&key];
            let root = self.union.find(id);
            let w = class_wire.entry(root).or_insert_with(wire).clone();
            wire_map.insert(key, w);
        }
        wire_map
    }
}
