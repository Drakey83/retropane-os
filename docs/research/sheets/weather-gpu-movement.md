# Sheet: the Weather app's movement on the GPU path (ADR-0034), 2026-10-30

**What it decides.** How each Weather movement is shown once it leaves the
2D renderer: which of ADR-0034's frame modes it uses, what of that is
already proven on the X51v, and which device tests are still owed. The
owner, 2026-10-30, after device run 2: movement goes on the GPU path
ADR-0034 decided (option D), not more 2D redrawing: "This is the whole
reason I wanted to have GPU figured out before i moved to UI/UX." Later
the same evening: "one more design pass with 3d to get that working", and
the GPU used only while the screen moves (R133). The 2D round-2 sheet
(`weather-drag-speed.md`) is set aside for this one.

**No code and no device run until the owner's go** (the alternatives-sheet
gate). Every piece not proven on the X51v is a device test below, never an
assumption.

## Measured so far (device run 2, `local/rp-today-weather-run2.txt`, 2D renderer after 9e7bad7e)

| Movement | fps | CPU a frame | queue to flip | Why |
|---|---|---|---|---|
| Page entrance (Stagger) | ~20 | 10-24 ms | 3-29 ms | the rows' glass recomposed over the wallpaper every frame |
| Hourly dragged or flung | ~13.5-15 | 37-40 ms | 21-26 ms | the whole page's background rebuilt every frame (~190k px) |
| Radar loop frames | ~6-15 | ~40 ms | 22-29 ms | 200-300k px rebuilt per loop frame |
| Static costs | | | | the map under the page 38 ms, the wallpaper back 161 ms |

The cause in one line: glass rows move **over a wallpaper that stays
still**, so every moving frame recomposes pixels that didn't change. The
fix is to stop recomposing: let the display mix the moving glass over the
still wallpaper as it scans out.

## Rule 0: iterate, don't imitate

- **Windows Mobile's way (a finding):** `ScrollWindowEx` copies what stays
  visible and the app repaints the uncovered strip on an opaque background;
  page changes redraw the whole window. No glass, nothing retained, no
  motion that the CPU doesn't redraw. DirectDraw overlays exist on WM5 but
  only for video.
- **Dell's way (REA, searches #37-#40 in `local/re/rea-search-log.md`):**
  - **No panning.** `VSCADR` (the video plane's address) is written only
    when the plane is switched off (`0x211fe70`) and put back after a
    clock change (`0x2120370`). It is never repointed per frame, and
    there is no panned page. Dell's only per-frame repoint is the
    `GSCADR` flip (#5).
  - **No overlay code elsewhere.** The video decoder, PowerVR, D3D, GLES,
    `ddi` and `gx` modules never touch the plane registers themselves.
  - **3D clock: on while a 3D context exists, not while 3D is drawing.**
    Each unit of the 2700G has a clock on/off pair with use flags:
    `0x211f5d8` turns a unit on, `0x211f7e4` turns it off. Units 0-1 are
    the 2D clock (`MBXCLK | 1`) and units 2-3 the 3D clock (`| 2`).
    - The 3D clock goes **on when a 3D context is created** (`0x2119c2c`,
      after the 16 KiB 3D memory-manager block is set up).
    - It goes **off only when the last context is destroyed**
      (`0x2119b44`), or at `0x21182ac`'s teardown.
    - So a GLES program keeps the 3D core clocked while its screen is
      still. There is no idle timer and no per-frame gating.
- **The modern way:**
  - **Composition:** Android's SurfaceFlinger hands layers to the display
    controller's overlay planes through the Hardware Composer. A scroll
    or a swipe moves a layer, the display mixes the layers as it scans
    out, and the GPU composes only what the planes can't. iOS Core
    Animation keeps layers drawn once and moves their positions.
  - **Power:** both let the GPU idle (devfreq, power-gated islands)
    whenever the display alone can show the frame. That is R133's
    question, answered by design.
- **Ours, in one line:** the page's glass and text drawn once into a
  per-pixel-alpha picture on the **video plane**, over a **still screen 0**
  (the wallpaper and the chrome) on the graphics plane. Every drag, fling,
  rebound, page turn, Rise entrance and radar step becomes a plane
  repoint in the blanking: no engine runs, the GPU's cores can stay
  clocked off, and the CPU only reads the pen and writes one register.
  Only movements that move parts separately use an engine (the 2D engine
  for Stagger and the radar's cells, the 3D core for Expand).
- **Why it beats each:** WM and Dell redraw on every move. The modern way
  needs a hardware composer with several planes; we have two planes and
  one blend stage, and the layout below fits the Weather app into exactly
  those two. It also beats ADR-0034's C4 as voted on the look: the
  wallpaper stays still under moving glass (see the vote, V1).

## What is proven on the X51v, and what is not

| Fact | Status | Evidence |
|---|---|---|
| A `GSCADR` or `VSCADR` repoint takes effect at once; written in the blanking, a pan is smooth | **proven** | W1, W4 (run C); run CH W1: 160 frames, 0 late, 0-4 us |
| Plane switches are clean only when written in lines 643-3 (~160 us) | **proven** | GC2DC run 2 |
| A hidden plane (stream off) still takes part in the slot order | **proven** | GC2DC run 2 |
| A flip at a deadline, `flip_due()` from `DVLNUM` and R111's line period | **proven** | R115: 1,534 flips, all on line 643 |
| 2D changes copied behind the scan into screen 0 | **proven** | R115: 0 torn of 218 refreshes; touch to flush 16.2 ms median |
| The video plane takes RGB formats (RGB565 exactly; ARGB4444 and ARGB8888 by how they read) | **proven** (formats) | run D, five runs, photos |
| A UYVY page panned on `VSCADR` under per-pixel graphics glass; an RGB page panned on `GSCADR` under a video tint at global alpha | **proven** | run C, W5, W6 |
| The video plane's place and size on screen (`VSADR` X/Y, `VSCTRL` size) at 1:1 | **proven** | P9 step 6, the owner's photos |
| ARGB4444 + 4x4 ordered dither looks like ARGB8888 for smooth glass | **proven** (owner's eyes) | `rp-blend`, 2026-10-12 |
| A row of 480 x 48 drawn by the 2D engine from an atlas | **proven** | run CH W2: 616 us, CPU ~0 |
| 3D textured scaling (an app opening) | **proven** (cost) | run CH W3: render 10.1 ms median, 355 ms for 15 frames |
| 3D clock off and on keeps the core's set-up | **proven** | ADR-0026 T9: a clock-on frame ~1.0 ms against ~5.6 |
| R121's alpha-0 overlay: divide-1 copy-path frames right at full speed | **proven** (knob off) | gpubench-0237 to -0241; not used here (divide 2 ships) |
| **The video plane's per-pixel alpha with an ARGB format** (above the graphics plane) | **not measured** | 2700g.md, run D's note |
| **A partial-height video window panned through a tall RGB/ARGB picture** (stride 1920, start moved by rows) | **not measured** | W5 panned a full-screen UYVY picture only |
| **Horizontal repoint** (start moved by 16-byte steps: 4 px at 32 bits) | **not measured** | |
| **Scan-out with an RGB565 screen plus a 32-bit video window** (memory bandwidth: underruns?) | **not measured** | |
| **Turning planes on and off between frames** (screen 0 alone ↔ screen 0 + page layer) without a flash | **not measured** (the window is known) | |
| **The 2D engine writing ARGB pictures** whose alpha the display then uses (plain fills and copies) | **not measured** | the engine's blend onto RGB565 measured (~16 Mpx/s), not alpha-keeping copies |
| **The display's mix** against the 2D renderer's own compose (a hand-off seen as a jump?) | **not measured** | the scan-out model's arithmetic is "unknown on the device" (2700g.md) |
| **MBXCLK 0 (2D and 3D both off) with the display still scanning**; `ODFBPWR` low power while the display reads screen 0 | **not measured** | Dell's off path writes 0 (#39); never run by us |
| **What any 2700G state draws from the battery** | **not measurable yet** | no battery current reading built (REA #35 found Dell's) |

## The layout (ours, "L2")

- **Screen 0** (`0x0`, RGB565, the graphics plane, the bottom slot,
  opaque) holds the wallpaper and the chrome: the header, the tabs and
  their underline, the softkeys, the scroll thumb. It is the screen
  R115 already writes. Small changes to it (the underline following the
  pen, the thumb) are copied in behind the scan, as now.
- **The page layer** (the video plane, the slot above, per-pixel alpha)
  holds a page's glass rows and text with real alpha, transparent
  between the rows. Its window is the page's rectangle (480 x 462 at
  y 94, `VSADR` and `VSCTRL`), so the header and softkeys are never
  covered.
  - It reads from a **tall picture**: the whole Hourly list (24 rows,
    1,092 lines), plus 120 transparent lines above and below for the
    rebound (`motion.rs` `BAND_MAX_PX`).
  - In ARGB8888 that is 480 x 1,332 x 4 = 2.4 MiB; in ARGB4444 + dither,
    1.2 MiB. The pool has 8.4 MiB (`rp-vram-map` `POOL`).
- **The display mixes them.** The page layer is the one translucent
  layer over still content, which is ADR-0022 point 3's own case (one
  blend stage, "the planes swapped").
- **What is drawn when.** The rows are drawn once, when the data, the
  units or the theme change, never to move them (ADR-0022 point 3: "a
  layer that moves with the content is frozen into the panned picture").
  A press highlight redraws one row (~0.6 ms by the 2D engine, as W2).
- **While the 3D core draws, the video plane is off** (amendment P,
  rules 2 and 4). The page layer lives outside the safe region, so the
  Weather app keeps 3D to the moments where it is needed (Expand), and
  hands the page back to screen 0 before them (below).

## Each movement, mapped

| Movement | ADR-0034 mode | How | Engines a frame | Predicted | Rests on (tests below) |
|---|---|---|---|---|---|
| **Hourly drag 1:1, fling, rebound** (and 10 Day's rebound) | 3, pan under glass (planes swapped) | `VSCADR` = the picture's start + scroll x 1,920 bytes, written at the deadline in the blanking (`flip_due`, R115's model). The rebound shows the transparent margin, so the still wallpaper shows past the ends | **none** (one register; the thumb's copy on screen 0) | 60 fps; CPU under 1 ms a frame (pen + `motion.rs`); latency about one refresh | M1, M2, M4 |
| **Page turn by swipe** (Now ↔ Hourly ↔ 10 Day ↔ Radar) | 3, horizontal | A **strip picture**: the shown page and its neighbour side by side (960 x 462, stride 3,840 at 32 bits), built when a swipe starts or ahead while idle; the window follows the pen by moving the start address (4 px steps at 32 bits, 8 at 16). It settles on a multiple of 480 (exact); the underline moves on screen 0 by copies | **none** while moving; one 2D build of the neighbour (or prepared ahead) | 60 fps; the neighbour's build (~a page compose, to measure) before the first frame, or none when prepared | M1, M2, M3, M5 |
| **Entrance: None** | 1, still | The page layer switched on at a blanking | none | 0 frames | M4 |
| **Entrance: Rise** (the page rises 40 px as one) | 2-3, pan | The same repoint through the margin, eased (`rise()`), 15-20 frames | **none** | 60 fps | M1, M2, M4 |
| **Entrance: Stagger** (rows slide in from the right one after another) | 2D into the page layer's back picture, then a flip by repoint | Two page pictures. Each frame the 2D engine clears the band (one fill, alpha 0) and copies each row's finished pixels at its offset (rows don't overlap, so plain copies, no blending), then repoints at the blanking | 2D: ~190k px of plain 32-bit copies a frame | ~60 fps if the copies take under ~10 ms (estimate from W2's rate; to measure) | M3, M5 |
| **Entrance: Expand** (the hero card grows into the app) | 4-5, the 3D core | Starts from the Today screen on screen 0 (no planes): the card and the page as textures scaled into place, rendered into screen 0 behind the scan (P1 C, R-B), with the copy path behind it. When it ends, the page layer takes over at a blanking with the same picture | 3D (+ uploads before it: a page texture ~35 ms, run E) | ~30-40 fps (W3's render, 10.1 ms median; W3 missed 250 ms by 105) | M6 |
| **Radar loop and its scrub** (rain cells over the Night map) | 3, a swapped overlay | The map is drawn once into screen 0 (opaque, still). The rain is in the page layer: two ARGB4444 pictures of the map's area. The next loop frame is cleared and its cells copied in by the 2D engine (plain copies; where cells overlap, the heavier rain step is kept, decided on the CPU), then a repoint. The scrub follows the pen the same way | 2D: a few cell copies per loop step (every 400 ms) | 60 fps for the scrub; each step a fraction of a frame | M3, M5 |
| **Leaving for 3D** (any 3D frame while the page layer shows) | 3 → 4 | The page is composed into screen 0 first (2D, behind the scan), then the video plane goes off at a blanking, then 3D starts (amendment P rule 2) | 2D, once | one compose before the 3D frame (to measure; today's page compose is 10-40 ms) | M4, M5 |

The rest stays as it is today: a still screen draws nothing (mode 1),
taps answer through R115's copy behind the scan, and settings and the
keyboard stay 2D pages (their own redesign comes as mock-ups, run 2's
finding).

## The alternatives

| | What | Wins | Loses |
|---|---|---|---|
| A. **2D round 2** (`weather-drag-speed.md`, set aside) | Compose moved glass once a frame, profile what is left | Small change | Predicted ~25 fps for a drag: still the 2D engine and the CPU every frame. The owner ruled it out for movement |
| B. **ADR-0034 C4 exactly as voted ("L1")** | The page with its wallpaper drawn into it, RGB565 and opaque, on the video plane; the graphics plane the chrome | 60 fps pans, half the memory (RGB565), opaque (no per-pixel alpha test) | **The wallpaper scrolls with the list**, and every tall page holds its own wallpaper copy (C4's minus). The glass no longer moves over a still backdrop, which is the look the mock-ups show. Page turns need the neighbour with the wallpaper |
| C. **Everything through the 3D core** | Pages and the wallpaper as textures, every moving frame rendered (mode 4 or 5) | One mechanism for all movements; scaling and fades anywhere | A full-screen textured layer is 25-29 ms (~35 fps) and glass adds ~8 us a tile; each page uploads ~35 ms first; the display must read only the safe region (no planes while moving); **the 3D clock runs for every drag**, against R133 |
| D. **WM's way** (`ScrollWindowEx`: copy, repaint the strip) | Copy the page up or down by the 2D engine, draw the new strip | Old and simple | With glass over a still wallpaper, every pixel under a moving row changes, so the "strip" is the whole page again: this is what run 2 measured |
| **Ours ("L2")** | The page's glass on the video plane with per-pixel alpha over a still screen 0; repoints for every whole-page movement; the 2D engine only for Stagger and the radar's cells; the 3D core only for Expand | 60 fps for drag, fling, rebound, turns, Rise and the radar; **no engine clocked while a page moves**; the wallpaper still under moving glass; only the changed row is ever redrawn | Rests on four unmeasured plane facts (M1-M4); 2x C4's memory at 32 bits (1x at ARGB4444); text on ARGB4444 may look worse than on 8888 (the owner's eyes decide); **changes C4**, so it needs the owner's vote (V1) |

## ADR rules checked

- **ADR-0034 C1** (layers the 3D core moves: drawn in SDRAM, reordered,
  DMA). Holds: only Expand uploads textures.
- **C2** (16-bit opaque, glass ARGB4444 + dither or ARGB8888). Holds:
  screen 0 is RGB565 and the page layer is glass. Which depth is V3.
- **C3** (full-screen 3D effects as ~0.3 s moves at ~20-35 fps). Holds:
  Expand is the only full-mode effect, at W3's ~30-40.
- **C4** (the video plane is the panned page with its wallpaper drawn in).
  **Changed by L2, so it needs the owner's vote (V1).** B above is C4 as
  voted.
- **C5** (text and glass over 3D as layers). Untouched.
- **Gap 1** (never a 3D render over a plane picture). Holds: 3D only
  composes textures, and the page goes back to screen 0 before any 3D
  frame.
- **Gap 2** (the price table fixed, renders logged only). Holds: Expand's
  frames are priced and logged as run CH did.
- **Amendment P, rules 1-2** (the display reads only the safe region while
  the 3D core draws). Holds: the video plane is off for every 3D frame.
- **Rule 4** (planes reading outside the region only in frames where the
  3D core is idle). Holds: every L2 movement is a 3D-idle frame.
- **R-B and the copy** (screen 0 shown in 3D frames, the copy fallback,
  correctness before speed). Holds: Expand uses them unchanged.
- **The safe end `0xB0000`**. Holds: screen 0 alone inside it; the page
  layer outside it.
- **ADR-0022 point 3** (one translucent layer over a pan by scan-out
  blending, which arrangement "settled by those results, with the owner";
  bounded content drawn once). Holds: the Hourly list is bounded (24 rows).
- **ADR-0030 G2** (one queue: 2D and 3D in turns). Holds: repoints use no
  engine.
- **G3** (clients get handles from the pool under a quota). The page
  layer, strip and radar pictures are pool blocks by handle.
- **ADR-0026 D8** (the 3D clock off 100 ms after the last render). Kept;
  R133 below asks for more.
- **ADR-0035 rule 8** (clock changes only where the display can take it).
  No clock change here.
- **Rule 6** (the idle guard: one 64 x 64 check a second while the GPU is
  idle). **This conflicts with R133:** it wakes the 3D clock every second
  while the screen is still. That is V2.
- **ADR-0027** (one pool, quotas, no silent eviction). Pages are built on
  demand and dropped when off screen (C4's own rule, kept).

## R133: the GPU only while the screen moves

- **What already holds.** A still screen draws nothing (mode 1). Under L2,
  a moving page needs no engine either: the display repoints, and the
  display runs anyway (the panel's refresh). Only Stagger (2D), the radar
  steps (2D, every 400 ms), content changes (2D) and Expand (3D) need a
  core.
- **The policy proposed.**
  - The 3D clock goes **on at the first 3D frame and off 100 ms after the
    last** (D8, built: `clock::Gate`). That is unlike Dell, which keeps
    it on for a context's lifetime (#40).
  - The 2D clock goes off after the same idle time, **if M7 shows the
    display keeps scanning with it off**.
  - The on-die memory goes to active low power while still, **if M7 shows
    the display reads screen 0 right that way**.
- **The wake time in the frame budget:**
  - the 3D clock on and the first frame ~1.0 ms (T9), plus the power
    manager's 164 us;
  - the 2D clock's wake, to measure (M7).
  - The first moving frame after a still screen is a repoint, which needs
    no wake, so a drag never waits. Only Stagger, Expand and the radar's
    first step pay the wake, priced into their first frame.
- **Measuring the draw needs a meter we haven't built.** Dell's battery
  driver reads the battery current through the TSC2046's AUX input (REA
  #35); we have no current reading yet. Proposed, as R133's own first
  step:
  - build that reading and check its resolution against a known load (the
    backlight's steps);
  - if it can't resolve a few milliamps, a night drain soak per state
    instead (the battery's voltage over hours, the cradle out, owner's go).

## The device tests (one image, ~10 minutes, the owner watching; no run without his go)

| Test | What | Answers | Overturns L2 if |
|---|---|---|---|
| **M1** | ARGB8888 and ARGB4444 + dither pictures on the video plane, per-pixel alpha, in the slot above an RGB565 screen 0. Every refresh's `DSIG` against the scan-out model's picture; the underrun flag; the owner's eyes on text over glass, 8888 against 4444 | Whether per-pixel alpha works on the video plane, the depth for V3, the bandwidth | per-pixel alpha is ignored or wrong on the video plane, or underruns. **Then:** B (C4 as voted, opaque RGB565) for drags, Stagger on the 3D core |
| **M2** | The window 480 x 462 at y 94 over a 480 x 1,332 picture, panned by `VSCADR` at the deadline: 300 frames of a scripted fling and rebound. Torn refreshes by `DSIG`, late repoints, CPU a frame | Whether a partial window pans like W5's full screen | any torn or late frames from the window itself. **Then:** the window full-screen, with transparent rows over the chrome area (more memory, same mechanism) |
| **M3** | Horizontal: a 960-wide strip (stride 3,840) moved in 4 px steps, then 1 px steps (to show what a non-16-byte start does), slow and fast | The turn's granularity; whether a 4 px step shows on a slow drag (the owner's eyes) | 4 px steps visibly jitter. **Then:** the window's X position for the last 0-3 px, or the turn on the 3D core at C3's 20-35 fps |
| **M4** | Hand-offs: screen 0 alone ↔ screen 0 + the page layer, switched in lines 643-3, 50 times each way, with the same picture before and after. `DSIG` of the two compositions; the owner watching for a flash or a 1-step colour jump | Whether the handover is invisible; how far the display's mix is from the 2D renderer's | a visible jump. **Then:** the page shown through the plane from its first frame (never composed into screen 0 while it shows) |
| **M5** | The 2D engine building ARGB pictures: alpha-0 fills, plain copies of ARGB rows and radar cells, a Stagger frame (10 rows), the neighbour page for a turn, the page composed back into screen 0. Times, CPU time, and every picture hashed against the CPU model's | Stagger's fps, the turn's first-frame cost, the hand-back's cost | a Stagger frame over ~14 ms. **Then:** Stagger on the 3D core (rows as textured quads, ~30-40 fps) or a different entrance (R131) |
| **M6** | Expand on the 3D core at 149.50 / 2: the card and the page as textures, 15-20 frames into screen 0 with the copy fallback; uploads timed. The run CH ways (turns against the pipeline), every render checked | Expand's fps against the 30 rule | under 30 fps after the C3 degrade order. **Then:** Expand is optimized first (the owner's 30 fps rule), and drops only if it can't be made faster |
| **M7** | R133: with the page layer and screen 0 showing, `MBXCLK` 2D off, then both off (Dell's off path, #39), then `ODFBPWR` 1, **one setting per step**, the owner watching that the screen holds. Each wake timed (2D: first command done; 3D: first render, against T9) | What may sleep while still, and the wake time for the budget | the screen glitches with a clock off. **Then:** that clock stays on while showing; only the 3D clock gates (D8) |

**Afterwards, not in this run:** the battery-current meter (R133's step
above), then the draw per state. The emulator: the reference scan-out
model (`drivers/mbx-display/src/scanout.rs`) treats any RGB format on the
video plane as "off" (`video_fmt == video`). Run D proved those formats,
so the model and the QEMU fork must model them before M1's emulator check
(the emulator-tracks-device rule), along with the video window's
position.

## The build, after the go and the tests (each step: host test, emulator check, device run on "run it")

1. A **page layer in `rp-graphics`**: the video plane's window, format,
   slot and blend mode, a repoint at the deadline (R115's `flip_due`),
   the pool block by handle. The scan-out model and the fork learn the
   video plane's RGB formats.
2. **`rp-shell`**: `WeatherApp` draws each page into its layer once
   (rows cached); `motion.rs` gives the scroll offset, which becomes the
   repoint. The renderer draws only screen 0's chrome.
3. Page turns (the strip), Rise and None; then Stagger (2D into the back
   picture).
4. The radar's two pictures.
5. Expand on the 3D core, then R133's clock policy (after M7).

**Expected on the X51v** (to be measured, not promised):

- drag, fling, rebound, turns and Rise at 60 fps, with the CPU under a
  millisecond a frame against 37-40 ms today;
- Stagger near 60;
- Expand ~30-40;
- radar steps within a frame;
- no 2700G core clocked while a page simply moves.

## Rule 0 for the choices left (the owner, 2026-10-30: "rule 0 2-4 making sure you give me the cheapest most clever options")

### V2: the GPU guard against R133

**What the guard is for:** ADR-0035 rule 6. A 2700G that starts failing at
our raised clock (149.50 / 2, above Windows Mobile's 99.67 / 2) is caught
within ~1 s while idle and ~10 s in a game, and the clock drops to Windows
Mobile's.

- **Windows Mobile and Dell (a finding):** no guard. They never run above
  their own clock (REA #34), and the 3D clock stays on for a context's
  life (#40).
- **The modern way:** nothing runs test pictures on a timer. Errors are
  caught as they happen:
  - hang detection and reset (Windows' TDR, Linux's GPU scheduler
    timeout);
  - memory error counters;
  - thermal and voltage monitors that lower the clock.
- **The fact that makes it cheap:** a failing chip harms only frames it
  draws, and **a still screen draws nothing**. A check while idle can't
  protect a frame; only a check near real work can.

| | What | Wakes a still screen | Catches a failure | Cost |
|---|---|---|---|---|
| A. Rule 6 as it stands | One 64 x 64 check a second while idle | **yes, every second** (3D clock on, ~1 ms) | within ~1 s idle, ~10 s in a game | ~0.1% GPU, but the clock never stays off: fights R133 |
| B. Check on wake | One check before the first real frame after idle | no | before the first frame drawn | adds ~1-2 ms to the first moving frame's latency |
| C. Only frames that already exist | Compare real frames against a reference | no | **can't**: a real frame has no known answer, except the screen's own `DSIG` for pictures shown twice (rare) | none, but it protects nothing new |
| **D. Ours: check in the tail** | The 3D clock already stays on 100 ms after the last render (D8). One rotating check runs inside those 100 ms, after each burst of work, then the clock gates. A game: one every 10 s as now. The CPU's hang detection and the underrun flag stay as they are | **no** | after every burst of work, before the next; all kinds covered within ~10 bursts | ~1 ms of GPU inside a window already paid for; **zero latency, zero extra wakes** |
| D+. D, plus temperature | D, and when the battery's thermistor (REA #35, the R76 temperature guard) shows a rise since the last check, the next burst starts with a check | no | as D, and earlier when it heats up | as D |

**Recommended: D now, D+ when the battery temperature reading exists.**
And at Windows Mobile's own clock the guard isn't needed at all. **What
overturns D:** a failure that appears while idle and spoils the first
frame of the next burst. The guard's history has none (every failure seen
was immediate; ADR-0035's review, point 2). **The test:** none extra. It
is host logic over the built clock gate, and its effect shows in M7's
clock log.

### The device run itself (the fast-tests rule)

- **Dell and Windows Mobile (a finding):** no test of this kind.
- **The modern way:** automated display CRCs per frame (Linux's IGT `kms`
  tests, the pipe's CRC), people only for what a CRC can't judge. **Ours
  already has that:** the display's own `DSIG` per refresh (R110, R115).

How the run is cut to the fewest minutes that answer it:

1. **The emulator first.** Fix the scan-out model's video-plane RGB
   formats and window (the fork too), and run the whole image there.
   Wiring mistakes cost no device time.
2. **Ask R131 before building the image.** Only the chosen entrance is
   tested: Rise, None and the new free options below need no extra test
   beyond M2's window. **M6 (Expand on the 3D core) and M5's Stagger part
   drop out** unless the owner picks one of them.
3. **Unattended first, by hashes.** M1-M5's pictures are each checked by
   `DSIG` against the model's picture, every refresh, with the times
   logged to the card. That takes about 2 minutes, and the owner doesn't
   need to watch.
4. **One watched block of about 2 minutes at the end.** The owner's eyes
   for only what a hash can't judge:
   - M1's text at the two depths, side by side on one screen (not two
     steps);
   - M3's slow 4 px drag;
   - M4's hand-off at full speed.
5. **M7 apart and last.** A clock switched off could hang a register read
   (`DSIG` included), so M7 writes its hang marker first (ADR-0035 rule 5)
   and the owner says whether the screen holds. It runs after everything
   else is safely on the card, so a hang there loses nothing.

**Estimate:** about 5 minutes on the device instead of ~10, one load.
**What overturns it:** the emulator can't model `DSIG`'s exact value
(docs/hw/emulator.md). Then the unattended checks compare `DSIG` between
two showings of the same picture (repeatability), as R110 did, and not
against the model.

### R131: how a page arrives

- **Windows Mobile (a finding):** no transition; the new screen appears.
- **SPB Mobile Shell:** not checked for its page entrances. Its own videos
  would answer it (the owner's reference, ui-spb scope).
- **The modern way:**
  - Material's motion system: "container transform" for a card that
    becomes a page, which is Expand; "shared axis" (slide and fade) for
    sibling pages; staggered lists as an option.
  - iOS: the app grows out of its icon; lists appear whole.
  - Both honour "reduce motion" (None, or a plain fade).
- **What's new for us:** under L2, the video plane's **window** (its place
  and size, `VSADR` and `VSCTRL`) is as cheap to change in the blanking as
  its address. Two new entrances come from it, both free:

| Entrance | How it runs under L2 | Engines a frame | fps | Looks like |
|---|---|---|---|---|
| None | the layer switched on | none | n/a | instant |
| Rise (the mock-up) | the address moved through the margin | none | 60 | the page eases up 40 px |
| **Reveal (new)** | **the window grows from the tapped card's rectangle to the page's**, the page in place behind it, revealed, not scaled (a clip, as container transforms often do) | **none** | 60 | Expand's feel: the card opens into the app |
| **Cascade (new)** | **the window's height grows downward**, rows appearing top to bottom, plus Rise's ease | **none** | 60 | Stagger's top-to-bottom order, without per-row slides |
| Stagger (the mock-up) | 2D copies of each row into the back picture | 2D, ~190k px | ~60 if M5's copies fit | rows slide in one by one |
| Expand (the mock-up) | 3D: the card and the page scaled | 3D + a ~35 ms upload first | ~30-40 | the card grows, content scaled |
| Fade (only if M1 shows it) | per-pixel alpha with the global alpha on top; one mode field may not allow both | none | 60 | a fade-in |

- **Recommended:** Reveal from the Today screen's card, and Rise or
  Cascade between pages. Every one is free and holds 60 fps.
- **Stagger and Expand stay choices,** at the costs shown; the owner
  picks.
- **The mock-ups:** Reveal and Cascade need adding to the preview before
  he judges them. That is a small change in `rp-shell`'s `Motion`,
  recorded as GIFs like the others, and waits for his go.
- **What overturns them:** the window's size and place not changing
  cleanly in the blanking (tears, or a frame with the old size and the
  new place). M2 adds that check: the window grown and shrunk every
  frame, `DSIG` per refresh. If it fails, Reveal falls back to Expand on
  the 3D core and Cascade to Rise. Fade's extra check is one setting in
  M1.

## The owner's rulings (2026-10-30, evening)

"2 do recommended, 3 you can do the whole 10 min test if itll be better,
do 5 if its just as good, 4 i like cascade":

- **V2: D.** The guard's check runs in the 3D clock's 100 ms tail after
  each burst, never on a timer while still; D+ (temperature) when the
  battery temperature reading exists. Written into ADR-0035 rule 6.
- **The device run: the ~5 minute cut.** It answers the same questions
  for what will ship: the tests it drops (M5's Stagger part, M6) only
  measure entrances not chosen, and the window check Cascade rests on
  stays in M2. If the owner later picks Stagger or Expand, those two
  run then.
- **R131: the recommendation (the owner, then: "4, do what you
  reccomend").** **Reveal** when the app opens from the Today screen's
  card, **Cascade** between pages (his "i like cascade" picks it over
  Rise). The moving mock-ups of both come first, from the preview's real
  frames like the others, so he sees them before the device build.
  Stagger and Expand are not built for the device; M5's Stagger part and
  M6 drop out of the run.
- **The mock-ups (made the same night):**
  `docs/design/mockups/weather-choice-reveal.gif` and
  `weather-choice-cascade.gif`, from the shell's own frames.
  - **Reveal, one free refinement:** the app is read lower inside the
    growing window (the start address offset by up to 52 rows), so the
    card turns into the app's big panel in place and eases up with it.
    It is the shared-element feel of Expand, without scaling. The window
    stops under the title bar, which both screens share.
  - **Composed as the planes will show it:** the Today screen around the
    window, the app inside it (`reveal_over` in `xtask/src/emu.rs`).
  - **The 2D renderer's fallback,** until the plane path exists: the app
    inside the window over the wallpaper.
  - **Cascade:** the page's window grows down from the tabs while the page
    rises 40 px; nothing inside it moves or counts up.
  - The build keeps Stagger as the device default until the owner has
    seen these.
- **V1: L2** (the owner, the same evening: "Yes, L2"). Written into
  ADR-0034 as a change to C4, with C4 as first voted the fallback if M1
  fails.
- **V3 (depth): ARGB4444 + dither** (the owner, 2026-10-30, after run
  WG's look, "8888 basically looks the same, just a little darker": "if
  4444 is cheaper just use that"). Half the memory (1.2 MiB for Hourly),
  half the display's reads and the 2D engine's bytes. At 2 bytes a pixel
  the start moves in 8-pixel steps, so the 1-pixel turn moves the window
  by 0-7 pixels inside the cards' clear 8-pixel edge: not yet run on the
  X51v (run WG turned at 8888), so it goes in R134's run as way H.

## The device image, built (2026-10-30, the owner's "Yes, build the test")

`rp-gpubench` run WG (`RP_GPUBENCH_WEATHER=1 cargo xtask codegen-variant
rp-gpubench wg`; code `boot/gpubench/src/weather.rs`, the pictures and
window states shared with the host test in `wg.rs`; emulator check
`RETROPANE_GPUBENCH_RUNS="run WG" cargo xtask emu-gpubench`). One load,
about 5 minutes, the loader's Windows Mobile clock (no clock change).

- **Unattended (~2 minutes), every refresh's `DSIG` judged:** every state
  a test shows gets its reference first (written in the blanking, a whole
  refresh with nothing written, its signature), and the first is taken
  again at the end (repeatable or the test reports "not judged"). Each
  refresh is then whole as due, the state before (late), another state,
  or torn. M1 (9 layouts: screen 0 alone, the page, a clear window over
  the margin that must read as screen 0 alone, the alpha ignored, at
  ARGB8888 and ARGB4444 + the shipped dither, Fade's probe, mode 2;
  the underrun flag over 1 s each; each also composed by the scan-out
  model), M2 (the fling and rebound twice, then Reveal and Cascade 8
  grow-and-shrink cycles each), M3 (4-pixel steps across, then 1-pixel
  steps over 64 pixels), M5 (an alpha-0 fill, ten rows' cards, 24 radar
  cells, the neighbour page, the hand-back composed into a second screen,
  an ARGB4444 row: each timed and checked word for word), M4 (the
  composed screen alone against screen 0 + the page layer, 50 hand-offs
  each way, and the model's mix against the 2D engine's compose).
- **Watched (~2 minutes, started by a button, or after 2 minutes):**
  the two depths, the turn dragged slowly in 4- and 1-pixel steps, the
  hand-off every half second, Reveal, Cascade and the fling at speed.
- **M7 last (~20 s):** with the page showing, the 2D clock off, `MBXCLK`
  0, then `ODFBPWR` 1, each 5 s with every refresh's `DSIG` read, its
  marker in the kept log first, put back, the first 2D fill after it
  timed.

**Where the build differs from the sheet's words, and why:**

1. **The depths in turn on one place, not side by side.** A plane takes
   one format, and screen 0 must stay the wallpaper, so the only
   side-by-side would be a picture composed by the CPU, which isn't the
   display's mix being judged. 8888 and 4444 swap every 2 seconds on the
   same rows (a blink comparison), the header naming the one showing.
2. **M7 times the 2D clock's wake only.** At Windows Mobile's `MBXCLK`
   `0x5` the 3D clock is already off (bits 1-0: 1 = 2D only), so there is
   no "3D off" step separate from "both off"; the 3D wake stays T9's
   ~1.0 ms (ADR-0026). "2D off" is bits 1-0 = 0 with the divider kept;
   "both off" is Dell's whole-register 0 (#39).
3. **The 1-pixel steps are made, not shown failing.** `VSCADR` holds the
   start in 16-byte units, so a start between them can't be written. The
   build takes the sheet's own fallback at once: the start on the next
   4-pixel step and the window moved right by the 0-3 pixels left (its
   width cut to stay even, inside the cards' clear 8-pixel edge).
4. **M2 at ARGB8888 only,** the heavier depth (V3 open; M1 holds both).
5. **The references are the device's own** (the emulator doesn't model
   `DSIG`), as the sheet's "what overturns it" foresaw. The scan-out model
   checks the layouts on the host and in the image (`model` hashes), and
   the emulator run checks the wiring and the 2D pictures.

## Run on the X51v (2026-10-30, gpubench-0248 and -0249, twice)

Full numbers in `docs/hw/2700g.md` "Run WG on the X51v". In plain words:

- **M1 holds L2:** per-pixel alpha on the video plane works at both
  depths, no underrun. The fallback (B, C4 as first voted) isn't needed.
- **M2 and M3 hold:** the fling and rebound, Cascade and the turn (4- and
  1-pixel steps) were whole every refresh in both runs, with the CPU
  under 4 us a frame (37-40 ms today). **Reveal tore 15 and 10 of 304
  refreshes**: the only test that moves the window's top edge. Not yet a
  reason to fall back to Expand: the cause comes first (which steps, the
  write order).
- **Fade is not free:** mode 3 ignores the global alpha.
- **M4:** every hand-off landed in its refresh, none torn, but the
  composed screen and the layer are not the same picture; whether the
  difference shows is the owner's eye.
- **M5:** the 2D pieces of a turn and a hand-back: the neighbour page
  7.3 ms, a hand-back 24.6 ms; the 2D blend differs from our model in 19%
  of words (to find).
- **M7:** the screen held with the 2D clock off and with `MBXCLK` 0, and
  with the on-die buffer told to go low power; the 2D engine back in
  ~90 us. R133 can gate the 2D clock while a page is still.
- **The owner's eye (the watched block, run 2), his words:**
  - The depths: "8888 basically looks the same, just a little darker"
    (V3 not yet ruled: the look is nearly a tie, and 4444 is half the
    memory).
  - The turn in 4-pixel steps: "the white lettering flashes a little bit";
    in 1-pixel steps (the start plus the window's place): "VERY smooth".
    So a turn follows the pen to the pixel, never in 4-pixel steps.
  - Asked again by step number: the hand-off, no flash or colour jump
    (so the display's mix differing from the 2D engine's doesn't show);
    Reveal looks good but he saw its tears ("can we chase down the tears
    though?": R134, sheet `r134-reveal-tears.md`); Cascade and the fling
    look good; M7: the page held still and right in all three steps.

## Votes needed (results are not decisions)

- **V1: the page layer.** L2 (ours: the glass over a still wallpaper on
  the video plane; this changes ADR-0034 C4) or B (C4 as voted: the
  wallpaper scrolls with the page).
- **V2: ADR-0035 rule 6's idle guard against R133.** Recommended D (the
  check in the clock's 100 ms tail after each burst, no wakes), D+ with
  the battery temperature later; the table above has A-C.
- **V3: the page layer's depth**, after M1's look: ARGB8888 (2.4 MiB for
  Hourly) or ARGB4444 + dither (1.2 MiB).
- **R131 (pending): the entrance.** None, Rise and the new Reveal and
  Cascade cost nothing under L2; Stagger costs 2D copies (M5); Expand a
  3D effect (M6). Mock-ups of Reveal and Cascade first, on the go.
- **The go** for the device image as cut above (~5 minutes, one load, a
  2-minute watched block), after R131 so only the chosen entrance is
  tested.
