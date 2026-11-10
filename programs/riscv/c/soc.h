/* Peripherals of SOC_RV32 (see ROADMAP_32BIT.md). */
#include <stdint.h>

#define IO(off) (*(volatile uint32_t *)(0x10000000u + (off)))
#define GPIO_OUT IO(0x00)
#define GPIO_IN IO(0x04)
#define DISPLAY IO(0x08)
#define UART_DATA IO(0x0C)
#define UART_STATUS IO(0x10)
#define UART_CTRL IO(0x14)
#define UART_DIV IO(0x18)

static inline void putc_uart(char c) {
    while (UART_STATUS & 1) {
    }
    UART_DATA = (uint8_t)c;
}

static inline void puts_uart(const char *s) {
    while (*s) putc_uart(*s++);
}

/* Prints v in decimal. */
static inline void put_dec(uint32_t v) {
    char buf[11];
    int i = 10;
    buf[i] = 0;
    do {
        buf[--i] = '0' + v % 10;
        v /= 10;
    } while (v);
    puts_uart(buf + i);
}
