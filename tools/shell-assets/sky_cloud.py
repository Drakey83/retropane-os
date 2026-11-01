"""Makes the living sky's clouds (shell/core/src/sky-cloud.alpha, ADR-0021).

Eight whole clouds, four long and four small or double, each a soft, uneven mass with wispy edges, lit from
above, to sit with VClouds' realistic pictures (the owner, 2026-11-01: the
lobed first try was "too cartoony"; one cloud of this kind "looks much
better"; clouds built from puffs "look like cobbled together smaller
clouds"): fractal value noise, its sampling bent by more noise (billows,
not blobs), inside a soft envelope of one to three lumps with a flatter,
fainter base, the domes inside brighter; nothing at the picture's border.
Eight shapes (long: low banks, a two-part and a three-part bank; small:
tufts, doubles, a little bank), so the clouds on screen together are never alike. They don't fade: they drift in and out under the weather
panel's edges, as clouds cross a window.

One coverage byte a pixel, w x h, row by row, the clouds one after another
(the long first). Made once, offline (fixed seeds): the device stores 32
KB and computes nothing.

Run: python tools/shell-assets/sky_cloud.py
"""

import math
import os

SEED = 20261101

# Each cloud: its size (w, h), its noise salt, its envelope as lumps (centre
# x, centre y as fractions; half-width; half-height above, below), how
# dense. Four long, four small or double (the owner, 2026-11-01: "small
# ones can work along side bigger ones"; "make 3 of the clouds long as the
# 2 you have now, and 3 small or double ... or 4").
CLOUDS = [
    ((128, 40), 11, [(0.5, 0.62, 0.46, 0.40, 0.24)], 1.0),
    ((128, 40), 51, [(0.5, 0.60, 0.48, 0.30, 0.20)], 0.95),
    ((128, 40), 67, [(0.3, 0.64, 0.26, 0.40, 0.22), (0.68, 0.6, 0.30, 0.44, 0.22)], 1.0),
    ((128, 40), 37, [(0.22, 0.64, 0.2, 0.36, 0.22), (0.5, 0.58, 0.22, 0.46, 0.22), (0.78, 0.64, 0.2, 0.34, 0.22)], 1.0),
    ((80, 32), 79, [(0.5, 0.6, 0.38, 0.44, 0.26)], 1.1),
    ((80, 32), 97, [(0.33, 0.62, 0.26, 0.46, 0.24), (0.68, 0.58, 0.26, 0.48, 0.24)], 1.05),
    ((80, 32), 101, [(0.5, 0.58, 0.40, 0.36, 0.24)], 1.05),
    ((80, 32), 113, [(0.38, 0.6, 0.30, 0.50, 0.26), (0.7, 0.66, 0.22, 0.36, 0.22)], 1.1),
]

SALT = [0]
# The noise's feature size (px): smaller clouds, finer billows.
SCALE = [22.0]


def lattice(ix, iy, octave):
    h = (ix * 374761393 + iy * 668265263 + octave * 2147483647 + SEED + SALT[0]) & 0xFFFFFFFF
    h = ((h ^ (h >> 13)) * 1274126177) & 0xFFFFFFFF
    return ((h ^ (h >> 16)) & 0xFFFF) / 65535.0


def value(x, y, octave):
    ix, iy = math.floor(x), math.floor(y)
    fx, fy = x - ix, y - iy
    sx, sy = fx * fx * (3 - 2 * fx), fy * fy * (3 - 2 * fy)
    a, b = lattice(ix, iy, octave), lattice(ix + 1, iy, octave)
    c, d = lattice(ix, iy + 1, octave), lattice(ix + 1, iy + 1, octave)
    return (a + (b - a) * sx) + ((c + (d - c) * sx) - (a + (b - a) * sx)) * sy


def fbm(x, y, base, octaves=5):
    total, amp, freq, norm = 0.0, 1.0, 1.0, 0.0
    for o in range(octaves):
        total += amp * value(x * freq / SCALE[0], y * freq / SCALE[0], base + o)
        norm += amp
        amp *= 0.55
        freq *= 2.03
    return total / norm


def smoothstep(a, b, v):
    t = min(1.0, max(0.0, (v - a) / (b - a)))
    return t * t * (3 - 2 * t)


def cloud(W, H, lumps, density):
    out = bytearray()
    for y in range(H):
        for x in range(W):
            # Billows: the noise read through a field bent by noise.
            wx = x + 9.0 * (fbm(x, y, 40, 3) - 0.5)
            wy = y + 6.0 * (fbm(x, y, 80, 3) - 0.5)
            n = fbm(wx, wy, 0)
            env = 0.0
            for (fx, fy, hw, up, down) in lumps:
                dx = (x - W * fx) / (W * hw)
                dy = (y - H * fy) / (H * (up if y < H * fy else down))
                env = max(env, math.exp(-2.2 * (dx * dx + dy * dy)))
            d = env * 1.05 * density + (n - 0.5) * 1.1 - 0.2
            c = smoothstep(0.05, 0.75, d)
            # Inside, billows: brighter domes and softer hollows, lit from
            # above.
            dome = fbm(wx * 1.6, wy * 1.6 - 3.0, 120, 4)
            shade = 0.55 + 0.6 * smoothstep(0.35, 0.75, dome)
            lit = 1.0 - 0.42 * (y / H)
            # To nothing at the picture's border: no wisp cut off.
            ex = min(x + 1, W - x) / 12.0
            ey = min(y + 1, H - y) / 8.0
            edge = min(1.0, ex) * min(1.0, ey)
            out.append(int(round(min(1.0, 235 / 255 * c * shade * lit * edge) * 255)))
    return out


def main():
    out = bytearray()
    for (w, h), salt, lumps, density in CLOUDS:
        SALT[0] = salt * 7919
        SCALE[0] = 22.0 * h / 44
        out += cloud(w, h, lumps, density)
    path = os.path.join(os.path.dirname(__file__), "..", "..", "shell", "core", "src", "sky-cloud.alpha")
    with open(os.path.normpath(path), "wb") as f:
        f.write(out)
    print(f"sky-cloud.alpha: {len(CLOUDS)} clouds, {len(out)} bytes")


if __name__ == "__main__":
    main()
