; Fibonacci numbers 0, 1, 1, 2, ... 233 (0xE9) on the 7-segment display.
        ADDI R1, R0, 0      ; a
        ADDI R2, R0, 1      ; b
        ADDI R4, R0, 13     ; numbers left after the first
loop:   OUT  R1
        BRZ  R4, done
        ADD  R3, R1, R2     ; next = a + b
        ADD  R1, R2, R0     ; a = b
        ADD  R2, R3, R0     ; b = next
        SUBI R4, R4, 1
        JMP  loop
done:   HALT
