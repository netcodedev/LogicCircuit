use std::collections::HashMap;

use super::{CircuitBuilder, CldlDefinition, IoPinDef, PinRef};
use crate::circuit::GateKind;

/// A single IO pin of a circuit or part. `width` is `Some(n)` for an n-bit bus.
#[derive(Debug, Clone)]
pub struct PinView {
    pub name: String,
    pub width: Option<usize>,
}

/// One `PART` instance inside a circuit definition.
#[derive(Debug, Clone)]
pub struct PartView {
    pub instance_name: String,
    pub part_type: String,
    pub inputs: Vec<PinView>,
    pub outputs: Vec<PinView>,
    /// True if the part has its own definition with sub-parts (i.e. it can be
    /// opened one level deeper). False for the NAND primitive.
    pub expandable: bool,
    /// NAND gates in one instance of this part (built-in gates count as
    /// their NAND definition; 0 if its type is unknown).
    pub nand_count: usize,
}

/// One side of a `CONNECT`. `part` is `None` when the pin belongs to the
/// enclosing circuit (an IO pin or an internal net).
#[derive(Debug, Clone)]
pub struct EndpointView {
    pub part: Option<String>,
    pub pin: String,
    pub index: Option<usize>,
}

impl EndpointView {
    /// The wire-map key of this endpoint relative to the enclosing circuit,
    /// without bus expansion (e.g. "G1.out", "Data.2").
    pub fn local_key(&self) -> String {
        let base = match &self.part {
            Some(part) => format!("{}.{}", part, self.pin),
            None => self.pin.clone(),
        };
        match self.index {
            Some(idx) => format!("{}.{}", base, idx),
            None => base,
        }
    }
}

/// One `CONNECT` statement. `width` is the bus width carried by the
/// connection, `None` for a single bit.
#[derive(Debug, Clone)]
pub struct ConnectionView {
    pub source: EndpointView,
    pub dest: EndpointView,
    pub width: Option<usize>,
}

/// How far `CircuitBuilder::flatten` expands the hierarchy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlatLevel {
    /// Down to the NAND gates the simulation runs on.
    Nand,
    /// Down to basic blocks (definitions from the `primitive` directory).
    Basic,
}

/// One pin of a flattened block. `nets` holds one net index per bit (LSB
/// first), indexing `FlatCircuit::nets`.
#[derive(Debug, Clone)]
pub struct FlatPin {
    pub name: String,
    pub output: bool,
    pub width: Option<usize>,
    pub nets: Vec<usize>,
}

/// One block of a flattened circuit: a NAND gate or a basic block.
#[derive(Debug, Clone)]
pub struct FlatBlock {
    /// Hierarchical instance path, e.g. "Cpu.Datapath.Alu.FA0.HA1.G1".
    pub id: String,
    /// Part type, e.g. "NAND", "XOR", "AND_4B".
    pub kind: String,
    pub pins: Vec<FlatPin>,
}

/// A circuit reduced to blocks and the nets connecting them.
#[derive(Debug, Clone)]
pub struct FlatCircuit {
    /// One wire-map key per net (usable to read the net's value).
    pub nets: Vec<String>,
    pub blocks: Vec<FlatBlock>,
    /// Top-level input bits (bus bits as "Name.i") and their nets.
    pub inputs: Vec<(String, usize)>,
    /// Top-level output bits and their nets.
    pub outputs: Vec<(String, usize)>,
    /// Every wire key of the build (e.g. "Cpu.Datapath.Alu.Z") and its net.
    pub aliases: HashMap<String, usize>,
}

/// A single, non-recursive level of a circuit definition: its own IO pins,
/// its direct parts and the connections between them.
#[derive(Debug, Clone)]
pub struct CircuitView {
    pub name: String,
    pub inputs: Vec<PinView>,
    pub outputs: Vec<PinView>,
    pub parts: Vec<PartView>,
    pub connections: Vec<ConnectionView>,
    /// NAND gates in the whole circuit.
    pub nand_count: usize,
}

fn pin_views(pins: &[IoPinDef]) -> Vec<PinView> {
    pins.iter()
        .map(|p| PinView {
            name: p.name.clone(),
            width: p.width,
        })
        .collect()
}

impl CircuitBuilder {
    /// Names of all known circuit definitions, sorted.
    pub fn circuit_names(&self) -> Vec<String> {
        // Elaborate the parametric parts used anywhere (e.g. `ADDER<8>`), so
        // they are listed too. Errors surface when the circuit is built.
        let plain: Vec<String> = self.definitions.borrow().keys().filter(|k| !k.contains('<')).cloned().collect();
        for name in &plain {
            let _ = self.check(name);
        }
        let mut names: Vec<String> = self
            .definitions
            .borrow()
            .iter()
            .filter(|(k, d)| **k == d.name)
            .map(|(k, _)| k.clone())
            .collect();
        names.sort();
        names
    }

    /// Number of NAND gates one instance of `name` is built from, i.e. the
    /// number of gates `build_nand(name)` creates (built-in gates count as
    /// their NAND definition). `None` if `name` or any part
    /// type below it is unknown.
    pub fn nand_count(&self, name: &str) -> Option<usize> {
        self.count_nands(name, &mut HashMap::new())
    }

    fn count_nands(&self, name: &str, memo: &mut HashMap<String, usize>) -> Option<usize> {
        if name == "NAND" {
            return Some(1);
        }
        // A built-in gate counts as the NAND gates of its NAND definition.
        if let Some(nand_def) = GateKind::from_name(name).and_then(GateKind::nand_definition) {
            return self.count_nands(nand_def, memo);
        }
        if let Some(&n) = memo.get(name) {
            return Some(n);
        }
        let def = self.def(name)?;
        let mut total = 0;
        for part in &def.parts {
            total += self.count_nands(&part.part_type, memo)?;
        }
        memo.insert(name.to_string(), total);
        Some(total)
    }

    /// Longest combinational path of `name` in NAND gates: from a top-level
    /// input or flip-flop output to a top-level output or flip-flop input.
    /// Flip-flops (`DFLIP_FLOP`) cut the paths; any other feedback loop is
    /// cut where the search meets it again. A rough, technology-independent
    /// measure for comparing designs (e.g. ripple vs. lookahead adders).
    pub fn logic_depth(&self, name: &str) -> Option<usize> {
        let flat = self.flatten_until(name, &|t: &str| t == "DFLIP_FLOP")?;
        // NAND output net -> its two input nets.
        let mut driver: HashMap<usize, [usize; 2]> = HashMap::new();
        let mut ends: Vec<usize> = flat.outputs.iter().map(|&(_, n)| n).collect();
        for block in &flat.blocks {
            let net = |pin: &str| block.pins.iter().find(|p| p.name == pin).map(|p| p.nets[0]);
            match block.kind.as_str() {
                "NAND" => {
                    driver.insert(net("out")?, [net("in1")?, net("in2")?]);
                }
                _ => ends.extend(net("D")),
            }
        }
        fn depth(net: usize, driver: &HashMap<usize, [usize; 2]>, memo: &mut HashMap<usize, Option<usize>>) -> usize {
            match memo.get(&net) {
                Some(Some(d)) => return *d,
                Some(None) => return 0, // on the current path: a loop
                None => {}
            }
            let Some(inputs) = driver.get(&net) else { return 0 };
            memo.insert(net, None);
            let d = 1 + inputs.iter().map(|&n| depth(n, driver, memo)).max().unwrap_or(0);
            memo.insert(net, Some(d));
            d
        }
        let mut memo = HashMap::new();
        Some(ends.into_iter().map(|n| depth(n, &driver, &mut memo)).max().unwrap_or(0))
    }

    /// Describes one level of the named circuit for visualization.
    /// Returns `None` if no such definition exists.
    pub fn describe(&self, name: &str) -> Option<CircuitView> {
        let def = self.def(name)?;
        let mut memo = HashMap::new();

        let parts = def
            .parts
            .iter()
            .map(|part| {
                if part.part_type == "NAND" {
                    let bit = |n: &str| PinView {
                        name: n.to_string(),
                        width: None,
                    };
                    PartView {
                        instance_name: part.instance_name.clone(),
                        part_type: part.part_type.clone(),
                        inputs: vec![bit("in1"), bit("in2")],
                        outputs: vec![bit("out")],
                        expandable: false,
                        nand_count: 1,
                    }
                } else {
                    let sub_def = self.def(&part.part_type);
                    let sub_def = sub_def.as_deref();
                    PartView {
                        instance_name: part.instance_name.clone(),
                        part_type: part.part_type.clone(),
                        inputs: sub_def.map_or_else(Vec::new, |d| pin_views(&d.inputs)),
                        outputs: sub_def.map_or_else(Vec::new, |d| pin_views(&d.outputs)),
                        // A built-in gate is a cell of its own (its NAND
                        // definition can be opened by name).
                        expandable: GateKind::from_name(&part.part_type).is_none()
                            && sub_def.is_some_and(|d| !d.parts.is_empty()),
                        nand_count: self.count_nands(&part.part_type, &mut memo).unwrap_or(0),
                    }
                }
            })
            .collect();

        let connections = def
            .connections
            .iter()
            .map(|conn| {
                let width = match (conn.source_pin.index, conn.dest_pin.index) {
                    (None, None) => self
                        .get_pin_width(&conn.source_pin.base_name, &def)
                        .or_else(|| self.get_pin_width(&conn.dest_pin.base_name, &def)),
                    _ => None,
                };
                ConnectionView {
                    source: endpoint(&conn.source_pin, &def),
                    dest: endpoint(&conn.dest_pin, &def),
                    width,
                }
            })
            .collect();

        Some(CircuitView {
            name: def.name.clone(),
            inputs: pin_views(&def.inputs),
            outputs: pin_views(&def.outputs),
            parts,
            connections,
            nand_count: self.count_nands(name, &mut memo).unwrap_or(0),
        })
    }
}

/// Splits a pin reference into part instance and pin name, using the same
/// `rfind('.')` rule as `get_pin_width`.
fn endpoint(pin_ref: &PinRef, def: &CldlDefinition) -> EndpointView {
    if let Some(dot_pos) = pin_ref.base_name.rfind('.') {
        let (part, pin) = pin_ref.base_name.split_at(dot_pos);
        if def.parts.iter().any(|p| p.instance_name == part) {
            return EndpointView {
                part: Some(part.to_string()),
                pin: pin[1..].to_string(),
                index: pin_ref.index,
            };
        }
    }
    EndpointView {
        part: None,
        pin: pin_ref.base_name.clone(),
        index: pin_ref.index,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn nand_count_matches_built_gates() {
        let mut builder = CircuitBuilder::new();
        builder
            .load_circuits_from_path(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/circuits")))
            .unwrap();
        for name in builder.circuit_names() {
            let counted = builder.nand_count(&name).unwrap();
            assert_eq!(counted, builder.build_nand(&name).gate_count(), "{name}");
            let nands = builder.flatten(&name, FlatLevel::Nand).unwrap().blocks;
            assert_eq!(counted, nands.len(), "{name}");
            // Basic blocks must add up to the same number of NANDs.
            let basic = builder.flatten(&name, FlatLevel::Basic).unwrap().blocks;
            let via_basic: usize = basic.iter().map(|b| builder.nand_count(&b.kind).unwrap()).sum();
            assert_eq!(counted, via_basic, "{name}");
        }
    }

    #[test]
    fn logic_depth_counts_nand_levels() {
        let builder = CircuitBuilder::new();
        assert_eq!(builder.logic_depth("NAND"), None);
        assert_eq!(builder.logic_depth("NOT"), Some(1));
        assert_eq!(builder.logic_depth("AND"), Some(2));
        // Flip-flops cut paths: a register is shallow despite its latches.
        assert!(builder.logic_depth("REGISTER<16>").unwrap() < 10);
        let ripple = builder.logic_depth("ADDER<16>").unwrap();
        let lookahead = builder.logic_depth("ADDER_CLA_16B").unwrap();
        assert!(lookahead * 4 < ripple, "{lookahead} vs {ripple}");
    }
}
