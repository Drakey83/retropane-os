/*
 * rp-wmgpu: what draws Windows Mobile's screen? (overnight item 69, first step)
 *
 * rp-wmbench found WM's full-screen fills queued and finished later at
 * ~190 MB/s, where the CPU's own stores reach ~41 MB/s (docs/perf.md). So
 * something other than the CPU writes the 2700G's memory: the PXA270's DMA,
 * a 2D engine, or the MBX core. This program watches the hardware while WM
 * draws, and logs what it sees to rp-wm-gpu.txt next to itself. It only
 * reads registers (mapped read-only); no code or data from Windows Mobile
 * is copied (docs/CLEANROOM.md: observations become facts in docs/hw).
 *
 * 1. CPU free: a lowest-priority thread counts while 20 fills drain (the
 *    program sleeps), against the same sleep with nothing drawn. Near 100%:
 *    not the CPU. Near 0%: a CPU thread draws later.
 * 2. DMA: the PXA270's 32 channels sampled while fills drain, against an
 *    idle sample: channels running and their source, target and command.
 * 3. 2700G registers known to exist (v7: the system block's named ones,
 *    and the MBX core's at its own base, 0x0FFE4000; v2-v6 read MBX offsets
 *    on the system block and hung the bus): idle, then how often each
 *    changes while fills drain, then while nothing is drawn.
 *
 * Built by `cargo xtask wmbench` (cegcc in WSL).
 */

#define _WIN32_WCE 0x0501
#include <windows.h>
#include <string.h>
#include <stdio.h>

BOOL VirtualCopy(LPVOID dest, LPVOID src, DWORD size, DWORD protect);

static LARGE_INTEGER freq;
static HANDLE logf_ = INVALID_HANDLE_VALUE;
static HWND win;
static HDC dc;
static int w, h;

static void out(const char *fmt, ...)
{
    char line[256];
    DWORD n;
    va_list ap;
    va_start(ap, fmt);
    n = (DWORD)_vsnprintf(line, sizeof line - 2, fmt, ap);
    va_end(ap);
    if (n > sizeof line - 2)
        n = sizeof line - 2;
    line[n++] = '\r';
    line[n++] = '\n';
    WriteFile(logf_, line, n, &n, NULL);
}

/* Saved after every phase: a hang later still leaves these lines. */
static void flush(void)
{
    FlushFileBuffers(logf_);
}

static unsigned long long now_ticks(void)
{
    LARGE_INTEGER t;
    QueryPerformanceCounter(&t);
    return (unsigned long long)t.QuadPart;
}

static unsigned long us_between(unsigned long long a, unsigned long long b)
{
    return (unsigned long)((b - a) * 1000000ULL / (unsigned long long)freq.QuadPart);
}

static void status(const WCHAR *what)
{
    RECT r = {0, 0, w, h};
    FillRect(dc, &r, (HBRUSH)GetStockObject(WHITE_BRUSH));
    ExtTextOutW(dc, 20, 40, 0, NULL, L"RetroPane GPU probe: please wait", 32, NULL);
    ExtTextOutW(dc, 20, 80, 0, NULL, what, (UINT)wcslen(what), NULL);
}

/* 20 full-screen fills, black and white, issued without waiting. */
static void issue_fills(void)
{
    int i;
    for (i = 0; i < 20; i++)
        PatBlt(dc, 0, 0, w, h, (i & 1) ? BLACKNESS : WHITENESS);
}

/* Physical memory mapped read-only and uncached. */
static volatile unsigned long *map_phys(DWORD phys, DWORD size)
{
    void *v = VirtualAlloc(NULL, size, MEM_RESERVE, PAGE_NOACCESS);
    if (!v)
        return NULL;
    if (!VirtualCopy(v, (void *)(phys >> 8), size, PAGE_READONLY | PAGE_NOCACHE | PAGE_PHYSICAL)) {
        out("# map %08lx: VirtualCopy failed, error %lu", (unsigned long)phys,
            (unsigned long)GetLastError());
        VirtualFree(v, 0, MEM_RELEASE);
        return NULL;
    }
    return (volatile unsigned long *)v;
}

/* ---- 1. CPU free while fills drain --------------------------------- */

static volatile unsigned long counted;
static volatile int counting = 1;

static DWORD WINAPI counter(LPVOID arg)
{
    (void)arg;
    while (counting)
        counted++;
    return 0;
}

static void cpu_free(void)
{
    unsigned long idle[3], busy[3], c0, i;
    unsigned long long t0, t1;
    HANDLE th = CreateThread(NULL, 0, counter, NULL, 0, NULL);
    if (!th) {
        out("# cpu_free: no thread");
        return;
    }
    CeSetThreadPriority(th, 255); /* lowest: runs only when nothing else does */
    Sleep(50);
    for (i = 0; i < 3; i++) {
        c0 = counted;
        Sleep(100);
        idle[i] = counted - c0;
        c0 = counted;
        issue_fills();
        Sleep(100);
        busy[i] = counted - c0;
        t0 = now_ticks();
        (void)GetPixel(dc, w - 1, h - 1);
        t1 = now_ticks();
        out("INFO round %lu: counted idle %lu, while 20 fills drain %lu (%lu%%); "
            "pixel read after the sleep %lu us",
            i, idle[i], busy[i], idle[i] ? busy[i] * 100 / idle[i] : 0, us_between(t0, t1));
    }
    {
        unsigned long a = idle[0] + idle[1] + idle[2], b = busy[0] + busy[1] + busy[2];
        out("BENCH cpu_free_during_fills %lu %%", a ? b * 100 / a : 0);
    }
    counting = 0;
    WaitForSingleObject(th, 1000);
    CloseHandle(th);
}

/* ---- 2. DMA channels while fills drain ------------------------------ */

#define DMA_PHYS 0x40000000UL
#define DCSR_RUN 0x80000000UL
#define DCSR_STOPSTATE 0x00000008UL

static void dma_sample(volatile unsigned long *dma, const char *what, int draw)
{
    unsigned long active[32], src[32], dst[32], cmd[32], samples = 0, ch;
    unsigned long long t0, end;
    memset(active, 0, sizeof active);
    if (draw)
        issue_fills();
    t0 = now_ticks();
    end = t0 + (unsigned long long)freq.QuadPart / 10; /* 100 ms */
    while (now_ticks() < end) {
        for (ch = 0; ch < 32; ch++) {
            unsigned long csr = dma[ch];
            if ((csr & DCSR_RUN) && !(csr & DCSR_STOPSTATE)) {
                if (!active[ch]) {
                    src[ch] = dma[(0x200 + 16 * ch + 4) / 4];
                    dst[ch] = dma[(0x200 + 16 * ch + 8) / 4];
                    cmd[ch] = dma[(0x200 + 16 * ch + 12) / 4];
                }
                active[ch]++;
            }
        }
        samples++;
    }
    out("INFO dma %s: %lu samples in 100 ms", what, samples);
    for (ch = 0; ch < 32; ch++)
        if (active[ch])
            out("BENCH dma_%s ch%lu active %lu/%lu src %08lx dst %08lx cmd %08lx", what, ch,
                active[ch], samples, src[ch], dst[ch], cmd[ch]);
    if (draw)
        (void)GetPixel(dc, w - 1, h - 1);
}

static void dma(void)
{
    volatile unsigned long *regs = map_phys(DMA_PHYS, 0x1000);
    if (!regs)
        return;
    out("INFO dma raw DCSR0-3 %08lx %08lx %08lx %08lx, DINT %08lx", regs[0], regs[1], regs[2],
        regs[3], regs[0xF0 / 4]);
    dma_sample(regs, "idle", 0);
    dma_sample(regs, "fills", 1);
    dma_sample(regs, "idle_again", 0);
}

/* ---- 3. The 2700G's registers, idle and while drawing ----------------- */

/*
 * Only registers Dell's driver itself reads (static analysis, 2026-10-13:
 * the offsets passed to its register-read helper, on the register block
 * at 0x0FFE0000), so reading them is safe: v1 read the whole 128 KB block
 * and froze WM. Names where Linux's mbxfb has one.
 */
#define REGS_PHYS 0x0FFE0000UL
#define REGS_SIZE 0x5000UL
static const struct { unsigned long off; const char *name; } SAFE[] = {
    /* The system block (0x0FFE0000): named by Linux, read fine under WM. */
    {0x0000, "SYSCFG"}, {0x002c, "SYSCLKSRC"}, {0x0040, "PLLSTAT"},
    {0x0044, "VOVRCLK"}, {0x0048, "PIXCLK"}, {0x0050, "M24CLK"},
    {0x0054, "MBXCLK"}, {0x0060, "LCD_CONFIG"}, {0x0204, "PWMCFG"},
    {0x0ff0, "ID"}, {0x100c, "LMPWRSTAT"}, {0x2154, "DSCTRL"},
    {0x2308, "DVLNUM"}, {0x0300, "?0300"}, {0x0304, "?0304"},
    /* The MBX core's block at 0x0FFE4000 (v7, 2026-10-13: v2-v6 read these
     * offsets on the system block, where they hung the bus). Its ID first
     * (read fine under the loader: 0x01020000, 0x00010000), then the four
     * offsets that froze WM, now at the right base. */
    {0x4f00, "MBX f00 ID"}, {0x4f10, "MBX f10 REV"},
    {0x412c, "MBX 12c"}, {0x430c, "MBX 30c"}, {0x485c, "MBX 85c"},
};
#define NSAFE (sizeof SAFE / sizeof SAFE[0])

static void mbx_regs(void)
{
    volatile unsigned long *regs = map_phys(REGS_PHYS, REGS_SIZE);
    unsigned long idle[NSAFE], first[NSAFE], changes[NSAFE], last[NSAFE];
    unsigned long i, k, rounds = 0;
    unsigned long long end;
    if (!regs) {
        out("# regs: no mapping");
        return;
    }
    /* One at a time, the name saved before each read: v2 froze reading
     * them in a loop, so a freeze now names the register. */
    for (k = 0; k < NSAFE; k++) {
        out("# reading %04lx %s", SAFE[k].off, SAFE[k].name);
        flush();
        idle[k] = regs[SAFE[k].off / 4];
        out("BENCH reg_idle %04lx %-10s %08lx", SAFE[k].off, SAFE[k].name, idle[k]);
        flush();
    }
    /* While 20 fills drain: how often each register changes, and to what. */
    for (k = 0; k < NSAFE; k++) {
        first[k] = last[k] = idle[k];
        changes[k] = 0;
    }
    issue_fills();
    end = now_ticks() + (unsigned long long)freq.QuadPart / 10; /* 100 ms */
    while (now_ticks() < end) {
        for (k = 0; k < NSAFE; k++) {
            unsigned long v = regs[SAFE[k].off / 4];
            if (v != last[k]) {
                if (changes[k] == 0)
                    first[k] = v;
                changes[k]++;
                last[k] = v;
            }
        }
        rounds++;
    }
    (void)GetPixel(dc, w - 1, h - 1);
    out("INFO regs while drawing: %lu rounds in 100 ms", rounds);
    for (k = 0; k < NSAFE; k++)
        if (changes[k])
            out("BENCH reg_drawing %04lx %-10s changed %lu times, first to %08lx, last %08lx",
                SAFE[k].off, SAFE[k].name, changes[k], first[k], last[k]);
    /* The same 100 ms with nothing drawn, to tell drawing from background. */
    for (k = 0; k < NSAFE; k++) {
        last[k] = regs[SAFE[k].off / 4];
        changes[k] = 0;
    }
    end = now_ticks() + (unsigned long long)freq.QuadPart / 10;
    while (now_ticks() < end)
        for (k = 0; k < NSAFE; k++) {
            unsigned long v = regs[SAFE[k].off / 4];
            if (v != last[k]) {
                changes[k]++;
                last[k] = v;
            }
        }
    for (k = 0; k < NSAFE; k++)
        if (changes[k])
            out("BENCH reg_idle_changes %04lx %-10s changed %lu times", SAFE[k].off,
                SAFE[k].name, changes[k]);
    (void)i;
}

/* ---- The run -------------------------------------------------------- */

int WINAPI WinMain(HINSTANCE inst, HINSTANCE prev, LPWSTR cmd, int show)
{
    WCHAR dir[MAX_PATH], logpath[MAX_PATH], *slash;
    SYSTEMTIME st;
    (void)prev; (void)cmd; (void)show;

    QueryPerformanceFrequency(&freq);
    GetModuleFileNameW(inst, dir, MAX_PATH);
    slash = wcsrchr(dir, L'\\');
    if (slash)
        *slash = 0;
    wsprintfW(logpath, L"%s\\rp-wm-gpu.txt", dir);
    logf_ = CreateFileW(logpath, GENERIC_WRITE, 0, NULL, OPEN_ALWAYS, 0, NULL);
    if (logf_ == INVALID_HANDLE_VALUE) {
        MessageBoxW(NULL, L"Can't write the log next to the program.", L"rp-wmgpu", MB_OK);
        return 1;
    }
    SetFilePointer(logf_, 0, NULL, FILE_END);
    GetLocalTime(&st);
    out("# rp-wmgpu run %04d-%02d-%02d %02d:%02d:%02d", st.wYear, st.wMonth, st.wDay, st.wHour,
        st.wMinute, st.wSecond);

    MessageBoxW(NULL,
                L"RetroPane GPU probe: about 15 seconds. The screen will flash. "
                L"The last step reads a few of the graphics chip's registers (only ones "
                L"Dell's driver reads itself). If Windows Mobile freezes, soft-reset; "
                L"the results so far are saved.",
                L"rp-wmgpu", MB_OK);

    w = GetSystemMetrics(SM_CXSCREEN);
    h = GetSystemMetrics(SM_CYSCREEN);
    win = CreateWindowExW(WS_EX_TOPMOST, L"STATIC", L"rp-wmgpu", WS_POPUP | WS_VISIBLE, 0, 0, w,
                          h, NULL, NULL, inst, NULL);
    if (!win) {
        out("# no window");
        CloseHandle(logf_);
        return 1;
    }
    SetForegroundWindow(win);
    UpdateWindow(win);
    dc = GetDC(win);
    SystemIdleTimerReset();

    status(L"1/3: is the CPU free while WM draws?");
    cpu_free();
    flush();
    SystemIdleTimerReset();
    status(L"2/3: DMA channels while WM draws");
    dma();
    flush();
    SystemIdleTimerReset();
    status(L"3/3: graphics chip registers");
    out("# mbx_regs: starting");
    flush();
    mbx_regs();
    out("# done");
    CloseHandle(logf_);
    status(L"Done");
    MessageBoxW(win, L"Done: rp-wm-gpu.txt is next to the program.", L"rp-wmgpu",
                MB_OK | MB_TOPMOST);
    ReleaseDC(win, dc);
    DestroyWindow(win);
    return 0;
}
