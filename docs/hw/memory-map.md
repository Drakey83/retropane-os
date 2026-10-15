# X51v physical memory map

The physical windows Windows Mobile maps on the X51v. Source: the
statically mapped address table in Dell's OAL (`OEMAddressTable`: virtual base,
physical base, size in MiB), read from the A08 ROM (`DiAd_K_AximX51v_WM6_1_A08EN`,
file offset 0x17C8; identical in the copy at 0x3113C8). Block names come from
the PXA27x developer's manual memory map.

| Physical base | Size | WM virtual | Block | Confidence | Evidence |
|---|---|---|---|---|---|
| `0x00000000` | 64 MiB | `0x80000000` | CS0: boot ROM / DiskOnChip G3 | confirmed | OAL table; PXA manual |
| `0x04000000` | 64 MiB | `0x84000000` | CS1 | confirmed (mapped) | OAL table |
| `0x08000000` | 32 MiB | `0x94000000` | CS2 (static) | confirmed (mapped) | OAL table |
| `0x0C000000` | 64 MiB | `0x9C000000` | CS3: Intel 2700G (docs/hw/2700g.md) | confirmed | OAL table; 2700g.md |
| `0x10000000` | 1 MiB | `0x9B800000` | CS4: board CPLD (docs/hw/cpld.md) | confirmed | OAL table; cpld.md |
| `0x14000000` | 1 MiB | `0x9B900000` | CS5 | confirmed (mapped) | OAL table |
| `0x15000000` | 1 MiB | `0x9BA00000` | CS5 + 16 MiB | confirmed (mapped) | OAL table |
| `0x15100000` | 1 MiB | `0x9BB00000` | CS5 + 17 MiB | confirmed (mapped) | OAL table |
| `0x16000000` | 1 MiB | `0x9BC00000` | CS5 + 32 MiB | confirmed (mapped) | OAL table |
| `0x20000000` | 2 MiB | `0x96000000` | PC Card socket 0 I/O | confirmed (mapped) | OAL table; PXA manual |
| `0x28000000` | 8 MiB | `0x96200000` | PC Card socket 0 attribute | confirmed (mapped) | OAL table |
| `0x2C000000` | 4 MiB | `0x96A00000` | PC Card socket 0 memory | confirmed (mapped) | OAL table |
| `0x30000000` | 2 MiB | `0x96E00000` | PC Card socket 1 I/O | confirmed (mapped) | OAL table |
| `0x38000000` | 8 MiB | `0x97000000` | PC Card socket 1 attribute | confirmed (mapped) | OAL table |
| `0x3C000000` | 64 MiB | `0x97800000` | PC Card socket 1 memory | confirmed (mapped) | OAL table |
| `0x40000000` | 32 MiB | `0x88000000` | PXA27x peripherals | confirmed | OAL table; PXA manual |
| `0x44000000` | 1 MiB | `0x8A000000` | LCD controller | confirmed | OAL table; PXA manual |
| `0x48000000` | 1 MiB | `0x8A100000` | Memory controller | confirmed | OAL table; PXA manual |
| `0x4C000000` | 1 MiB | `0x8A200000` | USB host | confirmed | OAL table; PXA manual |
| `0x50000000` | 1 MiB | `0x8A300000` | Quick Capture (camera) | confirmed (mapped) | OAL table; PXA manual |
| `0x58000000` | 1 MiB | `0x8A400000` | Internal memory control | confirmed (mapped) | OAL table; PXA manual |
| `0x5C000000` | 1 MiB | `0x8A500000`, `0x9BD00000` | Internal SRAM (256 KiB) | confirmed | OAL table; PXA manual |
| **`0xA8000000`** | **64 MiB** | `0x8C000000` | **SDRAM (SDCS2)** | **confirmed** | OAL table; HaRET boots the X50/X51 with `ramaddr 0xa8000000`, `ramsize 64M` |
| `0xAC000000` | 64 MiB | `0x90000000` | SDCS3 | likely empty on 64 MB units | OAL table maps it; HaRET's RAM size says 64 MiB in total |
| `0xE0000000` | 4 MiB | `0x8A600000` | Not a PXA27x device window | unknown | OAL table (possibly a cache-clean area) |

## Consequences for RetroPane

- **SDRAM layout used by RetroPane's bare-metal images** (until the kernel
  owns memory): `0xA8000100` HaRET's boot tags; `0xA8001320` the CPLD's
  shared shadow table (40 bytes: Windows Mobile's at hand-over, RetroPane's
  after, cpld.md "Shadow at hand-over"); `0xA8004000` bring-up page
  table (16 KiB); `0xA8008000`–`0xA80F0000` the image; `0xA80F0000` vector
  page; `0xA80F1000` the KData page and `0xA80F2000` the top MiB's
  second-level table (1 KiB), once an image splits that MiB into pages
  (ADR-0010 rule 8); `0xA80F8000` cache-clean area (32 KiB); **`0xA8400000`–`0xA8500000`
  the resident loader** (ADR-0012); `0xA8500000` the run log (64 KiB); `0xA8510000` the trace ring (4 KiB, item 54);
  `0xA8800000` scratch (4 MiB, up to `0xA8C00000`);
  `0xA9000000` the kernel's RAM pool (48 MiB).
- **SDRAM starts at 0xA8000000, not at the PXA27x's bank 0 (0xA0000000).**
  Nothing is at 0xA0000000 on this board. Bare-metal images load at
  `0xA8008000`, the address HaRET copies a kernel to (`ramaddr` + 32 KiB), with
  its boot tags at `0xA8000100`. **Measured on a real X51v (2026-10-11):**
  HaRET's machine detection doesn't apply its X50/X51 entry to this device;
  it falls back to `ramaddr 0xA0000000` and copies the kernel into empty
  address space (its read-back there hangs the bus). The SD kit's
  `default.txt` therefore sets `ramaddr 0xa8000000` explicitly. The same run
  confirmed the visible framebuffer at physical `0x0C000000` (2700G memory). The emulator places its RAM to match
  (retropane-qemu `hw/arm/x51v.c`, rp-v0.6.0 and later).

## Open questions

- Is anything at `0xAC000000` (SDCS3)? A probe on real hardware will tell
  (128 MB units have been reported but not confirmed).
- What is the 4 MiB window at `0xE0000000`?
