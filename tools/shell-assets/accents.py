"""Mutes the apps' accent colours (apps/api/src/kit.rs, ADR-0021).

The owner, 2026-11-14: accents are welcome, but the bright ones clash with
the Aero / Windows XP look. Windows XP's Luna and Vista's Aero kept their
accents at moderate saturation under glass (Luna's Start green and
selection blue sit near half the chroma of today's neon UI tints); a
designer would pick each muted tint by eye. Ours computes them: every
accent keeps its hue, its chroma is scaled in OKLCH (a perceptual space, so
coral, lime and violet mute by the same amount as the eye sees it, which an
HSV saturation cut does not do), and its lightness is then set to the
lowest value that still reads at the contrast floor against the glass
cards, measured from the rendered screens. So the tints are as quiet as
the look wants and never fall below legibility.

Run: python -I tools/shell-assets/accents.py [strength] [--write]
  strength: the chroma kept, 0..1 (1 = the current colours).
  --write:  rewrites the colour constants in apps/api/src/kit.rs.
"""

import io
import math
import os
import re
import sys

KIT = os.path.join(os.path.dirname(__file__), "..", "..", "apps", "api", "src", "kit.rs")

# The accents as first chosen (2026-11-14), by name: the source of every
# muted set, so strengths never compound.
ORIGINAL = {
    "DONE": 0x5CE07A,
    "LATE": 0xFF7A6B,
    "AMBER": 0xFFC44D,
    "CALENDAR": 0xFF7A5C,
    "CONTACTS": 0x35C2B0,
    "TASKS": 0x4C9BFF,
    "NOTES": 0xFFC44D,
    "CALCULATOR": 0xFF9F2E,
    "ID0": 0xFF7A5C,
    "ID1": 0xFFC44D,
    "ID2": 0xA6E05A,
    "ID3": 0x35C2B0,
    "ID4": 0x4CB8FF,
    "ID5": 0x6C7BFF,
    "ID6": 0xB07CFF,
    "ID7": 0xFF6FA8,
    # Weather's (shell/core/src/weather_app.rs), muted the same way so the
    # whole system reads as one (the owner, 2026-11-14: "everything to
    # feel cohesive"); Weather's own tint is the sky.
    "WEATHER": 0x4CB8FF,
    "W_COLD": 0x4FA8FF,
    "W_WARM": 0xFFA040,
    "W_RAIN": 0x7FC8FF,
    "W_RAIN_BAR": 0x5C94DC,
    "W_CURVE": 0xF2B45A,
    "W_ALERT": 0xF2B45A,
}
WEATHER = os.path.join(os.path.dirname(__file__), "..", "..", "shell", "core", "src", "weather_app.rs")

# The glass card under the accents, measured from the rendered agenda
# (target/ui-preview/v2-calendar-agenda.png, card centres): the brightest
# of the samples, the hardest case for contrast.
GLASS = (64, 52, 56)
# Accents are bars, dots and short capital labels: WCAG's 3:1 for UI
# graphics and large text is the floor; 4.5:1 for the label text.
FLOOR = 4.5


def srgb_to_lin(c):
    c /= 255.0
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def lin_to_srgb(c):
    c = max(0.0, min(1.0, c))
    v = 12.92 * c if c <= 0.0031308 else 1.055 * c ** (1 / 2.4) - 0.055
    return round(v * 255)


def to_oklch(rgb):
    r, g, b = (srgb_to_lin(c) for c in rgb)
    l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b
    m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b
    s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b
    l, m, s = (math.copysign(abs(x) ** (1 / 3), x) for x in (l, m, s))
    L = 0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s
    a = 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s
    bb = 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s
    return L, math.hypot(a, bb), math.atan2(bb, a)


def from_oklch(L, C, h):
    a, b = C * math.cos(h), C * math.sin(h)
    l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3
    m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3
    s = (L - 0.0894841775 * a - 1.2914855480 * b) ** 3
    r = 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s
    g = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s
    bl = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s
    return lin_to_srgb(r), lin_to_srgb(g), lin_to_srgb(bl)


def luminance(rgb):
    r, g, b = (srgb_to_lin(c) for c in rgb)
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def contrast(a, b):
    la, lb = sorted((luminance(a), luminance(b)), reverse=True)
    return (la + 0.05) / (lb + 0.05)


def mute(hex_rgb, strength):
    rgb = ((hex_rgb >> 16) & 255, (hex_rgb >> 8) & 255, hex_rgb & 255)
    L, C, h = to_oklch(rgb)
    C *= strength
    # The lowest lightness (no brighter than the original) that keeps the
    # floor: muting brightness too is what takes the neon out.
    best = from_oklch(L, C, h)
    lo = L
    while lo > 0.3:
        cand = from_oklch(lo - 0.005, C, h)
        if contrast(cand, GLASS) < FLOOR:
            break
        lo -= 0.005
        best = cand
    # Lightness lowered only part way: halfway between the original and
    # the floor keeps the tint glowing on glass instead of dull.
    mid = from_oklch((L + lo) / 2, C, h)
    return mid if contrast(mid, GLASS) >= FLOOR else best


# The light look's glass at its darkest, measured from the rendered app
# screens in the light look (target/ui-preview/v2-*.png, card centres,
# 2026-11-14): the hardest case for a dark accent's contrast.
LIGHT_GLASS = (144, 148, 144)


def in_gamut(L, C, h):
    """The colour at L and h with the most chroma up to C that sRGB holds
    (a dark colour keeps its colour instead of clipping to another hue)."""
    while C > 0:
        a, b = C * math.cos(h), C * math.sin(h)
        l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3
        m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3
        s_ = (L - 0.0894841775 * a - 1.2914855480 * b) ** 3
        r = 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s_
        g = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s_
        bl = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s_
        if min(r, g, bl) >= -1e-4 and max(r, g, bl) <= 1 + 1e-4:
            break
        C -= 0.002
    return from_oklch(L, max(C, 0), h)


def light_twin(rgb, floor):
    """The accent for the light look: its hue and chroma kept in OKLCH
    (uniform RGB scaling, the first way, turned coral brown and blue
    black), its lightness lowered until it reads at `floor` against the
    light glass at its darkest."""
    L, C, h = to_oklch(rgb)
    while L > 0.05:
        cand = in_gamut(L, C, h)
        if contrast(cand, LIGHT_GLASS) >= floor:
            return cand
        L -= 0.005
    return in_gamut(0.05, C, h)


def palette(strength):
    return {k: mute(v, strength) for k, v in ORIGINAL.items()}


def rust(rgb):
    r, g, b = rgb
    return f"0xFF{r:02X}_{g:02X}{b:02X}"


def write(p):
    s = io.open(KIT, encoding="utf-8", newline="").read()
    for name in ("DONE", "LATE", "AMBER"):
        s, n = re.subn(
            rf"(pub const {name}: Argb = theme\()0xFF[0-9A-F_]+(\);)", rf"\g<1>{rust(p[name])}\2", s
        )
        assert n == 1, name
    for name in ("CALENDAR", "CONTACTS", "TASKS", "NOTES", "CALCULATOR", "WEATHER"):
        s, n = re.subn(
            rf"(    pub const {name}: Argb = theme\()0xFF[0-9A-F_]+(\);)", rf"\g<1>{rust(p[name])}\2", s
        )
        assert n == 1, name
    i = s.index("pub const IDENTITY: [Argb; 8] = [")
    j = s.index("];", i)
    body = "".join(f"    theme({rust(p[f'ID{k}'])}),\n" for k in range(8))
    s = s[:i] + "pub const IDENTITY: [Argb; 8] = [\n" + body + s[j:]
    # The light look's twins of every accent: (dark look, fill 3:1, text
    # 4.5:1); the shell's look pass reads them (rp_app::kit::light_twin).
    seen = []
    for v in p.values():
        if v not in seen:
            seen.append(v)
    rows = "".join(
        f"    (theme({rust(v)}), theme({rust(light_twin(v, 3.0))}), theme({rust(light_twin(v, 4.5))})),\n"
        for v in seen
    )
    i = s.index("pub const LIGHT_TWINS: [(Argb, Argb, Argb); ")
    j = s.index("];", i)
    s = s[:i] + f"pub const LIGHT_TWINS: [(Argb, Argb, Argb); {len(seen)}] = [\n" + rows + s[j:]
    io.open(KIT, "w", encoding="utf-8", newline="\n").write(s)
    w = io.open(WEATHER, encoding="utf-8", newline="").read()
    for name in ("COLD", "WARM", "RAIN", "RAIN_BAR", "CURVE", "ALERT"):
        w, n = re.subn(
            rf"(const {name}: Argb = (?:rp_draw::look::theme\()?)0xFF[0-9A-F_]+",
            rf"\g<1>{rust(p['W_' + name])}",
            w,
        )
        assert n == 1, name
    io.open(WEATHER, "w", encoding="utf-8", newline="\n").write(w)


if __name__ == "__main__":
    strength = float(sys.argv[1]) if len(sys.argv) > 1 and sys.argv[1] != "--write" else 0.6
    p = palette(strength)
    for k, v in p.items():
        o = ORIGINAL[k]
        orig = ((o >> 16) & 255, (o >> 8) & 255, o & 255)
        print(
            f"{k:11} {rust(orig)} -> {rust(v)}  contrast {contrast(orig, GLASS):.1f} -> {contrast(v, GLASS):.1f}"
            f"  light: fill {rust(light_twin(v, 3.0))} text {rust(light_twin(v, 4.5))}"
        )
    if "--write" in sys.argv:
        write(p)
        print("written:", KIT)
