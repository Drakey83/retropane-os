# Memory budget: the whole device, top-down

The per-image budgets in `xtask/src/size.rs` stop any one file growing
unseen. This page is the budget above them: how the X51v's **64 MB of RAM**
is shared, so the OS as a whole can't creep up on the apps. `cargo xtask
cross` (and so CI) checks it on every build.

## The rule

**The OS leaves apps at least 52 MB**: what Windows Mobile leaves free on
the owner's X51v (ADR-0005). So the OS's parts together get at most 12 MB.
Framebuffers, window surfaces and textures live in the 2700G's own 16 MB
(ADR-0005 rule 5) and don't count here.

| Part | Share | Built so far (budget) |
|---|---|---|
| Kernel image: code, data, static tables, boot stacks | 1 MiB | `rp-kernel`: 65 + 120 KiB |
| Kernel memory in use: page tables, compressed store, thread stacks | 1 MiB | measured at run time |
| Driver hosts: storage, display+input, audio, connectivity, power (ADR-0016) | 1 MiB | audio: the DMA ring ~44 KiB and a few hundred bytes per stream (ADR-0032 R10; programs' rings, ~50 KiB per stream, in their own budgets); connectivity grows a Bluetooth process, its budget set at ADR-0033 P-S2 |
| Service manager and system services (files, registry) | 1 MiB | |
| Window manager (GWES), shell, Today screen | 3 MiB | |
| Windows Mobile libraries (coredll, aygshell, GDI, GAPI): code shared by all apps | 2 MiB | |
| Networking: TCP/IP, TLS 1.3, Wi-Fi | 2 MiB | |
| Headroom | 1 MiB | |
| **OS total** | **12 MiB** | |
| **Apps (at least)** | **52 MiB** | |

What the check enforces (`check_allotments` in `xtask/src/size.rs`):

- the parts plus the apps' 52 MiB fit in 64 MiB;
- every OS image belongs to one part, and its budget (loaded + zeroed)
  counts against that part in full;
- the images in a part fit inside its share.

A part's share changes only with the owner's approval, like any OS budget,
and only by taking from another part or the headroom: the apps' floor
doesn't move.

## What keeps it possible

- Code is demand-paged from its file and shared between processes (ADR-0009):
  an app's untouched code costs nothing, a library's code exists once.
- Reclaim (item 41): pages untouched for a sweep are compressed or let go.
- Compressed RAM (ADR-0005 rule 6) instead of swap: no flash wear.
- Fixed-size kernel tables sized for the real limits (62 processes, 255
  threads), all zero at start, so they cost nothing in the image; handle
  tables beyond 8 handles grow into pages a process pays for, with a
  quota (docs/research/process-limit.md).
- Cold code built for size; Thumb where it helps (ADR-0015).

## Today (2026-10-12)

During bring-up the kernel runs under the resident loader, which keeps the
low 16 MiB, so the kernel image manages the top 48 MiB (12,281 frames free
after its own setup). Once RetroPane boots on its own, the whole 64 MiB is
its. The kernel image takes about 176 KiB of its 1 MiB share.

**The shell's interface layer (noted 2026-10-13, ADR truth pass):**
`rp-today` composes in a 480 x 640 ARGB8888 layer in SDRAM, 1.2 MiB
(ADR-0021), not yet counted in the shell's 3 MiB share above. ADR-0022
(proposed) would move GPU-drawn surfaces into the 2700G's own 16 MiB,
which takes this out of SDRAM; until it is decided the layer counts
against the window manager's share.

**Since 2026-10-14 (ADR-0023, option B):** `rp-today` draws with the 2D
engine and has no SDRAM layer; the 1.2 MiB is back. The 2700G's own
16 MiB, as `rp_mbx_render::Layout::X51V` lays it out:

| 2700G local memory | Offset | Size |
|---|---|---|
| Loader's / Windows Mobile's plane (left alone) | below 1 MiB | ~600 KiB |
| Picture 0 (XRGB8888, 480 x 640) | 0x200000 | 1.17 MiB |
| Picture 1 | 0x340000 | 1.17 MiB |
| Wallpaper (XRGB8888, converted once) | 0x480000 | 1.17 MiB |
| Row ring (64 rows x 2.5 KiB, stride-0 fills and gradients) | 0x5C0000 | 160 KiB |
| Image cache | 0x600000 | 256 KiB |
| Glyph atlases (per font and colour; the shell's take 35, 24 and 75 KiB each) | 0x640000 | 512 KiB |
| Background (wallpaper plus the glass, drawn once) | 0x6C0000 | 1.17 MiB |
| Dither pattern (4 rows) | 0x800000 | 7.5 KiB |
| Fence word | 0xFFF000 | 4 bytes |
| Area cache (finished areas reused: an opened menu is one copy; 8 areas; approved by the owner 2026-10-14) | 0x810000 | 1.17 MiB |
| Free for program surfaces (ADR-0022 decision 2) | 0x940000 up | ~6.8 MiB |

The SVC stack is 16 KiB in every image unless it sizes its own (`rp-today`: 40 KiB since 2026-10-17; it used 23.5 of its old 24); each now reports its deepest use
when it returns to the loader (`stack: deepest N of 16384 bytes` in the
run log), since a silent overflow lost a glyph atlas on 2026-10-14.
