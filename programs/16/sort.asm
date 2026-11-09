; Bubble sort of eight words in data memory (0x0100..0x0107), then shows
; them in ascending order. The largest, 0x1234, stays on the display.
        LI   R1, 0x0100     ; array base
        LI   R2, 503
        SW   R2, [R1]
        LI   R2, 0x1234
        SW   R2, [R1+1]
        MOVI R2, 7
        SW   R2, [R1+2]
        LI   R2, 999
        SW   R2, [R1+3]
        MOVI R2, 42
        SW   R2, [R1+4]
        MOVI R2, 0
        SW   R2, [R1+5]
        LI   R2, 0x0FFF
        SW   R2, [R1+6]
        MOVI R2, 1
        SW   R2, [R1+7]

        MOVI R5, 7          ; passes left
outer:  MOV  R6, R1         ; element pointer
        MOV  R7, R5         ; comparisons in this pass
inner:  LW   R2, [R6]
        LW   R3, [R6+1]
        CMP  R3, R2
        BHS  noswap         ; already in order (unsigned)
        SW   R3, [R6]
        SW   R2, [R6+1]
noswap: INC  R6
        DEC  R7
        BNE  inner
        DEC  R5
        BNE  outer

        MOV  R6, R1
        MOVI R7, 8
show:   LW   R2, [R6]
        OUT  R2
        INC  R6
        DEC  R7
        BNE  show
        HALT
