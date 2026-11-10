; Benchmark: division in hardware (DIVU/MODU, CPU_16B_SOC only). Same digit
; sum as div.asm.
; Out = 24 + 15 + 27 = 66 (0x0042)

        LI   R10, 0x1000
        LI   R1, 65535
        SW   R1, [R10]
        LI   R1, 12345
        SW   R1, [R10+1]
        LI   R1, 999
        SW   R1, [R10+2]

        MOVI R2, 10
        MOVI R12, 3
        MOVI R5, 0          ; digit sum
next:   LW   R1, [R10]
digit:  CMPI R1, 0
        BEQ  done1
        MOV  R4, R1
        MODU R4, R2         ; last digit
        ADD  R5, R4
        DIVU R1, R2
        BRA  digit
done1:  INC  R10
        DEC  R12
        BNE  next
        OUT  R5
        HALT
