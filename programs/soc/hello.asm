; Memory-mapped I/O on SOC_16B (I/O page 0xF0xx, see circuits/IO_16B.cldl):
; shows 0xC0DE on the display, sends "Hello!" over the UART and copies the
; GPIO inputs to the GPIO outputs.
; Run: cargo run -- run programs/soc/hello.asm --input 0 (default top SOC_16B)

        LI   R1, 0xF000     ; I/O base
        LI   R2, 0xC0DE
        SW   R2, [R1+2]     ; DISPLAY
        MOVI R2, 3
        SW   R2, [R1+5]     ; UART_DIV: 4 clocks per bit
        LI   R4, text
next:   LW   R2, [R4]
        CMPI R2, 0
        BEQ  done
busy:   LW   R3, [R1+4]     ; UART_STATUS
        ANDI R3, 1
        BNE  busy
        SW   R2, [R1+3]     ; UART_DATA: send
        INC  R4
        JMP  next
done:   LW   R3, [R1+4]     ; wait until the last byte is out
        ANDI R3, 1
        BNE  done
        LW   R3, [R1+1]     ; GPIO_IN
        SW   R3, [R1]       ; GPIO_OUT
        HALT

text:   .word 0x48          ; H
        .word 0x65          ; e
        .word 0x6C          ; l
        .word 0x6C          ; l
        .word 0x6F          ; o
        .word 0x21          ; !
        .word 0
