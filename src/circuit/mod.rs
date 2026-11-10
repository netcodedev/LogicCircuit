use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use uuid::Uuid;

mod circuit;
mod gates;

pub use gates::{Gate, GateKind};

/// A struct to hold the wire's value and its unique ID
pub struct WireData {
    pub id: Uuid,
    pub value: bool,
}

pub type Wire = Rc<RefCell<WireData>>;

pub trait LogicGate: std::fmt::Debug {
    fn evaluate(&self) -> bool;
}

#[derive(Debug)]
pub struct Circuit {
    name: String,
    gates: Vec<Box<dyn LogicGate>>,
    inputs: HashMap<String, Wire>,
    outputs: HashMap<String, Wire>,
    wire_map: HashMap<String, Wire>,
    /// Instance path -> type of every built-in gate.
    gate_kinds: HashMap<String, GateKind>,
}

/// Creates a new, unique wire with a v4 UUID.
pub fn wire() -> Wire {
    Rc::new(RefCell::new(WireData {
        id: Uuid::new_v4(),
        value: false,
    }))
}

impl std::fmt::Debug for WireData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state_char = if self.value { '●' } else { '○' }; // ● for on, ○ for off
        write!(f, "({} {})", state_char, self.id)
    }
}

