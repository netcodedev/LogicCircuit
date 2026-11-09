use include_dir::{Dir, DirEntry, include_dir};
use logic_circuit::circuit::Circuit;
use logic_circuit::cldl::{CircuitBuilder, CircuitView, EndpointView, FlatLevel, PinView};
use logic_circuit::export::verilog::{CellLib, VerilogOptions, export_verilog};
use logic_circuit::{isa, machine};
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

// The builder already embeds src/circuits; the top-level circuits/ directory
// (CPUs, datapaths) is only loaded at runtime by the CLI, so embed it here.
static EXTRA_CIRCUITS: Dir = include_dir!("$CARGO_MANIFEST_DIR/../circuits");

// Example programs offered in the viewer.
static PROGRAMS: Dir = include_dir!("$CARGO_MANIFEST_DIR/../programs");

fn load_extra(dir: &Dir, builder: &mut CircuitBuilder) {
    for entry in dir.entries() {
        match entry {
            DirEntry::Dir(sub) => load_extra(sub, builder),
            DirEntry::File(file) => {
                if file.path().extension().is_some_and(|e| e == "cldl") {
                    if let Some(src) = file.contents_utf8() {
                        if let Err(e) = builder.load_cldl_str(src) {
                            web_sys_log(&format!("Failed to parse {:?}: {}", file.path(), e));
                        }
                    }
                }
            }
        }
    }
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = warn)]
    fn web_sys_log(s: &str);
}

fn pins_json(pins: &[PinView]) -> Value {
    pins.iter()
        .map(|p| json!({ "name": p.name, "width": p.width }))
        .collect()
}

fn endpoint_json(e: &EndpointView) -> Value {
    json!({ "part": e.part, "pin": e.pin, "index": e.index })
}

fn view_json(view: &CircuitView) -> Value {
    json!({
        "name": view.name,
        "nands": view.nand_count,
        "inputs": pins_json(&view.inputs),
        "outputs": pins_json(&view.outputs),
        "parts": view.parts.iter().map(|p| json!({
            "name": p.instance_name,
            "type": p.part_type,
            "inputs": pins_json(&p.inputs),
            "outputs": pins_json(&p.outputs),
            "expandable": p.expandable,
            "nands": p.nand_count,
        })).collect::<Value>(),
        "connections": view.connections.iter().map(|c| json!({
            "source": endpoint_json(&c.source),
            "dest": endpoint_json(&c.dest),
            "width": c.width,
        })).collect::<Value>(),
    })
}

/// A circuit library plus (optionally) one built, simulated circuit.
#[wasm_bindgen]
pub struct Sim {
    builder: CircuitBuilder,
    circuit: Option<Circuit>,
    /// Instruction memory served on PC -> Instruction.
    rom: Vec<u16>,
}

#[wasm_bindgen]
impl Sim {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Sim {
        console_error_panic_hook::set_once();
        let mut builder = CircuitBuilder::new();
        load_extra(&EXTRA_CIRCUITS, &mut builder);
        Sim {
            builder,
            circuit: None,
            rom: Vec::new(),
        }
    }

    /// JSON array of all circuit names.
    pub fn circuit_names(&self) -> String {
        json!(self.builder.circuit_names()).to_string()
    }

    /// The named circuit flattened to NAND gates (`level` = "nand") or basic
    /// blocks ("basic"), or "null" if unknown. JSON:
    /// {"nets":[key..], "blocks":[{"id","kind","pins":[{"name","output","width","nets":[..]}]}..],
    ///  "inputs":[[key,net]..], "outputs":[[key,net]..]} with nets as indices into "nets".
    pub fn flatten(&self, name: &str, level: &str) -> String {
        let level = if level == "basic" { FlatLevel::Basic } else { FlatLevel::Nand };
        match self.builder.flatten(name, level) {
            Some(flat) => json!({
                "nets": flat.nets,
                "blocks": flat.blocks.iter().map(|b| json!({
                    "id": b.id,
                    "kind": b.kind,
                    "pins": b.pins.iter().map(|p| json!({
                        "name": p.name, "output": p.output, "width": p.width, "nets": p.nets,
                    })).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
                "inputs": flat.inputs,
                "outputs": flat.outputs,
            })
            .to_string(),
            None => "null".to_string(),
        }
    }

    /// Structural Verilog of `name`. `level`: "basic" or "nand"; `cells`:
    /// "sky130" or "generic".
    pub fn export_verilog(&self, name: &str, level: &str, cells: &str, keep_latches: bool) -> Result<String, JsError> {
        let opts = VerilogOptions {
            level: if level == "nand" { FlatLevel::Nand } else { FlatLevel::Basic },
            cells: if cells == "generic" { CellLib::Generic } else { CellLib::Sky130 },
            keep_latches,
        };
        export_verilog(&self.builder, name, &opts).map(|n| n.text).map_err(|e| JsError::new(&e))
    }

    /// JSON object mapping every circuit name to its NAND gate count.
    pub fn nand_counts(&self) -> String {
        let counts: serde_json::Map<String, Value> = self
            .builder
            .circuit_names()
            .into_iter()
            .filter_map(|name| self.builder.nand_count(&name).map(|n| (name, json!(n))))
            .collect();
        Value::Object(counts).to_string()
    }

    /// JSON description of one hierarchy level, or "null" if unknown.
    pub fn describe(&self, name: &str) -> String {
        match self.builder.describe(name) {
            Some(view) => view_json(&view).to_string(),
            None => "null".to_string(),
        }
    }

    /// Adds or replaces a definition from CLDL source; returns its name.
    pub fn load_cldl(&mut self, src: &str) -> Result<String, JsError> {
        self.builder.load_cldl_str(src).map_err(|e| JsError::new(&e))
    }

    /// Builds `top` (running its initialization) and settles it. On a
    /// runnable circuit the word at PC 0 is presented right away.
    pub fn build(&mut self, top: &str) {
        let circuit = self.builder.build(top);
        circuit.evaluate_stable();
        self.circuit = Some(circuit);
        self.fetch();
    }

    /// Sets a top-level input (bus bits as "Name.i") without evaluating.
    pub fn set_input(&self, key: &str, value: bool) {
        if let Some(c) = &self.circuit {
            c.set_input(key, value);
        }
    }

    pub fn eval(&self) {
        if let Some(c) = &self.circuit {
            c.evaluate_stable();
        }
    }

    /// Assembles source. JSON: {"ok":true,"words":[..],"lines":[..],"listing":[..]}
    /// or {"ok":false,"line":n,"message":".."}.
    pub fn assemble(&self, src: &str) -> String {
        match isa::assemble(src) {
            Ok(p) => json!({
                "ok": true,
                "listing": p.words.iter().map(|&w| isa::disassemble(w)).collect::<Vec<_>>(),
                "words": p.words,
                "lines": p.lines,
            }),
            Err(e) => json!({ "ok": false, "line": e.line, "message": e.message }),
        }
        .to_string()
    }

    pub fn disassemble(&self, word: u16) -> String {
        isa::disassemble(word)
    }

    /// Example programs as JSON [{"name":..,"source":..}], sorted by name.
    pub fn examples(&self) -> String {
        let mut list: Vec<Value> = PROGRAMS
            .files()
            .filter(|f| f.path().extension().is_some_and(|e| e == "asm"))
            .map(|f| {
                json!({
                    "name": f.path().file_stem().and_then(|s| s.to_str()).unwrap_or(""),
                    "source": f.contents_utf8().unwrap_or(""),
                })
            })
            .collect();
        list.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        Value::Array(list).to_string()
    }

    /// Replaces the instruction memory and presents the word at the current PC.
    pub fn set_rom(&mut self, words: Vec<u16>) {
        self.rom = words;
        self.fetch();
    }

    /// True if the built circuit has PC/Instruction/CLK ports.
    pub fn is_runnable(&self) -> bool {
        self.circuit.as_ref().is_some_and(machine::is_runnable)
    }

    pub fn fetch(&self) {
        if let Some(c) = self.circuit.as_ref().filter(|c| machine::is_runnable(c)) {
            machine::fetch(c, &self.rom);
        }
    }

    /// Executes one instruction; false if halted (or not runnable).
    pub fn step(&self) -> bool {
        match self.circuit.as_ref().filter(|c| machine::is_runnable(c)) {
            Some(c) => machine::step(c, &self.rom),
            None => false,
        }
    }

    pub fn pc(&self) -> u32 {
        self.circuit.as_ref().map_or(0, |c| machine::pc(c) as u32)
    }

    pub fn halted(&self) -> bool {
        self.circuit.as_ref().is_some_and(machine::halted)
    }

    /// Values for newline-separated wire keys: 0, 1, or 2 if the key is unknown.
    pub fn values(&self, keys: &str) -> Vec<u8> {
        let Some(c) = &self.circuit else {
            return Vec::new();
        };
        let map = c.get_wire_map();
        keys.split('\n')
            .map(|k| map.get(k).map_or(2, |w| w.borrow().value as u8))
            .collect()
    }
}

impl Default for Sim {
    fn default() -> Self {
        Self::new()
    }
}
