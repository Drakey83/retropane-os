/*
 * rp-wmaudio: Windows Mobile's audio state while it plays a sound
 * (docs/hw/audio.md, quality-audit A4). RetroPane's own tone test found no
 * codec on I2C twice (2026-10-18), with the codec's power bit, its I2C
 * gate, the I2S pins and its master clock set as Dell's driver sets them.
 * So this program measures what Windows Mobile has when audio works:
 *
 * 1. The headphone jack: the candidate lines (GPIO96, GPIO83) and all GPIO
 *    levels with headphones in, then out (the owner plugs, a box says when).
 * 2. Every audio-related register, read-only: GPIO levels, directions and
 *    functions; the clock enables; the I2S controller (never its data
 *    register: a read would take a sample); both I2C units (standard and
 *    power); Windows Mobile's CPLD shadow (the CPLD can't be read back).
 *    Before a sound, while one plays (headphones, then speaker), after it,
 *    and 10 s later (Windows Mobile powers audio down when idle).
 * 3. While Windows Mobile opens the sound device and starts the tone, a
 *    high-priority thread samples both I2C units and the I2C gate (GPIO82)
 *    once a millisecond (sleeping in between, so the sound driver, which
 *    runs at lower priority, is never starved) and logs every change:
 *    which bus the driver uses, the bytes it leaves in the data register,
 *    and the gate around them.
 *
 * It only reads registers (mapped read-only, uncached) and plays its own
 * quiet tone through the standard wave API; no code or data from Windows
 * Mobile is copied (docs/CLEANROOM.md). The log, rp-wm-audio.txt, is
 * written next to the program (its own folder on the card: the card is
 * shared) and flushed after every phase.
 *
 * Built by `cargo xtask wmbench` (cegcc in WSL).
 */

#define _WIN32_WCE 0x0501
#include <windows.h>
#include <mmsystem.h>
#include <string.h>
#include <stdio.h>

BOOL VirtualCopy(LPVOID dest, LPVOID src, DWORD size, DWORD protect);

static HANDLE logf_ = INVALID_HANDLE_VALUE;

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

static void flush(void)
{
    FlushFileBuffers(logf_);
}

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

/* The blocks (PXA27x developer's manual; docs/hw). Page-aligned bases. */
static volatile unsigned long *gpio;    /* 0x40E00000 */
static volatile unsigned long *clk;     /* 0x41300000 */
static volatile unsigned long *i2s;     /* 0x40400000 */
static volatile unsigned long *i2c;     /* 0x40301000: the unit at +0x680 */
static volatile unsigned long *pwr;     /* 0x40F00000: the unit at +0x180 */
static volatile unsigned long *shadow;  /* 0xA8001000: WM's CPLD shadow at +0x320 */

#define R(p, off) ((p) ? (p)[(off) / 4] : 0xDEADDEADul)

/*
 * A unit whose clock is off hangs the bus when read (2026-10-18: the first
 * version froze Windows Mobile on its first snapshot, audio idle, I2S and
 * I2C clocks off). So every read of a clocked unit asks CKEN first:
 * I2S bit 8, I2C bit 14, power I2C bit 15. The GPIO block, the clock
 * manager and RAM are always readable.
 */
#define CKEN_I2S (1ul << 8)
#define CKEN_I2C (1ul << 14)
#define CKEN_PI2C (1ul << 15)

static unsigned long clock_bit(volatile unsigned long *const *block)
{
    if (block == &i2s)
        return CKEN_I2S;
    if (block == &i2c)
        return CKEN_I2C;
    if (block == &pwr)
        return CKEN_PI2C;
    return 0;
}

/* The register, or 0xC10CC0FF ("clock off") without touching the unit. */
static unsigned long read_reg(volatile unsigned long *const *block, DWORD off)
{
    unsigned long bit = clock_bit(block);
    if (bit && !(R(clk, 0x004) & bit))
        return 0xC10CC0FFul;
    return R(*block, off);
}

/* Registers read in every snapshot: block, offset, name. Only registers
 * with no read side effect (no data FIFOs). */
struct reg {
    volatile unsigned long **block;
    DWORD off;
    const char *name;
};

static const struct reg REGS[] = {
    {&gpio, 0x000, "GPLR0"}, {&gpio, 0x004, "GPLR1"}, {&gpio, 0x008, "GPLR2"},
    {&gpio, 0x100, "GPLR3"}, {&gpio, 0x00C, "GPDR0"}, {&gpio, 0x010, "GPDR1"},
    {&gpio, 0x014, "GPDR2"}, {&gpio, 0x10C, "GPDR3"}, {&gpio, 0x054, "GAFR0_L"},
    {&gpio, 0x058, "GAFR0_U"}, {&gpio, 0x05C, "GAFR1_L"}, {&gpio, 0x060, "GAFR1_U"},
    {&gpio, 0x064, "GAFR2_L"}, {&gpio, 0x068, "GAFR2_U"}, {&gpio, 0x06C, "GAFR3_L"},
    {&gpio, 0x070, "GAFR3_U"}, {&clk, 0x000, "CCCR"}, {&clk, 0x004, "CKEN"},
    {&clk, 0x008, "OSCC"}, {&i2s, 0x000, "SACR0"}, {&i2s, 0x004, "SACR1"},
    {&i2s, 0x00C, "SASR0"}, {&i2s, 0x014, "SAIMR"}, {&i2s, 0x060, "SADIV"},
    {&i2c, 0x680, "I2C IBMR"}, {&i2c, 0x688, "I2C IDBR"}, {&i2c, 0x690, "I2C ICR"},
    {&i2c, 0x698, "I2C ISR"}, {&i2c, 0x6A0, "I2C ISAR"}, {&pwr, 0x180, "PI2C IBMR"},
    {&pwr, 0x188, "PI2C IDBR"}, {&pwr, 0x190, "PI2C ICR"}, {&pwr, 0x198, "PI2C ISR"},
    {&pwr, 0x1A0, "PI2C ISAR"},
};
#define NREGS (sizeof REGS / sizeof REGS[0])

/* WM's CPLD shadow: slot i holds register CPLD_REG[i] (docs/hw/cpld.md). */
static const unsigned CPLD_REG[10] = {0x02, 0x10, 0x12, 0x14, 0x16, 0x18, 0x1A, 0x1C, 0x1E, 0x08};

static int gpio_level(unsigned pin)
{
    static const DWORD plr[4] = {0x000, 0x004, 0x008, 0x100};
    return (int)((R(gpio, plr[pin / 32]) >> (pin % 32)) & 1);
}

static void snapshot(const char *phase)
{
    unsigned k;
    out("== %s (CKEN %08lx; c10cc0ff = clock off, not read)", phase, R(clk, 0x004));
    for (k = 0; k < NREGS; k++)
        out("  %-10s %08lx", REGS[k].name, read_reg(REGS[k].block, REGS[k].off));
    for (k = 0; k < 10; k++)
        out("  CPLD %02x   %08lx", CPLD_REG[k], R(shadow, 0x320 + 4 * k));
    out("  GPIO82 %d  GPIO83 %d  GPIO96 %d  GPIO113 %d  GPIO117 %d  GPIO118 %d",
        gpio_level(82), gpio_level(83), gpio_level(96), gpio_level(113), gpio_level(117),
        gpio_level(118));
    flush();
}

/* ---- The bus watcher ------------------------------------------------- */

#define WATCH_MAX 400
struct seen {
    DWORD t_us;
    unsigned long icr, isr, idbr, picr, pisr, pidbr, gplr2;
};
static struct seen watch[WATCH_MAX];
static volatile LONG watched;
static volatile int watching;
static LARGE_INTEGER freq, t0;

static DWORD us_now(void)
{
    LARGE_INTEGER t;
    QueryPerformanceCounter(&t);
    return (DWORD)((t.QuadPart - t0.QuadPart) * 1000000 / freq.QuadPart);
}

static DWORD WINAPI watcher(LPVOID arg)
{
    struct seen last, now;
    (void)arg;
    memset(&last, 0xFF, sizeof last);
    while (watching) {
        now.icr = read_reg(&i2c, 0x690);
        now.isr = read_reg(&i2c, 0x698);
        now.idbr = read_reg(&i2c, 0x688);
        now.picr = read_reg(&pwr, 0x190);
        now.pisr = read_reg(&pwr, 0x198);
        now.pidbr = read_reg(&pwr, 0x188);
        /* GPIO82 (the gate), 83 and 95-ish lines live in GPLR2. */
        now.gplr2 = R(gpio, 0x008);
        if (memcmp(&now.icr, &last.icr, sizeof now - sizeof now.t_us) != 0) {
            LONG k = watched;
            if (k < WATCH_MAX) {
                now.t_us = us_now();
                watch[k] = now;
                watched = k + 1;
            }
            last = now;
        }
        Sleep(1);
    }
    return 0;
}

/* ---- The tone ---------------------------------------------------------- */

#define RATE 22050
#define SECS 2
static short pcm[RATE * SECS * 2];

static void make_tone(void)
{
    /* 441 Hz square-ish wave at about -12 dB of full scale: loud enough to
     * judge by ear at volume 50% (the first version's -30 dB at 12.5% was
     * inaudible: quality-audit C8). */
    unsigned k;
    for (k = 0; k < RATE * SECS; k++) {
        short v = (short)(((k / 25) & 1) ? 8000 : -8000);
        pcm[2 * k] = v;
        pcm[2 * k + 1] = v;
    }
}

static void play(const char *what, int watch_open)
{
    WAVEFORMATEX f;
    WAVEHDR hdr;
    HWAVEOUT wo;
    HANDLE th = NULL;
    MMRESULT r;
    LONG k;

    memset(&f, 0, sizeof f);
    f.wFormatTag = WAVE_FORMAT_PCM;
    f.nChannels = 2;
    f.nSamplesPerSec = RATE;
    f.wBitsPerSample = 16;
    f.nBlockAlign = 4;
    f.nAvgBytesPerSec = RATE * 4;

    if (watch_open) {
        watched = 0;
        watching = 1;
        QueryPerformanceCounter(&t0);
        th = CreateThread(NULL, 0, watcher, NULL, 0, NULL);
        if (th)
            SetThreadPriority(th, THREAD_PRIORITY_TIME_CRITICAL);
        Sleep(50);
    }
    snapshot(what);
    r = waveOutOpen(&wo, WAVE_MAPPER, &f, 0, 0, CALLBACK_NULL);
    out("# %s: waveOutOpen %u at %lu us", what, (unsigned)r, (unsigned long)us_now());
    if (r != MMSYSERR_NOERROR) {
        watching = 0;
        flush();
        return;
    }
    waveOutSetVolume(wo, 0x80008000);
    memset(&hdr, 0, sizeof hdr);
    hdr.lpData = (LPSTR)pcm;
    hdr.dwBufferLength = sizeof pcm;
    waveOutPrepareHeader(wo, &hdr, sizeof hdr);
    waveOutWrite(wo, &hdr, sizeof hdr);
    out("# %s: waveOutWrite at %lu us", what, (unsigned long)us_now());
    Sleep(400);
    if (watch_open) {
        watching = 0;
        if (th) {
            WaitForSingleObject(th, 2000);
            CloseHandle(th);
        }
        out("# bus watcher: %ld changes (std I2C ICR/ISR/IDBR, power I2C ICR/ISR/IDBR, GPLR2)",
            (long)watched);
        for (k = 0; k < watched; k++)
            out("  %7lu us  I2C %08lx %08lx %02lx  PI2C %08lx %08lx %02lx  GPLR2 %08lx (GPIO82 %lu)",
                (unsigned long)watch[k].t_us, watch[k].icr, watch[k].isr, watch[k].idbr & 0xFF,
                watch[k].picr, watch[k].pisr, watch[k].pidbr & 0xFF, watch[k].gplr2,
                (watch[k].gplr2 >> 18) & 1);
        flush();
    }
    {
        char name[64];
        _snprintf(name, sizeof name, "%s, playing", what);
        snapshot(name);
    }
    while (!(hdr.dwFlags & WHDR_DONE))
        Sleep(50);
    waveOutUnprepareHeader(wo, &hdr, sizeof hdr);
    waveOutClose(wo);
    {
        char name[64];
        _snprintf(name, sizeof name, "%s, closed", what);
        snapshot(name);
    }
}

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
    wsprintfW(logpath, L"%s\\rp-wm-audio.txt", dir);
    logf_ = CreateFileW(logpath, GENERIC_WRITE, 0, NULL, OPEN_ALWAYS, 0, NULL);
    if (logf_ == INVALID_HANDLE_VALUE) {
        MessageBoxW(NULL, L"Can't write the log next to the program.", L"rp-wmaudio", MB_OK);
        return 1;
    }
    SetFilePointer(logf_, 0, NULL, FILE_END);
    GetLocalTime(&st);
    out("# rp-wmaudio run %04d-%02d-%02d %02d:%02d:%02d", st.wYear, st.wMonth, st.wDay, st.wHour,
        st.wMinute, st.wSecond);
    flush();

    gpio = map_phys(0x40E00000, 0x1000);
    clk = map_phys(0x41300000, 0x1000);
    i2s = map_phys(0x40400000, 0x1000);
    i2c = map_phys(0x40301000, 0x1000);
    pwr = map_phys(0x40F00000, 0x1000);
    shadow = map_phys(0xA8001000, 0x1000);
    make_tone();

    snapshot("start (audio idle)");

    MessageBoxW(NULL, L"Plug the headphones IN now, then tap OK.", L"rp-wmaudio 1/4", MB_OK);
    Sleep(300);
    snapshot("headphones in, no sound");
    play("tone 1, headphones in (quiet)", 1);

    MessageBoxW(NULL, L"Now UNPLUG the headphones, then tap OK. A quiet tone plays on the speaker.",
                L"rp-wmaudio 2/4", MB_OK);
    Sleep(300);
    snapshot("headphones out, no sound");
    play("tone 2, speaker (quiet)", 1);

    MessageBoxW(NULL, L"Leave it for 10 seconds (tap OK and wait).", L"rp-wmaudio 3/4", MB_OK);
    Sleep(10000);
    snapshot("10 s after the sound (audio idle again)");

    out("# done");
    flush();
    CloseHandle(logf_);
    MessageBoxW(NULL, L"Done. Thank you. You can start the RetroPane loader now.",
                L"rp-wmaudio 4/4", MB_OK);
    return 0;
}
