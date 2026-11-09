; n! for n on the input port (In), computed recursively on the stack.
; 8! = 40320 (0x9D80) is the largest that fits in 16 bits.
        IN   R1
        CALL fact
        OUT  R3
        HALT

; R3 = R1! (R1 and R2 are changed).
fact:   CMPI R1, 1
        BGT  frec
        MOVI R3, 1          ; 0! = 1! = 1
        RET
frec:   PUSH R1
        DEC  R1
        CALL fact           ; R3 = (n-1)!
        POP  R1
        MOV  R2, R3
        CALL mul            ; R3 = n x (n-1)!
        RET

; R3 = R1 x R2 (low 16 bits). Keeps all other registers.
mul:    PUSH R1
        PUSH R2
        PUSH R4
        MOVI R3, 0
mloop:  CMPI R2, 0
        BEQ  mdone
        MOV  R4, R2
        ANDI R4, 1
        BEQ  mskip
        ADD  R3, R1
mskip:  SHL  R1, 1
        SHR  R2, 1
        BRA  mloop
mdone:  POP  R4
        POP  R2
        POP  R1
        RET
