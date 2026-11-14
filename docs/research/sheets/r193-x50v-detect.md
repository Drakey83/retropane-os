# Sheet: R193, the loader tells the X50v from the X51v (rule 0, before code)

The owner's "Go" (2026-11-13) on the next step after
[r193-x50v-cpld1c-bit3.md](r193-x50v-cpld1c-bit3.md): CPLD `0x1C` bit 3 is
proven the X50v's USB switch (report
[2026-11-13-loader-0002-x50v-bit3.md](../../reports/runs/2026-11-13-loader-0002-x50v-bit3.md)).
The loader should set it on an X50v by itself, with no special folder or
command line. For that it has to know which unit it runs on.

## What others do (findings, not the answer)

- **Dell:** a separate ROM per model; nothing asks. Dell's X51v `AximSDK`
  (REA) has `GetProcessorModel`, `GetFlashChipModel` and `GetLCMModel` but
  no board or model call: each build runs on one model only.
- **Linux (AxDroid):** one board file per machine, chosen by the machine
  number HaRET passes (`r1`). HaRET's own list has a single X50/X51
  entry, so the number can't tell them apart (it is the same for both).
- **Modern:** a board-ID strap or EEPROM read at boot (device tree
  chosen by it).

## The ways

| | Way | For | Against |
|---|---|---|---|
| A | **CPLD `0x06`, a status input with no latch** (it never reads back a write; no shadow slot): X51v `0x0002` in 15 logged runs over weeks (docked and not, CF card pulled; `local/rp-storfast-C*.txt`, cf.md), X50v `0x000B` in its 3 boots (cradle and cable) | a read with no side effect, already in the start-up log; a strap-like value that never changed on either unit | one unit of each model measured: a different board revision could read otherwise. So only the exact X50v value counts as an X50v, and anything unknown keeps today's behaviour |
| B | CPLD `0x14` (X51v `0x0002`, X50v `0x000A`) | differs too | has a shadow slot (amplifier and audio latches): a status read mixed with written bits. Logged as a second witness, not used |
| C | Windows Mobile's CPLD table contents | already read | Windows Mobile's state, not the board's (bit 3 itself moves with the cradle) |
| D | The flash chip (CFI query on CS0) | a real hardware difference (X50v StrataFlash) | a CFI query is a write to the flash's command interface: a board change the loader shouldn't make for this |

**Proposed: A.**
- `rp_hal_x51v::board::unit(cpld06)`: `0x000B` → X50v, `0x0002` → X51v,
  anything else → unknown.
- The start-up log names the unit and the value it was read from.
- On an X50v, at the first start after HaRET, the loader sets bit 3 before
  USB, exactly as `rp.x50v-usb-bit3=0` does. The board record keeps it for
  returns.
- On an X51v or an unknown unit nothing changes; the command-line opt-in
  still works on any unit.

Why ours beats the originals: Dell needed a ROM per model and Linux a
machine number that HaRET doesn't distinguish; ours is one loader for both
units, chosen by a read-only board input. Unknown boards fall back to
today's behaviour, not a guess.

## The ADRs' rules (`docs/adr-areas.md`: boot/loader)

- **ADR-0012** rule 1: the `udc` layer is unchanged; rule 4: the
  development loop on the X51v is unchanged (its `0x06` reads `0x0002`).
- **ADR-0013, ADR-0014:** the card log as built.
- **ADR-0017** rule 4: no secret involved.

## How it is proven

- Host tests of `unit` (both values, unknown, the bits that differ).
- The emulator (its CPLD reads the X51v's `0x0002`): the start-up log says
  `unit: X51v` and nothing is switched (the existing check); with the
  opt-in the bit is still set.
- On the X50v: the loader from the CF card's `RetroPane` folder (no
  command line) configures over USB. On the X51v: the next start-up log
  says `unit: X51v` and no bit is set.

## What would overturn it

- Another X51v or X50v reading a different `0x06` (it would be "unknown"
  and keep today's behaviour: logged, so it shows up in its start-up log).
- `0x06` changing with a state we haven't tried (battery low, charging):
  the same fallback.
