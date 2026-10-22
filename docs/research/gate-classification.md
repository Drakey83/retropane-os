# The gate's lines classified (the owner's rulings, 2026-10-22)

The owner's brief (2026-10-22, "Gate restructure: architectural gate +
payoff-ordered backlog"): tag every line of
[2700g-full-potential.md](2700g-full-potential.md) and
[cpu-full-potential.md](cpu-full-potential.md) **ARCH** (its answer changes
a design decision: an ADR's decision, memory layout, clock or power
settings, driver architecture, a compositor or renderer choice) or **OPT**
(faster, no design changes), one line of why each, boundary cases flagged.

**Ruled (owner, 2026-10-22), and applied the same day:** B6 yes (the
architectural lines are phase-tagged, `arch N`; the gate lifts per phase on
that phase's lines; video and power lines gate their own phases); B7
decline with the reason, no defaults (P16 and CPU V4 declined, reopened if
measurable); B10 yes (C3 and T2 ticked on the shared SRAM layout, the rest
to the backlog); B11 yes (T8 = P8, counted once). Standing rules: "the
ratchet running" stays in the lift; verify-yourself unchanged; the
compositor harness stays in the gate. The other flags stand as proposed
below. Applied: the ledgers' Kind column, [backlog.md](backlog.md),
`cargo xtask gate` (counts by phase and checks the backlog), the ROADMAP's
gate section, CLAUDE.md. The phases assigned: 2 (kernel memory: CPU V2,
V7, C3, C4), 4 (drawing: F6), 5 (the rest of the graphics, clocks and
compositor lines), 7 (video: P11, V8), 9 (power and clocks for battery,
the cursor plane for mice).

Tags: **ARCH**, **OPT**, and **META** for the three lines that are sweeps
or reads producing other lines (neither kind; proposed: done ones stay
done, P19 flagged). **B** in the last column = a boundary case, explained
under "Boundary cases".

## The 2700G (40 lines)

| Line | Status | Tag | Why | B |
|---|---|---|---|---|
| V1 | open | ARCH | Whether the local memory has its own clock decides the 2700G clock plan (gate step 3's ADR) | |
| V2 | partial | ARCH | The memory's CL against clock sets the clock ADR's settings | |
| V3 | partial | OPT | 2 and 4 bpp sources: more formats for the same drawing; 8 and 1 bpp (the ones a design could rest on) are proven | B1 |
| V4 | done | ARCH | Decided where rotation runs (2D engine, not 3D) | |
| V5 | open | ARCH | The video plane's scaler: the fact under P9 | |
| V6 | open | ARCH | The cursor plane's formats: the fact under P10's cursor | B2 |
| V7 | open | OPT | Colour keys at scan-out: a cut-out trick the compositor doesn't need | B2 |
| V8 | open | ARCH | MVED1's registers: the fact under P11 | B6 |
| V9 | partial | OPT | Second texture unit, fog, TSP bit 14: effects in fewer passes | B3 |
| V10 | done | META | The sweep that made V15-V18 | |
| V11 | done | ARCH | `PLLSTAT` lock: the clock-change procedure | |
| V12 | partial | ARCH | Lost memory-controller writes: how the driver changes clocks (read back and rewrite) | |
| V13 | partial | ARCH | The cores at divide 1: the clock ADR | |
| V14 | partial | ARCH | CL3 above 100 MHz: the clock ADR | |
| V15 | partial | OPT | Dell's 2D command words (clip, fills, scale, palettes, masks): one operation each, none changes the 2D driver's shape | |
| V16 | partial | OPT | Plane formats left (indexed, inverse alpha, packed YUV): extra formats; packed YUV only matters with video | B6 |
| V17 | partial | OPT | Status bit 10's meaning; the architectural answer (no 2D completion bit, so 2D waits stay polled) is already in | B4 |
| V18 | done | OPT | Declined: Windows Mobile's plumbing | |
| P1 | partial | ARCH | The cores' divide: the clock ADR (gate step 3) | |
| P2 | partial | ARCH | The system clock's steps: the clock ADR | |
| P3 | done | ARCH | Render-to-texture: declined (no hardware), a renderer choice | |
| P4 | partial | ARCH | The TA's speed against CPU-built lists: which 3D path the renderer is built on | |
| P5 | partial | OPT | The second texture unit: effects in one pass instead of two | B3 |
| P6 | partial | OPT | The 2D scaler works; using it for zoom and thumbnails is routing inside the voted compositor | |
| P7 | partial | OPT | Palettes left (2/4 bpp, brush): more formats | B1 |
| P8 | partial | ARCH | 2D and 3D at once or in turns: ADR-0030 G2 | |
| P9 | open | ARCH | The video plane scaled and placed: a sheet or pop-up moved by registers is a compositor choice | |
| P10 | open | ARCH | The cursor plane as a free sprite layer: the pointer's design (ADR-0033's mice) | B2 |
| P11 | open | ARCH | MVED1: video decoding in hardware or on the CPU | B6 |
| P12 | partial | OPT | DMA burst and width settings; the architectural part (DMA into the command window works: T3) is answered | B4 |
| P13 | partial | OPT | Lines and the brush palette: per-operation routing for GDI | |
| P14 | partial | ARCH | The 3D core's limits (texture cache size, objects a frame): atlas layout and the compositor's frame budget | B5 |
| P15 | partial | ARCH | Scan-out with both engines busy: where surfaces may live (the 16-bit screen came from this) | |
| P16 | open | ARCH | Power per engine and clock: the clock and power settings | B7 |
| P17 | partial | ARCH | Refresh rate is a display clock setting (battery against smoothness) | B5 |
| P18 | partial | ARCH | Interrupts against polling per unit: driver architecture (the TA's done bit left) | B4 |
| P19 | partial | META | A read of Dell's code for unused features: makes lines, answers none | B8 |
| P20 | done | ARCH | The 16-bit screen (ADR-0023) | |
| P21 | partial | ARCH | The dither add's alternatives (folded into the background, the 3D core) change how the 16-bit screen is composed | B5 |
| F6 | partial | ARCH | Text: ~6 us an operation fixed, so the lever is fewer operations (words or lines kept rendered): the text renderer's design | B5 |

## The CPU (31 lines)

| Line | Status | Tag | Why | B |
|---|---|---|---|---|
| V1 | partial | OPT | The I-cache and mini cache sizes: feed C5's tuning | |
| V2 | done | ARCH | SRAM's size and speed: ADR-0006's layout | |
| V3 | open | ARCH | WM's seven speeds set and measured: the operating points (C1) | |
| V4 | open | ARCH | The voltage patterns ranked: the power settings | B7 |
| V5 | open | ARCH | 624 MHz the top: the overclock decision (C2) | B9 |
| V6 | partial | OPT | The refresh period: matters only for timing changes, which stay Dell's (the owner's rule) | B9 |
| V7 | partial | ARCH | The hop with 4 KiB pages: the kernel's page-size policy | B5 |
| V8 | partial | OPT | DMA priorities under contention: a per-channel setting | |
| V9 | done | META | The sweep that made V10, V11 and C12 | |
| V10 | partial | OPT | Partition 3's emptiness; the CL rule declined by the owner's rule | B9 |
| V11 | open | ARCH | The power manager's bits: the sleep states' design (C10) | B6 |
| C1 | partial | ARCH | Operating points and their switch costs: the clock and power policy | |
| C2 | open | ARCH | The overclock study: a clock setting | B9 |
| C3 | partial | ARCH | What lives in SRAM: answered by the owner's shared layout (2026-10-22) | B10 |
| C4 | partial | ARCH | Page sizes, table placement, TLB lock-down: memory layout | |
| C5 | partial | OPT | The mini cache and line locking: tuning hot loops | B5 |
| C6 | partial | OPT | iWMMXt per hot path: the wiring is decided (the SIMD crate) | |
| C7 | partial | OPT | Copies: faster routines, same callers | |
| C8 | partial | OPT | The event counters in the benches: tooling | B8 |
| C9 | partial | OPT | LTO and inlining left: build settings (frame pointers decided) | |
| C10 | partial | ARCH | Wake latency per sleep depth: which depth the idle policy uses | B6 |
| C11 | partial | OPT | DMA as a copy engine, the polling pattern in other drivers | |
| C12 | open | OPT | Stopping SDCLK0/3 and unused clocks: one register, no design depends on it | B5 |
| T1 | open | ARCH | A pipeline, not turns: the compositor's frame loop | |
| T2 | partial | ARCH | Upload staging in SRAM: memory layout (the owner's shared layout decides it) | B10 |
| T3 | partial | ARCH | Commands by DMA: how the 2D driver submits work (measured: works; the driver choice open) | |
| T4 | open | ARCH | Each unit's job: the compositor's routing table | |
| T5 | partial | OPT | Converters in the GPU's formats: speed of the same path | |
| T6 | done | ARCH | Never reading the 2700G: a renderer rule | |
| T7 | open | ARCH | Clocks per moment: the power policy | B7 |
| T8 | partial | ARCH | = P8 | B11 |

## The count, as proposed

| | 2700G | CPU | All |
|---|---|---|---|
| ARCH | 26 (4 done) | 18 (2 done) | **44 (6 done)** |
| OPT | 12 (1 done) | 12 | **24 (1 done)** |
| META | 2 (1 done) | 1 (1 done) | **3 (2 done)** |
| Boundary flags | 19 | 15 | 34 |

So the architectural gate would hold 44 lines, 6 already done and 38 to
go (37 with T8 counted once with P8; 35 if the owner also ticks B10's
two). That is most of the gate: the lines were mostly written about
design questions, which is why the restructure's dependency rule (point 4)
matters more than the split itself.

## Boundary cases (the owner's ruling wanted)

- **B1 (V3, P7) Formats nobody needs yet.** 2 and 4 bpp palette sources
  could become the icon or font format (half the memory of 8 bpp), which
  would make them ARCH. Proposed: OPT, re-tagged only if an asset format ADR
  wants them.
- **B2 (V6, V7, P10) The cursor plane and colour keys.** A hardware pointer
  is a compositor choice for ADR-0033's mice (ARCH); keys are a trick
  (OPT). Proposed as split above; the whole of P10 could be OPT if the
  pointer is simply composed like any layer.
- **B3 (V9, P5) The second texture unit.** Alone it is an effect in one
  pass (OPT); but it needs the TA path, which hangs (C53), so its answer is
  bound to P4's (ARCH). Proposed OPT, depending on P4.
- **B4 (V17, P12, P18) Lines whose architectural part is answered.** Each
  has a design answer in (2D waits polled, DMA command feed works) and
  leftovers that are tuning. Proposed: P18 stays ARCH for the TA's done bit
  (only if P4 picks the TA); the others OPT with their answered part noted.
- **B5 (V7, P14, P17, P21, F6, C5, C12) Settings or limits that shape a
  design only somewhat.** Each could go either way: page size policy (V7),
  the texture cache's size for atlases (P14), refresh rate (P17), the
  dither add's alternatives (P21), text caching (F6) are tagged ARCH; the
  mini cache (C5) and idle clocks (C12) OPT. The test applied: would the
  answer appear in an ADR's decision section.
- **B6 (V8, V11, P11, C10) Architectural, but for later phases.** MVED1
  decides the video path; the power manager's bits and wake latency decide
  the sleep states. Nothing being built now depends on them. Under point 4
  they block nothing today, but under point 2 the gate can't lift until
  they're ticked, so **the gate would wait on video bring-up and power
  states**. Proposed alternative: an ARCH line names the phase whose design
  needs it, and the gate's lift needs only the lines the next phase needs;
  the rest gate their own phase. The owner's call.
- **B7 (V4, P16, T7) Lines that may not be measurable.** P16 needs a
  battery reading, and none is found (the TSC2046's VBAT isn't wired; WM's
  gauge goes through Dell's HAL codes); V4's voltages can't be read without
  opening the device (the owner's rule: never). If they stay ARCH, the gate
  needs a rule for "unmeasurable": declined with the reason, or decided by
  a default the owner sets.
- **B8 (P19, C8) Discovery and tooling.** P19 (read Dell's code for unused
  features) is where new lines come from: under point 2, anything it finds
  that is architectural needs the owner's yes before joining. Proposed:
  P19 META, closed when the read is done, its finds proposed to the owner.
  C8 (event counters) is tooling for the ratchet and the backlog, not a
  speed-up in itself: OPT, near the top of the backlog because it makes the
  others cheaper to measure.
- **B9 (V5, V6, V10, C2) Clock and memory settings the owner's rules
  already fix.** RAM timing stays Dell's and tuning is for every X51v, not
  one unit (2026-10-19). V6's refresh period and V10's CL rule only matter
  for timing changes the rule forbids: OPT, likely declined. The overclock
  (C2, V5) is a clock setting (ARCH) but must hold on every unit, with no
  voltage readable: proposed ARCH, with the owner choosing whether to study
  it at all (declined is a valid tick).
- **B10 (C3, T2) Answered by today's ruling.** The owner's shared SRAM
  layout (ADR-0006 amendment) is the architectural answer for both; what's
  left is building it. Proposed: tick both ARCH parts, leftovers to the
  backlog. Needs the owner's yes (a ledger row changes only with evidence:
  here the evidence is run E1e plus the ruling).
- **B11 (T8) Counted twice.** T8 is P8 by definition. Proposed: count once.

## Conflicts with standing rules (for the owner)

- **The gate's hard rule** (memory, 2026-10-21: no moving forward until
  every lever is ticked) is replaced by this restructure once ruled; the
  memory and ROADMAP get the new rule then, not before.
- **The lift's second half.** Gate step 8 (the owner's vote, 2026-10-21):
  every line ticked *and the ratchet running*. Point 5 keeps the ratchet as
  the regression guarantee; proposed: the lift keeps "the ratchet running"
  too. Today it runs for the benchmarks but not touch, frames or boot.
- **"Verify yourself"** (hard rule): the V lines check Dell and Intel
  claims. Tagged OPT, an unverified claim would wait in the backlog; but
  wherever such a claim is used in code it stays an open item in the
  quality audit (the work-the-audit rule), so nothing unverified gets
  built on silently. Proposed: no change to that rule.
- **The compositor harness** (T1, T4, T7: the owner's vote, built inside
  the gate) stays inside the gate: all three are ARCH.

## What happens after the ruling (not done yet)

The ledgers get a tag column that `cargo xtask gate` counts (ARCH lines
gating, OPT lines a backlog with payoff and device cost, ordered), the
ROADMAP's gate section is rewritten with the new lift condition and the
dependency rule, and the memory's gate rule is replaced. Payoff and device
cost for the 22 OPT lines are estimated then, not now.
