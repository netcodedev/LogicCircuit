# Counts the primes below 1000 by trial division (div/rem) and shows the
# count on the display (168 = 0xA8); the primes go to the buffer at "list".
    li s0, 0x10000000
    la s1, list
    li a0, 0                # count
    li a1, 2                # candidate
    li a2, 1000
next:
    li t0, 2
1:  mul t1, t0, t0
    bgtu t1, a1, prime      # t0 * t0 > candidate: no divisor
    remu t2, a1, t0
    beqz t2, composite
    addi t0, t0, 1
    j 1b
prime:
    slli t3, a0, 2
    add t3, t3, s1
    sw a1, 0(t3)
    addi a0, a0, 1
composite:
    addi a1, a1, 1
    bltu a1, a2, next
    sw a0, 8(s0)            # DISPLAY
    ebreak
    .align 4
list:
