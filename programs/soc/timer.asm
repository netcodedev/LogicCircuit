; Timer interrupts on SOC_16B: the timer interrupts every 200 clocks, the
; handler counts the ticks in R9 and shows them on the display. The main
; program spins on CMPI/BLT until it sees 8 ticks: an interrupt between the
; two would break the loop if RETI did not restore the flags.
; Out = 8 (ticks seen by the loop), R5 = loop rounds.

        JMP  main           ; two words: the handler starts at 0x0002

isr:    PUSH R1             ; 0x0002: interrupt vector
        PUSH R2
        INC  R9             ; ticks (changes the flags; RETI restores them)
        LI   R1, 0xF000
        SW   R9, [R1+2]     ; DISPLAY
        MOVI R2, 3
        SW   R2, [R1+7]     ; TIMER_CTRL: keep running, acknowledge
        POP  R2
        POP  R1
        RETI

main:   LI   R1, 0xF000
        MOVI R2, 199
        SW   R2, [R1+6]     ; TIMER: 200 clocks per tick (longer than the
                            ; handler, also with slow memory)
        MOVI R2, 3
        SW   R2, [R1+7]     ; TIMER_CTRL: enable, interrupt enable
        MOVI R9, 0
        MOVI R5, 0
        EI
loop:   INC  R5
        MOV  R6, R9         ; snapshot: a tick may still come after the loop
        CMPI R6, 8
        BLT  loop
        DI
        OUT  R6
        HALT
