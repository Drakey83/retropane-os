# X51v audio (WM8750 codec, PXA27x I2S and I2C)

> Spec v0.1, 2026-10-12. Facts only, per [`../CLEANROOM.md`](../CLEANROOM.md).
> **Sources:** (M) Intel PXA27x Developer's Manual, I2S and I2C chapters;
> (W) Wolfson WM8750 datasheet (public); (C) AxDroid board code, via
> `docs/research/hardware-inventory.md`.

## Board (C)

| Item | Value | Confidence |
|---|---|---|
| Codec | Wolfson WM8750 (stereo DAC/ADC, headphone and speaker drivers) | confirmed (code) |
| Control | PXA27x standard I2C, GPIO117 (SCL) / GPIO118 (SDA) | confirmed (code) |
| Codec address | 0x1A or 0x1B (the WM8750's CSB pin; not recorded): the driver tries both | unknown |
| Audio data | PXA27x I2S, GPIO28-31 and GPIO113 (system clock) | confirmed (code) |
| Speaker amplifier | **CPLD `0x14` bit 1** (on only for the speaker, 70 ms to settle); GPIO82 is the codec's I2C gate, not the amplifier | **confirmed** (device: Windows Mobile measured 2026-10-18, `rp-tone` run 9 played on the speaker) |
| Headphone detect | **GPIO83, high = plugged in, valid only while the codec is powered and set up** (likely the codec's own jack-detect output) | **confirmed** (device, 2026-10-18: under Windows Mobile and `rp-tone` run 9) |

## PXA27x side (M)

- I2C (standard unit): registers from `0x40301680` (`IBMR`; `IDBR` +0x08,
  `ICR` +0x10, `ISR` +0x18, `ISAR` +0x20), clock `CKEN` bit 14. A write is
  start + address (`ICR` START | TB), data bytes (TB, STOP on the last), each
  ending with `ISR` ITE; `ISR` ACKNAK set means refused; `ICR` MA sends a
  stop after a refusal.
- I2S: registers from `0x40400000` (`SACR0`, `SACR1` +0x04, `SASR0` +0x0C,
  `SADIV` +0x60, `SADR` +0x80), clock `CKEN` bit 8. The PXA is the clock
  master: `SACR0` ENB | BCKD; `SADIV` 0x0C gives 48 kHz (system clock
  256 fs = 12.288 MHz), 0x0D 44.1 kHz; `SACR1` DREC turns recording off.
  `SADR` takes one stereo frame per write (left in the low half);
  `SASR0` TNF says the 16-entry FIFO has room. DMA request lines: receive
  2, transmit 3.

## Codec side (W)

Write-only registers: two bytes per write, register number (7 bits) and
value (9 bits). Playback set-up used by `rp-drv-wm8750`: reset (R15),
power (R25 VMID/VREF; R26 DACs and both output pairs), I2S 16-bit slave
(R7 = 0x002), 48 kHz from 12.288 MHz (R8 = 0), DAC unmuted (R5), DACs to
the output mixers (R34, R37 = 0x150), output volume (R2/R3 headphones,
R40/R41 speaker: 0x79 = 0 dB, 1 dB steps, below 0x30 muted; written at zero
crossings, the right channel latching both).

## Software (emulator-verified, device run pending)

- `drivers/pxa-i2c`, `drivers/wm8750`, `drivers/pxa-i2s` (with `Tone`, a
  fixed-point sine from a phase accumulator and a 65-entry quarter table).
- `rp-tone` (`cargo xtask tone`; until 2026-10-16 the loader's `tone`
  command, moved out in the loader diet: 3.5 KiB of the loader): finds the
  codec, streams 440 Hz for 500 ms at 48 kHz by DMA, logs how long it took
  (the codec's clock paces it) and the CPU's busy share, then waits 15 s for
  the headphone jack to change and plays 1000 Hz for 50 ms to the new output.
- Streaming (`rp_drv_pxa_i2s::stream`): DMA channel 1 loops an 8 KiB ring
  (two 4 KiB halves, 1,024 frames each, ~21 ms) into `SADR` (request line
  3, word-wide, 32-byte bursts); the CPU refills the half the engine has
  left, writes it back from the cache, and is free in between. Stops after
  the last frame has played; a ring that stops moving for 200 ms is an
  error, not a hang.
- Headphone jack: GPIO83 read each second, debounced over two reads; high
  is taken as plugged in (unconfirmed: `rp-tone` logs the level at start and
  each change, so a device run settles it). Plugged in, the speaker pair is
  muted and sound goes to the headphones; unplugged, back to the speaker.
  The log names the output.
- Not yet: the speaker amplifier, recording, sample rates other than
  48 kHz on the codec side.

## Device run (with the owner there: it makes sound)

`cargo xtask tone` with headphones in (then pull them out while it waits):
the log names the codec's address (recording it here), the time matches
500 ms if the clocks are right, and the jack's level for in and out. Then,
for the speaker, the amplifier lines (GPIO82, CPLD 0x14 bit 3) one at a
time.

**First device run (P, 2026-10-16, `rp-tone`, `local/rp-tone-run1.txt`):
no sound.** With headphones plugged in: GPIO83 read **0** ("out"), so
either the jack reads low when plugged (the assumed polarity is wrong) or
GPIO83 isn't the jack; and **no codec answered on I2C** at 0x1a or 0x1b.
The emulator's run passes, so this is something about the board not yet
modelled. Candidates, to settle in this order: the I2C pins' alternate
functions (GPIO117/118 as Windows Mobile leaves them, read before ours),
the I2C unit's clock (CKEN), the codec's power or reset (a CPLD or GPIO
line Windows Mobile switches on with sound), and the codec's identity and
address on the X51v. Next step: a read-only probe image that reports the
pins' functions, CKEN and an I2C address scan, run once with sound playing
in Windows Mobile just before (HaRET) and once without.

## Dell's audio driver, static read (2026-10-17, facts only)

Source (D): `wavedev.dll` from the A08 ROM (`local/imgfs-a08`), disassembled
with llvm-objdump (`local/re/wavedev/wavedev.asm`). No code copied.

| Fact | Confidence | Evidence |
|---|---|---|
| The codec is on the PXA27x **standard I2C** unit (`IDBR` 0x40301688, `ICR` 0x40301690, `ISR` 0x40301698): AxDroid's bus is right | confirmed (code) | (D): the only I2C registers it touches |
| Its address is **0x1A**; register writes are two bytes (register << 1 \| value bit 8, then value bits 7-0), registers 0-42 (the WM8750's) | confirmed (code) | (D): the register-write routine passes 26 to the I2C write, refuses registers ≥ 43 |
| **GPIO82 is driven high for every codec I2C transfer and low after it**, inside a lock shared with another driver (`CI2C_Mutex_H`): a gate or switch on the codec's I2C lines (likely), **not** the speaker amplifier AxDroid named. Without it the codec can't hear the bus: the first device run's "no codec at 0x1a or 0x1b" | confirmed (code); its circuit role likely | (D): the transfer routine sets GPIO82 to 1 before START and to 0 after STOP, on success and failure alike |
| The I2C unit's clock (`CKEN` bit 14) is switched on before each transfer and off after it (on request) | confirmed (code) | (D) |
| Fast mode: `ICR` = 0x8060 (fast mode, unit enable, SCL enable); `ISR` cleared with 0x6F0 | confirmed (code) | (D) |
| ~~Headphone detect is GPIO96, low = plugged in~~ **Superseded (measured 2026-10-18):** GPIO96 reads 1 whenever audio is powered, plugged in or not; the jack is GPIO83 (table above). The static reading took the flag for the jack | superseded | `rp-wmaudio`, `rp-tone` runs 3-9 |
| The driver keeps DMA buffers or descriptors in the PXA27x's **internal SRAM** (0x5C038000-0x5C03E90C) and feeds `SADR` (0x40400080) | confirmed (addresses) | (D) |

**Device run 2 and 3 (2026-10-18 night, `rp-tone` quiet at level 20,
`local/rp-tone-run2/3-console.txt`): still no codec** with GPIO82 open, and
with GPIO117/118 confirmed in I2C mode (found: GPIO117 fn 1 out, GPIO118 fn
1 out, GPIO82 fn 0 out; GPIO96 read 0). So the gate alone is not enough;
the static read below found why.

**Power, from the same driver (static, 2026-10-18):**

| Fact | Confidence | Evidence |
|---|---|---|
| The driver changes CPLD bits through Windows Mobile's shared shadow (cpld.md): a set helper and a clear helper indexed by slot (3 = `0x14`, 7 = `0x1C`) | confirmed (code) | (D): the 10-way switches over `0xAC001320`-`0xAC001344`, compare-and-swap, then the register write |
| **CPLD `0x1C` bit 1 is the codec's power** (or its I2C side's): set on the driver's power-up path; cleared together with turning GPIO117/118 into plain outputs held low and a 15 ms wait, on power-down | likely (role inferred from the sequence) | (D): set at `0x02464a5c`, cleared at `0x024653cc` |
| **CPLD `0x14` bit 1 is the speaker amplifier**: set once when output starts, then a **70 ms** wait (an amplifier's settle), flagged so it isn't set twice | likely | (D): `0x02465678` |
| At hand-over both are **off** (`14=0008`, `1C=000C`: bit 1 clear in both): an unpowered codec can't answer, which fits every device run | confirmed (values) | cpld.md, `rp-handover` |
| On audio power-down Windows Mobile turns GPIO117/118 into plain outputs held low (function 0) and on power-up back to function 1 (I2C); on the owner's X51v they were already function 1 at hand-over | confirmed (code + device) | (D) `0x02465380`; run 3 |

**Next device step (owner watching, one board bit per run):** set CPLD
`0x1C` bit 1 through the shared shadow (`Cpld::adopt`), wait, look for the
codec; put the bit back as found at the end. Only if the codec then answers:
a second run that also sets `0x14` bit 1 (70 ms settle) for the speaker.

**Run (2026-10-18 ~10:00, `RP_TONE_LEVEL=20`, owner present at the
start):** CPLD found `14=0008 1C=000C`; codec power on (`0x1C` bit 1), 50 ms
settle; GPIO117/118 function 1 outputs, GPIO82 function 0 output; GPIO96
read 1 (headphones out); **no codec answered on I2C at `0x1a` or `0x1b`**
with the gate (GPIO82) driven; power put back off; clean return to the
loader, no reset. A measurement, not a decision. Options for the owner:
(a) a longer settle (500 ms) and the amplifier bit `0x14` bit 1 as well,
one more board bit, owner watching; (b) a read-only `rp-probe` from HaRET
**while Windows Mobile is playing a sound**, recording the CPLD, GPIO
functions/levels and I2C/I2S registers as Dell leaves them when audio
works, then match exactly (no board bit switched; best evidence); (c) a
static read of what wavedev.dll does between power-on and its first I2C
write (clocks: `CKEN` for I2C/I2S, a SYSCLK/MCLK the codec may need, the
power I2C unit instead of the standard one).

**Power-up order (static, 2026-10-18, option (c) above, from `wavedev.dll`
`0x02464A54`-`0x02464AF0`).** Windows Mobile never talks to the codec
before its master clock runs:

1. CPLD `0x1C` bit 1 on (slot 7, the shared shadow); 1 ms.
2. The I2S pins: GPIO28 (BITCLK) function 1 out, GPIO29 (SDATA_IN)
   function 2 in, GPIO30 (SDATA_OUT) and GPIO31 (SYNC) function 1 out,
   **GPIO113 (I2S SYSCLK, the codec's MCLK) function 1 out** (`0x02464654`).
3. `CKEN` bit 8 (I2S clock) on, through a helper keeping a shadow
   (`0x02467838`, `CKEN` at `0x41300004`).
4. `SACR1 |= 0x18` (record and replay held off), `SADIV = 0x0D`, then
   `SACR0 = 0x8805` (enable, bit clock driven, FIFO thresholds), polled
   until enable reads back (`0x024643F0`).
5. 1 ms; then the first I2C writes: **register 15 (reset)**, register 25
   (power management 1) = `0x1C2`; then 75 ms.
6. Then it reads GPIO83 (`GPLR2` bit 19) and GPIO96 (`GPLR3` bit 0, the
   jack).

**rp-tone run 1 did it the other way round:** it looked for the codec
(step 5) before the I2S pins were set or the controller ran (it starts
I2S only to play), so the codec had power but no MCLK on GPIO113, and its
I2S pins were whatever Windows Mobile left after powering audio down.
Likely the reason nothing answered (whether the WM8750's control
interface needs MCLK is not settled from here; Dell's order is the
evidence). **rp-tone now follows Dell's order** (pins, I2S running at 48
kHz, 1 ms, then the probe; `emu-tone` passes). The next owner-watched run
switches the same one board bit as run 1 (`0x1C` bit 1, already shown to
return cleanly); the pin functions are Dell's own and can't hold the bus.

**Run 2 (2026-10-18 ~17:00, `0ea7387`, owner watching, headphones in at
low volume):** CPLD found `14=0008 1C=000D` (bit 0 of `0x1C` now set,
`000C` in run 1: Windows Mobile booted in between; meaning unknown);
codec power on; I2S pins and the controller running first (SYSCLK on
GPIO113), then 1 ms: **still no codec on I2C at `0x1a` or `0x1b`**; clean
return. **GPIO96 read 1 ("out") with the owner's headphones plugged in**:
either the jack reads the other way round, or GPIO96 isn't the jack
(Dell's driver reads `GPLR3` bit 0 and `GPLR2` bit 19 = GPIO83 after
power-up). A measurement. Ruled out now: the power-up order and the
codec's master clock. Options for the owner: (b) above, a read-only probe
under Windows Mobile while it plays a sound (recommended: no board bit,
answers what Dell's state is when audio works); (c) further static reading
of the I2C path Dell's codec writes take (`0x02467640`: which I2C unit,
which address, which gate).

**Windows Mobile's working state, measured (`rp-wmaudio`, 2026-10-18 ~17:20,
two runs alike; log `local/wmaudio/rp-wm-audio-run1.txt`):** read-only,
while Windows Mobile played its own quiet tone (`waveOutOpen` returned 0
both times; the owner heard only a slight click, but the tone was far too
quiet to judge by ear: about -30 dB at 12.5% volume, a test-design
mistake, so this is no evidence that audio is broken under Windows
Mobile).

| Fact | Confidence | Evidence |
|---|---|---|
| **The codec is on the standard I2C unit at address `0x1A`, and it acknowledges Windows Mobile**: the watcher caught `IDBR` `0x34` (address `0x1A`, write) then `0x32`, `ISR` `0x44` (unit busy, transmit empty, ACK) with GPIO82 (the gate) high during the transfer and low after | confirmed (device) | `rp-wmaudio` bus watcher |
| **Windows Mobile drives I2C in fast mode**: `ICR` `0x8061`/`0x8060` (bit 15 FM, 400 kHz; unit and SCL enabled); RetroPane's driver uses standard mode. The I2C clock (`CKEN` bit 14) is on only around transfers | confirmed (device) | same |
| Windows Mobile talked to the codec ~260 ms after opening the device (open at ~76 ms, the transfer at ~338 ms, just before the tone started); `rp-tone` waits 50 ms after power | confirmed (device; earlier transfers may have been missed: 1 kHz sampling) | same |
| While audio plays: CPLD `0x1C` `0x0D` -> `0x0F` (**bit 1, codec power: confirmed**), and for the speaker only CPLD `0x14` `0x08` -> `0x0A` (**bit 1, speaker amplifier: confirmed**); `CKEN` bit 8 on; I2S pins GPIO28-31 and 113 to their functions (`GAFR0_U` `0x591A851A`, `GAFR3_U` bit 1), `SACR0` `0x8805`, `SACR1` `0x08`, `SADIV` `0x0D` | confirmed (device) | snapshots "playing" |
| **The headphone jack is GPIO83, high = plugged in, valid only while audio is powered** (idle it reads 0 either way). **GPIO96 is not the jack**: it reads 1 whenever audio is powered, plugged or not (so `rp-tone`'s "headphones out" was GPIO96, wrongly read as the jack) | confirmed (device, both states) | snapshots with headphones in and out |
| Idle (audio off): `CKEN` `0x00D91AA3` (I2S, I2C, power I2C clocks off), CPLD `0x1C` `0x0D`, `0x14` `0x08` | confirmed | snapshot "start" |

**The owner confirms Windows Mobile's sound works on this X51v**
(2026-10-18): the codec and its wiring are good. **`rp-tone` run 3**
(standard and fast mode, ~50 and 300+ ms after power): no answer any of
the four ways, and **each probe took ~185 ms: the driver's timeout, not a
refusal** (an absent device is refused within a millisecond). So
RetroPane's I2C controller never finished its first byte: the fault is on
our side of the bus (the controller, its pins, or the lines held), not
the codec. GPIO83 read 0 with headphones in while powered: the jack line
is likely the codec's own jack-detect output, valid only once Windows
Mobile has set the codec up. Next run logs the bus lines (`IBMR`), the
controller's registers and each failure's kind and time, first with the
codec unpowered.

**SOUND WORKS (2026-10-18 ~18:00, `rp-tone` run 6, owner listening).**
The cause was ours: the I2C driver checked for the codec with an
address-only transfer. The X51v's controller never finishes a byte sent
with START and STOP together (runs 2-4: every probe timed out, 92 ms, while
a START alone was answered in ~120 us), and after an acknowledged address a
master abort left SCL held low (run 5). Fixed as a class (`pxa-i2c`:
address-only writes refused as `Unsupported`, STOP only with the last data
byte, a controller left busy is reset; the WM8750 found by its reset write,
as Windows Mobile starts; guard tests and mutants; the fork's I2C model
does the same). Run 6: the codec acknowledged at `0x1a` in standard and
fast mode, 1 ms and 300 ms after power (neither speed nor timing
mattered); 440 Hz for 514 ms to the headphones by DMA, CPU 0% busy; the
owner unplugged and GPIO83 went low; the speaker amplifier (CPLD `0x14`
bit 1, 70 ms) on and 1000 Hz on the speaker; both board bits put back.

**I2S must stay on through a session (2026-10-18, owner listening).**
With `play` switching I2S off after each note, every note after the first
stalled (run 7, twice): the DMA channel never left its first descriptor
and the I2S FIFO sat full (level 15), so no samples left the controller.
One change each in run 8: the next note **at once: stalled; after 200 ms
off: played; at once after an I2S reset: stalled**. A DMA halt that
waits for the channel's stop (added the same day, kept: a correct start)
didn't change it. Windows Mobile never switches I2S off between sounds
(`SACR0` `0x8805` even after the device closes: `rp-wmaudio`). With I2S
started once and stopped once at the end, **all six notes played**, three
to the headphones and three to the speaker (run 9). Design rule for the
OS's audio service: I2S on for the whole session, off only when audio
powers down after an idle time, as Windows Mobile does.

**How long I2S ignores a switch-on (2026-10-18, `RP_TONE_I2S_GAPS`, no
codec, no board bit):** I2S stopped (DMA first, then `SACR0` = 0), left off
0-500 ms, switched on again, the DMA timed to its first move. The tries
alternated: every other one moved at once (`SACR0` `0x805`); in the
others **`SACR0` read 0 (the switch-on write was ignored)** and the DMA
moved only ~645 ms after the stop whatever the gap (0 ms off: 645; 50:
595; 150: 495; 300: 345). The working tries followed a failed one (I2S
really off well over 645 ms). So: **after I2S is switched off, a write
switching it on within ~645 ms is dropped**. Run 8's "200 ms off plays"
followed a failed try. `PxaI2s::start` now reads `SACR0` back and reports
a dropped switch-on; the OS keeps I2S on through a session.

**The audio service's S1 core (2026-10-18, `services/audio`, ADR-0032;
emulator-verified, device run pending):** 44.1 kHz on both sides
(`SADIV` `0x0D`; the codec's `SR` = 10000, register 8 = `0x020`, from the
datasheet's table as the emulator's model has it); power-up and power-down
in one order each, everything put back on any failure; the I2S restart
guard (700 ms); the jack by GPIO83's edge latch (`GEDR`, both edges),
debounced 50 ms while up. **Found in the emulator and fixed:** switching
the codec off with headphones in drops GPIO83 (it reads high only while
powered), so the latch catches an edge of our own making; the service
clears it at power-down. Because of the same rule, **a jack change while
audio is off most likely makes no edge at all** (Windows Mobile measured:
GPIO83 reads 0 idle, plugged or not): the next power-up reads the jack
anyway. `rp-audio-s1` (`cargo xtask audio-s1`) is the guided device run:
three notes, the jack while up and while down, the guard, and GPIO83
sampled 0/25/75/150 ms after set-up (Dell's driver waits 75 ms: copied
until measured).

**S1 device run 1 (2026-10-18 night, `rp-audio-s1`, owner listening,
headphones in at the start; log `local/rp-audio-s1-run1.txt`).** A
measurement, not a decision:

- **GPIO83 read 0 with the headphones in, at every power-up**: 80 ms
  after the codec's supply, and 0, 25, 75 and 150 ms after its set-up,
  three power-ups alike. So the jack read at power-up (Dell's 75 ms,
  copied) **routed to the speaker while headphones were in**. The line read
  1 later (18.9 s after power, after an edge, during the "plug in" prompt),
  and the service re-routed to the headphones then.
- While audio was off, an edge **did** come during the "unplug" prompt
  (the latch reported it); whether that was the unplug or the line's late
  fall after power-down is not settled by this run.
- The restart guard: down, then straight back up waited 700 ms; the note
  after it played (the X51v's 645 ms rule handled on the device).
- 44.1 kHz: 44,100 frames in 1,016 ms by DMA, each of six notes (1 s plus
  the ~15 ms start and stop seen at 48 kHz too); CPU 0% busy.
- Clean return; both board bits put back as found (`14=0008 1C=000D`).
- **The owner heard nothing in the headphones**, although the log sends
  one note (660 Hz) to them after the re-route. Unlike `rp-tone` run 9 (48 kHz,
  heard on both outputs), this run set both sides to 44.1 kHz: what was
  heard from the speaker, and what was done at each prompt, is being asked.

So the jack line becomes valid much later than 150 ms after set-up (or
only after something Dell's driver does that ours doesn't). Options, for
the owner: (a) a run that samples GPIO83 every 20 ms for 3 s after set-up,
headphones in, to find when it rises; (b) a static read of which codec
registers Dell's driver writes before it reads the jack (a jack-detect
enable such as the WM8750's headphone-switch bits, register 24, would
explain "valid only once set up"); (c) until settled, the service's
power-up keeps both outputs muted until the jack line has been stable for
a measured time. (a) and (b) can be combined; (b) needs no device.

**Dell's start-up, read again after run 1 (static, `wavedev.dll`
`0x02464AA8`-`0x02464BC4`, facts only):** codec reset; **register 25 =
`0x1C2`** (VMID on its 5 kΩ fast-start divider, VREF, mic bias); **75 ms**;
GPIO83 read (`GPLR2` bit 19), then GPIO96 (`GPLR3` bit 0); register 25 =
`0x0C2` or `0x0C0` (the 50 kΩ playback divider, mic bias by GPIO96);
register 16 = `0x080`, register 24 = `0x008`. RetroPane wrote `0x0C0` at
once (the slow divider), which fits run 1's jack line staying low for
seconds. **Changed for run 2:** `0x1C0` at set-up, the jack read 75 ms
later, then `0x0C0`; and GPIO83 sampled every 50 ms for 1 s after
power-up. Registers 16 and 24 are not written yet (their meaning to be
read from the datasheet before copying them).

**S1 device run 2 (2026-10-18 night, `rp-audio-s1` with VMID's fast
start, owner listening, fixed steps; log `local/rp-audio-s1-run2.txt`).**
A measurement, not a decision:

- **The jack read right at every power-up** (80 ms after power, 75 ms after
  set-up): 1 with headphones in (steps 1 and 4), 0 with them out (step 3);
  GPIO83 steady over the next second (every 50 ms). With run 1's slow
  divider it read 0 with headphones in: **VMID's fast start is what makes
  the line valid in time.**
- Step 2 (pulled out while up): the edge latch re-routed to the speaker
  after the debounce.
- **A plug-in while audio was off did make an edge** (step 4: the latch
  reported it). So the jack line is not simply "low whenever unpowered" on
  the board; run 1's open question is answered: edges do come while off.
- The guard: 700 ms, then the note was sent.
- **The owner heard steps 1 and 4 in the headphones as expected, but only
  one note on the speaker across steps 2 and 3**, where the log sends two
  (step 2's 660 Hz right after the re-route, the amplifier on 70 ms before;
  step 3's 440 Hz right after a fresh power-up, the amplifier switched off
  at the power-down and on again ~0.9 s later, 70 ms before the note). Both
  streamed in full (44,100 frames in 1,016 ms). Which one was silent is
  being asked; the amplifier's real settle after a quick off-on is the first
  suspect (Dell's driver sets it once and never cycles it quickly).
- Clean return; both board bits put back as found.

**S1 device run 3 (same image, the owner keeping the headphones out
through step 3; log `local/rp-audio-s1-run3.txt`):** the log matches run
2 line for line (the jack right at every power-up, both speaker notes
streamed in full); **the owner again heard only one of the two speaker
notes** in steps 2-3. Next: a diagnostic run on the speaker only, one
numbered test per candidate (the amplifier switched off and on quickly,
with 70 ms and longer settles; the power-down and straight back up).

**Speaker amplifier diagnostic (2026-10-18 night, `rp-audio-s1` built
with `RP_AUDIO_AMP_TEST`, owner listening, headphones out; log
`local/rp-audio-s1-amptest1.txt`).** Five numbered notes on the speaker,
all streamed in full. **The owner heard tests 2-5 and not test 1.** A
measurement, not a decision:

| Test | What came before the note | Heard |
|---|---|---|
| 1 | The first power-up of the run; the amplifier switched on for the first time since Windows Mobile left it off (`14=0008`), 70 ms, then 1 s of jack sampling: on for ~1.1 s before the note | **no** |
| 2 | The amplifier off 0.9 s, on, 70 ms | yes |
| 3 | Off 0.9 s, on, 500 ms | yes |
| 4 | Off 3 s, on, 70 ms | yes |
| 5 | Audio down and straight back up (the amplifier off ~1 s) | yes |

So **the first speaker sound after the amplifier has been off a long time
is lost** (more than ~2 s from its switch-on, the note included), while
it comes back at once after being off for up to 3 s. It fits runs 2 and 3
too: their missing speaker note was step 2's, the run's first sound on
the speaker. A slow first start (a bias or bypass capacitor charging, or
the amplifier's own soft start) is the likely cause; Dell's 70 ms settle
doesn't cover it. Options for the owner: (a) a timed run from cold (the
amplifier switched on, then short numbered notes every 0.5 s for 5 s) to
measure how long the first start takes; (b) a static read of whether
Windows Mobile keeps the amplifier on longer, or switches it on earlier,
than its 70 ms wait suggests; (c) design: the service switches the
amplifier on at power-up whenever the speaker may be used, early enough
to cover the measured start, and keeps it on while audio is up.

**Dell's private equalizer controls (2026-10-18, from X50Mix, read only):**
`WAV1:` control codes `0x1D2000` (set) and `0x1D2004` (get), a 9-byte
block each, handled by `wavedev.dll` `0x02463E54`; the hardware is the
WM8750's bass/treble (tone) controls. The CE personality's `WAV1:` must
answer them for X50Mix and anything else that uses them
(docs/design/apps-and-packs.md §6b).

Remaining difference between Windows Mobile's working transfer and
`rp-tone`'s unanswered one: the I2C speed (fast vs standard) and the wait
after power (~260 ms vs 50 ms). Next run (owner watching): each alone,
then both, in one run (one variable per try).

**What changes:** `rp-drv-wm8750`'s caller raises GPIO82 (as an output)
around each codec write and lowers it after, as Windows Mobile does (never
leaving it high: the line is shared with another device, the lock's other
user unknown); the jack is GPIO96, low = in. The speaker amplifier's line
is open again (GPIO82 was AxDroid's guess; CPLD 0x14 bit 3 remains a
candidate). The emulator's codec answers without GPIO82: add the gate to
the fork's model (docs/hw/emulator.md).
