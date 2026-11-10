use include_dir::{Dir, DirEntry, include_dir};
use logic_circuit::circuit::{Circuit, GateKind};
use std::cell::RefCell;
use std::collections::HashMap;
use logic_circuit::cldl::{FLIP_FLOP, CircuitBuilder, CircuitView, EndpointView, FlatLevel, PinView};
use logic_circuit::export::verilog::{CellLib, VerilogOptions, export_verilog};
use logic_circuit::isa::Isa;
use logic_circuit::machine::{self, Machine};
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

// The builder already embeds src/circuits; the top-level circuits/ directory
// (CPUs, datapaths) is only loaded at runtime by the CLI, so embed it here.
static EXTRA_CIRCUITS: Dir = include_dir!("$CARGO_MANIFEST_DIR/../circuits");

// Example programs offered in the viewer: CPU_8B programs at the top level,
// CPU_16B programs in programs/16, SOC_16B I/O programs in programs/soc.
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
        "blocks": view.block_count,
        "inputs": pins_json(&view.inputs),
        "outputs": pins_json(&view.outputs),
        "parts": view.parts.iter().map(|p| json!({
            "name": p.instance_name,
            "type": p.part_type,
            "inputs": pins_json(&p.inputs),
            "outputs": pins_json(&p.outputs),
            "expandable": p.expandable,
            "nands": p.nand_count,
            "blocks": p.block_count,
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
    /// Memories and clock of the built circuit, if it is runnable.
    machine: Option<Machine>,
    /// One circuit per built-in gate type, built from NAND gates: gives the
    /// values of the wires inside a gate for the NAND and transistor views.
    nand_cells: RefCell<HashMap<GateKind, Circuit>>,
    /// Latch version of the flip-flop, for the wires inside built-in flip-flops.
    flop_cell: RefCell<Option<Circuit>>,
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
            machine: None,
            nand_cells: RefCell::new(HashMap::new()),
            flop_cell: RefCell::new(None),
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
    /// "sky130" (hd), "sky130hs" or "generic".
    pub fn export_verilog(&self, name: &str, level: &str, cells: &str, keep_latches: bool) -> Result<String, JsError> {
        let opts = VerilogOptions {
            level: if level == "nand" { FlatLevel::Nand } else { FlatLevel::Basic },
            cells: match cells {
                "generic" => CellLib::Generic,
                "sky130hs" => CellLib::Sky130Hs,
                _ => CellLib::Sky130,
            },
            keep_latches,
            // The download shows the gates, as the basic-block view does.
            mux_cells: false,
        };
        export_verilog(&self.builder, name, &opts).map(|n| n.text).map_err(|e| JsError::new(&e))
    }

    /// Places and routes `name` with our own router (see logic_circuit::pnr)
    /// and returns the layout in the binary format of `chip extract`.
    pub fn route_layout(&self, name: &str, layers: i32, utilization: f64, effort: f64) -> Result<Vec<u8>, JsError> {
        let opts = logic_circuit::pnr::Options { layers, utilization, effort, ..Default::default() };
        let (layout, report) = logic_circuit::pnr::place_and_route(&self.builder, name, &opts).map_err(|e| JsError::new(&e))?;
        if report.overflow > 0 || !report.failed_nets.is_empty() {
            return Err(JsError::new(&format!(
                "Verdrahtung nicht vollständig: {} überlappende Gitterpunkte, {} offene Netze – mehr Lagen versuchen",
                report.overflow,
                report.failed_nets.len()
            )));
        }
        Ok(layout.write_binary())
    }

    /// JSON object mapping every circuit name to its category (the
    /// directory under src/circuits, e.g. "adder"), or null for the
    /// designs from circuits/ and definitions added in the viewer.
    pub fn categories(&self) -> String {
        let map: serde_json::Map<String, Value> = self
            .builder
            .circuit_names()
            .into_iter()
            .map(|name| {
                let cat = self.builder.category(&name);
                (name, json!(cat))
            })
            .collect();
        Value::Object(map).to_string()
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

    /// JSON object mapping every circuit name to its number of basic blocks
    /// (built-in gates and multi-bit blocks, as in the basic-block view).
    pub fn block_counts(&self) -> String {
        let counts: serde_json::Map<String, Value> = self
            .builder
            .circuit_names()
            .into_iter()
            .filter_map(|name| self.builder.block_count(&name).map(|n| (name, json!(n))))
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
    /// runnable circuit the word at PC 0 is presented right away. The
    /// program stays loaded if the instruction set did not change; the data
    /// memory starts cleared.
    pub fn build(&mut self, top: &str) {
        let circuit = self.builder.build(top);
        circuit.evaluate_stable();
        let old = self.machine.take();
        self.machine = Machine::new(&circuit).map(|mut m| {
            if let Some(old) = old.filter(|o| o.isa() == m.isa()) {
                m.rom = old.rom;
            }
            // Power-on reset as in the CLI runner (SOC_16B: synchronizers,
            // idle UART line).
            m.reset(&circuit);
            m
        });
        self.circuit = Some(circuit);
        self.fetch();
    }

    fn isa_of(&self) -> Isa {
        self.machine.as_ref().map_or(Isa::Cpu8, Machine::isa)
    }

    /// Instruction set of the built circuit: 8 (CPU_8B) or 16 (CPU_16B).
    pub fn isa(&self) -> u32 {
        match self.isa_of() {
            Isa::Cpu8 => 8,
            Isa::Cpu16 => 16,
        }
    }

    /// Width of the PC in bits (0 if not runnable).
    pub fn pc_width(&self) -> u32 {
        self.machine.as_ref().map_or(0, |m| m.pc_width() as u32)
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

    /// Assembles source for the instruction set of the built circuit. JSON: {"ok":true,"words":[..],"lines":[..],"listing":[..]}
    /// or {"ok":false,"line":n,"message":".."}.
    pub fn assemble(&self, src: &str) -> String {
        let isa = self.isa_of();
        match isa.assemble(src) {
            Ok(p) => json!({
                "ok": true,
                "listing": isa.listing(&p.words),
                "words": p.words,
                "lines": p.lines,
            }),
            Err(e) => json!({ "ok": false, "line": e.line, "message": e.message }),
        }
        .to_string()
    }

    /// Example programs for the instruction set of the built circuit as
    /// JSON [{"name":..,"source":..}], sorted by name. A circuit with a UART
    /// (SOC_16B) gets the I/O programs of programs/soc first, named "soc/…".
    pub fn examples(&self) -> String {
        let group = |dir: Option<&Dir>, prefix: &str| -> Vec<Value> {
            let mut list: Vec<Value> = dir
                .into_iter()
                .flat_map(|d| d.files())
                .filter(|f| f.path().extension().is_some_and(|e| e == "asm"))
                .map(|f| {
                    let stem = f.path().file_stem().and_then(|s| s.to_str()).unwrap_or("");
                    json!({ "name": format!("{prefix}{stem}"), "source": f.contents_utf8().unwrap_or("") })
                })
                .collect();
            list.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
            list
        };
        let list = match self.isa_of() {
            Isa::Cpu8 => group(Some(&PROGRAMS), ""),
            Isa::Cpu16 if self.has_uart() => {
                [group(PROGRAMS.get_dir("soc"), "soc/"), group(PROGRAMS.get_dir("16"), "")].concat()
            }
            Isa::Cpu16 => group(PROGRAMS.get_dir("16"), ""),
        };
        Value::Array(list).to_string()
    }

    /// True if the built circuit has a UART line `Tx` (SOC_16B).
    pub fn has_uart(&self) -> bool {
        self.circuit.as_ref().is_some_and(|c| c.has_wire(machine::TX))
    }

    /// Text received on the UART line since the circuit was built.
    pub fn uart_text(&self) -> String {
        self.machine.as_ref().map_or(String::new(), |m| String::from_utf8_lossy(&m.uart).into_owned())
    }

    /// Queues `text` for the circuit's UART receive line (`Rx`); it is sent
    /// while the program runs, at the bit time the circuit's UART is set to.
    pub fn send_uart(&mut self, text: &str) {
        if let Some(m) = &mut self.machine {
            m.uart_in.extend(text.bytes());
        }
    }

    /// Replaces the instruction memory and presents the word at the current PC.
    pub fn set_rom(&mut self, words: Vec<u16>) {
        if let Some(m) = &mut self.machine {
            m.rom = words;
        }
        self.fetch();
    }

    /// True if the built circuit has PC/Instruction/CLK ports.
    pub fn is_runnable(&self) -> bool {
        self.machine.is_some()
    }

    pub fn fetch(&self) {
        if let (Some(c), Some(m)) = (&self.circuit, &self.machine) {
            m.fetch(c);
        }
    }

    /// Executes one instruction; false if halted (or not runnable).
    pub fn step(&mut self) -> bool {
        match (&self.circuit, &mut self.machine) {
            (Some(c), Some(m)) => m.step(c),
            _ => false,
        }
    }

    pub fn pc(&self) -> u32 {
        match (&self.circuit, &self.machine) {
            (Some(c), Some(m)) => m.pc(c) as u32,
            _ => 0,
        }
    }

    pub fn halted(&self) -> bool {
        self.circuit.as_ref().is_some_and(machine::halted)
    }

    /// Values for newline-separated wire keys: 0, 1, or 2 if the key is unknown.
    /// A wire inside a built-in gate (only in the NAND and transistor views)
    /// is computed from the gate's inputs with its NAND definition; a wire
    /// inside a built-in flip-flop from its D, CLK and Q with the latch version.
    pub fn values(&self, keys: &str) -> Vec<u8> {
        let Some(c) = &self.circuit else {
            return Vec::new();
        };
        keys.split('\n')
            .map(|k| match c.wire_value(k) {
                Some(v) => v as u8,
                None => self.inner_value(c, k).map_or(2, |v| v as u8),
            })
            .collect()
    }
}

impl Sim {
    /// Value of `key` inside a built-in gate or flip-flop, e.g.
    /// "Alu.G3.N1.out" in the gate "Alu.G3": its NAND definition is set to
    /// the gate's input values and evaluated. `None` if `key` is not inside
    /// a built-in gate or flip-flop.
    fn inner_value(&self, c: &Circuit, key: &str) -> Option<bool> {
        // The outermost flip-flop or innermost gate instance that is a prefix of the key.
        let prefixes = || key.match_indices('.').map(|(i, _)| (&key[..i], &key[i + 1..]));
        if let Some((flop, rest)) = prefixes().find(|(p, _)| c.flop(p).is_some()) {
            return self.flop_inner_value(c, flop, rest);
        }
        let (gate, rest) = prefixes().filter(|(p, _)| c.gate_kind(p).is_some()).last()?;
        let kind = c.gate_kind(gate)?;
        let mut cells = self.nand_cells.borrow_mut();
        let cell = cells.entry(kind).or_insert_with(|| self.builder.build_nand(kind.name()));
        for pin in kind.inputs() {
            let v = c.wire_value(&format!("{gate}.{pin}"))?;
            cell.set_input(pin, v);
        }
        cell.evaluate_stable();
        // The one-gate wrapper names its gate "G".
        cell.wire_value(&format!("G.{rest}"))
    }

    /// Value of `rest` inside the built-in flip-flop `flop`: the latch
    /// version (from NAND gates) is brought into the same state. Its master
    /// latch holds D while CLK is 0 and the sampled Q while CLK is 1.
    fn flop_inner_value(&self, c: &Circuit, flop: &str, rest: &str) -> Option<bool> {
        let f = c.flop(flop)?;
        let (d, clk, q) = (c.value(f.d), c.value(f.clk), c.value(f.q));
        let mut shadow = self.flop_cell.borrow_mut();
        let cell = shadow.get_or_insert_with(|| self.builder.build_nand(FLIP_FLOP));
        let step = |pins: &[(&str, bool)]| {
            for &(pin, v) in pins {
                cell.set_input(pin, v);
            }
            cell.evaluate_stable();
        };
        // Both latches to Q, then the clock and D as they are now.
        step(&[("D", q), ("CLK", false)]);
        step(&[("CLK", true)]);
        step(&[("CLK", false)]);
        step(&[("CLK", clk)]);
        step(&[("D", d)]);
        cell.wire_value(rest)
    }
}

impl Default for Sim {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wires inside built-in flip-flops (only the latch version has them)
    /// read the same as in the latch version, at every phase of the clock.
    #[test]
    fn values_inside_flip_flops_match_latches() {
        let mut sim = Sim::new();
        sim.build("SYNC<2>");
        let slow = sim.builder.build_latches("SYNC<2>");
        let fast = sim.circuit.as_ref().unwrap();
        let mut inner: Vec<&str> = slow.wire_names().map(|(k, _)| k).filter(|k| !fast.has_wire(k)).collect();
        inner.sort();
        assert!(inner.iter().any(|k| k.contains("LATCH_M")));
        let keys = inner.join("\n");
        for step in 0..24 {
            let (pin, v) = match step % 4 {
                0 => ("D.0", step % 8 == 0),
                1 => ("D.1", step % 12 != 1),
                2 => ("CLK", true),
                _ => ("CLK", false),
            };
            for c in [fast, &slow] {
                c.set_input(pin, v);
                c.evaluate_stable();
            }
            let want: Vec<u8> = inner.iter().map(|k| slow.get_wire_value(k) as u8).collect();
            let got = sim.values(&keys);
            let bad: Vec<_> = inner.iter().zip(got.iter().zip(&want)).filter(|(_, (g, w))| g != w).map(|(k, (g, w))| format!("{k}={g}/{w}")).collect();
            assert!(bad.is_empty(), "step {step}: {bad:?}");
        }
    }
}
