# Dell Axim X51v: the hardware datasheet

Everything RetroPane has established about the Dell Axim X51v (and, where
noted, the X50v), on one page: the board, its memory map, a pinout of the
PXA270's GPIOs as wired on this board, the CPLD's registers, and each
device's interface and quirks. It is a summary: each section links the
page that holds the evidence, and those pages stay the source of truth
(update them, then this).

*As of 2026-10-22 (the SD card's write programming time, from the card log; the baseline audit's device runs: the 2D FIFO's level, the long count, the SD card's end, a CCCR write alone; texture shapes from Dell's GLES library; Bluetooth and USB host from Dell's ROM, the audio service's jack findings, and the Bluetooth radio's power sequence, 2026-10-18). Facts come from measurements on the owner's X51v (a
replacement X50v front bezel), Windows Mobile's own drivers read for facts
only (docs/CLEANROOM.md), Intel's manuals, and AxDroid's board code.*

**Confidence**, as everywhere in `docs/hw`: **confirmed** (measured on the
device, or two independent sources agree), **likely** (one good source, or
code read but not run), **guess** (an inference worth testing),
**unknown**.

![The X51v board: blocks and buses](img/board.svg)

## 1. The chips

| Part | What | Interface | Page |
|---|---|---|---|
| Intel **PXA270** (XScale, ARMv5TE, CPU ID `0x69054117`) | the processor: 91-624 MHz, 256 KiB internal SRAM, iWMMXt | — | [clocks.md](clocks.md) |
| 64 MiB **SDRAM** | main memory at `0xA8000000` (SDCS2) | memory controller | [memory-map.md](memory-map.md) |
| **DiskOnChip G3** | built-in flash (Windows Mobile's ROM and file store) | CS0 | [memory-map.md](memory-map.md) |
| Intel **2700G** ("Marathon"): MBX Lite 3D core, 2D engine, MVED1 video decoder, display controller, 16 MiB local memory | graphics; drives the panel | CS3 (VLIO, 32-bit), interrupt GPIO85 | [2700g.md](2700g.md) |
| Board **CPLD** | power switches, resets, the panel type | CS4, sixteen 16-bit write-only latches | [cpld.md](cpld.md) |
| **3.7" VGA panel**, Sharp **LS037V7DD05** (on the owner's unit, CPLD LCD id 0) | 480 x 640, 18-bit; 56.46 Hz as Windows Mobile drives it, 60.0 Hz measured working (display calibration C3). Its datasheet (LCP-04006A, tentative): a bare COG panel, no timing controller or converters (the board makes CLS/SPS/SPL/LP, VCOM and five rails: +3.3, +5, +8, −4, −8 V); line 35.5-39.5 kHz, clock 19.4-28 MHz (its reference 60 Hz at 25.175 MHz); 8-colour (CO) and QVGA (MO) modes on pins; response 30 + 50 ms typ.; panel ~45 mW, LED backlight 32.4 V at 15 mA = 486 mW | the 2700G, through a timing controller on the board | [2700g.md](2700g.md), [backlight.md](backlight.md), [screen-swap.md](../research/screen-swap.md) §1a |
| Wolfson **WM8750** | audio codec: DAC, headphone and speaker outputs, jack detect | I2C (control, `0x1A`), I2S (data) | [audio.md](audio.md) |
| TI **TSC2046** | touch screen controller | SSP1 | [touch.md](touch.md) |
| TI **TNETW1100B (ACX100)** | Wi-Fi, 802.11b | PC Card socket 0 (`0x2C000000`) | [cf.md](cf.md), [../research/hardware-inventory.md](../research/hardware-inventory.md) |
| TI **BRF6150** | Bluetooth **1.2** (no Low Energy, PIN pairing only), firmware 3.0.48, 921,600 baud | BTUART | [bluetooth.md](bluetooth.md) |

## 2. Memory map (physical)

| Base | Size | What | Conf. |
|---|---|---|---|
| `0x00000000` | 64 MiB | CS0: boot ROM / DiskOnChip G3 | confirmed |
| `0x0C000000` | 64 MiB | CS3: 2700G (registers at `0x0FFE0000`: display `+0x2000`, video decoder `+0x3000`, MBX core `+0x4000`; command windows at `0x0FFF0000` (video) and `0x0FFF4000`-`0x0FFF6FFF` (MBX; the 2D stream at `0x0FFF5000`); local memory below) | confirmed |
| `0x10000000` | 1 MiB | CS4: board CPLD | confirmed |
| `0x20000000`-`0x2FFFFFFF` | — | PC Card socket 0 (Wi-Fi) | confirmed (mapped) |
| `0x30000000`-`0x3FFFFFFF` | — | PC Card socket 1 (CompactFlash slot) | confirmed |
| `0x40000000` | 32 MiB | PXA270 peripherals (GPIO `0x40E00000`, I2C `0x40301680`, I2S `0x40400000`, PWM `0x40B00000`, clocks `0x41300000`, DMA `0x40000000`, UDC `0x40600000`) | confirmed |
| `0x4C000000` | 1 MiB | USB host (OHCI) | confirmed |
| `0x5C000000` | 256 KiB | internal SRAM | confirmed |
| `0xA8000000` | 64 MiB | SDRAM (HaRET needs `ramaddr 0xa8000000`) | confirmed |

Full table with Windows Mobile's virtual addresses: [memory-map.md](memory-map.md).

## 3. Pinout: the PXA270's GPIOs on this board

![GPIO map](img/gpio-map.svg)

Only lines with a known or suspected job are listed; "fn" is the PXA27x
alternate function. Directions are as the board uses them.

| GPIO | Fn | Dir | Job on the X51v | Conf. | Page |
|---|---|---|---|---|---|
| 0 | 0 | in | **Power button**; reads 0 at rest; both edges wake and interrupt | confirmed | [README](README.md) |
| 1 | 0 | in | a wake source (`PWER` `0x80007803`: GPIO0, 1, 11-14 and the RTC alarm); device unknown | unknown | [probe](probe-2026-10-11.md) |
| 2 | — | — | not the USB D+ pull-up (tried both ways: the pull-up is hard-wired) | confirmed (negative) | [udc.md](udc.md) |
| 3, 4, 38 | 0 | out | **core voltage select** (three levels to the regulator: 1050-1574 mV; GPIO3/4 are the power-I2C pins used as plain GPIOs) | likely (code) | [clocks.md](clocks.md) |
| 10 | 1 | out | **32.768 kHz clock out** (`OSCC` TOUT_EN), most likely the Bluetooth radio's sleep clock | likely | [bluetooth.md](bluetooth.md) |
| 11 | 0 | in | an interrupt, both edges (SYSINTR 18), and a wake source; device unknown | unknown | [interrupts.md](interrupts.md) |
| 12 | 0 | in | **SD card detect**, low = card in | confirmed | [sd.md](sd.md) |
| 13 | 0 | in | **CompactFlash card detect**, low = card in; **sticks at "no card" after a removal until a reset** (use CPLD `0x04` bit 1) | confirmed | [cf.md](cf.md) |
| 14 | 0 | in | USB cable detect (likely); reads 0 in the cradle | likely | [udc.md](udc.md) |
| 15 | 0 | out | driven low in Windows Mobile's software reboot | likely | [README](README.md) |
| 17 | PWM0 | out | **backlight brightness** (PWM0, ~30 kHz) | confirmed | [backlight.md](backlight.md) |
| 18 | RDY | in | the 2700G's VLIO ready | likely | [2700g.md](2700g.md) |
| 23, 25, 26 | SSP1 | — | **touch controller** clock (23), data out (25), data in (26) | likely | [touch.md](touch.md) |
| 24 | 0 | out | **touch chip select**; Windows Mobile leaves it as SSP1's frame (fn 2): set it to a plain GPIO first | confirmed | [touch.md](touch.md) |
| 27 | 0 | in | **SD write protect**, high = locked | likely | [sd.md](sd.md) |
| 28 | 1 | out | **I2S bit clock** | confirmed | [audio.md](audio.md) |
| 29 | 2 | in | **I2S data in** (from the codec) | confirmed | [audio.md](audio.md) |
| 30 | 1 | out | **I2S data out** (to the codec) | confirmed | [audio.md](audio.md) |
| 31 | 1 | out | **I2S frame sync** | confirmed | [audio.md](audio.md) |
| 32 | MMCLK | out | SD clock (seen toggling every ~16 µs) | likely | [2700g.md](2700g.md), [sd.md](sd.md) |
| 36 | — | — | **not** the CF reset (AxDroid's guess); a charger/cradle line, perhaps | unknown | [cf.md](cf.md) |
| 42, 44 | 1 | in | **BTUART** receive (42) and CTS (44), the Bluetooth radio | confirmed (code) | [bluetooth.md](bluetooth.md) |
| 43, 45 | 2 | out | **BTUART** transmit (43) and RTS (45) | confirmed (code) | [bluetooth.md](bluetooth.md) |
| 82 | 0 | out | **codec I2C gate**: high during each codec transfer, low after (never left high: shared) | confirmed | [audio.md](audio.md) |
| 83 | 0 | in | **headphone jack**, high = plugged in, valid only while the codec is powered and set up | confirmed | [audio.md](audio.md) |
| 84 | 0 | in | **CF slot powered / ready**, high | confirmed | [cf.md](cf.md) |
| 85 | 0 | in | **2700G interrupt**, both edges (SYSINTR 27); Windows Mobile's disable leaves it an output driven low: make it an input first | confirmed | [2700g.md](2700g.md), [interrupts.md](interrupts.md) |
| 86/87 | 0 | in | an interrupt, both edges (SYSINTR 34); device unknown | unknown | [interrupts.md](interrupts.md) |
| 89 | 0 | out | Wi-Fi reset | likely | [cpld.md](cpld.md) |
| 94 | 0 | in | **touch pen-down**, low while touched (falling edge, SYSINTR 17) | likely | [touch.md](touch.md) |
| 96 | 0 | in | reads 1 whenever audio is powered (not the jack: measured); both edges, SYSINTR 39 | confirmed (meaning unknown) | [audio.md](audio.md) |
| 99 | 0 | out | **board reset**: driven high, the board resets (Windows Mobile's software reboot) | likely | [README](README.md) |
| 107 | 0 | in | **Wi-Fi interrupt**, falling | confirmed (driver) | [interrupts.md](interrupts.md) |
| 113 | 1 | out | **I2S system clock** (the codec's master clock) | confirmed | [audio.md](audio.md) |
| 117 | 1 | — | **I2C clock** (SCL) | confirmed | [audio.md](audio.md) |
| 118 | 1 | — | **I2C data** (SDA) | confirmed | [audio.md](audio.md) |

The keypad uses the PXA27x keypad controller's matrix and direct inputs
(pins as its alternate functions; the key map in [keypad.md](keypad.md)).

## 4. The board CPLD

![CPLD bit map](img/cpld-map.svg)

Sixteen-bit latches at `0x10000000 + offset`. **They can't be read back**:
Windows Mobile's drivers share one shadow copy per register (RAM
`0xA8001320`, ten slots), and RetroPane adopts that table at hand-over
(`Cpld::adopt`). Writing a register blind switches other devices off.

| Offset | Bit | Job | Conf. |
|---|---|---|---|
| `0x04` (read) | 1 | **CF card present when clear** (follows the card both ways, unlike GPIO13) | likely |
| `0x10` | 12 | **CF slot power, stage 2** | confirmed |
| `0x14` | 1 | **speaker amplifier** (70 ms to settle) | confirmed |
| `0x14` | 3 | display (guess); **clearing it, or `0x1A` bit 1, for 1 s wedged the device** (hardware watchdog reset; which of the two is unknown) | guess: never touch |
| `0x16` | 0 | **SD card power** | confirmed |
| `0x16` | 1 | **CF slot power, stage 1** (then `0x10` bit 12, 10 ms apart) | confirmed |
| `0x18` | 1 | **CF RESET** (set = held in reset) | likely |
| `0x18` | 2 | **backlight enable** | confirmed |
| `0x10` | 2 | **Bluetooth power, stage 2** | likely |
| `0x16` | 3 | **Bluetooth power, stage 1** | likely |
| `0x18` | 0 | **Bluetooth reset**, active low (set = running) | likely |
| `0x1C` | 1 | **audio codec power** | confirmed |
| `0x1C` | 3 | **USB / dock**: on in the dock, off out of it (Windows Mobile's shadow, one observation) | likely |
| `0x1E` | 3 (write) / 1-0 (read) | **LCD type readout**: 0 = 3.7" Sharp VGA (the owner's), 1 and 3 = 3.5" Sharp QVGA, 2 = 3.7" Samsung VGA | confirmed |
| `0x1E` | 2-0 (write) | **the touch controller's AUX switch** (REA #35): `0,1,0` the battery's thermistor (B = 3174 K, 25 °C), `0,0,0` battery voltage, `1,0,0` battery current, `1,1,0` battery type, `0,1,1` unknown; Windows Mobile's battery driver reads each through TSC2046 AUX ([touch.md](touch.md)) | likely (Dell's use) |

Other bits with a likely or guessed owner (Wi-Fi's power sequence,
Bluetooth/serial, USB, battery, notification LED, display sequence) are in
[cpld.md](cpld.md). **Rule:** a bit with any recorded owner is never
switched by a test image; an unknown bit only one per run, with the owner
watching.

## 5. Devices

### Display (2700G)
- The video plane in RGB565 (`VSCTRL` format 7, the colour-space
  conversion off), shown exactly (run D): the driver's
  `VideoFormat::Rgb565` (2026-10-27), a 16-bit 3D target shown through
  the scaler while the core draws the other of a pair (`rp-gpubench`
  GC2BE measures it at divide 1).
- The video plane in ARGB8888 (code 8) and ARGB4444 (code 4) with
  per-pixel alpha (mode 3) over an RGB565 graphics plane: exact, no
  underrun (`rp-gpubench` run WG, gpubench-0248/-0249). Mode 3 ignores
  `VBBASE`'s global alpha. Window repoints written in lines 643-2 are
  whole (`VSCADR`, `VSCTRL`, `SSSIZE`, `VSADR` x); moving `VSADR` y tore
  10-15 of 304 refreshes. `MBXCLK` 0 leaves the display scanning
  (300 of 300 refreshes); the 2D clock back in ~90 us.
- `DMCTRL` (`0x2188`), the display's memory control (mbxfb's name):
  Windows Mobile leaves `0x10303010`, Linux mbxfb writes `0xD0303010`;
  its two high bits change nothing for the divide-1 corruption
  (`rp-glesfeat` G17); its four byte fields' meaning is unread
  (`rp-gpubench` GC2BC measures them as the fetch's burst or FIFO
  thresholds against the 3D core's divide-1 fault and the underrun
  flag). The scan line is `DVLNUM` (`0x2308`, bits 0-11).
- Flips (Dell's driver, read 2026-10-29, `2700g.md` "Dell's flip path,
  read"): `GSCADR` rewritten in the VEVENT0 interrupt (line 643, GPIO85),
  then `DUCTRL` (`0x230C`) = 1; GLES triple-buffers (`BackBuffers` 2,
  `VSyncLocked` 1). A `GSCADR` write takes effect at once with the X51v's
  `DSCTRL` update bits (16-19, 24-26) at 0 (W1; R113 on the X51v: 180 of
  180 writes at lines 100, 320, 600 tore their refresh, with or without
  Dell's `DUCTRL` = 1, and with bit 25, 26 or `UPDWAIT` 1 set). **With
  `DSCTRL` bit 24 (`UPDCNT`) set and `DUCTRL` = 1, no write tore (0 of
  180) but none showed in the next refresh either.** R113 run 2: with bit
  24 set a `GSCADR` write is held until `DUCTRL` = 1, which applies it
  at once where the scan is: at line ~3 the new picture shows whole from
  that refresh; mid-refresh the display shows neither picture, that
  refresh and after, until `DUCTRL` is written again. Not a refresh-
  boundary latch: the trigger must be in the blanking.
- One graphics plane composed by the 2D engine (ADR-0023), double
  buffered; its interrupt line is GPIO85. Windows Mobile's 648 x 645
  frame at `DISPPLL` `0xEC5` (23.56 MHz) gives 56.46 Hz; 60.0 Hz was
  measured working (display calibration C3).
- **MBX 2D engine, 32 to 16 bits** (`rp-mbx-blend` B7, X51v,
  2026-10-21): a copy from a 32-bit source to RGB565 **truncates** each
  channel (all 256 levels, red, green and blue); a blend onto RGB565
  truncates too (B1-B3's fitted arithmetic, exact at alpha 0xFF and
  0x80). **No 2D dither bit**: header bits 19, 22, 27 and blend-word bits
  `[31:28]`, `[19:0]` change nothing on a 64 x 64 ramp; **header bit 21
  writes nothing** (no stray writes, no hang). At WM's clock: a
  full-screen 32-to-16 conversion 9.6 ms (480 x 80: 1.2 ms), a 16-bit
  full-screen copy 5.0 ms (0.63 ms), a 32-bit half-screen copy 5.1 ms, a
  32-bit full-screen stride-0 copy 10.0 ms, the renderer's 4-blit dither
  add over a full 32-bit screen 18.9 ms.
- **2D and 3D at once corrupt the 3D render** (`rp-gpubench` run FT,
  X51v, 2026-10-21): with full-screen 2D copies running, a full-screen
  RGB565 render gets ~220 wrong pixels in 307,200 (450+ with 2D fills),
  spread evenly, half far off; alone it repeats exactly. Together saves
  9% of the time. The engines take turns (ADR-0030 G2); the cause is
  open (`2700g.md` "baseline audit").
- **2D rotation and masks** (`rp-mbx-blend` B11, X51v, 2026-10-22):
  header bits 25-26 = 1/2/3 turn a copy 90° cw / 180° / 270° cw
  (rectangle in the source's shape); 1 bpp masks (`0x40000000`,
  `0x50000000`, `0xE4000000`, address) expand a colour, high bit = left,
  ROP4 as GDI's.
- **3D second texture unit** (`rp-glesfeat` M, X51v, 2026-10-22): TSP
  bit 16 = a second coordinate set at the TA; the TA's two-unit object is
  ISP, TEX0-2 x2, TSP, x/y, z, 1/w, then colour, u0, v0, u1, v1 a vertex
  (list layout 12); the 3D stage hangs on it (open).
- **2D commands by DMA** (`rp-gpubench` run FD, X51v, 2026-10-22): DMA
  into the command window (`0x0FFF5000`) works; 352 words cost the CPU 2
  us to arm against 43-60 us to write; batches must fit the FIFO's room.
- **The TA's window shares the 2D FIFO's level** (`rp-tabench` run 1,
  X51v, 2026-10-25; ADR-0026 T8): with the 2D engine idle, TA words fill
  `0x12c`'s FIFO level to 510 (full) while the TA bins; the CPU then
  writes at the TA's pace. DMA into the TA window works (pieces of whole
  32-byte lines within the reported room): 5,000 quads' list in 30.7 ms
  against 53.9 ms by CPU stores. TA-built scenes of 100-5,000 textured
  quads into a 16-bit target rendered wrong while the frame began with the
  GLES clear's colourless object, which draws nothing (audit C80); with a
  plain strip as the background and the memory manager set up after a core
  reset, every TA picture equals our binner's (`rp-tabench` run 5). About
  one TA render in three hung the 3D stage while the memory manager was
  set up again between scenes or shared its parameter memory with the CPU
  binner's; set up once, in a parameter memory of its own, 0 of 150 hung
  (`rp-tabench` run 9, audit C81 fixed).
- **The 2D engine's clock off** (`rp-gpubench` run FS, X51v, 2026-10-22,
  `local/cardlog/gpubench-0044.txt`): `MBXCLK` enable 0 (divider kept)
  with 20 fills and a fence queued pauses the work; the clock back on, it
  finishes (the fence after 124 ms) and draws right. **One read of the
  MBX's status (`0x12c`) with the 2D clock off froze the CPU's bus**: the
  watchdog reset the device (Dell's restore screen). Rule: no MBX register
  access while its clock is off (audit C43).
- **MBX status `0x12c`** (`rp-gpubench` run FI, X51v, 2026-10-22): raw
  (the enables `0x130` don't change it); 2D sets no completion bit (bits
  16-23 hold the FIFO's fill while it works): 2D waits are polled; a
  render's end sets bits 2 and 3 (bit 3's meaning unknown), bit 6 =
  parameter memory ready; `0x134` all ones clears them.
- **DMA memory-to-memory** (`rp-probe`, X51v, 2026-10-22): 128 KiB in
  2.90 / 4.78 / 8.50 ms with 32 / 16 / 8-byte bursts (~0.5 us fixed a
  burst; 45 MB/s at best against `memcpy`'s 108); the DCMD width field
  0 or word makes no difference.
- **iWMMXt** (`rp-probe`, X51v, 2026-10-22): `wCID` `0x69051020`, access
  (CPAR bits 0-1) on as Windows Mobile leaves it; saturating adds
  saturate. Against plain ARM code: a 16-bit saturating mix 6.7x faster
  in the cache, 2.1x from memory; a rounded byte average 2.8x / 1.5x;
  64-bit copies no faster than `memcpy` (the bus).
- **CPU, measured** (`rp-probe`, X51v at 624 MHz, 2026-10-22): the D-cache
  is 32 KiB (a line walk misses from 40 KiB, ~233 ns a missed line);
  `memcpy` ~375 MB/s in the cache, ~96 MB/s from memory; the internal
  SRAM copies at 286 MB/s, a random hop 111 ns (SDRAM 491); a page table
  in the SRAM makes a 60-section TLB spill 5.2x cheaper. **A D-TLB miss
  on 4 KiB pages** (`rp-probe` pages run 1, 2026-10-22: 257 misses in
  4,096 hops over 8 MiB): ~565 ns with both page-table levels in SDRAM,
  ~424 with the second level in the SRAM, ~331 with the first, **~25 with
  both in the SRAM**; the hop itself 448.6 ns on sections, 484.1 on 4 KiB
  pages (tables in SDRAM), 450.2 with both tables in the SRAM. **Worst
  case** (run 2, a new 4 KiB page every hop): 1,016 ns a hop with the
  tables in SDRAM, 473 with both levels in the SRAM (sections 449); a
  miss ~550 / ~24 ns. **D-TLB lock-down works** (`c10, c8, 0`, run 3):
  a locked page stops missing; a walk with the data cached costs ~544 ns
  (tables in SDRAM) or ~94 ns (both levels in the SRAM). Details: cpu.md.
  `memcpy` with its prefetch 128 bytes ahead copies 109 MB/s from memory
  (96 at 64 bytes, 47 with none); DMA copies SDRAM to SDRAM at 45 MB/s,
  the CPU keeping 86% of its speed meanwhile. SD multi-block reads by DMA
  run 26-28% faster with a pause between the driver's status polls
  (256 blocks 15.95 ms, ~8.2 MB/s): polling the SD controller during its
  own transfer slows it.
- **MBX 2D engine, more** (`rp-mbx-blend` B8, B9, X51v, 2026-10-22): the
  dither add's cost is its blend pass (one full-pattern blit = four row
  blits, 18.9 ms); the CPU polling the fence doesn't slow the engine; the
  clip rectangle's right and bottom are exclusive; **the scale word scales
  (nearest pixel, `s` = 32 x source / destination), at half a copy's
  speed**; **8 bpp sources through a 256-entry palette** (`0xC0000000`,
  its address; required), 63 ms a full screen; 1 bpp sources through
  palette entries 0/1, high bit first. All 256 ROP3s are GDI's (B10);
  8 and 24 bpp fills (24 packed, blue first); header bit 21 suppresses the
  write (fills, copies); status bit 10 isn't "commands queued".
- **3D texture row order** (`rp-gpubench` GC2CH, X51v, 2026-10-28):
  TEX0 bit 0 = texels in plain rows (not twiddled), 16- and 32-bit, exact on
  every pixel; the row stride is 8 texels whatever the size codes say; TEX1
  bit 29 (nearest mip level) made it the declared width (32); no TEX0, TEX1,
  TEX2 or render-block bit sets it otherwise. Texels sampled at the pixel's
  corner. **With TEX1 bit 29 the stride is the declared width** (across
  code; 32-1,024 texels; GC2CI), no cost against twiddled reads (GC2CJ).
- **2D placements beside a 3D render** (`rp-gpubench` GC2CP low, X51v,
  2026-10-29): clean at WM clock; 16-38 stray words a 240 x 320 frame at
  74.75 / 1 (as at 143.00 / 1); with the 2D finished first, none.
- **CS3 bus check** (`rp-cs3bus` self-test, R71, X51v, 2026-10-29): six
  patterns over 512 KiB of 2700G memory, 1,572,864 transfers a check, 0
  misses at 7/3/1 (`MSC1` `0x13742481`) and at 7/2/0 (`0x02742481`) cold
  and after 300 s of traffic at 7/2/0 (this unit).
- **3D combine arithmetic** (`rp-gpubench` GC2CW, GC2CX, X51v,
  2026-10-28): DOT3 (TEX0 bit 13, TEX2 `0x86096612`) at a single-channel
  tint 32 = `floor(128 v / 255)`, at 64 = v (all 256 values); the plain
  textured TEX2 `0xD6886610` ignores the tint; the per-channel 4x multiply
  (`0x86096612` without bit 13) reads ARGB8888 texels at 5 bits; blending
  ONE, ONE adds exactly, alpha included, clamped at 255.
- **3D four-corner objects** (object list flags; G2 measured, layout 9
  static): list word `0x60A00000 | offset` (mask bits 30-29, layout 5) =
  one flat or Gouraud object of four corners in strip order, 22 words
  (ISP, TSP, x/y x 4, z x 4, 1/w x 4, colour x 4): drew exactly what its two
  triangles draw (`rp-glesfeat` G2, X51v, 2026-10-23). `0x61200000 |
  offset` (layout 9) = the textured form, 33 words (ISP, TEX0-2, TSP, x/y x
  4, z x 4, 1/w x 4, then colour, u, v a corner): Dell's display driver's
  rectangle (`ddi_2700g.dll` `0x02117d9c`, `0x02118414`; static,
  2026-10-28), **not yet drawn by our code on the X51v**. The renderer
  writes both (`Objects::Quads`, R78 item 3).
- **TEX2 `0x86096612`** (Dell's DOT3 entry, kinds 3-4) = per-channel
  `clamp(4 x texel x vertex colour)`, no cross-channel sum; `0xC6046608`
  draws white (`rp-gpubench` GC2CJ, X51v, 2026-10-28).
- **DOT3 = TEX0 bit 13 + TEX2 `0x86096612`/`0x86094612`**: `clamp(4 x (texel .
  vertex colour))` over R, G, B, replicated to R, G, B and A, no bias, no
  sign (`rp-gpubench` GC2CL, X51v, 2026-10-28; Dell's builder `libGLES_CL`
  `0x02267434` sets bit 13 for DOT3).
- **DOT3's rounding** (`rp-gpubench` GC2CM, X51v, 2026-10-28): fits
  `floor(4 x sum(t c) x 255 / 65536)` (t, c 8-bit, read as x / 256) on every
  logged word. Our RGB-to-UYVY (U' = B, V' = R, Y' by DOT3, 2D ROP3 `0xCA`
  placement) exact in U' and V', 16.45 ms for 240 x 320 at WM's clock.
  **Exact at divide 1 too** (`rp-gpubench` GC2CP, X51v, 2026-10-28: 74.75 /
  1 and 143.00 / 1, Y' as at WM's clock); Y' is deterministic but not
  exactly that formula (a fixed ~10-25k words a frame a step off). Two
  renderers can share the core switching only the start table's registers
  (no reset or memory-manager set-up), right at both clocks. Game frames
  (fastest whole frame against native 480 x 640): 480 x 320 1.56-2.06x,
  240 x 320 converted 0.89-1.37x serial, 1.13-1.62x pipelined (stray
  words from the 2D placements beside the render). These times don't
  follow the clocks (a render waited to the renderer's predicted end).
  **With the renderer learning the real pace (GC2CP's second run, X51v,
  2026-10-28), at 143.00 / 1:** native 9.2-13.6 ms (was 18.7-36.0), the
  conversion's render 3.55 ms (6.9 at 74.75 / 1), 480 x 320 with two frames
  in flight 2.3-5.0 ms, 240 x 320 converted with its lists pipelined
  6.4-8.1 ms and clean (the 2D placements beside the render leave stray
  words at divide 1; apart, none). The CPU's set-up costs ~2 us a tile
  (region array and list choice), about 1 word a us: CPU-bound, not the
  bus's ~33 MB/s.
- **The MBX render block and the Dreamcast's PowerVR2** (`rp-gpubench`
  GC2CQ, X51v, 2026-10-28): `0x650` is the PVR2's `FB_W_CTRL` (the same
  pack-mode codes and dither bit); of 1,005 field settings (`0x61C` read as
  `FPU_PARAM_CFG`, bits 2-0 of every render-block word at 1-7, every 4-bit
  field of the render and TA blocks at 15) only 20 change the picture at
  WM's clock: `0x61C`'s low nibble, `0x644` bits 0-1, `0x614` / `0x618`
  bits 28-31 (address-like: the scene not drawn). No sample-position or
  delay field; none cures divide 1's faults.
- **The divide-1 step by picture size** (`rp-gpubench` GC2CR, X51v,
  2026-10-28, warm, 100 renders a frame): 480 x 640 right from 143.00 / 1
  down; 480 x 320 right from 139.75 / 1 down (its textured frame wrong in
  every render at 143.00 / 1); 240 x 320 the same, plus a one-pixel fault
  on its flat mesh in ~1% of renders at every divide-1 step (143.00 to
  108.33 / 1).
- **The video plane's vertical-only scale of RGB565** (`rp-gpubench`
  GC2CS, X51v, 2026-10-28, the owner's photos): right for a 240-wide
  source (GC2CE), **wrong for a 480-wide one** (right over the left ~40%,
  streaks, then nothing); 240 x 320 UYVY through the 2x both ways right.
  **480-wide UYVY through the vertical-only scale is right** (GC2CT): the
  width limit is RGB's. **`GDRCTRL` bit 29 alone doubles lines only**
  (480 x 320 shown right full screen, GC2CT, the owner's photos).
- **MBX 2D engine's surface format codes** (`rp-gpubench` GC2CG, X51v,
  2026-10-28, an RGB565 copy to each code, read back): bit 4 of the code is
  ignored (16-31 repeat 0-15); 0 1 bpp, 1 2 bpp, 2 4 bpp, 3 and 4 8 bpp,
  5 4 bpp (another), 6 RGB332, 7 ARGB4444, 8 and 9 1555, 10 RGB565, 11
  RGB888 packed, 12-15 ARGB8888. **No code writes or reads YUV**: the
  datasheet's CSCBLT isn't a surface format (none of 64 copies converted,
  none hung).
- **MBX 3D core**: tile-based (8 x 16 px tiles at 32 bpp, 60 x 40 on the
  screen); each object carries an alpha-test word (bits 31-29 compare,
  28-21 reference), a TSP word (Gouraud bit 22, depth mode and write,
  fog bit 24 / no fog bit 25) and texture words (texture code 10 = 8-bit
  grey, code 8 = grey + alpha, code 7 = ARGB4444; full list in 2700g.md);
  all eight depth modes proven (ADR-0026); render done is bit 2 of the
  interrupt status `0x12c`. **The display's fetch of the 32-bit screen (~74 MB/s at 60 Hz) costs the 3D core ~10% and CPU reads of 2700G memory ~20%** at WM's clock (`rp-gpubench` runs W and T, 2026-10-21: 3D 37.9 ms with a 32-bit plane, 35.6 ms with a 16-bit one, 34.3 ms stopped; reads 38.1 / 32.5 / 31.9 ms). **Under load the 32- and 24-bit targets get wrong pixels** (run P2X, 2026-10-21: ~40 of the bars' pixels a full-screen render, ~650 pixels differing between renders at 32 bits, 0 at RGB565; each wrong pixel the next object's colour), at Windows Mobile's clock, **only while the display scans out a 32-bit screen** (run P2Y: with the screen at 16 bits or stopped, 32-bit renders exact and identical; the CPU's reads exact). **The target format `0x650`** (`rp-gpubench` run P, 2026-10-21, X51v): bits 2-0 the layout (0 RGB555 with the top bit 0, 1 RGB565, 2 ARGB4444, 3 ARGB1555 with the alpha bit set for opaque, 4 packed 24-bit RGB888 (3 bytes a pixel, blue first; the planes show it as display format 6, owner-seen in run P2W; renders a third slower than 16/32-bit), 5 xRGB8888 with the top byte 0, 6 ARGB8888, 7 ARGB4444 like 2 for opaque pixels), **bit 3 dithers** every 16-bit layout with a 4 x 4 ordered matrix (run P2; RGB565 ramps smooth to the owner's eye, run P2W), not 24/32-bit; codes 2 and 7 identical; code 6 stores alpha (blended like the colour), 5 writes 0, 3 forces the top bit 1, 0 forces 0; **`0x644` bit 0 / bit 1: a 2:1 averaging downsample on write-out, vertical / horizontal (both: 2 x 2 box): supersampling's resolve**, free (run T3: a full-screen render 15.95 ms plain, 14.46 ms with the 2 x 2 average) (run T), bits 4-19 kept but no effect seen, bits 20-31 read back 0; every code writes rows at `0x660`'s stride, none twiddled (no render-to-texture by this register), none past its rows. Textures are twiddled, sides powers of two up
  to 2048, rectangles too (512 x 1024 drawn right, `rp-gpubench`
  2026-10-19); Dell's GLES has no row-order mode. At WM's clock a bare
  tile renders in ~5.3 us, a textured one ~11.3 us, each translucent
  layer adds ~8 us: a full-screen textured layer takes 25-29 ms. A
  `GSCADR` or `VSCADR` repoint takes effect at once (write it in the blanking); a pan by repointing either plane is smooth (`rp-gpubench` run C). The video plane takes the graphics plane's RGB formats too (`VSCTRL` 4-8; RGB565 shown exactly, run D), not only YUV. The video plane's scaler (`SSSIZE`, `SPOCTRL`, `SHCTRL`, mbxfb's layout) takes nine settings up 2x, down to 1/2 and placed, read back as written; scaling down (or up with 1 tap) underruns the video stream (`DINTRS` bit 5) unless `SPOCTRL` bit 27 is set as Windows Mobile sets it (`rp-blend` P9, 2026-10-24; the pictures not yet judged). Where the scan-out picture sits in local memory matters: with the display reading from 0x9D6000 a multi-quad 3D render varies by single pixels and takes ~2 ms longer; with it reading from offset 0 (Windows Mobile's place) every render is identical and as fast as with scan-out paused (`rp-gpubench` C71, 2026-10-24; `rp-glesfeat` G17 the same at divide 1). DMA into 2700G local memory: 30.7 MB/s with 32-byte bursts (24.6 at 16, 16.7 at 8), the width field no difference (`rp-gpubench` BX, 2026-10-24). The PXA270's mini data cache: 2 KiB (X bit, C 1, B 0), and streaming 1 MiB through it keeps a 24 KiB hot set in the main cache (19 us against 180 to walk it after: `rp-probe` mini, 2026-10-24; cpu.md). At Windows Mobile's clock into a 16-bit target: a translucent full-screen layer costs 5.18 us a tile, linear to 32 layers; a textured 64 x 64 quad ~178 us, a flat one ~1 us; 32 textures cost no more than one; a whole texture over the screen 6.5-12.6 us a tile from side 64 to 1024, no cache cliff (`rp-gpubench` LIM, 2026-10-24). `SINTSTAT` bit 4 reads set throughout, `MINTSTAT` 0; no status bit follows the 2D FIFO's level (FI2). A 480 x 640 2D copy 9.1 ms; a 640 x 480 picture turned into 480 x 640 by the 2D engine 41.8 ms; a screen of text from cached lines 9.9 ms (F7, CH). The panel counted at every pixel clock in its rated range: 56.5, 58.5, 60.5 and 62.5 Hz at 23.56, 24.38, 25.19 and 26.00 MHz (`DISPPLL` M 29-32, N 4, P 2), no faults (`rp-refresh` P17 ladder, 2026-10-24). The CPU's bus to the 2700G (CS3, `MSC1`): Windows Mobile's 17 clocks a beat gives ~33 MB/s writes, 12 MB/s reads; the 2700G paces it with RDY, so 14 clocks gains only ~7% and 13 clocks reads wrong (`rp-cs3bus`, 2026-10-19). An 8-bit grey texture (code 10) draws faster than RGB565 (a full screen 22.7 ms vs 27.2); a 2D blit costs ~14 us of engine time each (`rp-gpubench` run E, 2026-10-20); the 2D engine's 4 bpp coverage blend onto RGB565 runs at ~16 Mpx/s, a small plain copy (12 x 16 px) costs ~8.8 us of which ~6 us is fixed per operation (not per command word: the engine keeps its surfaces between operations, and a copy runs without its colour word, run F's F6), and a scattered single-word CPU write to 2700G memory ~380 ns (run F). The clocks (`rp-gpubench` run G1, 2026-10-20, inconclusive): with the cores at an unchanged 49.8 MHz, halving the system (memory) clock made the 2D fill 2.7x and 3D 1.6x slower; at 108.33 MHz with Dell's CL3 rule bus, memory and 2D were right (2D 9% faster) and 3D all wrong; a step could leave the 2700G broken after its registers were put back, and `PLLSTAT`'s lock bit read set at once even across a frequency change (unproven as a lock signal): run G2 retests. Run G2 (the owner's eyes; its log lost to a soft reset): steps 1-6 right, including 74.75 MHz with the cores at divide 1 and 108.33 MHz with CL2; 108.33 MHz with Dell's CL3 never came back (Dell's driver never runs CL3 on the X51v: its 100 MHz clamp). Run G3 (whole log): `PLLSTAT` bit 2 drops after a PLL write and relocks in ~16 us (read too early, it shows the old lock); writes to the memory controller (`LMTYPE`, `LMREFRESH`) written back to back on the reference clock can be lost (Linux waits 1 ms each; `set_speed` now checks and rewrites); a texture in 2700G memory was damaged across clock changes; **the cores at divide 1 at 99.67 MHz ran right: 3D 1.48x and 2D 1.22x faster** with the memory at WM's clock. **Run G4 (all 28 steps, both passes alike): the cores at divide 1 at 99.67 MHz, the memory at WM's clock, made 2D 1.89x and 3D 1.81x faster, right; CL2 ran right to 133.25 MHz; CL3 always breaks the 3D core (and only it); the bus stays 33 MB/s past 99.67 MHz; the display can stop showing across a clock change**: the graphics stream underruns while the system clock sits on the 13 MHz reference, `DINTRS` bit 4 is left set and `DSIG` reads 0, and nothing is shown until the flag is cleared (runs R and S; run U reproduces and clears it). The 3D render-table registers (`0x608`-`0x6DC`) read back what was written (`rp-gpubench` run GC2, 2026-10-23); **the host model's Gouraud and bilinear pixels are not the device's** (GC2: 8 of 9 references differ; flat and 1:1 textured scenes equal). **The MBX cores' clock limits Gouraud shading** (run GC2, 2026-10-23, every robustness rule holding): wrong at the cores' divide 1 at every system clock from 74.75 MHz; with divide 2, every kind right to 117 MHz (cores 58.5 MHz), ARGB8888 Gouraud a few words wrong at 125.67 and 133.25; the memory right at 133.25 MHz with the cores at divide 3 or 4; `MBXCLK` bits 3-2 divide by 1-4; a second PLL setting changes nothing. **The cores at divide 1 draw interpolated content wrong at every speed, 58.5 MHz included** (run GC2D, 2026-10-23), so the cores' speed comes only through divide 2 or more. Intel's datasheet: GFXCLK rated 13-75 MHz, MEMCLK 13-133 MHz (133 only with the memory interface at 2.5 V, 100 at 1.8 V), VCC_CORE 1.104-1.296 V (no register sets it). `0x61C`'s low bits change how Gouraud is drawn (Dell's OpenGL ES value 7 against the 2D driver's 4: a third of an RGB565 Gouraud picture differs); `0x614`/`0x618` hold bits 31-3 only (run GC2F). **The local memory runs right at 149.5 MHz and the cores at 74.75 MHz through divide 2, every RGB565 kind right (1.40-1.50x Windows Mobile's speed), ARGB8888 Gouraud a few words wrong** (run GC2H). **The local memory fails from 156 MHz** (run GC2H2: every kind wrong at 156.00 / 4, renders hung at 162.50 and 169.00). **Of 426 undocumented bits near the cores, only `0x644`'s write-out downsample (bits 0, 1) draws right at divide 1** (run GC2X), **and only by hiding a share of the errors in its averaging: ARGB8888 Gouraud still wrong at 99.67 and 133.25 / 1 under every downsample** (run GC2W). Reading the system block's `0x12C` (no register) froze Windows Mobile's bus but returned 0 under RetroPane's loader (`rp-gpubench` run H, 2026-10-20, twice; cause open). **The 2700G cold-starts from RetroPane's own code** (run Z, 2026-10-20, owner-seen): `SYSRST`, then `SYSCFG` `0xFFFFFF0C` (Linux's bus timeout: without it the bus froze), clocks, local memory (Linux's order), system block, display (`DHDET`/`DVDET` are the display-enable window), writes 1 ms apart, no reads until the memory runs.
- **Clocks (rp-gpubench G4/G5, 2026-10-20, X51v):** the 2700G system clock
  right from 49.83 to 133.25 MHz with the memory at CL2 (CL3 breaks 3D);
  the MBX cores at divide 1 right up to 133.25 MHz (3D 1.26x faster than at
  99.67, 2D likewise); the CPU's bus to it capped at ~33 MB/s writes from
  99.67 MHz up. A stopped display (`DSIG` 0) is brought back by the
  rebuild (run Z) without a soft reset; G4's dark display was an underrun
  (`DINTRS` bit 4), G5's start-up one was not. `DSIG` (the output
  signature) reads 0 for an all-black picture (run N): it tells a stopped
  display only with non-black pixels on screen. **It covers the whole active area, every bit, on the native, line-doubled and scaled routes, returns to the same value for the same picture, and updates once a frame at scan line 2** (`rp-gpubench` GC2DB, 2026-10-29). **A video plane with its stream off still takes part in the slot order (`VSCADR` bits 24-25): left in the graphics plane's slot it changes the picture** (GC2DC run 2; the driver parks a hidden plane out of it, `layers::park_hidden_video`). **The plane registers act at once:** a route switch is whole only when written on lines 643-3 of the 645-line frame (GC2DC run 2). **Warm soak (run GS,
  2026-10-22, 20 min each, stressed, one X51v): 0 BAD at 99.67 MHz and at
  133.25 MHz CL2, the cores at divide 1** (1,320 rounds, 3D pixel-exact,
  no drift, registers held, no underrun, the display sending throughout);
  with a 16-bit screen scanning out 3D 22.85 / 17.95 ms and 2D fill 4.23 /
  3.09 ms against 35.58 / 6.35 ms at Windows Mobile's setting (1.56x /
  1.98x 3D). **The cold soak (an hour off at room temperature) the same:
  0 BAD, the same times.** **The local memory runs on the system clock**
  (run GV, 2026-10-22: with the cores held, halving the system clock made
  a full-screen 2D copy 1.85x slower; doubling the cores with it held,
  only 1.20x faster). **The 2D engine and the 3D core can't overlap
  while the display scans out** (run FT2, 2026-10-22): the render gets
  10-17 wrong words a frame with a 16-bit screen, ~200-240 with a 32-bit
  one, none with scan-out stopped or with either engine alone; placing
  their memory apart doesn't help. **At 133.25 MHz with the cores at
  divide 1 an RGB565 Gouraud scene isn't repeatable even alone** (run
  FT2 fast: 104-365 words differ render to render; exact at Windows
  Mobile's clock), though run GS's 32-bit scene was exact there for 40
  minutes: the content matters. **Run GC (the content map, 2026-10-22):
  with the cores at divide 1, Gouraud shading is wrong at every system
  clock (99.67-133.25 MHz: 65-1,026 words a render), textures, blending
  and flat fills right; the cores at divide 2 with the system clock at
  133.25 MHz right but one word in two of three ARGB8888 Gouraud
  renders. The cores' clock limits Gouraud; Dell's 75 MHz cap may hold
  for it.** **Run GC2S (2026-10-24, `gpubench-0082`): with the scan-out
  buffer inside the safe region (offset 0, C71; with the 3D target the screen at 0, the 2D engine and the 3D core at once are exact at Windows Mobile's clock and 149.50 / 2, ~380 words wrong at 108.33 / 1 with 2D copies behind the render, FT3 2026-10-24; the safe region's ~704 KiB
  matches the 2700G5's on-die memory as Wikipedia gives it, 384 kB on the
  2700G3: 2700g.md, source W; **measured 2026-10-29, R23T, gpubench-0224:
  a 3D target is faster wholly inside 0x0-0xB0000 (to the edge) and slows
  by ~3.7 us a KiB of target past 0xB0000, the textured game frame 13.04 ms
  on die against 15.28 ms in SDRAM at 149.50 / 2: the first 704 KiB are
  faster memory, its edge at 0xB0000**; **the safe region's end measured at the same edge, 2026-10-29, R23E, gpubench-0226: with the display reading a screen ending at 0xAE000 or 0xB0000 no render of 160 is wrong at 74.75 / 1 and 149.50 / 2; ending at 0xB0400, 0xB2000, 0xB6000, 1, 6 and 10 of 80 renders wrong at 74.75 / 1 (none at 149.50 / 2): the safe region is the on-die memory, 0x0-0xB0000** (`SAFE_END` raised to 0xB0000 by the owner, 2026-10-30; runs 2-3, gpubench-0227/-0233, agree at divide 2); **divide 1 into SDRAM with the display on, 2026-10-30, R121 tests 1-3 and R117T, gpubench-0237 to -0241: at 143.00 / 1 with the lists on die and the underlay a light frame misses one pixel of a tile's last row in ~2 of 100 renders (lists in SDRAM: 78 of 100); an alpha-0 full-screen quad blended over the frame cures it (0 of 480, ~8.7 ms a light frame) and beats the divider switch to 143.00 / 2 (0 of 400, 9.2-11.3 ms); the zero-wait divider switch soaked an hour with the display on, 16,572 round trips, every render right, no underrun, 1 us a switch; the overlay soaked an hour (R121S, gpubench-0241), 0 of 32,592 renders wrong**), every kind is right at
  133.25 and 149.50 MHz / 2 (7 of 9 with the screen outside) and renders
  4-5% faster (**20-minute soaks, 2026-10-29, all nine kinds: 149.50 / 2
  hung one ARGB8888 Gouraud capped render after 10.6 minutes; 133.25 / 2
  7,866 renders and 117.00 / 2 7,587, every one right, none hung; R103: 149.50 / 2 with the six 16-bit kinds inside the safe region, 21,654 renders in 40 minutes, every one right, none hung**: 2700g.md); at divide 1 only Dell's 74.75 MHz gains (7 of 9 against
  3), and above it Gouraud stays wrong as before (3 of 9 to 133.25):
  placement isn't divide 1's whole fault.** **Test G21 (2026-10-24,
  `glesfeat-0027`): Dell's frame with its render target inside the safe
  region (drawn straight into the screen at `0x0`) is right at 99.67 and
  133.25 MHz / 1, 20 of 20 each, where the same frame with the target
  outside (`0xB27000`) is wrong (3,186 and 9,567 words a render): above
  74.75 MHz at divide 1, where the 3D core writes its picture decides it
  (one scene, one X51v so far); the render alone 4.69 ms at 133.25 / 1
  against 12.51 at Windows Mobile's clock. The display reading the
  picture while it is drawn does no harm (right with the fetch on or
  paused). Run G21 (`gpubench-0083`): C71's five scenes drawn into the
  screen at Windows Mobile's clock exact, 0 of 300 renders off, up to
  4.5% faster than off screen, saving the 2D copy (4.6-4.8 ms).**
  **Run GC2I (2026-10-24, `gpubench-0084`): GC2's six 16-bit kinds with
  their target inside the safe region, Gouraud included, are right at every
  divide-1 setting from 74.75 to 133.25 MHz (20 of 20 each, both
  directions; the two 32-bit Gouraud kinds left outside wrong as before);
  a soak at 99.67 / 1 ran 20 minutes, 4,536 renders, all right; at 133.25
  / 1 one pixel was wrong after 187 s (732 renders right), in the kind
  reading a 32-bit texture from outside: 99.67 / 1 holds, 133.25 / 1 not
  yet proven.** **Run GC2J (2026-10-24, `gpubench-0085`), 20 minutes each:
  108.33 / 1 0 wrong of 4,564 renders; 117.00 / 1 1 of 4,578; 133.25 / 1
  10 of 4,697, every one a single pixel in one patch beside a textured
  quad's corner over Gouraud (FT2), the texture inside the safe region or
  outside alike; every other kind right throughout.** **Run GC2K
  (`gpubench-0086`), 40 minutes at 133.25 / 1, 9,510 renders: the patch
  needs a textured quad (untextured 0 of 1,902) with its bottom-left corner
  near (40, 600) (moved to (61, 571): 0 of 1,902); its bottom edge at 605
  instead of 600 made it 20 times as frequent (11% of renders); always one
  pixel, in the tile beside the corner's, the tile row's last four rows;
  the texture's format no factor.** **Run GC2L (`gpubench-0087`): with the
  region array's tiles in reverse order the patch never appeared (0 of 784
  renders at 117.00 and 133.25 / 1) where row order went wrong in about 11%
  at 133.25; the target's address (`0xF000`, `0x17000`, `0x0`) made no
  difference; 117.00 / 1 clean apart from the patch (12 of 13,932 renders,
  all patch ways).** **Run GC2P (`gpubench-0092`): one fault at divide 1,
  two triggers: a finished tile's last rows (rows 5-15 of its 16, rising
  to the last) go wrong while local memory is busy at its write-out (the
  next tile's texture fetch at 133.25 / 1: 40 of 5,850 renders in row
  order; 2D copies queued behind the render at 108.33 / 1: 369-502 words
  every round, FT3); reversing the tile order moves the patch to the corner
  drawn next (16 renders), columns reversed leaves 1, and no order helps
  FT3.** **But Dell's own driver draws Gouraud right at 74.75 MHz / 1** (`rp-wmprobe`, three runs, 2026-10-23: 20 of 20 identical to divide 2), with `COREPLL` `0xBC1`, `M24CLK` 0, `LMTIM` `0x62272` and `LMREFRESH` `0x490` (ours `0x48F`): the fault at divide 1 is in our renderer's set-up or objects, not the silicon (test G, `rp-glesfeat`, 2026-10-23: the TA writes our very list word and object for a Gouraud triangle, so the layout is not it; **a strip of four corners, which the TA writes as one 22-word object with layout 5 and a two-triangle mask, was right in 5 of 20 renders at 74.75 / 1 with 33 wrong words in all, against 0 of 20 and 16,338 for our two triangle objects**; **test G2: the cause is our flat background object: with the GLES clear's colourless object (layout 0) underneath instead, not one word inside the picture was wrong in 80 renders at 74.75 / 1, by our direct path as by the TA; with ours, about 820 a render, each wrong word grey (its channels equal)**; the clear's own colour at the edge is the one thing left); **test G3: no other background fixes it (Dell's opaque clear, screen-sized, z 1.0, Gouraud, none: all wrong at 74.75 and 99.67 / 1), Dell's exact sequence through the TA is wrong under our register set-up; nothing rendered at 133.25 / 1, the test's CL3 (ADR-0035: CL2 always); **test G4: Dell's whole render table (and his RGB565 target) doesn't fix it either: the difference lies outside the render table**; **Dell's driver read on the PC (2026-10-23): `LocalMemoryPriority` and `LocalMemoryTimeout` are not registers** (a background thread's priority and its sleep, which puts the local memory into self-refresh when idle); every system-block register Dell writes matches ours except **`NINTPW` (`0x14`), which Dell sets to the cycles in 290 ns at every clock change (21 at 74.75 MHz) where ours stays 28**; at a clock change Dell turns both MBX clocks off before writing the divider and the display planes off around it; the 3D core's own state outside the 17 words read under Windows Mobile is the next measurement (`rp-wmprobe` v3)). Details: 2700g.md.
- **The video plane's scaler (2026-10-24, static read and run S1):** Dell's
  drivers never write it (`SPOCTRL` `0xC8030000`, both directions bypassed;
  `SVCTRL` and the 14 coefficient words 0 under Windows Mobile); scaling
  needs a filter loaded first (Linux mbxfb's set, `layers::SCALER_FILTER`,
  C73): **proven on the X51v 2026-10-24 (`p9c`): up 2x and down to 1/2
  and 3/4 right; `SPOCTRL` bit 27 needed when shrinking** (underrun and
  streaks without it). Dell's own 2x for 320 x 240 programs is
  `GDRCTRL`'s pixel and line doubling (bits 31, 29), untested by us.
- **2D FIFO (2026-10-21, `rp-gpubench` run FF):** its level reads full on
  the device (8 bytes of room in 2,040 at the lowest); a full-screen 32-bit
  fill takes ~6.3 ms at WM's clock, so a busy engine makes room well
  inside the driver's 100 ms.
- **32-bit renders under a 16-bit screen (run P2L, 2026-10-21):** 0 wrong
  bar pixels in 1,000 renders (153.6 million), against ~47 a render with
  the screen at 32 bits (ADR-0023's vote B); **version 3: 0 wrong or
  differing pixels in 1,000 renders (307.2 million)**, confirmed.
- **2D rectangles:** a fill with `y1 < y0` writes far past its surface on
  the device (confirmed 2026-10-21, `rp-gpubench` run P2L version 2); the
  driver refuses empty, inverted and past-13-bit rectangles.
- Panel brightness: PWM0 (GPIO17) up to full duty, Windows Mobile caps it
  at 137/216; PWM1 drives the panel's VCOM (Dell's value 70/216).
- Pages: [2700g.md](2700g.md), [backlight.md](backlight.md),
  ADR-0022/0023/0024/0026.

### Audio (WM8750)
- Control: standard I2C at `0x1A` (fast mode works; standard too), **through
  the gate GPIO82**; power CPLD `0x1C` bit 1.
- Data: I2S master (bit clock and frame from the PXA), 48 kHz proven, by
  DMA (channel map `AUDIO`).
- Outputs: headphones (jack GPIO83) and speaker (amplifier CPLD `0x14`
  bit 1). GPIO83 is high only while the codec is powered, so it falls
  when the codec goes off with headphones in (an edge of our own making),
  and a plug while audio is off likely makes no edge (emulator-modelled;
  device run S1 pending). RetroPane's session rate is **44.1 kHz**
  (ADR-0032 A1: `SADIV` `0x0D`, codec register 8 = `0x020`), **measured at
  44,317 frames a second** (+0.49%, 2026-10-19, against the OS timer).
  `SADIV` is a choice of fixed settings, not a divider, and none is exact:
  `0x0C` 47,795 (-0.43%), `0x0D` 44,317 (+0.49%), `0x1A` 21,959 (-0.41%),
  `0x34` 10,978 (-0.43%); undocumented `0x0B`, `0x0E`-`0x10` act as
  `0x1A`.
- **Traps measured on this board:** the I2C controller can't do
  address-only transfers (a byte with START and STOP never finishes; after
  an acknowledged address a master abort holds SCL low); a switch-on of I2S
  within ~645 ms of a switch-off is ignored (keep I2S on through a session).
- **The audio DMA may stop and restart while I2S runs** (2026-10-19, ADR-0032
  A6's bench, 15 of 15): a stop at a STOP link, left 0-1,000 ms, restarted
  by `DDADR` and `RUN` moves 8 us later (once the FIFO has emptied); I2S stays
  on. An empty FIFO sets `SASR0` `TUR` (bit 5, sticky until `SAICR`), which
  doesn't block the restart.
- **While the engine runs a descriptor whose link has STOP, `DDADR` reads
  that link: the STOP bit and the next descriptor's address bits**
  (2026-10-19, ADR-0032 A6's listening run: the ring's race told apart and
  restarted 5 of 5 by it, none failed). Restarted at once after such a
  stop, the engine played on with no measurable gap (0 us).
- Dell's private equalizer controls: `WAV1:` codes `0x1D2000`/`0x1D2004`.
- Page: [audio.md](audio.md).

### Touch (TSC2046)
- SSP1 at its alternate pins; chip select GPIO24 (as a plain GPIO); pen
  down GPIO94.
- Readings settle slowly and creep while the stylus lands; RetroPane's
  filter decides by pressure (replay-tested). Page: [touch.md](touch.md).

### Keys
- PXA27x keypad controller: a matrix (Wi-Fi, Record, Calendar, Contacts,
  Mail, Home, the D-pad) and the power button on GPIO0. A short tap can be
  missed by polling; the controller's latched bits catch it. Page:
  [keypad.md](keypad.md).

### SD card
- PXA27x MMC controller, 4-bit, DMA; detect GPIO12 (low = in), write
  protect GPIO27 (high = locked), power CPLD `0x16` bit 0.
- A multi-block write's CMD12 needs the stop-transmission bit; the card
  stays busy after it (poll CMD13).
- Blocks past the end are refused by the driver before any command
  (confirmed 2026-10-21). Reading the last block makes the card report
  `OUT_OF_RANGE` under 76 us after the read, kept until a command collects
  it (confirmed 2026-10-21, 20 of 20; a CMD13 straight after the read is
  too early). The driver polls CMD13 for it, ~1 ms at most. Not every
  such read raises it (guard run version 6: 30 of 31 within 5 CMD13s; the
  one after refused writes didn't, and the next reads were fine). Page:
  [sd.md](sd.md).
- A single-block write's programming (the card in `prg` after the data)
  took up to 61.5 ms on the owner's card (the card log's slowest of 333
  writes, 2026-10-22), and 90.3 ms in a later run of 852 (`rp-today`, the
  same day). The card log now waits for it only before the
  card's next command (quality audit C59). How it splits into transfer
  and programming isn't measured yet. Page: [sd.md](sd.md).

### CompactFlash slot
- PC Card socket 1; power CPLD `0x16` bit 1 then `0x10` bit 12; RESET CPLD
  `0x18` bit 1; ready GPIO84; presence CPLD `0x04` bit 1 (GPIO13 sticks).
  No earlier port got CF working. Page: [cf.md](cf.md).

### USB
- **Device**: the PXA27x UDC through port 2's transceiver; **the D+ pull-up
  is hard-wired**, so the PC sees an unplug only by SE0 from the host
  controller (`UP2OCR.HXS`) or a board reset; double-buffered bulk
  endpoints by DMA. Page: [udc.md](udc.md).
- **Host**: the OHCI controller; its port 2 shares that connector (the
  loader uses it for the unplug). Windows Mobile has **no USB host
  drivers** on the X51v; power to a device (VBUS): unknown, to check
  ([bluetooth.md](bluetooth.md),
  [../design/peripherals.md](../design/peripherals.md)).

### Serial, IrDA, Bluetooth, Wi-Fi
- FFUART: the debug console (the loader's log); STUART: IrDA; BTUART:
  the **TI BRF6150** Bluetooth 1.2 radio at 921,600 baud, its PCM voice port
  and its own I2C possibly wired to the codec ([bluetooth.md](bluetooth.md));
  Wi-Fi: the ACX100
  on socket 0, reset GPIO89, interrupt GPIO107. Interrupt lines:
  [interrupts.md](interrupts.md).

### CPU core (XScale)
- A multiply costs by its `Rs` operand's size: base when it fits in 15
  bits, +1 cycle up to 27 bits, +2 cycles beyond (measured 2026-10-22,
  `rp-probe` `mul_chain_*`, confirming Intel's early termination); LLVM
  doesn't schedule around it. Page: [cpu.md](cpu.md).
- Stores don't allocate D-cache lines: clearing memory not in the cache
  misses every store (~4,600 cycles for 8 KiB), clearing it after a read
  doesn't. Line allocate (c7, c2, 5) used to zero memory hung the device
  (2026-10-22, not isolated): kept out of data paths. Page: [cpu.md](cpu.md).

### Clocks, voltage, reset
- Seven operating points (91, 104, 208, 312, 416, 520, 624 MHz) with core voltages
  1050-1574 mV set through GPIO3/4/38; memory clocks from `MDREFR`.
- A `CCCR` write alone changes no clock (confirmed 2026-10-21, `rp-clocks`
  run F); `CLKCFG` bit 1 (F) reads back as last written.
- Reset: Windows Mobile's software reboot drives GPIO99 high (after
  `PCFR` and GPIO15); a hardware watchdog reset lands on Dell's
  factory-restore prompt (press Mail for No). Pages: [clocks.md](clocks.md),
  [README.md](README.md).

## 6. Open questions

The bits and lines marked unknown or guess above; the BRF6150's power
lines and whether its PCM port reaches the codec; VBUS on the USB connector; GPIO11, 86/87 and 96's jobs;
the 2700G's TA vertex formats for fog and a second texture unit. Each is
listed with its next step on its own page and in
[../quality-audit.md](../quality-audit.md).
