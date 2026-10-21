<!-- Static read of Windows Mobile's OAL (2026-10-15), hardware facts only (docs/CLEANROOM.md). Every value is a finding to work (docs/research/imitation-audit.md), not a decision: see ADR-0031. -->

# Clocks and core voltage under Windows Mobile (hardware facts)

Sources: `local/re/oal/nk.asm` / `nk.exe` (WM6.1 A08 OAL, VA base 0x8c200000). The table was also checked byte-for-byte in `rom-a08`, `rom-l11` and `rom-l16` .nb0: it is identical in all three.
Scratch disassembly was local to the session that did the read (not kept).
Clean-room: register values, addresses and ordering only. No code structure is reproduced.

## Address mappings seen (OAL virtual to physical)
| VA | Phys | Block | Confidence |
|---|---|---|---|
| 0xA8xxxxxx / 0xA9xxxxxx | 0x40xxxxxx / 0x41xxxxxx (+0x68000000) | peripherals (PM 0xA8F00000, GPIO 0xA8E00000, OST 0xA8A00000, ICU 0xA8D00000, clocks 0xA9300000) | high |
| 0xAA100000 | 0x48000000 | memory controller (MDCNFG +0, MDREFR +4, MSC0 +8, MCMEM0..MCIO1 +0x28..+0x3C) | high (the DRI mask 0xFFF and KxDB2 bit tests match MDREFR) |
| 0xAA000000 | 0x44000000 | LCD controller (LCCR0 +0, LCCR3 +0xC, LCCR4 +0x10) | medium-high (the bit usage matches LCCR3/LCCR4) |
| 0x8A536000 | internal SRAM (assumed) | The clock-change code and table are copied here at init (8c253194) | medium |

Note: MDREFR is phys **0x48000004**, not 0x4800000C (0x4800000C is MSC1). The OAL uses 0xAA100004.

## 1. Operating-point table (7 entries, 0x38 bytes each). RAM VA 0x8c506a78, nk.exe file offset 0x62278
Fields used: +0x00 CCCR, +0x04 CLKCFG (written OR 0x2 = F), +0x08 core voltage in mV (the voltage routine's key), +0x0C LCCR3, +0x10 LCCR4, +0x14 MDCNFG, +0x18 MDREFR, +0x1C MSC0, +0x20 MCMEM0, +0x24 MCMEM1, +0x28 MCATT0, +0x2C MCATT1, +0x30 MCIO0, +0x34 MCIO1.

| idx | CCCR | L / 2N / A | CLKCFG written | core MHz (turbo) | run MHz | mV field | LCCR4 | MDREFR | DRI |
|---|---|---|---|---|---|---|---|---|---|
| 0 | 0x00000107 | 7 / 2 / 0 | 0x2 (F) | 91 | 91 | 0x41A = 1050 | 0x80000000 | 0x201EE015 | 21 |
| 1 | 0x00000108 | 8 / 2 / 0 | 0x2 (F) | 104 | 104 | 0x41A = 1050 | 0x80000000 | 0x0014E018 | 24 |
| 2 | 0x02000110 | 16 / 2 / 1 | 0xB (T,F,B) | 208 | 208 | 0x48B = 1163 | 0 | 0x201EE030 | 48 |
| 3 | 0x02000190 | 16 / 3 / 1 | 0xB | 312 | 208 | 0x4F2 = 1266 | 0 | 0x201EE030 | 48 |
| 4 | 0x02000210 | 16 / 4 / 1 | 0xB | 416 | 208 | 0x5BE = 1470 | 0 | 0x201EE030 | 48 |
| 5 | 0x02000290 | 16 / 5 / 1 | 0xB | 520 | 208 | 0x5BE = 1470 | 0 | 0x201EE030 | 48 |
| 6 | 0x02000310 | 16 / 6 / 1 | 0xB | 624 | 208 | 0x626 = 1574 | 0 | 0x201EE030 | 48 |

The other fields are the same in every entry: LCCR3 0x04F00001. MDCNFG 0x0ACB2BCC.
- Indices 0-1: MSC0 0x12821898. MCMEM0 0x00010684, MCMEM1 0x000C0684, MCATT0/1 0x00010684, MCIO0/1 0x00010288.
- Indices 2-6: MSC0 0x128238D8. MCMEM0/1 0x00020508, MCATT0/1 0x00020E08, MCIO0/1 0x00020510.
- DRI × 32 / memory clock ≈ 7.4 µs at every point, so the refresh interval is held constant.
- HT (CLKCFG bit 2) is never set. B=1 and A=1 are used together for every point at 208 MHz and above.

| Fact | Value | Where | Conf. |
|---|---|---|---|
| MHz list the OAL reports for indices 0..6 | 91, 104, 208, 312, 416, 520, 624 (0x5B,0x68,0xD0,0x138,0x1A0,0x208,0x270) | 8c24aaa8-8c24aafc | high |
| CCCR writes | only one: `str` to 0xA9300000 at 8c258d54, immediately followed by CLKCFG = entry+4 OR 0x2 at 8c258d60. The code runs from its SRAM copy (0x8A536200) with IRQs off (8c20e288). | 8c258bb8-8c258e0c, copy at 8c253194 | high |
| Other CLKCFG writes | 8c25252c: CLKCFG=1 (T only). 8c252538: CLKCFG=0. No callers found. 8c258998 is a generic CLKCFG writer (copied to SRAM). | | high |
| 13 MHz mode (CPDIS/PPDIS, CCCR bits 31/26) | never used. No table value or CCCR write sets them. | | high |
| OSCC at startup | phys 0x41300008 = 2 (OON) | 8c2011b4 | high |
| Boot default index | from the board-ID register (VA 0xBB80001E bits 1:0, via 8c253724): type 1 → idx 5 (520), types 2/3 → idx 6 (624), else idx 4 (416) | 8c253b20 | medium |
| Highest index by model byte (0xAC00142F) | 0→3, 1→4, 2/3→5 (used by the "balanced" sub-command); the max index is stored at 0x8c53de74 | 8c24ab24, 8c24aca4 | medium |

## 2. Core voltage: GPIO VID pins, not I2C
| Fact | Value | Where | Conf. |
|---|---|---|---|
| Control method | three GPIO outputs: **GPIO3, GPIO4, GPIO38**, set and cleared through GPSR0/GPSR1/GPCR0/GPCR1 (0xA8E00018/1C/24/28) | function 8c253238 | high |
| mV to pin levels (GPIO38, GPIO4, GPIO3) | 1050 → 0,0,0. 1163 → 1,0,0. 1266 → 1,0,1. 1364 → 1,1,0. **1470 → 1,1,0** (same as 1364). 1467 → 1,1,1. 1574 → 1,1,1. Any other value: no change. | 8c253238-8c253340 | high (raw); the real rail voltage per code is unverified |
| PWR_I2C / PI2C (0x40F00180..1A0) | **not referenced** anywhere in nk.asm. PCFR is written with PI2C_EN (bit 6) clear (0x5031 and 0x81). GPIO3/4 are the PWR_SCL/PWR_SDA pins, used here as plain GPIOs. | grep, 8c24636c, 8c249874 | high |
| PVCR / PCMD0-31 / PWRMODE VC (0x8) | not referenced. PWRMODE values written are only 1, 3 and 7. | | high |
| Regular I2C (0x40301680) for the PMIC | no literal reference to 0xA8301680-0xA83016A0 in the OAL | | medium |
| PMIC part / slave address | **not found**: no I2C traffic and no part-number strings | | n/a |
| Order on a speed-up | if new mV > current: set the VID pins, wait 2000 units (8c253cc8 waits units×4 OSCR ticks at 3.25 MHz, so about 2.46 ms), then change frequency | 8c2533c8-8c2533f8 | high |
| Order on a slow-down | change frequency first, then lower the VID pins. No settle wait. | 8c253508-8c25355c | high |
| Equal mV | the VID pins are not touched | 8c2533d8 | high |
| Extra 60-unit wait (~74 µs) | before the change when the LCD-sync path was taken (r6) | 8c2533ec | medium |
| LCD around the change | LCCR3 = (old & 0xC0000000) or entry+0xC (0x04700001 if the panel/model query returns 3). LCCR4 = entry+0x10. Written after the frequency change on a speed-up and before it on a slow-down. A deferred path disables the LCD (LCCR0 & 0x07FFFFBF, ICMR bit 17), saves the index at 0xAC001447 and finishes in the LCD interrupt. | 8c253400-8c253500, 8c247258-8c2472b0 | medium |
| Before suspend | switches to index 0 (91 MHz), waits 20 × 4000 OSCR ticks (~24.6 ms), then restores the saved index byte | 8c2494a0-8c2494b8 | high |

## 3. Idle and sleep
| Fact | Value | Where | Conf. |
|---|---|---|---|
| Idle | PWRMODE=1 (8c24664c), called from OEMIdle at 8c249360. Sleep length is clamped to 30000 ms (0x7530); OSMR0 = OSCR + ms × 3250 (0xCB2 ticks/ms, so OSCR = 3.25 MHz); OIER bit 0 is set. The clock is not lowered on idle. | 8c24927c-8c249368 | high |
| Sleep | PWRMODE=3 in a loop (8c246424). Before it: PSPR = checksum; MDREFR &= 0xFFFFF000 (DRI=0); 32 KB D-cache clean (0x8A600000, 0x400 lines); PCFR=0x5031; PSLR=0xCC400004; PSSR=1; ICMR=0; 0x80000000 written to 0xA8000000-0xA800003C (DMA DCSR0-15); 0xD2 written to 0xBB900000 (board/debug port) | 8c246314-8c246424 | high |
| Deep sleep (PWRMODE=7) at 8c246894 | before it: PWER \|= 1; PRER \|= 0x7801; PFER \|= 0x5803; PEDR = 0x7803; then the same MDREFR, cache, PCFR=0x5031, PSLR=0xCC400004 and PSSR=1 sequence | 8c246720-8c246894 | high |
| "Power off" deep sleep (magic 'sOpF' = 0x46704F73 at 0xAC0013C4) | PFER bit 0 cleared; PCFR = 0x81; PSLR = 0xCC400004; PFER bit 1, PRER bit 1 and PWER bit 1 cleared; PWRMODE=7 | 8c249838-8c2498b0 | high |
| PCFR 0x5031 decode | bits 0 (OPDE), 4 (GPR_EN), 5, 12 (GPROD?), 14 (PO?). PI2C_EN and DC_EN are clear. Check the bit names against the PXA27x manual. | | medium |
| PSLR 0xCC400004 | raw value. Upper byte 0xCC (SYS_DEL/PWR_DEL fields), bit 22, bit 2 (SL_PI). Check the names against the manual. | | raw high, names medium |
| Normal-suspend wake set-up (PWER/PRER/PFER/PKWR) | computed at runtime from saved variables (8c25216c-8c252184; PKWR from 0xAC001058 with bit 10 conditionally cleared at 8c2495b0). **Values not extracted.** | | low |
| Debug/POST port | phys 0x14000000 receives 0xA1-0xA6 (exceptions), 0xAA-0xAC (boot) and 0xC3 (deep sleep) | 8c201020ff, 8c24688c | high |

## 4. Policy
| Fact | Value | Where | Conf. |
|---|---|---|---|
| Software interface | KernelIoControl 0x0101010D. Input DWORD: low byte = sub-command, bits 31:16 = argument. | AximSDK.dll Get/SetProcessorPowerMode; OAL dispatch table 8c24aa5c | high |
| Sub-commands | 0 = get mode (1/2/4, +0x10 if automatic). 1 = target idx 2. 2 = per-model middle idx (3/4/5). 3 = target = max idx. 4 = automatic on (0xAC001418=1, the cold-boot default). 5 = automatic off. 7 = set idx (arg) now. 8 = MHz of current idx. 9 = target = arg. | 8c24aa7c table | medium |
| Load thresholds / sampling period | **not found** in the OAL. Nothing reads the automatic flag outside the IOCTL. The automatic policy probably lives in a user-mode component that was not identified. | | n/a |

## 5. MDREFR handling on a frequency change (SRAM routine 8c258bb8)
1. New DRI = min(current DRI, the entry's DRI). If K0DB4 (bit 29) is clear in the current value and set in the entry, the entry's value is used as the base.
2. If the current value has K1DB2 or K2DB2 (mask 0xA0000) set and the new value has neither: first write DRI=0xFFF, read it back, wait 8 OSCR ticks (~2.5 µs), then write (base & 0xE3DFE000) OR DRI.
3. If the A bit goes 0→1: MDCNFG, MDREFR (same KxDB2 sequence), MSC0 and MCMEM/MCATT/MCIO are written **before** CCCR/CLKCFG. Otherwise they are written **after**.
4. MDREFR values: 0x201EE030 for 208 MHz and above (K0DB4, APD, K2DB2, K2RUN, K1DB2, K1RUN, E1PIN, K0DB2, K0RUN). 0x0014E018 at 104 MHz (no K1DB2/K2DB2, no K1RUN). 0x201EE015 at 91 MHz.

## Not found / open
- PMIC part, I2C slave address and register bytes: none exist in the OAL. Voltage is set by 3 GPIO VID pins. The voltage each pin pattern produces is unmeasured; the mV labels are Dell's numbers.
- Oddities: 1470 mV (idx 4/5) gives the same pin pattern as 1364; 1467 gives the same pattern as 1574.
- The automatic load-based policy constants.
- Normal-suspend wake-enable values (computed at runtime).
- The VA for 0x8A536000 and the board-ID register 0xBB80001E were not resolved through OEMAddressTable.
- `drivers-a08/PowerStateManager.dll` is HTC "Raphael" code (qtv_mp4_decoder / headset / A2DP hooks) and `battdrvr.dll` is an HTC-style SSP battery driver. Neither touches clock or voltage hardware: neither imports VirtualCopy/MmMapIoSpace, and battdrvr only uses KernelIoControl and GetIdleTime.

## 5. Measured on the X51v (`rp-clocks` run F, 2026-10-21)

`local/rp-clocks-runF1.txt`: found `CCCR` `0x02000310` (L 16, 2N 6),
`CCSR` `0x30000310`, `CLKCFG` `0xB`, the core counted 623,999 kHz.

- **Confirmed: a `CCCR` write alone changes nothing.** `CCCR` asking 2N 2
  (`0x02000110`): `CCSR` unchanged, the core counted 623,975 kHz.
- **Confirmed: the fault path's put-back keeps the running point** (C31)
  and, with C42's option b, leaves `CCCR` naming it (`0x02000310`).
- `CLKCFG` read `0xB` as Windows Mobile left it: bit 1 (F) reads back as
  last written. Written without F by a put-back, it reads `0x9`; the point
  (turbo, fast bus) is the same.
