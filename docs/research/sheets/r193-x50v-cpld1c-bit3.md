# Sheet: R193, one watched X50v run setting CPLD `0x1C` bit 3 (rule 0, before code)

**Built 2026-11-13 (the owner's "go, build way A"), emulator-proven; the
X50v run waits for his go.**

The X50v's start-up log (2026-11-13, report
[docs/reports/runs/2026-11-13-loader-0001-x50v.md](../../reports/runs/2026-11-13-loader-0001-x50v.md),
the whole log in `local/cardlog/loader-0001.txt`;
docs/hw/udc.md "The X50v") ruled out the transceiver: `UP2OCR`
`0x00020000` and `UDCCR` match the working X51v. In 5 minutes there were
0 setups, and the 37 resets came one per loader replug, so the PC most
likely never saw an attach. CPLD `0x1C` bit 3 was off at every stage.
This sheet is the first run that **switches a board bit on purpose**, so
it needs the owner's go (no-blind-board-bits rule: one unknown bit per
run, the owner watching). The owner's "go, write the sheet" (2026-11-13)
covers this sheet only.

## What others do (findings, not the answer)

- **Dell, X50v A12 `usbfndrv`** (REA, `local/re/ghidra-c/x50v-a12_usbfndrv.dll`):
  - `0x257b45c(on)` sets or clears mask `8` in shadow slot 7, which is
    register `0x1C` (slot order `0x02, 0x10, 0x12, 0x14, 0x16, 0x18, 0x1A,
    0x1C`, docs/hw/cpld.md).
  - On attach (`0x257b390`'s flag `0xAC00148E`), the driver's thread
    (`0x257abf0`) sets bit 3 **and then** starts the controller
    (`0x257a60c`). On detach and at power-down it clears bit 3.
  - "Attached" comes from a power-status call (`Ordinal_1581`, the values
    `0x200000`/`0x400000`), not from a GPIO the driver reads itself, so
    Dell's code doesn't name the X50v's cable pin.
- **The X51v A08 driver** never touches bit 3, and its D+ pull-up is
  hard-wired. On the X51v, `rp-reconnect` (2026-10-11) switched bit 3
  off→on with no visible effect. Windows Mobile's shadow reads `000F`
  docked and `0007` out of the dock.
- **AxDroid's X50 board file (Linux)** sets bit 3 once at boot ("USB
  power?") and never clears it.
- **Ours until now:** the loader keeps `0x1C` as Windows Mobile left it
  (C39's board record), and it puts the recorded value back on every
  return.

## The ways (for this run)

| | Way | For | Against |
|---|---|---|---|
| A | **Opt-in by HaRET's command line:** a second folder on the CF card, `RetroPaneX50v/`, with its own `haret.exe` and a `startup.txt` that carries `set cmdline "rp.x50v-usb-bit3"` and loads `RetroPane/retropane.bin`. The loader reads the command line HaRET passes (the ATAG list whose address HaRET puts in `r2`, `ramaddr + 0x100` = `0xA8000100` with our script; read at entry, before anything can overwrite it) | the X51v, booted from the same card's `RetroPane/`, behaves exactly as today; the X50v run is chosen by which folder the owner starts; no new file read before USB | the loader learns to read ATAGs (new code, host-tested on a built list); a 143 KB second copy of `haret.exe` |
| B | A marker file on the CF card (`RetroPane/x50v-usb.txt`) | no ATAG code | the same card goes back into the X51v afterwards and would switch bit 3 there too, unwatched. Rejected |
| C | Detect the X50v from the hardware and act on it | the right end state | no X50v/X51v fact has been proven to tell the two apart yet (both PXA270 at 624 MHz, both 2700G, both VGA); a wrong guess flips a bit on the wrong board. Later, once a run finds one |
| D | Set bit 3 at the start, as AxDroid does | simplest | no before/after in the same run: a change in the PC's behaviour could be the reboot. Rejected for the order: B-then-A in one log |

**Proposed: A, with before and after in one run.** How the run goes:
1. **0-30 s:** the loader runs exactly as today with bit 3 as found. This
   is the baseline, logged.
2. **At 30 s:** it sets bit 3 through the `Cpld` API (the bus write, and
   Windows Mobile's shadow slot 7, so the table stays true). It logs
   `CPLD 0x1C bit 3 on (opt-in rp.x50v-usb-bit3)` with the value written.
   It then updates the board record, so a return from an image keeps the
   bit on (C39's put-back would otherwise undo it). Then one replug, so
   the PC sees a fresh attach. Order: bit then controller, as Dell does.
3. **From 30 s to 5 minutes:** the log goes on as now (setups,
   configured, layout). The heartbeat adds `GPLR0-3`, and any change in a
   GPIO level gets its own line, at most one every 50 ms.
4. **At about 2 minutes:** the owner pulls the USB cable out for 10 s and
   plugs it back in. The GPIO lines then show which pin is the X50v's
   cable detect, which Dell's code doesn't tell us. This answers a second
   question in the same run with no extra board change.
5. **On the PC:** I record USB device arrivals during the run from my own
   shell, hidden, with no window. That separates "the PC saw an attach"
   from "the X50v saw a setup".

Why this beats the originals:
- Dell's way needs a cable pin we don't know yet.
- AxDroid's way can't tell its effect from the reboot's.
- Ours gives before and after, the PC's side and the cable pin in one
  5-minute run, and it can't touch the X51v.

## The ADRs' rules (`docs/adr-areas.md`: boot/loader)

- **ADR-0012** rule 1: the `udc` layer stays polled and unchanged; the
  replug is the existing one. Rule 4: the development loop is unchanged
  without the opt-in.
- **ADR-0013** rule 2 and **ADR-0014** rule 3: the start-up log's writes
  stay as built (reserved slot, sector writes only, closed before the
  drive serves).
- **ADR-0017** rule 4: the command line holds no secret; the loader
  accepts exactly one known word and ignores the rest. It is parsed
  bounded: the ATAG walk stops at 64 tags, a tag's size is checked against
  the 16 KiB list, and a malformed list counts as "no option".
- **ADR-0036/0037:** unchanged.

## How it is proven before the device

- **Host tests of the ATAG reader:** a real list, an empty one, a missing
  `ATAG_CORE`, a size running past the end, a command line with the word
  among others or as part of a longer word (which must not match).
- **The emulator:** the same `emu-cardlog` run with an ATAG list holding
  the opt-in. It checks that the log shows bit 3 set at 30 s, the shadow
  slot updated and the record kept. A run without the opt-in must show no
  CPLD write at all (a mutant that always sets it fails).
- **`cargo xtask ci`** on the commit; the full emulator suite (~1 h).

## Stop rule (one decisive run)

- **Setups or a PC arrival after 30 s:** the lead is proven. Next: the
  X50v's board detection (way C) and Dell's attach/detach tracking on the
  pin this run finds.
- **Nothing after 30 s:** bit 3 alone is not the X50v's pull-up. The lead
  closes; it is not retried with timing variants. Next leads, in order:
  - Dell's power steps (`KernelIoControl` `0x010100A0`/`0x010100A4`, read
    first).
  - The X50v's own Windows Mobile 2003 driver (not yet read).
- **Stopping at any moment is harmless:** a soft reset brings Windows
  Mobile back, and its own driver owns bit 3 from then on.

## What would overturn it

- HaRET on Windows Mobile 2003 doesn't pass `cmdline`, or the loader's entry
  doesn't keep `r2`. Then the log's first line says `opt-in: none`, nothing
  is set, and the run costs 5 minutes. Fallback: way B with the owner
  deleting the marker before the card returns to the X51v.
- Bit 3 powers something else on the X50v (it's Dell's USB bit on both
  units, so this is unlikely). The log and the owner's eyes cover the
  30 s after it is set; a soft reset undoes it.
