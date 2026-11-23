# R360: the flashlight's 3.2 ms (rule 0 sheet, 2026-11-22)

Measured (X51v, the fixed-seed `-rpfight` on c2a5f, 500 frames): no
flashlight 16,579 ms (run 335b); flashlight 18,187 ms (run 337a, +3.2 ms a
frame); flashlight with the rewrite hold off 17,541 ms (run 337b, the hold
1.3 ms of it). Samples in the play stretch, 337a against 335b:
`rp_gfx_wait` +173, `R_MarkLights` +46, `RP_SurfLightmap` +29,
`RP_Upload` +17, the models' dynamic light (`R_EntityDynamicLight`,
`RP_StudioCmdLit`) about +45 to +80, of about 6,800.

## Fix 1: lightmap rewrites never wait (built)

| | What it does |
|---|---|
| GoldSrc GL / ref_gl | relights the lightmap on the CPU and uploads it every frame (`glTexSubImage2D`); the GL driver renames or copies a texture in use (memory we don't have) |
| ref_soft | relights the surface cache; draws serially, never meets the hazard |
| Dell's GLES, textbook | waits for the render before writing (our a68545bb hold: 1.3 ms) |
| **Ours** | a home not drawn by a frame still on the chip is written at once; otherwise the new lightmap goes to a 3-slot ring (frame mod 3, 128 KiB a slot at the texture pool's start), the home untouched. A dynamic light alone never rewrites a home: the plain lightmap stays there while the light is on it |
| Why it wins | no wait, no copy, no hazard; the light leaving costs nothing; 384 KiB of the 3 MiB pool (c2a5f's fight uses 320 KiB of it) |
| What overturns it | ring overflow counted (`no ring room`): if plays show it, a bigger slot |

## Fix 2 as briefed: dynamic lights as an additive 2700G pass (not built)

| | What it does |
|---|---|
| GoldSrc GL | CPU relight (above); the light clamped with the lightmap (one byte a channel) |
| Quake 3 / Doom 3 | an additive pass per light with a falloff texture; overbright allowed (their look) |
| ref_soft (ours today) | `R_AddDynamicLights`: octagonal distance (`max + min/2`) at luxel centres, linear `rad - dist`, added to the style light *before* the gamma table and the cap at level 31 |
| The brief | texture x light added with ONE, ONE "the same sum, the look unchanged" |

**The look is not unchanged.** The old path caps light plus lightmap at
level 31 (the texture at full brightness). The flashlight's centre alone
is 80 x 3,060 = 244,800, past the cap (196,605): its spot saturates by
itself. Added on top of the lit wall, the spot would reach up to twice
the texture's colour where the game shows the texture itself, a brighter,
washed-out spot on every lit wall. That trades away the agreed look.

The exact way exists: the pass draws D' = level(L + D) - level(L) per
luxel, so the two passes sum to the capped light (bilinear filtering is
linear, so it holds after filtering; within one step of the 565 screen).
But D' needs L at each luxel the light reaches, which is the same style
sum and gamma work as today's relight, over the light's box instead of
the whole face. The whole relight measured 46 samples of 6,778 (0.7%,
about 0.25 ms a frame); the exact pass could save a part of that and adds a
second chip pass for every lit face. Not worth a device run on its own.

**The flashlight's other costs, bigger and look-free (next):**
`R_MarkLights` (46 samples: the light walked through every visible brush
entity's tree, a float transform each) and the models' dynamic light
(45-80). Each is a candidate for the backlog, measured with
`-rpfight -rpflash`.
