/*
 * The first C program on RetroPane (docs/design/half-life-readiness.md,
 * lines 2 and 3): proves the C compiler, picolibc and the C boundary by
 * using what a game port leans on (printf with floats, the heap, software
 * floating point, the maths library, 64-bit division, sorting, parsing)
 * and checking every answer. `cargo xtask emu-chello` runs it and looks
 * for the last line.
 */
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int check(const char *what, int ok)
{
    printf("  %s: %s\n", what, ok ? "ok" : "FAILED");
    return ok;
}

static int by_value(const void *a, const void *b)
{
    return *(const int *)a - *(const int *)b;
}

/* The card's files through fopen (the port layer, rp_port::sys): a file
 * written, read back and removed in this program's own folder (the card is
 * shared: nothing outside it is touched), and a file the PC put on the
 * card read. Without a card the first open fails and the checks are
 * skipped; `cargo xtask emu-chello` gives a card and wants "files: ok". */
static int files(void)
{
    static const char path[] = "rp-chello/out.bin";
    static unsigned char out[20000], in[20000];
    int ok = 1;
    FILE *f = fopen(path, "wb");
    if (!f) {
        puts("  files: skipped (no card)");
        return 1;
    }
    for (size_t i = 0; i < sizeof out; i++)
        out[i] = (unsigned char)(i * 37 ^ i >> 8);
    /* In three pieces of different sizes, the last one odd. */
    size_t done = fwrite(out, 1, 7000, f);
    done += fwrite(out + 7000, 1, 12999, f);
    done += fwrite(out + 19999, 1, 1, f);
    ok &= check("fwrite", done == sizeof out);
    ok &= check("fclose puts it on the card", fclose(f) == 0);

    f = fopen("RP-CHELLO\\OUT.BIN", "rb");
    ok &= check("fopen in any case, either slash", f != NULL);
    if (f) {
        ok &= check("fseek to the end and ftell", fseek(f, 0, SEEK_END) == 0 && ftell(f) == (long)sizeof out);
        rewind(f);
        ok &= check("fread", fread(in, 1, sizeof in, f) == sizeof in && memcmp(in, out, sizeof in) == 0);
        ok &= check("the end of the file", fgetc(f) == EOF && feof(f));
        ok &= check("fseek into it", fseek(f, 12345, SEEK_SET) == 0 && fgetc(f) == out[12345]);
        fclose(f);
    }
    ok &= check("remove", remove(path) == 0 && fopen(path, "rb") == NULL);

    /* A file made on the PC (the emulator's card has it; a card without it
     * is no failure). */
    f = fopen("chello.txt", "r");
    if (f) {
        char line[40] = "";
        ok &= check("a file from the PC", fgets(line, sizeof line, f) && strcmp(line, "written by the PC\n") == 0);
        fclose(f);
    }
    puts(ok ? "  files: ok" : "  files: FAILED");
    return ok;
}

int main(void)
{
    int ok = 1;
    char line[80];

    printf("C on RetroPane: picolibc %s\n", __PICOLIBC_VERSION__);

    snprintf(line, sizeof line, "%d %u %x %s %c %5.2f %lld", -42, 42u, 0xbeef, "str", 'c',
             3.14159, 1234567890123LL);
    ok &= check("printf", strcmp(line, "-42 42 beef str c  3.14 1234567890123") == 0);

    /* The heap: a big block and a small one, written to their last byte,
     * freed and taken again. */
    char *big = malloc(100000);
    char *small = malloc(33);
    if (big && small) {
        memset(big, 0x5a, 100000);
        strcpy(small, "heap");
    }
    ok &= check("malloc", big && small && big[99999] == 0x5a && strcmp(small, "heap") == 0);
    free(big);
    char *again = malloc(50000);
    ok &= check("free and malloc again", again != NULL);
    free(small);
    free(again);

    /* Software floating point (the PXA270 has no FPU) and the maths
     * library; volatile so the compiler cannot work the answers out. */
    volatile float two = 2.0f;
    volatile double half = 0.5, one = 1.0, ten = 10.0, low = -1.5;
    ok &= check("sqrtf", fabsf(sqrtf(two) - 1.41421356f) < 1e-6f);
    ok &= check("sin", fabs(sin(half) - 0.479425538604203) < 1e-12);
    ok &= check("atan2", fabs(atan2(one, one) * 4 - 3.14159265358979) < 1e-12);
    ok &= check("pow and floor", pow(two, ten) == 1024.0 && floor(low) == -2.0);
    volatile float fa = 1.5f, fb = 2.25f;
    ok &= check("float multiply and compare", fa * fb == 3.375f && fa < fb);
    ok &= check("float to int", (int)(fa * 100.0f) == 150);

    /* 64-bit division (a library call on this CPU). */
    volatile int64_t wide = 1000000007LL * 1000;
    volatile int thousand = 1000;
    ok &= check("64-bit divide", wide / thousand == 1000000007LL && wide % thousand == 0);

    int v[5] = {5, 3, 1, 4, 2};
    qsort(v, 5, sizeof v[0], by_value);
    ok &= check("qsort", v[0] == 1 && v[1] == 2 && v[2] == 3 && v[3] == 4 && v[4] == 5);

    int n = 0;
    float f = 0;
    ok &= check("sscanf", sscanf("17 2.25", "%d %f", &n, &f) == 2 && n == 17 && f == 2.25f);
    ok &= check("strtod", strtod("2.5e3", NULL) == 2500.0);

    /* A file that is not there is refused, not a crash. */
    ok &= check("fopen of nothing is refused", fopen("/nothing", "rb") == NULL);
    ok &= files();

    puts(ok ? "RETROPANE-C-HELLO-OK" : "RETROPANE-C-HELLO-FAILED");
    return ok ? 0 : 1;
}
