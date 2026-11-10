# Loops and taken/not-taken branches: counts the set bits of 0..499.
    li a0, 0                # total
    li a1, 0                # n
    li a2, 500
1:  mv t0, a1
2:  beqz t0, 3f
    andi t1, t0, 1
    add a0, a0, t1
    srli t0, t0, 1
    j 2b
3:  addi a1, a1, 1
    blt a1, a2, 1b
    ebreak
