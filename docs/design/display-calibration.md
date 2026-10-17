# Getting the most out of the X51v's screen (plan, 2026-10-17)

**Owner's goal (2026-10-17):** make the screen as calibrated and as
optimized as our control of the hardware and software allows, then make
every game port avoid the downfalls that make the GLES games dark and
muddy. The games' half is a standing rule already
([apps-and-packs.md](apps-and-packs.md) §6a): textures at 16 bits dithered
or better, never 4 bits a channel, and a brightness curve per game. This
page is the screen's half. **C1 is built and checked in the emulator
(2026-10-17) and ran on the device (run 2: findings under C1). C2-C5 are
not built yet.**

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

   **Built (2026-10-17): `boot/display-cal`, `cargo xtask load rp-display-cal`.**
   The picture is drawn once into a 32 bpp plane, so a curve change touches
   only the table: near-black 0-32 and near-white 224-255 in steps of 4 (one
   step per level of the 18-bit panel), grey and R/G/B ramps, a gamma check
   (squares that match 50% line pairs on panels of gamma 1.0, 1.5, 1.8, 2.2,
   2.5 and 3.0), colour bars, and a dark corridor drawn from arithmetic with
   a torch and a status bar. Curves: Stock (the table as found), Shadows,
   Contrast (S-curve), Warm, Cool, each 0-10 and starting at 5. Each curve is
   applied to the table as found, so Stock is exactly what Windows Mobile
   left. Controls: tap or D-pad right/left for the next/previous curve, up/down
   for strength, centre for A/B (the curve against Stock), Home when done.
   Everything is put back as found; it returns by itself after 10 minutes
   with no input. `cargo xtask emu-display-cal` checks the picture
   pixel-exact through Stock, Shadows 5 and 6, A/B and Contrast 5, driven
   by keys and a tap. A mutant (curve changes never reach the table) must
   fail it. 25.9 KiB, budget 29 KiB.

   **Device run 2 (2026-10-17, owner watching; run 1 timed out unwatched,
   cleanly):** picture drawn in 102 ms. The owner flipped through every
   curve (61 changes, strength left at 5 throughout) and pressed Home.
   Everything was put back as found. Findings, by eye:
   - **Near-black: 6-7 of the 9 steps visible.** The bottom 2-3 (0-8 of
     255) all look black: shadow detail is lost on the stock curve.
   - **Near-white: all 9 steps look like one band.** 224-255 are not
     told apart: the top of the range is clipped (panel or backlight).
     Content above about 220 is wasted on this screen.
   - **Gamma check: inconclusive.** No square vanished into the line
     pairs. On this panel the one-pixel lines don't blend into a flat
     grey at arm's length. A different pattern is needed to measure the
     panel's gamma (e.g. a 2 x 2 checkerboard, or a ramp matched against
     dithered halftones).
   - **Pick: Shadows 5 looks best "when it comes to being able to see"**
     (overall and for the dark corridor). Strengths other than 5 were
     not tried.
   **Run 3 built (2026-10-17, owner's choice A):** two new curves after
   Shadows: **Highlights** (linear to 128, the rest squeezed up to
   255 - 5 x strength, so 205 at 10: the near-white steps land under the
   clip, apart) and **Shadows+Highlights** (both at one strength). The
   gamma check is now a one-pixel checkerboard (no rows to see). The
   caption's second line names the controls ("Up/Dn strength") and turns
   yellow while A/B shows Stock. emu-display-cal: pixel-exact through
   Stock, Shadows 5/6, A/B, Highlights 5. 27.6 KB. Then settle the system
   default (C5).

   **Device run 3 (2026-10-17, owner watching, 181 changes, Home, all put
   back):** Highlights barely helps: at 10, 3-4 of the 9 near-white
   squares are barely told apart. That is about the arithmetic limit: the
   squares are one 18-bit panel level apart (steps of 4), and squeezing
   224-255 into 186-203 leaves about 4 panel levels for 9 squares.
   Squeezing can't add levels. **Shadows+Highlights looks best at 10, the
   strongest setting**, so the range should go further. Owner: Part 1
   "needs a bigger change" to judge. Next: wider-step near-white squares
   (where does white stop getting brighter?) and strengths past 10.

   **Run 4 rebuilt as dials (2026-10-17, owner: "steps 1-50 ... really
   make drastic changes", one number per question, plus any other numbers
   I need):** the curves are replaced by five dials, each 1-50, seen all at
   once in the caption (the chosen one yellow). D-pad up/down picks a dial,
   right/left moves it (held: repeats), centre compares with the original
   (caption red, "ORIG"), Home finishes. All dials combine into one table.
   | Dial | Question it answers | 1 | 50 |
   |---|---|---|---|
   | Black | the setting where all 9 near-black squares show | none | black raised to 98 |
   | White | the setting where all 9 near-white squares show | none | white down to 108, the top 64 levels kept 4 apart (the squeezing of run 3 lost levels; this doesn't) |
   | Mid | which gamma box (1-6) matches the checkerboard, at which setting | mids much darker (25 = none) | mids much brighter |
   | Warm | where the grey ramp looks neutral (no blue or yellow) | red at 61% (cool; 25 = none) | blue at 61% (warm) |
   | Bright | the backlight the owner wants (C2's question, asked here) | 1/50 of full | full |
   The gamma boxes are now labelled 1-6 (panel gamma 1.0, 1.5, 1.8, 2.2,
   2.5, 3.0). The run log keeps every setting once it settles (1 s) and the
   last one with its table. Touch input dropped (D-pad only). emu-display-cal:
   pixel-exact through the original, Black 2, White 2, compare and back,
   Warm 24, with a mutant. 27.7 KB. **This image is the seed of the
   screen-calibration tool in Settings** (ROADMAP, Phase 5): every owner's
   aged panel dialled in with the same patterns.
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
4-7). What could be done now was a static read of DoomGLES's texture
uploads, to learn whether the game sends 4-bit data or Dell's driver made
it 4-bit.

**Result (2026-10-17, static read of the owner's DoomGLES.exe; Doom2GLES
is the same program):** neither the game's code nor Dell's driver picks the
depth. **The game's data files do.** Almost every texture comes from a
pre-converted PowerVR `.pvr` file (`\OpenGLES\<game>\*.pvr`: per-level
walls and flats, sprite pages, skies, fonts, `haze.pvr`). The game's built-in
PVR loader reads each file's pixel-type field and passes the matching
format straight to `glTexImage2D`: 4444, 5551, 565, 8888, luminance, or
PVRTC through `glCompressedTexImage2D`. It uploads every mip level the file
holds. Only one 128 x 128 texture (likely the haze/corona) is built by the
game itself, as 8-bit RGBA. There is no `glTexSubImage2D` and no palette
path (it checks for paletted textures and reports them unsupported).
Confidence: *confirmed* for the code path; *likely, not confirmed* that the
files are 4444. Next check: byte 0x10 of any `.pvr` header on the card
(0x10 = RGBA4444). If they are 4444, our own driver alone won't fix
DoomGLES. Converting the textures again from the WADs at 565/8888 would
(the native port does that anyway). Closed games must be checked one by
one: those that ship 4444 data stay 4444 whatever the driver does. The
game's only picture setting is `usegamma` 0-4 in `doomgles.cfg`.
