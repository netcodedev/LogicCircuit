# Insertion sort of 64 pseudo-random words.
    la s0, data
    li s1, 64
    li t0, 0
    li a0, 7
1:  slli t1, t0, 2
    add t1, t1, s0
    li t2, 1664525
    mul a0, a0, t2
    li t2, 1013904223
    add a0, a0, t2
    sw a0, 0(t1)
    addi t0, t0, 1
    blt t0, s1, 1b
    li t0, 1
2:  slli t1, t0, 2
    add t1, t1, s0
    lw t2, 0(t1)            # key
    mv t3, t1
3:  beq t3, s0, 4f
    lw t4, -4(t3)
    bgeu t2, t4, 4f
    sw t4, 0(t3)
    addi t3, t3, -4
    j 3b
4:  sw t2, 0(t3)
    addi t0, t0, 1
    blt t0, s1, 2b
    lw a0, 0(s0)
    lw a1, 252(s0)
    ebreak
    .align 4
data:
