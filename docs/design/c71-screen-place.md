# C71's class fix: what the second screen becomes (a ruling for the owner)

**Status: question for the owner (2026-10-25). Nothing here is decided.**
Quality audit C71; ADR-0034 amendment P (accepted 2026-10-24, P1 changed
to C); ADR-0023 (two screens, the flip); ADR-0027 (`rp-vram-map`).

## In plain words

When the 3D core draws while the display reads 2700G memory past the
first ~696 KiB (the safe region, `0x0`-`0xAE000` on the owner's X51v),
some of what the 3D core writes comes out wrong. That is C71: stale pixels
in partial frames, at Windows Mobile's own clock too. The cure is proven:
the screen the display reads placed at `0x0` (C71's runs: 0 of 100 renders
wrong against up to 20 of 20; GC2S: every kind right at 133.25 and 149.50
/ 2, 4-5% faster).

Amendment P already ruled where things go while the 3D core draws: the
shown screen inside the safe region (rule 2), and since P1 changed to C,
the 3D core draws straight into that screen (divide 1 above 74.75 MHz
fails with the 3D core's target outside the region: G21). What it didn't
rule: **its rule 5 says the map gets "one screen at `0x0`; the second
becomes the 3D core's off-screen target", which was written for option A.
Under C nothing draws off screen, so the second screen has no job written
down.** ADR-0023's 2D renderer flips between two screens; only one 600 KiB
screen fits in 696 KiB. That gap is the whole ruling.

## The evidence

| Run | What it showed |
|---|---|
| C71 runs 1-4, control (2026-10-24) | 99.67 / 2, five multi-quad scenes: 1-20 of 20 renders differ with the screen at `0x9D6000`; the 3D core's renders inconsistent at Windows Mobile's own clock |
| C71 paused, placement (`gpubench-0080`, `0081`) | the display's fetch paused, or the screen at `0x0`: 0 of 100 differ in every kind, each render ~2 ms faster |
| G18, G19 | right while the screen ends by `0xAE000`, wrong from `0xB6000` |
| G20 | the 2D engine unaffected by where the display reads |
| G21 | at divide 1 above 74.75 the 3D core's target outside the region fails (why P1 became C) |
| GC2S | the screen at `0x0`: 9 of 9 kinds right at 133.25 and 149.50 / 2 (7 of 9 outside), 4-5% faster |

## The options

**Iterate (rule 0):** Dell and Windows Mobile keep one screen at `0x0` and
no flip (G17: Windows Mobile's screen sits there); the textbook answer is
a double buffer. Ours: whichever option is chosen, the 2D engine (immune
to the contention, G20) does the moving, and the 3D core only ever writes
the safe region, so neither copy nor flip touches the measured fault.

| | What the map holds | Pros | Cons |
|---|---|---|---|
| **R-A: one screen** | screen 0 at `0x0`; screen 1's 600 KiB returned to the pool | Simplest; nothing ever reads outside the region; +600 KiB for textures | Every frame, 2D too, draws into the visible picture: tearing in 2D frames that today flip tear-free, until R20's design (drawing behind the scan line) covers 2D as well as 3D |
| **R-B: a safe screen and a 2D back picture (recommended)** | screen 0 at `0x0` (the only picture shown while the 3D core works); screen 1 kept outside as the 2D renderer's back picture | 2D-only frames keep today's tear-free flip (amendment P rule 4 already allows planes outside the region when the 3D core is idle); in 3D frames 2D still draws off screen into screen 1 and the 2D engine copies the changed rectangles into screen 0 (G20: exact, ~5 ms for a whole screen, rectangles in proportion), then the 3D core draws into screen 0 | Most code: the 2D renderer lands on screen 0 before any 3D work, and a 3D frame's area must be carried into screen 1 for the next 2D frame (a copy); 3D frames still tear until R20 |
| R-C: amendment P's A | screen 0 at `0x0`, the 3D core draws off screen, the 2D engine copies | Tear-free everywhere | Contradicts P1 = C: fails at divide 1 above 74.75 (G21); only if amendment D's Q1 ends at divide 2 |

Not options: B (the display's fetch paused during renders: flicker, voted
down in amendment P) and D (keep the map: C71 stays).

## What is ready, and what the ruling decides

Ready on branch `c71-screen-place` (not merged; built on `gpu-gate`):
the part both R-A and R-B share. `rp-vram-map`'s screen 0 at `0x0`, the
safe region's end as a constant (`SAFE_END`, 696 KiB, G19, the smallest
proven until each device's self-test measures its own, amendment P P3),
build-time checks that screen 0 lies inside it and that nothing else of
the OS map does but the video band right after it (rule 1), the loader's
and Windows Mobile's planes region moved past the screen, host tests.
Screen 1 stays where it is (`0x296000`), so screen 0's old 600 KiB at
`0x200000` sits unused until the ruling: R-A returns both to the pool.
Device risk to check: the loader's own picture at `0x60000` lies under
screen 0, so the loader's screen after an image returns must be looked
at. The test images' frozen map (`bench32`) is
untouched.

After the ruling: R-A removes screen 1 and makes the 2D renderer draw into
the one picture; R-B keeps screen 1 and changes the 2D renderer's `back`
and the graphics service's frame (land on screen 0 before any 3D work,
copy 2D's rectangles in, carry the 3D area back). Either way the device
check is the same: `rp-today` on the X51v (the shell drawn right, the
loader's screen right after it returns), and C71's W5 partial-frame scene
held to a fresh render each frame (0 words off expected, against 1-3
today). Until the ruling, partial frames in the harness stay held to fresh
renders, as the audit says.

## Question

Under P1 = C, what does the second screen become: **R-A** (gone, one
screen), **R-B** (kept as 2D's back picture, never shown while the 3D core
works), or **R-C** (only if amendment D's Q1 lands on divide 2)?
