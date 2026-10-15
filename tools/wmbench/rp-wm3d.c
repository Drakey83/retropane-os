/*
 * rp-wm3d: a real 3D scene, captured (ADR-0024 M3).
 *
 * rp-mbx3d drew flat and shaded triangles on the MBX 3D core under the
 * loader from objects we write ourselves. Textured objects (and the tile
 * accelerator's own output) are written by the core, not by any code we can
 * read, so their format is best learned from a real game: this program
 * runs under Windows Mobile while the owner plays DoomGLES, and copies the
 * 2700G's 16 MiB of local memory to the SD card twice. On the PC the copies
 * give the region arrays, object lists, textured objects, texture words and
 * texel layouts of real frames (docs/hw/2700g.md, "The scene bytes").
 *
 * Observation only (docs/CLEANROOM.md): memory is mapped read-only and
 * uncached; of the registers, only the two Dell's driver itself reads
 * (MBXCLK, the core's status at 0x12c). To get a frame not torn by the
 * game, the game's threads are suspended (the program in front when the
 * copy is due) while a slice is copied, 50 ms after, for the core to finish
 * what it has.
 *
 * Version 6 (2026-10-14): 0x100000-0x2FFFFF, three times, 20 s apart, in
 * one 2 MiB piece if the RAM allows (one frame, not torn): version 5's top
 * 4 MiB was only textures; version 4's low part had a region array at
 * 0x130000, a likely textured object at 0x12A000, and most of its
 * float-like words in 0x140000-0x2FFFFF.
 *
 * Version 5 (2026-10-14): only the top 4 MiB (0xC00000-0xFFFFFF), twice,
 * 30 s apart: version 4 saved 0x000000-0xBFFFFF (textures, Dell's clear
 * scene) before the owner left, ~5 s a 1.5 MiB slice; the game's frame is
 * likely above. The file holds just that range.
 *
 * Version 4 (2026-10-14): version 3's process list came back empty
 * (now without heap lists, errors logged), so the window in front is used
 * too when its title says Doom.
 *
 * Version 3. Version 1 copied into RAM first: with the game
 * running there were no 16 MiB to be had (4 MiB free), so nothing was
 * copied; and it drew on its window when done, and the game froze there
 * (music still playing; a soft reset): nothing is drawn after the start
 * screen now, beeps only. Version 2 wrote to the card while the game was
 * suspended and stopped after 64 KiB: a suspended game thread can hold a
 * lock the card's writes need. Now nothing touches a file while the game is
 * suspended: per slice of up to 3 MiB, suspend, copy to RAM (~0.2 s),
 * resume, then write. Slices are of different frames; the PC compares two
 * captures.
 *
 * Its log is rp-wm3d.txt next to itself; the copies rp-wm3d-1.bin and
 * rp-wm3d-2.bin (16 MiB each). Built by `cargo xtask wmbench`.
 */

#define _WIN32_WCE 0x0501
#include <windows.h>
#include <string.h>
#include <stdio.h>
#include <tlhelp32.h>

BOOL VirtualCopy(LPVOID dest, LPVOID src, DWORD size, DWORD protect);

#define MEM_PHYS 0x0C000000UL
#define MEM_SIZE (16UL << 20)
/* The part copied: where the scene seems to be. */
#define CAP_FROM 0x100000UL
#define CAP_SIZE 0x200000UL
#define CAPTURES 3
#define REGS_PHYS 0x0FFE0000UL
#define REGS_SIZE 0x5000UL
#define MBXCLK 0x0054UL
#define MBX_STATUS 0x412cUL

/* Seconds before each copy: time to start the game and get playing. */
static const DWORD AT_S[CAPTURES] = {60, 20, 20};

static LARGE_INTEGER freq;
static HANDLE logf_ = INVALID_HANDLE_VALUE;
static WCHAR dir[MAX_PATH];
static HWND win;

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

static void say(const WCHAR *line1, const WCHAR *line2)
{
    HDC dc = GetDC(win);
    RECT r;
    GetClientRect(win, &r);
    FillRect(dc, &r, (HBRUSH)GetStockObject(WHITE_BRUSH));
    ExtTextOutW(dc, 10, 30, 0, NULL, line1, (UINT)wcslen(line1), NULL);
    ExtTextOutW(dc, 10, 60, 0, NULL, line2, (UINT)wcslen(line2), NULL);
    ReleaseDC(win, dc);
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

#define MAX_THREADS 64

/* Logs every process, and returns the one whose program name has "doom"
 * in it (any case), else 0. Version 2 suspended the program owning the
 * window in front: 3 threads, and the game didn't pause, so that wasn't
 * the game. */
static DWORD find_game(void)
{
    PROCESSENTRY32 pe;
    DWORD game = 0;
    /* No heap lists: version 3's snapshot came back empty with the game
     * running (3-9 MiB of RAM free). */
    HANDLE snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS | 0x40000000 /* NOHEAPS */, 0);
    if (snap == INVALID_HANDLE_VALUE) {
        out("# processes: snapshot error %lu", (unsigned long)GetLastError());
        return 0;
    }
    pe.dwSize = sizeof pe;
    if (!Process32First(snap, &pe))
        out("# processes: Process32First error %lu", (unsigned long)GetLastError());
    else {
        do {
            char name[MAX_PATH];
            int i;
            /* Lower-case ASCII, for the log and the match. */
            for (i = 0; pe.szExeFile[i] && i < MAX_PATH - 1; i++) {
                WCHAR c = pe.szExeFile[i];
                name[i] = c >= 'A' && c <= 'Z' ? (char)(c + 32) : c < 128 ? (char)c : '?';
            }
            name[i] = 0;
            out("# process %08lx: %s, %lu threads", (unsigned long)pe.th32ProcessID, name,
                (unsigned long)pe.cntThreads);
            if (!game && strstr(name, "doom"))
                game = pe.th32ProcessID;
        } while (Process32Next(snap, &pe));
    }
    CloseToolhelp32Snapshot(snap);
    return game;
}

/* Suspends (or, `resume`, resumes) every thread of process `pid`; the
 * count of threads. */
static int suspend_all(DWORD pid, int resume, DWORD *ids, int n)
{
    int k;
    if (!resume) {
        THREADENTRY32 te;
        HANDLE snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        n = 0;
        if (snap == INVALID_HANDLE_VALUE)
            return 0;
        te.dwSize = sizeof te;
        if (Thread32First(snap, &te)) {
            do {
                if (te.th32OwnerProcessID == pid && n < MAX_THREADS)
                    ids[n++] = te.th32ThreadID;
            } while (Thread32Next(snap, &te));
        }
        CloseToolhelp32Snapshot(snap);
    }
    /* In CE a thread's ID is its handle. */
    for (k = 0; k < n; k++) {
        if (resume)
            ResumeThread((HANDLE)ids[k]);
        else
            SuspendThread((HANDLE)ids[k]);
    }
    return n;
}

/* The chip's memory to rp-wm3d-<k>.bin, a slice at a time: the game
 * suspended only while a slice is copied to RAM, never during file work. */
static void capture(int k, volatile unsigned long *mem, volatile unsigned long *regs)
{
    DWORD ids[MAX_THREADS], pid = 0, mine = GetCurrentProcessId(), slice = CAP_SIZE;
    DWORD at, i, w, worst_us = 0;
    MEMORYSTATUS ms;
    HWND fg = GetForegroundWindow();
    WCHAR path[MAX_PATH];
    HANDLE f;
    unsigned long *buf = NULL, clk = 0, st0 = 0, st1 = 0;
    unsigned long long t0, t1;
    int n = 0, ok = 1;

    ms.dwLength = sizeof ms;
    GlobalMemoryStatus(&ms);
    if (fg)
        GetWindowThreadProcessId(fg, &pid);
    out("# capture %d: in front: window %08lx, process %08lx; free RAM %lu KiB", k,
        (unsigned long)fg, (unsigned long)pid, (unsigned long)(ms.dwAvailPhys >> 10));
    {
        /* The window in front named (title and class, ASCII): no drawing. */
        WCHAR t[64], c[64];
        char tn[64], cn[64];
        int i;
        t[0] = c[0] = 0;
        if (fg) {
            GetWindowTextW(fg, t, 64);
            GetClassNameW(fg, c, 64);
        }
        for (i = 0; i < 63 && t[i]; i++)
            tn[i] = t[i] >= 'A' && t[i] <= 'Z' ? (char)(t[i] + 32) : t[i] < 128 ? (char)t[i] : '?';
        tn[i] = 0;
        for (i = 0; i < 63 && c[i]; i++)
            cn[i] = c[i] < 128 ? (char)c[i] : '?';
        cn[i] = 0;
        out("# capture %d: in front: title \"%s\", class \"%s\"", k, tn, cn);
        {
            DWORD named = find_game();
            if (named)
                pid = named;
            else if (!strstr(tn, "doom"))
                pid = 0;
        }
    }
    out("# capture %d: the game: process %08lx", k, (unsigned long)pid);
    if (!pid || pid == mine) {
        out("# capture %d: no program or window named *doom*: nothing copied", k);
        return;
    }
    while (slice >= (256UL << 10) &&
           !(buf = (unsigned long *)VirtualAlloc(NULL, slice, MEM_COMMIT, PAGE_READWRITE)))
        slice /= 2;
    if (!buf) {
        out("# capture %d: no RAM for a slice: nothing copied", k);
        return;
    }
    _snwprintf(path, MAX_PATH, L"%srp-wm3d-%d.bin", dir, k);
    f = CreateFileW(path, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, 0, NULL);
    if (f == INVALID_HANDLE_VALUE) {
        out("# capture %d: CreateFile error %lu", k, (unsigned long)GetLastError());
        VirtualFree(buf, 0, MEM_RELEASE);
        return;
    }
    out("# capture %d: slices of %lu KiB", k, (unsigned long)(slice >> 10));
    out("# capture %d: 2700G memory %08lx-%08lx", k, CAP_FROM, CAP_FROM + CAP_SIZE - 1);
    for (at = CAP_FROM; at < CAP_FROM + CAP_SIZE && ok; at += slice) {
        out("# capture %d: slice at %08lx", k, (unsigned long)at);
        DWORD len = CAP_FROM + CAP_SIZE - at < slice ? CAP_FROM + CAP_SIZE - at : slice;
        /* Suspended: no file, no log, no drawing until resumed. */
        n = suspend_all(pid, 0, ids, 0);
        Sleep(50);
        if (at == CAP_FROM) {
            clk = regs[MBXCLK / 4];
            st0 = regs[MBX_STATUS / 4];
        }
        t0 = now_ticks();
        for (i = 0; i < len / 4; i++)
            buf[i] = mem[at / 4 + i];
        t1 = now_ticks();
        st1 = regs[MBX_STATUS / 4];
        suspend_all(pid, 1, ids, n);
        if (us_between(t0, t1) > worst_us)
            worst_us = us_between(t0, t1);
        ok = WriteFile(f, buf, len, &w, NULL) && w == len;
        if (!ok)
            out("# capture %d: WriteFile error %lu at %08lx", k, (unsigned long)GetLastError(),
                (unsigned long)at);
    }
    CloseHandle(f);
    VirtualFree(buf, 0, MEM_RELEASE);
    out("CAPTURE %d: MBXCLK %08lx, status %08lx first, %08lx last; %d game threads; "
        "%s; longest slice copy %lu ms", k, clk, st0, st1, n,
        ok ? "saved whole" : "SAVED IN PART", worst_us / 1000);
}

static LRESULT CALLBACK proc(HWND h, UINT m, WPARAM w, LPARAM l)
{
    return DefWindowProcW(h, m, w, l);
}

int WINAPI WinMain(HINSTANCE inst, HINSTANCE prev, LPWSTR cmd, int show)
{
    WCHAR logpath[MAX_PATH], *slash;
    WNDCLASSW wc;
    volatile unsigned long *mem, *regs;
    int k;
    (void)prev;
    (void)cmd;

    GetModuleFileNameW(inst, dir, MAX_PATH);
    slash = wcsrchr(dir, L'\\');
    if (slash)
        slash[1] = 0;
    _snwprintf(logpath, MAX_PATH, L"%srp-wm3d.txt", dir);
    logf_ = CreateFileW(logpath, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, 0, NULL);
    if (logf_ == INVALID_HANDLE_VALUE)
        return 1;
    QueryPerformanceFrequency(&freq);

    memset(&wc, 0, sizeof wc);
    wc.lpfnWndProc = proc;
    wc.hInstance = inst;
    wc.lpszClassName = L"rpwm3d";
    RegisterClassW(&wc);
    win = CreateWindowW(L"rpwm3d", L"RetroPane 3D capture", WS_VISIBLE, 0, 0,
                        CW_USEDEFAULT, CW_USEDEFAULT, NULL, NULL, inst, NULL);
    ShowWindow(win, show);
    UpdateWindow(win);

    out("RETROPANE-WM3D: the 2700G's memory while a game draws (ADR-0024 M3)");
    mem = map_phys(MEM_PHYS, MEM_SIZE);
    regs = map_phys(REGS_PHYS, REGS_SIZE);
    if (!mem || !regs) {
        say(L"RetroPane 3D capture: could not map the chip.", L"Nothing captured.");
        out("# no mapping: nothing captured");
        Sleep(5000);
        return 1;
    }
    say(L"Start DoomGLES now; play 2.5 minutes.", L"Short hitches from 1 minute.");
    MessageBeep(MB_OK);
    for (k = 0; k < CAPTURES; k++) {
        Sleep(AT_S[k] * 1000);
        capture(k + 1, mem, regs);
    }
    out("RETROPANE-WM3D: done");
    /* Nothing drawn while the game is in front: a beep only. */
    MessageBeep(MB_ICONEXCLAMATION);
    CloseHandle(logf_);
    return 0;
}
