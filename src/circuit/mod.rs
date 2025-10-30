use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;
use uuid::Uuid;

mod circuit;

/// A struct to hold the wire's value and its unique ID
pub struct WireData {
    pub id: Uuid,
    pub value: bool,
}

pub type Wire = Rc<RefCell<WireData>>;

pub trait LogicGate: std::fmt::Debug {
    fn evaluate(&self) -> bool;
}

pub struct NandGate {
    pub id: String,
    pub in1: Wire,
    pub in2: Wire,
    pub out: Wire,
}

#[derive(Debug)]
pub struct Circuit {
    name: String,
    gates: Vec<Box<dyn LogicGate>>,
    inputs: HashMap<String, Wire>,
    outputs: HashMap<String, Wire>,
    wire_map: HashMap<String, Wire>,
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

impl fmt::Debug for NandGate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NandGate")
            .field("id", &self.id)
            .field("in1", &self.in1)
            .field("in2", &self.in2)
            .field("out", &self.out)
            .finish()
    }
}

impl LogicGate for NandGate {
    fn evaluate(&self) -> bool {
        let val1 = self.in1.borrow().value;
        let val2 = self.in2.borrow().value;

        let new_val = !(val1 && val2);

        let old_val = self.out.borrow().value;
        self.out.borrow_mut().value = new_val;

        let changed = new_val != old_val;

        changed
    }
}
