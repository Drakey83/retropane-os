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
faults.** Proposed: RetroPane sets `0xFC5` when it takes the display (only
from Windows Mobile's `0xEC5`), so every screen runs at 60 Hz; `X51V_VGA`'s
pixel clock becomes 25.1875 MHz.

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
