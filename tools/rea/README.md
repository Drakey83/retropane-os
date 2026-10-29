# REA with Ghidra (R82)

[REA](https://github.com/morluto/rea) drives Ghidra's decompiler for the
agent: a standing tool (the owner, 2026-10-28: use it whenever it helps),
first for reading Dell's Windows CE drivers (hardware facts only, the
clean-room rule; output under `local/`, never committed).

- **Linux (WSL) only for Dell's drivers.** REA 6.8.0 on Windows refuses PE
  machine `0x01C2` (Windows CE's ARM/Thumb); in WSL it opens them as ARM.
- `setup.sh` installs JDK 21, Node 24.22.1, Ghidra 12.1.4 and REA 6.8.0 in
  `~/re`, each checked against its published SHA-256; no root.
- `rea.sh` runs REA's CLI with them. One query costs about 30 s (Ghidra
  analyses the file each time; a snapshot barely helps), so for a whole
  driver run `decompile-all` once (`ddi_2700g.dll`: 869 functions in 54 s)
  and search the files.
- Not registered as an MCP server: its 229 tools would sit in every
  session's context; the CLI costs nothing until used.

First use, 2026-10-28: `ddi_2700g`'s doubling code (`0x021270B8`, found by hand
with `llvm-objdump`, 2700g.md) decompiled in 28 s, the same facts as the
hand reading and the instructions, plus the function around it
(`0x02126c90`, the mode set), its callers and where the two flags come from.

## What is decompiled (2026-10-28), and the index

Every Dell / HTC driver that touches hardware is decompiled once, in
`local/re/ghidra-c/<module>/functions/` (one C file a function): the 2700G
drivers (`ddi_2700g` from two ROMs, `libGLES_CL`, `d3d_2700g`, `pvrvadd`,
the WMV9 decoder), the display and game-graphics layer (`ddi`, `gx`,
`gxdma`), Quake 3's GLES and EGL, Windows Mobile's kernel and Dell's OAL
(`nk.exe`), and the board drivers (backlight, battery, keypad, LED,
notification, CF socket, peripheral, SD, serial, touch, Wi-Fi, Bluetooth,
USB, audio, HTC's display service, `AximSDK`). Questions are answered by
searching those files, never by decompiling again.

`python tools/rea/xref.py` (rerun after any new `decompile-all`) writes
`local/re/xref.md` and `xref.json`: per module and function, the board
CPLD registers it touches through Windows Mobile's shared shadow (with each
set / clear helper's call sites translated to register and bits), the
PXA270 blocks it names by address (registers in a block's first 4 KiB, or
a block mapped by a call: float constants like `0x40490FDB`, pi, are
left out), its `KernelIoControl` codes, and in the GPU drivers every
literal in the 3D (`0x600`-`0x6FC`) and TA (`0x800`-`0x87C`) blocks with
its line. Check: it reproduces the REA pass of 2026-10-28 by hand
(`ddi_2700g` maps the clocks and memory controller only; the panel's
`0x12` bits 1, 2, 4, 8 stepped by `0x2110568`).

## Comparing two versions: `fdiff.py` (BinDiff's job, no download)

BinDiff 8 needs BinExport, whose newest Ghidra plugin targets Ghidra
11.0.3 (ours is 12.1.4). `python tools/rea/fdiff.py <module A> <module B>
--out local/re/<name>.md` does the job on REA's output instead: every
function normalized (REA's header dropped, every Ghidra name carrying an
address masked, long hex literals masked), identical bodies matched first,
the rest paired by similarity, a unified diff for each changed one. First
use, 2026-10-28: `ddi_2700g` from ROM A08 against L16, **868 of 869
functions identical**, the last a decompiler cast only: Dell changed
nothing in the 2700G display driver between those ROMs.

## Running Dell's functions on the PC: `dellrun.py` (R91)

Unicorn Engine 2.1.4 (installed by `setup.sh` in `~/re/unicorn-venv`, its
wheel checked against PyPI's SHA-256) runs one function of a driver DLL
from `local/` at its preferred base: the DLL's imports answered by stubs
that log each call (0 unless told otherwise), hardware windows that log
every read and write in order with the instruction's address, the
caller's structures built in a scratch heap. Dell's register sequences and
the words it builds then come from its own code, not from reading C by
hand. Run in WSL:

    ~/re/unicorn-venv/bin/python -I tools/rea/dellrun.py check

The self-check runs `ddi_2700g`'s `0x02118414` (the corners of Dell's
textured rectangle) on a known rectangle and checks each word where
`rp_drv_mbx3d::tex::quad` puts it. First run, 2026-10-28: right, corners
bottom left, top left, bottom right, top right; TEX0 `0xA3200000` for 64
x 32 (the rectangular size codes, bits 27-24 and 23-20). As a library,
`Dell(path)`, `.mmio(name, base, size, reads)`, `.alloc`, `.put`, `.call`,
`.log`. Clean room as ever: Dell's bytes stay in `local/`, logs go there.

## Watching Dell's driver live: HaRET's `MMUTRACE`

HaRET (on the card; source in `local/haret-src`, `ko/src/l1trace.cpp`)
traces accesses to chosen virtual addresses while Windows Mobile runs:
`set MMUTRACE <start> <size> rw` then `wirq <seconds>` makes every access
in those ranges fault; HaRET emulates and logs each one (address, value,
the instruction's PC) and returns. Section (1 MiB) mappings only unless
`PERMISSIVEMMUTRACE` is set; every traced access costs a fault. Use: a
GLES game under Windows Mobile with the 2700G's register window traced, to
record the register sequences Dell's drivers write, in order (a device
run, the owner starting HaRET). The script is ready:
`tools/haret/trace-2700g.txt` (R92; its header says what it traces and
why), copied to the card beside `haret.exe` with an empty
`earlyharetlog.txt` so the log lands in `haretlog.txt`.
