# X51v CompactFlash slot (PXA27x PC Card socket 1)

> Spec v0.1, 2026-10-12. Facts only, per [`../CLEANROOM.md`](../CLEANROOM.md).
> **Sources:** (M) Intel PXA27x Developer's Manual, memory controller and
> PC Card chapters; (C) AxDroid board code (`pxa2xx_aximx50.c`, via
> `docs/research/hardware-inventory.md`); (CF) CompactFlash Association
> specification (attribute memory, CIS, True IDE / memory / I/O modes);
> (P) RetroPane's probe on the owner's X51v.

The X51v's PXA27x drives two PC Card sockets: **socket 0** is the built-in
ACX100 Wi-Fi (docs/research/hardware-inventory.md), **socket 1** is the
CompactFlash Type II slot. A CF card is a small ATA disk (or an I/O card:
Wi-Fi, modem, Bluetooth), so the driver to come is a PC Card socket layer
plus an ATA block device; the USB drive (ADR-0014) then offers it as LUN 1
with no USB work.

## Board (C, P)

| Item | Value | Confidence | Evidence |
|---|---|---|---|
| Socket | PXA27x PC Card socket 1 | confirmed (code) | (C) |
| Card detect | GPIO13, **active low: 0 = card present** | confirmed | (C); reads 0 with the owner's CF card in the slot (P, owner, 2026-10-12); WM arms only the rising edge (removal) |
| Card ready / IRQ | GPIO84 | confirmed (code) | (C); reads 1 (P) |
| Card reset | GPIO36, an output | confirmed (code) | (C); WM leaves it an output, driven 0 (P) |
| Card power | a CPLD bit (docs/hw/cpld.md: "CF power") | likely; which bit unknown | (C), Dell OAL to analyse |

## PXA27x side (M)

Memory windows, socket 1 (socket 0 is the same at `0x2xxxxxxx`):

| Space | Physical base | Use |
|---|---|---|
| I/O | `0x30000000` | True IDE / I/O-mode registers |
| Attribute memory | `0x38000000` | the card's CIS (what the card is), configuration registers |
| Common memory | `0x3C000000` | memory-mode registers / data |

Memory controller registers (base `0x48000000`):

| Register | Offset | Fields | Confidence |
|---|---|---|---|
| `MECR` | 0x14 | `[0]` NOS: two sockets; `[1]` CIT: card inserted (set by software) | likely (M) |
| `MCMEM0/1` | 0x28 / 0x2C | common memory timing: `[6:0]` SET, `[11:7]` ASST, `[19:14]` HOLD (in memory clocks) | likely (M) |
| `MCATT0/1` | 0x30 / 0x34 | attribute memory timing, same fields | likely (M) |
| `MCIO0/1` | 0x38 / 0x3C | I/O timing, same fields | likely (M) |

Pins: the PC Card strobes (`nPOE`, `nPWE`, `nPIOR`, `nPIOW`, `nPCE1/2`,
`nPSKTSEL`, `nPREG`, `nPWAIT`, `nIOIS16`) are GPIO alternate functions
(M); the probe records `GAFR*` as Windows Mobile set them, the reference
for the driver's set-up.

## As Windows Mobile left it (P, 2026-10-12)

| Register | Value | Decoded (fields above) |
|---|---|---|
| `MECR` | `0x00000003` | two sockets, card inserted (possibly just the built-in Wi-Fi on socket 0) |
| `MCMEM0`, `MCMEM1` | `0x00020508` | SET 8, ASST 10, HOLD 8 |
| `MCATT0`, `MCATT1` | `0x00020E08` | SET 8, ASST 28, HOLD 8 |
| `MCIO0`, `MCIO1` | `0x00020510` | SET 16, ASST 10, HOLD 8 |

## Card side (CF)

- Attribute memory holds the Card Information Structure: tuples from
  offset 0, on even bytes only (`CISTPL_DEVICE` 0x01, `CISTPL_MANFID` 0x20,
  `CISTPL_FUNCID` 0x21: 4 = fixed disk, `CISTPL_CONFIG` 0x1A gives the
  configuration registers' base, `CISTPL_END` 0xFF).
- A storage card is set to True IDE or memory mode through its
  Configuration Option Register; then it speaks ATA (IDENTIFY DEVICE, READ /
  WRITE SECTORS, with LBA28 addressing up to 128 GiB), 512-byte sectors.
- Reset: pulse RESET (GPIO36), then wait for READY (GPIO84) before the
  first attribute access; the pulse and wait times come from (CF) when the
  driver is written (not yet recorded here).

## Driver (`drivers/cf`, `rp-drv-cf`; emulator-verified, device run pending)

Memory mode, the one every CF storage card must support: configuration
index 0 in the Configuration Option Register (after a soft reset through
its bit 7), the ATA task file at common memory 0x0-0x7 (data at 0x0 as
16-bit accesses, the other registers as bytes), device control at 0xE
(nIEN set: the driver polls). The CIS walk (even bytes, bounded to the
first 1 KiB) must find `CISTPL_FUNCID` = fixed disk and `CISTPL_CONFIG`
with the configuration registers' base; anything else (a Wi-Fi or modem
card) is left alone with nothing written to it. IDENTIFY DEVICE gives the
LBA28 capacity (words 60-61, word 49 bit 9) and the model (words 27-46);
SET FEATURES 0x82 turns the write cache off so a completed write is on the
card (the crash-safe file system order needs that), refused by cards with
no cache, which is fine. READ / WRITE SECTORS move up to 256 sectors a
command, polled per sector (DRQ), every wait bounded.

`devices::take_cf()` (board HAL) sets socket 1's timing registers to the
values Windows Mobile used and `MECR` to two sockets / card inserted, then
hands out the attribute and common windows. The loader's `cf` command
(`cargo xtask` control frame `cf`) identifies the card read-only, and only
when GPIO13 reads a card; nothing touches the socket at start-up.

Not yet: card power (the CPLD bit is unknown; the loader relies on Windows
Mobile having left the slot powered, as HaRET hands over), the card's
ready/IRQ line, I/O-mode cards, hot swap, and the USB drive's LUN 1.

## Safety for experiments

An access to a socket window with the card unpowered, or before its timing
registers are set, can hold the bus until the card answers, which on a
device with nobody there means a watchdog reset into Dell's factory prompt.
So the probe only reads the memory controller and GPIO registers (always
safe); attribute memory is read only once power-on and detect are
understood, with the owner present.

## Open

| Question | How to settle |
|---|---|
| GPIO13 polarity (present = 0 or 1?) | probe with and without a card (owner: one swap). Hint (P): WM arms only GPIO13's *rising* edge (`GRER0` bit 13 set, `GFER0` bit 13 clear); if that is the insertion event, present = 1 and the 0 read on 2026-10-12 means an empty slot |
| Which CPLD bit powers the slot | Dell OAL analysis (nk.exe), or a careful CPLD experiment with the owner present |
| Whether WM's `CIT` reflects socket 1 | probe with and without a card |
