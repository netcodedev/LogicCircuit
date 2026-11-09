//! Verilog testbench from a .clt test suite, for checking an exported
//! netlist against our own simulator.
//!
//! The testbench mirrors `CircuitTester`: one simulation per suite, test
//! cases run back to back on the same state. SET drives inputs, EVAL and every
//! SEQ step advance time by 10 ns (plenty for ps cell delays to settle), ASSERT compares a net (any wire key of the
//! circuit, also internal ones) with `!==`, so X and Z count as failures.
//! Each failure prints a line starting with `LC_FAIL`, the end a line
//! `LC_RESULT checks=.. failures=..`.

use std::fmt::Write;

use crate::clt::{CltCommand, CltPinValue, CltTestSuite, CltValue};
use crate::export::verilog::{VerilogNetlist, port_name};

pub struct Testbench {
    pub text: String,
    pub checks: usize,
    /// Assertions that could not be checked, e.g. on nets inside a mapped
    /// flip-flop cell, with the reason.
    pub skipped: Vec<String>,
}

fn bit(v: &CltValue) -> u8 {
    matches!(v, CltValue::One) as u8
}

pub fn testbench(net: &VerilogNetlist, suite: &CltTestSuite) -> Testbench {
    let mut t = String::new();
    let mut checks = 0;
    let mut skipped = Vec::new();

    // Testbench signals are named like the ports.
    let port_expr = |key: &str| -> Option<String> {
        let (name, idx) = match key.rsplit_once('.') {
            Some((n, i)) if i.parse::<usize>().is_ok() => (n, Some(i)),
            _ => (key, None),
        };
        let (pname, width, _) = net.ports.iter().find(|(n, _, _)| n == name)?;
        match (width, idx) {
            (Some(_), Some(i)) => Some(format!("{}[{i}]", port_name(pname))),
            (None, None) => Some(port_name(pname)),
            _ => None,
        }
    };

    writeln!(t, "`timescale 1ns/1ps").unwrap();
    writeln!(t, "module tb;").unwrap();
    for (name, width, output) in &net.ports {
        let range = width.map(|w| format!("[{}:0] ", w - 1)).unwrap_or_default();
        if *output {
            writeln!(t, "    wire {range}{};", port_name(name)).unwrap();
        } else {
            writeln!(t, "    reg {range}{};", port_name(name)).unwrap();
        }
    }
    let conns: Vec<String> = net.ports.iter().map(|(n, _, _)| format!(".{p}({p})", p = port_name(n))).collect();
    writeln!(t, "    {} dut ({});", net.module, conns.join(", ")).unwrap();
    writeln!(t, "    integer checks = 0;\n    integer failures = 0;\n").unwrap();
    writeln!(t, "    initial begin").unwrap();

    // Inputs start where our simulator has them after building.
    let mut init: Vec<_> = net.input_init.iter().collect();
    init.sort();
    for (key, value) in init {
        if let Some(e) = port_expr(key) {
            writeln!(t, "        {e} = 1'b{};", *value as u8).unwrap();
        }
    }
    writeln!(t, "        #10;").unwrap();

    let set = |t: &mut String, pv: &CltPinValue| {
        let key = pv.pin_ref.to_canonical_key();
        match port_expr(&key) {
            Some(e) => writeln!(t, "        {e} = 1'b{};", bit(&pv.value)).unwrap(),
            None => writeln!(t, "        // SET {key}: not a top-level input, ignored").unwrap(),
        }
    };

    for case in &suite.test_cases {
        let label = case.name.replace(['"', '\\'], "'");
        writeln!(t, "\n        // TEST \"{label}\"").unwrap();
        for (idx, cmd) in case.body.iter().enumerate() {
            match cmd {
                CltCommand::Set(pvs) => pvs.iter().for_each(|pv| set(&mut t, pv)),
                CltCommand::Eval => writeln!(t, "        #10;").unwrap(),
                CltCommand::Seq(steps) => {
                    for step in steps {
                        step.assignments.iter().for_each(|pv| set(&mut t, pv));
                        writeln!(t, "        #10;").unwrap();
                    }
                }
                CltCommand::Assert(pv) => {
                    let key = pv.pin_ref.to_canonical_key();
                    let sig = net.aliases.get(&key).and_then(|n| net.net_signal.get(n));
                    let Some(sig) = sig else {
                        skipped.push(format!("{label}: {key} (not a net of the exported netlist)"));
                        continue;
                    };
                    checks += 1;
                    let v = bit(&pv.value);
                    writeln!(
                        t,
                        "        checks = checks + 1;\n        if (dut.{sig} !== 1'b{v}) begin failures = failures + 1; \
                         $display(\"LC_FAIL {label} | cmd {idx} | {key} expected {v} got %b\", dut.{sig}); end"
                    )
                    .unwrap();
                }
            }
        }
    }
    writeln!(t, "\n        $display(\"LC_RESULT checks=%0d failures=%0d\", checks, failures);").unwrap();
    writeln!(t, "        $finish;\n    end\nendmodule").unwrap();

    Testbench { text: t, checks, skipped }
}
