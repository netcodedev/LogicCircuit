# Sends "Hello, RISC-V!" on the UART and shows 0x00C0FFEE on the display.
    .equ IO, 0x10000000
    .equ UART_DATA, 0x0C
    .equ UART_STATUS, 0x10

    li s0, IO
    li t0, 0x00C0FFEE
    sw t0, 8(s0)            # DISPLAY
    la a0, text
    call puts
    ebreak

# Sends the zero-terminated string at a0 (waits while the UART is busy).
puts:
1:  lbu t1, 0(a0)
    beqz t1, 3f
2:  lw t2, UART_STATUS(s0)
    andi t2, t2, 1
    bnez t2, 2b
    sb t1, UART_DATA(s0)
    addi a0, a0, 1
    j 1b
3:  ret

text:
    .asciz "Hello, RISC-V!\n"
