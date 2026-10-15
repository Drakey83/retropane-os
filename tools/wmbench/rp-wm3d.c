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
 * copy is due), 50 ms are left for the core to finish what it has, and the
 * memory goes straight from the chip to the card in 64 KiB pieces (~15 s);
 * then the game resumes.
 *
 * Version 2 (2026-10-14). Version 1 copied into RAM first: with the game
 * running there were no 16 MiB to be had, so nothing was copied; and it
 * drew on its window when done, and the game froze there (music still
 * playing; a soft reset). Now nothing is drawn after the start screen:
 * beeps only.
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
#define REGS_PHYS 0x0FFE0000UL
#define REGS_SIZE 0x5000UL
#define MBXCLK 0x0054UL
#define MBX_STATUS 0x412cUL

/* Seconds before each copy: time to start the game and get playing. */
static const DWORD AT_S[2] = {60, 30};

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
static unsigned long piece[65536 / 4];

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

/* The chip's memory, straight to rp-wm3d-<k>.bin in 64 KiB pieces; true
 * if all of it was written. */
static int save(int k, volatile unsigned long *mem)
{
    WCHAR path[MAX_PATH];
    HANDLE f;
    DWORD done = 0, w, i;
    _snwprintf(path, MAX_PATH, L"%srp-wm3d-%d.bin", dir, k);
    f = CreateFileW(path, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, 0, NULL);
    if (f == INVALID_HANDLE_VALUE) {
        out("# save %d: CreateFile error %lu", k, (unsigned long)GetLastError());
        return 0;
    }
    while (done < MEM_SIZE) {
        for (i = 0; i < sizeof piece / 4; i++)
            piece[i] = mem[done / 4 + i];
        if (!WriteFile(f, piece, sizeof piece, &w, NULL) || w != sizeof piece) {
            out("# save %d: WriteFile error %lu at %lu", k, (unsigned long)GetLastError(),
                (unsigned long)done);
            break;
        }
        done += w;
    }
    CloseHandle(f);
    return done == MEM_SIZE;
}

static void capture(int k, volatile unsigned long *mem, volatile unsigned long *regs)
{
    DWORD ids[MAX_THREADS], pid = 0, mine = GetCurrentProcessId();
    MEMORYSTATUS ms;
    HWND fg = GetForegroundWindow();
    unsigned long clk, st0, st1;
    unsigned long long t0, t1;
    int n, ok;

    ms.dwLength = sizeof ms;
    GlobalMemoryStatus(&ms);
    if (fg)
        GetWindowThreadProcessId(fg, &pid);
    out("# capture %d: in front: window %08lx, process %08lx; free RAM %lu KiB", k,
        (unsigned long)fg, (unsigned long)pid, (unsigned long)(ms.dwAvailPhys >> 10));
    if (!pid || pid == mine) {
        out("# capture %d: no game in front: nothing copied", k);
        return;
    }
    n = suspend_all(pid, 0, ids, 0);
    out("# capture %d: %d threads of the game suspended", k, n);
    Sleep(50);
    clk = regs[MBXCLK / 4];
    st0 = regs[MBX_STATUS / 4];
    t0 = now_ticks();
    ok = save(k, mem);
    t1 = now_ticks();
    st1 = regs[MBX_STATUS / 4];
    suspend_all(pid, 1, ids, n);
    out("CAPTURE %d: MBXCLK %08lx, status %08lx before, %08lx after; %s in %lu ms", k, clk,
        st0, st1, ok ? "saved whole" : "SAVED IN PART", us_between(t0, t1) / 1000);
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
    say(L"Start DoomGLES now and play.", L"It pauses at 1 min and ~2 min. Beeps when done.");
    MessageBeep(MB_OK);
    for (k = 0; k < 2; k++) {
        Sleep(AT_S[k] * 1000);
        capture(k + 1, mem, regs);
    }
    out("RETROPANE-WM3D: done");
    /* Nothing drawn while the game is in front: a beep only. */
    MessageBeep(MB_ICONEXCLAMATION);
    CloseHandle(logf_);
    return 0;
}
