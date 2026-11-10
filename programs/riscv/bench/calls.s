# Recursive calls with the stack: fib(15) the slow way.
    lui sp, 0x100
    li a0, 15
    call fib
    ebreak
fib:
    li t0, 2
    blt a0, t0, 1f
    addi sp, sp, -12
    sw ra, 0(sp)
    sw s0, 4(sp)
    sw s1, 8(sp)
    mv s0, a0
    addi a0, a0, -1
    call fib
    mv s1, a0
    addi a0, s0, -2
    call fib
    add a0, a0, s1
    lw ra, 0(sp)
    lw s0, 4(sp)
    lw s1, 8(sp)
    addi sp, sp, 12
1:  ret
