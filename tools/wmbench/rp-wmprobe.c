/*
 * rp-wmprobe: the Windows Mobile probe and baseline suite (the owner,
 * 2026-10-23: one program on the card, run from Windows Mobile, one log:
 * every baseline the gate's lines want from Windows Mobile, and probes of
 * how Windows Mobile sets the hardware up).
 *
 * v2, every launch, in order (each module's lines start with its name):
 *   env     Windows Mobile's version, memory, uptime, the running programs
 *           (a shell such as SPB shows here);
 *   pxa     the PXA270's clock, memory-timing, power, LCD and GPIO
 *           registers as Windows Mobile leaves them, the core-voltage pins
 *           (GPIO3, 4, 38) decoded;
 *   2700g   all 146 named 2700G registers below the MBX block;
 *   reg     every value of Dell's display driver key;
 *   hop     the 4 KiB page hop (gate line CPU V7): a random walk over 8 MiB,
 *           one word a page, against the same walk over 32 KiB and 256 KiB;
 *   vram    CPU reads of 2700G memory (256 KiB, the best of three);
 *   gl      3D through Dell's libGLES_CL: clear, flat, Gouraud, RGB565
 *           texture nearest and bilinear, ARGB8888 texture, translucent,
 *           two translucent textured layers, 2,000 small triangles; each 20
 *           frames timed (glFinish), the first and last read back and
 *           compared; glReadPixels and eglSwapBuffers timed;
 *   bench   rp-wmbench run beside it ("suite": no questions, its own folder
 *           only): CPU, memory, threads, SHA-256, 2D drawing, the card;
 *           its rows copied into this log;
 *   div1    module 1 below, last.
 * A second launch (after module 1's soft reset) repeats everything at Dell's
 * divide-1 setting.
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
#include <tlhelp32.h>

#include "rp-2700g-regs.h"

/* Windows CE's flag for a process list without heap lists (tlhelp32.h on
 * CE; cegcc's header lacks it). */
#ifndef TH32CS_SNAPNOHEAPS
#define TH32CS_SNAPNOHEAPS 0x40000000
#endif

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
static void (*glGenTextures)(GLsizei, unsigned int *);
static void (*glBindTexture)(GLenum, unsigned int);
static void (*glTexImage2D)(GLenum, GLint, GLint, GLsizei, GLsizei, GLint, GLenum, GLenum,
                            const void *);
static void (*glTexParameterx)(GLenum, GLenum, GLfixed);
static void (*glEnable)(GLenum);
static void (*glDisableClientState)(GLenum);
static void (*glTexCoordPointer)(GLint, GLenum, GLsizei, const void *);
static void (*glTexEnvx)(GLenum, GLenum, GLfixed);
static void (*glBlendFunc)(GLenum, GLenum);
static void (*glColor4x)(GLfixed, GLfixed, GLfixed, GLfixed);

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
    GET(glGenTextures); GET(glBindTexture); GET(glTexImage2D); GET(glTexParameterx);
    GET(glEnable); GET(glDisableClientState); GET(glTexCoordPointer); GET(glTexEnvx);
    GET(glBlendFunc); GET(glColor4x);
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

/* ---- env: Windows Mobile itself ------------------------------------- */

static void module_env(void)
{
    OSVERSIONINFO ver;
    MEMORYSTATUS ms;
    SYSTEMTIME st;
    HANDLE snap;
    PROCESSENTRY32 pe;
    ver.dwOSVersionInfoSize = sizeof ver;
    GetVersionEx(&ver);
    ms.dwLength = sizeof ms;
    GlobalMemoryStatus(&ms);
    GetLocalTime(&st);
    out("env: run %04d-%02d-%02d %02d:%02d:%02d", st.wYear, st.wMonth, st.wDay, st.wHour,
        st.wMinute, st.wSecond);
    out("env: Windows CE %lu.%lu build %lu", (unsigned long)ver.dwMajorVersion,
        (unsigned long)ver.dwMinorVersion, (unsigned long)ver.dwBuildNumber);
    out("env: memory %lu KB, %lu KB free; uptime %lu ms", (unsigned long)(ms.dwTotalPhys / 1024),
        (unsigned long)(ms.dwAvailPhys / 1024), (unsigned long)GetTickCount());
    snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS | TH32CS_SNAPNOHEAPS, 0);
    if (snap == INVALID_HANDLE_VALUE) {
        out("env: no process list, error %lu", (unsigned long)GetLastError());
        return;
    }
    pe.dwSize = sizeof pe;
    if (Process32First(snap, &pe))
        do {
            char name[64];
            int k;
            for (k = 0; pe.szExeFile[k] && k < 63; k++)
                name[k] = (char)pe.szExeFile[k];
            name[k] = 0;
            out("env: process %s (%lu threads)", name, (unsigned long)pe.cntThreads);
        } while (Process32Next(snap, &pe));
    CloseToolhelp32Snapshot(snap);
}

/* ---- pxa: the processor's registers as Windows Mobile leaves them ---- */

static void dump_block(const char *what, DWORD phys, const unsigned long *offs, const char *const *names,
                       unsigned n, DWORD size)
{
    volatile unsigned long *r = map_phys(phys, size);
    unsigned i;
    if (!r)
        return;
    for (i = 0; i < n; i++)
        out("pxa: %s %s (0x%08lx) 0x%08lx", what, names[i], (unsigned long)(phys + offs[i]),
            r[offs[i] / 4]);
    VirtualFree((void *)r, 0, MEM_RELEASE);
}

static void module_pxa(void)
{
    static const unsigned long clk_o[] = {0x0, 0x4, 0x8, 0xC};
    static const char *const clk_n[] = {"CCCR", "CKEN", "OSCC", "CCSR"};
    static const unsigned long mem_o[] = {0x0, 0x4, 0x8, 0xC, 0x10, 0x14, 0x1C, 0x40, 0x58};
    static const char *const mem_n[] = {"MDCNFG", "MDREFR", "MSC0", "MSC1", "MSC2",
                                        "MECR", "SXCNFG", "MDMRS", "MDMRSLP"};
    static const unsigned long pm_o[] = {0x0, 0x4, 0x8, 0x1C, 0x30};
    static const char *const pm_n[] = {"PMCR", "PSSR", "PSPR", "PCFR", "RCSR"};
    static const unsigned long lcd_o[] = {0x0, 0x4, 0x8, 0xC, 0x10};
    static const char *const lcd_n[] = {"LCCR0", "LCCR1", "LCCR2", "LCCR3", "LCCR4"};
    static const unsigned long gp_o[] = {0x0, 0x4, 0x8, 0x100, 0xC, 0x10, 0x14, 0x10C};
    static const char *const gp_n[] = {"GPLR0", "GPLR1", "GPLR2", "GPLR3",
                                       "GPDR0", "GPDR1", "GPDR2", "GPDR3"};
    volatile unsigned long *gp;
    dump_block("clocks", 0x41300000UL, clk_o, clk_n, 4, 0x1000);
    dump_block("memory", 0x48000000UL, mem_o, mem_n, 9, 0x1000);
    dump_block("power", 0x40F00000UL, pm_o, pm_n, 5, 0x1000);
    dump_block("lcd", 0x44000000UL, lcd_o, lcd_n, 5, 0x1000);
    dump_block("gpio", 0x40E00000UL, gp_o, gp_n, 8, 0x1000);
    /* The core-voltage pins (docs/hw/clocks.md: GPIO38, GPIO4, GPIO3). */
    gp = map_phys(0x40E00000UL, 0x1000);
    if (gp) {
        out("pxa: core voltage pins GPIO38 %lu, GPIO4 %lu, GPIO3 %lu (Dell's 1,1,1 = its 1467/1574 mV)",
            gp[1] >> 6 & 1, gp[0] >> 4 & 1, gp[0] >> 3 & 1);
        VirtualFree((void *)gp, 0, MEM_RELEASE);
    }
}

/* ---- 2700g: every named register ------------------------------------- */

static void module_2700g(void)
{
    unsigned i;
    if (!sys)
        return;
    for (i = 0; i < sizeof named_2700g / sizeof named_2700g[0]; i++)
        out("2700g: %s (0x%04lx) 0x%08lx", named_2700g[i].name, named_2700g[i].off,
            sys[named_2700g[i].off / 4]);
}

/* ---- reg: Dell's display driver key ---------------------------------- */

static void module_reg(void)
{
    HKEY key;
    DWORD i;
    if (RegOpenKeyExW(HKEY_LOCAL_MACHINE, KEY, 0, 0, &key) != ERROR_SUCCESS) {
        out("reg: HKLM\\Drivers\\Display\\PowerVR not opened");
        return;
    }
    for (i = 0;; i++) {
        WCHAR name[64];
        BYTE data[128];
        DWORD nl = 64, dl = sizeof data, type;
        char an[64];
        int k;
        if (RegEnumValueW(key, i, name, &nl, NULL, &type, data, &dl) != ERROR_SUCCESS)
            break;
        for (k = 0; name[k] && k < 63; k++)
            an[k] = (char)name[k];
        an[k] = 0;
        if (type == REG_DWORD)
            out("reg: %s = %lu (0x%lx)", an, *(DWORD *)data, *(DWORD *)data);
        else if (type == REG_SZ) {
            char av[64];
            WCHAR *v = (WCHAR *)data;
            for (k = 0; v[k] && k < 63 && (DWORD)k * 2 < dl; k++)
                av[k] = (char)v[k];
            av[k] = 0;
            out("reg: %s = \"%s\"", an, av);
        } else
            out("reg: %s (type %lu, %lu bytes)", an, (unsigned long)type, (unsigned long)dl);
    }
    RegCloseKey(key);
}

/* ---- hop: the 4 KiB page hop (CPU V7) -------------------------------- */

/* A random walk over `bytes`, one word every `step` bytes: ns a hop. */
static unsigned long walk(unsigned long *buf, unsigned long bytes, unsigned long step)
{
    unsigned long n = bytes / step, i, j, t, *idx, at = 0, hops = 200000;
    unsigned long long t0;
    volatile unsigned long sink;
    idx = (unsigned long *)malloc(n * sizeof *idx);
    if (!idx)
        return 0;
    for (i = 0; i < n; i++)
        idx[i] = i;
    srand(12345);
    for (i = n - 1; i > 0; i--) {
        j = (unsigned long)rand() % (i + 1);
        t = idx[i];
        idx[i] = idx[j];
        idx[j] = t;
    }
    for (i = 0; i < n; i++)
        buf[idx[i] * (step / 4)] = idx[(i + 1) % n] * (step / 4);
    free(idx);
    for (i = 0; i < 1000; i++)
        at = buf[at];
    t0 = ticks();
    for (i = 0; i < hops; i++)
        at = buf[at];
    sink = at;
    (void)sink;
    return (unsigned long)((unsigned long long)us_since(t0) * 1000ULL / hops);
}

static void module_hop(void)
{
    unsigned long *buf = (unsigned long *)VirtualAlloc(NULL, 8UL << 20, MEM_COMMIT, PAGE_READWRITE);
    if (!buf) {
        out("hop: no 8 MiB");
        return;
    }
    out("hop: 8 MiB, one word a 4 KiB page: %lu ns a hop", walk(buf, 8UL << 20, 4096));
    out("hop: 256 KiB, one word a 32-byte line: %lu ns a hop", walk(buf, 256UL << 10, 32));
    out("hop: 32 KiB (the D-cache), a 32-byte line: %lu ns a hop", walk(buf, 32UL << 10, 32));
    VirtualFree(buf, 0, MEM_RELEASE);
}

/* ---- vram: CPU reads of 2700G memory ---------------------------------- */

static void module_vram(void)
{
    volatile unsigned long *m = map_phys(0x0C000000UL + (8UL << 20), 256UL << 10);
    unsigned long best = 0xFFFFFFFFUL, us, i, s = 0;
    int k;
    if (!m)
        return;
    for (k = 0; k < 3; k++) {
        unsigned long long t0 = ticks();
        for (i = 0; i < (256UL << 10) / 4; i++)
            s += m[i];
        us = us_since(t0);
        if (us < best)
            best = us;
    }
    out("vram: 256 KiB read by the CPU in %lu us (best of 3; sum 0x%lx)", best, s);
    VirtualFree((void *)m, 0, MEM_RELEASE);
}

/* ---- gl: 3D through Dell's driver ------------------------------------ */

static unsigned short tex565[256 * 256];
static unsigned long tex8888[256 * 256];
static GLfixed many_xy[2000 * 3 * 2];
static unsigned char many_rgba[2000 * 3 * 4];
static const GLfixed quad_uv[8] = {0, 0, 65536, 0, 0, 65536, 65536, 65536};

enum scene { CLEAR, FLAT, GOURAUD, TEX_NEAREST, TEX_BILINEAR, TEX_8888, TRANSLUCENT, TWO_LAYERS, MANY };
static const char *const scene_name[] = {
    "clear only", "flat", "Gouraud", "RGB565 texture nearest", "RGB565 texture bilinear",
    "ARGB8888 texture", "translucent flat", "two translucent textured layers",
    "2000 small Gouraud triangles",
};

static unsigned int tex_ids[2];

static void scene_setup(enum scene s)
{
    glDisable(0x0BE2);
    glDisable(0x0DE1);
    glDisableClientState(0x8078);
    glEnableClientState(0x8076);
    glVertexPointer(2, 0x140C, 0, quad_xy);
    glColorPointer(4, 0x1401, 0, quad_rgba);
    switch (s) {
    case FLAT:
    case TRANSLUCENT:
        glDisableClientState(0x8076);
        glColor4x(0x8000, 0x4000, 0xC000, s == TRANSLUCENT ? 0x8000 : 0x10000);
        if (s == TRANSLUCENT) {
            glEnable(0x0BE2);
            glBlendFunc(0x0302, 0x0303);
        }
        break;
    case TEX_NEAREST:
    case TEX_BILINEAR:
    case TEX_8888:
    case TWO_LAYERS:
        glDisableClientState(0x8076);
        glColor4x(0x10000, 0x10000, 0x10000, s == TWO_LAYERS ? 0x8000 : 0x10000);
        glEnable(0x0DE1);
        glBindTexture(0x0DE1, tex_ids[s == TEX_8888 ? 1 : 0]);
        glTexParameterx(0x0DE1, 0x2801, s == TEX_BILINEAR ? 0x2601 : 0x2600);
        glTexParameterx(0x0DE1, 0x2800, s == TEX_BILINEAR ? 0x2601 : 0x2600);
        glTexEnvx(0x2300, 0x2200, 0x2100); /* modulate */
        glEnableClientState(0x8078);
        glTexCoordPointer(2, 0x140C, 0, quad_uv);
        if (s == TWO_LAYERS) {
            glEnable(0x0BE2);
            glBlendFunc(0x0302, 0x0303);
        }
        break;
    case MANY:
        glVertexPointer(2, 0x140C, 0, many_xy);
        glColorPointer(4, 0x1401, 0, many_rgba);
        break;
    default:
        break;
    }
}

static void scene_draw(enum scene s)
{
    glClearColorx(0, 0, 0, 65536);
    glClear(0x4000);
    if (s == CLEAR)
        return;
    if (s == MANY)
        glDrawArrays(4, 0, 2000 * 3);
    else {
        glDrawArrays(5, 0, 4);
        if (s == TWO_LAYERS)
            glDrawArrays(5, 0, 4);
    }
}

static unsigned long pic_sum(const unsigned long *p)
{
    unsigned long s = 0x811C9DC5UL, i;
    for (i = 0; i < (unsigned long)(w * h); i++)
        s = (s ^ p[i]) * 0x01000193UL;
    return s;
}

static void module_gl(void)
{
    unsigned long *pic = (unsigned long *)malloc((size_t)w * h * 4);
    int x, y, i, f;
    if (!pic || !load_gles() || !gl_start()) {
        out("gl: not started");
        free(pic);
        return;
    }
    for (y = 0; y < 256; y++)
        for (x = 0; x < 256; x++) {
            tex565[y * 256 + x] = (unsigned short)(((x >> 3) << 11) | (((x + y) >> 2 & 63) << 5) | (y >> 3));
            tex8888[y * 256 + x] = 0x80000000UL | (unsigned long)x << 16 | (unsigned long)((x ^ y) & 255) << 8 | (unsigned long)y;
        }
    glGenTextures(2, tex_ids);
    glBindTexture(0x0DE1, tex_ids[0]);
    glTexImage2D(0x0DE1, 0, 0x1907, 256, 256, 0, 0x1907, 0x8363, tex565);
    glBindTexture(0x0DE1, tex_ids[1]);
    glTexImage2D(0x0DE1, 0, 0x1908, 256, 256, 0, 0x1908, 0x1401, tex8888);
    srand(7);
    for (i = 0; i < 2000 * 3; i++) {
        many_xy[i * 2] = (GLfixed)((rand() % 480) << 16);
        many_xy[i * 2 + 1] = (GLfixed)((rand() % 640) << 16);
        many_rgba[i * 4] = (unsigned char)rand();
        many_rgba[i * 4 + 1] = (unsigned char)rand();
        many_rgba[i * 4 + 2] = (unsigned char)rand();
        many_rgba[i * 4 + 3] = 255;
    }
    for (i = CLEAR; i <= MANY; i++) {
        unsigned long best = 0xFFFFFFFFUL, worst = 0, us, first = 0, last;
        unsigned long long t0;
        scene_setup((enum scene)i);
        for (f = 0; f < RENDERS; f++) {
            MSG msg;
            while (PeekMessageW(&msg, NULL, 0, 0, PM_REMOVE))
                DispatchMessageW(&msg);
            SystemIdleTimerReset();
            t0 = ticks();
            scene_draw((enum scene)i);
            glFinish();
            us = us_since(t0);
            if (us < best)
                best = us;
            if (us > worst)
                worst = us;
            if (f == 0 || f == RENDERS - 1)
                glReadPixels(0, 0, w, h, 0x1908, 0x1401, pic);
            if (f == 0)
                first = pic_sum(pic);
            eglSwapBuffers(dpy, surf);
        }
        last = pic_sum(pic);
        out("gl: %s: %lu to %lu us a frame (%d frames); the last frame %s the first", scene_name[i], best,
            worst, RENDERS, last == first ? "as" : "DIFFERS FROM");
    }
    {
        unsigned long long t0 = ticks();
        glReadPixels(0, 0, w, h, 0x1908, 0x1401, pic);
        out("gl: glReadPixels of the whole screen %lu us", us_since(t0));
        t0 = ticks();
        eglSwapBuffers(dpy, surf);
        out("gl: eglSwapBuffers %lu us", us_since(t0));
    }
    out("gl: GL error 0x%x", glGetError());
    gl_stop();
    free(pic);
}

/* ---- bench: rp-wmbench beside it ------------------------------------- */

static void module_bench(void)
{
    WCHAR exe[MAX_PATH], logp[MAX_PATH];
    PROCESS_INFORMATION pi;
    HANDLE f;
    DWORD before = 0, size, n;
    char *buf;
    _snwprintf(exe, MAX_PATH, L"%s\\rp-wmbench.exe", dir);
    _snwprintf(logp, MAX_PATH, L"%s\\rp-wm-baseline.txt", dir);
    f = CreateFileW(logp, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, NULL, OPEN_EXISTING, 0, NULL);
    if (f != INVALID_HANDLE_VALUE) {
        before = GetFileSize(f, NULL);
        CloseHandle(f);
    }
    WCHAR args[] = L"suite";
    if (!CreateProcessW(exe, args, NULL, NULL, FALSE, 0, NULL, NULL, NULL, &pi)) {
        out("bench: rp-wmbench.exe not started (error %lu): is it beside rp-wmprobe.exe?",
            (unsigned long)GetLastError());
        return;
    }
    WaitForSingleObject(pi.hProcess, 15 * 60 * 1000);
    CloseHandle(pi.hThread);
    CloseHandle(pi.hProcess);
    f = CreateFileW(logp, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, 0, NULL);
    if (f == INVALID_HANDLE_VALUE) {
        out("bench: no rp-wm-baseline.txt");
        return;
    }
    size = GetFileSize(f, NULL);
    if (size > before && (buf = (char *)malloc(size - before)) != NULL) {
        SetFilePointer(f, (LONG)before, NULL, FILE_BEGIN);
        ReadFile(f, buf, size - before, &n, NULL);
        out("bench: rp-wmbench's rows (%lu bytes) follow", (unsigned long)n);
        WriteFile(logf_, buf, n, &n, NULL);
        FlushFileBuffers(logf_);
        free(buf);
    } else
        out("bench: rp-wmbench wrote nothing");
    CloseHandle(f);
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
    out("rp-wmprobe v2 (Windows Mobile probe and baseline suite), tick %lu",
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
    out("== probes and baselines");
    module_env();
    module_pxa();
    module_2700g();
    module_reg();
    module_hop();
    module_vram();
    module_gl();
    module_bench();
    out("== module 1");
    divide1();
    out("== end");
    CloseHandle(logf_);
    return 0;
}
