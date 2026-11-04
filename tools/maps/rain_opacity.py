"""The rain's opacity per map look (R184 step 2, M6), fitted, not picked.

RainViewer's frames carry their own alpha (light drizzle 25-190, heavy rain
255). On the dark map (Night) that reads well; on the light maps (Paper,
Ink) see-through green and blue are weak. Each look gets a factor k (in
256ths) on the rain's alpha, alpha' = min(255, (alpha * k + 128) >> 8),
fitted so the rain's mean colour difference from the land (CIE DeltaE
2000) reaches the same share of what that map allows (every rain colour
opaque) on each light map as on Night at k = 256 (today's rain, the
owner's approved look). Equal DeltaE itself is out of reach: RainViewer's
greens and yellows are light, so on Paper even opaque rain stands out less
(26.7) than Night's today (26.3 of 42.1) once its blues are counted
(measured by this script, 2026-11-04).

The rain's colours: every colour of RainViewer's colour scheme 2 seen in
the test tile (shell/weather/tests/data/rainviewer-7-29-52.png), weighted
equally. The land: each look's land colour at the shade layer's middle
(128), as rp_weather::maplayers colours it.

    python -I tools/maps/rain_opacity.py

prints the factors; rp_weather::maplayers::RAIN holds them, and its test
re-fits them (the same arithmetic in Rust) to within one step.
"""

import math
import sys
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
TILE = ROOT / "shell/weather/tests/data/rainviewer-7-29-52.png"

# rp_weather::maplayers' palettes: land, lo, span.
LOOKS = {
    "Night": (0x22282E, 141, 230),
    "Paper": (0xE2DDD2, 200, 87),
    "Ink": (0xECEAE4, 215, 56),
}


def land(look):
    c, lo, span = LOOKS[look]
    lit = lo + span * 128 // 255
    return tuple(min(255, ((c >> s) & 0xFF) * lit // 256) for s in (16, 8, 0))


def over(m, c, a):
    return tuple((mv * (255 - a) + cv * a + 127) // 255 for mv, cv in zip(m, c))


def lab(rgb):
    def lin(v):
        v /= 255
        return v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4

    r, g, b = (lin(v) for v in rgb)
    x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047
    y = 0.2126 * r + 0.7152 * g + 0.0722 * b
    z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883

    def f(t):
        return t ** (1 / 3) if t > 216 / 24389 else (24389 / 27 * t + 16) / 116

    fx, fy, fz = f(x), f(y), f(z)
    return 116 * fy - 16, 500 * (fx - fy), 200 * (fy - fz)


def de2000(p, q):
    l1, a1, b1 = lab(p)
    l2, a2, b2 = lab(q)
    c1, c2 = math.hypot(a1, b1), math.hypot(a2, b2)
    cm = (c1 + c2) / 2
    g = 0.5 * (1 - math.sqrt(cm**7 / (cm**7 + 25**7)))
    a1, a2 = a1 * (1 + g), a2 * (1 + g)
    c1, c2 = math.hypot(a1, b1), math.hypot(a2, b2)
    h1 = math.degrees(math.atan2(b1, a1)) % 360
    h2 = math.degrees(math.atan2(b2, a2)) % 360
    dl, dc = l2 - l1, c2 - c1
    dh = 0.0
    if c1 * c2:
        dh = h2 - h1
        if dh > 180:
            dh -= 360
        elif dh < -180:
            dh += 360
    dhh = 2 * math.sqrt(c1 * c2) * math.sin(math.radians(dh / 2))
    lm, cm = (l1 + l2) / 2, (c1 + c2) / 2
    hm = h1 + h2
    if c1 * c2:
        if abs(h1 - h2) > 180:
            hm += 360 if h1 + h2 < 360 else -360
        hm /= 2
    t = (
        1
        - 0.17 * math.cos(math.radians(hm - 30))
        + 0.24 * math.cos(math.radians(2 * hm))
        + 0.32 * math.cos(math.radians(3 * hm + 6))
        - 0.20 * math.cos(math.radians(4 * hm - 63))
    )
    sl = 1 + 0.015 * (lm - 50) ** 2 / math.sqrt(20 + (lm - 50) ** 2)
    sc = 1 + 0.045 * cm
    sh = 1 + 0.015 * cm * t
    rt = (
        -2
        * math.sqrt(cm**7 / (cm**7 + 25**7))
        * math.sin(math.radians(60 * math.exp(-(((hm - 275) / 25) ** 2))))
    )
    return math.sqrt(
        (dl / sl) ** 2 + (dc / sc) ** 2 + (dhh / sh) ** 2 + rt * (dc / sc) * (dhh / sh)
    )


def rain_colours():
    im = Image.open(TILE).convert("RGBA")
    return sorted({p for p in im.get_flattened_data() if p[3]})


def contrast(look, k, colours):
    m = land(look)
    return sum(
        de2000(m, over(m, c[:3], min(255, (c[3] * k + 128) >> 8))) for c in colours
    ) / len(colours)


OPAQUE = 1 << 16


def share(look, k, colours):
    return contrast(look, k, colours) / contrast(look, OPAQUE, colours)


def fit(look, colours, target):
    return min(range(256, 1024), key=lambda k: abs(share(look, k, colours) - target))


def main():
    colours = rain_colours()
    target = share("Night", 256, colours)
    print(f"{len(colours)} rain colours; Night's share of its most at k 256: {target:.3f}")
    for look in ("Paper", "Ink"):
        k = fit(look, colours, target)
        print(
            f"{look}: share {share(look, 256, colours):.3f} at k 256; k {k} gives "
            f"{share(look, k, colours):.3f} (mean DeltaE {contrast(look, 256, colours):.2f} "
            f"-> {contrast(look, k, colours):.2f})"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
