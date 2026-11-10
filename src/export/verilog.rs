//! Structural Verilog export of a flattened circuit.
//!
//! Every block of `CircuitBuilder::flatten_until` becomes one cell instance per
//! bit (multi-bit basic blocks like `AND_4B` are split), every net one wire.
//! Two cell libraries are supported:
//!
//! - `CellLib::Generic`: our own `LC_*` cell modules, appended to the output so
//!   the file simulates on its own (Icarus, Verilator). Each cell output starts
//!   at the value our simulator has after building the circuit (`INIT`
//!   parameter), so latches and flip-flops begin in the same state. Cells get
//!   small, varying delays (`DELAY`, 1-3 ps): with zero delay, cross-coupled
//!   latch gates switching at the same instant oscillate forever.
//! - `CellLib::Sky130`: `sky130_fd_sc_hd` standard cells for OpenROAD.
//!
//! Names: nets become `n_<key>` and instances `u_<path>`, with `.` replaced by
//! `__` and any other character outside `[A-Za-z0-9_]` by `_`. The mapping
//! back to our keys is returned in `VerilogNetlist::names`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write;

use crate::circuit::Circuit;
use crate::cldl::{CircuitBuilder, FlatBlock, FlatLevel};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellLib {
    Generic,
    Sky130,
}

#[derive(Debug, Clone, Copy)]
pub struct VerilogOptions {
    pub level: FlatLevel,
    pub cells: CellLib,
    /// Keep flip-flops as our own latch gates instead of mapping `DFLIP_FLOP`
    /// to a flip-flop cell.
    pub keep_latches: bool,
    /// Map every `MUX_2_1` to one multiplexer cell (sky130 `mux2_1`, about
    /// half the area of its NOT/AND/AND/OR gates) instead of its gates. Only
    /// at `FlatLevel::Basic`; used for chip builds, while the views and the
    /// plain export keep showing the gates.
    pub mux_cells: bool,
}

impl Default for VerilogOptions {
    fn default() -> Self {
        Self { level: FlatLevel::Basic, cells: CellLib::Sky130, keep_latches: false, mux_cells: false }
    }
}

/// The exported netlist plus what is needed to refer back into it.
pub struct VerilogNetlist {
    pub text: String,
    pub module: String,
    /// Verilog name -> our key (nets: wire key, instances: hierarchy path).
    pub names: BTreeMap<String, String>,
    /// Every wire key of the circuit -> net index.
    pub aliases: HashMap<String, usize>,
    /// Verilog expression for each net that appears in the netlist.
    pub net_signal: HashMap<usize, String>,
    /// Top-level ports: (name, width, is_output).
    pub ports: Vec<(String, Option<usize>, bool)>,
    /// Cell instances per cell type.
    pub cell_counts: BTreeMap<String, usize>,
    /// Value of every top-level input bit after building (generic export
    /// only): the state the netlist's INIT values correspond to.
    pub input_init: HashMap<String, bool>,
}

/// One per-bit cell: our pin names (inputs in cell order, then output) and
/// the cell's pin names in the same order.
struct CellMap {
    generic: &'static str,
    sky130: &'static str,
    ours_in: &'static [&'static str],
    ours_out: &'static str,
    cell_in: &'static [&'static str],
    cell_out_generic: &'static str,
    cell_out_sky130: &'static str,
}

const fn cell(
    generic: &'static str,
    sky130: &'static str,
    ours_in: &'static [&'static str],
    ours_out: &'static str,
    cell_in: &'static [&'static str],
    cell_out_sky130: &'static str,
) -> CellMap {
    CellMap { generic, sky130, ours_in, ours_out, cell_in, cell_out_generic: "Y", cell_out_sky130 }
}

const TWO: &[&str] = &["in1", "in2"];
const AB: &[&str] = &["A", "B"];

/// Block type -> cell. Multi-bit blocks (AND_4B, ...) use the single-bit
/// entry per bit with pins A[i], B[i] -> Out[i].
fn cell_for(kind: &str) -> Option<CellMap> {
    Some(match kind {
        "NAND" => cell("LC_NAND2", "sky130_fd_sc_hd__nand2_1", TWO, "out", AB, "Y"),
        "NOT" => cell("LC_INV", "sky130_fd_sc_hd__inv_1", &["in"], "out", &["A"], "Y"),
        "AND" => cell("LC_AND2", "sky130_fd_sc_hd__and2_1", TWO, "out", AB, "X"),
        "AND3" => cell("LC_AND3", "sky130_fd_sc_hd__and3_1", &["A", "B", "C"], "Out", &["A", "B", "C"], "X"),
        "OR" => cell("LC_OR2", "sky130_fd_sc_hd__or2_1", TWO, "out", AB, "X"),
        "NOR" => cell("LC_NOR2", "sky130_fd_sc_hd__nor2_1", TWO, "out", AB, "Y"),
        "NOR4" => cell("LC_NOR4", "sky130_fd_sc_hd__nor4_1", &["A", "B", "C", "D"], "Out", &["A", "B", "C", "D"], "Y"),
        "XOR" => cell("LC_XOR2", "sky130_fd_sc_hd__xor2_1", TWO, "out", AB, "X"),
        "XNOR" => cell("LC_XNOR2", "sky130_fd_sc_hd__xnor2_1", TWO, "out", AB, "Y"),
        // Out = S ? B : A; mux2_1: X = S ? A1 : A0 (only with `mux_cells`)
        MUX_BLOCK => cell("LC_MUX2", "sky130_fd_sc_hd__mux2_1", &["A", "B", "S"], "Out", &["A0", "A1", "S"], "X"),
        _ => return None,
    })
}

/// The 2:1 multiplexer block mapped to a cell with `VerilogOptions::mux_cells`.
const MUX_BLOCK: &str = "MUX_2_1";

/// Single-bit type a multi-bit basic block is made of.
fn bitwise_base(kind: &str) -> Option<&'static str> {
    match kind {
        "AND_4B" | "AND_8B" => Some("AND"),
        "OR_4B" | "OR_8B" => Some("OR"),
        "XOR_4B" | "XOR_8B" => Some("XOR"),
        _ => None,
    }
}

const VERILOG_KEYWORDS: &[&str] = &[
    "always", "and", "assign", "begin", "buf", "case", "default", "else", "end", "endcase",
    "endmodule", "for", "function", "if", "initial", "inout", "input", "integer", "module",
    "nand", "nor", "not", "or", "output", "parameter", "reg", "supply0", "supply1", "task",
    "wire", "xnor", "xor",
];

fn sanitize(key: &str) -> String {
    key.replace('.', "__")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect()
}

pub(crate) fn port_name(name: &str) -> String {
    if VERILOG_KEYWORDS.contains(&name) { format!("\\{name} ") } else { name.to_string() }
}

/// Unique Verilog names with a fixed prefix.
struct Namer {
    used: HashSet<String>,
}

impl Namer {
    fn unique(&mut self, base: String) -> String {
        let mut name = base.clone();
        let mut n = 1;
        while !self.used.insert(name.clone()) {
            name = format!("{base}_{n}");
            n += 1;
        }
        name
    }
}

/// One cell instance in the making.
struct Instance {
    cell: String,
    name: String,
    params: Option<String>,
    pins: Vec<(String, String)>,
}

pub fn export_verilog(builder: &CircuitBuilder, top: &str, opts: &VerilogOptions) -> Result<VerilogNetlist, String> {
    let view = builder.describe(top).ok_or_else(|| format!("Circuit '{top}' not found"))?;
    let map_flops = !opts.keep_latches;
    let map_muxes = opts.mux_cells && opts.level == FlatLevel::Basic;
    let stop = |t: &str| {
        (opts.level == FlatLevel::Basic && builder.is_basic(t))
            || (map_flops && t == "DFLIP_FLOP")
            || (map_muxes && t == MUX_BLOCK)
    };
    let flat = builder.flatten_until(top, &stop).ok_or_else(|| format!("Circuit '{top}' not found"))?;

    // Initial state for the generic cells.
    let built: Option<Circuit> = (opts.cells == CellLib::Generic).then(|| {
        let c = builder.build(top);
        c.evaluate_stable();
        c
    });
    let init_of = |net: usize| -> u8 {
        built.as_ref().map_or(0, |c| {
            c.get_wire_map().get(&flat.nets[net]).is_some_and(|w| w.borrow().value) as u8
        })
    };

    let mut names = BTreeMap::new();
    let mut namer = Namer { used: HashSet::new() };
    let mut ports = Vec::new();
    for (pins, output) in [(&view.inputs, false), (&view.outputs, true)] {
        for p in pins {
            namer.used.insert(port_name(&p.name));
            ports.push((p.name.clone(), p.width, output));
        }
    }

    // Port bit expression for a key like "A.3" or "CLK".
    let port_bit = |key: &str| -> String {
        let width_of = |name: &str| ports.iter().find(|(n, _, _)| n == name).and_then(|p| p.1);
        match key.rsplit_once('.') {
            Some((name, bit)) if width_of(name).is_some() => format!("{}[{bit}]", port_name(name)),
            _ => port_name(key),
        }
    };

    // --- Net naming: a net carrying a top-level input is that input; one
    // carrying an output is that output; anything else is an internal wire.
    let mut net_signal: HashMap<usize, String> = HashMap::new();
    let mut extra_outputs: Vec<(usize, String)> = Vec::new(); // more output bits on an already named net
    for (key, net) in &flat.inputs {
        net_signal.entry(*net).or_insert_with(|| port_bit(key));
    }
    for (key, net) in &flat.outputs {
        if net_signal.contains_key(net) {
            extra_outputs.push((*net, port_bit(key)));
        } else {
            net_signal.insert(*net, port_bit(key));
        }
    }
    let mut wires: Vec<String> = Vec::new();
    let mut signal = |net: usize, net_signal: &mut HashMap<usize, String>, names: &mut BTreeMap<String, String>| -> String {
        net_signal
            .entry(net)
            .or_insert_with(|| {
                let name = namer.unique(format!("n_{}", sanitize(&flat.nets[net])));
                names.insert(name.clone(), flat.nets[net].clone());
                wires.push(name.clone());
                name
            })
            .clone()
    };

    // --- Cells.
    let mut instances: Vec<Instance> = Vec::new();
    let mut driven: HashSet<usize> = flat.inputs.iter().map(|(_, n)| *n).collect();
    let mut loads: Vec<usize> = flat.outputs.iter().map(|(_, n)| *n).collect();
    let mut inst_namer = Namer { used: HashSet::new() };

    let pin_nets = |b: &FlatBlock, pin: &str| -> Result<Vec<usize>, String> {
        b.pins
            .iter()
            .find(|p| p.name == pin)
            .map(|p| p.nets.clone())
            .ok_or_else(|| format!("{} ({}) has no pin '{pin}'", b.id, b.kind))
    };

    for b in &flat.blocks {
        // (cell map, per-bit our-pin -> net) for each emitted cell
        let mut emit: Vec<(String, String, Vec<(String, usize)>, Option<String>)> = Vec::new();
        if b.kind == "DFLIP_FLOP" {
            let d = pin_nets(b, "D")?[0];
            let clk = pin_nets(b, "CLK")?[0];
            let q = pin_nets(b, "Q")?[0];
            let nq = pin_nets(b, "nQ")?[0];
            let (cell, mut pins) = match opts.cells {
                CellLib::Generic => ("LC_DFF".to_string(), vec![("CLK".into(), clk), ("D".into(), d), ("Q".into(), q), ("QN".into(), nq)]),
                CellLib::Sky130 => (
                    "sky130_fd_sc_hd__dfxbp_1".to_string(),
                    vec![("CLK".into(), clk), ("D".into(), d), ("Q".into(), q), ("Q_N".into(), nq)],
                ),
            };
            // Without a load on nQ the cheaper flip-flop without Q_N will do.
            let nq_used = flat.blocks.iter().any(|o| o.pins.iter().any(|p| !p.output && p.nets.contains(&nq)))
                || flat.outputs.iter().any(|(_, n)| *n == nq);
            if !nq_used && opts.cells == CellLib::Sky130 {
                pins.pop();
                emit.push(("sky130_fd_sc_hd__dfxtp_1".into(), b.id.clone(), pins, None));
            } else {
                let params = (opts.cells == CellLib::Generic).then(|| format!("#(.INIT(1'b{}), .DELAY(@D))", init_of(q)));
                emit.push((cell, b.id.clone(), pins, params));
            }
            driven.insert(q);
            driven.insert(nq);
            loads.extend([d, clk]);
        } else {
            let (base, bits) = match bitwise_base(&b.kind) {
                Some(base) => (base, pin_nets(b, "Out")?.len()),
                None => (b.kind.as_str(), 1),
            };
            let cm = cell_for(base).ok_or_else(|| {
                format!("No cell mapping for '{}' ({}); use --level nand or add it to the cell table", b.kind, b.id)
            })?;
            for bit in 0..bits {
                let (ours_in, ours_out): (Vec<String>, String) = if bits > 1 || bitwise_base(&b.kind).is_some() {
                    (vec!["A".into(), "B".into()], "Out".into())
                } else {
                    (cm.ours_in.iter().map(|s| s.to_string()).collect(), cm.ours_out.to_string())
                };
                let net_of = |pin: &str| -> Result<usize, String> { Ok(pin_nets(b, pin)?[bit]) };
                let mut pins = Vec::new();
                for (ours, theirs) in ours_in.iter().zip(cm.cell_in) {
                    let n = net_of(ours)?;
                    loads.push(n);
                    pins.push((theirs.to_string(), n));
                }
                let out = net_of(&ours_out)?;
                driven.insert(out);
                let out_pin = match opts.cells {
                    CellLib::Generic => cm.cell_out_generic,
                    CellLib::Sky130 => cm.cell_out_sky130,
                };
                pins.push((out_pin.to_string(), out));
                let (cell, params) = match opts.cells {
                    CellLib::Generic => (cm.generic.to_string(), Some(format!("#(.INIT(1'b{}), .DELAY(@D))", init_of(out)))),
                    CellLib::Sky130 => (cm.sky130.to_string(), None),
                };
                let id = if bits > 1 { format!("{}.{bit}", b.id) } else { b.id.clone() };
                emit.push((cell, id, pins, params));
            }
        }
        for (cell, id, pins, params) in emit {
            let name = inst_namer.unique(format!("u_{}", sanitize(&id)));
            names.insert(name.clone(), id);
            let pins = pins
                .into_iter()
                .map(|(pin, net)| (pin, signal(net, &mut net_signal, &mut names)))
                .collect();
            instances.push(Instance { cell, name, params, pins });
        }
    }

    // --- Tie-low cells for nets that are read but never driven (they read 0
    // in our simulator), and buffers for output bits sharing a net.
    let mut undriven: Vec<usize> = loads.iter().copied().filter(|n| !driven.contains(n)).collect();
    undriven.sort_unstable();
    undriven.dedup();
    for net in undriven {
        let sig = signal(net, &mut net_signal, &mut names);
        let name = inst_namer.unique(format!("u_tie_{}", sanitize(&flat.nets[net])));
        names.insert(name.clone(), format!("(tie-low) {}", flat.nets[net]));
        let (cell, pin) = match opts.cells {
            CellLib::Generic => ("LC_TIELO", "Y"),
            CellLib::Sky130 => ("sky130_fd_sc_hd__conb_1", "LO"),
        };
        instances.push(Instance { cell: cell.into(), name, params: None, pins: vec![(pin.into(), sig)] });
    }
    for (net, port) in extra_outputs {
        let sig = signal(net, &mut net_signal, &mut names);
        let name = inst_namer.unique(format!("u_buf_{}", sanitize(&port)));
        names.insert(name.clone(), format!("(buffer) {port}"));
        let (cell, a, y, params) = match opts.cells {
            CellLib::Generic => ("LC_BUF", "A", "Y", Some(format!("#(.INIT(1'b{}), .DELAY(@D))", init_of(net)))),
            CellLib::Sky130 => ("sky130_fd_sc_hd__buf_1", "A", "X", None),
        };
        instances.push(Instance { cell: cell.into(), name, params, pins: vec![(a.into(), sig), (y.into(), port)] });
    }

    // --- Text.
    let module = sanitize(top);
    let mut t = String::new();
    let lib = match opts.cells {
        CellLib::Generic => "generic LC_* cells (appended below)",
        CellLib::Sky130 => "sky130_fd_sc_hd standard cells",
    };
    let level = match opts.level {
        FlatLevel::Nand => "NAND gates",
        FlatLevel::Basic => "basic blocks",
    };
    if opts.cells == CellLib::Generic {
        writeln!(t, "`timescale 1ns/1ps").unwrap();
    }
    writeln!(t, "// {top}: exported by LogicCircuit as {level}, {lib}.").unwrap();
    writeln!(t, "// Flip-flops: {}.", if map_flops { "mapped to flip-flop cells" } else { "kept as latch gates" }).unwrap();
    if map_muxes {
        writeln!(t, "// 2:1 multiplexers (MUX_2_1): mapped to multiplexer cells.").unwrap();
    }
    let port_list: Vec<String> = ports.iter().map(|(n, _, _)| port_name(n)).collect();
    writeln!(t, "module {module} ({});", port_list.join(", ")).unwrap();
    for (name, width, output) in &ports {
        let dir = if *output { "output" } else { "input" };
        match width {
            Some(w) => writeln!(t, "    {dir} [{}:0] {};", w - 1, port_name(name)).unwrap(),
            None => writeln!(t, "    {dir} {};", port_name(name)).unwrap(),
        }
    }
    writeln!(t).unwrap();
    for w in &wires {
        writeln!(t, "    wire {w};").unwrap();
    }
    writeln!(t).unwrap();
    let mut cell_counts = BTreeMap::new();
    for (i, inst) in instances.iter().enumerate() {
        *cell_counts.entry(inst.cell.clone()).or_insert(0) += 1;
        let pins: Vec<String> = inst.pins.iter().map(|(p, s)| format!(".{p}({s})")).collect();
        let params = inst
            .params
            .as_deref()
            .map(|p| format!(" {}", p.replace("@D", &(1 + i % 3).to_string())))
            .unwrap_or_default();
        writeln!(t, "    {}{params} {} ({});", inst.cell, inst.name, pins.join(", ")).unwrap();
    }
    writeln!(t, "endmodule").unwrap();
    if opts.cells == CellLib::Generic {
        t.push('\n');
        t.push_str(GENERIC_CELLS);
    }

    let input_init = flat
        .inputs
        .iter()
        .map(|(key, net)| (key.clone(), init_of(*net) == 1))
        .collect();
    Ok(VerilogNetlist { text: t, module, names, aliases: flat.aliases, net_signal, ports, cell_counts, input_init })
}

/// Behavioural models of the generic cells. Outputs are registers with an
/// initial value (INIT) so feedback loops start in a defined state.
pub const GENERIC_CELLS: &str = r#"// --- LogicCircuit generic cells ---------------------------------------------
// Delays are in ps; outputs start at INIT.
`timescale 1ps/1ps
module LC_NAND2 #(parameter INIT = 1'b0, parameter DELAY = 1) (input A, input B, output reg Y);
    initial Y = INIT;
    always @(A or B) Y <= #(DELAY) ~(A & B);
endmodule
module LC_INV #(parameter INIT = 1'b0, parameter DELAY = 1) (input A, output reg Y);
    initial Y = INIT;
    always @(A) Y <= #(DELAY) ~A;
endmodule
module LC_BUF #(parameter INIT = 1'b0, parameter DELAY = 1) (input A, output reg Y);
    initial Y = INIT;
    always @(A) Y <= #(DELAY) A;
endmodule
module LC_AND2 #(parameter INIT = 1'b0, parameter DELAY = 1) (input A, input B, output reg Y);
    initial Y = INIT;
    always @(A or B) Y <= #(DELAY) A & B;
endmodule
module LC_AND3 #(parameter INIT = 1'b0, parameter DELAY = 1) (input A, input B, input C, output reg Y);
    initial Y = INIT;
    always @(A or B or C) Y <= #(DELAY) A & B & C;
endmodule
module LC_OR2 #(parameter INIT = 1'b0, parameter DELAY = 1) (input A, input B, output reg Y);
    initial Y = INIT;
    always @(A or B) Y <= #(DELAY) A | B;
endmodule
module LC_NOR2 #(parameter INIT = 1'b0, parameter DELAY = 1) (input A, input B, output reg Y);
    initial Y = INIT;
    always @(A or B) Y <= #(DELAY) ~(A | B);
endmodule
module LC_NOR4 #(parameter INIT = 1'b0, parameter DELAY = 1) (input A, input B, input C, input D, output reg Y);
    initial Y = INIT;
    always @(A or B or C or D) Y <= #(DELAY) ~(A | B | C | D);
endmodule
module LC_XOR2 #(parameter INIT = 1'b0, parameter DELAY = 1) (input A, input B, output reg Y);
    initial Y = INIT;
    always @(A or B) Y <= #(DELAY) A ^ B;
endmodule
module LC_XNOR2 #(parameter INIT = 1'b0, parameter DELAY = 1) (input A, input B, output reg Y);
    initial Y = INIT;
    always @(A or B) Y <= #(DELAY) ~(A ^ B);
endmodule
module LC_MUX2 #(parameter INIT = 1'b0, parameter DELAY = 1) (input A0, input A1, input S, output reg Y);
    initial Y = INIT;
    always @(A0 or A1 or S) Y <= #(DELAY) (S ? A1 : A0);
endmodule
module LC_TIELO (output Y);
    assign Y = 1'b0;
endmodule
// Rising-edge D flip-flop (our DFLIP_FLOP). QN is ~Q.
module LC_DFF #(parameter INIT = 1'b0, parameter DELAY = 1) (input CLK, input D, output reg Q, output QN);
    initial Q = INIT;
    always @(posedge CLK) Q <= #(DELAY) D;
    assign QN = ~Q;
endmodule
"#;
