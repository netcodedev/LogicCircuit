//! The built-in logic gates: the cells a CMOS process offers directly
//! (sky130: nand2, inv, and2, and3, or2, nor2, nor4, xor2, xnor2). The
//! simulator evaluates them in Rust; each one (except NAND itself) also has
//! a CLDL definition built only from NAND gates, `<NAME>_NAND` in
//! `src/circuits/nand/`, used for the NAND views, NAND counts and the NAND
//! netlist export.

use super::WireId;

/// A built-in gate type. Pin names match the CLDL parts that used to define
/// them (`in1`/`in2`/`out`, `in`/`out` for NOT, `A`..`D`/`Out` for AND3 and
/// NOR4), so existing circuits connect unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GateKind {
    Nand,
    Not,
    And,
    And3,
    Or,
    Nor,
    Nor4,
    Xor,
    Xnor,
}

const TWO: &[&str] = &["in1", "in2"];

impl GateKind {
    pub const ALL: [GateKind; 9] = [
        GateKind::Nand,
        GateKind::Not,
        GateKind::And,
        GateKind::And3,
        GateKind::Or,
        GateKind::Nor,
        GateKind::Nor4,
        GateKind::Xor,
        GateKind::Xnor,
    ];

    /// The gate for a CLDL part type, e.g. "AND".
    pub fn from_name(name: &str) -> Option<GateKind> {
        GateKind::ALL.into_iter().find(|k| k.name() == name)
    }

    /// CLDL part type name.
    pub fn name(self) -> &'static str {
        match self {
            GateKind::Nand => "NAND",
            GateKind::Not => "NOT",
            GateKind::And => "AND",
            GateKind::And3 => "AND3",
            GateKind::Or => "OR",
            GateKind::Nor => "NOR",
            GateKind::Nor4 => "NOR4",
            GateKind::Xor => "XOR",
            GateKind::Xnor => "XNOR",
        }
    }

    /// Input pin names, in evaluation order.
    pub fn inputs(self) -> &'static [&'static str] {
        match self {
            GateKind::Not => &["in"],
            GateKind::And3 => &["A", "B", "C"],
            GateKind::Nor4 => &["A", "B", "C", "D"],
            _ => TWO,
        }
    }

    /// Output pin name.
    pub fn output(self) -> &'static str {
        match self {
            GateKind::And3 | GateKind::Nor4 => "Out",
            _ => "out",
        }
    }

    /// The gate's function.
    pub fn eval(self, i: &[bool]) -> bool {
        match self {
            GateKind::Nand => !(i[0] && i[1]),
            GateKind::Not => !i[0],
            GateKind::And => i[0] && i[1],
            GateKind::And3 => i[0] && i[1] && i[2],
            GateKind::Or => i[0] || i[1],
            GateKind::Nor => !(i[0] || i[1]),
            GateKind::Nor4 => !(i[0] || i[1] || i[2] || i[3]),
            GateKind::Xor => i[0] ^ i[1],
            GateKind::Xnor => !(i[0] ^ i[1]),
        }
    }

    /// Name of the CLDL definition that builds this gate from NAND gates
    /// (same pins), `None` for NAND.
    pub fn nand_definition(self) -> Option<&'static str> {
        Some(match self {
            GateKind::Nand => return None,
            GateKind::Not => "NOT_NAND",
            GateKind::And => "AND_NAND",
            GateKind::And3 => "AND3_NAND",
            GateKind::Or => "OR_NAND",
            GateKind::Nor => "NOR_NAND",
            GateKind::Nor4 => "NOR4_NAND",
            GateKind::Xor => "XOR_NAND",
            GateKind::Xnor => "XNOR_NAND",
        })
    }

    /// CLDL source of the definition that makes the gate usable as a circuit
    /// of its own (top level of a test, the viewer): one part of this kind.
    pub(crate) fn wrapper_source(self) -> String {
        let ins = self.inputs();
        let out = self.output();
        let mut src = format!(
            "CIRCUIT {} {{\n    // Built-in gate, simulated in Rust; from NAND gates: {}.\n    INPUT {};\n    OUTPUT {out};\n    PART G: {};\n",
            self.name(),
            self.nand_definition().unwrap_or("-"),
            ins.join(", "),
            self.name(),
        );
        for pin in ins {
            src.push_str(&format!("    CONNECT {pin} -> G.{pin};\n"));
        }
        src.push_str(&format!("    CONNECT G.{out} -> {out};\n}}\n"));
        src
    }
}

/// One built-in gate in a built circuit (as handed to `Circuit::add_builtin`).
#[derive(Debug, Clone)]
pub struct Gate {
    pub id: String,
    pub kind: GateKind,
    pub inputs: Vec<WireId>,
    pub out: WireId,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cldl::CircuitBuilder;

    /// Every built-in gate computes the same as its NAND definition, for all
    /// inputs, and has the same pins.
    #[test]
    fn built_in_gates_match_their_nand_definitions() {
        let builder = CircuitBuilder::new();
        for kind in GateKind::ALL {
            let name = kind.name();
            assert_eq!(GateKind::from_name(name), Some(kind));
            let native = if kind == GateKind::Nand { None } else { Some(builder.build(name)) };
            let nand = kind.nand_definition().map(|d| builder.build(d));
            let ins = kind.inputs();
            for bits in 0..1u32 << ins.len() {
                let values: Vec<bool> = (0..ins.len()).map(|i| bits >> i & 1 == 1).collect();
                let want = kind.eval(&values);
                for circuit in native.iter().chain(nand.iter()) {
                    for (pin, &v) in ins.iter().zip(&values) {
                        circuit.set_input(pin, v);
                    }
                    circuit.evaluate_stable();
                    assert_eq!(circuit.get_output(kind.output()), Some(want), "{} {:?}", circuit.get_name(), values);
                }
            }
            if let Some(n) = &nand {
                assert_eq!(n.gate_count(), builder.nand_count(name).unwrap(), "{name}");
            }
        }
        assert_eq!(GateKind::Nand.eval(&[true, true]), false);
    }
}
