/*
 * rp-wmprobe: the Windows Mobile probe and baseline suite (the owner,
 * 2026-10-23: one program on the card, run from Windows Mobile, one log).
 *
 * Module 1, divide 1 under Dell's own driver. Intel's datasheet lists the
 * 2700G's graphics clock divider as 1, 2 or 4 (graphics clock 13-75 MHz),
 * and Dell's driver picks divide 1 itself when its CoreClock setting is at
 * or under 75 MHz (docs/hw/2700g.md: the divider choice at 0x021204f8,
 * the 75,000,000 threshold at 0x02120724; CoreClock in Hz, Dell's default
 * 133,000,000 clamped to the X51v memory's 100 MHz). RetroPane's own
 * renderer draws Gouraud wrong at divide 1 at every speed (runs GC2D-GC2W);
 * this asks whether Dell's driver does too.
 *
 * Two runs, a soft reset between:
 *   run 1 (CoreClock absent: Windows Mobile's 99.67 MHz, divide 2): the
 *     clock registers logged; a smooth-shaded quad drawn 20 times through
 *     Dell's libGLES_CL and read back (glReadPixels), each timed and
 *     compared with the first; the first saved as the reference; then
 *     CoreClock set to 75,000,000.
 *   run 2 (CoreClock 75,000,000): the value deleted first (the next reset
 *     is back to normal whatever happens), the registers logged (is it
 *     divide 1?), the same 20 draws compared with each other and with run
 *     1's reference, the render-table registers logged.
 *
 * Observation only otherwise (docs/CLEANROOM.md): registers mapped
 * read-only and uncached; the one write is Dell's documented registry value.
 * The log is rp-wmprobe.txt next to the program; the pictures
 * rp-wmprobe-ref.bin and rp-wmprobe-div1.bin (480 x 640 RGBA). Built by
 * `cargo xtask wmbench`.
 */

#define _WIN32_WCE 0x0501
#include <windows.h>
#include <string.h>
#include <stdio.h>
#include <stdlib.h>

BOOL VirtualCopy(LPVOID dest, LPVOID src, DWORD size, DWORD protect);

#define SYS_PHYS 0x0FFE0000UL
#define MBX_PHYS 0x0FFE4000UL
#define RENDERS 20
#define CORECLOCK_DIV1 75000000UL
#define KEY L"Drivers\\Display\\PowerVR"

static LARGE_INTEGER freq;
static HANDLE logf_ = INVALID_HANDLE_VALUE;
static WCHAR dir[MAX_PATH];
static HWND win;
static int w, h;

static void out(const char *fmt, ...)
{
    char line[400];
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

static unsigned long us_since(unsigned long long t0)
{
    LARGE_INTEGER t;
    QueryPerformanceCounter(&t);
    return (unsigned long)(((unsigned long long)t.QuadPart - t0) * 1000000ULL /
                           (unsigned long long)freq.QuadPart);
}

static unsigned long long ticks(void)
{
    LARGE_INTEGER t;
    QueryPerformanceCounter(&t);
    return (unsigned long long)t.QuadPart;
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

/* ---- the registers -------------------------------------------------- */

static volatile unsigned long *sys, *mbx;

static const struct { const char *name; unsigned long off; } clock_regs[] = {
    {"SYSCFG", 0x000}, {"SYSCLKSRC", 0x02C}, {"COREPLL", 0x038}, {"DISPPLL", 0x03C},
    {"PLLSTAT", 0x040}, {"MEMCLK", 0x04C}, {"M24CLK", 0x050}, {"MBXCLK", 0x054},
    {"SDCLK", 0x058}, {"LMTYPE", 0x1014}, {"LMTIM", 0x1018}, {"LMREFRESH", 0x101C},
};

/* The render table's words (each read safely under RetroPane's loader,
 * run GC2R), 0x6D0 left out (never read). */
static const unsigned long table_regs[] = {
    0x608, 0x60C, 0x610, 0x614, 0x618, 0x61C, 0x644, 0x648, 0x64C,
    0x650, 0x654, 0x658, 0x65C, 0x660, 0x674, 0x6D8, 0x6DC,
};

static void log_clocks(const char *when)
{
    unsigned i;
    unsigned long pll, m, n, p, div;
    if (!sys) {
        out("%s: the 2700G's registers not mapped", when);
        return;
    }
    for (i = 0; i < sizeof clock_regs / sizeof clock_regs[0]; i++)
        out("%s: %s (0x%03lx) 0x%08lx", when, clock_regs[i].name, clock_regs[i].off,
            sys[clock_regs[i].off / 4]);
    pll = sys[0x038 / 4];
    m = pll >> 7 & 63;
    n = pll >> 4 & 7;
    p = pll >> 1 & 7;
    div = (sys[0x054 / 4] >> 2 & 3) + 1;
    if (n)
        out("%s: the system clock %lu kHz, the MBX cores at divide %lu = %lu kHz", when,
            13000UL * m / (n << p), div, 13000UL * m / (n << p) / div);
}

static void log_table(const char *when)
{
    unsigned i;
    if (!mbx)
        return;
    for (i = 0; i < sizeof table_regs / sizeof table_regs[0]; i++)
        out("%s: MBX 0x%03lx 0x%08lx", when, table_regs[i], mbx[table_regs[i] / 4]);
}

/* ---- Dell's OpenGL ES (libGLES_CL, by name) ------------------------- */

typedef void *EGLDisplay, *EGLConfig, *EGLSurface, *EGLContext;
typedef int EGLint;
typedef unsigned int GLenum, GLbitfield;
typedef int GLint, GLsizei, GLfixed;

static EGLDisplay (*eglGetDisplay)(void *);
static int (*eglInitialize)(EGLDisplay, EGLint *, EGLint *);
static int (*eglChooseConfig)(EGLDisplay, const EGLint *, EGLConfig *, EGLint, EGLint *);
static EGLSurface (*eglCreateWindowSurface)(EGLDisplay, EGLConfig, HWND, const EGLint *);
static EGLContext (*eglCreateContext)(EGLDisplay, EGLConfig, EGLContext, const EGLint *);
static int (*eglMakeCurrent)(EGLDisplay, EGLSurface, EGLSurface, EGLContext);
static int (*eglSwapBuffers)(EGLDisplay, EGLSurface);
static int (*eglDestroyContext)(EGLDisplay, EGLContext);
static int (*eglDestroySurface)(EGLDisplay, EGLSurface);
static int (*eglTerminate)(EGLDisplay);
static EGLint (*eglGetError)(void);
static void (*glViewport)(GLint, GLint, GLsizei, GLsizei);
static void (*glClearColorx)(GLfixed, GLfixed, GLfixed, GLfixed);
static void (*glClear)(GLbitfield);
static void (*glMatrixMode)(GLenum);
static void (*glLoadIdentity)(void);
static void (*glOrthox)(GLfixed, GLfixed, GLfixed, GLfixed, GLfixed, GLfixed);
static void (*glShadeModel)(GLenum);
static void (*glDisable)(GLenum);
static void (*glEnableClientState)(GLenum);
static void (*glVertexPointer)(GLint, GLenum, GLsizei, const void *);
static void (*glColorPointer)(GLint, GLenum, GLsizei, const void *);
static void (*glDrawArrays)(GLenum, GLint, GLsizei);
static void (*glFinish)(void);
static void (*glReadPixels)(GLint, GLint, GLsizei, GLsizei, GLenum, GLenum, void *);
static GLenum (*glGetError)(void);

static int load_gles(void)
{
    HMODULE m = LoadLibraryW(L"libGLES_CL.dll");
    int missing = 0;
    if (!m) {
        out("libGLES_CL.dll: not loaded, error %lu", (unsigned long)GetLastError());
        return 0;
    }
#define GET(f)                                                  \
    do {                                                        \
        FARPROC a_ = GetProcAddressW(m, L## #f);                \
        memcpy(&f, &a_, sizeof f);                              \
        if (!f) {                                               \
            out("libGLES_CL.dll: no %s", #f);                   \
            missing++;                                          \
        }                                                       \
    } while (0)
    GET(eglGetDisplay); GET(eglInitialize); GET(eglChooseConfig);
    GET(eglCreateWindowSurface); GET(eglCreateContext); GET(eglMakeCurrent);
    GET(eglSwapBuffers); GET(eglDestroyContext); GET(eglDestroySurface);
    GET(eglTerminate); GET(eglGetError);
    GET(glViewport); GET(glClearColorx); GET(glClear); GET(glMatrixMode);
    GET(glLoadIdentity); GET(glOrthox); GET(glShadeModel); GET(glDisable);
    GET(glEnableClientState); GET(glVertexPointer); GET(glColorPointer);
    GET(glDrawArrays); GET(glFinish); GET(glReadPixels); GET(glGetError);
#undef GET
    return missing == 0;
}

/* ---- the drawing ---------------------------------------------------- */

/* The quad RetroPane's run GC2 draws first ("RGB565 Gouraud"): black, white,
 * yellow, blue at the top left, top right, bottom left, bottom right
 * (a strip's order), the colours of gc2.rs's CORNERS. */
static const GLfixed quad_xy[8] = {
    0, 0, 480 << 16, 0, 0, 640 << 16, 480 << 16, 640 << 16,
};
static const unsigned char quad_rgba[16] = {
    0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xE0, 0x00, 0xFF, 0x10, 0x20, 0xA0, 0xFF,
};

static EGLDisplay dpy;
static EGLSurface surf;
static EGLContext ctx;

static int gl_start(void)
{
    static const EGLint attrs[] = {0x3024, 5, 0x3023, 6, 0x3022, 5, 0x3033, 4, 0x3038};
    EGLConfig cfg;
    EGLint maj, min, num = 0;
    dpy = eglGetDisplay(NULL);
    if (!eglInitialize(dpy, &maj, &min)) {
        out("eglInitialize failed, 0x%x", eglGetError());
        return 0;
    }
    out("EGL %d.%d", maj, min);
    if (!eglChooseConfig(dpy, attrs, &cfg, 1, &num) || num < 1) {
        out("eglChooseConfig: none, 0x%x", eglGetError());
        return 0;
    }
    surf = eglCreateWindowSurface(dpy, cfg, win, NULL);
    ctx = eglCreateContext(dpy, cfg, NULL, NULL);
    if (!surf || !ctx || !eglMakeCurrent(dpy, surf, surf, ctx)) {
        out("EGL surface or context failed, 0x%x", eglGetError());
        return 0;
    }
    glViewport(0, 0, w, h);
    glMatrixMode(0x1701);
    glLoadIdentity();
    glOrthox(0, 480 << 16, 640 << 16, 0, -65536, 65536);
    glMatrixMode(0x1700);
    glLoadIdentity();
    glShadeModel(0x1D01);
    glDisable(0x0BD0);
    glEnableClientState(0x8074);
    glEnableClientState(0x8076);
    glVertexPointer(2, 0x140C, 0, quad_xy);
    glColorPointer(4, 0x1401, 0, quad_rgba);
    return 1;
}

static void gl_stop(void)
{
    eglMakeCurrent(dpy, NULL, NULL, NULL);
    eglDestroyContext(dpy, ctx);
    eglDestroySurface(dpy, surf);
    eglTerminate(dpy);
}

/* One draw read back into pic; its time in us (the draw and glFinish). */
static unsigned long draw(unsigned long *pic)
{
    unsigned long long t0;
    unsigned long us;
    glClearColorx(0, 0, 0, 65536);
    glClear(0x4000);
    t0 = ticks();
    glDrawArrays(5, 0, 4);
    glFinish();
    us = us_since(t0);
    glReadPixels(0, 0, w, h, 0x1908, 0x1401, pic);
    eglSwapBuffers(dpy, surf);
    return us;
}

/* Pixels of a differing from b; the first three logged. */
static unsigned long differ(const unsigned long *a, const unsigned long *b, const char *what)
{
    unsigned long i, n = 0, words = (unsigned long)w * (unsigned long)h;
    for (i = 0; i < words; i++)
        if (a[i] != b[i]) {
            if (n < 3)
                out("  %s: (%lu,%lu) 0x%08lx, not 0x%08lx", what, i % (unsigned long)w,
                    i / (unsigned long)w, a[i], b[i]);
            n++;
        }
    return n;
}

static int save(const WCHAR *name, const unsigned long *pic)
{
    WCHAR path[MAX_PATH];
    DWORD n = 0;
    HANDLE f;
    _snwprintf(path, MAX_PATH, L"%s\\%s", dir, name);
    f = CreateFileW(path, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, 0, NULL);
    if (f == INVALID_HANDLE_VALUE)
        return 0;
    WriteFile(f, pic, (DWORD)(w * h * 4), &n, NULL);
    CloseHandle(f);
    return n == (DWORD)(w * h * 4);
}

static int load(const WCHAR *name, unsigned long *pic)
{
    WCHAR path[MAX_PATH];
    DWORD n = 0;
    HANDLE f;
    _snwprintf(path, MAX_PATH, L"%s\\%s", dir, name);
    f = CreateFileW(path, GENERIC_READ, 0, NULL, OPEN_EXISTING, 0, NULL);
    if (f == INVALID_HANDLE_VALUE)
        return 0;
    ReadFile(f, pic, (DWORD)(w * h * 4), &n, NULL);
    CloseHandle(f);
    return n == (DWORD)(w * h * 4);
}

/* RENDERS draws: each timed, each compared with the first (and with ref,
 * if given). The first is left in first. */
static void draws(unsigned long *first, unsigned long *pic, const unsigned long *ref)
{
    unsigned i;
    unsigned long best = 0xFFFFFFFFUL, worst = 0, us, d, wrong = 0, wrong_ref = 0, total = 0;
    for (i = 0; i < RENDERS; i++) {
        MSG msg;
        while (PeekMessageW(&msg, NULL, 0, 0, PM_REMOVE))
            DispatchMessageW(&msg);
        us = draw(i ? pic : first);
        if (us < best)
            best = us;
        if (us > worst)
            worst = us;
        if (i) {
            d = differ(pic, first, "differs from draw 1");
            wrong += d != 0;
            total += d;
        }
        if (ref) {
            d = differ(i ? pic : first, ref, "differs from run 1's reference");
            wrong_ref += d != 0;
            if (d)
                out("  draw %u: %lu pixels differ from run 1's reference", i + 1, d);
        }
    }
    out("%d draws: %lu to %lu us each; %lu of %d differ from draw 1 (%lu pixels in all)%s",
        RENDERS, best, worst, wrong, RENDERS - 1, total, ref ? "" : "");
    if (ref)
        out("against run 1's reference (divide 2): %lu of %d draws differ", wrong_ref, RENDERS);
    out("GL error 0x%x", glGetError());
}

/* ---- the module ----------------------------------------------------- */

static LRESULT CALLBACK wndproc(HWND hw, UINT m, WPARAM wp, LPARAM lp)
{
    return DefWindowProcW(hw, m, wp, lp);
}

static void say(const WCHAR *a, const WCHAR *b)
{
    MessageBoxW(NULL, b, a, MB_OK | MB_TOPMOST);
}

static void divide1(void)
{
    HKEY key;
    DWORD type, v = 0, size = sizeof v, disp;
    LONG got;
    int run2;
    unsigned long *first, *pic, *ref = NULL;

    if (RegCreateKeyExW(HKEY_LOCAL_MACHINE, KEY, 0, NULL, 0, 0, NULL, &key, &disp) != ERROR_SUCCESS) {
        out("HKLM\\Drivers\\Display\\PowerVR: not opened");
        return;
    }
    got = RegQueryValueExW(key, L"CoreClock", NULL, &type, (BYTE *)&v, &size);
    run2 = got == ERROR_SUCCESS && type == REG_DWORD && v == CORECLOCK_DIV1;
    if (got == ERROR_SUCCESS && !run2) {
        out("CoreClock is already set (type %lu, %lu) and not this tool's: nothing changed",
            (unsigned long)type, (unsigned long)v);
        RegCloseKey(key);
        say(L"rp-wmprobe", L"CoreClock was set already: nothing changed. See rp-wmprobe.txt.");
        return;
    }
    out("== module 1, divide 1 under Dell's driver: run %d (CoreClock %s)", run2 ? 2 : 1,
        run2 ? "75000000" : "absent: Dell's default");
    if (run2) {
        /* First of all: the next reset is back to normal whatever happens. */
        RegDeleteValueW(key, L"CoreClock");
        RegFlushKey(key);
        out("CoreClock deleted (the next soft reset is back to normal)");
    }
    log_clocks(run2 ? "run 2 before" : "run 1 before");

    first = (unsigned long *)malloc((size_t)w * h * 4);
    pic = (unsigned long *)malloc((size_t)w * h * 4);
    if (run2)
        ref = (unsigned long *)malloc((size_t)w * h * 4);
    if (!first || !pic || (run2 && !ref)) {
        out("no memory for the pictures");
        RegCloseKey(key);
        return;
    }
    if (run2 && !load(L"rp-wmprobe-ref.bin", ref)) {
        out("run 1's reference (rp-wmprobe-ref.bin) not found: compared within this run only");
        free(ref);
        ref = NULL;
    }
    if (!load_gles() || !gl_start()) {
        RegCloseKey(key);
        return;
    }
    draws(first, pic, ref);
    log_clocks(run2 ? "run 2 after" : "run 1 after");
    log_table(run2 ? "run 2 after" : "run 1 after");
    gl_stop();
    if (!run2) {
        save(L"rp-wmprobe-ref.bin", first);
        v = CORECLOCK_DIV1;
        RegSetValueExW(key, L"CoreClock", 0, REG_DWORD, (const BYTE *)&v, sizeof v);
        RegFlushKey(key);
        out("CoreClock set to 75000000: soft-reset, then run rp-wmprobe again");
        RegCloseKey(key);
        say(L"rp-wmprobe: run 1 done",
            L"Now soft-reset the Axim, then run rp-wmprobe again (run 2).");
    } else {
        save(L"rp-wmprobe-div1.bin", first);
        out("run 2 done; soft-reset to return to normal");
        RegCloseKey(key);
        say(L"rp-wmprobe: run 2 done", L"Done. Soft-reset to return to normal. Log: rp-wmprobe.txt");
    }
    free(first);
    free(pic);
    free(ref);
}

int WINAPI WinMain(HINSTANCE inst, HINSTANCE prev, LPWSTR cmd, int show)
{
    WCHAR path[MAX_PATH];
    WNDCLASSW wc;
    WCHAR *slash;
    (void)prev;
    (void)cmd;
    (void)show;
    QueryPerformanceFrequency(&freq);
    GetModuleFileNameW(NULL, dir, MAX_PATH);
    slash = wcsrchr(dir, L'\\');
    if (slash)
        *slash = 0;
    _snwprintf(path, MAX_PATH, L"%s\\rp-wmprobe.txt", dir);
    logf_ = CreateFileW(path, GENERIC_WRITE, FILE_SHARE_READ, NULL, OPEN_ALWAYS, 0, NULL);
    if (logf_ == INVALID_HANDLE_VALUE)
        return 1;
    SetFilePointer(logf_, 0, NULL, FILE_END);
    out("rp-wmprobe v1 (Windows Mobile probe and baseline suite), tick %lu",
        (unsigned long)GetTickCount());

    sys = map_phys(SYS_PHYS, 0x2000);
    mbx = map_phys(MBX_PHYS, 0x1000);

    w = 480;
    h = 640;
    memset(&wc, 0, sizeof wc);
    wc.lpfnWndProc = wndproc;
    wc.hInstance = inst;
    wc.lpszClassName = L"rp-wmprobe";
    RegisterClassW(&wc);
    win = CreateWindowW(L"rp-wmprobe", L"rp-wmprobe", WS_VISIBLE | WS_POPUP, 0, 0, w, h, NULL,
                        NULL, inst, NULL);
    if (!win) {
        out("no window, error %lu", (unsigned long)GetLastError());
        return 1;
    }
    divide1();
    out("== end");
    CloseHandle(logf_);
    return 0;
}
