; Benchmark: division. Sum of the decimal digits of three numbers, with a
; shift-and-subtract divide-by-10 subroutine (no divider in hardware).
; Out = (6+5+5+3+5) + (1+2+3+4+5) + (9+9+9) = 24 + 15 + 27 = 66 (0x0042)
; (The numbers are stored to data memory first: on the Harvard CPUs, LW
; cannot read the program.)

        LI   R10, 0x1000
        LI   R1, 65535
        SW   R1, [R10]
        LI   R1, 12345
        SW   R1, [R10+1]
        LI   R1, 999
        SW   R1, [R10+2]

        MOVI R12, 3
        MOVI R5, 0          ; digit sum
next:   LW   R1, [R10]
digit:  CMPI R1, 0
        BEQ  done1
        MOVI R2, 10
        CALL divu           ; R3 = R1 / 10, R4 = R1 % 10
        ADD  R5, R4
        MOV  R1, R3
        BRA  digit
done1:  INC  R10
        DEC  R12
        BNE  next
        OUT  R5
        HALT

; R3 = R1 / R2, R4 = R1 % R2 (unsigned, R2 != 0); uses R6, R7
divu:   MOVI R3, 0
        MOVI R4, 0
        MOVI R6, 16         ; bits
        MOV  R7, R1
dloop:  SHL  R4, 1          ; remainder <<= 1 | top bit of dividend
        SHL  R3, 1
        ADD  R7, R7         ; C = top bit
        BCC  dnob
        ORI  R4, 1
dnob:   CMP  R4, R2
        BLO  dsmall
        SUB  R4, R2
        ORI  R3, 1
dsmall: DEC  R6
        BNE  dloop
        RET
