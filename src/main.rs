use clap::Parser;
use std::path::{Path, PathBuf};

use logic_circuit::cldl::CircuitBuilder;
use logic_circuit::clt::{CircuitTester, TestSummary}; // Import TestSummary
use logic_circuit::{isa, machine};

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
    if args.directory.is_none() && matches!(args.command, Some(Commands::Run { .. } | Commands::Nands { .. })) {
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
