; Benchmark: multiplication. Dot product of two 8-element vectors with a
; shift-and-add multiply subroutine (no multiplier in hardware).
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
        CALL mul            ; R3 = R1 * R2
        ADD  R5, R3
        INC  R10
        INC  R11
        DEC  R12
        BNE  dot
        OUT  R5
        HALT

; R3 = R1 * R2 (low 16 bits); uses R4
mul:    MOVI R3, 0
        MOV  R4, R1
mloop:  CMPI R2, 0
        BEQ  mdone
        MOV  R6, R2
        ANDI R6, 1
        BEQ  mskip
        ADD  R3, R4
mskip:  SHL  R4, 1
        SHR  R2, 1
        BRA  mloop
mdone:  RET
