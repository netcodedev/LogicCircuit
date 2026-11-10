; UART echo on SOC_16B: receives a line, stores it byte by byte (SB) and
; sends it back reversed and in upper case (LB). LB/SB take byte
; addresses: byte address 0x2000 is the low byte of word 0x1000.
; Run: cargo run -- run programs/soc/echo.asm --uart-in "hello\n"

        LI   R1, 0xF000     ; I/O base
        MOVI R2, 3
        SW   R2, [R1+5]     ; UART_DIV: 4 clocks per bit
        LI   R3, 0x2000     ; buffer (byte address)
        MOV  R4, R3
recv:   LW   R2, [R1+4]     ; UART_STATUS
        ANDI R2, 2          ; bit 1: a byte arrived
        BEQ  recv
        LW   R2, [R1+8]     ; UART_RX (acknowledges it)
        CMPI R2, 10         ; newline ends the line
        BEQ  send
        CMPI R2, 0x61       ; 'a'..'z' -> upper case
        BLO  store
        CMPI R2, 0x7B
        BHS  store
        ADDI R2, -32
store:  SB   R2, [R4]
        INC  R4
        BRA  recv
send:   CMP  R4, R3         ; back to the start of the buffer?
        BEQ  done
        DEC  R4
        LB   R2, [R4]
busy:   LW   R5, [R1+4]
        ANDI R5, 1          ; bit 0: still sending
        BNE  busy
        SW   R2, [R1+3]     ; UART_DATA
        BRA  send
done:   LW   R5, [R1+4]     ; wait until the last byte is out
        ANDI R5, 1
        BNE  done
        HALT
