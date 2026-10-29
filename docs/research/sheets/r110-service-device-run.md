# Sheet: the graphics service on the X51v (R110, test), 2026-10-29

**What it decides.** Whether the graphics service itself (not a bench
harness) drives a 3D scene tear-free on the device: the scan race, the
copy fallback, the bottom-row start, with its counters (`starts()`,
`copy_stalls()`, `last_copy_line`) logged. No image drives a 3D scene
through `rp-graphics` today (`rp-today` is 2D only), so it needs one.

## 1. Original way (Windows Mobile / Dell's product)
No such test in Dell's code: no `DSIG` read anywhere (REA log #13, #14,
#21). Windows Mobile was tested by use.

## 2. Dell's way (REA)
Searched local/re for a self-test of the display or 3D path:
`0x2184|,0x184[,)]` (the signature) in `ddi_2700g`, `libGLES_CL`,
`d3d_2700g`: no Dell function found. The only frame-timing reads are
`DVLNUM` in GetScanLine 0x02125b64, the plane-off wait 0x0211fe70 and the
mode switch 0x021273d4 (REA log #7).

## 3. Modern way
An on-device integration test of the compositor with per-refresh scan-out
CRCs and the driver's own counters (DRM/IGT kms tests). Training
knowledge, a claim.

## 4. The alternatives, and why ours wins
| | What | Wins | Loses |
|---|---|---|---|
| A. Do nothing | Trust the proof run (rp-gpubench) for the policy | No new image | The service's own code (its jobs, the 2D/3D hand-offs, the held 2D frame) never runs on the device: "proven on hardware" unmet |
| B. Fold it into the proof run | rp-gpubench calls `rp-graphics` directly | One image | gpubench's harness and the service's frame loop mixed; a fault is harder to place |
| **C. Ours** | A small image (`rp-scene`): a 3D scene through `rp-graphics` with 2D changes beside it, at 149.50 / 2, the four game frames' meshes, every refresh's `DSIG` against references, the counters logged on the card (rp-cardlog) | The shipped code proven as shipped | A new image to build and keep |

## 5. What would overturn each, and the test
- **C overturned** if the proof run already fails the copy (C93): this run
  waits for it.
- **A's risk shown** if C finds a tear or stall the proof run didn't.
- **The test:** 600 frames, each policy's share counted (`behind`,
  `after_area`, `screen1`), torn refreshes by `DSIG`, `copy_stalls`, R96's
  failed/hung/undrawn counts.

**Asked of the owner:** the go to build the image after the proof run's
result; a separate go for the device run.
