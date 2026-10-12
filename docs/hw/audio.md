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
- Loader `tone <hz> <ms>` (`cargo xtask` control frame): finds the codec on
  first use, plays a sine at 48 kHz polled into the FIFO, replies with how
  long it took (the codec's clock paces it).
- Not yet: DMA streaming, the speaker amplifier, headphone detect,
  recording, sample rates other than 48 kHz on the codec side.

## Device run (with the owner there: it makes sound)

`tone 440 500` with headphones in: the reply names the codec's address
(recording it here), the time matches 500 ms if the clocks are right. Then,
for the speaker, the amplifier lines (GPIO82, CPLD 0x14 bit 3) one at a
time.
