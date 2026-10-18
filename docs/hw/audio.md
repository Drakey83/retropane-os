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
| Speaker amplifier | GPIO82, plus CPLD 0x14 bit 3 ("audio amplifier power") | likely (code); polarity unknown |
| Headphone detect | GPIO83 | likely (code) |

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
| **Headphone detect is GPIO96, low = plugged in**, not GPIO83 high: the first run's "GPIO83 0 with headphones in" fits | likely (the flag's meaning is inferred from its use) | (D): `GPLR3` bit 0 tested, the "in" state set when it reads 0 |
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

**What changes:** `rp-drv-wm8750`'s caller raises GPIO82 (as an output)
around each codec write and lowers it after, as Windows Mobile does (never
leaving it high: the line is shared with another device, the lock's other
user unknown); the jack is GPIO96, low = in. The speaker amplifier's line
is open again (GPIO82 was AxDroid's guess; CPLD 0x14 bit 3 remains a
candidate). The emulator's codec answers without GPIO82: add the gate to
the fork's model (docs/hw/emulator.md).
