use clap::Parser;
use std::path::{Path, PathBuf};

use logic_circuit::cldl::CircuitBuilder;
use logic_circuit::clt::{CircuitTester, TestSummary}; // Import TestSummary
use logic_circuit::cldl::FlatLevel;
use logic_circuit::export::verilog::{CellLib, VerilogOptions, export_verilog};
use logic_circuit::export::verify_with_iverilog;
use logic_circuit::chip::{self, BuildOptions, RunMetrics};
use logic_circuit::{clt, isa, machine};

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum Level {
    /// Basic blocks (NOT, AND, OR, XOR, ...) as standard cells
    Basic,
    /// Everything as 2-input NAND cells
    Nand,
}

impl From<Level> for FlatLevel {
    fn from(l: Level) -> Self {
        match l {
            Level::Basic => FlatLevel::Basic,
            Level::Nand => FlatLevel::Nand,
        }
    }
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum Cells {
    /// sky130_fd_sc_hd standard cells (for OpenROAD)
    Sky130,
    /// Self-contained LC_* cell models (for Icarus Verilog / Verilator)
    Generic,
}

// --- Command Line Argument Parsing ---
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Optional path to a directory containing additional .cldl circuit
    /// definitions AND/OR .clt test files. Searched recursively.
    /// If provided, definitions augment/override embedded ones, and
    /// the 'test' command ONLY runs tests from this directory.
    /// If not provided, 'test' command runs tests from src/circuits/.
    #[arg(short = 'd', long, value_name = "DIR")]
    directory: Option<PathBuf>,
}

#[derive(clap::Subcommand, Debug)]
enum Commands {
    /// Run circuit tests (.clt files).
    /// Runs tests either from the directory specified by -d
    /// or from src/circuits/ if -d is not provided.
    Test,
    Export {
        #[arg(value_name = "COMPONENT_NAME")]
        component_name: String,
    },
    /// Assemble a program (.asm) and run it on a CPU circuit.
    /// Loads ./circuits automatically if -d is not given.
    Run {
        #[arg(value_name = "PROGRAM")]
        program: PathBuf,
        /// Circuit to run on (needs PC, Instruction and CLK ports).
        #[arg(long, default_value = "COMPUTER_8B")]
        top: String,
        /// Stop after this many instructions if the program does not HALT.
        #[arg(long, default_value_t = 10_000)]
        max_steps: usize,
        /// Print every executed instruction.
        #[arg(long)]
        trace: bool,
    },
    /// Count the NAND gates a circuit is built from, broken down by part.
    /// Without a name, lists every known circuit.
    /// Loads ./circuits automatically if -d is not given.
    Nands {
        #[arg(value_name = "COMPONENT_NAME")]
        component_name: Option<String>,
        /// How many hierarchy levels of parts to break down.
        #[arg(long, default_value_t = 1)]
        depth: usize,
    },
    /// Export a circuit as structural gate-level Verilog.
    /// Loads ./circuits automatically if -d is not given.
    ExportVerilog {
        #[arg(value_name = "COMPONENT_NAME")]
        component_name: String,
        #[arg(long, value_enum, default_value_t = Level::Basic)]
        level: Level,
        #[arg(long, value_enum, default_value_t = Cells::Sky130)]
        cells: Cells,
        /// Keep flip-flops as our latch gates instead of flip-flop cells.
        #[arg(long)]
        keep_latches: bool,
        /// Output file (default: <COMPONENT_NAME>.v).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Also write a TSV mapping Verilog names back to wire keys / parts.
        #[arg(long, value_name = "FILE")]
        names: Option<PathBuf>,
    },
    /// Chip layout: place and route with OpenROAD (Docker) and report.
    /// Loads ./circuits automatically if -d is not given.
    Chip {
        #[command(subcommand)]
        command: ChipCommand,
    },
    /// Check exported netlists against the .clt tests with Icarus Verilog.
    /// Without a name, checks every circuit that has tests.
    VerifyVerilog {
        #[arg(value_name = "COMPONENT_NAME")]
        component_name: Option<String>,
        #[arg(long, value_enum, default_value_t = Level::Basic)]
        level: Level,
        #[arg(long)]
        keep_latches: bool,
    },
}
#[derive(clap::Subcommand, Debug)]
enum ChipCommand {
    /// Export as sky130 netlist, then floorplan, place, clock tree and route.
    /// Results go to chip/<NAME>/<run>/ (run name from the options).
    Build {
        #[arg(value_name = "COMPONENT_NAME")]
        component_name: String,
        #[arg(long, value_enum, default_value_t = Level::Basic)]
        level: Level,
        #[arg(long)]
        keep_latches: bool,
        /// Highest metal layer for routing, power grid and pins.
        #[arg(long, default_value = "met5", value_parser = ["met2", "met3", "met4", "met5"])]
        max_layer: String,
        /// Clock period in ns for timing analysis.
        #[arg(long, default_value_t = 10.0)]
        clock_period: f64,
        /// Core utilisation in percent.
        #[arg(long, default_value_t = chip::DEFAULT_UTIL)]
        util: u32,
        /// Fixed square die edge in um instead of --util.
        #[arg(long)]
        die_size: Option<f64>,
        /// Let OpenROAD buffer and resize cells (changes the netlist).
        #[arg(long)]
        optimize: bool,
    },
    /// Compare all runs of a circuit (or show one run in detail).
    Report {
        #[arg(value_name = "COMPONENT_NAME")]
        component_name: String,
        /// Show only this run, with its cell list.
        #[arg(long)]
        run: Option<String>,
        /// Print JSON instead of tables.
        #[arg(long)]
        json: bool,
    },
}
// --- End Command Line Args ---

fn main() {
    let args = Args::parse();

    // Step 1: Create builder (always loads all embedded circuits from src/circuits)
    println!("Initializing CircuitBuilder (loading embedded circuits)...");
    let mut builder = CircuitBuilder::new();

    // Step 2: Load additional circuits ONLY if -d is specified
    if let Some(dir_path) = args.directory.as_deref() {
        if !dir_path.exists() || !dir_path.is_dir() {
            // Make this a fatal error as the user explicitly provided a path
            eprintln!(
                "Error: Specified directory does not exist or is not a directory: {:?}",
                dir_path
            );
            std::process::exit(1);
        } else {
            println!(
                "Loading additional/override circuits from specified directory: {:?}",
                dir_path
            );
            if let Err(e) = builder.load_circuits_from_path(dir_path) {
                eprintln!("Error loading circuits from {:?}: {}", dir_path, e);
                std::process::exit(1); // Make it fatal
            }
        }
    }

    // The CPU circuits live in ./circuits, which is not embedded.
    if args.directory.is_none() && matches!(
        args.command,
        Some(
            Commands::Run { .. }
                | Commands::Nands { .. }
                | Commands::ExportVerilog { .. }
                | Commands::VerifyVerilog { .. }
                | Commands::Chip { .. }
        )
    ) {
        let default_dir = Path::new("./circuits");
        if default_dir.is_dir() {
            if let Err(e) = builder.load_circuits_from_path(default_dir) {
                eprintln!("Error loading circuits from {:?}: {}", default_dir, e);
                std::process::exit(1);
            }
        }
    }

    // Step 3: Dispatch Command
    match args.command {
        Some(Commands::Test) => {
            let tester = CircuitTester::new();
            let mut summary = TestSummary::new(); // Initialize summary
            let mut test_run_error = false;

            // Determine which directory to run tests in
            let test_dir_path = match args.directory.as_deref() {
                Some(p) => {
                    println!("Running tests specified in directory: {:?}", p);
                    p // Use the directory specified by -d
                }
                None => {
                    // -d not provided, default to src/circuits
                    let default_path = Path::new("./src/circuits");
                    println!("Running tests in default directory: {:?}", default_path);
                    default_path
                }
            };

            // Ensure the chosen test directory exists
            if !test_dir_path.exists() || !test_dir_path.is_dir() {
                eprintln!(
                    "Error: Test directory does not exist or is not a directory: {:?}",
                    test_dir_path
                );
                std::process::exit(1);
            }

            // Run tests using the determined path and the populated builder
            match tester.run_tests_in_dir(test_dir_path, &builder) {
                Ok(s) => summary = s, // Store the summary
                Err(e) => {
                    eprintln!("Test runner failed: {}", e);
                    test_run_error = true;
                    // summary remains default (0 passed, 0 failed)
                }
            }

            // Report final summary
            println!(
                "\nTest run finished: {} passed, {} failed.",
                summary.passed, summary.failed
            );
            if summary.failed > 0 || test_run_error {
                std::process::exit(1); // Exit with error code if tests failed or runner errored
            } else {
                println!("All specified tests passed! ✅");
                std::process::exit(0); // Exit successfully
            }
        }
        Some(Commands::Export { component_name }) => {
            println!("Exporting '{}' to DOT format...", component_name);
            
            // Generate the .dot string using the provided component name
            // Note: builder.export_to_dot will panic if the name is not found,
            // which is acceptable for this command-line tool.
            let dot_string = builder.export_to_dot(&component_name);
            
            // Create a dynamic filename, e.g., "ALU_4B.dot"
            let output_filename = format!("{}.dot", component_name);
            
            match std::fs::write(&output_filename, dot_string) {
                Ok(_) => println!("Successfully saved graph to '{}'", output_filename),
                Err(e) => eprintln!("Error saving .dot file: {}", e),
            }
        }
        Some(Commands::Run { program, top, max_steps, trace }) => {
            run_program(&builder, &program, &top, max_steps, trace);
        }
        Some(Commands::ExportVerilog { component_name, level, cells, keep_latches, output, names }) => {
            let opts = VerilogOptions {
                level: level.into(),
                cells: match cells {
                    Cells::Sky130 => CellLib::Sky130,
                    Cells::Generic => CellLib::Generic,
                },
                keep_latches,
            };
            let netlist = export_verilog(&builder, &component_name, &opts).unwrap_or_else(|e| {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            });
            let path = output.unwrap_or_else(|| PathBuf::from(format!("{}.v", component_name)));
            if let Err(e) = std::fs::write(&path, &netlist.text) {
                eprintln!("Error writing {:?}: {}", path, e);
                std::process::exit(1);
            }
            let total: usize = netlist.cell_counts.values().sum();
            println!("\nWrote {} ({} cells)", path.display(), group_digits(total));
            for (cell, n) in &netlist.cell_counts {
                println!("  {:<28} {:>8}", cell, group_digits(*n));
            }
            if let Some(names_path) = names {
                let tsv: String = netlist.names.iter().map(|(v, k)| format!("{}\t{}\n", v, k)).collect();
                if let Err(e) = std::fs::write(&names_path, tsv) {
                    eprintln!("Error writing {:?}: {}", names_path, e);
                    std::process::exit(1);
                }
                println!("Wrote name map {}", names_path.display());
            }
        }
        Some(Commands::Chip { command }) => match command {
            ChipCommand::Build { component_name, level, keep_latches, max_layer, clock_period, util, die_size, optimize } => {
                let opts = BuildOptions {
                    level: level.into(),
                    keep_latches,
                    max_layer,
                    clock_period_ns: clock_period,
                    utilization: util,
                    die_size_um: die_size,
                    optimize,
                };
                println!("\nBuilding chip/{}/{} with OpenROAD …", component_name, opts.run_name());
                let started = std::time::Instant::now();
                match chip::build(&builder, &component_name, &opts, Path::new(".")) {
                    Ok(m) => {
                        println!("Done in {:.0} s.", started.elapsed().as_secs_f64());
                        print_chip_runs(&component_name, &[m]);
                    }
                    Err(e) => {
                        eprintln!("Error: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            ChipCommand::Report { component_name, run, json } => {
                let mut runs = chip::runs(Path::new("."), &component_name);
                if let Some(r) = &run {
                    runs.retain(|m| &m.run == r);
                }
                if runs.is_empty() {
                    eprintln!("No chip runs found for '{}' (chip build {} first).", component_name, component_name);
                    std::process::exit(1);
                }
                if json {
                    println!("{}", chip::to_json(&component_name, &runs));
                } else {
                    print_chip_runs(&component_name, &runs);
                    if run.is_some() {
                        print_chip_cells(&runs[0]);
                    }
                }
            }
        },
        Some(Commands::VerifyVerilog { component_name, level, keep_latches }) => {
            let opts = VerilogOptions { level: level.into(), cells: CellLib::Generic, keep_latches };
            verify_verilog(&builder, component_name.as_deref(), opts);
        }
        Some(Commands::Nands { component_name, depth }) => match component_name {
            Some(name) => print_nand_breakdown(&builder, &name, depth),
            None => print_nand_table(&builder),
        },
        None => {
            println!("\nNo command specified. Use 'test' or 'manual-test'.");
            println!("Use '-d <DIR>' to specify an additional directory for circuits and tests.");
            println!("Use '--help' for more options.");
        }
    }
}

fn run_program(builder: &CircuitBuilder, path: &Path, top: &str, max_steps: usize, trace: bool) {
    let source = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("Error reading {:?}: {}", path, e);
        std::process::exit(1);
    });
    let program = isa::assemble(&source).unwrap_or_else(|e| {
        eprintln!("{}:{}", path.display(), e);
        std::process::exit(1);
    });
    if builder.describe(top).is_none() {
        eprintln!("Circuit '{}' not found (use -d to load its directory).", top);
        std::process::exit(1);
    }

    let circuit = builder.build(top);
    if !machine::is_runnable(&circuit) {
        eprintln!("Circuit '{}' has no PC/Instruction/CLK ports.", top);
        std::process::exit(1);
    }
    let rom = &program.words;
    println!("\nRunning {} ({} words) on {}", path.display(), rom.len(), top);

    machine::fetch(&circuit, rom);
    let mut steps = 0;
    let started = std::time::Instant::now();
    // Time spent printing the trace is not simulation time.
    let mut trace_time = std::time::Duration::ZERO;
    while steps < max_steps && !machine::halted(&circuit) {
        let pc = machine::pc(&circuit);
        let word = machine::word_at(rom, pc);
        machine::step(&circuit, rom);
        steps += 1;
        if trace {
            let t = std::time::Instant::now();
            println!(
                "  [{:4}] PC={:02X}  {:04X}  {:<18} Out={:02X}",
                steps,
                pc,
                word,
                isa::disassemble(word),
                machine::read_bus(&circuit, "Out", 8)
            );
            trace_time += t.elapsed();
        }
    }
    let elapsed = started.elapsed().saturating_sub(trace_time);

    let out = machine::read_bus(&circuit, "Out", 8);
    if machine::halted(&circuit) {
        println!("Halted after {} instructions at PC={:02X}.", steps, machine::pc(&circuit));
    } else {
        println!("Stopped after {} instructions (no HALT, see --max-steps).", steps);
    }
    println!("Out = 0x{:02X} ({})", out, out);
    let secs = elapsed.as_secs_f64();
    let rate = if secs > 0.0 { steps as f64 / secs } else { 0.0 };
    let rate = if rate >= 1000.0 { format!("{:.1} kHz", rate / 1000.0) } else { format!("{:.0} Hz", rate) };
    println!("Time = {:.1} ms for {} instructions (≈ {})", secs * 1000.0, steps, rate);

    // Show the 7-segment outputs if the circuit has them (Seg1 = high digit).
    let digits: Vec<u32> = ["Seg1", "Seg0"]
        .iter()
        .filter(|name| circuit.get_wire_map().contains_key(&format!("{}.0", name)))
        .map(|name| machine::read_bus(&circuit, name, 7))
        .collect();
    if !digits.is_empty() {
        let seg = |d: u32, i: u32, on: char| if (d >> i) & 1 == 1 { on } else { ' ' };
        let rows = [
            digits.iter().map(|&d| format!(" {} ", seg(d, 0, '_'))).collect::<Vec<_>>(),
            digits.iter().map(|&d| format!("{}{}{}", seg(d, 5, '|'), seg(d, 6, '_'), seg(d, 1, '|'))).collect(),
            digits.iter().map(|&d| format!("{}{}{}", seg(d, 4, '|'), seg(d, 3, '_'), seg(d, 2, '|'))).collect(),
        ];
        println!();
        for row in rows {
            println!("    {}", row.join(" "));
        }
    }
}

/// Formats a count with thousands separators, e.g. 12,345.
fn group_digits(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn print_nand_table(builder: &CircuitBuilder) {
    let mut rows: Vec<(String, usize)> = builder
        .circuit_names()
        .into_iter()
        .filter_map(|name| builder.nand_count(&name).map(|n| (name, n)))
        .collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    println!("\n  {:<20} {:>10}", "Circuit", "NANDs");
    for (name, n) in rows {
        println!("  {:<20} {:>10}", name, group_digits(n));
    }
}

fn print_nand_breakdown(builder: &CircuitBuilder, name: &str, depth: usize) {
    let Some(total) = builder.nand_count(name) else {
        eprintln!("Circuit '{}' not found (use -d to load its directory).", name);
        std::process::exit(1);
    };
    println!("\n{}: {} NANDs\n", name, group_digits(total));
    println!("  {:<32} {:<18} {:>10} {:>7}", "Part", "Type", "NANDs", "Share");
    print_parts(builder, name, total, 1, depth);
}

fn print_parts(builder: &CircuitBuilder, name: &str, total: usize, level: usize, depth: usize) {
    let Some(view) = builder.describe(name) else { return };
    let mut parts = view.parts;
    parts.sort_by(|a, b| b.nand_count.cmp(&a.nand_count));
    for part in parts {
        let label = format!("{}{}", "  ".repeat(level - 1), part.instance_name);
        let share = 100.0 * part.nand_count as f64 / total.max(1) as f64;
        println!(
            "  {:<32} {:<18} {:>10} {:>6.1}%",
            label,
            part.part_type,
            group_digits(part.nand_count),
            share
        );
        if level < depth && part.expandable {
            print_parts(builder, &part.part_type, total, level + 1, depth);
        }
    }
}

/// Collects all .clt test suites below the given directories.
fn collect_suites(dirs: &[&Path]) -> Vec<clt::CltTestSuite> {
    fn walk(dir: &Path, out: &mut Vec<clt::CltTestSuite>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "clt") {
                match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|s| clt::parse_clt(&s)) {
                    Ok(file) => out.extend(file.suites),
                    Err(e) => eprintln!("Skipping {:?}: {}", path, e),
                }
            }
        }
    }
    let mut out = Vec::new();
    for dir in dirs {
        walk(dir, &mut out);
    }
    out
}

fn verify_verilog(builder: &CircuitBuilder, only: Option<&str>, opts: VerilogOptions) {
    let suites: Vec<_> = collect_suites(&[Path::new("./src/circuits"), Path::new("./circuits")])
        .into_iter()
        .filter(|s| only.is_none_or(|n| s.circuit_name == n))
        .collect();
    if suites.is_empty() {
        eprintln!("No .clt tests found{}.", only.map(|n| format!(" for '{}'", n)).unwrap_or_default());
        std::process::exit(1);
    }
    let work = std::env::temp_dir().join("logiccircuit-verify");
    println!("\nVerifying exported netlists with Icarus Verilog (work dir {})", work.display());
    println!("  {:<20} {:>7} {:>9} {:>8}", "Circuit", "Checks", "Failures", "Skipped");
    let mut failed = 0;
    for suite in &suites {
        match verify_with_iverilog(builder, suite, opts, &work) {
            Ok(r) => {
                let mark = if r.failures.is_empty() { "✅" } else { "❌" };
                println!(
                    "  {:<20} {:>7} {:>9} {:>8}  {}",
                    suite.circuit_name,
                    r.checks,
                    r.failures.len(),
                    r.skipped.len(),
                    mark
                );
                for f in r.failures.iter().take(5) {
                    println!("      {}", f);
                }
                if !r.failures.is_empty() {
                    failed += 1;
                }
            }
            Err(e) => {
                println!("  {:<20} error: {}", suite.circuit_name, e.lines().next().unwrap_or(""));
                failed += 1;
            }
        }
    }
    if failed > 0 {
        println!("\n{} of {} suites failed.", failed, suites.len());
        std::process::exit(1);
    }
    println!("\nAll {} suites match our simulator.", suites.len());
}

fn print_chip_runs(name: &str, runs: &[RunMetrics]) {
    let fmt_f = |v: f64| group_digits(v.round() as usize);
    println!("\n{}\n", name);
    println!(
        "  {:<24} {:>13} {:>11} {:>8} {:>12} {:>10} {:>8} {:>4} {:>8} {:>9}",
        "Run", "Die (um)", "Logic um2", "Cells", "Transistors", "Wire um", "Vias", "DRC", "Slack", "fmax"
    );
    for r in runs {
        let cells = match r.exported_cells {
            Some(e) if e != r.logic_cells() => format!("{}*", group_digits(r.logic_cells())),
            _ => group_digits(r.logic_cells()),
        };
        println!(
            "  {:<24} {:>13} {:>11} {:>8} {:>12} {:>10} {:>8} {:>4} {:>8} {:>9}",
            r.run,
            format!("{:.0} x {:.0}", r.die_um.0, r.die_um.1),
            fmt_f(r.logic_area_um2),
            cells,
            group_digits(r.transistors),
            fmt_f(r.total_wire_um()),
            r.vias.map_or("-".into(), group_digits),
            r.drc_violations,
            r.worst_slack_ns.map_or("-".into(), |v| format!("{:.2} ns", v)),
            r.fmax_mhz().map_or("-".into(), |v| format!("{:.0} MHz", v)),
        );
    }
    if runs.iter().any(|r| r.exported_cells.is_some_and(|e| e != r.logic_cells())) {
        println!("  * more cells than exported: clock tree buffers (and buffers from --optimize)");
    }

    // Wire length per layer, one column per run.
    let layers: Vec<&str> = chip::LAYERS.to_vec();
    print!("\n  {:<6}", "Wire");
    for r in runs {
        print!(" {:>24}", r.run);
    }
    println!();
    for layer in layers {
        print!("  {:<6}", layer);
        for r in runs {
            let v = r.wire_um.get(layer).copied().unwrap_or(0.0);
            let share = 100.0 * v / r.total_wire_um().max(1.0);
            print!(" {:>24}", format!("{} um ({:.0}%)", fmt_f(v), share));
        }
        println!();
    }
}

fn print_chip_cells(run: &RunMetrics) {
    println!("\n  {:<36} {:>8}", "Cell", "Count");
    let mut cells: Vec<_> = run.cells.iter().collect();
    cells.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    for (cell, n) in cells {
        println!("  {:<36} {:>8}", cell, group_digits(*n));
    }
}
