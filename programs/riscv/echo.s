# UART echo with the receive interrupt: every byte comes back in upper
# case; a newline ends the program.
#   cargo run --release -- riscv run programs/riscv/echo.s --uart-in 'hallo\n'
    .equ IO, 0x10000000
    li s0, IO
    la t0, handler
    csrw mtvec, t0
    li t0, 7
    sw t0, 0x18(s0)         # UART_DIV: 8 clocks per bit
    li t0, 1
    sw t0, 0x14(s0)         # UART_CTRL: interrupt on receive
    li t0, 0x800
    csrw mie, t0            # MEIE
    csrsi mstatus, 8        # MIE
    li s2, '\n'
1:  wfi
    bne s1, s2, 1b
    ebreak

handler:
    lbu s1, 0x0C(s0)        # received byte (acknowledges it)
    addi t0, s1, -'a'
    sltiu t0, t0, 26
    slli t0, t0, 5
    sub a0, s1, t0          # upper case
2:  lw t1, 0x10(s0)
    andi t1, t1, 1
    bnez t1, 2b
    sb a0, 0x0C(s0)
    mret
