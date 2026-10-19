/*
 * rp-wmtones: the A/B test's notes played by Windows Mobile through Dell's
 * own sound driver (docs/hw/audio.md, "A/B run": RetroPane's 440 Hz at
 * level 60 is unheard on the speaker, its 660 Hz heard). If Dell's 440 Hz
 * is heard where RetroPane's isn't, RetroPane's codec set-up is missing
 * something; if it's as faint, the speaker is the limit.
 *
 * Eight numbered tests, a big number on the screen before each 1 s note,
 * 5 s of quiet after it: 660 Hz and 440 Hz in pairs at Windows Mobile
 * volumes 100%, 50%, 25% and 12%. The notes are made as RetroPane makes
 * them: 44.1 kHz, 16-bit stereo, the same sample on both channels,
 * amplitude 12000 of 32767.
 *
 * Read-only: no hardware register is read (reading a clocked unit with its
 * clock off froze Windows Mobile once); only RAM (Windows Mobile's CPLD
 * shadow, to log whether the speaker amplifier was on) and the standard
 * wave API. No code or data from Windows Mobile is copied
 * (docs/CLEANROOM.md). The log, rp-wm-tones.txt, is written next to the
 * program (its own folder on the card: the card is shared).
 *
 * Built by `cargo xtask wmbench` (cegcc in WSL).
 */

#define _WIN32_WCE 0x0501
#include <windows.h>
#include <mmsystem.h>
#include <math.h>
#include <string.h>
#include <stdio.h>

BOOL VirtualCopy(LPVOID dest, LPVOID src, DWORD size, DWORD protect);

#define RATE 44100
#define AMPLITUDE 12000

static HANDLE logf_ = INVALID_HANDLE_VALUE;
static volatile unsigned long *shadow; /* 0xA8001000: WM's CPLD shadow at +0x320 */
static HWND win;
static HFONT big;
static WCHAR text[80];
static short note_buf[RATE * 2];

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

static volatile unsigned long *map_phys(DWORD phys, DWORD size)
{
    void *v = VirtualAlloc(NULL, size, MEM_RESERVE, PAGE_NOACCESS);
    if (!v)
        return NULL;
    if (!VirtualCopy(v, (void *)(phys >> 8), size, PAGE_READONLY | PAGE_NOCACHE | PAGE_PHYSICAL)) {
        VirtualFree(v, 0, MEM_RELEASE);
        return NULL;
    }
    return (volatile unsigned long *)v;
}

/* CPLD shadow slots (docs/hw/cpld.md): 3 = 0x14, 7 = 0x1C. */
static unsigned long slot(int k)
{
    return shadow ? shadow[(0x320 + 4 * k) / 4] & 0xFFFF : 0xFFFF;
}

static LRESULT CALLBACK wndproc(HWND h, UINT m, WPARAM w, LPARAM l)
{
    if (m == WM_PAINT) {
        PAINTSTRUCT ps;
        RECT r;
        HDC dc = BeginPaint(h, &ps);
        GetClientRect(h, &r);
        FillRect(dc, &r, (HBRUSH)GetStockObject(WHITE_BRUSH));
        SelectObject(dc, big);
        DrawTextW(dc, text, -1, &r, DT_CENTER | DT_VCENTER | DT_WORDBREAK);
        EndPaint(h, &ps);
        return 0;
    }
    return DefWindowProcW(h, m, w, l);
}

static void pump(void)
{
    MSG msg;
    while (PeekMessageW(&msg, NULL, 0, 0, PM_REMOVE)) {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

static void show(const WCHAR *s)
{
    wcsncpy(text, s, 79);
    text[79] = 0;
    InvalidateRect(win, NULL, TRUE);
    UpdateWindow(win);
    pump();
}

static void wait_ms(DWORD ms)
{
    DWORD t = GetTickCount();
    while (GetTickCount() - t < ms) {
        pump();
        SystemIdleTimerReset();
        Sleep(20);
    }
}

/* One note, 1 s, through the wave API at `pct` percent volume. */
static void play(int n, int hz, int pct)
{
    WAVEFORMATEX f;
    WAVEHDR h;
    HWAVEOUT wave;
    MMRESULT r;
    DWORD vol, got = 0;
    int i;
    double w = 2.0 * 3.14159265358979 * hz / RATE;
    for (i = 0; i < RATE; i++) {
        short s = (short)(AMPLITUDE * sin(w * i));
        note_buf[2 * i] = s;
        note_buf[2 * i + 1] = s;
    }
    memset(&f, 0, sizeof f);
    f.wFormatTag = WAVE_FORMAT_PCM;
    f.nChannels = 2;
    f.nSamplesPerSec = RATE;
    f.wBitsPerSample = 16;
    f.nBlockAlign = 4;
    f.nAvgBytesPerSec = RATE * 4;
    r = waveOutOpen(&wave, WAVE_MAPPER, &f, 0, 0, CALLBACK_NULL);
    if (r != MMSYSERR_NOERROR) {
        out("TEST %d: waveOutOpen failed %u", n, (unsigned)r);
        return;
    }
    vol = (DWORD)(0xFFFF * pct / 100);
    waveOutSetVolume(wave, vol | vol << 16);
    waveOutGetVolume(wave, &got);
    memset(&h, 0, sizeof h);
    h.lpData = (LPSTR)note_buf;
    h.dwBufferLength = sizeof note_buf;
    waveOutPrepareHeader(wave, &h, sizeof h);
    waveOutWrite(wave, &h, sizeof h);
    wait_ms(300);
    out("TEST %d: %d Hz at %d%% (volume set %08lx, read %08lx); CPLD 14=%04lx 1C=%04lx while playing",
        n, hz, pct, (unsigned long)(vol | vol << 16), (unsigned long)got, slot(3), slot(7));
    while (!(h.dwFlags & WHDR_DONE)) {
        pump();
        Sleep(20);
    }
    waveOutUnprepareHeader(wave, &h, sizeof h);
    waveOutClose(wave);
}

int WINAPI WinMain(HINSTANCE inst, HINSTANCE prev, LPWSTR cmd, int show_)
{
    static const int tests[8][2] = {
        {660, 100}, {440, 100}, {660, 50}, {440, 50}, {660, 25}, {440, 25}, {660, 12}, {440, 12},
    };
    WCHAR dir[MAX_PATH], logpath[MAX_PATH], line[80], *slash;
    WNDCLASSW wc;
    LOGFONTW lf;
    SYSTEMTIME st;
    int k;
    (void)prev; (void)cmd; (void)show_;

    GetModuleFileNameW(inst, dir, MAX_PATH);
    slash = wcsrchr(dir, L'\\');
    if (slash)
        *slash = 0;
    wsprintfW(logpath, L"%s\\rp-wm-tones.txt", dir);
    logf_ = CreateFileW(logpath, GENERIC_WRITE, 0, NULL, OPEN_ALWAYS, 0, NULL);
    if (logf_ == INVALID_HANDLE_VALUE) {
        MessageBoxW(NULL, L"Can't write the log next to the program.", L"rp-wmtones", MB_OK);
        return 1;
    }
    SetFilePointer(logf_, 0, NULL, FILE_END);
    GetLocalTime(&st);
    out("# rp-wmtones run %04d-%02d-%02d %02d:%02d:%02d", st.wYear, st.wMonth, st.wDay, st.wHour,
        st.wMinute, st.wSecond);
    shadow = map_phys(0xA8001000, 0x1000);

    memset(&wc, 0, sizeof wc);
    wc.lpfnWndProc = wndproc;
    wc.hInstance = inst;
    wc.lpszClassName = L"rpwmtones";
    RegisterClassW(&wc);
    memset(&lf, 0, sizeof lf);
    lf.lfHeight = 56;
    lf.lfWeight = FW_BOLD;
    big = CreateFontIndirectW(&lf);
    win = CreateWindowW(L"rpwmtones", L"rp-wmtones", WS_VISIBLE, 0, 0,
                        GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN), NULL, NULL,
                        inst, NULL);

    MessageBoxW(win,
                L"Headphones OUT. Tap OK, then watch the numbers: TEST 1 to TEST 8, "
                L"one note each, 5 seconds apart.",
                L"rp-wmtones", MB_OK);
    show(L"Get ready...");
    wait_ms(3000);
    for (k = 0; k < 8; k++) {
        wsprintfW(line, L"TEST %d", k + 1);
        show(line);
        wait_ms(500);
        play(k + 1, tests[k][0], tests[k][1]);
        wsprintfW(line, L"TEST %d done\nwrite it down", k + 1);
        show(line);
        wait_ms(5000);
    }
    out("# done");
    show(L"DONE");
    CloseHandle(logf_);
    MessageBoxW(win, L"Done. Thank you. Start the RetroPane loader now.", L"rp-wmtones", MB_OK);
    return 0;
}
