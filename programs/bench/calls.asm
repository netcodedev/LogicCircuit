; Benchmark: calls. Recursive Fibonacci fib(9) with the stack (CALL, RET,
; PUSH, POP).
; Out = fib(9) = 34 (0x0022)

        MOVI R1, 9
        CALL fib
        OUT  R2
        HALT

; R2 = fib(R1); keeps R1
fib:    CMPI R1, 2
        BLO  small
        PUSH R1
        PUSH R3
        DEC  R1
        CALL fib            ; R2 = fib(n-1)
        MOV  R3, R2
        DEC  R1
        CALL fib            ; R2 = fib(n-2)
        ADD  R2, R3
        POP  R3
        POP  R1
        RET
small:  MOV  R2, R1
        RET
