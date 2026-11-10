; Benchmark: multiplication in hardware (MULL, CPU_16B_SOC only). Same dot
; product as mul.asm.
; Out = 1*8 + 2*7 + ... + 8*1 = 120 (0x0078)

        LI   R10, 0x1000    ; vector a
        LI   R11, 0x1010    ; vector b
        MOVI R12, 8         ; fill a[i] = i+1, b[i] = 8-i
        MOVI R1, 1
fill:   SW   R1, [R10]
        MOV  R2, R12
        SW   R2, [R11]
        INC  R1
        INC  R10
        INC  R11
        DEC  R12
        BNE  fill

        LI   R10, 0x1000
        LI   R11, 0x1010
        MOVI R12, 8
        MOVI R5, 0          ; sum
dot:    LW   R1, [R10]
        LW   R2, [R11]
        MULL R1, R2
        ADD  R5, R1
        INC  R10
        INC  R11
        DEC  R12
        BNE  dot
        OUT  R5
        HALT
