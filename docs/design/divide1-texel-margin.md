# Divide 1's texel picks: a margin proven, not guessed

**Status: design for a test (run GC2AM), 2026-10-26. Nothing decided:
the owner rules ADR-0035 amendment D.** Owner's ask: "Chase it with a
close look at me be clever, modernize, and iterate dont imitate rule.
research how similar issues have been solved, cleverly, in the past when
issues like this have popped up, losslessly."

## What the fault is (from GC2AL's card log, gpubench-0112)

FT2's 8888 texture is built so a texel's red is its column (`texel8888`:
red = u << 3, so RGB565 red = u). Every wrong pixel of GC2AL's
every-render scenes has red exactly one higher than the reference, green
and blue unchanged: **the core sampled the texel one column to the
right**, nothing else. The wrong pixels are whole screen columns spaced 23
pixels apart (x 43, 66, 112, 135, 158, 181), and those rectangles map 32
texels onto 184 pixels: 5.75 pixels a texel, so 23 pixels is 4 texels.
With the half-texel shift the samples sit at u = 0.5 + 4m/23 texels:
every 4th texel boundary has a sample 1/46 of a texel from it (the
closest any sample comes; exact ties are impossible at this ratio). At
Windows Mobile's clock the core rounds those samples one way; at divide 1
the other, every render. So divide 1's texture coordinate is off by at
least 1/46 texel (5.6 of the 8-bit steps texture units commonly keep) at
those places, in the positive direction, and deterministically. The
remaining textured faults of GC2AK at 117.00 / 1 are the same kind (red
+1, or a clamp edge's texel), not the tile race.

## How this has been solved before (research, 2026-10-26)

- **Direct3D 9's half-pixel offset** (Microsoft, "Directly Mapping Texels
  to Pixels"; Pranckevičius, "Solving DX9 Half-Pixel Offset"): pixel and
  texel grids offset by half a cell put point samples exactly on texel
  edges, so different hardware picks different texels; the fix aligns the
  sample to the texel's centre. Our GC2Z shift is this rule.
- **Texture units round coordinates to fixed point** (Reed, "Texture
  Gathers and Coordinate Precision": D3D11 requires at least 8 fractional
  bits of sub-texel precision, Vulkan at least 4; most hardware keeps 8):
  within the last fraction of a texel the hardware can disagree with
  arithmetic done elsewhere; the fix is a small fixed offset (1/512 texel
  there) so no sample sits at the rounding edge.
- **Per-vendor epsilons** (DuckStation's PowerVR position epsilon 0.001
  against 0.00001 elsewhere; mobile guidance to keep texture coordinates
  at full precision): workable, but tuned by trial on each GPU.
- **Point sampling exactly at edges is unspecified** (a 2010 Mesa report
  of r600 sampling the neighbouring texel at an edge was closed as not a
  bug): only a sample clear of an edge has one right answer.

What they share: **keep every sample clear of texel edges**. What none of
them does is measure how clear is enough for the hardware at hand; they
guess an offset and tune it until the artefacts go away.

## Ours (rule 0): the margin is measured, and the geometry guarantees it

1. **Measure the error, not guess it.** Slide the texture's phase under
   the faulting rectangle in steps finer than the fault (part 1): the
   phase where divide 1 first disagrees with Windows Mobile's clock is the
   error's size, on this hardware, at each clock.
2. **Guarantee the margin by the ratio, not by luck.** At a scale of p / q
   pixels a texel (lowest terms), the samples' fractions are p evenly
   spaced values, so the best phase leaves every sample at least 1 / (2p)
   of a texel from an edge, and no phase can do better. 1:1 gives half a
   texel (128 of the 256 steps), 2:1 a quarter, 4:1 an eighth, 1:2 half;
   5.75:1 (p = 23) only 1/46: smaller than the measured error, which is
   why no offset fixes it. So the 3D core draws point-sampled textures
   only at ratios whose margin clears the measured error with room to
   spare, each at its best phase (1 / (2p) texel; the half-texel shift is
   the 1:1 case).
3. **Everything else is scaled by a part the fault doesn't touch.** A
   picture wanted at another size is scaled first by the 2D engine or the
   video plane's scaler, with our designed filter tables (filter.rs,
   `designed`), then drawn by the 3D core at a safe ratio. The OS's
   compositor draws icons, panels and pictures at 1:1 almost always, so it
   lives in the safe class by nature.

**Why it beats the precedents:** a guessed epsilon is statistical (it holds
until a scene finds the next bad case); a margin larger than the measured
error is a proof, per ratio, for every sample of every object. It costs
nothing at draw time (a phase per object, chosen once) and keeps divide
1's speed for the work the OS actually does. **What it doesn't cover:**
free 3D (games: perspective, arbitrary ratios) can't promise a ratio; that
is the owner's call (divide 2 for such work, or a measured residual rate).

## The test (run GC2AM)

- **Part 1, the error:** GC2AL's faulting rectangle (scene 1, rectangle 2:
  x 23-207, y 224-384, 32 texels on 184 x 160 pixels) alone over one full
  layer and the Gouraud, its u phase stepped across one 1/23-texel period
  (the sample pattern's period at this ratio) in 31 steps of 3 / 65,536 of
  the texture (about 1/683 of a texel), 3 renders each, at 149.50 and
  117.00 / 1. Where the wrong columns start and stop gives the error.
- **Part 2, the rule:** textured rectangles at 1:1, 2:1, 4:1 and 1:2, each
  at its best phase, five a scene at all 128 corner places of a tile (the
  geometry class GC2AJ showed matters), 30 rounds a scene, at 149.50 and
  117.00 / 1. Every target poisoned, every plan with its references and
  its controls at Windows Mobile's clock.
