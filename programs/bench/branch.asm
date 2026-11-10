; Benchmark: branches. Counts the set bits of 0..31 with a bit-by-bit loop
; (many short loops, taken and not-taken branches).
; Out = 80 (0x0050)

        MOVI R1, 31         ; number
        MOVI R5, 0          ; total
outer:  MOV  R2, R1
inner:  CMPI R2, 0
        BEQ  idone
        MOV  R3, R2
        ANDI R3, 1
        BEQ  izero
        INC  R5
izero:  SHR  R2, 1
        BRA  inner
idone:  DEC  R1
        BPL  outer
        OUT  R5
        HALT
