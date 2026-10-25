# Memory and bus timings: what Dell set, what the parts allow (2026-10-15)

The owner's brief (item 68, docs/research/imitation-audit.md): is the X51v's
memory running at its full capability, or at Dell's conservative defaults?
**Report only. Nothing was changed on the device.** The owner decides
whether anything proceeds, after seeing these numbers.

Sources: (D) the device, `rp-probe` loaded by the resident loader on
2026-10-15 (build `c6ea6bf0+`, every line's checksum valid), compared with
the first probe on 2026-10-11 (docs/hw/probe-2026-10-11.md). (M) the
Intel PXA27x Developer's Manual, 280000-003, chapter 6 and table 3-7.
(E) the PXA270 Electrical, Mechanical and Thermal Specification (EMTS),
tables 6-7 (SDRAM) and 6-20 (VLIO). (S) the SDRAM datasheet: Infineon
HY[B/E]25L256160A[F/C]-7.5, rev 1.3, 2004-11. (W) the handhelds.org wiki
page "DellAximX50Hardware" (2007, Wayback Machine), the only published
board-level chip markings. (L) Linux `mbxfb` (v4.19) for the 2700G's
local-memory registers.

Confidence: **confirmed** (read on the device, or in a datasheet for a
known part), **likely** (strong evidence, not proven), **guess**.

## Owner's decisions (2026-10-19)

- **SDRAM timings stay at Dell's values.** The rule behind it: RetroPane
  must work the best it can on **any** X51v, not one unit. The margin is
  ~1% on a part only *likely* fitted, other units may carry other chips
  (SDRAM has no ID register), and the owner won't open the device to read
  markings. Alternative 2 is closed; alternatives 7 (touch less memory)
  stay the way to faster memory.
- **The 2700G bus experiment (alternative 6) goes ahead:** `rp-cs3bus`
  (written 2026-10-15, not yet run) on the owner's go. Any setting it
  finds is used only with margin for every unit: a fixed value at least
  two steps looser than the tightest that passed, or a per-device
  calibration at boot with Dell's value as the fallback (the owner's
  choice once the results are in).

## `rp-cs3bus` on the device (2026-10-19, `local/rp-cs3bus-run1.txt`)

CS3 stepped from Windows Mobile's `MSC1` (`0x24842481`), one notch at a
time, each step checked (a full-screen flush verified word by word,
256 KiB read back, six patterns over 512 KiB, a register, a 2D fill):

| Step | RDF / RDN | Clocks a beat | Flush (1.2 MB) | Read 256 KiB | Result |
|---|---|---|---|---|---|
| 0 (WM) | 8 / 4 | 17 | 36.9 ms, 33 MB/s | 21.5 ms, 12 MB/s | all good |
| 1 | 7 / 4 | 16 | 35.5 ms, 34 MB/s | 21.5 ms | all good |
| 2 | 6 / 4 | 15 | 35.4 ms, 34 MB/s | 21.5 ms | all good |
| 3 | 5 / 4 | 14 | 34.4 ms, 35 MB/s | 21.5 ms | all good |
| 4 | 4 / 4 | 13 | 31.6 ms | (wrong data) | **reads wrong** (the flush's words, re-read at WM's timing, were right: a read fault); the 2D engine's fence never read back |
| soak at step 1 | 7 / 4 | 16 | 231 rounds in 60 s | | **0 misses** |

- **The 2700G sets the pace, not `MSC1`:** three clocks off a beat
  bought ~7% on writes and nothing on reads (12 MB/s throughout), so the
  2700G holds RDY; the hoped-for 60-75 MB/s isn't there.
- **Its read strobe needs at least 6 clocks** (`RDF` 5) on this unit;
  writes still landed at `RDF` 4.
- **With the any-Axim margin** (two steps looser than the tightest pass:
  step 1) the gain is **~4%** (a flush 36.9 -> 35.5 ms).
- Open for the owner: keep Windows Mobile's value (recommended: the gain
  is small and proven on one unit) or step 1 as a fixed value.

## The answer in brief

- **The SDRAM clock is already at the PXA270's maximum.** SDCLK2 =
  104 MHz (9.6 ns, EMTS `tsdCLK` minimum). There is no clock headroom.
  The controller can't run SDRAM faster, whatever the chips allow.
- **Dell picked the slowest SDRAM timing class (CL3, tRP/tRCD 3).** The
  likely chips are rated for the next class up (CL2, tRP/tRCD 2), but only
  just: the margins are about **1%** at 104 MHz. The best case is about
  **3-8% faster on memory-bound work.**
- **The refresh rate is within 4% of the chips' rated minimum.** Dell is
  not over-refreshing, and the possible gain is nil.
- **The 2700G's bus (`MSC1`, CS3) is set about 2× slower than the PXA's
  fastest legal setting.** This is the only big possible gain:
  framebuffer writes from **~41 MB/s to perhaps 60-75 MB/s**, which would
  make a full-screen flush **~14.5 ms → ~8-10 ms**. This holds only if the
  2700G doesn't stretch the cycles itself with RDY, and its datasheet is
  not public, so only a measurement can tell.
- **The real memory-speed wins are elsewhere** (alternatives below).
  RetroPane's SDRAM miss costs ~490 ns against WM's ~280 ns, and most of
  that difference is TLB walks, not timings. *(2026-10-24: not like with like: the two chases
  walk different patterns, and RetroPane's runs had no TLB misses at all;
  CPU V7's `RP_PROBE_WMHOP` runs Windows Mobile's own programs.)* Large mappings and page tables
  in SRAM are worth more than any timing change, at no risk.
- **Found on the way:** today Windows Mobile left the core at **624 MHz
  with `CCCR[A]` = 1** (memory clock 208 MHz). The manual's table 3-7 does
  not list that combination; its 624 MHz row has `A` = 0. Also,
  `MDREFR[DRI]` counts memory clocks. A clock change (item 67) that halves
  the memory clock without rewriting DRI would **double the refresh
  interval to 15 µs and silently corrupt memory**.

## 1. What is running (D, decoded with M)

### Clocks

| Register | 2026-10-11 | 2026-10-15 | Decoded |
|---|---|---|---|
| `CCCR` | `0x02000290` | `0x02000310` | `L` = 16 (run 208 MHz); `2N` = 5 then **6** (turbo 520 then **624** MHz); **`A` = 1: memory clock = system bus** |
| `CLKCFG` | `0xB` | `0xB` | turbo on, fast bus (`B` = 1): system bus 208 MHz |
| `CCSR` | `0x30000290` | `0x30000310` | matches `CCCR` (both PLLs locked) |

So **CLK_MEM = 208 MHz (4.81 ns)**. Both runs use the same memory clock;
only the core multiplier differs. `memcpy_64k` measured **687 µs at both
520 and 624 MHz**, so copying is bound by memory, not by the core.

Table 3-7 (M) lists 520 MHz with `A` = 1 (C5 stepping; `CP15 ID
0x69054117` is C5). It lists 624 MHz only with `A` = 0, meaning CLK_MEM
104 MHz and SDCLK 104 MHz undivided. WM's 624 + `A` = 1 point works on this
unit. Whether Intel's specification update allows it is **open**, and
item 67 has to settle it before RetroPane picks a 624 MHz set-up.

### SDRAM controller

**`MDCNFG 0x0ACB2BCC`** (same on both days). The RAM is on partition pair
2/3 (SDCS2 at `0xA8000000`, docs/hw/memory-map.md), so the upper half
applies:

| Field | Value | Meaning |
|---|---|---|
| `DE2` / `DE3` | 1 / 1 | partitions 2 **and 3** enabled (only 64 MB fitted: partition 3 at `0xAC000000` is likely empty, see below) |
| `DWID2` | 0 | 32-bit bus |
| `DCAC2` / `DRAC2` / `DNB2` | 9 / 13 / 4 banks | 512 columns × 8192 rows × 4 banks × 4 bytes = **64 MiB per partition**: exactly two 256 Mbit ×16 chips |
| **`DTC2`** | **0b10** | **tRP 3, CL 3, tRCD 3, tRAS 7, tRC 10 SDCLKs** (tWR fixed at 2) |
| `DTC0` (unused pair 0/1) | **0b11** | CL 3. **The controller uses the larger CL of the two pairs**, so CL2 needs `DTC0` changed too |
| `STACK1:0` | 0b01 | SDRAM address on `MA<24:23,13:1>` (the manual's mode for 16-bit flash on partition 0) |
| `SETALWAYS`, `DADDR2`, `DSA1110_2`, `MDENX` | 1, 0, 0, 0 | normal addressing, 256 MB map |

**`MDREFR 0x201EE030`** (same on both days):

| Field | Value | Meaning |
|---|---|---|
| **`DRI`** | **48** | refresh every 48 × 32 + 31 = **1567 CLK_MEM = 7.53 µs** |
| `K2RUN`, **`K2DB2`** | 1, 1 | **SDCLK2 = CLK_MEM / 2 = 104 MHz** (the RAM's clock) |
| `K1RUN`, `K1DB2` | 0, 1 | SDCLK1 (pair 0/1) off |
| `K0RUN`, `K0DB2`, `K0DB4` | 1, 1, 1 | SDCLK0/3 for synchronous flash at CLK_MEM / 4, running although `SXCNFG` enables none (`0x40044004`, `SXEN` = 0) |
| `E1PIN`, `APD` | 1, 1 | clock enable on, auto power-down on |
| `SLFRSH`, `KxFREE`, `ALTREF*` | 0 | |

**`MDMRS 0x00320032`**: CL 3, sequential, burst 4. On the PXA27x the burst
is always 4 (M §6.4.10: "always programmed to sequential and four"), so
burst length is not a knob.

**`MDMRSLP 0xC0000000`** (read for the first time, added to the probe
whitelist for this report): `MDLPEN2` = 1 and the value `0x4000`. Through
the `STACK` = 01 mapping that is likely BA1 = 1, BA0 = 0: the extended
mode register with every field 0. That means **full-array self-refresh,
TCSR on the on-chip temperature sensor, no partial array** (likely).

### Static memory (MSC0-2; S = CLK_MEM cycles, 4.81 ns)

`RDF` and `RDN` codes 12-15 decode to 15, 20, 26 and 30 (M table 6-28).

| CS | Device (memory-map.md) | Half | Type | Width | RDF (dec.) | RDN | RRR | What the timing means |
|---|---|---|---|---|---|---|---|---|
| 0 | DiskOnChip G3 (boot) | `0x38D8` | non-burst ROM/flash | 16 | 13 (**20**) | 8 | 3 | first read 22 clk = **106 ns**; 7-clk recovery |
| 1 | mapped, unknown | `0x1282` | burst-of-4 ROM | 32 | 8 | 2 | 1 | first 10 clk, next 3 clk |
| 2 | mapped, unknown | `0x2481` | SRAM | 32 | 8 | 4 | 2 | |
| **3** | **Intel 2700G** | **`0x2484`** | **VLIO** | **32** | **8** | **4** | **2** | **nOE/nPWE asserted ≥ 9 clk (+ RDY waits), released 8 clk: ≥ 17 clk = 81.7 ns per 32-bit beat** |
| 4 | board CPLD | `0x7FFC` | VLIO | 16 | 15 (**30**) | 15 (**30**) | 7 | the slowest setting there is: ~91 clk = 0.44 µs per access |
| 5 | mapped, unknown | `0x7FF4` | VLIO | 32 | 15 (30) | 15 (30) | 7 | slowest |

`RBUFF` = 0 everywhere (slow-device buffering), `MECR 0x3` (two PC Card
sockets, card present). The PC Card timings belong to the CF timing brief
(sibling session `cf-timing`), not to this report.

### The 2700G's own memory (D, decoded with L)

| Register | Value | Decoded |
|---|---|---|
| `LMTYPE` | `0xAC8` | **CL 2**, 4 banks, 12 row / 8 column bits on 32 bits = 16 MiB |
| `LMTIM` | `0x73392` | tRAS 7, tRP 3, tRCD 3, tRC 9, tDPL 2 (in 2700G memory clocks) |
| `LMREFRESH` | `0x615` | half of Linux mbxfb's `0xC2B` (refresh twice as often, or a different clock) |

These are identical to the values published for an X50v under WinCE (W).
Note that **Dell runs the 2700G's memory at CL2 where Linux uses CL3**. So
Dell was not uniformly conservative.

**The 2700G's clock (found by the MBX 3D session, `rp-mbx3d` run 26).**
`COREPLL 0xBB1` gives 13 MHz × 23 / 3 = **99.7 MHz**. Dell's default
`CoreClock` literal is 133 MHz; the X51v's settings evidently pick ~100.
Dell's display driver (`ddi_2700g`, `0x02120160`-`0x021201e8`) chooses
the PXA-side bus timing for CS3 from that clock:

| 2700G clock | CS3 timing |
|---|---|
| ≥ 99 MHz | `0x2484` / `0x2481` (today's `MSC1`) |
| 25-99 MHz | `0x79D4` / `0x7DF1` |
| < 25 MHz | `0x7DF4` / `0x7FF1` |

So `MSC1` is a clock-dependent table, not one fixed choice. Dell gave the
fastest tier the same timing up to 133 MHz.

**Confirmed from Dell's driver (MBX 3D session, 2026-10-15, docs/hw/2700g.md
commit 18a430b on `claude/mbx3d-render-2`).** The 2700G's system clock is
its local SDRAM's clock. Dell's driver (`0x0211fb98`) computes `LMTIM`,
`LMREFRESH` and `LMTYPE` from it, choosing CL by clock thresholds. The
X51v's memory table (`0x0213F93C`, the only one of Dell's ten that gives
WM's `LMTIM 0x73392` with CL2) rates the part as follows:
- tRAS 48 ns, tRP 25 ns, tRCD 25 ns, tRC 85 ns;
- CL3 from 100 MHz, at most 100 MHz.

Dell clamps the 2700G's clock to that limit, hence 99.67 MHz. At 10.03 ns,
WM's settings give:

| | WM's setting | Rated | Margin |
|---|---|---|---|
| tRCD, tRP | 3 clk = 30.1 ns | 25 ns | 20% |
| tRAS | 7 clk = 70 ns | 48 ns | 46% |
| tRC | 9 clk = 90 ns | 85 ns | 6% |
| CL | 2 | 2, just under the 100 MHz switch to CL3 | - |

**The 2700G's memory is at its rated clock limit: no headroom.** Raising
`COREPLL` was withdrawn.

**The 75 MHz option (the owner's GPU clock choice A).** 75 MHz falls in
Dell's 25-99 MHz tier, where CS3 is set to `0x79D4`. That decodes to VLIO,
32-bit, `RDF` 13 (decoded 20), `RDN` 9, `RRR` 7: 21 + 18 = **39 memory
clocks per beat (187 ns, ~21 MB/s ceiling)**, 2.3× slower than the fast
tier. Dell evidently gives the 2700G's host interface longer strobes when
its clock is slower. So at 75 MHz the safe starting point is `0x79D4`, not
the fast tier's `0x2484`, and the legal-minimum steps of §7 are less
likely to hold.

## 2. The physical chips

| Part | Identification | Confidence |
|---|---|---|
| SDRAM | **Infineon HYB25L256160AC-7.5**, two chips (256 Mbit ×16, 4 banks, 13 row / 9 col, 2.5 V Mobile-RAM, 54-ball BGA) | **confirmed on the X50** (W, chip markings); **likely on the X51v**: the geometry in `MDCNFG` matches exactly, and the X50 and X51 share model numbers (HC03U/HD04U). A later unit may carry a second-source part (Samsung K4S56163L*, Micron MT48H16M16LF, Hynix): **guess** |
| Boot flash | DiskOnChip G3 on CS0 (memory-map.md); X50 boards carry Intel L18 StrataFlash `4000L0YBQ0` (W) | X51v markings unknown |
| 2700G memory | 16 MiB, 32-bit (from `LMTYPE`). Whether it is stacked in the package (2700G7) or a separate chip beside a 2700G5 is unresolved: public sources disagree | open |

No FCC internal photos (grantee E2K) and no Dell service manual with a
parts list were found. **The owner can settle the SDRAM question by
reading the two BGA RAM chips' markings** (on the X50 they sit next to
the larger Xilinx CPLD, XC2C128), and the marking of any chip next to the
2700G. This matters before any SDRAM change: the margins below are 1%,
and a second-source part with tCK2 = 10 ns would make CL2 illegal.

Rated timings, HYB25L256160AC-7.5 (S):

| | tCK CL2 | tCK CL3 | tRCD | tRP | tRAS | tRC | tWR | Refresh |
|---|---|---|---|---|---|---|---|---|
| Rated | ≥ 9.5 ns (105 MHz) | ≥ 7.5 ns (133 MHz) | ≥ 19 ns | ≥ 19 ns | 45 ns-100 µs | ≥ 67 ns | ≥ 14 ns | 8192 rows / 64 ms = 7.81 µs |

Self-refresh current IDD6 per chip (S, table 12) at 45 / 70 °C: all banks
250 / 475 µA, half 150 / 250 µA, quarter 100 / 150 µA. The extended mode
register supports PASR (4, 2, 1, ½ or ¼ bank) and TCSR, and has no
drive-strength field.

## 3. Actual against rated: the headroom per field

SDRAM at SDCLK = 104 MHz (9.615 ns per clock):

| Field | Dell (`DTC` 10) | Rated minimum | Headroom | Reachable step (`DTC` 01) | Margin left there |
|---|---|---|---|---|---|
| Clock | 104 MHz | chip 133 MHz; **PXA270 max 104 MHz** | **none (PXA limit)** | - | - |
| CL | 3 | 2 at ≤ 105 MHz | 1 clock | CL 2 (9.615 ≥ 9.5 ns) | **1.2%** |
| tRCD | 3 = 28.8 ns | 19 ns | 1 clock (34%) | 2 = 19.2 ns | **1.2%** |
| tRP | 3 = 28.8 ns | 19 ns | 1 clock (34%) | 2 = 19.2 ns | **1.2%** |
| tRAS | 7 = 67.3 ns | 45 ns | 2 clocks | 5 = 48.1 ns | 7% |
| tRC | 10 = 96.2 ns | 67 ns | 3 clocks | 8 = 76.9 ns | 15% |
| tWR | 2 = 19.2 ns (fixed) | 14 ns | - | 2 | 37% |
| Refresh | 7.53 µs (`DRI` 48) | 7.81 µs | **3.6%** | `DRI` 49 = 7.69 µs | 1.6% |

`DTC` 00 (tRCD 1, tRAS 3, tRC 4) breaks the datasheet by about 2×, so `DTC`
01 is the only legal step. **Dell is not detuning grossly.** CL3 at
104 MHz is the setting that works for every 256 Mbit part a factory might
fit (any -8 grade part needs it), and the refresh is set to the rated
interval. The one thing Dell gave up is one clock of CAS, tRCD and tRP,
which this exact part allows by a hair.

2700G bus (CS3, VLIO; E table 6-20, CLK_MEM 4.81 ns):

| Field | Dell | PXA minimum for VLIO | Headroom |
|---|---|---|---|
| Strobe asserted (`RDF`+1) | 9 clk (43 ns) | 4 clk (`RDF` ≥ 3) | 5 clk |
| Strobe released (`RDN`×2: address setup/hold) | 8 clk (38 ns) | 4 clk (`RDN` ≥ 2) | 4 clk |
| CS gap (`RRR`×2+1) | 5 clk | 1 clk | 4 clk |
| Per 32-bit beat (no RDY waits) | **17 clk = 81.7 ns → 49 MB/s ceiling** | **8 clk = 38.5 ns → 104 MB/s ceiling** | 2.1× |

Measured: 41 MB/s for CPU `stm` bursts (docs/perf.md), 30-33 MB/s by PXA
DMA. That is about 84% of Dell's configured ceiling, so ~3 clocks per beat
go to RDY waits, CS gaps and write-buffer gaps. **The 2700G's own minimum
strobe and setup times are unknown** (no public datasheet). How long the
2700G holds RDY decides what any change is worth.

## 4. What it could buy, in numbers

All against docs/perf.md and today's probe (`c6ea6bf0+`, 624 MHz):

| Change | Today | Estimate | How the estimate is built |
|---|---|---|---|
| SDRAM `DTC` 10 → 01 (CL2, tRCD/tRP 2) | `memcpy_64k` 687 µs (~95 MB/s); `chase_sdram_256k` 2.01 ms (491 ns per hop) | memcpy **~3-8% faster** (~98-103 MB/s); chase **~6% faster** (~460 ns/hop) | a 32-byte line is two 4-beat bursts; a read saves 1 clock of CAS, plus 2 clocks when the row is closed: ~1-2 of ~17.5 SDCLKs per line moved today (190 MB/s of traffic). A random miss saves ≤ 3 clocks = 28.8 ns of ~490 ns |
| `DRI` 48 → 49 | refresh takes ~1.3% of SDRAM time | **0.03% bandwidth**; refresh power ~7 mW at most (IDD5 × tRC / interval, 2 chips), so **~0.15 mW saved** | negligible: rejected |
| PASR ¼ in sleep (data packed into bank 0 first) | full array in self-refresh | **~0.75-1.6 mW less in sleep** (2 chips × 150-325 µA × 2.5 V, 45-70 °C) | only matters once RetroPane sleeps (item 67); nothing to gain while awake |
| 2700G `MSC1` toward the minimum | full-screen flush 14.5 ms (~41 MB/s) | if the 2700G adds no waits: **60-75 MB/s, flush ~8-10 ms**; if it holds RDY: **no gain** | same ~3 clocks of overhead per beat on an 8-clock beat; must be measured |
| CPLD (CS4), CS5 | slowest settings | nothing worth it | accessed a few times per key press |
| Flush by DMA / the GPU drawing in its own memory (items 69, 70) | | the CPU is free; GPU fills ~190 MB/s inside the 2700G | not a timing change: listed because it beats any CS3 change for drawing |

## 5. Alternatives considered

1. **Keep Dell's settings** (status quo). It costs 3-8% of memory-bound
   speed and carries zero risk. **Recommended for SDRAM for now.**
2. **SDRAM `DTC` 01.** It is legal for the confirmed X50 part with a 1.2%
   margin on three timings, and it is unconfirmed for the X51v's chips.
   The gain is small, and the failure mode is silent corruption that may
   depend on temperature or unit. Only after the chip markings are read,
   and only behind the stress plan (§7).
3. **Raise the SDRAM clock** (`K2DB2` = 0: 208 MHz). **Impossible**: the
   PXA270's SDCLK minimum period is 9.6 ns (E), and the chip's limit is
   133 MHz. The rumoured X50v 150 MHz overclock (W) is outside both specs.
   Rejected.
4. **Fewer refreshes.** At most 2% fewer, worth nothing. Rejected.
5. **PASR/TCSR in sleep.** A real but small battery gain. The kernel must
   pack live pages into the refreshed banks before sleeping, because the
   other banks' contents are lost by design. **Fold into item 67's sleep
   design.**
6. **Tune the 2700G's bus (`MSC1`).** The largest gain on the table, and
   the failure is mostly visible: wrong pixels, or a 2700G hang caught by
   the watchdog. Register and command writes go over the same bus,
   though, so a marginal setting can corrupt GPU commands. **Worth a
   bounded, measured experiment** (§7) if the owner agrees. Do it after
   item 70's DMA flush and item 69's GPU drawing are measured, because
   they change how much traffic crosses this bus.
7. **Don't touch timings: touch less memory.** These are the bigger and
   risk-free wins, and partly queued already:
   - large mappings (64 KiB / 1 MiB) and page tables in internal SRAM
     (ADR-0006: a TLB miss is 5.2× cheaper with the table in SRAM; item
     78);
   - cache-aware layouts and `pld`;
   - iWMMXt copies (item 71);
   - the 2700G drawing in its own memory (items 69, 70).

   The gap between RetroPane's 491 ns and WM's ~280 ns per SDRAM hop is
   larger than anything `DTC` can give.
8. **Disable partition 3** (`DE3`, likely empty). Not a speed gain. It is
   a robustness point: an access to `0xAC000000` reads whatever the bus
   floats to, silently. The kernel must never map it. RetroPane's boot code
   should enable only what is fitted, after a size check.
9. **Stop SDCLK0** (`K0RUN` = 1 with no synchronous flash). A tiny power
   saving; `APD` probably stops it already. Check in item 67.

## 6. Difficulty and risk calibration (honest)

- **Writing the register is trivial; trusting it is the hard part.**
  Tighter SDRAM timings rarely fail at once. They flip a bit on one row,
  at one temperature, under one access pattern, maybe weeks later, in a
  file or a page table. Nothing reports it. On a device whose OS and user
  data live in that RAM, that is the worst failure mode in the project.
- **The margins are paper-thin**: 1.2% on three SDRAM timings, from a
  datasheet for a part only *likely* fitted. Board flight time, the PXA's
  own output delays (EMTS `tsdSDOS` 2.5 ns, input setup 3.0 ns), supply
  droop and temperature all eat into 0.1 ns. There is no tighter legal
  step to test against, so margin can't be shown by passing one step
  beyond.
- **One unit is not the population.** Other X51v units may carry other
  vendors' chips, and SDRAM has no ID register to tell. Any SDRAM change
  would have to be a per-device calibration, never a shipped constant.
- **The 2700G bus is less dangerous, but not harmless.** Too-short strobes
  can corrupt register and command writes, and a wedged RDY hangs the
  core (M: "the memory controller indefinitely waits for the RDY
  signal"). Only a watchdog gets out of that, and the hardware watchdog
  has reset this Axim into Dell's factory-restore prompt before
  (docs/perf.md, 2026-10-12).
- **Calibration of the expected gain: modest for SDRAM** (3-8% on
  memory-bound work, nothing on most code), **possibly large for the
  2700G bus** (flush up to ~1.5-1.8×), which a single measurement run can
  settle either way.
- **Effort:** a step-and-verify image and harness ≈ 1-2 sessions, plus
  owner time at the device for the temperature runs.

## 7. Stress-test plan for any future change (paranoid)

None of this runs without the owner's go. Each step is one device run.

**Isolation.** The trial code, its stack and the known-good values run from
the PXA's internal SRAM (256 KiB at `0x5C000000`), never from the SDRAM it
retimes. It writes nothing to flash or the SD card. Before each step, a
soft watchdog (the FIQ soft watchdog already in use) is armed: if the trial
stops checking in, it restores Dell's values and returns to the loader.
The hardware watchdog is the last resort only, because of the
factory-restore prompt. The loader at `0xA8400000` is copied aside, or
the region is excluded and tested separately after the run.

**Stepping.** One field and one step at a time. SDRAM goes `DTC2` and
`DTC0` together 10 → 01 (CL is shared). The 2700G goes `RDF` 8 → 3 one at
a time, then `RDN` 4 → 2, then `RRR` 2 → 0. Each register write follows
the manual's sequence (M §6.4.10, §6.5.1.4: rewrite `DRI` and re-issue
`MDMRS` after any change). After every step comes a full verify pass. The
first failure ends the run and restores Dell's values.

**SDRAM patterns, all 64 MiB, every bank and row:**
- walking ones and zeros on the data bus;
- address-in-address (catches row, column and bank aliasing);
- checkerboard and inverse;
- xorshift random with a stored seed, verified on a second pass;
- bank ping-pong, alternating rows in the same bank to force tRP, tRCD
  and tRC on every access;
- row hammering of neighbours;
- a dwell test: write, wait 2 × 64 ms with refresh only, verify. This
  catches refresh and retention trouble.

Each pattern runs with caches on and off, with concurrent PXA DMA
memory-to-memory traffic and 2700G scan-out reading, to stress the
arbiter.

**2700G patterns:** the same data patterns written over the whole 16 MiB,
read back slowly at Dell's settings to separate write faults from read
faults. Register write-and-readback on known read/write registers only.
Then a GPU smoke test: item 69's clear, triangle and texture, checked
pixel for pixel.

**Conditions.** Each candidate is run cold (device from a fridge, ~5 °C),
at room temperature, and hot (≥ 45 °C case after 30 minutes at full load,
in a warm room). It is also run on battery near empty and on the charger,
since supply level affects output timing. The candidate passes only with
**zero** errors in every condition, then a 24-hour soak.

**Margin rule.** For the 2700G, adopt the setting **one step looser** than
the tightest that passed. For SDRAM no tighter step exists, so the
temperature and voltage corners stand in for margin, and the chip
markings must match the datasheet used.

**Run time (if anything ships).** These guards would apply:
- the boot code keeps two tables, Dell's safe values and the calibrated
  ones;
- a boot counter in persistent state falls back to safe values after any
  unexplained crash or reset;
- the kernel scrubs pages it can verify (code and clean page-cache pages
  hashed at load) in the background, and logs any mismatch to the crash
  log;
- one mismatch reverts to safe timings for good, pending a re-run;
- debug builds check a sampled CRC of flushed rectangles on the 2700G.

## 8. What this means for the flash boot (bare-metal first)

When RetroPane boots from flash, **its own boot code programs the memory
controller**: nothing inherited, no WM state. The boot design needs:
- the PXA27x SDRAM power-up sequence (M §6.4.10), in this order:
  - clocks, `MDREFR` in steps;
  - `MDCNFG` with only the fitted partitions;
  - eight refreshes;
  - `MDMRS`;
  - then `APD`;
- `MSC0-2` set before CS3 (2700G) or CS4 (CPLD) is touched;
- `CCCR`/`CLKCFG` chosen by item 67, with **`DRI` recomputed on every
  memory-clock change**.

The values in §1 are the known-good baseline the boot code starts from,
the "safe" table above. The DiskOnChip read timing (CS0: 106 ns first
access, 16-bit) also limits how fast the OS image loads from flash. Its
headroom needs the DiskOnChip's datasheet and is an open item for the
flash-boot design.

## 9. Test designs for the flash's and the CPLD's timing (reminders R6, R8; 2026-10-24)

Dell's values for both were decoded in section 1 (2026-10-15); the tests
that would show whether ours could be faster were never designed until now.

**The internal flash (CS0, DiskOnChip G3; reminder R6).** `MSC0`'s low half
`0x38D8`: non-burst flash, 16 bits, RDF code 13 (20 clocks), RDN 8, RRR 3:
**106 ns for a first read**. Dell's OAL keeps that time constant across
clocks (`0x1898` at its 104 MHz points, `0x38D8` at 208: clocks.md), so the
value is a time Dell chose, not one the chip was shown to need.
- **What it limits:** the DiskOnChip has its own controller; a 512-byte
  page costs its internal flash read plus 256 bus reads (~27 us at 106 ns,
  a ~19 MB/s bus ceiling). Which of the two dominates isn't measured; if
  the internal read does, bus timing gains little.
- **The safety rule:** the DiskOnChip holds Windows Mobile and Dell's
  bootloader. Only the **read** strobe (RDF) is ever stepped; the write and
  recovery fields (RDN, RRR) stay Dell's, so every command the chip
  receives is latched at Dell's timing, and no erase or program command is
  ever sent. Each step first reads the chip's ID registers; a wrong ID ends
  the run with Dell's value put back.
- **The test (to build, the owner watching):** (1) at Dell's timing, time
  one page read and its parts (commands, the chip's busy time, the 256
  reads) to learn whether the bus matters at all; (2) if it does, read the
  boot block's directly mapped window (no command needed) 1,000 times at
  each RDF code from 12 down, compared with Dell's reading, stopping at the
  first difference; (3) the fastest clean code, then a page-read soak at
  it. One step labelled on screen at a time; the log on the card.
- **What it would decide:** the flash-boot design's read path (boot and
  program loading from flash, ADR-0005), not a gate line.

**The CPLD (CS4; reminder R8).** `MSC2`'s low half `0x7FFC`: the slowest
setting there is (~0.44 us an access). The CPLD (an XC2C128 CoolRunner-II
on the X50, likely the same on the X51v) switches in nanoseconds, so a far
shorter strobe is plausible. **But:** its registers don't read back
(cpld.md), so a faster write can only be checked by its effect on the board,
which the no-blind-board-bits rule forbids testing unattended; and the
CPLD is touched a few times per power or backlight change (buttons are on
GPIOs), so even a 10x faster access saves microseconds a day. ~~Proposed: decline~~ **The owner, 2026-10-24: test and verify it** ("any
percentage of performance boost, when all added together, is totally worth
it"). The way: reads first, unattended (a readable status register, such
as the panel-type readout, read 1,000 times at each faster strobe and
compared with Dell's timing: reading changes nothing); then writes with the
owner watching, one step at a time, on a switch whose effect is visible and
harmless (the backlight enable), Dell's timing put back after each.

**Built 2026-10-24: `rp-cpldbus`** (`boot/cpldbus`). It refuses to start
unless `MSC2` is Windows Mobile's `0x7FF47FFC`; only CS4's three timing
fields change (`diag::set_cs4_vlio`, host-tested at its range edges), and
`MSC2` goes back at the end and from the crash path. **Reads:** CPLD offsets
`0x00`, `0x02`, `0x04`, `0x06`, `0x14`, `0x16` (the four non-zero status
inputs and two zero neighbours, so a read landing on the wrong register
shows) read 1,000 times each at Windows Mobile's timing (an offset that
changes by itself isn't compared), then at each step: `RDF` 15 down to 3,
`RDN` 15 down to 2, `RRR` 7 down to 0, one code at a time (106 memory
clocks an access down to 9), each step's 10,000 reads timed; a field stops
at its first wrong read (Windows Mobile's timing back, the CPLD read again
there) and the next starts from the last clean step. **Writes**, only in
the watched build (`RP_CPLDBUS_WRITES=1`): at Windows Mobile's timing (the
control) and then at each step the reads proved clean (so the CPLD decodes
its addresses right there), the backlight off for a second and on again,
Windows Mobile's timing back and the backlight on again at it; the owner
says which steps blinked. Every step is labelled on screen. Emulator:
`emu-cpldbus` (all 32 read steps clean, `MSC2` put back; timing not
modelled, so the device's answer is its own).

**Reads on the X51v (2026-10-24, `local/rp-cpldbus-reads.txt`):** every
step clean down to the PXA's VLIO minimum (`RDF` 3, `RDN` 2, `RRR` 0: 9
memory clocks an access against Windows Mobile's 106), 0 wrong of 6,000
reads a step; the six offsets read as at hand-over (`0x00` `0x0048`, `0x04`
`0x00a0`, `0x06` `0x0002`, `0x14` `0x0002`, `0x02` and `0x16` 0). A read
took 327 ns at Windows Mobile's timing, 260 at `RDF` 11, 235 at `RDF` 10-7
and 215 ns from `RDF` 6 down, and shortening `RDN` and `RRR` changed
nothing: past `RDF` 6 something else paces the access (the CPLD's RDY, or
the controller's own overhead), so CPLD reads are 1.52x faster at most,
not 12x. **Writes on the X51v (2026-10-24, the owner watching,
`local/rp-cpldbus-writes-log.txt`): the backlight went dark and lit again at
every one of the 33 steps**, Windows Mobile's timing and each faster one down
to the minimum: writes latch right at 9 clocks too. A second watched run
(the blinks numbered on screen) followed. **For the owner (results, not
decisions):** which CS4 timing the OS uses: the fastest proven (`RDF` 3,
`RDN` 2, `RRR` 0), or a margin for every X51v (the any-Axim rule; rp-cs3bus
took two steps looser), knowing reads gain nothing past `RDF` 6 (215 ns).

## Open questions

1. The X51v's SDRAM chip markings (the owner can read them).
2. The 2700G's minimum VLIO strobe times and RDY behaviour (measure: §7).
3. The 2700G's local-memory clock source (`MEMCLK`; the core PLL is 99.7 MHz), and whether Dell changes `LMTYPE` per clock tier.
4. Whether the PXA270 C5 officially supports 624 MHz with `CCCR[A]` = 1
   (Intel specification update), for item 67.
5. What is on CS1, CS2 and CS5, and whether partition 3 (`0xAC000000`)
   is empty.
6. The DiskOnChip G3's rated access time (flash boot).
