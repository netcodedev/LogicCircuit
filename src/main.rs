use clap::Parser;
use std::path::{Path, PathBuf};

mod circuit;
mod cldl;
mod clt;

use crate::cldl::CircuitBuilder;
use crate::clt::{CircuitTester, TestSummary}; // Import TestSummary

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
    }
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
        None => {
            println!("\nNo command specified. Use 'test' or 'manual-test'.");
            println!("Use '-d <DIR>' to specify an additional directory for circuits and tests.");
            println!("Use '--help' for more options.");
        }
    }
}
