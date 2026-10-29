# Overnight log, 2026-10-28 to 29

Reid's overnight order: 13 items in order, device tests only at Windows
Mobile's clock, 74.75 / 1 and divide 2; no divide-1 work. Each item: a
plain note first, then the detail. The morning report is at the end.

## 1. R89: exact colours for the smooth UYVY route

**In plain English:** the GPU's colour conversion now stores the numbers
the video plane's converter expects, and on the X51v every word of the
test card came out exactly as designed (38,400 of 38,400). With the
converter set to our own coefficients, the colours should come back exact
to the panel's precision, but the screen itself can't be read back: that
is your eye check in the morning. Until then, smooth 2x is **eye-right
only, not exact**.

**Detail:**

- **Design (rule 0):** the textbook (and GC2CS's CPU way) stores BT.601
  YUV, offsets included, and lets the converter's standard matrix invert
  it; our GPU passes only multiply, so they can't add +16 / +128. Ours:
  the offsets come from a flat quad drawn first and the pass added to it
  one-to-one (the blend, measured exact), and each value is halved so it
  never overflows: V = 128 + floor(128 R / 255), U the same of B, Y = 16 +
  floor(128 G / 255) (green alone carries Y: the pixel pair's odd pixel
  then keeps its own green, where a luma Y would mix in its neighbour's red
  and blue). The converter: `CSC02` = 512 / 512 (green from Y, red from
  V), `CSC04` = 512 (blue from U), the rest 0. Every division in Intel's
  formula is then exact whatever its rounding; host-checked: every 5-bit
  red and blue and every 6-bit green comes back exact (8-bit: the last bit
  lost, which the panel doesn't show).
- **GC2CW (`local/rp-gpubench-runGC2CW.txt`, WM clock, 64 ms):** the
  arithmetic measured on a 0-255 ramp, every value. DOT3 at green tint 32
  = `floor(128 v / 255)` exactly (0 of 256 off); at tint 64 = v exactly;
  the plain textured word gives v whatever the tint (it is not a multiply);
  the additive blend over a first quad exact (offset + value, clamped at
  255), alpha byte included.
- **GC2CX (`local/rp-gpubench-runGC2CX.txt`, then `...CX2.txt`, WM
  clock):** GC2CM's conversion with the offsets. First run: Y0 and Y1
  exact in every word, U and V not halved (the plain word ignores the
  tint). Pass A then moved to Dell's DOT3 word without TEX0 bit 13 (GC2CJ's
  per-channel 4x multiply) at red and blue tint 32. Second run: **the
  RGB565 card exact in all 38,400 words, every byte**. The ARGB8888 card:
  Y exact (61 of 38,400 one step low in Y0), U and V low by 1-3 steps: the
  per-channel multiply reads an 8888 texel at 5 bits (14 read as 8). The
  game frames are RGB565, so the route is exact; an 8888 source would need
  DOT3 passes for U and V too.
- **Cost:** 17.46 ms a frame serial at WM clock (passes 4.39-4.46 ms, the
  two 2D placements 2.11 ms): the same as GC2CM's 16.45-17.4 (the offsets
  quad adds nothing measurable).
- **For the morning:** `target/variants/gc2cx-look/rp-gpubench.bin` (from
  this commit): converts the card, then shows **photo 1: the GPU's exact
  UYVY with our coefficients** and **photo 2: GC2CS's CPU-made UYVY of the
  same card with Intel's standard coefficients**, each 2x on the video
  plane until a button. The look part reuses GC2CS's display calls; it
  was not run in the emulator (it waits for buttons).
- **Not done:** the route in the game path's driver (R90's build, after
  the ADR vote).

## 2. R90: the game path ADR, drafted

**In plain English:** a draft decision for your vote, nothing built:
[ADR-0038](../adr/0038-game-render-path.md). All three routes kept; the
driver picks a route each frame from how long recent frames took (drops a
level when slow, climbs back only after a long run of fast frames, so it
never flickers between them), switches only in the screen's blanking, a
per-game override, two frames in flight for the reduced routes, and each
route's own clock step (values held for the divide-1 work). Six questions
(G1-G6), each with a recommendation.

**Detail:** the measured table is GC2CP's test frames, marked as such (not
game performance); the 240 x 320 route's on-screen colour waits on R89's
eye check; G4 (two in flight) waits on item 8's stray-word check.

## 3. R80 at Windows Mobile's clock and 74.75 / 1

**In plain English:** the rare one-pixel errors on the smaller game
frames are real at 74.75 / 1 (Dell's own divide-1 speed) and absent at
Windows Mobile's clock: up to 7% of renders on the flat frames have one
wrong pixel, the textured frame none. And the time a frame spends outside
the GPU's drawing is now all the CPU preparing it (about 1.9 ms); R3's
faster bus timing (7/3/1) doesn't shorten it at all. The small-frame path
isn't shipped on these numbers (and needs this fault answered first).

**Detail** (`local/rp-gpubench-runGC2CY.txt`, run GC2CY, 185 s, stack 60%):

- **Part A: 250 renders of each game frame at 360 x 480 and 240 x 320,
  shown scaled as GC2CA showed them, against WM-clock references.** WM
  clock: **0 of 2,000 wrong**. 74.75 / 1:

  | Frame | 360 x 480 | 240 x 320 |
  |---|---|---|
  | flat | 17 of 250 | 6 of 250 |
  | over a layer | 15 of 250 | 6 of 250 |
  | textured | 0 of 250 | 0 of 250 |
  | depth tested | 7 of 250 | 3 of 250 |

  Every wrong render logged was **one pixel, all eight neighbours right**,
  the wrong value another colour of the scene (e.g. `0x4F4A` for
  `0x679F`), at every x mod 8 (0-7, not only 5 as GC2CA's two cases
  suggested), anywhere in the frame. The renderer now draws quads (the
  default since 55e6e220). Divide-1 analysis held for later (the owner's
  order); the 240 x 320 fault at divide-1 speeds is out of scope.
- **Part B: the time outside the core's render, native, the region array
  written whole each frame (4,800 words):** CPU set-up 1,880-1,900 us
  fastest at both clocks; **the poll sees the end 0-1 us after the
  render** (the learned pace, a256a2a4: the schedule costs nothing now);
  so the time outside the core is the CPU's set-up alone. **CS3 at R3's
  7 / 3 / 1 against WM's 8 / 4 / 2: set-up 1,892 against 1,892 us (WM
  clock), 1,900 against 1,900 (74.75 / 1): no change**; core and whole
  within 1-2% either way (noise). With the region array kept (22 words,
  GC2CP run 4) the set-up was 1,860 us: 4,800 posted writes cost ~30 us,
  so the bus isn't the limit for these writes; R3's timing would matter
  for reads (the 8 MB/s read-back), not this.
- **A finding for item 10:** GC2CA's own emulator check now peaks at 89%
  of its stack (it was under 80% before today's renderer growth); GC2CY
  runs standalone with its renderer in `.bss`.

## 4. R20: the tearing scheduler, built and tested

**In plain English:** the scheduler that decides, each frame, when to
start drawing so the picture never tears is built and works on the X51v:
120 of 120 frames it drew straight into the shown picture were tear-free,
at Windows Mobile's clock and 74.75 / 1, including right after sudden
slow frames. It falls back to the safe two-picture way (A) for the first
frame and right after each spike. 120 clean frames isn't proof of zero
tears (it bounds the rate under about 2.5%). The games path stays held
until you judge GC2CA's picture.

**Detail:**

- **Built:** `rp_mbx3d_render::scan` (pure, host-tested): the start line
  from a band of recent render times, the fastest x 4/5 to never overtake
  the scan, twice the last frame (never under the slowest x 11/10) to
  finish before the scan comes round, a 16-line margin each side, the
  earliest safe line taken; A for no history, a band that doesn't fit,
  frames past ~35 ms, or a non-top-down order (the checkerboard). Seven
  host tests against a tile-row model of the scan (stricter than the
  device by a tile row: it counts a tile row the scan passes through while
  it lands as torn); every plan checked tear-free across its whole band.
- **GC2CZ on the X51v** (`local/rp-gpubench-runGC2CZ2.txt`; GC2BZ's
  watched renders, 40 frames a setting with textured spikes at about 2x):

  | Clock | Order | Behind the scan | Tear-free | By A | Start line |
  |---|---|---|---|---|---|
  | WM 99.67 / 2 | interleaved | 30 | 30 | 10 | 290 |
  | WM 99.67 / 2 | rows | 30 | 30 | 10 | 290 |
  | 74.75 / 1 | interleaved | 30 | 30 | 10 | 412 |
  | 74.75 / 1 | rows | 30 | 30 | 10 | 412 |

  Every picture right. The first run (`...GC2CZ.txt`) doubled the slowest
  recent frame: 20 of 20 tear-free but A for 35 of 40 (a spike kept A for
  eight frames); the band was changed to twice the last frame.
- **Not done:** the scheduler wired into the graphics service's shell path
  (it is the shell's and apps' design, ADR-0034 P1 = C; the service's
  frame loop isn't built yet); the games path held (R20).

## 5. R71: the CS3 bus check, built and run

**In plain English:** the part of the first-docking self-test that
decides each Axim's bus speed is built and works: it writes known
patterns to the graphics chip and reads every word back, over 1.5 million
transfers a check, checks again after five minutes of heavy traffic, and
moves a unit to the faster setting only if every check is clean. On your
X51v it passed everything: this unit would run 7/2/0. Every unit still
starts at 7/3/1, and one that fails stays there. The divide-1 parts of
the self-test are not touched.

**Detail:**

- **Built:** `rp_hal_pxa27x::buscheck` (pure, host-tested: a fault of 1
  in 1,000 transfers caught and named; a check under 100,000 transfers
  never counts as a pass; 7/2/0 only with the default, cold and warm all
  clean). Six patterns (address and its complement, alternating bits,
  walking ones and zeros, hashed noise) over 512 KiB: 1,572,864 transfers
  a check. `rp-cs3bus` gained the self-test mode (`RP_CS3BUS_SELFTEST`;
  the grid's patterns now come from the shared module); `emu-cs3bus`
  checks both modes.
- **On the X51v** (`local/rp-cs3bus-selftest.txt`): MSC1 found as Windows
  Mobile's (`0x24842481`); 7/3/1 (`0x13742481`) 0 misses in 1,572,864;
  7/2/0 (`0x02742481`) cold 0 misses; 300 s of traffic at 7/2/0 (1,674
  rounds of a 1.2 MB flush and a 2D fill, every word checked) 0 misses;
  7/2/0 warm 0 misses; **pick 7/2/0**; MSC1 put back. This is one unit and
  one run: it shows the check works and this unit earns 7/2/0, not that
  every unit will.
- **Not done (held, divide-1):** the self-test's ladder, sentinel scenes
  and corner sweeps (R71's other parts).

## 7. R79: the fitted enlarging filter

**In plain English:** I fitted the video plane's 2x enlarging filter to
the game frames by least squares and checked it on frames it never saw.
It is a small, honest gain over today's filter (mbxfb's): +0.3 dB on
flat-shaded frames, +0.9 dB on textured ones. Plain bilinear does as well
on flat frames, and on these magnified point-sampled textures Dell's
blocky doubling is actually closest to the native picture (the native
picture is blocky there too). Nothing is shipped: a look image shows all
three for your eye in the morning.

**Detail:**

- **Rule 0:** mbxfb's set is a textbook windowed sinc; Dell doubles
  pixels; ours fits the 12 values 2x uses (8 across at 0.25-3.75 pixels, 4
  down at 0.25-1.75) to make each 240 x 320 frame, enlarged, match the
  native 480 x 640 render of the same frame (RAISR's idea cut to this
  scaler). Data: 48 frame pairs from the host model of the 3D core
  (`rp-gpubench`'s `r79_frames`: 24 warped meshes, flat and textured);
  seeds 0-15 train, 16-23 held out. Fitter: `tools/scaler-filter/fit2x.py`
  (alternating constrained least squares, rounded to the scaler's signed
  bytes, each phase summing to 64); result kept in
  `tools/scaler-filter/fit2x-2026-10-29.json`.
- **Held-out PSNR against native (dB):**

  | Way | Flat | Textured |
  |---|---|---|
  | Dell's doubling | 29.39 | **53.87** |
  | bilinear | **31.00** | 43.25 |
  | mbxfb (today's 2x) | 30.58 | 44.00 |
  | fitted, signed bytes | 30.92 | 44.89 |

  The fit came out near-bilinear (across 50 / 13, down 52 / 11, no
  ringing lobes), the train and held-out scores within 0.2 dB (not
  over-fitted). Caveats: frames from the host model (its flat and 1:1
  textured pixels match the device; GC2), the scaler's 2x phase alignment
  assumed centred (the filled-in phases make the set right whichever it
  is), PSNR not your eye.
- **Built for the morning:** `Filter::from_2x` (host-tested: every phase
  sums to 64, the fitted phases hold the fit) and the look image
  `target/variants/gc2da/rp-gpubench.bin`: the flat and the textured frame
  at 240 x 320, each shown three ways (mbxfb, fitted, Dell's doubling),
  each until a button (6 photos). Not shipped until you judge.

## 6. R77: G9's log, fixed and run again

**In plain English:** G9 failed because its register dump is bigger than
the loader's run log. Its emulator check now reads the run's card log
(which holds everything, as on the Axim), and it passes. On the Axim tonight
G9 ran whole again: both dumps complete on the card. One stale value in
the check was also wrong (it expected a clock setting Windows Mobile never
uses); it now expects the measured one. Nothing was loosened.

**Detail:**

- **The class fix:** `boot_loader_glesfeat_g9` gives the emulator boot an
  SD card image and checks the run's card log (256 KiB a run), as
  `emu-gpubench` does; every original check kept (1,024 MBX, 56 SYS and
  every PXA line a dump, both "ends", no frozen register read).
- **A stale constant found by it:** the check wanted `MBXCLK 0x6` read
  back after the run; the X51v reads `0x5` (Windows Mobile's own, run 26's
  hand-off, and G9's device run of 2026-10-23), so the check never could
  pass. Now `0x5`. The same `0x6` sits in the other G tests' check
  (emu.rs, the controls' read-back); they pass on the emulators as they
  are, so it is left for your word (it may hide the same staleness).
- **Emulator:** `emu-glesfeat` G9 OK on the WM-clocks build ("both dumps
  whole, back at the loader"); on the pinned release it fails only that
  clock check (the release starts the clocks at 0).
- **On the X51v** (`local/rp-glesfeat-G9-2.txt`, card log glesfeat-0028
  read back to `local/rp-glesfeat-G9-2-card.txt`): read back `COREPLL
  0xbb1 MBXCLK 0x5`, both dumps whole (2,296 lines), done, returned. G9's
  own measurement: 797 words off its reference at 74.75 / 1 (the
  Gouraud-at-divide-1 behaviour G9 dumps for; recorded, not analysed).

## 8. GC2CP's pipelined ways at Windows Mobile's clock and 74.75

**In plain English:** the faster ways of drawing a game frame overlap the
graphics chip's work with the processor's. I split each frame's time into
the processor's set-up and the waiting on the chip: set-up is under 1 ms,
waiting is 4-13 ms, so the chip sets the pace. The careful overlapped way
(d') and the two-frames-at-once way (e) came out with **zero stray words**
at both clocks. The way that also runs the 2D engine during the 3D render
(d) is clean at Windows Mobile's clock but **leaves 16-38 wrong words a
frame at 74.75**. Two frames at once is 12-19% faster than one at a time,
with its picture checked only on each test's last frame.

**Detail** (`local/rp-gpubench-runGC2CP-low-2.txt`; 2700g.md "GC2CP's
pipelined ways at Windows Mobile's clock and 74.75 / 1"):

- New: `RP_GPUBENCH_GC2_HALF_ROUTES_LOW` (WM clock and 74.75 / 1, no step
  above), the set-up / wait split in (d), (d') and (e), and `emu-gpubench`
  "run GC2CP low".
- **A false alarm first:** the first run showed 16 U'/V' words on (d') and
  88 on (e), on the textured frame only, at both clocks. GC2CP drew its
  references as triangles and its ways as quads (today's default); the
  textured mesh's edge ties differ by form (GC2CV). References now use the
  default form; rerun: 0.
- **Stray words:** (d') 0 at both clocks; (e) 0 on the last picture at
  both; **(d) 0 at WM clock, 16-38 a frame at 74.75 / 1.** Not fixed
  tonight: (d') is the fix (the 2D finished before the render), and the
  overlap fault belongs to R30. The two-in-flight win is claimed for (d')
  and (e) only.
- **Times** (fastest, flat / layered / textured / depth): (e) 6.4-10.2 ms
  at WM, 4.2-8.8 ms at 74.75 / 1, against (b) serial 7.3-11.2 and
  5.2-9.7; native 14.3-21.3 and 10.2-17.8. (d') 14.1-16.2 and 12.4-15.0:
  below the core's clock-up it barely beats native, and is slower than
  native on flat frames at 74.75 / 1.
- The stack: 41,048 of 57,344 bytes (72%).

## 9. The emulator's release, and what it does not do

**In plain English:** the newer emulator build (with today's quad objects
and Windows Mobile's clocks) is **not released**: one of its checks still
hangs, and publishing it would mean a push tonight's order doesn't allow.
Nothing relies on it as a release. Every check tonight that needed it
was also run on the Axim. The emulator page now says plainly what the
emulator can't show you: the scaled picture, row-order textures, divide-1
errors, the screen's memory reads slowing the 3D core, 2D/3D overlap
faults, and bus timing.

**Detail:**

- The pinned release stays rp-v0.31.0 (`xtask/src/emu.rs`
  `QEMU_RELEASE`); CI uses it. The local build is
  `rp-v0.31.0-2-g7fad420638` (branch `rp-next-032`); `emu-mbx3d` still
  never returns on it (R86, audit C41; the lead, unproven, is the display
  registers starting at 0 while rp-mbx3d's clock path waits on the
  display).
- emulator.md "What the emulator does not do (2026-10-29)", each point
  read from the model's code: the video plane shown 1:1 (the scaler's
  words stored only); textures always twiddled; renders exact at any
  clock; render time from the render's own work, never the planes' fetch;
  no 2D/3D interference; `MSC1` stored only.

## 10. The stack after tonight's renderer changes

**In plain English:** the 3D renderer grew today (quads, the bigger list
cache), and one test (GC2CA) now used 89% of its stack in the emulator,
over the 80% safety line. I didn't raise the line: I moved that test's
renderer out of the stack into fixed memory, as the other tests already
do. It's back under 75%, and every one of tonight's test runs passes its
stack check. The memory budget didn't change.

**Detail:**

- Found: `emu-gpubench` "run GC2CA" failed, "the stack went 50808 of
  57344 bytes deep". The class: a renderer held in a run function's frame.
  GC2CP (R94), GC2CY and GC2DA already hold theirs in `rt::Slot`s; GC2CA
  was the last.
- Fix: `GC2CA_HELD` (`rt::Slot<Option<(usize, Renderer)>>`). An earlier
  try today (moving GC2CA's renderer the same way, inside a bigger change)
  hung in the emulator; this one, alone, doesn't: the hang's cause was
  never found and didn't come back.
- Checked: GC2CA (under 75%: the image prints nothing below it), GC2CP,
  GC2CP low, GC2CU, GC2CW, GC2CX, GC2CY, GC2CZ all OK on the emulator;
  GC2CP low on the X51v 41,048 of 57,344 B (72%).
- `size.rs`: the decision and its reasons written at rp-gpubench's budget;
  the stack stays 56 KiB, zeroed RAM within 76 KiB (61,160 B for GC2CA's
  variant). No guard raised.

## 11. The display's fetch: P9c's arrangement kept

**In plain English:** the video plane only shows a stable picture when
the graphics plane is also reading the screen (invisible, over it). That
arrangement (P9c's) is the one proven by your eye and the picture
checksums, so everything tonight that shows a scaled picture uses it. Why
the video plane needs it is still **not understood**; nothing tonight
claims it is. No new arrangement was built, so there is no new one to
check.

**Detail:** read from the code: every scaled or converted picture tonight
goes through `gc2ca_show` or sets the planes itself with `GC2CB_GRAPHICS`
(the loader's 480 x 640 screen at 0, fetching, middle slot, global alpha
0) over `GC2CB_VIDEO_BOTTOM` (the video plane, bottom slot, opaque):
GC2CY's frames, GC2CX's look image, GC2DA's three ways. Dell's doubling
uses the graphics plane alone (`Layer::SOLO`), as proven. The lead from
GC2CB stays a lead (the graphics plane's fetch may pace the display's
line timing for both planes); the emulator can't test it (item 9).

## 12. C71: the one-page ruling brief

**The question (yours to answer):** under amendment P's P1 = C (the 3D
core draws straight into the shown screen, inside the first 696 KiB of
2700G memory), what does the second screen become?

**Why it matters:** C71 is the stale pixels in partial 3D frames. The cure
is proven (the shown screen at 0x0: 0 of 100 renders wrong where up to 20
of 20 were, and 4-5% faster), but the memory map can't move the screen
until the second screen has a job. Only one 600 KiB screen fits in the
safe 696 KiB.

| | What it means | For | Against |
|---|---|---|---|
| **R-A: one screen** | screen 1 deleted, its 600 KiB back to the pool | simplest; nothing ever read outside the safe region; +600 KiB for textures | 2D frames draw into the visible picture: they tear unless the tearing scheduler covers 2D too |
| **R-B: keep screen 1 as 2D's back picture** (the design page's recommendation) | 2D draws off screen into screen 1 and flips as today while the 3D core is idle; in 3D frames the picture lands on screen 0 first | 2D keeps today's tear-free flip; amendment P rule 4 already allows it | the graphics service's frame gets one more step (copy 2D's rectangles in, carry the 3D area back) |
| **R-C: amendment P's A** | the 3D core draws off screen, the 2D engine copies | tear-free everywhere | contradicts P1 = C: fails at divide 1 above 74.75 (G21); only if amendment D ends at divide 2, which is held for divide-1 work |

**New since the design page (2026-10-25), for your weighing, not a
ruling:** R20's tearing scheduler now exists for 3D frames (tonight, item
4: 120 of 120 frames begun behind the scan were tear-free on the X51v, at
WM clock and 74.75 / 1). That weakens R-A's "against" for 3D frames.
Covering 2D frames with it is not built or measured, so for 2D, R-A's
tearing risk stands as written.

**Ready, either way:** branch `c71-screen-place` (one commit, 34b709d1,
2026-10-25): screen 0 at 0x0, `SAFE_END` = 0xAE000, build-time checks that
nothing else of the map sits in the safe region, host tests. It merges
into today's main with no conflicts (checked tonight, `git merge-tree`;
not merged). Screen 1 stays at 0x296000 until your ruling.

**After the ruling (not started):** R-A deletes screen 1 and points the 2D
renderer at the one picture; R-B changes the 2D renderer's back picture
and the graphics service's frame. Then the same device check either way:
`rp-today` on the X51v (the shell right, the loader's screen right after
it returns: the loader's picture at 0x60000 sits under screen 0), and
C71's W5 partial-frame scene held to a fresh render every frame (0 words
off, against 1-3 today).

Full page: `docs/design/c71-screen-place.md`; reminder R70.
