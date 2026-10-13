"""Converts a photo into the video plane's wallpaper (ADR-0021).

    python -I tools/shell-assets/wallpaper.py <photo.jpg> <out.yuv>

Crops the middle 3:4 (portrait) of the photo, scales it to 480 x 640 and
writes planar 4:2:0: the Y plane, then U, then V (each a quarter of Y),
BT.601 studio range with the same integer formulas as
`rp_drv_mbx_display::scanout::rgb_to_yuv`, chroma averaged over each 2 x 2
block and rounded as `blendtest` does. 460,800 bytes.

Run with `-I` (isolated): the photo is downloaded data. The output goes in
`local/`, never committed; the build falls back to a generated sky.
"""

import sys

from PIL import Image

W, H = 480, 640


def yuv(r, g, b):
    y = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16
    u = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128
    v = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128
    return y, u, v


def main():
    src, out = sys.argv[1], sys.argv[2]
    img = Image.open(src).convert("RGB")
    w, h = img.size
    # The largest 3:4 portrait window, centred.
    if w * 4 > h * 3:
        cw, ch = h * 3 // 4, h
    else:
        cw, ch = w, w * 4 // 3
    left, top = (w - cw) // 2, (h - ch) // 2
    img = img.crop((left, top, left + cw, top + ch)).resize((W, H), Image.LANCZOS)
    px = img.load()
    ys = bytearray(W * H)
    us = bytearray(W * H // 4)
    vs = bytearray(W * H // 4)
    for y in range(0, H, 2):
        for x in range(0, W, 2):
            su, sv = 2, 2  # round to nearest, as blendtest
            for dx, dy in ((0, 0), (1, 0), (0, 1), (1, 1)):
                l, cu, cv = yuv(*px[x + dx, y + dy])
                ys[(y + dy) * W + x + dx] = l
                su += cu
                sv += cv
            us[y // 2 * (W // 2) + x // 2] = su // 4
            vs[y // 2 * (W // 2) + x // 2] = sv // 4
    with open(out, "wb") as f:
        f.write(ys + us + vs)
    print(f"{out}: {len(ys) + len(us) + len(vs)} bytes (crop {cw}x{ch} at {left},{top})")


if __name__ == "__main__":
    main()
