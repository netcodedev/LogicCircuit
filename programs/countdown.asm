; Countdown 9..0 on the 7-segment display.
        ADDI R1, R0, 9
loop:   OUT  R1
        BRZ  R1, done
        SUBI R1, R1, 1
        JMP  loop
done:   HALT
