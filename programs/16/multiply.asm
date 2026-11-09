; 1234 x 37 = 45658 (0xB25A) with a shift-and-add subroutine.
        LI   R1, 1234
        MOVI R2, 37
        CALL mul
        OUT  R3
        HALT

; R3 = R1 x R2 (low 16 bits). Keeps all other registers.
mul:    PUSH R1
        PUSH R2
        PUSH R4
        MOVI R3, 0
mloop:  CMPI R2, 0
        BEQ  mdone
        MOV  R4, R2
        ANDI R4, 1          ; lowest bit of the multiplier
        BEQ  mskip
        ADD  R3, R1
mskip:  SHL  R1, 1
        SHR  R2, 1
        BRA  mloop
mdone:  POP  R4
        POP  R2
        POP  R1
        RET
