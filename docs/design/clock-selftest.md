# The 2700G clock self-test (draft for the owner's vote, 2026-10-22)

**Status: the four questions voted by the owner (2026-10-22): 1A, 2A,
3A, 4A** (all as recommended; recorded in
[ADR-0035](../adr/0035-2700g-clocks.md), proposed). Nothing here is built
yet. It makes the outline in [2700g-full-potential.md](../research/2700g-full-potential.md)
("Past Dell's limits: the per-device self-test") concrete, for the
clocks ADR (ROADMAP step 3). The four questions at the end are for the
owner's vote.

## Why it's needed

The soaks proved **one** X51v: 0 BAD at 99.67 MHz and at 133.25 MHz CL2,
the cores at divide 1, 20 minutes each, warm and cold (runs GS,
[2700g.md](../hw/2700g.md)). Another X51v's 2700G or its local memory may
have less margin: 133.25 MHz is a third past the memory's rated 100 MHz.
The owner's any-Axim rule: a setting past Dell's ships only with a test on
each device and a safe fallback, never as a constant tuned on one unit.
Windows Mobile's setting (99.67 MHz, the cores at divide 2) is the
fallback: every X51v has run it for years. Dell's choices are leads, not
proof (the cores' 75 MHz cap turned out to be far below what this chip
does), so the fallback is "known to work everywhere", not "best".

Why the risk is acceptable here when the SDRAM's timing wasn't: the 2700G's
memory holds pictures, textures and commands, not the user's files. An
error shows as wrong pixels or a GPU hang, which the test and the guard
catch, not silent data loss. No voltage is changed (none can be), so a
clock too fast for a chip makes errors before it does harm.

## What it does

### 1. The test

A ladder of settings, slowest first, each run only if the one below it
passed:

| Step | Setting | Run so far (one unit) |
|---|---|---|
| 0 | 99.67 MHz, cores / 2 (Windows Mobile's: the fallback) | every run |
| 1 | 99.67 MHz, cores / 1 | GS warm and cold: 0 BAD |
| 2 | 108.33 MHz CL2, cores / 1 | G5: right in short checks |
| 3 | 117.00 MHz CL2, cores / 1 | G5: right in short checks |
| 4 | 133.25 MHz CL2, cores / 1 | GS warm and cold: 0 BAD |

Each step runs run GS's round for a fixed time (question 3), the chip
warmed first by two minutes of the same work at Windows Mobile's clock:

- the CPU's bus to the 2700G: a 1.2 MB flush, every word read back;
- **all** free 2700G memory written and read with a pattern and its
  inverse (GS covered 4 MiB; the shipped test covers everything the OS
  isn't using);
- a full-screen 2D fill and copy, checked;
- the 3D scene rendered, pixel-exact against its picture at Windows
  Mobile's clock;
- the CPU streaming its own SDRAM meanwhile;
- `COREPLL`, `LMTYPE`, `LMREFRESH`, `LMTIM`, `MBXCLK` read back every
  round (V12's lost writes);
- the display scanning out a 16-bit picture all through, its underrun flag
  (`DINTRS` bit 4) and output signature (`DSIG`) read (G4's dark display).

**The first BAD ends the ladder:** Windows Mobile's clock goes back, a
control round runs there, and that step and every one above it are
marked failed. Nothing is retried silently.

**A hang counts as a failure.** Before each step a marker is written to
storage ("testing step N"); a passed step replaces it with its result. If
the device resets in the middle (the watchdog), the next start finds the
marker without a result, marks step N failed, and never runs it again
unattended. (On the SD-card stage a watchdog reset currently lands in
Dell's restore-factory-settings prompt, which a user must not be asked to
answer: before the self-test ships to anyone, that path needs its own
answer, in the flash-install work or the loader.)

**Its content must cover every kind of drawing (2026-10-22, run FT2 at
133.25 / 1):** an RGB565 Gouraud scene failed at a setting where GS's
32-bit textured scene passed 40 minutes. The round renders every target
format the OS uses (RGB565, dithered RGB565, ARGB8888), Gouraud and flat
and textured fills in every texture format, blending, and 2D copies and
fills in both depths, each against its picture at Windows Mobile's clock.

### 2. The choice

The setting used is chosen from the steps that passed (question 1): the
fastest, or one step below it for margin.

### 3. The guard while running

- Now and then, while the GPU is idle, a small block of spare 2700G
  memory is pattern-checked, and a small known render is compared with
  its picture.
- Any check failing, a GPU hang the driver recovers from, the underrun
  flag set outside a clock change, or a reset nobody asked for: the clock
  drops straight back to Windows Mobile's, the event is logged, and a
  re-test is scheduled. A quiet notice in Settings says so; no pop-up.

### 4. Re-tests

- After an OS update that changes the GPU driver or the clock code.
- After the guard trips.
- When the device isn't the one that was tested: on the SD-card stage the
  card can move to another X51v, or to the owner's X50v. The result is
  stored with a fingerprint of the device (a hardware identity such as the
  Wi-Fi chip's MAC address: to choose and verify), and a mismatch means
  Windows Mobile's clock until a new test.
- Not on temperature: the cold soak gave the same numbers as the warm
  one.
- On request: Settings, "Graphics speed: Automatic (tested) / Windows
  Mobile's", the result shown, a "test again" button.

### 5. What it doesn't decide

- **Battery:** a faster clock costs more power while the GPU works. The
  power lines (phase 9) decide when the device runs slower to save
  battery; the self-test only finds each device's safe ceiling.
- **The CPU:** overclocking the PXA270 is a separate gate line (CPU C2).
  The same design would carry it, as its own ladder.

### 6. How it's checked before it ships

- Emulator: the ladder, the markers, the choice and the fallback, with a
  build switch that makes a chosen step fail (the models never fail at
  any clock: emulator.md), and a reset mid-step (the marker path).
- Device: the whole ladder on the owner's X51v, then on the X50v when it
  runs RetroPane.
- A mutant per rule: a step run after a failure, a hang not marked
  failed, a result kept on a fingerprint mismatch.

## For the owner's vote (voted 2026-10-22: 1A, 2A, 3A, 4A)

1. **Margin.** A (recommended): use one step below the fastest that
   passed. 133.25 passing means running at 117 (3D about 7% slower than
   at 133.25, from G5's short checks), with a whole step of margin for the
   chip on a bad day. B: use the fastest that passed: all the speed, the
   test's length as the only margin.
2. **When it runs.** A (recommended): in the background the first time
   the device sits docked with the screen off (the first night), at
   Windows Mobile's clock until then; nobody waits. B: a screen at first
   boot, about 25 minutes, docked, skippable. C: only when the user asks
   in Settings.
3. **How long a step.** A (recommended): 5 minutes a step (about 25
   minutes for the ladder with the warm-up). GS's soaks were 20 minutes a
   setting, with no error at any minute. B: 20 minutes a step (about 1.5
   hours, overnight only). C: 2 minutes (a quick check, less margin).
4. **The ladder.** A (recommended): the five steps above (fine choice of
   margin). B: only the two soaked (99.67 / 1 and 133.25 / 1): shorter,
   but with question 1's A the margin step is then 99.67 / 1, giving up
   most of the gain.
