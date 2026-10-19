/*
 * rp-wmpower: what an idle, powered audio path costs in battery current,
 * measured under Windows Mobile (ADR-0032 R5 and S1; the owner's
 * condition 5: idle power-down between sounds is built only if this says
 * it's worth it).
 *
 * RetroPane has no battery gauge yet (ADR-0031 step c), but Windows Mobile
 * reports one: GetSystemPowerStatusEx2 gives the battery's current, average
 * current and voltage (if Dell's battery driver fills them in; the log says
 * if it doesn't). The program runs on battery, screen on, untouched, in
 * phases of 40 s, each twice:
 *
 *   A  the sound device closed (audio idle, as Windows Mobile leaves it);
 *   B  the sound device open and playing silence (codec powered, I2S and
 *      DMA running: the "audio up but quiet" state of RetroPane's baseline,
 *      plus the DMA's own cost);
 *   C  the sound device open, nothing written (whatever Windows Mobile
 *      powers for an open device).
 *
 * Every second it logs the power status and Windows Mobile's CPLD shadow
 * (0x1C bit 1 = codec power, 0x14 bit 1 = speaker amplifier: docs/hw/
 * cpld.md), so each phase shows what was really powered. Per phase, the
 * mean current over its last 30 samples.
 *
 * Read-only: no hardware register is read (reading a clocked unit with its
 * clock off froze Windows Mobile once: docs/hw/README.md); only RAM (the
 * shadow) and the system's own power API. Plays nothing audible (zeros).
 * No code or data from Windows Mobile is copied (docs/CLEANROOM.md). The
 * log, rp-wm-power.txt, is written next to the program (its own folder on
 * the card: the card is shared) and flushed every second.
 *
 * Built by `cargo xtask wmbench` (cegcc in WSL).
 */

#define _WIN32_WCE 0x0501
#include <windows.h>
#include <mmsystem.h>
#include <string.h>
#include <stdio.h>

BOOL VirtualCopy(LPVOID dest, LPVOID src, DWORD size, DWORD protect);
void SystemIdleTimerReset(void);

/* Windows CE's SYSTEM_POWER_STATUS_EX2, under our own name (the headers'
 * copy may differ or be missing); the call is looked up at run time. */
typedef struct {
    BYTE ACLineStatus;
    BYTE BatteryFlag;
    BYTE BatteryLifePercent;
    BYTE Reserved1;
    DWORD BatteryLifeTime;
    DWORD BatteryFullLifeTime;
    BYTE Reserved2;
    BYTE BackupBatteryFlag;
    BYTE BackupBatteryLifePercent;
    BYTE Reserved3;
    DWORD BackupBatteryLifeTime;
    DWORD BackupBatteryFullLifeTime;
    DWORD BatteryVoltage;          /* mV */
    LONG BatteryCurrent;           /* mA, negative = discharging */
    LONG BatteryAverageCurrent;    /* mA */
    DWORD BatteryAverageInterval;  /* ms */
    DWORD BatterymAHourConsumed;
    DWORD BatteryTemperature;      /* 0.1 degrees C */
    DWORD BackupBatteryVoltage;
    BYTE BatteryChemistry;
} RpPowerEx2;

typedef DWORD (*GetPowerEx2)(RpPowerEx2 *, DWORD, BOOL);

static HANDLE logf_ = INVALID_HANDLE_VALUE;
static GetPowerEx2 get_power;
static volatile unsigned long *shadow; /* 0xA8001000: WM's CPLD shadow at +0x320 */

/* Slot i holds CPLD register CPLD_REG[i] (docs/hw/cpld.md). */
#define SLOT_14 3
#define SLOT_1C 7

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

static unsigned long slot(int k)
{
    return shadow ? shadow[(0x320 + 4 * k) / 4] : 0xFFFFFFFFUL;
}

/* Silence: four half-second buffers at 44.1 kHz, 16-bit stereo. */
#define RATE 44100
#define BUF_FRAMES (RATE / 2)
static short silence[4][BUF_FRAMES * 2];
static WAVEHDR hdr[4];
static HWAVEOUT wave;

static int open_wave(void)
{
    WAVEFORMATEX f;
    MMRESULT r;
    memset(&f, 0, sizeof f);
    f.wFormatTag = WAVE_FORMAT_PCM;
    f.nChannels = 2;
    f.nSamplesPerSec = RATE;
    f.wBitsPerSample = 16;
    f.nBlockAlign = 4;
    f.nAvgBytesPerSec = RATE * 4;
    r = waveOutOpen(&wave, WAVE_MAPPER, &f, 0, 0, CALLBACK_NULL);
    if (r != MMSYSERR_NOERROR) {
        out("# waveOutOpen failed: %u", (unsigned)r);
        return 0;
    }
    return 1;
}

static void queue_all(void)
{
    int k;
    for (k = 0; k < 4; k++) {
        memset(&hdr[k], 0, sizeof hdr[k]);
        hdr[k].lpData = (LPSTR)silence[k];
        hdr[k].dwBufferLength = sizeof silence[k];
        waveOutPrepareHeader(wave, &hdr[k], sizeof hdr[k]);
        waveOutWrite(wave, &hdr[k], sizeof hdr[k]);
    }
}

/* Puts every played buffer back in the queue (never runs dry: 2 s queued,
 * checked every 250 ms). */
static void requeue(void)
{
    int k;
    for (k = 0; k < 4; k++)
        if (hdr[k].dwFlags & WHDR_DONE)
            waveOutWrite(wave, &hdr[k], sizeof hdr[k]);
}

static void close_wave(int prepared)
{
    int k;
    waveOutReset(wave);
    if (prepared)
        for (k = 0; k < 4; k++)
            waveOutUnprepareHeader(wave, &hdr[k], sizeof hdr[k]);
    waveOutClose(wave);
}

#define PHASE_S 40
#define SETTLE_S 10

/* One phase: PHASE_S seconds, a sample a second; the mean current of the
 * samples after SETTLE_S. */
static void phase(const char *name, int playing)
{
    long sum = 0, avg_sum = 0;
    int n = 0, t, q;
    for (t = 0; t < PHASE_S; t++) {
        RpPowerEx2 p;
        DWORD got;
        for (q = 0; q < 4; q++) {
            if (playing)
                requeue();
            SystemIdleTimerReset(); /* screen and device stay on */
            Sleep(250);
        }
        memset(&p, 0, sizeof p);
        got = get_power ? get_power(&p, sizeof p, TRUE) : 0;
        out("%s t=%2d ac=%u pct=%3u mV=%4lu mA=%5ld avg_mA=%5ld (over %lu ms) 1C=%04lx 14=%04lx%s",
            name, t, p.ACLineStatus, p.BatteryLifePercent, (unsigned long)p.BatteryVoltage,
            (long)p.BatteryCurrent, (long)p.BatteryAverageCurrent,
            (unsigned long)p.BatteryAverageInterval, slot(SLOT_1C) & 0xFFFF,
            slot(SLOT_14) & 0xFFFF, got ? "" : " (no power status)");
        if (t >= SETTLE_S) {
            sum += p.BatteryCurrent;
            avg_sum += p.BatteryAverageCurrent;
            n++;
        }
        flush();
    }
    out("%s MEAN over %d s: mA=%ld avg_mA=%ld", name, n, n ? sum / n : 0, n ? avg_sum / n : 0);
    flush();
}

static void run_phase(const char *name)
{
    if (name[0] == 'A') {
        phase(name, 0);
    } else if (name[0] == 'B') {
        if (!open_wave()) {
            out("%s skipped", name);
            return;
        }
        queue_all();
        phase(name, 1);
        close_wave(1);
    } else {
        if (!open_wave()) {
            out("%s skipped", name);
            return;
        }
        phase(name, 0);
        close_wave(0);
    }
}

int WINAPI WinMain(HINSTANCE inst, HINSTANCE prev, LPWSTR cmd, int show)
{
    static const char *const order[] = {"A1", "B1", "C1", "A2", "B2", "C2", "A3"};
    WCHAR dir[MAX_PATH], logpath[MAX_PATH], *slash;
    SYSTEMTIME st;
    HMODULE core;
    RpPowerEx2 p;
    unsigned k;
    (void)prev; (void)cmd; (void)show;

    GetModuleFileNameW(inst, dir, MAX_PATH);
    slash = wcsrchr(dir, L'\\');
    if (slash)
        *slash = 0;
    wsprintfW(logpath, L"%s\\rp-wm-power.txt", dir);
    logf_ = CreateFileW(logpath, GENERIC_WRITE, 0, NULL, OPEN_ALWAYS, 0, NULL);
    if (logf_ == INVALID_HANDLE_VALUE) {
        MessageBoxW(NULL, L"Can't write the log next to the program.", L"rp-wmpower", MB_OK);
        return 1;
    }
    SetFilePointer(logf_, 0, NULL, FILE_END);
    GetLocalTime(&st);
    out("# rp-wmpower run %04d-%02d-%02d %02d:%02d:%02d", st.wYear, st.wMonth, st.wDay, st.wHour,
        st.wMinute, st.wSecond);
    core = LoadLibraryW(L"coredll.dll");
    get_power = core ? (GetPowerEx2)GetProcAddressW(core, L"GetSystemPowerStatusEx2") : NULL;
    out("# GetSystemPowerStatusEx2: %s", get_power ? "found" : "MISSING");
    shadow = map_phys(0xA8001000, 0x1000);
    flush();

    for (;;) {
        memset(&p, 0, sizeof p);
        if (get_power)
            get_power(&p, sizeof p, TRUE);
        if (p.ACLineStatus != 1)
            break;
        if (MessageBoxW(NULL,
                        L"The Axim is on outside power. Take it out of the cradle (unplug USB and "
                        L"the charger), then tap OK. Cancel stops.",
                        L"rp-wmpower", MB_OKCANCEL) != IDOK) {
            out("# stopped: on outside power");
            CloseHandle(logf_);
            return 0;
        }
    }
    MessageBoxW(NULL,
                L"On battery. Headphones OUT. Tap OK, then put the Axim down and don't touch it "
                L"for about 5 minutes (the screen stays on). A box says when it's done.",
                L"rp-wmpower", MB_OK);
    for (k = 0; k < sizeof order / sizeof order[0]; k++)
        run_phase(order[k]);
    out("# done");
    CloseHandle(logf_);
    MessageBoxW(NULL, L"Done. Thank you. Put it back in the cradle and start the RetroPane loader.",
                L"rp-wmpower", MB_OK);
    return 0;
}
