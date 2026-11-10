//! Netlist export for the chip flow (see ROADMAP.md).

pub mod testbench;
pub mod verilog;

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Wall-clock limit for one simulation; a netlist that oscillates never
/// reaches $finish.
const SIM_TIMEOUT: Duration = Duration::from_secs(120);

use crate::cldl::CircuitBuilder;
use crate::clt::CltTestSuite;
use verilog::{CellLib, VerilogOptions, export_verilog};

/// Result of simulating an exported netlist against one .clt suite.
pub struct VerifyResult {
    pub checks: usize,
    pub failures: Vec<String>,
    pub skipped: Vec<String>,
}

/// Exports `suite.circuit_name` with the generic cells, generates a testbench
/// from the suite, and runs both with Icarus Verilog (`iverilog` + `vvp` must
/// be on PATH). Files are written to `work_dir`.
///
/// With `x_init` every cell starts unknown (`x`) instead of at the state our
/// simulator has after building, as flip-flops do after power-on: a suite
/// then only passes if its tests reset the circuit into a known state
/// first. Needs flip-flop cells (not `keep_latches`): latches built from
/// gates would stay unknown.
pub fn verify_with_iverilog(
    builder: &CircuitBuilder,
    suite: &CltTestSuite,
    opts: VerilogOptions,
    x_init: bool,
    work_dir: &Path,
) -> Result<VerifyResult, String> {
    if x_init && opts.keep_latches {
        return Err("x-init needs flip-flop cells (without keep-latches)".into());
    }
    let opts = VerilogOptions { cells: CellLib::Generic, ..opts };
    let mut netlist = export_verilog(builder, &suite.circuit_name, &opts)?;
    if x_init {
        netlist.text = netlist.text.replace(".INIT(1'b0)", ".INIT(1'bx)").replace(".INIT(1'b1)", ".INIT(1'bx)");
    }
    let tb = testbench::testbench(&netlist, suite);

    std::fs::create_dir_all(work_dir).map_err(|e| e.to_string())?;
    let v = work_dir.join(format!("{}.v", netlist.module));
    let tbv = work_dir.join(format!("{}_tb.v", netlist.module));
    let sim = work_dir.join(format!("{}.vvp", netlist.module));
    std::fs::write(&v, &netlist.text).map_err(|e| e.to_string())?;
    std::fs::write(&tbv, &tb.text).map_err(|e| e.to_string())?;

    let compile = Command::new("iverilog")
        .args(["-g2005", "-o"])
        .arg(&sim)
        .arg(&v)
        .arg(&tbv)
        .output()
        .map_err(|e| format!("cannot run iverilog: {e}"))?;
    if !compile.status.success() {
        return Err(format!("iverilog failed:\n{}", String::from_utf8_lossy(&compile.stderr)));
    }
    let log = work_dir.join(format!("{}.log", netlist.module));
    let mut child = Command::new("vvp")
        .arg("-n")
        .arg(&sim)
        .stdout(std::fs::File::create(&log).map_err(|e| e.to_string())?)
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot run vvp: {e}"))?;
    let started = Instant::now();
    loop {
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            break;
        }
        if started.elapsed() > SIM_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("simulation did not finish within {} s (oscillation?)", SIM_TIMEOUT.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let stdout = std::fs::read_to_string(&log).unwrap_or_default();

    let failures: Vec<String> = stdout
        .lines()
        .filter_map(|l| l.strip_prefix("LC_FAIL "))
        .map(str::to_string)
        .collect();
    if !stdout.lines().any(|l| l.starts_with("LC_RESULT")) {
        return Err(format!("simulation did not finish:\n{stdout}"));
    }
    Ok(VerifyResult { checks: tb.checks, failures, skipped: tb.skipped })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cldl::FlatLevel;
    use crate::clt::parse_clt;
    use std::collections::BTreeMap;

    fn builder() -> CircuitBuilder {
        let mut b = CircuitBuilder::new();
        b.load_circuits_from_path(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/circuits")))
            .unwrap();
        b
    }

    #[test]
    fn exports_every_circuit_deterministically() {
        let b = builder();
        for name in b.circuit_names() {
            for level in [FlatLevel::Basic, FlatLevel::Nand] {
                for keep_latches in [false, true] {
                    let opts = VerilogOptions { level, cells: CellLib::Sky130, keep_latches, mux_cells: !keep_latches };
                    let first = export_verilog(&b, &name, &opts).unwrap_or_else(|e| panic!("{name}: {e}"));
                    let second = export_verilog(&b, &name, &opts).unwrap();
                    assert_eq!(first.text, second.text, "{name} not deterministic");
                    // Every cell instance maps back to a part path (or is a tie/buffer).
                    let instances = first.text.lines().filter(|l| l.trim_start().starts_with("sky130_")).count();
                    assert_eq!(instances, first.cell_counts.values().sum::<usize>(), "{name}");
                }
            }
        }
    }

    #[test]
    fn nand_level_without_flops_uses_only_nand_cells() {
        let b = builder();
        let opts = VerilogOptions { level: FlatLevel::Nand, cells: CellLib::Sky130, keep_latches: true, mux_cells: true };
        let n = export_verilog(&b, "COMPUTER_8B", &opts).unwrap();
        let nands = n.cell_counts.get("sky130_fd_sc_hd__nand2_1").copied().unwrap_or(0);
        assert_eq!(nands, b.nand_count("COMPUTER_8B").unwrap());
        assert!(n.cell_counts.keys().all(|c| c.ends_with("nand2_1") || c.ends_with("conb_1") || c.ends_with("buf_1")));
    }

    /// With `mux_cells` every MUX_2_1 becomes one mux2 cell; without, none.
    #[test]
    fn mux_cells_replace_every_multiplexer() {
        let b = builder();
        let muxes = b
            .flatten_until("COMPUTER_8B", &|t: &str| b.is_basic(t) || t == "MUX_2_1")
            .unwrap()
            .blocks
            .iter()
            .filter(|bl| bl.kind == "MUX_2_1")
            .count();
        assert!(muxes > 100);
        let mux2 = |mux_cells| {
            let opts = VerilogOptions { level: FlatLevel::Basic, cells: CellLib::Sky130, keep_latches: false, mux_cells };
            export_verilog(&b, "COMPUTER_8B", &opts).unwrap().cell_counts.get("sky130_fd_sc_hd__mux2_1").copied().unwrap_or(0)
        };
        assert_eq!(mux2(true), muxes);
        assert_eq!(mux2(false), 0);
    }

    /// The hs export is the hd export with the other library prefix.
    #[test]
    fn hs_cells_mirror_hd_cells() {
        let b = builder();
        let opts = |cells| VerilogOptions { level: FlatLevel::Basic, cells, keep_latches: false, mux_cells: true };
        let hd = export_verilog(&b, "COMPUTER_8B", &opts(CellLib::Sky130)).unwrap();
        let hs = export_verilog(&b, "COMPUTER_8B", &opts(CellLib::Sky130Hs)).unwrap();
        assert!(!hs.text.contains("sky130_fd_sc_hd__"));
        assert_eq!(hs.text, hd.text.replace("sky130_fd_sc_hd", "sky130_fd_sc_hs"));
        let renamed: BTreeMap<String, usize> =
            hd.cell_counts.iter().map(|(c, n)| (c.replace("_hd__", "_hs__"), *n)).collect();
        assert_eq!(hs.cell_counts, renamed);
    }

    /// Runs the .clt suites of a few circuits against the exported generic
    /// netlist. Skipped when Icarus Verilog is not installed.
    #[test]
    fn exported_netlists_match_simulator_in_icarus() {
        if Command::new("iverilog").arg("-V").output().is_err() {
            eprintln!("iverilog not found, skipping");
            return;
        }
        let b = builder();
        let root = env!("CARGO_MANIFEST_DIR");
        let cases = [
            ("src/circuits/adder/FULL_ADDER.clt", FlatLevel::Nand, true),
            ("src/circuits/memory/REGISTER_4B.clt", FlatLevel::Basic, true),
            ("circuits/COMPUTER_8B.clt", FlatLevel::Basic, false),
            ("src/circuits/mux/MUX_4_1_BUS.clt", FlatLevel::Basic, false),
        ];
        for (file, level, keep_latches) in cases {
            let src = std::fs::read_to_string(Path::new(root).join(file)).unwrap();
            for suite in parse_clt(&src).unwrap().suites {
                // As in chip builds: flip-flop and multiplexer cells.
                let opts = VerilogOptions { level, cells: CellLib::Generic, keep_latches, mux_cells: !keep_latches };
                let work = std::env::temp_dir().join(format!("logiccircuit-test-{}", std::process::id()));
                let r = verify_with_iverilog(&b, &suite, opts, false, &work).unwrap();
                assert!(r.checks > 0, "{file}");
                assert!(r.failures.is_empty(), "{file}: {:?}", r.failures);
            }
        }
    }
}
