use include_dir::{Dir, DirEntry, include_dir};
use logic_circuit::circuit::Circuit;
use logic_circuit::cldl::{CircuitBuilder, CircuitView, EndpointView, PinView};
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

// The builder already embeds src/circuits; the top-level circuits/ directory
// (CPUs, datapaths) is only loaded at runtime by the CLI, so embed it here.
static EXTRA_CIRCUITS: Dir = include_dir!("$CARGO_MANIFEST_DIR/../circuits");

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
        "inputs": pins_json(&view.inputs),
        "outputs": pins_json(&view.outputs),
        "parts": view.parts.iter().map(|p| json!({
            "name": p.instance_name,
            "type": p.part_type,
            "inputs": pins_json(&p.inputs),
            "outputs": pins_json(&p.outputs),
            "expandable": p.expandable,
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
        }
    }

    /// JSON array of all circuit names.
    pub fn circuit_names(&self) -> String {
        json!(self.builder.circuit_names()).to_string()
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

    /// Builds `top` (running its initialization) and settles it.
    pub fn build(&mut self, top: &str) {
        let circuit = self.builder.build(top);
        circuit.evaluate_stable();
        self.circuit = Some(circuit);
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
