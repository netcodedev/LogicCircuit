# Timer interrupt every 1000 clocks: the handler counts on the display;
# after 10 interrupts the program stops.
    li s0, 0x10000000
    la t0, handler
    csrw mtvec, t0
    lw t1, 0x20(s0)         # mtime
    addi t1, t1, 1000
    sw zero, 0x2C(s0)
    sw t1, 0x28(s0)         # mtimecmp
    li t0, 0x80
    csrw mie, t0            # MTIE
    csrsi mstatus, 8        # MIE
    li s2, 10
1:  wfi
    blt s1, s2, 1b
    ebreak

handler:
    addi s1, s1, 1
    sw s1, 8(s0)            # DISPLAY
    lw t1, 0x28(s0)
    addi t1, t1, 1000
    sw t1, 0x28(s0)         # next interrupt (acknowledges this one)
    mret
