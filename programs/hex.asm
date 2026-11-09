; The same arithmetic as raw machine words: 5 + 3 = 8 on the display.
        .word 0x9105        ; ADDI R1, R0, 5
        .word 0x9203        ; ADDI R2, R0, 3
        .word 0x1312        ; ADD  R3, R1, R2
        .word 0x6030        ; OUT  R3
        .word 0x0000        ; HALT
