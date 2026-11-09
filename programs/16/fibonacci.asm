; Fibonacci numbers 0, 1, 1, 2, ... 46368 (0xB520), the largest that fits
; in 16 bits, on the 7-segment display.
        MOVI R1, 0          ; a
        MOVI R2, 1          ; b
loop:   OUT  R1
        MOV  R3, R1
        ADD  R3, R2         ; next = a + b, C = 1 on overflow
        BCS  done
        MOV  R1, R2         ; a = b
        MOV  R2, R3         ; b = next
        BRA  loop
done:   OUT  R2
        HALT
