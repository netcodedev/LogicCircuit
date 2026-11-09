use super::{CircuitBuilder, CldlDefinition, IoPinDef, PinRef};

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

/// A single, non-recursive level of a circuit definition: its own IO pins,
/// its direct parts and the connections between them.
#[derive(Debug, Clone)]
pub struct CircuitView {
    pub name: String,
    pub inputs: Vec<PinView>,
    pub outputs: Vec<PinView>,
    pub parts: Vec<PartView>,
    pub connections: Vec<ConnectionView>,
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
        let mut names: Vec<String> = self.definitions.keys().cloned().collect();
        names.sort();
        names
    }

    /// Describes one level of the named circuit for visualization.
    /// Returns `None` if no such definition exists.
    pub fn describe(&self, name: &str) -> Option<CircuitView> {
        let def = self.definitions.get(name)?;

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
                    }
                } else {
                    let sub_def = self.definitions.get(&part.part_type);
                    PartView {
                        instance_name: part.instance_name.clone(),
                        part_type: part.part_type.clone(),
                        inputs: sub_def.map_or_else(Vec::new, |d| pin_views(&d.inputs)),
                        outputs: sub_def.map_or_else(Vec::new, |d| pin_views(&d.outputs)),
                        expandable: sub_def.is_some_and(|d| !d.parts.is_empty()),
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
                        .get_pin_width(&conn.source_pin.base_name, def)
                        .or_else(|| self.get_pin_width(&conn.dest_pin.base_name, def)),
                    _ => None,
                };
                ConnectionView {
                    source: endpoint(&conn.source_pin, def),
                    dest: endpoint(&conn.dest_pin, def),
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
