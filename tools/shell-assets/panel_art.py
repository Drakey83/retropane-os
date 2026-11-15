"""Makes the glass panels' art (shell/core/src/panel_art.rs, ADR-0021).

The owner's pick (2026-11-01): design B, "Glass", with the frosted grey of
his glassmorphism reference in the light look (docs/design/mockups/
glass-b-grey-*.png), dark frosted glass in the dark look. Each look is one
`rp_draw::PanelArt`: the body's colour from the top row to the bottom (a
light sheen at the top, the frosted grey darkening downward), the straight
rim's rows and columns (a crisp bright line, brightest from the top left),
and the four rounded corners as images, all computed here once from the
mock-up's model (a signed distance to a rounded rectangle, 4x supersampled
at the corners); the X51v draws a panel of any size from it with fills and
four small images, nothing computed per frame.

Run: python tools/shell-assets/panel_art.py
"""

import math
import os

OUT = os.path.join(os.path.dirname(__file__), "..", "..", "shell", "core", "src", "panel_art.rs")

K = 12  # the corners' radius (px)
RIM = 2.2  # the rim's width (px)

LOOKS = {
    "LIGHT": dict(top=(176, 178, 183), bottom=(140, 142, 148), alpha=0.93, rim=0.85, sheen=0.26),
    "DARK": dict(top=(64, 66, 74), bottom=(40, 42, 48), alpha=0.90, rim=0.65, sheen=0.18),
}
# A pressed panel (the weather card under the pen): the same glass a shade
# darker, words and pictures kept (a dark fill over it replaced them all,
# the card gone for the press, the owner's video of 2026-11-01).
LOOKS["LIGHT_PRESSED"] = dict(LOOKS["LIGHT"], dim=0.80)
LOOKS["DARK_PRESSED"] = dict(LOOKS["DARK"], dim=0.70)
# The Aero buttons (design-language.md version 2, the owner 2026-11-14:
# "Aero / Windows XP", not Metro's flat colour): an 8 px corner for chips
# and keys, and the classic gloss: a bright upper half, a hard line at the
# middle, a deeper lower half, a bright rim. KEY is a chip or key at rest,
# KEY_DOWN under the pen, ON the chosen one (the blue glow of XP and Vista
# selection).
LOOKS["KEY"] = dict(top=(92, 98, 112), bottom=(30, 34, 42), alpha=0.92, rim=0.75, sheen=0.0, gloss=0.30, k=8)
LOOKS["KEY_DOWN"] = dict(LOOKS["KEY"], dim=0.65, gloss=0.12)
LOOKS["ON"] = dict(top=(120, 196, 246), bottom=(22, 92, 176), alpha=0.95, rim=0.85, sheen=0.0, gloss=0.38, k=8)
# Window glass (design-language.md version 3, 2026-11-14): the glass of a
# card inside an app's own window, over its backdrop of light, not over a
# photo. Vista's Aero glass was thin (the desktop showed through, a bright
# rim and a soft sheen gave it its edge); the wallpaper's glass above is
# near opaque because words had to read over any photo. Over the calm
# backdrop the glass can be Aero's: two thirds opaque in the dark so the
# light glows through it, a brighter rim; in the light look white frost
# with a cool darker edge (a white rim on white glass has no edge).
LOOKS["WINDOW_DARK"] = dict(top=(86, 92, 108), bottom=(20, 24, 32), alpha=0.64, rim=0.95, sheen=0.24)
LOOKS["WINDOW_DARK_PRESSED"] = dict(LOOKS["WINDOW_DARK"], dim=0.70)
LOOKS["WINDOW_LIGHT"] = dict(
    # Soft, not white (the owner on the X51v, 2026-11-15: light mode "WAY
    # too bright ... doesnt have to mean blinding mode"; was 255 / 240-250
    # at 0.72).
    # Then 226-204, "still seems too blinding": set by measurement, the
    # glass near 40% of white's light over its backdrop (aurora.rs).
    top=(190, 194, 201), bottom=(170, 176, 186), alpha=0.68, rim=0.55, sheen=0.0, edge=(78, 90, 110)
)
LOOKS["WINDOW_LIGHT_PRESSED"] = dict(LOOKS["WINDOW_LIGHT"], dim=0.88)
# The light look's gel key: white gloss over a cool foot, the same edge.
LOOKS["KEY_LIGHT"] = dict(
    top=(186, 191, 199), bottom=(146, 154, 168), alpha=0.94, rim=0.6, sheen=0.0, gloss=0.3, k=8, edge=(72, 84, 104)
)
LOOKS["KEY_LIGHT_DOWN"] = dict(LOOKS["KEY_LIGHT"], dim=0.86, gloss=0.2)


def smooth(e0, e1, v):
    t = min(1.0, max(0.0, (v - e0) / (e1 - e0)))
    return t * t * (3 - 2 * t)


def body(look, f):
    """The body's colour at fraction f down the panel (0 top, 1 bottom):
    the frosted grey, a sheen near the top; a pressed panel's darker
    (`dim`)."""
    t, b = look["top"], look["bottom"]
    c = [t[i] + (b[i] - t[i]) * f for i in range(3)]
    g = look["sheen"] * math.exp(-(((f - 0.05) / 0.25) ** 2))
    # The Aero gloss: the upper half lifted toward white, cut sharply at
    # the middle (a slight fade into the line keeps it from banding).
    gl = look.get("gloss", 0.0)
    if gl and f < 0.48:
        g += gl * (1 - f / 0.48 * 0.4)
    return [(c[i] * (1 - g) + 255 * g) * look.get("dim", 1.0) for i in range(3)]


def rim_at(look, inward, lit):
    """The rim's strength `inward` px inside the edge, lit by `lit`."""
    return (1 - smooth(0, RIM, inward)) * lit * look["rim"]


def argb(rgb, a):
    a = max(0, min(255, round(a * 255)))
    r, g, b = (max(0, min(255, round(v))) for v in rgb)
    return (a << 24) | (r << 16) | (g << 8) | b


def mix(rgb, w, to=(255, 255, 255)):
    """`rgb` toward `to` (white: a lit rim; a look's `edge`: a drawn one)."""
    return [rgb[i] * (1 - w) + to[i] * w for i in range(3)]


def corner(look, k):
    """Corner k (0 top left, 1 top right, 2 bottom left, 3 bottom right),
    K x K, from a rounded rectangle of radius K (the look's own `k`)."""
    K = look.get("k", globals()["K"])
    ss = 4
    px = []
    left = k % 2 == 0
    top = k < 2
    for y in range(K):
        for x in range(K):
            acc_rgb = [0.0, 0.0, 0.0]
            acc_a = 0.0
            for sy in range(ss):
                for sx in range(ss):
                    fx = x + (sx + 0.5) / ss
                    fy = y + (sy + 0.5) / ss
                    # Distance to the corner's arc (inside negative).
                    cx = K - fx if left else fx
                    cy = K - fy if top else fy
                    d = math.hypot(cx, cy) - K if cx > 0 and cy > 0 else max(cx, cy) - K
                    cover = max(0.0, min(1.0, 0.5 - d))
                    if cover <= 0:
                        continue
                    # The body's own row at this height (rp_draw::panel_cmds
                    # draws the corners' bands with the art's first and last
                    # K rows as they are), rounded as the rows are.
                    f = (y if top else 63 - (K - 1 - y)) / 63
                    rgb = shown(body(look, f))
                    lit = 0.55 + 0.45 * max(0.0, min(1.0, 1 - (0 if left else 0.5) - (0 if top else 0.7)))
                    rgb = mix(rgb, rim_at(look, -d, lit), look.get("edge", (255, 255, 255)))
                    a = cover * look["alpha"]
                    for i in range(3):
                        acc_rgb[i] += rgb[i] * a
                    acc_a += a
            n = ss * ss
            a = acc_a / n
            rgb = [acc_rgb[i] / acc_a for i in range(3)] if acc_a > 0 else [0, 0, 0]
            px.append(argb(rgb, a))
    return px


def shown(rgb):
    """A body colour rounded to what the X51v's RGB565 screen shows (red
    and blue in steps of 8, green of 4): rows a step apart look the same
    there, and like rows join into one fill (R176: ~60 fills a panel to
    ~10-15, the look unchanged)."""
    step = (8, 4, 8)
    return [min(255, round(v / step[i]) * step[i]) for i, v in enumerate(rgb)]


def art(name, look):
    K = look.get("k", globals()["K"])
    rows = [argb(shown(body(look, i / 63)), look["alpha"]) for i in range(64)]
    top_lit, bottom_lit, side_lit = 0.8, 0.57, 0.73
    edge = look.get("edge", (255, 255, 255))
    top = [argb(mix(body(look, 0.0), rim_at(look, y + 0.5, top_lit), edge), look["alpha"]) for y in range(2)]
    bottom = [argb(mix(body(look, 1.0), rim_at(look, 1.5 - y, bottom_lit), edge), look["alpha"]) for y in range(2)]
    side = argb(mix(body(look, 0.5), rim_at(look, 0.5, side_lit), edge), look["alpha"])
    corners = [corner(look, k) for k in range(4)]
    out = []
    for k, c in enumerate(corners):
        out.append(f"static {name}_C{k}_PX: [Argb; {K * K}] = [")
        for i in range(0, len(c), 6):
            out.append("    " + ", ".join(f"0x{v:08X}" for v in c[i : i + 6]) + ",")
        out.append("];")
        out.append(f"static {name}_C{k}: Image = Image {{ w: {K}, h: {K}, px: &{name}_C{k}_PX }};")
    out.append(f"static {name}_ROWS: [Argb; 64] = [")
    for i in range(0, 64, 6):
        out.append("    " + ", ".join(f"0x{v:08X}" for v in rows[i : i + 6]) + ",")
    out.append("];")
    out.append(f"/// The {name.lower().replace('_', ' ')} glass panel.")
    out.append(f"pub static {name}: PanelArt = PanelArt {{")
    out.append(f"    radius: {K},")
    out.append(f"    rows: &{name}_ROWS,")
    out.append(f"    top: [0x{top[0]:08X}, 0x{top[1]:08X}],")
    out.append(f"    bottom: [0x{bottom[0]:08X}, 0x{bottom[1]:08X}],")
    out.append(f"    side: 0x{side:08X},")
    out.append(f"    corners: [&{name}_C0, &{name}_C1, &{name}_C2, &{name}_C3],")
    out.append("};")
    return out


def main():
    lines = [
        "//! The glass panels' art, made by `tools/shell-assets/panel_art.py` from",
        "//! the owner's pick (2026-11-01: design B, the light look in his",
        "//! reference's frosted grey). Do not edit.",
        "",
        "use rp_draw::{Argb, Image, PanelArt};",
        "",
    ]
    for name, look in LOOKS.items():
        lines += art(name, look)
        lines.append("")
    with open(os.path.normpath(OUT), "w", newline="\n") as f:
        f.write("\n".join(lines))
    print("panel_art.rs written")


if __name__ == "__main__":
    main()
