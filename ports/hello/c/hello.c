/*
 * The first C program on RetroPane (docs/design/half-life-readiness.md,
 * lines 2 and 3): proves the C compiler, picolibc and the C boundary by
 * using what a game port leans on (printf with floats, the heap, software
 * floating point, the maths library, 64-bit division, sorting, parsing)
 * and the port layer (the clock, the card's files) and checking every
 * answer. `cargo xtask emu-chello` runs it and looks
 * for the last line.
 */
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <time.h>
#include <unistd.h>

#include <retropane.h>

/* The C++ checks (cxx.cpp). */
int cxx_checks(void);

/* One check's line; also called from cxx.cpp. */
int check(const char *what, int ok);

int check(const char *what, int ok)
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

    /* What a game engine's own file layer asks beyond stdio: folders made
     * and listed, a log appended to, a file renamed into a folder, what a
     * path is, two handles on one file. */
    static const char dir[] = "rp-chello/dir", log[] = "rp-chello/log.txt", moved[] = "rp-chello/dir/moved.txt";
    struct stat st;
    ok &= check("mkdir, and again", mkdir(dir, 0777) == 0 && mkdir(dir, 0777) == 0);
    f = fopen(log, "w");
    if (f) {
        fputs("one\n", f);
        fclose(f);
    }
    f = fopen(log, "a");
    ok &= check("fopen to append", f != NULL);
    if (f) {
        fputs("two\n", f);
        ok &= check("fclose after appending", fclose(f) == 0);
    }
    errno = 0;
    ok &= check("stat of a file, a folder and nothing",
                stat(log, &st) == 0 && S_ISREG(st.st_mode) && st.st_size == 8 && stat(dir, &st) == 0 &&
                    S_ISDIR(st.st_mode) && stat("rp-chello/none", &st) == -1 && errno == ENOENT);
    ok &= check("rename into a folder", rename(log, moved) == 0 && stat(log, &st) == -1 && stat(moved, &st) == 0);
    DIR *d = opendir("rp-chello");
    int entries = 0, found = 0;
    if (d) {
        for (struct dirent *e; (e = readdir(d)) != NULL; entries++)
            found |= strcasecmp(e->d_name, "dir") == 0 && e->d_type == DT_DIR;
    }
    ok &= check("opendir, readdir, closedir", d != NULL && entries == 1 && found && closedir(d) == 0);
    int a = open(moved, O_RDONLY), b = dup(a);
    char one[4] = "", two[4] = "";
    ok &= check("dup: two places in one file",
                a >= 0 && b >= 0 && a != b && lseek(b, 4, SEEK_SET) == 4 && read(a, one, 3) == 3 &&
                    read(b, two, 3) == 3 && strcmp(one, "one") == 0 && strcmp(two, "two") == 0);
    ok &= check("fsync of a file read", fsync(a) == 0);
    close(a);
    close(b);
    char cwd[8] = "";
    ok &= check("getcwd", getcwd(cwd, sizeof cwd) == cwd && strcmp(cwd, "/") == 0);
    ok &= check("tidied", remove(moved) == 0 && remove(dir) == 0);

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

/* The clock (<retropane.h>): it moves forward and never back, a wait of
 * five milliseconds is five milliseconds by it, and the C library's own
 * time calls read the same clock. */
static int clock_checks(void)
{
    int ok = 1;
    uint64_t start = rp_time_us(), prev = start, now = start;
    int back = 0;
    /* Bounded: a clock that stood still ends the loop, as a failure. */
    for (volatile uint32_t spins = 0; spins < 200000000u && now - start < 5000; spins++) {
        now = rp_time_us();
        back |= now < prev;
        prev = now;
    }
    ok &= check("the clock moves forward", now - start >= 5000);
    ok &= check("the clock never goes back", !back);
    /* The step that crossed 5 ms was one read's worth, not a jump. */
    ok &= check("the clock counts in microseconds", now - start < 6000);

    struct timeval tv = {-1, -1};
    uint64_t before = rp_time_us();
    int got = gettimeofday(&tv, NULL);
    uint64_t after = rp_time_us();
    uint64_t at = (uint64_t)tv.tv_sec * 1000000u + (uint64_t)tv.tv_usec;
    ok &= check("gettimeofday is the same clock",
                got == 0 && tv.tv_usec >= 0 && tv.tv_usec < 1000000 && at >= before && at <= after);
    time_t t = time(NULL);
    ok &= check("time", t >= tv.tv_sec && t <= tv.tv_sec + 1);
    puts(ok ? "  clock: ok" : "  clock: FAILED");
    return ok;
}

/* Sound (<retropane.h>): a second of an 880 Hz tone at 22.05 kHz mono,
 * written as a game writes its sound: every 20 ms what has played is
 * topped up to a tenth of a second queued. The checks here are the calls'
 * own answers; `cargo xtask emu-chello` checks the emulator's recording
 * for the tone itself. On a system without sound the open says so and the
 * checks are skipped. */
static int sound_checks(void)
{
    enum { RATE = 22050, TOTAL = RATE, LEAD = RATE / 10 };
    static int16_t tone[TOTAL];
    int ok = 1;

    errno = 0;
    if (rp_snd_open(RATE, 1) != 0) {
        if (errno == ENOSYS) {
            puts("  sound: skipped (none on this system)");
            return 1;
        }
        return check("rp_snd_open", 0);
    }
    errno = 0;
    ok &= check("a second open is refused", rp_snd_open(RATE, 1) == -1 && errno == EBUSY);
    ok &= check("room for a tenth of a second", rp_snd_room() >= LEAD);
    ok &= check("nothing played yet", rp_snd_played() == 0);

    for (int i = 0; i < TOTAL; i++)
        tone[i] = (int16_t)(12000.0f * sinf(6.2831853f * 880.0f * (float)i / RATE));

    int written = 0, short_write = 0, back = 0, dry = 0;
    int64_t played = 0;
    uint64_t start = rp_time_us(), next = start;
    /* Bounded by the clock: three seconds for one second of sound. */
    while (played < TOTAL - 64 && rp_time_us() - start < 3000000) {
        if (rp_time_us() < next)
            continue;
        next += 20000;
        int64_t now = rp_snd_played();
        back |= now < played;
        played = now;
        int want = (int)(played + LEAD) - written;
        if (want > TOTAL - written)
            want = TOTAL - written;
        if (want > 0) {
            /* Nothing queued while more is still to come: it ran dry. */
            dry += written > 0 && played >= written - 64;
            int took = rp_snd_write(tone + written, (size_t)want);
            short_write |= took != want;
            if (took > 0)
                written += took;
        }
    }
    ok &= check("every sample was taken", written == TOTAL && !short_write);
    ok &= check("it all played", played >= TOTAL - 64 && played <= TOTAL);
    ok &= check("the played count never goes back", !back);
    ok &= check("the queue never ran dry", dry == 0);
    uint64_t took_us = rp_time_us() - start;
    /* A second of sound takes a second (the link's clock is within 1%). */
    ok &= check("a second of sound took a second", took_us > 950000 && took_us < 1250000);
    ok &= check("rp_snd_close", rp_snd_close() == 0);
    errno = 0;
    ok &= check("closed: the calls say so",
                rp_snd_close() == -1 && errno == EBADF && rp_snd_room() == -1 && rp_snd_played() == -1);
    errno = 0;
    ok &= check("a rate that is not taken is refused", rp_snd_open(1000, 1) == -1 && errno == EINVAL);
    puts(ok ? "  sound: ok" : "  sound: FAILED");
    return ok;
}

/* Input (<retropane.h>): the screen held in landscape with the device's
 * top at the left. Every event that arrives is printed and checked for
 * sense (a known key, the stylus inside 640 x 480, times that never go
 * back); `cargo xtask emu-chello` presses buttons and taps, and looks for
 * the lines. The D-pad's centre ends it; so do ten seconds with nothing
 * pressed (a run with nobody there), which is no failure. */
static int input_checks(void)
{
    int ok = 1, seen = 0, done = 0;
    struct rp_event ev;

    errno = 0;
    if (rp_input_open(RP_TURN_LEFT) != 0) {
        if (errno == ENOSYS) {
            puts("  input: skipped (none on this system)");
            return 1;
        }
        return check("rp_input_open", 0);
    }
    errno = 0;
    ok &= check("a second open is refused", rp_input_open(RP_TURN_NONE) == -1 && errno == EBUSY);
    puts("  input: press and tap (the D-pad's centre ends it)");
    fflush(stdout);

    uint64_t last = rp_time_us(), prev = 0;
    while (!done && rp_time_us() - last < 10000000) {
        int got = rp_input_poll(&ev);
        if (got == 0)
            continue;
        if (got != 1) {
            ok = check("rp_input_poll", 0);
            break;
        }
        seen++;
        last = rp_time_us();
        ok &= ev.time_us >= prev && ev.time_us <= last && ev.reserved == 0;
        prev = ev.time_us;
        if (ev.type == RP_EV_KEY) {
            ok &= ev.code >= RP_KEY_UP && ev.code <= RP_KEY_WIFI && ev.down <= 1 && ev.x == 0 && ev.y == 0;
            printf("  KEY %d %d\n", ev.code, ev.down);
            done = ev.code == RP_KEY_ENTER && !ev.down;
        } else if (ev.type == RP_EV_PEN) {
            ok &= ev.code >= RP_PEN_DOWN && ev.code <= RP_PEN_UP && ev.down == (ev.code != RP_PEN_UP);
            ok &= ev.x >= 0 && ev.x < 640 && ev.y >= 0 && ev.y < 480;
            if (ev.code == RP_PEN_UP)
                printf("  PEN up %d %d\n", ev.x, ev.y);
            else
                printf("  TOUCH %d %d\n", ev.x, ev.y);
        } else {
            ok = 0;
        }
        fflush(stdout);
    }
    if (seen == 0) {
        puts("  input: skipped (nothing pressed in 10 s)");
        return ok;
    }
    ok &= check("every event made sense", ok);
    puts(ok ? "  input: ok" : "  input: FAILED");
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
    ok &= cxx_checks();
    ok &= clock_checks();
    ok &= sound_checks();
    ok &= files();
    ok &= input_checks();

    puts(ok ? "RETROPANE-C-HELLO-OK" : "RETROPANE-C-HELLO-FAILED");
    return ok ? 0 : 1;
}
