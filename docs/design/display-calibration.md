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
| **PWM1** (Dell sets a panel-specific constant: 70 of 216 on the Sharp) | **The panel's VCOM** (C6 research, 2026-10-17): flicker and DC balance, not contrast | Untested | Low if small: steps inside Dell's 20-820 window, owner watching, Dell's value put back |
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

   **Device run 4 (2026-10-17, owner watching, 1649 changes, Home, all put
   back; the backlight found at 274/432 = 63%, Bright 32):** each dial was
   set alone, the others back at the original:
   | Dial | Owner's setting | Means |
   |---|---|---|
   | Black | 8 ("8?") | black raised to 14: all 9 near-black squares show |
   | White | 23 | white down to 189: all 9 near-white squares show |
   | Mid | 16, box 4 | box 4 (gamma 2.2) matches only with mids darkened: the panel shows mid-tones brighter than standard (about gamma 1.8) |
   | Warm | 20 | slightly cooler: the panel leans warm/yellow |
   | Bright | 50 | full backlight preferred over Windows Mobile's 63% |
   Owner's two problems: (1) **saturation drains away** with no dial to
   bring it back. The gamma table bends each colour on its own, so it can't
   add saturation, and Black and White squeeze the range, which washes
   colours out. Saturation needs channel mixing: in content (pictures,
   game rendering) or a 2700G colour matrix if one exists (open). (2)
   **At the original settings the corridor is too dark to see.** Expected:
   that is Windows Mobile's own curve, and the scene lives at levels 4-60
   where the panel crushes the bottom. Next: ready-made settings built from
   these numbers, plus a saturation dial that redraws the picture (a
   preview of what the OS would apply to content).

   **Run 4's log, read afterwards:** the owner's "too dark at the original
   numbers" was the test's fault. Black and White started at 1, Mid and
   Warm at 25. After the gamma check the owner reset Mid to 1 (Mid's
   darkest), and the Warm and Bright steps ran with it there. Black 8,
   White 23 and Mid 16 at box 4 stand. **Warm 20 and Bright 50 must be
   measured again.**

   **Run 5 built (2026-10-17): a guided test.** Every dial is 1-50 with
   **25 = unchanged** (Black below 25 deepens the shadows, White below 25
   pushes the top into white). Six steps, one dial each: Black, White, Mid,
   Warm, Bright (25 = the backlight as found, 50 = full), **Sat**
   (saturation, 25 as drawn, 1 grey, 50 double; the image redraws the
   picture, since the gamma table can't mix colours). A seventh step, **All
   together**, shows every pick at once. D-pad down = next step, up = back:
   moving puts every other dial back to 25 (the owner's request). Right/left
   adjust, centre compares with the original, Home finishes. The log keeps
   each step's pick and all picks with their table. emu-display-cal walks
   the steps pixel-exact (including the reset and the saturation redraw),
   with a mutant. 28.8 KiB, budget 32 KiB. Run 4's numbers on the new scale:
   Black 8 = about 29, White 23 = about 36, Mid 16 = 16.

   **Device run 5 (2026-10-17, owner watching, 1698 changes, Home, all put
   back):** Black 28, White 26 (owner said 27), Warm 22, Bright 35 (78% of
   full, vs Windows Mobile's 63%), Sat 30 (1.2x). **Gamma check, every box:**
   box 1 (gamma 1.0) vanishes at Mid 37, box 2 (1.5) at 25, box 3 (1.8) at
   19, box 4 (2.2) at 14, box 5 (2.5) at 12, box 6 (3.0) at about 1. So **the
   panel with Windows Mobile's table shows mid-tones like gamma 1.5**
   (bright and flat; standard is 2.2), and Mid 14 makes it standard (run 4:
   16, consistent). "All together looks much worse for the corridor": the
   test's fault again. Step 3 is a measurement, but its last value (Mid 8,
   left there after testing box 5) was used as a pick, darkening every
   mid-tone.

   **Run 6 built: the last step is "Looks"**, ready-made settings made from
   these numbers, flipped with left/right: Original, **Accurate** (the picks
   with Mid 14: standard gamma 2.2), **Everyday** (the picks with Mid 25:
   the panel's own bright mid-tones, the starting look), **Dark games**
   (Black 31, Mid 38, Bright 45, Sat 32: the corridor easy to see). The
   owner picks a look; it becomes the system default (C5).

   **Viewing angle (owner, 2026-10-17): the numbers vary a lot with the
   angle the screen is seen from.** Expected on this panel type (twisted
   nematic): tilting it up or down shifts the mid-tones strongly, side to
   side less. So every reading is only true at one angle. Rules for the
   tool: judge at the angle the device is normally held (straight on, at
   reading distance, ideally propped so it stays put); prefer settings that
   still look right a little off-axis over the ones that are best at
   exactly one angle; the install step says how to hold it. A colorimeter
   reading (layer 3 in the roadmap) is taken straight on, at one angle too.
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

## C3, refresh at 60 Hz (2026-10-17)

**Static check:** the display PLL (`DISPPLL`, `13 MHz x M / (N x 2^P)`) holds
Windows Mobile's `0xEC5` = M 29, N 4, P 2 = **23.5625 MHz**, Dell's pixel
clock, under the Sharp panel's 24.0-26.3 MHz (typical 25.2). With the X51v's
frame (648 clocks a line, 645 lines, measured) that is **56.4 Hz** (56.46
measured). Changing only M to 31 gives **25.1875 MHz, the panel's typical
clock, and 60.3 Hz**; the PLL's internal frequency stays near Windows
Mobile's (13 x 31 / 4 = 100.75 MHz against 94.25), the porches, syncs and
frame size don't change.

**`rp-refresh` (test image, 24.1 KiB, owner watching):** a 32 bpp picture with
a bar moving one step a frame and a grey ramp; frames counted from VEVENT0
over 2 s, the under/overrun bits recorded (the display running short of data
at the faster clock, a risk at 32 bpp). D-pad centre switches between the two
clocks (each switch waits for the PLL's lock and measures again); Home, or 3
minutes idle, puts the PLL and display back. It changes the PLL only when it
finds exactly `0xEC5`. Risk: a wrong clock shows a bad picture until Home.
`emu-refresh` checks the refusal to switch from an unknown value (the
fork's PLL reads 0) and the frame count; the PLL isn't modelled yet
(docs/hw/emulator.md).

**Device run (2026-10-17, owner watching, `local/rp-refresh-run1-log.txt`):**
found `0xEC5`, `PLLSTAT` 7. Measured 56.0 and 56.5 Hz at Dell's clock, **60.0
and 60.5 Hz at `0xFC5`** (2 s counts: half a hertz per frame counted), the PLL
locked at once each time, **no under/overrun bits at 32 bpp** at either
clock, put back to `0xEC5` on Home. Owner: "60 Hz ran well", nothing wrong
seen. **Result: the panel runs at its typical clock and 60 Hz with no
faults.** **Decided (owner): 60 Hz is RetroPane's default.** Built:
`Display::adopt` (every image's and the loader's takeover) calls
`refresh_60`, which moves exactly Windows Mobile's `0xEC5` to `0xFC5` and
leaves any other value alone (`pll::WM`, `pll::SIXTY` in rp-drv-mbx-display;
host test). Windows Mobile's own boot sets its clock again, so going back to
it is untouched. The loader shows 60 Hz once reinstalled on the card; images
already do. Still to do before it's final: a long run under a heavy 3D load
(the renderer's benchmark at 56 and 60 Hz: the scan-out takes 7% more memory
time) and a sleep/wake; and a **56 Hz compatibility switch per legacy
program**, for a game that times itself by the refresh (DirectDraw's
vertical-blank wait). For the flash-installed OS (no Windows Mobile boot to
set the PLL), RetroPane programs `0xFC5` itself.

## C6, the best picture this panel can give: full colour management (plan, 2026-10-17)

**Owner (2026-10-17):** "use those readings while researching that screen, to
give it the best contrast, brightness, saturated and color accurate picture",
however long it takes. The three measured tables of C1 were a first pass from
31 readings; the curves alone can only move tones around inside what the panel
shows, which is why Everyday and Dark games looked washed out. This is the
full job, as a professional display calibration does it, within the panel's
limits.

**What the panel is (measured, runs 2-3):** white 41 cd/m2 at Windows Mobile's
63% backlight, 57 at full (new: 165); black 1.89 (the screen's own glow, 0.45
even with the backlight off); contrast 22:1 (new: 200:1); white about
13,300 K (very blue: aged LEDs); primaries red 0.488/0.305, green 0.295/0.426,
blue 0.151/0.134 (a small gamut, the datasheet's NTSC 35%); grey curve gamma
1.80 with the top clipped (gamma 1.05 at 240). Viewing angle shifts tones
strongly (twisted nematic).

**What we control:** the backlight (to full); the 2700G's 17-point table per
colour (tone and white balance, free); everything about content, applied once
on the CPU or GPU: a colour transform for pictures at load, game textures at
upload (our own GLES driver), theme colours at build; PWM1 (unknown, C4); and
possibly a colour matrix in the 2700G's video plane (to check).

**Steps:**
1. **Research first.** C4: what Dell's PWM1 drives (Dell's display driver,
   facts only; the Sharp datasheet's pins; the board). If it moves the panel's
   black, contrast is the biggest win left. Also: whether the backlight driver
   can give more than full PWM (Dell's driver facts; current set by a resistor
   would mean hardware only), and whether the 2700G's video plane has a
   programmable colour matrix (mbxfb register facts).
2. **Profiling measurement.** A patch set made for profiling (about 150): each
   channel at 17 levels, greys at 17, secondaries, a colour set from Argyll's
   `targen` (skin, sky, foliage, saturated), and a separate verification set.
   Speed first: try driving `spotread` interactively through a Windows
   pseudo-console (ConPTY) to cut a reading from about 10 s to about 2 s; else
   one-shot readings (about 40 minutes).
3. **Profile.** Readings to an Argyll `.ti3`; `colprof` builds the panel's
   profile (shaper/matrix and a table-based one); `profcheck` gives the fit.
4. **Calibration, in the 17-point table, at full backlight:** neutral greys
   from black to white at a chosen white (the brightest that looks neutral:
   candidates D65, about 7,500 K and about 9,000 K, each with its brightness
   cost), the tone curve BT.1886 with the measured black (made for screens
   with raised black: shadow detail without the wash-out of a lifted curve).
   Measure greys through it, refine, two or three passes.
5. **Content colour management:** a 3D table (17 x 17 x 17, about 15 KB) from
   sRGB to the panel, made with Argyll's `collink` in three intents: accurate
   (relative colorimetric), balanced (perceptual), vivid (saturation, for
   "pop"); applied with integer interpolation on the device (no floats) to
   pictures, textures and theme colours.
6. **Pop for pictures (optional):** local contrast and a touch of sharpening
   per picture at load, judged on the device.
7. **Test image with real photos** (the wallpapers) and a colour chart: looks
   Original, Accurate, Balanced, Vivid, at full backlight, A/B against
   Original; the owner and family pick.
8. **Verify:** the chart measured through the chosen pipeline, a colour error
   (Delta E 2000) report per patch. The result replaces C1's three looks.

### C6 step 1: research results (2026-10-17, static, facts only)

**PWM1 is the panel's VCOM** (the common-electrode voltage), confirmed (code).
Dell's display drivers (`ddi_2700g.dll` and `ddi.dll`, the same routine) set
it once at display start-up: they look in RAM at `0xAC002608` for the tag
`"MOCV"` (`0x56434F4D`, "VCOM" read backwards) and, if it is there, take a
16-bit value at `+4`, accepted only from 20 to 820; with no tag they use a
per-panel default: **330 for the owner's panel type** (the CPLD's LCD-type
bits read 0), 280 for the other. The value is in thousandths of the PWM's
cycle: duty = value x 215 / 1000 (PWM1, prescale 1, 216 counts, 30 kHz,
filtered on the board into a DC level). 330 gives **70 of 216, exactly what
the owner's Axim runs**, so it uses Dell's default (or a factory value equal
to it); nothing in Windows Mobile's image writes the tag (searched: only the
two display drivers and their copies in the ROM read it).
- What VCOM does on an LCD: it centres the alternating drive across the liquid
  crystal. At the right value the picture doesn't flicker and no DC sits on
  the cells; off it, the picture flickers at half the refresh rate and, held
  for hours, images stick. It is **not a contrast control**: it may move black
  slightly, mostly it trades flicker. Dell's range check (20-820) is the safe
  window its code allows.
- **Worth one measured sweep, nothing more:** a few steps either side of 330
  (for example 290-370), each held under a minute, black and white read by the
  Spyder, a flicker pattern judged by eye, then 330 put back. If one value has
  a lower black and no flicker, it is a free gain; if not, Dell's default stays.
  Owner watching, steps small, never outside Dell's 20-820.

**Backlight maximum: full duty is the ceiling software has.** Dell's backlight
table caps the owner's panel type (and one other) at 137 of 216 (63%, levels
0-137 in nine steps), a third panel type at 193. RetroPane already goes to full duty
(57 cd/m2, measured). The LED current is set by the board's boost driver
(constant current); no register, CPLD bit or driver call that changes it was
found in Dell's code (only PWM0's duty and CPLD `0x18` bit 2, the enable).
More light means hardware (a new panel's fresh LEDs, or the driver's current
resistor), not software.

**No colour matrix for the screen's picture.** The 2700G has colour-conversion
registers `CSC01`-`CSC05` (`0x2330`-`0x2340`) but they belong to the **video
plane** (`VSCTRL[25]` turns on its YUV-to-RGB conversion), and the video plane
takes only YUV formats. Neither Dell's display driver (its register table has
no `CSC` entry) nor Linux's mbxfb writes them: they run at their reset values.
The graphics plane (everything RetroPane draws) has only the 17-point table
(`GFXGAM`) and a palette for indexed pixels. So:
- tone and white balance: the 17-point table (free);
- saturation and colour accuracy: **in content**, once, when a picture loads,
  a texture uploads or a theme is built (the 3D tables of step 5);
- later, for video playback: the CSC coefficients could add saturation and
  correct colour to video for free (field layout unknown; a device test
  before use, recorded as open).

### C6 step 2: the measuring tools (built 2026-10-17)

- **One image, any patches.** `rp-screen-measure` now holds an 8 KiB
  *program room*: the PC writes a measurement program into a copy of the
  image before sending it (up to 954 steps, each a full-screen colour, one of
  up to 8 tables of 17 points or the straight one, a backlight from off to
  full or as found, a time), so profiling, every refinement pass and the
  final verification are the same image with different data, never a
  rebuild. The device checks the whole program before showing anything
  (sizes, an FNV-1a checksum, table numbers, backlight at most 432, step
  times, the run at most 90 minutes, each table word's spare byte); a
  damaged one is refused and logged. An empty room runs the old 31-patch
  schedule. 28.2 KB (test-image budget raised to 30 KiB).
- **Faster readings.** Argyll's `spotread` reads keys from the console
  itself, so piped keys never reach it; it now runs inside a Windows
  pseudo-console (ConPTY, hosted by `xtask/src/conpty.ps1`), kept open for
  the whole run: one instrument start-up, then a key press per reading
  instead of a 10 s start-up each time. If the pseudo-console can't start,
  the run falls back to one `spotread` per reading with four times the
  windows. Proven with a console program reading its keys directly;
  spotread itself on the first run.
- **`cargo xtask measure --profile`**: Argyll's `targen` spread (150
  patches, dark region emphasised: this panel crushes its shadows), plus
  greys every 8 levels and each channel alone every 16 (the table's points),
  226 patches, with white read every 40 for the backlight's drift, at full
  backlight; about 26 minutes. Each patch's reading window follows its
  expected brightness (the Spyder4 integrates longer in the dark); any
  patch that missed its window is shown again right after with three times
  the window: the image returns to the loader by itself, so no hands are
  needed. Results: readings `.csv` and an Argyll `.ti3` (drift-corrected,
  white = 100, white's cd/m2 recorded) in `local/screen-measure/c6/`.
  `--ti1`, `--table`, `--light`, `--dry-run` for other passes.
- **`cargo xtask calibrate <ti3>`**: the panel modelled as black plus each
  channel's own light above black at the 17 table levels (its error on the
  mixed patches reported), then a table for each white candidate (native,
  9,000 K, 7,500 K, D65): the brightest white of that colour the panel
  reaches, greys from the panel's black to it neutral, the tone curve
  BT.1886 with the measured black. Point 16 stands for input 256, so it is
  aimed for input 255 to land on white. **`--verify native,9000,6504`**
  shows each table, reads 33 greys (every 8: the table's points and the
  levels between them, which test how the 2700G interpolates), corrects
  each point from its error, and reads again (2 passes by default): all
  hands-free in one sitting. Delta E 2000 checked against Sharma's
  reference pairs.
- **Known limit:** no dither on the 2700G's output was found, so table
  values land on the panel's 64 levels a channel; a strongly turned-down
  channel (blue, for D65) keeps fewer levels, which can show as bands in
  smooth blue gradients. The looks test shows ramps so this is judged on
  the screen, against the other whites.
- **Fallback if the pseudo-console fails on the first run:** Argyll's own
  tools can drive a display through a small web server (`-dweb:<port>`;
  checked on the PC 2026-10-17: `GET /ajax/messages` waits, then answers
  each new patch as `#RRGGBB`). A bridge from that to the Axim would let
  `dispread` and `dispcal` run the whole measurement with the instrument
  kept open, but it needs a live link to the running image (its own USB
  serial port, with the re-enumeration risks of docs/hw/udc.md), which the
  program room avoids. Kept in reserve, not built.
- **Not built yet: the VCOM sweep** (step 1's measured sweep). The program
  room has no field for PWM1; adding one (bounded to 290-370 of 1000, Dell's
  330 put back) is a small change, made only after the profile and
  calibration runs, with the owner watching for flicker.

**Reviewed and kept (2026-10-17):** the C6 tools above were built by a
parallel session, stopped by the owner, and committed whole (`e4dde85`);
this session reviewed them: `xtask ci` clean, `emu-screen-measure` passes,
`measure --profile --dry-run` plans 226 patches + 7 drift whites, about 26
minutes, program 1,884 of 8,192 bytes. The pseudo-console has not yet run
`spotread` with the Spyder (the instrument wasn't plugged in): the first
profiling run is its test.

### C6 profiling runs (2026-10-17, owner's Spyder under a thick cover)

- **Run 1:** the pseudo-console worked first time: **about 2.4 s a reading**
  (10 s before), 5 s for black. 205 of 233 good, then `rp-screen-measure`
  **froze 22 minutes in**: `OSCR0` (32 bits at 3.25 MHz) wraps every 1,321.5 s,
  its next patch was due just past the wrap, and "milliseconds since the
  start" fell back to 0 and never got there (the PC read the frozen dark red
  patch 28 more times: "drift 970%"). Home still worked (the wait loop checks
  it): everything put back, loader back, no reset. **Fixed as a class**
  (`42fff26`): `rt::Elapsed` and `rt::uptime_ms` count through the wrap; the
  audit found the same fault in the loader's `now_ms` (`ticks() / 3250`,
  falling to 0 every 22 minutes under the PC-write guard, the media-change
  timer and `reboot wm`: now `uptime_ms`, +24 B, card loader to reinstall) and
  in the I2S stream totals; host tests through one, three and ten wraps.
  Run 1's white also read 61.2 cd/m2 against about 57 in every other run:
  the sensor sat differently.
- **Run 2:** stopped by the PC after 17 readings for a sensor check (the
  owner's cat); seated properly.
- **Run 3 (`local/screen-measure/c6/profile-1792284643.*`), the profile:** all
  233 read in about 26 minutes, none needing a second pass, the patches past
  22 minutes changing as they should. White fell steadily 57.8 to 55.3 cd/m2
  over the run (4.6%: the LEDs dimming as they warm at full duty; corrected
  from the drift whites). Panel: **white 56.0 cd/m2 at about 13,600 K, black
  2.52, 22:1.** Argyll profiles from the `.ti3`: the **table-based (LUT)
  profile fits every reading to 0.18 Delta E 2000 on average, 0.62 at
  worst** (`axim-lut.icm`: the source for the 3D colour tables of step 5); a
  shaper/matrix one only to 1.45 average, 9.5 worst (this panel's channels
  don't add up simply). `calibrate`'s own additive model: 1.51 average, 4.17
  worst on the mixed patches (it is used for greys, which are then measured).
  Whites and what they cost: native 56.0 cd/m2 (100%), 9,000 K 45.9 (82%),
  7,500 K 44.5 (79%), **D65 43.6 (78%)**.
- **Verify run 1 (`calibrate --verify native,9000,6504`, about 20 minutes
  hands-free, `profile-1792284643-verify.md`):** each white's table shown,
  33 greys read, corrected, read again. Greys' error from the BT.1886 target
  at that white, mean (worst) Delta E 2000:
  | White | Pass 1 (model's table) | Pass 2 (once corrected) |
  |---|---|---|
  | native (about 13,600 K) | 2.91 (4.55) | **1.54** (3.06) |
  | 9,000 K | 3.62 (6.10) | **1.34** (3.38) |
  | D65 | 3.88 (6.80) | **1.57** (4.18) |
  One correction brings every white to about 1.5 on average (under 2 counts
  as a good professional calibration); the worst left are single greys and
  white itself. The **black end stays very blue** (about 28,000 K at 0, still
  15,000 K at 24): that is the panel's own glow, which no table can take blue
  out of, since the table can't go below zero. **The panel kept dimming**:
  white 55.2 cd/m2 at the start of the check, 53.3 at its end (after about an
  hour at full backlight in all, about 8% below run 3's start); the check
  scales targets to it. Third-pass refined tables written, not yet measured.
  Found while building the looks test: two of those tables (and two refined
  ones) had a point lower than the one before (native blue E0 then DE at
  white, D65 blue 6 then 0 at black): a falling table shows a brighter
  input darker. `calibrate` now keeps every table rising, bringing the
  earlier point down (never adding light: at the black end that keeps the
  blue blacks from getting bluer); guard test `refined_tables_never_fall`.
- **Looks test, `rp-looks` (built 2026-10-17; `cargo xtask load rp-looks`,
  owner watching):** four full-screen pages drawn once in 32 bpp: the
  owner's **Pier** and **Milky Way** photos (from `local/wallpapers`, never
  committed; LZ4-packed at build, 612 KB for both, under the loader's
  928 KiB for a loaded image), a **colour chart** (the 24-patch colour
  checker at its usual sRGB values: skin, sky, foliage, flowers, saturated
  colours, greys; the six full primaries and secondaries; a 16-step grey
  wedge) and **ramps** (grey, R, G, B, C, M, Y, and grey 0-63 stretched:
  banding and shadow detail). Five looks: **Original** (as found),
  **Original, full light**, **Native white**, **9000 K**, **D65** (the
  verified tables, at full backlight). D-pad left/right look, up/down
  page, centre flips to the original, Home ends; the caption shows for 3 s
  after a change; the log keeps every change and the seconds each look was
  on screen. `cargo xtask emu-looks` checks every page pixel-exact below
  the caption through the original, native and D65 tables, with a mutant.
  The tables are 1D (tone and white): saturation and colour accuracy come
  with step 5's 3D tables, after this pick.
- **Looks run 1 (2026-10-17, the owner's family, off the dock):** the owner
  saw **banding** on the photos, the same in Original as in the calibrated
  looks: the test's fault. It drew 8-bit pictures with no dither, and the
  18-bit panel keeps 6 bits a channel (the lesson ADR-0023 already
  recorded). Picks: the owner's son **Original, full light**, his wife
  **Original**, the owner in between; the calibrated whites "just looked
  yellow" (eyes used to the panel's 13,600 K white). The owner asked for
  **darker darks at full light** without the washed-out look, and whether a
  contrast pin was missed: no (the panel's 51 pins have none; PWM1 is VCOM,
  whose sweep may lower black a little).
- **Version 2 (built 2026-10-17):** every look's table is **applied in the
  picture at full precision, then ordered-dithered** (4 x 4) to the panel's
  64 levels a channel, with the hardware table straight (`shown`: each
  stored byte is its level x 4 + 2, so the straight table's squeezed top
  segment can't move it); a change redraws into the other screen, then
  shows it. Two looks added after full light: **deeper darks** (input to
  the power 1.35) and **more contrast** (S-curve), both through the table
  as found at full backlight: white stays full, darks and mid-tones come
  down. emu-looks pixel-exact through Original, deeper darks and D65, with
  a mutant. **Design consequence for the OS:** on this 6-bit panel a
  calibration or night curve belongs before the dither (in content, or in
  the renderer's dither step), not in the hardware table after it.
- **Looks run 2 (2026-10-17, v2, the owner): "Full light, more contrast
  looks the best by far."** (The S-curve through Windows Mobile's table at
  full backlight: the panel's native blue white, darks and mid-tones down,
  light tones up.) Run 1's log: Original 94 s and Original full light 96 s
  on screen, the calibrated whites 15-31 s each, 213 changes.

**Decided (owner, 2026-10-17, after looks run 2; replaces "Original is the
default" above): RetroPane's default look is "Full light, more contrast"**:
the backlight at full, the panel's own white, and an S-curve through Windows
Mobile's table, applied before the dither (in the picture or the renderer's
dither step, never in the hardware table after it). **Original stays a
choice in Settings > Display**; Everyday and Dark games stay offered there
until the strength test settles the curve. Costs to watch: battery at full
backlight (unmeasured), the LEDs' heat and wear (white dims about 8% over an
hour at full), and shadow detail in dark scenes (the strength test checks
it). The brightness slider still works: the curve is kept at any backlight.

- **Strength test (looks v3, built 2026-10-17):** `rp-looks` with five
  looks: Original, Full light (no curve), and the S-curve at **mild**
  (power 1.3), **current** (1.6, the pick) and **strong** (2.0), all at full
  backlight; same pages (both photos, chart, ramps with the stretched
  0-63 shadow band).
- **Strength run (looks v3, 2026-10-17, the owner): "Contrast strong by
  far"** (S-curve power 2.0). The strongest tested won, so the best may lie
  further: the next test spans 2.0 and up, with the shadow and highlight
  detail checked on the ramps and the Milky Way.

## Measured with a colorimeter (2026-10-17)

The owner has a Datacolor Spyder4, which takes the eye out of the readings
(C1's dials showed how much the eye, the viewing angle and the test's own
design can mislead). Tools on the PC: **ArgyllCMS 3.5.0** (open source,
AGPL; run as a separate program, nothing linked or shipped), the Spyder4's
factory calibration read by Argyll's `oeminst` from the `dccmtr.dll` of
Wacom's colour software, and Argyll's white-LED spectral correction (the
X51v's backlight is a white LED string). The Spyder runs on Argyll's USB
driver (switched in Device Manager; switch back for Wacom's software).

- **`rp-screen-measure`** (test image, 17.3 KiB): 31 full-screen patches on
  a fixed schedule, gamma table straight (output = input) so the readings
  are the panel's own: stray light (backlight off), 14 greys, red/green/blue
  at 128 and 255, black at full backlight, white at 1/8 to 8/8 backlight,
  white again for drift. About 8 minutes. Home stops it; everything is put
  back. The schedule lives in its library and is shared with the PC side.
- **`cargo xtask measure`**: starts the image and takes the loader's "started"
  reply as time zero, then runs one `spotread -O` reading per patch (about
  10 s each, measured: piped keystrokes don't reach Argyll's console input on
  Windows) at the scheduled moment; a reading that runs past its patch is
  dropped, never shifted. It uses only the USB instrument Argyll lists, never
  a serial port (the Axim is COM4). Report under `local/screen-measure/`:
  stray light subtracted; white, black, contrast; white point and colour
  temperature; primaries; grey curve and panel gamma; backlight steps; drift;
  and a **correcting gamma table** (gamma 2.2 between the panel's own black
  and white, white balanced to D65 by turning channels down).
- `cargo xtask emu-screen-measure` runs the whole schedule in the emulator:
  each patch within 300 ms of plan, grey 128 exact on screen.
- Method: dark room or a cloth over the sensor and the Axim; sensor flat and
  centred, not pressed (pressing distorts an LCD); straight on, so the
  viewing-angle shift remains.

**Measurement run 1 (2026-10-17):** the backlight-off and black readings ran
past their 16 s patches (the Spyder4 integrates for a long time near black;
dark patches are now 45 s, about 12 minutes in all). **Run 2** (cloth over the
Axim and sensor; owner will wrap it better next time), report
`local/screen-measure/report-1792274616.md`:
| | |
|---|---|
| Stray light, backlight off | 0.44 cd/m2 (room light leaking in, or the sensor's floor; subtracted) |
| White at Windows Mobile's 63% | 40.8 cd/m2; full backlight 56.9 (the datasheet's new panel: 165) |
| Black | 1.43 cd/m2 (2.11 at full backlight: it follows the backlight, so it is the panel's own glow, not room light) |
| Contrast | 29:1 (datasheet 200:1) |
| White point | x 0.269 y 0.264, about 13,500 K: very blue (aged white LEDs lose their yellow phosphor), **to verify** |
| Primaries | red 0.488/0.304, green 0.294/0.423, blue 0.151/0.133: a small gamut, as the datasheet's NTSC 35% |
| Panel gamma | **1.79** over the mid-tones (eye test said about 1.5); the top flattens (gamma 1.05 at 240): the white end clips, as the owner saw |
| Backlight | white 15.5 cd/m2 at 1/8 duty up to 56.9 at full: straight with a large offset |
| Drift | -0.7% over the run; run 1 and run 2 agree within a few percent |
The correcting table for gamma 2.2 and D65 cuts blue to 0.30 and red to
0.77, which would dim white by about a quarter: worth it only if the
13,500 K is real. Checks before building it in: (1) the same Spyder and
correction on a display of known white (the owner's calibrated Wacom
display), (2) a repeat on the Axim, wrapped better, compared with run 2.

**Sensor check on the owner's phone (2026-10-17; the Wacom display is too old
to be a reference):** the phone's white (white page, half brightness) read
7,290 and 7,180 K with the white-LED correction, 6,780 K with none, 6,210 K
with the CCFL one, 5,570 K with the RGB-LED one; brightness the same
(29.6-30.1 cd/m2) with all of them. A phone's white sits near 6,500 K, so the
sensor reads colour about right, perhaps a few hundred kelvin cool with the
white-LED correction. **The Axim's 13,500 K is real: its white is very blue**
(aged white LEDs), not a sensor error.

**Run 3 (2026-10-17, sensor centred, the Axim and sensor wrapped in a thick
blanket):** every patch within 1.4% of run 2, most within 0.5%; white
40.7 cd/m2, black 1.44, 28:1, about 13,240 K. **The measurement repeats.**
The backlight-off reading (0.448, run 2: 0.442) didn't change with the
better wrap, so it is not room light: the sensor's floor or the panel's own
faint glow with the backlight off. **Settled:** face down on something
opaque the sensor reads 0.02 cd/m2, so the 0.45 is the screen's own glow
with the backlight off: part of what the eye sees, no longer subtracted
(`cargo xtask measure --report <csv>` re-analyses saved readings). Run 3
then: black 1.89 cd/m2, white 41.2, **contrast 22:1**, 13,300 K, gamma 1.80.

**Three measured tables** (gamma 2.2 between the panel's own black and
white): **full** (D65: red 0.78, blue 0.32; white keeps 79% of its
brightness), **half-way** (about 9,000 K: red 0.81, blue 0.63; 87%), **gamma
only** (white as it is; 100%). Added to rp-display-cal's last step as the
looks "Measured full", "Measured half" and "Measured gamma", at full
backlight; emu-display-cal shows the first pixel-exact. The owner picks by
eye; then a measurement with the pick on confirms it hits its targets.

**Looks run (2026-10-17, the owner's family, 321 presses, every look seen
many times):** the owner's son picked **Original**, the owner leans the same
way; the owner's wife picked **Everyday or Dark games** ("you can see things
better"). Nobody picked a Measured look: made accurate, this screen looks
dimmer and duller than people are used to. Proposed: Original stays the
system default; games get the Dark games curve by default (the per-game
curve of apps-and-packs §6a); Settings > Display offers the looks (Original,
Everyday, Dark games, Accurate) for each owner to choose.

**Decided (owner, 2026-10-17): a choice of three picture looks, Original,
Everyday and Dark games**, offered at the first start (a one-tap pick, each
shown on the real screen) and in Settings > Display any time after. Original
is the default. The measured (Accurate) tables stay in the calibration tool
for owners who want them.

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
