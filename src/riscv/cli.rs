//! The `riscv` subcommands: assemble (or compile) a program and run it on
//! the instruction-level simulator or on SOC_RV32, and the benchmarks.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crate::cldl::CircuitBuilder;

use super::asm::{assemble, listing};
use super::isa::{REG_NAMES, disassemble};
use super::machine::Machine;
use super::sim::Rv32;

/// Default circuit of `riscv run` / `riscv bench`.
pub const DEFAULT_TOP: &str = "SOC_RV32";

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

/// The program's words: `.s`/`.asm` with the built-in assembler, `.c` with
/// clang + lld (see `programs/riscv/c/`).
pub fn load_program(path: &Path) -> Vec<u32> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("c") => compile_c(path),
        _ => {
            let src = std::fs::read_to_string(path).unwrap_or_else(|e| fail(format!("{}: {e}", path.display())));
            assemble(&src).unwrap_or_else(|e| fail(format!("{}:{e}", path.display()))).words
        }
    }
}

/// Compiles a C file for RV32IM with clang, links it with the start code
/// and linker script next to it (or in `programs/riscv/c/`) and returns the
/// flat binary from address 0.
fn compile_c(path: &Path) -> Vec<u32> {
    let support = [path.parent().unwrap_or(Path::new(".")).to_path_buf(), PathBuf::from("programs/riscv/c")]
        .into_iter()
        .find(|d| d.join("link.ld").is_file() && d.join("crt0.s").is_file())
        .unwrap_or_else(|| fail("crt0.s/link.ld not found next to the program or in programs/riscv/c"));
    let dir = std::env::temp_dir().join(format!("logiccircuit-rv32-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| fail(e));
    let elf = dir.join("prog.elf");
    let bin = dir.join("prog.bin");
    let flags = ["--target=riscv32", "-march=rv32im_zicsr", "-mabi=ilp32", "-mno-relax"];
    let run = |cmd: &mut Command| {
        let out = cmd.output().unwrap_or_else(|e| fail(format!("{:?}: {e} (clang/lld installed?)", cmd.get_program())));
        if !out.status.success() {
            fail(format!("{:?} failed:\n{}", cmd.get_program(), String::from_utf8_lossy(&out.stderr)));
        }
    };
    run(Command::new("clang")
        .args(flags)
        .args(["-O2", "-ffreestanding", "-nostdlib", "-fuse-ld=lld", "-static"])
        .arg("-I")
        .arg(&support)
        .arg("-T")
        .arg(support.join("link.ld"))
        .arg(support.join("crt0.s"))
        .arg(path)
        .arg("-o")
        .arg(&elf));
    run(Command::new("llvm-objcopy").args(["-O", "binary"]).arg(&elf).arg(&bin));
    let bytes = std::fs::read(&bin).unwrap_or_else(|e| fail(e));
    let _ = std::fs::remove_dir_all(&dir);
    bytes.chunks(4).map(|c| c.iter().rev().fold(0, |w, &b| w << 8 | b as u32)).collect()
}

/// `riscv asm`: the listing (address, word, source line).
pub fn asm(path: &Path) {
    let src = std::fs::read_to_string(path).unwrap_or_else(|e| fail(format!("{}: {e}", path.display())));
    let p = assemble(&src).unwrap_or_else(|e| fail(format!("{}:{e}", path.display())));
    for line in listing(&p) {
        println!("{line}");
    }
}

pub struct RunOptions {
    pub top: Option<String>,
    pub iss: bool,
    pub max: u64,
    pub trace: bool,
    pub latency: usize,
    pub uart_in: String,
    pub gpio_in: u32,
}

fn print_regs(x: &[u32; 32]) {
    for row in 0..8 {
        let cells: Vec<String> = (0..4).map(|c| row + 8 * c).map(|r| format!("{:>4} {:08x}", REG_NAMES[r], x[r])).collect();
        println!("  {}", cells.join("   "));
    }
}

/// `riscv run`: runs a program on the ISS (`--iss`) or a circuit.
pub fn run(builder: &CircuitBuilder, path: &Path, o: &RunOptions) {
    let words = load_program(path);
    if o.iss {
        let mut m = Rv32::new(&words);
        m.io.uart_in.extend(o.uart_in.bytes());
        m.io.gpio_in = o.gpio_in;
        let started = Instant::now();
        let mut steps = 0;
        while steps < o.max && !m.halted {
            if o.trace {
                println!("  {:08x}  {}", m.pc, disassemble(m.pc, m.ram_word(m.pc)));
            }
            m.step();
            steps += 1;
        }
        let secs = started.elapsed().as_secs_f64();
        println!("\n{} on the instruction-level simulator", path.display());
        println!("{} after {steps} instructions ({:.1} MHz)", if m.halted { "Halted" } else { "Stopped" }, steps as f64 / secs / 1e6);
        print_regs(&m.x);
        println!("  display {:08x}  gpio {:08x}", m.io.display, m.io.gpio_out);
        if !m.io.uart_out.is_empty() {
            println!("UART: {}", String::from_utf8_lossy(&m.io.uart_out));
        }
        return;
    }
    let top = o.top.as_deref().unwrap_or(DEFAULT_TOP);
    if builder.describe(top).is_none() {
        fail(format!("Circuit '{top}' not found (use -d to load its directory)."));
    }
    let c = builder.build(top);
    let mut m = Machine::new(&c, &words).unwrap_or_else(|| fail(format!("'{top}' does not have CPU_RV32's bus")));
    m.latency = o.latency;
    m.gpio_in = Some(o.gpio_in);
    m.uart_in.extend(o.uart_in.bytes());
    m.reset(&c);
    println!("\nRunning {} ({} words) on {top}", path.display(), words.len());
    let started = Instant::now();
    while m.clocks < o.max && !m.halted(&c) {
        if o.trace {
            let pc = m.pc(&c);
            println!("  [{:6}] PC={pc:08x}  {}", m.clocks, disassemble(pc, m.word(pc)));
        }
        m.step(&c);
    }
    let secs = started.elapsed().as_secs_f64();
    let halted = m.halted(&c);
    let clocks = m.clocks;
    // Let the UART finish its last byte.
    if c.has_wire("Tx") {
        for _ in 0..(m.uart_div(&c) as usize + 1) * 12 {
            m.clock(&c);
        }
    }
    println!(
        "{} after {} clocks ({:.2} kHz simulated)",
        if halted { "Halted" } else { "Stopped" },
        clocks,
        clocks as f64 / secs / 1e3
    );
    print_regs(&m.regs(&c));
    if c.has_wire("Tx") {
        println!("  display {:08x}  gpio {:08x}", m.display(&c), m.gpio_out(&c));
        if !m.uart.is_empty() {
            println!("UART: {}", String::from_utf8_lossy(&m.uart));
        }
    }
}

/// `riscv bench`: every program in `dir` on the ISS (instructions) and on
/// the circuits (clocks, clocks per instruction); checks that the circuit
/// ends with the same registers and output as the ISS.
pub fn bench(builder: &CircuitBuilder, dir: &Path, tops: &[&str], latency: usize) {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| fail(format!("{}: {e}", dir.display())))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("s" | "c")))
        .collect();
    paths.sort();
    print!("{:<14} {:>10}", "program", "instrs");
    for t in tops {
        print!(" {:>12} {:>6}", t, "CPI");
    }
    println!();
    for path in &paths {
        let words = load_program(path);
        let mut iss = Rv32::new(&words);
        let steps = iss.run(50_000_000);
        let name = path.file_stem().unwrap().to_string_lossy();
        print!("{name:<14} {steps:>10}");
        for top in tops {
            // A fresh circuit per program: the reset does not clear the registers.
            let c = &builder.build(top);
            let mut m = Machine::new(c, &words).unwrap_or_else(|| fail(format!("'{top}' does not have CPU_RV32's bus")));
            m.latency = latency;
            m.reset(c);
            let clocks = m.run(c, 200_000_000);
            let ok = m.halted(c) && m.regs(c) == iss.x;
            print!(" {clocks:>12} {:>6.2}{}", clocks as f64 / steps as f64, if ok { "" } else { " (differs!)" });
        }
        println!();
    }
}
