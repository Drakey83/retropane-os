# Getting the most out of the X51v's screen (plan, 2026-10-17)

**Owner's goal (2026-10-17):** make the screen as calibrated and as
optimized as our control of the hardware and software allows, then make
every game port avoid the downfalls that make the GLES games dark and
muddy. The games' half is a standing rule already
([apps-and-packs.md](apps-and-packs.md) §6a): textures at 16 bits dithered
or better, never 4 bits a channel, and a brightness curve per game. This
page is the screen's half. **A plan: nothing below is built yet.**

## What we control, and what we don't

| Control | What it changes | State | Risk |
|---|---|---|---|
| **Gamma table** (`GFXGAM0`-`16`: 17 points per colour, 8 bits each, applied as pixels go to the panel) | Brightness curve, contrast curve, white balance (each colour separately). Free: no work per frame | Format confirmed on the device (`rp-nightlight`); Windows Mobile leaves a straight ramp | None: a table, put back on exit |
| **Backlight** (PWM0, 432 steps at 30 kHz) | How bright the panel is; battery | Ours since 2026-10-16; Windows Mobile runs it at 137/216 = 63% | None up to full duty; battery cost to measure |
| **Pixel clock / refresh** (the 2700G's PLL and timings) | 55-56 Hz today; smoother motion at 60 Hz | Dell's 23.56 MHz is a little *under* the Sharp's 24.0-26.3 MHz range ([research/screen-swap.md](../research/screen-swap.md), datasheet S); 25.2 MHz typical gives ~60 Hz | Low, owner watching: a wrong timing only shows a bad picture until the next start |
| **PWM1** (Dell sets a panel-specific constant: 70 of 216 on the Sharp) | Unknown. Not the panel's bias supplies (those are made on the panel's own flex from 3.3 V, datasheet S); possibly a contrast or reference input | Untested | Unknown: research first, then small steps with the owner watching and Dell's value put back (no blind board bits) |
| **Dither** (4 x 4 on the engine, Floyd-Steinberg for pictures at load) | Smooth gradients on a 16/18-bit panel | Done; 16-bit wallpaper indistinguishable on the device (ADR-0023) | None |
| **Content** (wallpapers, icons, themes) | Saturation and contrast baked in once when converted; glass darkness, text brightness | Ours | None |
| Panel colours, black level, viewing angle | Physical: the panel's filters and backlight leakage (200:1 contrast, 165 nits, NTSC 35%) | Fixed | Only a new panel changes these (parked: research/screen-swap.md) |

There is no colorimeter: everything is judged by the owner's eye on the
device, with test patterns that make the judgement easy (as the 16-bit
wallpaper side by side was).

## Steps

1. **C1, `rp-display-cal` (test image): the gamma table, by eye.** Test
   patterns on the real panel: near-black steps (0-32), near-white steps
   (224-255), grey and colour ramps, a gamma checkerboard, colour bars, and
   a dark synthetic "game" scene (our own drawing, no game data). A tap
   flips between curves: stock (straight), shadows lifted, an S-curve for
   contrast, warmer and cooler white; the D-pad changes the chosen one's
   strength. The log records each choice. Owner picks the system default
   and a "dark games" default. No risk.
2. **C2, backlight range and cost.** The same image steps the backlight to
   full: how much brighter than Windows Mobile's 63% it gets, and the
   battery cost (current draw from the power IC when that's read; until
   then, the owner's sense of it).
3. **C3, refresh at 60 Hz.** Static check first (the Sharp's timing limits,
   Dell's panel table), then one owner-watched run that moves the pixel
   clock into the panel's range: steadier picture, smoother games, and
   no longer under the datasheet's minimum. Put back on exit.
4. **C4, what PWM1 is.** Static: Dell's display driver (facts only), the
   Sharp datasheet's pin list, the panel connector. Only if it's clearly a
   contrast-like input: a step test with the owner watching.
5. **C5, settle it.** The results become: the system's default curve and
   backlight range, the Display settings design (brightness, warmth,
   contrast if C4 finds a knob), and each game's default curve (§6a).

**Games, meanwhile:** the ports themselves come later (ROADMAP Phases
4-7). What can be done now: read DoomGLES's texture uploads (static, its
`glTexImage2D` calls) to learn whether the game sends 4-bit data or Dell's
driver made it 4-bit. If Dell's driver did, our own GLES driver fixes the
closed games too, simply by keeping their textures at 16 or 32 bits.
