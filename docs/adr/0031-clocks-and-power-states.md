# ADR-0031: RetroPane owns the CPU clocks, voltage and power states

- **Status:** Accepted (2026-10-15: the owner approved V1-V4 as recommended)
- **Date:** 2026-10-15
- **Audit item:** 67 (docs/research/imitation-audit.md, "CPU clock, voltage,
  power states": the highest-value item)

## Context

Until now RetroPane never programmed `CCCR`, `CLKCFG` or `PWRMODE`. It ran
at whatever Windows Mobile left before HaRET started the loader: 520 MHz
turbo on one boot (`CCSR 0x30000290`, `CLKCFG 0xB`, probe 2026-10-11), 624
MHz on another (perf.md, 2026-10-11). Its only power state was
`PWRMODE = 1` (idle) in `rt::idle`. Every benchmark depends on the clock,
and so does the battery. The finished OS boots from flash with no Windows
Mobile underneath, so nobody else will set anything up. This ADR is
written for that end state, and the SD-card stage runs the same code
(memory: "bare metal first").

What the chip offers (PXA27x Developer's Manual, section 3):

- **Core PLL:** run = 13 MHz x L, turbo = run x 2N / 2 (2N 2-6, at most 624
  MHz). The system bus is run (`CLKCFG` B, fast bus, up to 208 MHz) or run
  / 2. The memory clock is the system bus (`CCCR` A) or run / 1, 2 or 4 by
  L. The SDRAM clock is the memory clock, or half of it (`MDREFR` KxDB2;
  104 MHz at most).
- **Turbo on/off** (`CLKCFG` T, or HT for half) switches the core between
  run and turbo **without relocking the PLL**. Any multiplier change needs
  F (a relock). During the relock the SDRAM self-refreshes, so the change
  must run from code already in the I-cache.
- **Power modes** (`PWRMODE`, CP14 c7): 1 idle (core clock stopped, wake on
  any interrupt), 2 standby (SDRAM self-refresh, PLLs off, wake on
  GPIO/RTC/OS-timer-4..11 at 32 kHz), 3 sleep (core off, SDRAM
  self-refresh, wake through the power manager), 7 deep sleep (32 kHz
  oscillator only). Deep idle = idle in 13 MHz mode (`CCCR` CPDIS/PPDIS:
  the peripheral PLL stops too).
- **CP14 performance monitor:** the cycle counter `CCNT` (CP14 c1, c1) and
  four event counters. A read costs a cycle or two, against 846 ns for an
  OS-timer read (perf.md).

What the board does (`docs/hw/clocks.md`, a static read
of WM's OAL; facts only):

- **The core voltage is set by three GPIOs** (GPIO3, GPIO4, GPIO38), not by
  a PMIC on the power I2C. GPIO3/4 are the power I2C pins, used as plain
  GPIOs. Codes per frequency (`rp_hal_x51v::vcore`): 104 MHz and below
  `000`, 208 MHz `100`, 312 MHz `101`, 416 and 520 MHz `110`, 624 MHz
  `111`. The probe's levels under WM at 520 MHz read `110`, which agrees.
- WM raises the voltage, waits ~2.46 ms, then changes the frequency. Going
  down, it changes the frequency first, then lowers the voltage.
- WM's seven points are 91, 104, 208, 312, 416, 520 and 624 MHz. From 208
  MHz up they all use L 16 with A and B (bus and memory at 208 MHz), and
  624 is L16 x 2N6. WM never uses HT or 13 MHz mode. Its idle is
  `PWRMODE = 1` with no clock change. Its boot point depends on a board-ID
  register (624, 520 or 416).

## Decision

### D1: operating points (`rp_hal_pxa27x::clocks::POINTS`)

Seven points: 104, 156, 208, 312, 416, 520 and 624 MHz. **Every one uses
the fast bus and the alternate memory clock**, so the bus, the SDRAM and the
2700G never run slower than the run frequency allows (208 MHz from the 208
point up). The top point is **L16 x 2N6**: run 208, turbo 624, bus and
memory 208 MHz. The alternative, L24 x 2N4, would drop the bus to 156 MHz.
At L16 x 2N6, turbo off gives 208 MHz with no relock. Every point is
checked against the chip's limits (`Point::legal`) before it is used.

Every change is first worked out as a pure `clocks::plan` (the register
writes in order), and `change` only executes it. A host test runs every
pair of points, both ways, turbo on and off, and checks that the final
refresh interval is always the rule's for the new memory clock, that the
interval is never late during the change, and that any memory-clock change
is a relock. A switch can't skip the `DRI` rewrite (memory-timing report:
a stale interval at a lower memory clock corrupts SDRAM silently).

The SDRAM refresh interval is **derived** from the 64 ms rule and the row
count in `MDCNFG` (49 at 208 MHz, against WM's 48), not copied. Around a
change:

- the shorter of the old and new intervals goes in before the change;
- the SDRAM divider is set before a speed-up and cleared after a
  slow-down. When clearing it, the longest interval goes in first and the
  final value follows 3 µs later. That is WM's stricter order; Linux writes
  the final value at once. Until we understand the difference, this is a
  finding to work.

### D2: policy: race to idle, with turbo on demand (owner vote V1)

The scheduler (ADR-0010, already tickless) knows two things: whether any
thread is runnable, and when the next timer is due. The policy uses those,
not load sampling.

1. **Work queued → 624 MHz.** The PLL stays at L16 x 2N6 and voltage code
   `111` while the device is in use (screen on). Runnable threads switch
   turbo on (one `CLKCFG` write, no relock). This is "turbo at 624 MHz when
   work is queued".
2. **Nothing runnable → idle** (`PWRMODE = 1`) at once. The core clock
   stops. Turbo goes off first, so an interrupt's first instructions run at
   208 MHz and turbo comes back only if a thread wakes. Whether that costs
   or saves anything is measured in step (b).
3. **A long idle ahead** (screen off, next deadline beyond a break-even
   time measured in step (c)) → relock to the **104 MHz** point and lower
   the voltage to `000`, then idle or standby (D3). On wake with work, the
   voltage goes up (settle), then the PLL relocks to 624 MHz.
4. **Background-only work** (screen off: music, sync) → the 208 MHz point
   at code `100`. Energy per cycle scales with V², so a lower voltage saves
   energy where a lower clock alone doesn't. This applies only when a
   deadline-driven task says it doesn't need speed (an audio decoder knows
   its period).

Why race to idle rather than load-sampling scaling (Linux "ondemand",
WM's automatic mode): at a fixed voltage, a slower clock does the same
work with the same energy for the switching and more for the leakage (it
stays awake longer), and it adds latency. The only real saving is voltage,
and that needs a relock and a ~2.5 ms settle on the way up, too slow to
chase load. So the voltage drops only for known-long idles and for declared
background work, and turbo, which is free to switch, covers the
interactive case.

### D3: idle depths and what wakes them

| Depth | Entered when | Wakes on | Exit cost |
|---|---|---|---|
| Idle (`PWRMODE` 1), current point | nothing runnable | any unmasked interrupt (tickless alarm on OS timer match 1, devices) | ~µs (measure, step c) |
| Low idle: 104 MHz, code `000`, then idle | screen off and next deadline > break-even | same | relock + ~2.5 ms voltage settle before 624 |
| Standby (`PWRMODE` 2) | screen off, no audio or DMA streaming, next deadline > standby break-even | GPIO edges (power button GPIO0, keys, card detect), RTC alarm, OS timer 4-11 on the 32 kHz clock | ms: PLLs relock (measure) |
| Sleep (`PWRMODE` 3) | user "off" (power button) | power button, RTC alarm (calendar), charger/cradle edges; WM's `PWER 0x80007803` (GPIO0, 1, 11-14, RTC) is the candidate list, a finding to work | resume through the boot path (RAM kept, self-refresh) |
| Deep sleep (`PWRMODE` 7) | battery critical | power button only | cold boot |

Deep idle (13 MHz mode) is **not used** for now. It stops the peripheral
PLL, which stops SD, USB, UART and I2S. Standby saves more and wakes on
the same events, so deep idle has no niche unless step (c) shows standby's
wake cost is too high for audio-off screen-off waits.

### D4: timestamps: CP14 cycle counter where it fits, OS timer as the time base

- **Wall time and deadlines stay on the OS timer** (OSCR0, 3.25 MHz, from
  the 13 MHz crystal). It doesn't change with the core clock and doesn't
  stop in idle.
- **The cycle counter for short intervals and profiling.** The trace ring
  (item 54), benchmarks and lock-hold and IRQ-latency measurements use
  `CCNT`. It costs ~1 cycle to read, against 846 ns. It counts core cycles,
  so it changes rate with turbo and stops in idle. The kernel therefore
  records each point change and idle entry in the trace (an "epoch"), and
  converts cycles to time only within an epoch, at that epoch's
  `core_khz`. `clocks::measure_core_khz` checks that relation on the device.
- Wakes through standby and sleep use OS timer channels 4-11 clocked at 32
  kHz, or the RTC. The 3.25 MHz timer stops there.

### D5: drivers across clock changes

| Driver | Clock it runs on | Effect of a change | What it does |
|---|---|---|---|
| SD/MMC, USB, UART, I2S, I2C | peripheral PLL (312 / 48 MHz) | none: only the core PLL relocks | nothing |
| DMA (SD, I2S ring) | system bus | stalls while SDRAM self-refreshes during a relock (µs: measure) | nothing if the relock is shorter than the I2S FIFO's drain time; measured in step (a) |
| 2700G (display, 2D, 3D) | its own PLLs; the bus to it is static memory CS3 (`MSC1`, counted in memory clocks) | at points below 208 MHz the bus is slower, never faster (no point raises the memory clock above 208 MHz) | nothing for safety; the flush speed per point is measured (`vram_write_256k`); timings could be retuned per point later (items 68/75: the memory and CF timing sessions) |
| CF (PC Card timings `MCxx`) | memory clock | slower below 208 MHz, never faster | as the 2700G |
| PXA LCD controller | — | off on the X51v (the 2700G drives the panel) | WM's `LCCR3`/`LCCR4` rewrite on each change is dropped |

So no driver needs a "clock change" notifier. The power service makes
changes only from the idle path, with interrupts masked. A relock is
refused while an isochronous stream (audio) has less FIFO margin than the
measured relock time.

### D6: voltage (owner vote V2)

Raise first, wait `RAISE_SETTLE_US` (2.5 ms; WM's ~2.46 ms, the real slew
unmeasured), then the faster clock. Going down: the slower clock first,
then the lower code. **No code below WM's for a frequency**, until a stress
test (step c and later) shows margin. Undervolting is possible (the codes
are coarse and Dell validated them for every unit), but its failure mode is
silent corruption. V2 asks whether to try it.

### D7: the flash-boot end state

The boot code (Phase 2 launcher, later the flash boot) sets the 624 MHz
point and code `111` itself, before anything else runs, from the reset
state's 13 MHz/91 MHz. From then on the power service (D2) owns the clocks.
On the SD-card stage the same code runs over whatever WM left
(`clocks::save` / `restore` let test images put WM's state back on the
way out).

## Device tests

| Step | Image | Question | Status |
|---|---|---|---|
| (a) | `rp-clocks` v1 | Is each point really what the cycle counter counts? Does SDRAM survive every change (3 MiB kept, 256 KiB fresh)? What are CPU, SDRAM, SRAM and 2700G speeds at each point? How long do a turbo switch and a relock take? Do the clocks come back as found? | built, emulator passes (`cargo xtask emu-clocks`); waiting for the device run |
| (b) | `rp-clocks` v2 | Turbo on demand in the threads image: turbo on when runnable, off in idle; latency of the first instruction after wake at 208 vs 624; thread-switch and IRQ latency at a fixed 624 MHz | after (a) |
| (c) | `rp-clocks` v3 | Each idle depth's wake cost; break-even times; current draw by battery percentage over time at each depth (screen off) | after (b); needs a battery readout (the gauge is unknown: `battdrvr.dll` and CPLD `0x1A`/`0x1E` are the leads) |

Each image is guarded by the loader's soft run limit (FIQ, so it fires
with IRQs masked too). Its fault and panic paths put the clocks back
before the crash report. A fault after a voltage raise leaves the higher
code, which is safe.

## Votes

**Decided 2026-10-15 by the owner, as recommended:**

- V1: race to idle ("sprint, then rest");
- V2: no undervolting; Dell's voltage codes stay for now;
- V3: 624 MHz while the screen is on;
- V4: WM's 624 MHz set-up (A = 1) if `rp-clocks` shows it clean and
  faster, with the manual's (A = 0) as the automatic fallback.

The questions as they were put:

- **V1 (policy):** race to idle with turbo on demand and voltage drops only
  for long idles and declared background work (**recommended**), *or*
  load-sampling frequency scaling (like Linux ondemand / WM's automatic
  mode). Pros of the recommendation: fastest response, simplest, no
  sampling timer (keeps tickless), and energy as good or better at fixed
  voltage. Con: it needs the scheduler to know "background" (a task flag).
  Pros of sampling: no task flags. Cons: latency spikes, a periodic timer,
  and wasted energy in the ramp-up.
- **V2 (undervolting):** keep WM's voltage codes (**recommended for now**),
  *or* stress-test one code lower per point. Pro: battery. Con: silent
  corruption risk, and weeks of stress testing per point.
- **V3 (624 MHz while the screen is on):** **recommended**: the UI never
  waits for a clock ramp. The alternative is 520 MHz at code `110`: 17%
  slower, for about 25% less energy per unit of work (V² by Dell's voltage
  labels, 1.364 against 1.574 V; the real rail voltages are unmeasured).

- **V4 (which 624 MHz set-up):** the manual's table 3-7 lists 624 MHz for
  the C5 stepping (this unit: `CP15 ID 0x69054117`) only with `CCCR` A =
  0, which puts the memory clock at 104 MHz. SDRAM stays at 104 MHz either
  way, but the static buses run at half speed, the 2700G's among them.
  Windows Mobile runs A = 1 (memory clock 208 MHz) on this unit every day,
  and Linux's table carries it, but the manual doesn't list it
  (docs/research/memory-timing.md, open question 4). `rp-clocks` measures
  both (C2 `624` and `624m`). **Recommended:** A = 1 if the device run
  shows it clean and measurably faster (2700G writes, chases), with the
  manual's A = 0 as the fallback the boot code can drop to. Pro: the faster
  bus. Con: it is outside Intel's published table, though it has years of
  use behind it.

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | Every point measured on the device by the cycle counter, with SDRAM integrity checks (`rp-clocks`). The policy's thresholds (break-even times) come from measurements in step (c), not guesses. `clocks` is host-tested (decode, legality, refresh rule, `MDREFR` ordering) and `vcore` too. |
| ✨ Modernize | Race to idle driven by the tickless scheduler's knowledge (runnable, next deadline) instead of 2005-era load sampling. A cycle-counter timestamp for the trace ring. Refresh derived from the SDRAM rule. Voltage tied to declared intent (background tasks), not guessed from load. |
| 🗄️ Legacy support | WM programs that call `SetProcessorPowerMode` / KernelIoControl `0x0101010D` (AximSDK) can be served by the CE personality as hints (fixed modes → minimum point) without giving them the clocks. Nothing a legacy program can do changes the policy's safety rules. |
| ⚡ Better / faster | 624 MHz always available with the bus at 208 MHz (WM sometimes ran at 520). Turbo switches without a relock. Thread-switch and benchmark numbers become comparable across boots. Idle drops voltage and the PLL, which WM never did (its idle kept the clock). |

## Hardware limit or choice?

- 624 MHz maximum, 208 MHz bus, 104 MHz SDRAM: **chip limits** (manual).
- The voltage pins and codes: **board wiring** (GPIOs) is a limit. Which
  code per frequency is **Dell's choice**, tested on their units. We keep it
  for now (V2).
- 624 as L16 x 2N6 with the bus at 208: both WM and Linux chose it. It is
  also the point the arithmetic says is best (D1), so it is a measured
  choice, not imitation.
- WM's idle without a clock change and its load-sampling automatic mode:
  **software choices**, replaced (D2).
- The 2.46 ms voltage settle, `PSLR 0xCC400004`, `PCFR 0x5031` and WM's
  wake masks: **findings to work** (imitation-audit rule), counted below.

Copied values still open (count: 6): voltage codes per frequency, the 2.5
ms settle, the DB2-clearing order, `PSLR`, `PCFR`, the sleep wake masks.

## Alternatives considered

1. **Leave WM's clock** (today): it varies by boot (520 or 624) and doesn't
   exist on the flash boot. Rejected.
2. **L24 x 2N4 for 624 MHz:** run 312, but the bus drops to 156 MHz (fast
   bus is illegal above 208), slowing SDRAM and every 2700G write by a
   quarter. Rejected by arithmetic. Step (a) can measure the difference if
   wanted.
3. **Load-sampling frequency and voltage scaling** (ondemand-style): see V1.
4. **13 MHz deep idle as the long-idle state:** stops SD, USB and audio. It
   is beaten by standby for the screen-off case (D3). Kept as an option if
   standby's wake cost proves too high.

## Consequences

- Benchmarks get a known clock. perf.md's rows are re-measured at 624 MHz
  in step (a).
- The kernel's idle path gains the power service's decision (D2/D3). The
  scheduler gains a "background" task flag (V1).
- Battery measurement needs a gauge driver: a new RE lead (`battdrvr.dll`).
- The QEMU fork models the clock arithmetic and the cycle counter, or lists
  them (docs/hw/emulator.md, "PXA270 clocks").

**`rp-clocks` version 3, run 1 (2026-10-18 ~00:20, unattended with the
owner's permission; `local/rp-clocks-run3.txt`):** sent and started; it
never returned to the loader. The hardware watchdog reset the Axim to
Dell's restore-factory-settings screen (the owner pressed Mail and started
the loader). No run log survived (the loader's in-memory log is lost in a
reset; `CRASHLOG.TXT` holds only its 45-byte header), and Windows
Mobile's boot overwrote the breadcrumbs in 2700G memory. So which step
stopped it is unknown. **Measurement only; the next step is the owner's
decision.** Options: run it again with the owner watching the stripes (one
per step, painted before each change) and counting them; or keep the
breadcrumbs somewhere a reset can't clear them (the PXA's internal SRAM or
the card) before running again.

