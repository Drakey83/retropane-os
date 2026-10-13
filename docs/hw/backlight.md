# X51v backlight (PXA27x PWM0)

> Spec v0.1, 2026-10-12. Facts only, per [`../CLEANROOM.md`](../CLEANROOM.md).
> **Sources:** (M) Intel PXA27x Developer's Manual, PWM chapter; (C) AxDroid
> board code (via `docs/research/hardware-inventory.md`); (P) RetroPane's
> probe on the owner's X51v.

## Board

| Item | Value | Confidence | Evidence |
|---|---|---|---|
| Backlight brightness | PWM0, output on GPIO17 | confirmed (code) | (C) |
| Backlight enable | CPLD `0x18` bit 2 (`0x0004`), set = on; Windows Mobile leaves it on | confirmed | Dell's `Backlight.dll` and display drivers set it (cpld.md (D)); clearing it darkened the panel on the owner's X51v, setting it lit it again (2026-10-12) |
| PWM1 | running (duty 0x46 of 0xD7) as WM leaves it; purpose unknown (LCD contrast? keypad light?) | open | (P) |

## PXA27x PWM (M)

| Register | Offset (PWM0 at `0x40B00000`, PWM1 `0x40C00000`; PWM2/3 at +0x10) | Fields |
|---|---|---|
| `PWMCR` | 0x0 | `[5:0]` prescale (the counter runs at 13 MHz / (prescale + 1)), `[6]` abrupt shutdown |
| `PWMDCR` | 0x4 | `[9:0]` duty: counts high per cycle; `[10]` full duty |
| `PWMPCR` | 0x8 | `[9:0]` period: the cycle is period + 1 counts |

Clock: `CKEN[0]` (PWM0 and PWM2).

## As Windows Mobile left it (P, 2026-10-11)

| Register | Value | Meaning |
|---|---|---|
| `PWMCR0` | `0x01` | prescale 1: 6.5 MHz counter |
| `PWMPCR0` | `0xD7` | 216 counts per cycle: ~30 kHz, far above hearing |
| `PWMDCR0` | `0x89` | 137 / 216 = 63% duty (RetroPane's perceptual scale: ~80%) |

## How RetroPane drives it (`rp-drv-pxa-pwm`)

Levels are perceptual percent: duty = level² of full (50% looks half as
bright, not nearly full), at least one count for any level above 0, full
duty (`PWMDCR[10]`) at 100%. Windows Mobile's period is kept: a longer
period would give finer low steps but a lower frequency, towards the
audible range where backlight circuits whine; with 216 counts the lowest
step (1 count) looks like ~7%. Changes fade in 16 steps over ~¼ s.
`cargo xtask backlight [0-100]` through the loader.
