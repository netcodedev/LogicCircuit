//! Chip flow driver: export a circuit as a sky130 netlist, place and route it
//! with OpenROAD (pinned ORFS Docker image, `chip/flow/pnr.tcl`) and collect
//! the results.
//!
//! Every run lives in `chip/<NAME>/<run>/`, where the run name encodes the
//! options (e.g. `basic-met3`, `nand-met5-u70-latches-opt`), so several
//! variants of one circuit can be compared side by side.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::cldl::{CircuitBuilder, FlatLevel};
use crate::export::verilog::{CellLib, VerilogOptions, export_verilog};

pub const LAYERS: [&str; 5] = ["met1", "met2", "met3", "met4", "met5"];
const ORFS: &str = "/OpenROAD-flow-scripts";
/// Default core utilisation in percent; other values appear in the run name.
pub const DEFAULT_UTIL: u32 = 40;

#[derive(Debug, Clone)]
pub struct BuildOptions {
    pub level: FlatLevel,
    pub keep_latches: bool,
    /// Highest metal layer for routing, power grid and pins (met2..met5).
    pub max_layer: String,
    pub clock_period_ns: f64,
    /// Core utilisation in percent (ignored with `die_size_um`).
    pub utilization: u32,
    /// Fixed die size (width, height) in microns.
    pub die_size_um: Option<(f64, f64)>,
    /// Allow OpenROAD to buffer and resize (changes the netlist).
    pub optimize: bool,
}

impl BuildOptions {
    pub fn run_name(&self) -> String {
        let mut name = format!(
            "{}-{}",
            match self.level {
                FlatLevel::Basic => "basic",
                FlatLevel::Nand => "nand",
            },
            self.max_layer
        );
        match self.die_size_um {
            Some((w, h)) if w == h => name.push_str(&format!("-die{w}")),
            Some((w, h)) => name.push_str(&format!("-die{w}x{h}")),
            None if self.utilization != DEFAULT_UTIL => name.push_str(&format!("-u{}", self.utilization)),
            None => {}
        }
        if self.keep_latches {
            name.push_str("-latches");
        }
        if self.optimize {
            name.push_str("-opt");
        }
        name
    }
}

/// Everything known about a finished run.
#[derive(Debug, Clone, Default)]
pub struct RunMetrics {
    pub run: String,
    pub die_um: (f64, f64),
    pub logic_area_um2: f64,
    pub transistors: usize,
    pub max_layer: String,
    pub clock_period_ns: f64,
    pub optimize: bool,
    /// Instances per cell master (including fill/tap/clock buffers).
    pub cells: BTreeMap<String, usize>,
    /// Cells in the exported netlist.
    pub exported_cells: Option<usize>,
    pub wire_um: BTreeMap<String, f64>,
    pub vias: Option<usize>,
    pub worst_slack_ns: Option<f64>,
    pub drc_violations: usize,
}

impl RunMetrics {
    /// Cells that implement logic, i.e. everything but filler and tap cells.
    pub fn logic_cells(&self) -> usize {
        self.cells
            .iter()
            .filter(|(m, _)| !m.contains("__fill_") && !m.contains("__tapvpwrvgnd_"))
            .map(|(_, n)| n)
            .sum()
    }

    pub fn total_wire_um(&self) -> f64 {
        self.wire_um.values().sum()
    }

    /// Highest clock frequency the routed design meets (from the worst slack).
    pub fn fmax_mhz(&self) -> Option<f64> {
        let wns = self.worst_slack_ns?;
        let period = self.clock_period_ns - wns;
        (period > 0.0).then(|| 1000.0 / period)
    }
}

pub fn chip_dir(root: &Path, name: &str) -> PathBuf {
    root.join("chip").join(name)
}

/// Exports `name`, runs OpenROAD and returns the metrics. `root` is the
/// repository root (it is mounted into the container). OpenROAD's output goes
/// to `<run>/pnr.log`.
pub fn build(builder: &CircuitBuilder, name: &str, opts: &BuildOptions, root: &Path) -> Result<RunMetrics, String> {
    if !LAYERS[1..].contains(&opts.max_layer.as_str()) {
        return Err(format!("max layer must be met2..met5, got '{}'", opts.max_layer));
    }
    let flow = root.join("chip/flow/pnr.tcl");
    if !flow.is_file() {
        return Err(format!("{} not found; run from the repository root", flow.display()));
    }
    let image = std::fs::read_to_string(root.join("chip/flow/IMAGE"))
        .map_err(|e| format!("chip/flow/IMAGE: {e}"))?
        .trim()
        .to_string();

    let run = opts.run_name();
    let rel_dir = PathBuf::from("chip").join(name).join(&run);
    let run_dir = root.join(&rel_dir);
    if run_dir.exists() {
        std::fs::remove_dir_all(&run_dir).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(&run_dir).map_err(|e| e.to_string())?;

    // Netlist.
    let vopts = VerilogOptions { level: opts.level, cells: CellLib::Sky130, keep_latches: opts.keep_latches };
    let netlist = export_verilog(builder, name, &vopts)?;
    let netlist_file = format!("{}.v", netlist.module);
    std::fs::write(run_dir.join(&netlist_file), &netlist.text).map_err(|e| e.to_string())?;
    let names: String = netlist.names.iter().map(|(v, k)| format!("{v}\t{k}\n")).collect();
    std::fs::write(run_dir.join("names.tsv"), names).map_err(|e| e.to_string())?;
    let exported: usize = netlist.cell_counts.values().sum();
    std::fs::write(run_dir.join("export.txt"), format!("exported_cells {exported}\n")).map_err(|e| e.to_string())?;

    let clock = netlist
        .ports
        .iter()
        .find(|(n, w, out)| !out && w.is_none() && n.eq_ignore_ascii_case("clk"))
        .map(|(n, _, _)| n.clone())
        .unwrap_or_default();

    // OpenROAD in the container, as the current user so outputs stay ours.
    let id = |flag: &str| -> String {
        Command::new("id")
            .arg(flag)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|| "0".into())
    };
    let rel = |p: &Path| p.to_string_lossy().replace('\\', "/");
    let mut env = vec![
        format!("PLATFORM_DIR={ORFS}/flow/platforms/sky130hd"),
        format!("NETLIST={}", rel(&rel_dir.join(&netlist_file))),
        format!("TOP={}", netlist.module),
        format!("OUT_DIR={}", rel(&rel_dir)),
        format!("MAX_LAYER={}", opts.max_layer),
        format!("CLOCK_PORT={clock}"),
        format!("CLOCK_PERIOD={}", opts.clock_period_ns),
        format!("CORE_UTIL={}", opts.utilization),
        format!("OPTIMIZE={}", opts.optimize as u8),
        // Latch-based designs have no flip-flop cells for a clock tree.
        format!("NO_CTS={}", opts.keep_latches as u8),
        "HOME=/tmp".into(),
    ];
    if let Some((w, h)) = opts.die_size_um {
        env.push(format!("DIE_SIZE={w}x{h}"));
    }
    let mut cmd = Command::new("docker");
    cmd.args(["run", "--rm", "--user", &format!("{}:{}", id("-u"), id("-g"))])
        .arg("-v")
        .arg(format!("{}:/work", root.display()))
        .args(["-w", "/work"]);
    for e in &env {
        cmd.args(["-e", e]);
    }
    let log = std::fs::File::create(run_dir.join("pnr.log")).map_err(|e| e.to_string())?;
    let status = cmd
        .arg(&image)
        .arg(format!("{ORFS}/tools/install/OpenROAD/bin/openroad"))
        .args(["-no_init", "-exit", "chip/flow/pnr.tcl"])
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log)
        .stdin(Stdio::null())
        .status()
        .map_err(|e| format!("cannot run docker: {e}"))?;
    if !status.success() {
        let log = std::fs::read_to_string(run_dir.join("pnr.log")).unwrap_or_default();
        let errors: Vec<&str> = log.lines().filter(|l| l.contains("ERROR")).take(5).collect();
        return Err(format!(
            "OpenROAD failed ({}), see {}\n{}",
            status,
            run_dir.join("pnr.log").display(),
            errors.join("\n")
        ));
    }
    read_metrics(&run_dir)
}

/// Reads `metrics.txt`, `export.txt`, `drc.rpt` and `pnr.log` of a run.
pub fn read_metrics(run_dir: &Path) -> Result<RunMetrics, String> {
    let read = |f: &str| std::fs::read_to_string(run_dir.join(f));
    let metrics = read("metrics.txt").map_err(|e| format!("{}: {e}", run_dir.join("metrics.txt").display()))?;
    let mut m = RunMetrics {
        run: run_dir.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string(),
        ..Default::default()
    };
    for line in metrics.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        let num = |i: usize| parts.get(i).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
        match parts.first().copied() {
            Some("die_width_um") => m.die_um.0 = num(1),
            Some("die_height_um") => m.die_um.1 = num(1),
            Some("logic_area_um2") => m.logic_area_um2 = num(1),
            Some("transistors") => m.transistors = num(1) as usize,
            Some("max_layer") => m.max_layer = parts.get(1).unwrap_or(&"").to_string(),
            Some("clock_period_ns") => m.clock_period_ns = num(1),
            Some("optimize") => m.optimize = parts.get(1) == Some(&"1"),
            Some("cell") if parts.len() == 3 => {
                m.cells.insert(parts[1].to_string(), num(2) as usize);
            }
            _ => {}
        }
    }
    if let Ok(export) = read("export.txt") {
        m.exported_cells = export
            .split_whitespace()
            .nth(1)
            .and_then(|v| v.parse().ok());
    }
    m.drc_violations = read("drc.rpt")
        .map(|r| r.lines().filter(|l| l.trim_start().starts_with("violation type")).count())
        .unwrap_or(0);
    // The detailed router prints its summary last; keep the last values.
    if let Ok(log) = read("pnr.log") {
        for line in log.lines() {
            if let Some(rest) = line.strip_prefix("Total wire length on LAYER ") {
                // "met1 = 16476 um."
                let mut it = rest.split_whitespace();
                if let (Some(layer), Some(_), Some(v)) = (it.next(), it.next(), it.next()) {
                    if let Ok(v) = v.parse::<f64>() {
                        m.wire_um.insert(layer.to_string(), v);
                    }
                }
            } else if let Some(rest) = line.strip_prefix("Total number of vias = ") {
                m.vias = rest.trim_end_matches('.').trim().parse().ok();
            } else if let Some(rest) = line.strip_prefix("worst slack max ") {
                m.worst_slack_ns = rest.trim().parse().ok();
            }
        }
    }
    m.wire_um.retain(|layer, _| layer != "li1");
    Ok(m)
}

/// All runs of a circuit, sorted by name.
pub fn runs(root: &Path, name: &str) -> Vec<RunMetrics> {
    let Ok(entries) = std::fs::read_dir(chip_dir(root, name)) else {
        return Vec::new();
    };
    let mut out: Vec<RunMetrics> = entries
        .flatten()
        .filter(|e| e.path().join("metrics.txt").is_file())
        .filter_map(|e| read_metrics(&e.path()).ok())
        .collect();
    out.sort_by(|a, b| a.run.cmp(&b.run));
    out
}

/// Metrics as JSON (no serde in the core crate).
pub fn to_json(name: &str, runs: &[RunMetrics]) -> String {
    let opt = |v: Option<f64>| v.map_or("null".to_string(), |v| format!("{v}"));
    let map = |m: &BTreeMap<String, f64>| {
        m.iter().map(|(k, v)| format!("\"{k}\":{v}")).collect::<Vec<_>>().join(",")
    };
    let mut s = format!("{{\"circuit\":\"{name}\",\"runs\":[");
    for (i, r) in runs.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let cells = r.cells.iter().map(|(k, v)| format!("\"{k}\":{v}")).collect::<Vec<_>>().join(",");
        let _ = write!(
            s,
            "{{\"run\":\"{}\",\"max_layer\":\"{}\",\"die_um\":[{},{}],\"logic_area_um2\":{},\
             \"transistors\":{},\"logic_cells\":{},\"exported_cells\":{},\"wire_um\":{{{}}},\
             \"wire_total_um\":{},\"vias\":{},\"drc_violations\":{},\"clock_period_ns\":{},\
             \"worst_slack_ns\":{},\"fmax_mhz\":{},\"optimize\":{},\"cells\":{{{}}}}}",
            r.run,
            r.max_layer,
            r.die_um.0,
            r.die_um.1,
            r.logic_area_um2,
            r.transistors,
            r.logic_cells(),
            r.exported_cells.map_or("null".into(), |v| v.to_string()),
            map(&r.wire_um),
            r.total_wire_um(),
            r.vias.map_or("null".into(), |v| v.to_string()),
            r.drc_violations,
            r.clock_period_ns,
            opt(r.worst_slack_ns),
            opt(r.fmax_mhz()),
            r.optimize,
            cells
        );
    }
    s.push_str("]}");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_names_encode_options() {
        let mut o = BuildOptions {
            level: FlatLevel::Basic,
            keep_latches: false,
            max_layer: "met3".into(),
            clock_period_ns: 10.0,
            utilization: 40,
            die_size_um: None,
            optimize: false,
        };
        assert_eq!(o.run_name(), "basic-met3");
        o.level = FlatLevel::Nand;
        o.keep_latches = true;
        o.optimize = true;
        o.utilization = 70;
        assert_eq!(o.run_name(), "nand-met3-u70-latches-opt");
        o.die_size_um = Some((150.0, 150.0));
        assert_eq!(o.run_name(), "nand-met3-die150-latches-opt");
        o.die_size_um = Some((160.0, 225.0));
        assert_eq!(o.run_name(), "nand-met3-die160x225-latches-opt");
    }

    /// Extracts a real run if one has been built (needs `chip build
    /// COMPUTER_8B` and the platform cache); skipped otherwise.
    #[test]
    fn extracts_built_computer_layout() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let run = chip_dir(root, "COMPUTER_8B").join("basic-met5");
        if !run.join("metrics.txt").is_file() || !root.join("chip/.platform/sky130hd/sky130_fd_sc_hd.gds").is_file() {
            eprintln!("no built COMPUTER_8B run, skipping");
            return;
        }
        let (layout, checks) = extract(root, &run).unwrap();
        assert!(checks.missing_cells.is_empty(), "{:?}", checks.missing_cells);
        assert!(checks.open_nets.is_empty(), "{:?}", checks.open_nets);
        assert!(checks.unknown_vias.is_empty() && checks.masters_without_gds.is_empty());
        assert!(layout.nets.iter().filter(|n| !n.special).all(|n| n.key.is_some()));
        let (header, ints) = crate::layout::read_binary(&layout.write_binary()).unwrap();
        assert_eq!(header["design"], "COMPUTER_8B");
        let total: u64 = header["sections"].as_array().unwrap().iter().map(|s| s["length"].as_u64().unwrap()).sum();
        assert_eq!(total as usize, ints.len());
    }

    #[test]
    fn reads_metrics_and_log() {
        let dir = std::env::temp_dir().join(format!("logiccircuit-chip-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("metrics.txt"),
            "die_width_um 100\ndie_height_um 80\nlogic_area_um2 1234.5\ntransistors 42\nmax_layer met4\n\
             clock_period_ns 10\noptimize 0\ncell sky130_fd_sc_hd__nand2_1 7\ncell sky130_fd_sc_hd__fill_1 3\n",
        )
        .unwrap();
        std::fs::write(dir.join("export.txt"), "exported_cells 7\n").unwrap();
        std::fs::write(
            dir.join("pnr.log"),
            "Total wire length on LAYER li1 = 0 um.\nTotal wire length on LAYER met1 = 10 um.\n\
             Total wire length on LAYER met2 = 5 um.\nTotal number of vias = 9.\nworst slack max -0.5\n",
        )
        .unwrap();
        let m = read_metrics(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(m.die_um, (100.0, 80.0));
        assert_eq!(m.transistors, 42);
        assert_eq!(m.logic_cells(), 7);
        assert_eq!(m.exported_cells, Some(7));
        assert_eq!(m.total_wire_um(), 15.0);
        assert_eq!(m.vias, Some(9));
        assert!((m.fmax_mhz().unwrap() - 1000.0 / 10.5).abs() < 1e-9);
        assert!(!m.wire_um.contains_key("li1"));
    }
}

/// Files of the sky130hd platform needed outside the container.
const PLATFORM_FILES: [&str; 3] = [
    "lef/sky130_fd_sc_hd.tlef",
    "lef/sky130_fd_sc_hd_merged.lef",
    "gds/sky130_fd_sc_hd.gds",
];

/// Local copy of the platform files (`chip/.platform/sky130hd/`), copied out
/// of the pinned image on first use.
pub fn platform_dir(root: &Path) -> Result<PathBuf, String> {
    let dir = root.join("chip/.platform/sky130hd");
    let file_name = |f: &str| f.rsplit('/').next().unwrap().to_string();
    if PLATFORM_FILES.iter().all(|f| dir.join(file_name(f)).is_file()) {
        return Ok(dir);
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let image = std::fs::read_to_string(root.join("chip/flow/IMAGE"))
        .map_err(|e| format!("chip/flow/IMAGE: {e}"))?
        .trim()
        .to_string();
    let id = |flag: &str| {
        Command::new("id").arg(flag).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
    };
    let sources: Vec<String> = PLATFORM_FILES.iter().map(|f| format!("{ORFS}/flow/platforms/sky130hd/{f}")).collect();
    let status = Command::new("docker")
        .args(["run", "--rm", "--user", &format!("{}:{}", id("-u"), id("-g"))])
        .arg("-v")
        .arg(format!("{}:/out", dir.canonicalize().map_err(|e| e.to_string())?.display()))
        .arg(&image)
        .arg("cp")
        .args(&sources)
        .arg("/out/")
        .status()
        .map_err(|e| format!("cannot run docker: {e}"))?;
    if !status.success() {
        return Err(format!("copying platform files from {image} failed"));
    }
    Ok(dir)
}

/// Picks a run of `name`: the given one, else `basic-met5`, else the first.
pub fn resolve_run(root: &Path, name: &str, run: Option<&str>) -> Result<PathBuf, String> {
    // Runs of our own router have a layout.bin but no OpenROAD metrics.
    if let Some(r) = run {
        let dir = chip_dir(root, name).join(r);
        if dir.join("layout.bin").is_file() && !dir.join("metrics.txt").is_file() {
            return Ok(dir);
        }
    }
    let all = runs(root, name);
    let pick = match run {
        Some(r) => all.iter().find(|m| m.run == r),
        None => all.iter().find(|m| m.run == "basic-met5").or(all.first()),
    };
    pick.map(|m| chip_dir(root, name).join(&m.run))
        .ok_or_else(|| format!("no chip run {}for '{name}' (chip build {name} first)", run.map(|r| format!("'{r}' ")).unwrap_or_default()))
}

/// Extracts the layout of a run (see `crate::layout`).
pub fn extract(root: &Path, run_dir: &Path) -> Result<(crate::layout::Layout, crate::layout::Checks), String> {
    let platform = platform_dir(root)?;
    let def = std::fs::read_dir(run_dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "def"))
        .ok_or_else(|| format!("no DEF in {}", run_dir.display()))?;
    let names = run_dir.join("names.tsv");
    let (mut layout, checks) = crate::layout::extract(&crate::layout::Sources {
        def: &def,
        tech_lef: &platform.join("sky130_fd_sc_hd.tlef"),
        cell_lef: &platform.join("sky130_fd_sc_hd_merged.lef"),
        cell_gds: &platform.join("sky130_fd_sc_hd.gds"),
        names: names.is_file().then_some(names.as_path()),
    })?;
    // The run's metrics travel with the layout, for viewers.
    if let Ok(m) = read_metrics(run_dir) {
        let json: serde_json::Value = serde_json::from_str(&to_json(&layout.design, std::slice::from_ref(&m))).map_err(|e| e.to_string())?;
        layout.metrics = json["runs"].get(0).cloned();
    }
    Ok((layout, checks))
}
