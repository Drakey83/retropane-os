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
    return [(c[i] * (1 - g) + 255 * g) * look.get("dim", 1.0) for i in range(3)]


def rim_at(look, inward, lit):
    """The rim's strength `inward` px inside the edge, lit by `lit`."""
    return (1 - smooth(0, RIM, inward)) * lit * look["rim"]


def argb(rgb, a):
    a = max(0, min(255, round(a * 255)))
    r, g, b = (max(0, min(255, round(v))) for v in rgb)
    return (a << 24) | (r << 16) | (g << 8) | b


def mix(rgb, w):
    return [rgb[i] * (1 - w) + 255 * w for i in range(3)]


def corner(look, k):
    """Corner k (0 top left, 1 top right, 2 bottom left, 3 bottom right),
    K x K, from a rounded rectangle of radius K."""
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
                    f = 0.0 if top else 1.0
                    rgb = body(look, f)
                    lit = 0.55 + 0.45 * max(0.0, min(1.0, 1 - (0 if left else 0.5) - (0 if top else 0.7)))
                    rgb = mix(rgb, rim_at(look, -d, lit))
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
    rows = [argb(shown(body(look, i / 63)), look["alpha"]) for i in range(64)]
    top_lit, bottom_lit, side_lit = 0.8, 0.57, 0.73
    top = [argb(mix(body(look, 0.0), rim_at(look, y + 0.5, top_lit)), look["alpha"]) for y in range(2)]
    bottom = [argb(mix(body(look, 1.0), rim_at(look, 1.5 - y, bottom_lit)), look["alpha"]) for y in range(2)]
    side = argb(mix(body(look, 0.5), rim_at(look, 0.5, side_lit)), look["alpha"])
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
