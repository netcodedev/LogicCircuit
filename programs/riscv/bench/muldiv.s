# Multiply and divide: a linear congruential generator, digits by division.
    li a0, 12345
    li a1, 1103515245
    li a2, 0                # digit sum
    li s0, 200
    li s1, 10
1:  mul a0, a0, a1
    addi a0, a0, 1234
    srli t0, a0, 8
2:  remu t1, t0, s1
    add a2, a2, t1
    divu t0, t0, s1
    bnez t0, 2b
    addi s0, s0, -1
    bnez s0, 1b
    ebreak
