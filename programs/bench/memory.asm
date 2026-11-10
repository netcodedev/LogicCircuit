; Benchmark: memory. Fills 48 words, copies them and sums the copy (many
; loads and stores, load-use pairs).
; Out = sum of 3*i for i in 0..47 = 3384 (0x0D38)

        LI   R10, 0x2000    ; source
        MOVI R12, 48
        MOVI R1, 0
fill:   SW   R1, [R10]
        ADDI R1, 3
        INC  R10
        DEC  R12
        BNE  fill

        LI   R10, 0x2000
        LI   R11, 0x3000    ; copy
        MOVI R12, 48
copy:   LW   R1, [R10]
        SW   R1, [R11]
        INC  R10
        INC  R11
        DEC  R12
        BNE  copy

        LI   R11, 0x3000
        MOVI R12, 48
        MOVI R5, 0
sum:    LW   R1, [R11]
        ADD  R5, R1
        INC  R11
        DEC  R12
        BNE  sum
        OUT  R5
        HALT
