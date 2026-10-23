/*
 * rp-wmbench: the stock Windows Mobile baseline (ROADMAP Phase 0).
 *
 * A small Windows Mobile program, run by hand on the owner's X51v under
 * its own Windows Mobile, that measures what RetroPane's benchmark suite
 * measures (same row names where they match: docs/perf.md), plus Windows
 * Mobile's own drawing and card speeds, and appends the results to
 * rp-wm-baseline.txt next to itself (put it in the SD card's root and the
 * log lands there). RetroPane's numbers are then compared against these.
 *
 * Written for RetroPane (GPL-2.0-or-later); public Win32/CE API only,
 * built with cegcc (arm-mingw32ce). No Microsoft code.
 *
 * Rows: "BENCH <name> <value> <unit>". Times are per operation.
 *
 * v2 (2026-10-13): every measured result feeds a sink (v1's memcpy, memset
 * and SHA-256 rows were optimised away); the CPU clock is measured after a
 * 2 s warm-up (Windows Mobile raises it only under load); drawing goes to
 * the program's own full-screen window (v1's desktop drawing was clipped);
 * card reads come after a second file pushes the first out of the cache;
 * the backlight is held on and the idle timer reset, so the screen stays
 * lit (and the device awake) until 'Done'.
 *
 * v3: the drawing rows wait for the 2700G's 2D engine to finish (a pixel
 * read back), with per-operation rows and a coloured fill beside them.
 *
 * v4 (2026-10-23): started with "suite" on its command line (by
 * rp-wmprobe, the probe and baseline suite), it asks nothing and shows no
 * message, and tests only the card it's on, inside its own folder (the
 * owner's shared-card rule).
 */

#ifndef _WIN32_WCE
#define _WIN32_WCE 0x0501 /* CE 5.01 and later (the X51v's WM 6.1 is CE 5.2): pm.h needs it */
#endif
#include <windows.h>
#include <pm.h>
#include <string.h>
#include <stdio.h>

static LARGE_INTEGER freq;
static HANDLE logf_ = INVALID_HANDLE_VALUE;
static HWND win;
static HDC win_dc;
static int scr_w, scr_h;
/* Every measured loop's result goes here, so the compiler can't drop the
 * work (v1's memcpy/memset/SHA rows measured nothing). */
volatile unsigned long sink;

/* Shows what is being measured, on the program's own full-screen window. */
static void status(const WCHAR *what)
{
    RECT r = {0, 0, scr_w, scr_h};
    if (!win_dc)
        return;
    FillRect(win_dc, &r, (HBRUSH)GetStockObject(WHITE_BRUSH));
    ExtTextOutW(win_dc, 20, 40, 0, NULL, L"RetroPane baseline: please wait", 31, NULL);
    ExtTextOutW(win_dc, 20, 80, 0, NULL, what, (UINT)wcslen(what), NULL);
}

static unsigned long long now_ticks(void)
{
    LARGE_INTEGER c;
    QueryPerformanceCounter(&c);
    return (unsigned long long)c.QuadPart;
}

/* Nanoseconds between two counter readings, over `ops` operations. */
static unsigned long ns_per(unsigned long long t0, unsigned long long t1, unsigned long ops)
{
    unsigned long long ns = (t1 - t0) * 1000000000ULL / (unsigned long long)freq.QuadPart;
    return (unsigned long)(ns / (ops ? ops : 1));
}

static void out(const char *fmt, ...)
{
    char line[256];
    DWORD n;
    va_list ap;
    va_start(ap, fmt);
    _vsnprintf(line, sizeof line - 2, fmt, ap);
    va_end(ap);
    line[sizeof line - 3] = 0;
    strcat(line, "\r\n");
    if (logf_ != INVALID_HANDLE_VALUE)
        WriteFile(logf_, line, strlen(line), &n, NULL);
}

/* ---- CPU ------------------------------------------------------------ */

/* 100 dependent adds per pass: one cycle each on the XScale, plus a few
 * for the loop, so cycles / time estimates the clock Windows Mobile runs
 * the core at. */
static unsigned long clock_once(void)
{
    unsigned long passes = 200000, x = 0, i;
    unsigned long long t0 = now_ticks(), t1, ns;
    for (i = 0; i < passes; i++) {
#define A10 "add %0, %0, #1\n add %0, %0, #1\n add %0, %0, #1\n add %0, %0, #1\n add %0, %0, #1\n" \
            "add %0, %0, #1\n add %0, %0, #1\n add %0, %0, #1\n add %0, %0, #1\n add %0, %0, #1\n"
        __asm__ volatile(A10 A10 A10 A10 A10 A10 A10 A10 A10 A10 : "+r"(x));
    }
    t1 = now_ticks();
    sink += x;
    ns = (t1 - t0) * 1000000000ULL / (unsigned long long)freq.QuadPart;
    return (unsigned long)((unsigned long long)passes * 103ULL * 1000ULL / (ns ? ns : 1));
}

/* Windows Mobile lowers the clock when idle and raises it only after load
 * persists: busy for 2 s first, then the best and worst of 8 runs. */
static void cpu_clock(void)
{
    unsigned long best = 0, worst = 0xFFFFFFFF, i, first = clock_once();
    DWORD until = GetTickCount() + 2000;
    while ((LONG)(GetTickCount() - until) < 0)
        (void)clock_once();
    for (i = 0; i < 8; i++) {
        unsigned long m = clock_once();
        best = m > best ? m : best;
        worst = m < worst ? m : worst;
    }
    out("BENCH cpu_mhz_estimate %lu MHz", best);
    out("INFO cpu_mhz_idle_start %lu, after warm-up %lu..%lu", first, worst, best);
}

static void memory(void)
{
    unsigned char *a = (unsigned char *)malloc(65536 + 32), *b = (unsigned char *)malloc(65536 + 32);
    unsigned long i, rounds = 300;
    unsigned long long t0, t1;
    if (!a || !b) {
        out("# memory: no 128 KB");
        return;
    }
    memset(a, 1, 65536);
    t0 = now_ticks();
    for (i = 0; i < rounds; i++) {
        a[i & 0xFFFF] = (unsigned char)i;
        memcpy(b, a, 65536);
        sink += b[(i * 7) & 0xFFFF];
    }
    t1 = now_ticks();
    out("BENCH memcpy_64k %lu ns", ns_per(t0, t1, rounds));
    t0 = now_ticks();
    for (i = 0; i < rounds; i++) {
        memset(b, (int)i, 65536);
        sink += b[(i * 13) & 0xFFFF];
    }
    t1 = now_ticks();
    out("BENCH memset_64k %lu ns", ns_per(t0, t1, rounds));
    free(a);
    free(b);
}

/* Pointer chasing over 256 KB (past the 32 KB D-cache): the miss latency
 * of SDRAM, one hop per cache line, in large jumps a prefetcher can't
 * follow (as RetroPane's suite does). */
static void chase(void)
{
    unsigned long lines = 8192, step = 4099, i, hops = 400000;
    void **buf = (void **)malloc(lines * 32 + 32);
    void **p;
    unsigned long long t0, t1;
    if (!buf) {
        out("# chase: no 256 KB");
        return;
    }
    for (i = 0; i < lines; i++)
        buf[i * 8] = &buf[((i + step) % lines) * 8];
    p = (void **)buf[0];
    t0 = now_ticks();
    for (i = 0; i < hops; i++)
        p = (void **)*p;
    t1 = now_ticks();
    out("BENCH chase_sdram_256k %lu ns", ns_per(t0, t1, hops));
    sink += (unsigned long)p;
    free(buf);
}

static void timer_read(void)
{
    unsigned long i, n = 20000;
    unsigned long long t0 = now_ticks(), t1;
    for (i = 0; i < n; i++)
        sink += (unsigned long)now_ticks();
    t1 = now_ticks();
    out("BENCH timer_read %lu ns", ns_per(t0, t1, n));
    out("INFO performance_counter_hz %lu", (unsigned long)freq.QuadPart);
}

/* ---- Threads -------------------------------------------------------- */

static HANDLE ping, pong;
static volatile LONG rounds_left;

static DWORD WINAPI ponger(LPVOID unused)
{
    (void)unused;
    for (;;) {
        WaitForSingleObject(ping, INFINITE);
        if (rounds_left <= 0) {
            SetEvent(pong);
            return 0;
        }
        SetEvent(pong);
    }
}

static void threads(void)
{
    unsigned long n = 5000, i, worst = 0;
    unsigned long long t0, t1;
    HANDLE th;
    ping = CreateEvent(NULL, FALSE, FALSE, NULL);
    pong = CreateEvent(NULL, FALSE, FALSE, NULL);
    rounds_left = (LONG)n;
    th = CreateThread(NULL, 0, ponger, NULL, 0, NULL);
    t0 = now_ticks();
    for (i = 0; i < n; i++) {
        rounds_left--;
        SetEvent(ping);
        WaitForSingleObject(pong, INFINITE);
    }
    t1 = now_ticks();
    rounds_left = 0;
    SetEvent(ping);
    WaitForSingleObject(th, 2000);
    CloseHandle(th);
    /* Two switches per round trip. */
    out("BENCH thread_switch %lu ns", ns_per(t0, t1, 2 * n));
    for (i = 0; i < 20; i++) {
        unsigned long long a = now_ticks(), b;
        unsigned long late_us;
        Sleep(10);
        b = now_ticks();
        late_us = ns_per(a, b, 1) / 1000;
        late_us = late_us > 10000 ? late_us - 10000 : 0;
        if (late_us > worst)
            worst = late_us;
    }
    out("BENCH sleep_lateness_max %lu us", worst);
    CloseHandle(ping);
    CloseHandle(pong);
}

/* ---- SHA-256 (as RetroPane's suite hashes 4 KB) --------------------- */

static const unsigned long K[64] = {
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2};

#define ROR(x, n) (((x) >> (n)) | ((x) << (32 - (n))))

static void sha_block(unsigned long h[8], const unsigned char *p)
{
    unsigned long w[64], a, b, c, d, e, f, g, hh, t1, t2;
    int i;
    for (i = 0; i < 16; i++)
        w[i] = (unsigned long)p[4 * i] << 24 | (unsigned long)p[4 * i + 1] << 16 |
               (unsigned long)p[4 * i + 2] << 8 | p[4 * i + 3];
    for (i = 16; i < 64; i++) {
        unsigned long s0 = ROR(w[i - 15], 7) ^ ROR(w[i - 15], 18) ^ (w[i - 15] >> 3);
        unsigned long s1 = ROR(w[i - 2], 17) ^ ROR(w[i - 2], 19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16] + s0 + w[i - 7] + s1;
    }
    a = h[0]; b = h[1]; c = h[2]; d = h[3]; e = h[4]; f = h[5]; g = h[6]; hh = h[7];
    for (i = 0; i < 64; i++) {
        t1 = hh + (ROR(e, 6) ^ ROR(e, 11) ^ ROR(e, 25)) + ((e & f) ^ (~e & g)) + K[i] + w[i];
        t2 = (ROR(a, 2) ^ ROR(a, 13) ^ ROR(a, 22)) + ((a & b) ^ (a & c) ^ (b & c));
        hh = g; g = f; f = e; e = d + t1; d = c; c = b; b = a; a = t1 + t2;
    }
    h[0] += a; h[1] += b; h[2] += c; h[3] += d; h[4] += e; h[5] += f; h[6] += g; h[7] += hh;
}

static void sha(void)
{
    static unsigned char data[4096];
    unsigned long h[8], i, j, n = 200;
    unsigned long long t0, t1;
    for (i = 0; i < sizeof data; i++)
        data[i] = (unsigned char)i;
    t0 = now_ticks();
    for (j = 0; j < n; j++) {
        h[0] = 0x6a09e667; h[1] = 0xbb67ae85; h[2] = 0x3c6ef372; h[3] = 0xa54ff53a;
        h[4] = 0x510e527f; h[5] = 0x9b05688c; h[6] = 0x1f83d9ab; h[7] = 0x5be0cd19;
        data[j & 4095] ^= (unsigned char)j;
        for (i = 0; i < sizeof data; i += 64)
            sha_block(h, data + i);
        sink += h[0];
    }
    t1 = now_ticks();
    out("BENCH sha256_4k %lu ns", ns_per(t0, t1, n));
}

/* ---- The screen ----------------------------------------------------- */

#define GETRAWFRAMEBUFFER 0x00020001
typedef struct {
    WORD wFormat, wBPP;
    VOID *pFramePointer;
    int cxStride, cyStride, cxPixels, cyPixels;
} RawFrameBufferInfo;

static void screen(void)
{
    HDC dc = win_dc;
    RawFrameBufferInfo fb;
    unsigned long i, n = 20;
    unsigned long long t0, t1;
    int w = scr_w, hgt = scr_h;
    out("INFO screen %dx%d %d bpp", w, hgt, GetDeviceCaps(dc, BITSPIXEL));

    /* Windows Mobile's own drawing. v2 timed ~0.1 ms for a full-screen
     * fill: far beyond what the CPU writes (~19 ms), so the 2700G's 2D
     * engine does it, possibly queued and unfinished when the call returns.
     * Reading a pixel back waits for the engine: the throughput rows read
     * one after the loop, the _each rows after every operation. */
    t0 = now_ticks();
    for (i = 0; i < n; i++)
        PatBlt(dc, 0, 0, w, hgt, (i & 1) ? BLACKNESS : WHITENESS);
    sink += GetPixel(dc, w - 1, hgt - 1);
    t1 = now_ticks();
    out("BENCH gdi_fill_screen %lu ns", ns_per(t0, t1, n));
    t0 = now_ticks();
    for (i = 0; i < n; i++) {
        PatBlt(dc, 0, 0, w, hgt, (i & 1) ? BLACKNESS : WHITENESS);
        sink += GetPixel(dc, w - 1, hgt - 1);
    }
    t1 = now_ticks();
    out("BENCH gdi_fill_screen_each %lu ns", ns_per(t0, t1, n));
    {
        /* A coloured brush: not a special case the driver might shortcut. */
        HBRUSH b = CreateSolidBrush(RGB(0x33, 0x99, 0xCC));
        HGDIOBJ ob = SelectObject(dc, b);
        t0 = now_ticks();
        for (i = 0; i < n; i++)
            PatBlt(dc, 0, 0, w, hgt, PATCOPY);
        sink += GetPixel(dc, w - 1, hgt - 1);
        t1 = now_ticks();
        out("BENCH gdi_fill_colour_screen %lu ns", ns_per(t0, t1, n));
        SelectObject(dc, ob);
        DeleteObject(b);
    }
    {
        HDC mem = CreateCompatibleDC(dc);
        HBITMAP bm = CreateCompatibleBitmap(dc, w, hgt);
        HGDIOBJ old = SelectObject(mem, bm);
        PatBlt(mem, 0, 0, w, hgt, WHITENESS);
        t0 = now_ticks();
        for (i = 0; i < n; i++)
            BitBlt(dc, 0, 0, w, hgt, mem, 0, 0, SRCCOPY);
        sink += GetPixel(dc, w - 1, hgt - 1);
        t1 = now_ticks();
        out("BENCH gdi_bitblt_screen %lu ns", ns_per(t0, t1, n));
        t0 = now_ticks();
        for (i = 0; i < n; i++) {
            BitBlt(dc, 0, 0, w, hgt, mem, 0, 0, SRCCOPY);
            sink += GetPixel(dc, w - 1, hgt - 1);
        }
        t1 = now_ticks();
        out("BENCH gdi_bitblt_screen_each %lu ns", ns_per(t0, t1, n));
        SelectObject(mem, old);
        DeleteObject(bm);
        DeleteDC(mem);
    }
    {
        static const WCHAR text[] = L"The quick brown fox jumps over the lazy dog 0123456789";
        RECT r = {0, 0, w, 40};
        unsigned long m = 300;
        t0 = now_ticks();
        for (i = 0; i < m; i++)
            ExtTextOutW(dc, 4, 4, ETO_OPAQUE, &r, text, (UINT)wcslen(text), NULL);
        sink += GetPixel(dc, w - 1, 39);
        t1 = now_ticks();
        out("BENCH gdi_text_line %lu ns", ns_per(t0, t1, m));
    }

    /* Straight into the framebuffer (RetroPane's fb_solid_fill_vga). */
    if (ExtEscape(dc, GETRAWFRAMEBUFFER, 0, NULL, sizeof fb, (char *)&fb) > 0 &&
        fb.pFramePointer && fb.wBPP == 16) {
        out("INFO raw_framebuffer %dx%d stride %d/%d", fb.cxPixels, fb.cyPixels, fb.cxStride,
            fb.cyStride);
        /* Word stores along each row (as RetroPane's own fill), only when
         * pixels are contiguous; the bus to the 2700G as WM set it up. */
        if (fb.cxStride == 2) {
            t0 = now_ticks();
            for (i = 0; i < n; i++) {
                unsigned long v = (i & 1) ? 0x001F001FUL : 0xF800F800UL;
                int y, x;
                for (y = 0; y < fb.cyPixels; y++) {
                    unsigned long *row =
                        (unsigned long *)((char *)fb.pFramePointer + y * fb.cyStride);
                    for (x = 0; x < fb.cxPixels / 2; x++)
                        row[x] = v;
                }
            }
            t1 = now_ticks();
            out("BENCH fb_solid_fill_vga %lu ns", ns_per(t0, t1, n));
        } else {
            out("# raw framebuffer pixels not contiguous (stride %d): fill skipped", fb.cxStride);
        }
    } else {
        out("# no raw framebuffer access (GETRAWFRAMEBUFFER)");
    }
}

/* ---- Cards ---------------------------------------------------------- */

static unsigned long kbps(unsigned long bytes, unsigned long long t0, unsigned long long t1)
{
    return (unsigned long)((unsigned long long)bytes * (unsigned long long)freq.QuadPart /
                           1024ULL / (t1 - t0 ? t1 - t0 : 1));
}

static int write_file(const WCHAR *path, unsigned long total, unsigned char *chunk,
                      unsigned long size, unsigned long *speed)
{
    unsigned long long t0, t1;
    unsigned long done;
    DWORD n;
    HANDLE f = CreateFileW(path, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, FILE_FLAG_WRITE_THROUGH,
                           NULL);
    if (f == INVALID_HANDLE_VALUE)
        return 0;
    t0 = now_ticks();
    for (done = 0; done < total; done += size) {
        chunk[0] = (unsigned char)done;
        WriteFile(f, chunk, size, &n, NULL);
    }
    FlushFileBuffers(f);
    t1 = now_ticks();
    CloseHandle(f);
    *speed = kbps(total, t0, t1);
    return 1;
}

static void card(const WCHAR *dir, const char *name)
{
    WCHAR path[MAX_PATH], other[MAX_PATH];
    static unsigned char chunk[65536];
    unsigned long total = 4UL << 20, done, i, speed, speed2;
    unsigned long long t0, t1;
    DWORD n;
    HANDLE f;
    for (i = 0; i < sizeof chunk; i++)
        chunk[i] = (unsigned char)(i * 7);
    wsprintfW(path, L"%s\\rp-wmbench-a.tmp", dir);
    wsprintfW(other, L"%s\\rp-wmbench-b.tmp", dir);
    if (!write_file(path, total, chunk, sizeof chunk, &speed)) {
        out("# %s: can't create a test file", name);
        return;
    }
    out("BENCH %s_write_4m %lu KB/s", name, speed);
    /* A second 4 MB file pushes the first out of Windows Mobile's cache. */
    if (write_file(other, total, chunk, sizeof chunk, &speed2))
        out("INFO %s_write_4m_again %lu KB/s", name, speed2);
    f = CreateFileW(path, GENERIC_READ, 0, NULL, OPEN_EXISTING, FILE_FLAG_NO_BUFFERING, NULL);
    if (f != INVALID_HANDLE_VALUE) {
        t0 = now_ticks();
        for (done = 0; done < total; done += sizeof chunk)
            ReadFile(f, chunk, sizeof chunk, &n, NULL);
        t1 = now_ticks();
        CloseHandle(f);
        out("BENCH %s_read_4m %lu KB/s", name, kbps(total, t0, t1));
    }
    DeleteFileW(path);
    DeleteFileW(other);
}

/* ---- Staying awake --------------------------------------------------- */

/* Windows Mobile turns the backlight off (and on battery, suspends) when
 * nobody touches the screen; v1's run went dark before its 'Done' box.
 * A sleeping helper thread resets the idle timer every 5 s: it wakes 24
 * times in two minutes, which is noise against every row. */
static volatile int awake_run = 1;

static DWORD WINAPI keep_awake(LPVOID arg)
{
    (void)arg;
    while (awake_run) {
        SystemIdleTimerReset();
        Sleep(5000);
    }
    return 0;
}

/* ---- The run -------------------------------------------------------- */

int WINAPI WinMain(HINSTANCE inst, HINSTANCE prev, LPWSTR cmd, int show)
{
    WCHAR exe[MAX_PATH], dir[MAX_PATH], logpath[MAX_PATH], *slash;
    SYSTEMTIME st;
    MEMORYSTATUS ms;
    OSVERSIONINFO ver;
    WIN32_FIND_DATAW fd;
    HANDLE find, light, awake;
    int suite = cmd && wcsstr(cmd, L"suite") != NULL;
    (void)prev; (void)show;

    QueryPerformanceFrequency(&freq);
    GetModuleFileNameW(inst, exe, MAX_PATH);
    wcscpy(dir, exe);
    slash = wcsrchr(dir, L'\\');
    if (slash)
        *slash = 0;
    wsprintfW(logpath, L"%s\\rp-wm-baseline.txt", dir);
    logf_ = CreateFileW(logpath, GENERIC_WRITE, 0, NULL, OPEN_ALWAYS, 0, NULL);
    if (logf_ == INVALID_HANDLE_VALUE) {
        MessageBoxW(NULL, L"Can't write the log next to the program.", L"rp-wmbench", MB_OK);
        return 1;
    }
    SetFilePointer(logf_, 0, NULL, FILE_END);

    GetLocalTime(&st);
    ver.dwOSVersionInfoSize = sizeof ver;
    GetVersionEx(&ver);
    ms.dwLength = sizeof ms;
    GlobalMemoryStatus(&ms);
    out("# rp-wmbench run %04d-%02d-%02d %02d:%02d:%02d", st.wYear, st.wMonth, st.wDay, st.wHour,
        st.wMinute, st.wSecond);
    out("INFO os %lu.%lu build %lu", ver.dwMajorVersion, ver.dwMinorVersion, ver.dwBuildNumber);
    out("INFO memory_total_kb %lu", (unsigned long)(ms.dwTotalPhys / 1024));
    out("INFO memory_free_kb %lu", (unsigned long)(ms.dwAvailPhys / 1024));
    out("INFO uptime_ms %lu", (unsigned long)GetTickCount());

    out("INFO mode %s", suite ? "suite (run by rp-wmprobe)" : "alone");
    if (!suite)
        MessageBoxW(NULL,
                    L"RetroPane baseline: about two minutes. The screen will flash; don't touch "
                    L"the device until the 'Done' message.",
                    L"rp-wmbench", MB_OK);

    /* Backlight held fully on for the run (as video players do); the idle
     * timer reset covers devices whose backlight driver has another name. */
    light = SetPowerRequirement(L"BKL1:", D0, POWER_NAME, NULL, 0);
    awake = CreateThread(NULL, 0, keep_awake, NULL, 0, NULL);
    out("INFO backlight_held %s", light ? "yes" : "no (idle timer only)");

    scr_w = GetSystemMetrics(SM_CXSCREEN);
    scr_h = GetSystemMetrics(SM_CYSCREEN);
    win = CreateWindowExW(WS_EX_TOPMOST, L"STATIC", L"rp-wmbench", WS_POPUP | WS_VISIBLE, 0, 0,
                          scr_w, scr_h, NULL, NULL, inst, NULL);
    if (win) {
        SetForegroundWindow(win);
        UpdateWindow(win);
        win_dc = GetDC(win);
    }
    status(L"CPU clock");
    cpu_clock();
    status(L"Timer, memory, hashing");
    timer_read();
    memory();
    chase();
    sha();
    status(L"Threads");
    threads();
    if (win_dc)
        screen();
    else
        out("# no window: drawing rows skipped");
    status(L"This card (writes 8 MB, then removes it)");
    card(dir, "card_here");
    /* Other storage cards (the CF card): root folders marked temporary. */
    find = suite ? INVALID_HANDLE_VALUE : FindFirstFileW(L"\\*", &fd);
    if (find != INVALID_HANDLE_VALUE) {
        do {
            if ((fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) &&
                (fd.dwFileAttributes & FILE_ATTRIBUTE_TEMPORARY)) {
                WCHAR other[MAX_PATH];
                char label[64];
                int k;
                wsprintfW(other, L"\\%s", fd.cFileName);
                status(L"The other card (writes 8 MB, then removes it)");
                if (_wcsicmp(other, dir) == 0)
                    continue;
                for (k = 0; fd.cFileName[k] && k < 40; k++)
                    label[k] = (char)(fd.cFileName[k] == L' ' ? '_' : fd.cFileName[k]);
                label[k] = 0;
                card(other, label);
            }
        } while (FindNextFileW(find, &fd));
        FindClose(find);
    }
    out("INFO sink %lu", (unsigned long)sink);
    out("# done");
    CloseHandle(logf_);
    status(L"Done");
    if (!suite)
        MessageBoxW(win, L"Done: rp-wm-baseline.txt is next to the program.", L"rp-wmbench",
                    MB_OK | MB_TOPMOST);
    if (win) {
        ReleaseDC(win, win_dc);
        DestroyWindow(win);
    }
    awake_run = 0;
    if (awake) {
        WaitForSingleObject(awake, 6000);
        CloseHandle(awake);
    }
    if (light)
        ReleasePowerRequirement(light);
    return 0;
}
