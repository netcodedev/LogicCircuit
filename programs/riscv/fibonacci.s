# Fibonacci numbers up to 2^32 on the display; the last one stays there
# (0xB11924E1 = F(47)), a0 = how many there were.
    li s0, 0x10000000
    li t0, 0
    li t1, 1
    li a0, 1
loop:
    sw t1, 8(s0)            # DISPLAY
    add t2, t0, t1
    bltu t2, t1, done       # overflow
    mv t0, t1
    mv t1, t2
    addi a0, a0, 1
    j loop
done:
    ebreak
