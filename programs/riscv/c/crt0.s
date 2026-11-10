# Start code for C programs: stack at the top of the 1 MiB RAM, clear .bss,
# call main, show its result on the display and halt.
    .section .text.start
    .globl _start
_start:
    lui sp, 0x100           # 0x0010_0000
    la t0, __bss_start
    la t1, __bss_end
1:  bgeu t0, t1, 2f
    sw zero, 0(t0)
    addi t0, t0, 4
    j 1b
2:  call main
    lui t0, 0x10000
    sw a0, 8(t0)            # DISPLAY
    ebreak
