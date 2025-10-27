use std::path::{PathBuf};
use clap::Parser;

mod circuit;
mod cldl;
mod clt;

use crate::cldl::CircuitBuilder;
use crate::clt::CircuitTester;

// --- Command Line Argument Parsing ---
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(short='d', long, value_name = "DIR")]
    circuits_dir: Option<PathBuf>,
}

#[derive(clap::Subcommand, Debug)]
enum Commands {
    /// Run circuit tests defined in .clt files
    Test {
        #[arg(short, long, value_name = "DIR", default_value = "./src/circuits")]
        dir: PathBuf,
    },
}

fn main() {
    let args = Args::parse();

    println!("Initializing CircuitBuilder...");
    let mut builder = CircuitBuilder::new(); // Loads embedded circuits from src/circuits

    // Load additional circuits if specified via -c or --circuits-dir
    // These can augment or override the embedded ones.
    if let Some(circuits_path) = args.circuits_dir.as_deref() {
        if !circuits_path.exists() {
             eprintln!("Warning: Specified circuits directory does not exist: {:?}", circuits_path);
        } else if let Err(e) = builder.load_circuits_from_path(circuits_path) {
            eprintln!("Error loading custom circuits from {:?}: {}", circuits_path, e);
            std::process::exit(1);
        }
    }

    // --- Command Dispatch ---
    match args.command {
        Some(Commands::Test { dir }) => {
            // Run tests command using the specified (or default src/circuits) directory
            if !dir.exists() {
                 eprintln!("Error: Test directory does not exist: {:?}", dir);
                 eprintln!("Please create the directory or specify a different one using --dir.");
                 std::process::exit(1);
            }
            println!("Starting test runner in directory: {:?}", dir);
            let tester = CircuitTester::new();
            match tester.run_tests_in_dir(&dir, &builder) {
                Ok(summary) => {
                    // Exit with non-zero code if any tests failed
                    if summary.failed > 0 {
                        println!("\nTest run finished: {} passed, {} failed.", summary.passed, summary.failed);
                        std::process::exit(1);
                    } else {
                        println!("\nTest run finished: {} passed, {} failed. All tests passed!", summary.passed, summary.failed);
                         std::process::exit(0);
                    }
                }
                Err(e) => {
                    eprintln!("Test runner failed: {}", e);
                    std::process::exit(1);
                }
            }
        }
        None => {
            println!("\nNo command specified. Use 'test' to run tests (defaults to --dir ./src/circuits).");
            println!("Use '--help' for more options.");
        }
    }
}
