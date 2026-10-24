# Inherited values: every Dell, Windows Mobile and Linux number the code uses

**Status: first sweep, 2026-10-24 (reminder R9; the 2026-10-20 plan's step
3, never done until now).** The owner's rule: a value copied from Dell,
Windows Mobile, Linux or AxDroid is a claim, never a ceiling; each is either
measured by us (and beaten where it can be), ruled on by the owner, or a
hardware fact confirmed by use. **The sweep:** every `const`/`static` in the
shipping code (`drivers`, `hal`, `services`, `kernel`, `boot/loader`; not
test images) whose value or the comment above it names one of those
sources: **100 found, 6 tagged `// inherited:` in the code.** Test images'
copied values (`rp-gpubench`'s `CS3_WM`, Dell's frame in `rp-glesfeat`...)
are measurement tools and aren't listed. Values a component holds without
naming a source can still be inherited: later sweeps widen the net.

**Next for R9:** each value in group A tagged `// inherited: <this page's
row>` in the code (the policy lint's form, quality audit D1), so the code
itself says what is unproven; then each row's test run or folded into a run
(the reminders hook brings them up).

## A. Copied and never measured by us (the work)

| # | Value (where) | Source | What a test could gain | The test | Status |
|---|---|---|---|---|---|
| A1 | The flash's read timing, `MSC0` CS0 `0x38D8` (kept as found; memory-timing.md) | Dell's OAL | boot and program loading from flash | memory-timing.md section 9 (read strobe only, ID checked each step) | R6 |
| A2 | The CPU bus to the 2700G, `MSC1` CS3 `0x2484` (`CS3_WM`), only RDF stepped (`rp-cs3bus`) | Dell | 2D and upload speed (the bus caps them near 33 MB/s) | every RDF, RDN, RRR combination | R3 |
| A3 | The CPLD's timing, `MSC2` CS4 `0x7FFC` (the slowest) | Dell | microseconds a day | reads stepped unattended, writes watched on the backlight switch (the owner: test it, every gain counts) | R8 |
| A4 | The CF socket's timings `MCMEM`/`MCATT`/`MCIO` (WM's table, one for every card) | Windows Mobile | ~2x CF reads (the card rates 80 ns, WM programs 260) | storage-speed.md's step test, per card from its CIS | R7 (the owner's decision) |
| A5 | `PANEL_ID_SETTLE_US` 1,000 us (`drivers/cpld`) | Dell, AxDroid | ~1 ms of boot | the shortest wait that reads the panel ID right, 100 times a step | open |
| A6 | Audio settle times: 75 ms VMID charge (`wm8750`), 1 ms after codec power, 1 ms after I2S, 70 ms amplifier (`services/audio`) | Dell's `wavedev.dll` | ~145 ms off every audio start, if no pop | the shortest of each with no pop, the owner listening (speaker only) | tagged; open |
| A7 | `MINTENABLE` `0x800F` (`mbx3d-render`) | Dell | fewer 2700G interrupts, fewer CPU wake-ups | our minimal enable set, run P18's way | open |
| A8 | The 3D state words `TSP_DELL` `0x22200E80`, `TEX2_OPAQUE` `0xD6886610`, `TEX2_ALPHA` `0x86084610`, `PM_6D8` `0x09000000`, `EPSILON` (`mbx3d`, `mbx3d-render`) | Dell's GLES driver | correctness and speed of shading (GC2F found `0x61C` changes Gouraud's rounding) | each word's bits against our own needs, GC2F's way | open |
| A9 | The 2700G memory's refresh and timing `LMREFRESH`/`LMTIM` per clock (ADR-0035) | Dell | bandwidth at each clock | refresh period stepped at each clock, GS's soak rules | open |
| A10 | The scaler filter `SCALER_FILTER` and `SPOCTRL` bit 27 (`mbx-display` layers) | Linux mbxfb, Windows Mobile | sharper shrinking | the coefficients' format, a computed set | R15 (C70, C73) |
| A11 | The CF slot's power sequence waits: a 10 ms step, 308 ms off, 100 ms on (`boot/loader`) | Windows Mobile | ~0.4 s off every CF power-up | needs the supply's ramp seen (a scope, or the slot's power-good GPIO timed); not shortened blind (back-powering a card through its pins) | open |

## B. Copied, then measured, beaten or ruled (done)

| Value | Source | What we did |
|---|---|---|
| The 2700G's clock 99.67 / 2 (`clock::Speed::WM`) | Windows Mobile | beaten: 149.50 / 2 proven, 108.33 / 1 soaked (ADR-0035, ADR-0034 P1 = C) |
| The display PLL `DISPPLL` `0xEC5` (56.4 Hz) | Windows Mobile | measured: 56.5-62.5 Hz across the panel's rating (P17, the owner's ruling) |
| The CF card's reset hold, 100 ms | Windows Mobile | beaten: 100 us from the CF specification (`drivers/cf`) |
| The backlight PWM `0xD7` period | Windows Mobile | replaced by our own 432-count cycle at the same 30 kHz (`pxa-pwm`) |
| The gamma table `WM_GAMMA` | Windows Mobile | kept only as the reference; the screen's own calibration replaces it (`rp-display-cal`) |
| The SDRAM controller `MDCNFG`/`MDREFR` | Dell | ruled: kept Dell's for every X51v (the owner, 2026-10-19, any-Axim rule; memory-timing.md) |
| I2C at 400 kHz (`pxa-i2c`) | Windows Mobile | the PXA27x I2C's top rate (fast mode): a hardware limit, not a choice |
| The keys' repeat and hold times (`pxa-keypad`) | Windows Mobile | kept by the owner's rule: keys behave like Windows Mobile |
| WM's planes region at 0-`0xFC000` (`vram-map`) | Windows Mobile | superseded: ADR-0034 amendment P puts the screen at 0 inside the safe region |

## C. Hardware facts, confirmed by use (nothing to beat)

Register addresses and bit positions (`blit2d`, `mbx3d`, `mbx-display`,
`cpld` registers, `pxa-i2c` bits), the board's GPIO numbers (`boot/loader`:
USB pull-up, detect, CF ready, reset, SD lock, from AxDroid), HaRET's hand-off
addresses and Linux's machine number, the voltage pins (3, 4, 38), the
2700G's register base, the PE format's machine codes. Each is exercised by a
device run that would fail if it were wrong; the verify-yourself rule is
met by those runs, recorded in docs/hw.
