# Dell Axim X51v — Hardware Inventory and Prior Work

> Compiled 2026-10-10 from web research. Confidence: **confirmed** (datasheet or
> working code), **likely** (reviews or multiple sources), **guess**.
> Board facts below come from AxDroid's working code, not from datasheets.

## Components

| Component | Part | Confidence | Interface / wiring | Docs | Prior drivers / RE |
|---|---|---|---|---|---|
| SoC | Intel PXA270, 624 MHz, ARMv5TE + iWMMXt, no FPU | confirmed | — | PXA27x Developer's Manual (Intel 280000-003) | Old Linux mach-pxa |
| RAM | 64 MB SDRAM (some reports say 128 MB) | likely | — | — | — |
| Flash | M-Systems DiskOnChip G3 at 0x00201000 (X50v: Intel StrataFlash) | confirmed (code) | Static memory bus | — | Linux `docg3` (Mio A701) |
| GPU | Intel 2700G7 (PowerVR MBX Lite + MVED1), 16 MB stacked memory | confirmed | CS3, registers at CS3+0x3FE0000 | Display/clock/SDRAM only (Linux `mbxfb`, removed 2020). **3D, 2D and video: undocumented** | No open 3D driver anywhere |
| LCD | 3.7" VGA (Sharp or Samsung); type read from CPLD reg 0x1E | confirmed (code) | Wired through the 2700G; can pass through PXA LCD output | — | `pxafb` passthrough worked in AxDroid |
| Touch | TI TSC2046 (ADS7846-compatible) | confirmed (code) | SSP1, CS GPIO24, pen IRQ GPIO94, X/Y swapped (spec: `docs/hw/touch.md`) | Public datasheet | Linux `ads7846` |
| Audio | Wolfson WM8750 | confirmed (code) | I²S GPIO28–31,113; I²C GPIO117/118; amp GPIO82 + CPLD 0x14; HP detect GPIO83 | Public datasheet | Linux ASoC `wm8750` |
| Wi-Fi | TI TNETW1100B (ACX100), 802.11b | confirmed (code) | Memory-mapped on PC Card socket 0 (0x2C000000); IRQ GPIO107; reset GPIO89; power via CPLD | **No datasheet** | acx-mac80211 (out of tree, RE'd); AxDroid glue; never worked well |
| Bluetooth 1.2 | Broadcom module (BCM2035 is a guess) | likely | BTUART GPIO42–45; power GPIO22 | — | Never worked in AxDroid |
| Keys | PXA matrix keypad 7×3; power button GPIO0 | confirmed (code) | PXA keypad controller | PXA manual | Keymap in AxDroid |
| Backlight | PWM0 on GPIO17 | confirmed (code) | PXA PWM | PXA manual | generic |
| SD | PXA MMC; CD GPIO12, WP GPIO27, power via CPLD | confirmed (code) | PXA native | PXA manual | `pxamci` |
| CompactFlash | PXA PC Card socket 1; detect GPIO13, ready / slot power GPIO84 (reset GPIO36 per AxDroid: disputed, docs/hw/cf.md) | confirmed (code) | PXA native | PXA manual | `pxa2xx_aximx50.c` |
| USB client | PXA UDC; cable detect GPIO14, pull-up GPIO2 | confirmed (code) | — | PXA manual | mainline (old) |
| IrDA | PXA STUART | confirmed (code) | — | PXA manual | — |
| CPLD | Unknown part; 16 × 16-bit regs at CS4; power rails and LCD ID (spec: `docs/hw/cpld.md`) | confirmed (code) | Static memory bus | **None** | Partial map in AxDroid (buggy clear logic) |
| Power / battery / charger | **Unknown**; only AC-in = GPIO11 known | — | — | **None** | **None** |

Manufacturer is likely **HTC** (FCC filing; the FCC ID NM8HC03U is a guess, unverified).

## Reverse-engineering targets (priority)

1. 2700G **MBX 3D**: tile accelerator, parameter formats, textures, IRQs (0% public)
2. 2700G **2D blitter** (0% public)
3. **ACX100** Wi-Fi: host interface and firmware loading (partially known from acx-mac80211)
4. **CPLD**: full register map
5. 2700G **MVED1** video decoder (0% public)
6. **Power/battery IC**
7. **Bluetooth** power sequencing and chip identity

## Key references

- AxDroid kernel (board file, GPIO map, CPLD, audio, CF): https://github.com/paulburton/axdroid-kernel
- AxDroid ACX100 driver + Axim glue: https://github.com/paulburton/axdroid-acx-mac80211
- AxDroid build scripts: https://github.com/paulburton/axdroid
- PXA27x Developer's Manual: https://courses.cs.washington.edu/courses/cse466/08wi/labs/l7/pxa27x_developers_manual.pdf
- `mbxfb` regs.h (last version): https://android.googlesource.com/kernel/common/+/ff729a0610eb/drivers/video/fbdev/mbx/regs.h
- `mbxfb` original patch: https://gitea.osmocom.org/dect/linux-2.6/commit/22caf04297896e515c6d5cdfb8e08a79a523946c
- Intel 2700G overview: https://en.wikipedia.org/wiki/Intel_2700G
- MBX block overview (NXP AN3723): https://www.nxp.com/docs/en/application-note/AN3723.pdf
- Dreamcast PowerVR2 (KallistiOS docs): https://cadcdev.sourceforge.net/docs/kos-current/group__pvr__commands.html
- Mesa PowerVR (Series 6+ only; does not cover MBX): https://docs.mesa3d.org/drivers/powervr.html
- Maemo MBX driver history: https://wiki.maemo.org/MBX_drivers_status
- 2700G GLES library is `libGLES_CL.dll` in ROM: https://forums.imgtec.com/t/why-my-x51v-no-libgles-cl-dll/200
- 2700G game list (DoomGLES etc.): http://www.etcwiki.org/wiki/Intel_2700g_games
- DoomGLES: http://kokak.free.fr/DoomGLES.htm
- Linux PXA board removal (2022–23): https://lore.kernel.org/all/m2o7r9gzoc.fsf@free.fr/T/
- PXA deprecation (Dec 2024): https://lkml.iu.edu/hypermail/linux/kernel/2412.0/04403.html
- PXA2xx DTS drop discussion (Dec 2025): https://lkml.iu.edu/hypermail/linux/kernel/2512.2/03732.html
- QEMU PXA deprecation commit: https://git.cs.tu-dortmund.de/SYS-OSS/FRET-qemu/commit/a2531bb855079597444f2ef1dcd4118d6609baf1
- Rust armv5te-none-eabi: https://doc.rust-lang.org/stable/rustc/platform-support/armv5te-none-eabi.html
- CeGCC (CE API headers reference): https://cegcc.sourceforge.net/docs/what.html
- wcecl (CE→NT API mapping PoC): https://github.com/gweslab/wcecl
- NetSurf on 32 MB (3DS port): https://github.com/coderman64/netsurf-3ds
- WRP (web rendering proxy): https://github.com/tenox7/wrp
- Browservice: https://github.com/ttalvitie/browservice
- MacProxy Plus: https://github.com/mactjaap/macproxy_plus
- Early X50 Linux boot (2005): https://www.mobileread.com/forums/showthread.php?p=18723

## ROM ecosystem (relevant to RE, recovery and flash install)

- The user's unit runs **LennySh WM6.1** (CE 5.2.20273), a community ROM from 2008 built on Dell's
  X51v ROM base (A06/A07) by rebuilding the XIP section: https://xdaforums.com/t/dell-axim-x51v-wm6-1-lennysh-edition.379287/
- The same thread lists other X51v ROMs: Dell WM5 A17/R01, WM6 A01/A03 and Makuu WM6.1 A06. So the
  X51v ROM format (XIP + IMGFS) was well understood, and reflashing via Dell's updater is routine.
- What this means for us:
  - **Recovery exists:** a known-good ROM image can always be reflashed. Before any flash
    experiments, keep a full backup of the user's current ROM.
  - **RE source:** the 2700G/MBX driver DLLs, `libGLES_CL.dll` and the ACX100 firmware live in these
    ROM images. `tools/romdump` can parse XIP/IMGFS from the user's own device or update file.
    The images themselves are never committed (see CLEANROOM.md).
  - **Confirmed 2026-10-10:** DoomGLES runs with hardware acceleration on the user's LennySh
    WM6.1 ROM, so the 2700G GLES stack is intact. RE tracing can use the current ROM, with no
    reflash needed.
- Download mirrors from 2008 (Rapidshare) are likely dead.

## Dell driver modules in the X51v ROM (RE subjects)

Found by name in LennySh WM6.1 A08 and L11 `.nb0` images, both cooked from
Dell's X51v ROM (from LennySh's archive, 2026-10-10). The images live only in
the git-ignored `local/` folder.

| Hardware | Module(s) | Notes |
|---|---|---|
| 2700G display / 2D | `ddi_2700g.dll` | Display driver (DDI). Main source for display planes and the 2D blitter |
| 2700G 3D | `libGLES_CL.dll`, `d3d_2700g.dll` (+ `d3dm.dll`) | OpenGL ES 1.x Common-Lite and Direct3D Mobile for MBX |
| 2700G video (MVED1) | `wmv9decoder2700G.dll`, `pvrvadd.dll` | Hardware WMV9 decode; PowerVR video acceleration |
| Wi-Fi (ACX100) | `tiacxwln.dll`, `ACX100_dl.bin` | TI driver, plus firmware image (user-extracted only, never redistributed) |
| Bluetooth | `BTuartDMA.dll`, `bthcsr.dll`, `bthuart.dll`, `BTControl.dll` | `bthcsr.dll` hints at a **CSR** BlueCore chip, not Broadcom. Verify |
| Power / battery | `battdrvr.dll`, `PowerStateManager.dll`, `PowerExe.exe` | Main source for the unknown power/charger IC |
| Board / CPLD | `AximSDK.dll` | Dell's board API; AxDroid learned CPLD bits from it |
| Unknown | `maxim_radio_dl.bin` | A firmware blob, possibly for a Maxim radio/RF part. Investigate |
| Games API | `gx.dll` | GAPI (compatibility reference) |

All of these extract cleanly with `tools/romdump`. The A08 and L11 builds carry the **same
driver binaries**: identical link timestamps, differing only in the load address each
ROM cook assigned.

## The user's device (known)

- **RAM:** 64 MB (52.2 MB available in WM6.1).
- **ROM:** LennySh WM6.1 (CE 5.2.20273), cooked from Dell's X51v ROM. Matching images
  (A08 and L11) are in `local/` for analysis.
- **SD:** a 4 GB card works. The ROM's XIP partition contains `pxa27x_sdhc.dll`
  (the SD host driver) and `sdbus.dll`/`sdmemory.dll`.
- **Connection:** stock USB cradle only. **No serial cable is used** (DESIGN.md §10b).

Board details such as the LCD panel type (which the CPLD reports in register 0x1E) are
read at runtime by RetroPane and are documented from Dell's drivers (`ddi_2700g.dll`,
`AximSDK.dll`) as `romdump` extracts them.
