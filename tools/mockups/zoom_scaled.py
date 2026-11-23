"""Calendar zoom B for the owner's pick (calendar-rule0.md, group E): the
month's picture enlarged about the chosen day by the 2700G's 2D engine
scale word, with the X51v's measured arithmetic (rp-mbx-blend B9,
docs/hw/2700g.md: nearest pixel, source `(i * s) >> 5`, `s = 32 x src /
dst` in whole 32nds), frames 1-3 of 6 on the motion's ease table.

    zoom_scaled.py <month.png> <out prefix>

The month PNG is ui-preview's (the device's own renderer code); only the
scale is done here, as the engine does it."""
import sys
from PIL import Image

EASE = [0, 108, 180, 224, 247, 255, 256]
src = Image.open(sys.argv[1]).convert('RGB')
px = src.load()
PAGE = (0, 36, 480, 564)        # the app's content area
CELL = (80, 166, 64, 46)        # Monday 1 January's cell (tests check it)


def lerp(a, b, k):
    return a + (b - a) * EASE[k] // 256


for k in (1, 2, 3):
    # The month's part shown: the page shrinking toward the cell.
    w = lerp(PAGE[2], CELL[2], k)
    h = lerp(PAGE[3], CELL[3], k)
    # The engine's steps: whole 32nds of a source pixel per screen pixel.
    sx = max(1, 32 * w // PAGE[2])
    sy = max(1, 32 * h // PAGE[3])
    w, h = PAGE[2] * sx // 32, PAGE[3] * sy // 32
    cx = lerp(PAGE[0] + PAGE[2] // 2, CELL[0] + CELL[2] // 2, k)
    cy = lerp(PAGE[1] + PAGE[3] // 2, CELL[1] + CELL[3] // 2, k)
    x0 = min(max(cx - w // 2, 0), 480 - w)
    y0 = min(max(cy - h // 2, PAGE[1]), PAGE[1] + PAGE[3] - h)
    out = src.copy()
    o = out.load()
    for j in range(PAGE[3]):
        sy_ = y0 + ((j * sy) >> 5)
        for i in range(PAGE[2]):
            o[PAGE[0] + i, PAGE[1] + j] = px[x0 + ((i * sx) >> 5), sy_]
    out.save(f'{sys.argv[2]}-{k}.png')
    print(f'frame {k}: source {w} x {h} at ({x0}, {y0}), s = {sx}/32 across, {sy}/32 down')

# Zoom D: the arriving view's own picture (Day, drawn once, kept) shrunk
# into the growing card over the month, by the same scaler, each frame one
# scaled copy into the screen's card (565 to 565: no dither or conversion
# pass). Priced at the scaler's measured rate (a full-screen 2x in 18.7 ms:
# ~16.4 Mpx/s) plus ~6 us a command.
if len(sys.argv) > 3:
    day = Image.open(sys.argv[3]).convert('RGB')
    dp = day.load()
    for k in range(1, 7):
        x = lerp(CELL[0], PAGE[0], k)
        y = lerp(CELL[1], PAGE[1], k)
        w = lerp(CELL[2], PAGE[2], k)
        h = lerp(CELL[3], PAGE[3], k)
        sx = max(1, min(32 * 32, 32 * PAGE[2] // w))
        sy = max(1, min(32 * 32, 32 * PAGE[3] // h))
        out = src.copy() if k < 6 else day.copy()
        o = out.load()
        if k < 6:
            for j in range(h):
                syy = PAGE[1] + min(PAGE[3] - 1, (j * sy) >> 5)
                for i in range(w):
                    o[x + i, y + j] = dp[PAGE[0] + min(PAGE[2] - 1, (i * sx) >> 5), syy]
        out.save(f'{sys.argv[2]}-d-{k}.png')
        us = 6 + w * h / 16.4
        print(f'zoom D frame {k}: card {w} x {h} at ({x}, {y}), s = {sx}/32, {sy}/32: priced {us / 1000:.1f} ms')
