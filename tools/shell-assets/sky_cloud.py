"""Makes the living sky's cloud (shell/core/src/sky-cloud.alpha, ADR-0021).

A soft, uneven cloud mass to sit with VClouds' realistic pictures (the
owner, 2026-11-01: the first, lobed one looked "too cartoony"): fractal
value noise, its sampling bent by more noise (billows rather than blobs),
inside a soft wide envelope with a flatter, fainter base; wispy edges where
the density thins; lit from above (more coverage at the top, so brighter
by night and denser by day). Four different clouds (the owner: "it cant
just be one cloud over and over, we need like 3 or 4 variations"): a long
low bank, a taller heap, two lumps, a thin wisp. One coverage byte a pixel,
W x H, row by row, the four one after another; the image makes its fade
strengths from them at build time. 112 x 40 each, so four clouds at four
strengths fit the 2D engine's glyph memory with the screen's fonts (~290 of
512 KB). Made once, offline (fixed seeds): the device stores 18 KB and
computes nothing.

Run: python tools/shell-assets/sky_cloud.py
"""

import math
import os

W, H = 112, 40
SEED = 20261101

# Each cloud: its noise seed, its envelope as lumps (centre x, centre y as
# fractions, half-width, half-height above, half-height below), how dense.
CLOUDS = [
    (11, [(0.5, 0.6, 0.52, 0.46, 0.30)], 1.05),
    (23, [(0.5, 0.62, 0.38, 0.62, 0.32)], 1.1),
    (37, [(0.31, 0.62, 0.32, 0.52, 0.28), (0.69, 0.56, 0.32, 0.56, 0.28)], 1.05),
    (51, [(0.5, 0.56, 0.54, 0.32, 0.24)], 0.9),
]


SALT = [0]


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
        total += amp * value(x * freq / 22.0, y * freq / 22.0, base + o)
        norm += amp
        amp *= 0.55
        freq *= 2.03
    return total / norm


def smoothstep(a, b, v):
    t = min(1.0, max(0.0, (v - a) / (b - a)))
    return t * t * (3 - 2 * t)


def cloud(lumps, density):
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
            # A lumpier top: the envelope lifted where the noise is high.
            d = env * 1.05 * density + (n - 0.5) * 1.1 - 0.2
            c = smoothstep(0.05, 0.75, d)
            # Inside, billows: brighter domes and softer hollows, lit from
            # above (each dome's top catching more light).
            dome = fbm(wx * 1.6, wy * 1.6 - 3.0, 120, 4)
            shade = 0.55 + 0.6 * smoothstep(0.35, 0.75, dome)
            lit = 1.0 - 0.42 * (y / H)
            # To nothing at the picture's border: no wisp cut off.
            ex = min(x + 1, W - x) / 10.0
            ey = min(y + 1, H - y) / 7.0
            edge = min(1.0, ex) * min(1.0, ey)
            out.append(int(round(min(1.0, 235 / 255 * c * shade * lit * edge) * 255)))
    return out


def main():
    out = bytearray()
    for salt, lumps, density in CLOUDS:
        SALT[0] = salt * 7919
        out += cloud(lumps, density)
    path = os.path.join(os.path.dirname(__file__), "..", "..", "shell", "core", "src", "sky-cloud.alpha")
    with open(os.path.normpath(path), "wb") as f:
        f.write(out)
    print(f"sky-cloud.alpha: {len(CLOUDS)} clouds of {W} x {H}, {len(out)} bytes")


if __name__ == "__main__":
    main()
