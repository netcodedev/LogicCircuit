# Loads and stores: fills 256 words, then sums them with load-use pairs and
# copies them byte by byte.
    la s0, buf
    li t0, 0
    li t1, 256
1:  slli t2, t0, 2
    add t2, t2, s0
    mul t3, t0, t0
    sw t3, 0(t2)
    addi t0, t0, 1
    blt t0, t1, 1b
    li a0, 0
    mv t0, s0
    addi t1, s0, 1024
2:  lw t2, 0(t0)
    add a0, a0, t2          # load-use
    addi t0, t0, 4
    bltu t0, t1, 2b
    mv t0, s0
    addi t3, s0, 1024
3:  lbu t2, 0(t0)
    sb t2, 1024(t0)
    addi t0, t0, 1
    bltu t0, t3, 3b
    ebreak
    .align 4
buf:
