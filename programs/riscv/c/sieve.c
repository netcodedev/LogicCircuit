// Sieve of Eratosthenes: counts the primes below 2000 (303), prints them
// on the UART and returns the count (shown on the display).
//   cargo run --release -- riscv run programs/riscv/c/sieve.c
#include "soc.h"

#define N 2000
static uint8_t composite[N];

int main(void) {
    int count = 0;
    for (int i = 2; i < N; i++) {
        if (composite[i]) continue;
        count++;
        for (int j = i * i; j < N; j += i) composite[j] = 1;
    }
    puts_uart("primes below 2000: ");
    put_dec(count);
    putc_uart('\n');
    return count;
}
