//! CPU_RV32 and SOC_RV32 against the instruction-level simulator: the same
//! program on both, then the registers, the memory and the I/O must match.

use crate::circuit::Circuit;

use super::asm::assemble;
use super::machine::Machine;
use super::sim::Rv32;
use super::tests::{Rng, builder};

/// A fresh circuit per run: reset does not clear the register file, and the
/// ISS starts with all registers 0.
fn circuit(name: &str) -> Circuit {
    builder().build(name)
}

/// Runs `src` on the ISS and on the circuit and compares the results.
/// Returns the ISS state and the clocks the circuit took.
fn compare(c: &Circuit, src: &str, latency: usize) -> (Rv32, u64) {
    compare_io(c, src, latency, b"").0
}

/// [`compare`] with bytes to receive on the UART; also compares what was
/// sent and (SoC) the display and GPIO outputs. Returns the machine too.
fn compare_io(c: &Circuit, src: &str, latency: usize, input: &[u8]) -> ((Rv32, u64), Machine) {
    let p = assemble(src).unwrap_or_else(|e| panic!("{e}"));
    let mut iss = Rv32::new(&p.words);
    iss.io.uart_in.extend(input);
    iss.run(1_000_000);
    assert!(iss.halted, "ISS: no ebreak");
    let mut m = Machine::new(c, &p.words).unwrap();
    m.latency = latency;
    m.uart_in.extend(input);
    m.reset(c);
    let clocks = m.run(c, 2_000_000);
    assert!(m.halted(c), "circuit did not halt, PC {:#x}", m.pc(c));
    if c.has_wire("Tx") {
        // Let the UART finish sending.
        for _ in 0..2000 {
            m.clock(c);
        }
    }
    let regs = m.regs(c);
    for i in 0..32 {
        assert_eq!(regs[i], iss.x[i], "x{i}: circuit {:#x}, ISS {:#x}\n{src}", regs[i], iss.x[i]);
    }
    if let Some(i) = (0..m.ram.len()).find(|&i| m.ram[i] != iss.ram[i]) {
        panic!("RAM[{i:#x}]: circuit {:#x}, ISS {:#x}", m.ram[i], iss.ram[i]);
    }
    if c.has_wire("Tx") {
        assert_eq!(m.uart, iss.io.uart_out, "UART");
        assert_eq!(m.display(c), iss.io.display, "display");
        assert_eq!(m.gpio_out(c), iss.io.gpio_out, "GPIO");
    }
    ((iss, clocks), m)
}

fn cpu(src: &str) -> (Rv32, u64) {
    compare(&circuit("CPU_RV32"), src, 0)
}

#[test]
fn alu_and_branches() {
    cpu("
        li a0, 0x12345678
        li a1, -7
        add a2, a0, a1
        sub a3, a0, a1
        and a4, a0, a1
        or a5, a0, a1
        xor a6, a0, a1
        slt a7, a1, a0
        sltu s0, a1, a0
        sll s1, a0, a1
        srl s2, a1, a0
        sra s3, a1, a0
        slli s4, a0, 3
        srai s5, a1, 1
        sltiu s6, a0, -1
        xori s7, a0, -1
        auipc s8, 0x12345
        lui s9, 0xFEDCB
        li t0, 0
        li t1, 10
    loop:
        addi t0, t0, 3
        addi t1, t1, -1
        bnez t1, loop
        blt a1, a0, l1
        li t2, 99
    l1: bge a1, a0, l2
        li t3, 42
    l2: bltu a1, a0, l3
        li t4, 7
    l3: jal ra, sub1
        la t5, sub1
        jalr ra, 0(t5)
        ebreak
    sub1:
        addi t6, t6, 1
        ret
    ");
}

#[test]
fn loads_stores_and_forwarding() {
    cpu("
        la s0, data
        li a0, 0x8090A0B0
        sw a0, 0(s0)
        lw a1, 0(s0)
        add a2, a1, a1        // load-use
        lb a3, 0(s0)
        lbu a4, 1(s0)
        lh a5, 2(s0)
        lhu a6, 2(s0)
        sb a0, 5(s0)
        sh a0, 6(s0)
        lw a7, 4(s0)
        addi s1, s0, 8
        sw s1, 0(s1)          // forwarded address and data
        lw s2, 0(s1)
        lw s3, 0(s2)
        addi s3, s3, 1
        sw s3, 12(s0)
        ebreak
        .align 4
    data:
        .word 0, 0, 0, 0
    ");
}

#[test]
fn multiply_and_divide() {
    let mut rng = Rng(7);
    let mut src = String::new();
    let ops = ["mul", "mulh", "mulhsu", "mulhu", "div", "divu", "rem", "remu"];
    for k in 0..48 {
        let (a, b) = (rng.word() as i32, rng.word() as i32);
        let b = if k % 8 == 0 { 0 } else { b };
        let a = if k % 16 == 1 { i32::MIN } else { a };
        let b = if k % 16 == 1 { -1 } else { b };
        src += &format!("li a0, {a}\nli a1, {b}\n{} a{}, a0, a1\n", ops[k % 8], 2 + k % 6);
        src += &format!("add s{}, s{}, a{}\n", k % 8, k % 8, 2 + k % 6);
    }
    src += "mul t0, s0, s1\nmul t0, t0, t0\ndiv t1, t0, s2\nebreak\n";
    cpu(&src);
}

#[test]
fn traps_and_csrs() {
    cpu("
        la t0, handler
        csrw mtvec, t0
        csrwi mscratch, 5
        csrrs a0, mscratch, t0   // a0 = 5
        csrrc a1, mscratch, t0
        csrr a2, misa
        csrr a3, mhartid
        csrr a4, mstatus
        ecall
        .word 0xFFFFFFFF          // illegal
        la t1, data
        lw a5, 2(t1)              // misaligned load
        sh a5, 1(t1)              // misaligned store
        csrw cycle, a5            // read-only: illegal
        csrr a6, 0x7C0            // does not exist: illegal
        la t2, handler
        jalr ra, 2(t2)            // misaligned target
        csrr s5, minstret
        csrwi minstret, 3
        csrr s6, minstret
        ebreak
    handler:
        csrr s0, mcause
        csrr s1, mepc
        csrr s2, mtval
        add s3, s3, s0
        xor s4, s4, s2
        addi s1, s1, 4
        csrw mepc, s1
        mret
        .align 4
    data:
        .word 0x11223344
    ");
}

#[test]
fn misaligned_branch_targets() {
    // Taken branches to a target with bit 1 set trap (mepc = the branch);
    // not-taken ones do not. Forward (predicted not taken) and backward
    // (predicted taken) branches, and a jal.
    let (iss, _) = cpu("
        la t0, handler
        csrw mtvec, t0
        li a0, 1
    back:
        beq a0, a0, fwd+2         // forward, taken: trap
        bne a0, a0, fwd+2         // forward, not taken
        blt x0, a0, back+2        // backward, taken: trap
        bge x0, a0, back+2        // backward, not taken
        jal x0, fwd+2             // trap
        addi s7, s7, 1
    fwd:
        addi s8, s8, 1
        ebreak
    handler:
        csrr s0, mcause
        csrr s1, mepc
        csrr s2, mtval
        add s3, s3, s0
        xor s4, s4, s2
        add s5, s5, s1
        addi s6, s6, 1
        addi s1, s1, 4
        csrw mepc, s1
        mret
    ");
    assert_eq!(iss.x[22], 3, "three traps");
}

/// A random program: register-register and immediate operations, loads
/// and stores into a buffer (base in x31), short forward branches.
fn random_program(rng: &mut Rng, len: usize) -> String {
    let r_ops = [
        "add", "sub", "sll", "slt", "sltu", "xor", "srl", "sra", "or", "and", "mul", "mulh", "mulhsu", "mulhu", "div",
        "divu", "rem", "remu",
    ];
    let i_ops = ["addi", "slti", "sltiu", "xori", "ori", "andi"];
    let sh_ops = ["slli", "srli", "srai"];
    let loads = [("lb", 1), ("lbu", 1), ("lh", 2), ("lhu", 2), ("lw", 4)];
    let stores = [("sb", 1), ("sh", 2), ("sw", 4)];
    let mut src = String::from("la x31, buf\n");
    for r in 1..16 {
        src += &format!("li x{r}, {}\n", rng.word() as i32);
    }
    // Registers 1..15 are used most, so values flow between instructions.
    let reg = |rng: &mut Rng| if rng.next() % 4 == 0 { rng.next() % 31 } else { 1 + rng.next() % 15 };
    let mut label = 0;
    for _ in 0..len {
        let (rd, rs1, rs2) = (1 + reg(rng) % 30, reg(rng), reg(rng));
        src += &match rng.next() % 10 {
            0..=2 => format!("{} x{rd}, x{rs1}, x{rs2}\n", r_ops[rng.next() as usize % r_ops.len()]),
            3 | 4 => {
                let imm = (rng.next() % 4096) as i32 - 2048;
                format!("{} x{rd}, x{rs1}, {imm}\n", i_ops[rng.next() as usize % i_ops.len()])
            }
            5 => format!("{} x{rd}, x{rs1}, {}\n", sh_ops[rng.next() as usize % 3], rng.next() % 32),
            6 => {
                let (op, size) = loads[rng.next() as usize % loads.len()];
                format!("{op} x{rd}, {}(x31)\n", (rng.next() % 64) as u32 / size * size)
            }
            7 => {
                let (op, size) = stores[rng.next() as usize % stores.len()];
                format!("{op} x{rs2}, {}(x31)\n", (rng.next() % 64) as u32 / size * size)
            }
            8 => {
                let br = ["beq", "bne", "blt", "bge", "bltu", "bgeu"][rng.next() as usize % 6];
                label += 1;
                format!("{br} x{rs1}, x{rs2}, f{label}\naddi x{rd}, x{rd}, 1\nf{label}:\n")
            }
            _ => format!("lui x{rd}, {}\n", rng.next() % (1 << 20)),
        };
    }
    src += "ebreak\n.align 4\nbuf:\n.space 64\n";
    src
}

#[test]
fn random_programs() {
    let mut rng = Rng(0x5EED);
    for _ in 0..12 {
        let src = random_program(&mut rng, 150);
        cpu(&src);
    }
}

#[test]
fn slow_memory() {
    let mut rng = Rng(99);
    let src = random_program(&mut rng, 120);
    for latency in [1, 3] {
        compare(&circuit("CPU_RV32"), &src, latency);
    }
}


fn soc(src: &str, input: &[u8]) -> ((Rv32, u64), Machine) {
    compare_io(&circuit("SOC_RV32"), src, 0, input)
}

/// Sends the zero-terminated string at a0 on the UART (waits while busy).
const PUTS: &str = "
    puts:
        li t0, 0x10000000
    1:  lbu t1, 0(a0)
        beqz t1, 3f
    2:  lw t2, 0x10(t0)
        andi t2, t2, 1
        bnez t2, 2b
        sb t1, 0xC(t0)
        addi a0, a0, 1
        j 1b
    3:  ret
";

#[test]
fn soc_gpio_display_uart() {
    let src = format!(
        "
        li s0, 0x10000000
        li a0, 0xCAFE1234
        sw a0, 8(s0)          // display
        li a1, 0xA5
        sb a1, 1(s0)          // GPIO_OUT = 0xA500
        lw a2, 0(s0)
        lw a3, 8(s0)
        li a4, 3
        sw a4, 0x18(s0)       // UART: 4 clocks per bit
        lw a5, 0x18(s0)
        la a0, text
        call puts
        ebreak
        {PUTS}
    text:
        .asciz \"Hallo RISC-V\\n\"
    "
    );
    let ((iss, _), _) = soc(&src, b"");
    assert_eq!(iss.io.uart_out, b"Hallo RISC-V\n");
}

#[test]
fn soc_timer_interrupt() {
    // Three timer interrupts, each 200 clocks (ISS: instructions) after the
    // last; the main loop counts until the handler has run three times.
    soc(
        "
        li s0, 0x10000000
        la t0, handler
        csrw mtvec, t0
        lw t1, 0x20(s0)
        addi t1, t1, 200
        sw zero, 0x2C(s0)
        sw t1, 0x28(s0)
        li t0, 0x80
        csrw mie, t0          // MTIE
        csrsi mstatus, 8      // MIE
        li s2, 3
    1:  blt s1, s2, 1b
        csrci mstatus, 8
        csrr a0, mcause
        li t0, -1
        sw t0, 0x2C(s0)       // no more timer interrupts
        csrr a2, mip
        li t1, 0              // (time-dependent)
        ebreak
    handler:
        addi s1, s1, 1
        lw t1, 0x28(s0)
        addi t1, t1, 200
        sw t1, 0x28(s0)
        csrr s3, mcause
        mret
        ",
        b"",
    );
}

#[test]
fn soc_uart_receive_interrupt() {
    // Echo with the receive interrupt: the handler reads each byte and
    // sends it back (upper case); main waits for the newline.
    let src = format!(
        "
        li s0, 0x10000000
        la t0, handler
        csrw mtvec, t0
        li t0, 1
        sw t0, 0x14(s0)       // UART_CTRL: receive interrupt
        li t0, 7
        sw t0, 0x18(s0)       // 8 clocks per bit
        li t0, 0x800
        csrw mie, t0          // MEIE
        csrsi mstatus, 8
        li s2, 10
    1:  bne s1, s2, 1b
        la a0, done
        call puts
        ebreak
    handler:
        lbu s1, 0xC(s0)
        addi t0, s1, -97
        sltiu t0, t0, 26
        slli t0, t0, 5
        sub a0, s1, t0
    2:  lw t1, 0x10(s0)
        andi t1, t1, 1
        bnez t1, 2b
        sb a0, 0xC(s0)
        csrr s3, mcause
        mret
        {PUTS}
    done:
        .asciz \"ok\"
    "
    );
    let ((iss, _), _) = soc(&src, b"risc-v\n");
    assert_eq!(iss.io.uart_out, b"RISC-V\nok");
}
