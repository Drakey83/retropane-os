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
| PWM1 | running (duty 0x46 of 0xD7) as WM leaves it. Set only by Dell's display drivers at display start-up, to a panel-specific constant (one value for the Samsung panel, another for the Sharps; on the owner's Sharp it works out to 70 of 216): most likely the panel's bias or contrast, **not** a backlight brightness. RetroPane leaves it alone | likely | (P); (D) Dell's `ddi_2700g` and `Backlight` (static analysis, 2026-10-15: the PWM helper called with channel 1 from the display driver only, the value chosen by panel type) |

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

## Finer and lower steps (item 76, 2026-10-15)

| Fact | Confidence | Evidence |
|---|---|---|
| The PWM counts 13 MHz / (prescale + 1); Windows Mobile's prescale 1 and 216 counts make 30.09 kHz, so prescale 0 and 432 counts make exactly the same 30.09 kHz with steps half the size. A longer cycle at prescale 0 would lower the frequency (649 counts: 20 kHz, the edge of hearing) | confirmed (arithmetic, M) | (M) |
| One count of 216 is Windows Mobile's dimmest (~7% on the perceptual scale); one of 432 would be ~5% | confirmed (arithmetic) | |
| The display's graphics gamma table (`GFXGAM0`-`16`, `0x2250`-`0x2290`, 17 entries, used while `GSCTRL[26]` is set, which Windows Mobile leaves on) is applied at scan-out | likely | Intel's 2700G register names (mbxfb headers, B); `rp-light` (`GSCTRL` gamma bit set) |
| The gamma table's entries are `0x00RRGGBB`, 8 bits per colour; Windows Mobile leaves a straight grey ramp (`00`, `0F`, `1F` ... `EE`, `FD`); scaling it dims the picture on the device | confirmed | P, 2026-10-16, `rp-nightlight` (`local/rp-nightlight-run1.txt`), steps 4-5 seen dimmer |

**RetroPane (`rp-drv-pxa-pwm`):** the backlight is taken over at prescale 0
and 432 counts, keeping the brightness found (Windows Mobile's 137/216
becomes 274/432). Brightness is two-stage (`split`, in tenths of a
percent): down to one count only the light moves; below it the light stays
at one count and the **picture** is scaled through the gamma table, to 1/8
at the darkest: one eighth of half of Windows Mobile's dimmest, with no
CPU work per frame, nothing flickering and no new frequency to hear. The
picture stage waits for the gamma table's format from the device.

Not chosen: temporal dithering of the duty by the CPU (between n and n+1
counts every half millisecond or so). It needs a timer interrupt thousands
of times a second for as long as the screen is dim (the CPU never idles
deeply), and its modulation is in the audible range; `rp-nightlight` step 6
lets the owner judge it, for the record.

**Device check (`cargo xtask load rp-nightlight`, the owner watching and
listening):** steps 1-8 compare Windows Mobile's dimmest with 2/432 (the
same?) and 1/432 (dimmer, still lit?), the picture at 1/2 and 1/4, the
dithered duty, and listen for whine at half brightness on both cycles.

**Device result (P, 2026-10-16, three runs: one to log, one to watch, one
to listen; `local/rp-nightlight-run1..3.txt`).** All eight steps ran and the
light was put back as found. The screen stayed lit through every step,
including 1/432 and the 0-1/432 dithering. Step 5 (1/432, picture at 1/4)
was the dimmest and clearly dimmer than Windows Mobile's dimmest (step 1).
No whine or other sound on any step. Caveat, the owner's: high pitches are
hard to hear at 43. That matters little here: the cycle is 30 kHz, beyond
anyone's hearing, and step 6's 1 kHz modulation is well inside an adult's
range, so its silence is a real result; a coil's sub-harmonics can't be
ruled out by ear. The two-stage design stands; its darkest picture stage
(1/8) is one step past what was shown (1/4) and is checked by eye when the
night setting gets its UI.

## The night setting's control (decided 2026-10-16: A and C)

The device runs settled the mechanism (above): the light down to one count
of 432, then the picture through the gamma table, to 1/4 seen and liked
(the owner's darkest of steps 1-8) and 1/8 to judge (`rp-nightlight`
step 9). What's left is how the user reaches it. It lives in Settings
(ROADMAP: not built yet); keys keep Windows Mobile's jobs (no new button).

- **A. One brightness slider that goes further down (proposed).** Windows
  Mobile's Backlight settings have one brightness slider; RetroPane's goes
  below Windows Mobile's dimmest: the bottom of the slider first lowers the
  light to one count, then dims the picture. Pros: nothing new to learn, one
  control, the same place as on Windows Mobile. Cons: someone can slide into
  a very dark screen by accident; so the slider's night range starts below a
  visible mark, and the power button's off/on always comes back at least at
  Windows Mobile's dimmest (a dark screen is never a trap).
- **B. A separate "Night" switch.** The slider keeps Windows Mobile's range;
  a switch next to it lets the slider go into the night range. Pros: no
  accidental darkness. Cons: a second control Windows Mobile didn't have, and
  two things to set for one result.
- **C. On a schedule** (dark after a set time). An extra on top of A or B,
  later; not a substitute.

**Decided (owner, 2026-10-16): A and C.** One brightness slider reaching
into the night range, and a schedule that moves that same slider: at the
evening time it slides to the user's night level, in the morning back to
where it was (one state, so the schedule and a hand on the slider can't
disagree; a hand change during the night holds until the next scheduled
time). Built with Settings (ROADMAP); the schedule needs the clock (the
PXA27x RTC) and two times plus a level stored in settings.

No cost in memory or speed either way: the gamma table is 17 registers
written once per change, nothing per frame.

## How RetroPane drives it (`rp-drv-pxa-pwm`)

Levels are perceptual percent: duty = level² of full (50% looks half as
bright, not nearly full), at least one count for any level above 0, full
duty (`PWMDCR[10]`) at 100%. The cycle is RetroPane's (above): 432 counts
at prescale 0, Windows Mobile's frequency, the lowest step (1 count)
looking like ~5% instead of ~7%. Changes fade in 16 steps over ~¼ s.
`cargo xtask backlight [0-100]` through the loader.
