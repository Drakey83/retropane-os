"""Panel designs for the owner's pick (2026-11-01), drawn as the 9-slice
images the X51v would draw them from, under the device's own words.

The owner's references: glass tiles with rounded corners, a bright bevelled
white rim strongest at the top left, a body lighter at the top left and
darker at the bottom right, a soft specular glow and corner glints, a faint
drop shadow; in the dark look, frosted dark glass with a thin bright rim
("glassmorphism"). "This should all be pre rendered as png files."

Each style is rendered once per box by a signed distance to a rounded
rectangle (4x supersampled), which is exactly what its 9-slice image holds:
the corners as drawn, the edges a row or column stretched, the middle
filled. Composited over the wallpaper (the 2D model's) and under the words
and pictures (the CPU renderer's layer, from `cargo xtask ui-preview`).

Run: python tools/shell-assets/panel_mock.py
"""

import os

import numpy as np
from PIL import Image

HERE = os.path.dirname(__file__)
PRE = os.path.join(HERE, "..", "..", "target", "ui-preview")
OUT = os.path.join(HERE, "..", "..", "docs", "design", "mockups")
W, H = 480, 640

# The boxes on Today and on the Now page (x, y, w, h), as the shell lays
# them out.
TODAY = [(16, 80, 448, 206)] + [(16 + i * 91, 306, 83, 74) for i in range(5)] + [
    (16, 392, 448, 104)
] + [(16 + i * 91, 532, 84, 54) for i in range(5)]
NOW = [(16, 80, 448, 206), (16, 294, 448, 40)] + [
    (16 + i * 152, 342, 144, 52) for i in range(3)
] + [(16 + (i % 2) * 232, 402 + (i // 2) * 76, 216, 70) for i in range(4)]


def sdf_round_rect(w, h, r, ss):
    """Signed distance (px, negative inside) to a w x h rounded rectangle
    of corner radius r, sampled at ss x ss per pixel."""
    ys, xs = np.mgrid[0 : h * ss, 0 : w * ss]
    x = (xs + 0.5) / ss
    y = (ys + 0.5) / ss
    qx = np.abs(x - w / 2) - (w / 2 - r)
    qy = np.abs(y - h / 2) - (h / 2 - r)
    out = np.hypot(np.maximum(qx, 0), np.maximum(qy, 0))
    inside = np.minimum(np.maximum(qx, qy), 0)
    return out + inside - r, x, y


def down(a, ss):
    h, w = a.shape[0] // ss, a.shape[1] // ss
    return a.reshape(h, ss, w, ss, *a.shape[2:]).mean(axis=(1, 3))


def smooth(e0, e1, v):
    t = np.clip((v - e0) / (e1 - e0), 0, 1)
    return t * t * (3 - 2 * t)


def panel(w, h, st, dark):
    """The panel's RGBA (premultiplied off), w x h plus its shadow margin."""
    ss = 4
    m = st["shadow"] + 2
    W2, H2 = w + 2 * m, h + 2 * m
    d, x, y = sdf_round_rect(W2, H2, 0, ss)  # placeholder for the grid
    # Distances to the box (inside the margin).
    ys, xs = np.mgrid[0 : H2 * ss, 0 : W2 * ss]
    px = (xs + 0.5) / ss - m
    py = (ys + 0.5) / ss - m
    r = min(st["radius"], h / 2, w / 2)
    qx = np.abs(px - w / 2) - (w / 2 - r)
    qy = np.abs(py - h / 2) - (h / 2 - r)
    d = np.hypot(np.maximum(qx, 0), np.maximum(qy, 0)) + np.minimum(np.maximum(qx, qy), 0) - r
    cover = np.clip(0.5 - d * ss / ss, 0, 1)  # sharp, antialiased edge
    # The body: lighter at the top left, darker at the bottom right.
    t = np.clip((px / w) * 0.55 + (py / h) * 0.45, 0, 1)
    top, bot = np.array(st["dark_body" if dark else "body"])
    body = top[None, None, :] * (1 - t[..., None]) + bot[None, None, :] * t[..., None]
    alpha = st["alpha_dark" if dark else "alpha"]
    rgb = body.copy()
    a = cover * alpha
    # The rim: bright just inside the edge, fading inward over its width;
    # strongest along the top and left (the light from the top left).
    inward = -d
    rim = (1 - smooth(0, st["rim"], inward)) * (inward >= 0)
    lit = 0.55 + 0.45 * np.clip(1 - (px / w) * 0.5 - (py / h) * 0.7, 0, 1)
    rim_a = rim * lit * (st["rim_dark"] if dark else st["rim_light"])
    rgb = rgb * (1 - rim_a[..., None]) + 255 * rim_a[..., None]
    # The specular glows: soft ovals of white.
    for (gx, gy, gw, gh, ga) in st["glows"]:
        g = np.exp(-(((px - gx * w) / (gw * w)) ** 2 + ((py - gy * h) / (gh * h)) ** 2))
        ga = ga * (0.7 if dark else 1.0)
        rgb = rgb * (1 - (g * ga)[..., None]) + 255 * (g * ga)[..., None]
    # Corner glints: a short bright arc in the top-left and bottom-right
    # corners, just inside the rim.
    if st["glints"]:
        for (cx, cy) in [(r, r), (w - r, h - r)]:
            dd = np.hypot(px - cx, py - cy)
            ring = np.exp(-((dd - (r - 2.5)) / 0.9) ** 2) * (np.hypot(px - cx, py - cy) > 0)
            ang = (px - cx) * (1 if cx < w / 2 else -1) + (py - cy) * (1 if cy < h / 2 else -1)
            ring = ring * (ang < -r * 0.4)
            rgb = rgb * (1 - ring[..., None] * 0.8) + 255 * ring[..., None] * 0.8
    # The drop shadow, below and to the right, outside the box.
    sd_ = np.hypot(np.maximum(np.abs(px - w / 2 - 2) - (w / 2 - r), 0), np.maximum(np.abs(py - h / 2 - 3) - (h / 2 - r), 0)) - r
    sh = (1 - smooth(-1, st["shadow"], sd_)) * st["shadow_a"] * (1 - cover)
    out = np.zeros((H2 * ss, W2 * ss, 4))
    out[..., :3] = rgb * a[..., None]
    out[..., 3] = a + sh * (1 - a)
    # Premultiplied: the shadow black.
    out = down(out, ss)
    return out, m


def composite(wall, layer, boxes, st, dark):
    img = wall.astype(float).copy()
    for (bx, by, bw, bh) in boxes:
        p, m = panel(bw, bh, st, dark)
        x0, y0 = bx - m, by - m
        ph, pw = p.shape[:2]
        sx0, sy0 = max(0, -x0), max(0, -y0)
        x0c, y0c = max(0, x0), max(0, y0)
        x1, y1 = min(W, x0 + pw), min(H, y0 + ph)
        sub = p[sy0 : sy0 + (y1 - y0c), sx0 : sx0 + (x1 - x0c)]
        reg = img[y0c:y1, x0c:x1]
        img[y0c:y1, x0c:x1] = sub[..., :3] + reg * (1 - sub[..., 3:4])
    la = layer[..., 3:4] / 255.0
    img = layer[..., :3] * la + img * (1 - la)
    return np.clip(img, 0, 255).astype(np.uint8)


STYLES = {
    "a-bevel": dict(
        radius=14, rim=4, rim_light=0.95, rim_dark=0.55, alpha=0.96, alpha_dark=0.94,
        body=[(242, 244, 247), (214, 217, 222)], dark_body=[(58, 62, 70), (30, 32, 38)],
        glows=[(0.22, 0.18, 0.18, 0.22, 0.45)], glints=True, shadow=7, shadow_a=0.30,
    ),
    "b-glass": dict(
        radius=12, rim=2.2, rim_light=0.85, rim_dark=0.65, alpha=0.95, alpha_dark=0.90,
        body=[(236, 238, 242), (222, 225, 230)], dark_body=[(64, 66, 74), (40, 42, 48)],
        glows=[(0.3, 0.05, 0.45, 0.25, 0.30)], glints=False, shadow=4, shadow_a=0.18,
    ),
    "b-grey": dict(
        radius=12, rim=2.2, rim_light=0.85, rim_dark=0.65, alpha=0.96, alpha_dark=0.90,
        body=[(176, 178, 183), (140, 142, 148)], dark_body=[(64, 66, 74), (40, 42, 48)],
        glows=[(0.3, 0.05, 0.45, 0.25, 0.26)], glints=False, shadow=4, shadow_a=0.18,
    ),
    "c-soft-bevel": dict(
        radius=20, rim=7, rim_light=0.75, rim_dark=0.45, alpha=0.97, alpha_dark=0.95,
        body=[(240, 242, 246), (220, 223, 228)], dark_body=[(52, 56, 64), (28, 30, 36)],
        glows=[(0.2, 0.15, 0.2, 0.25, 0.35), (0.78, 0.8, 0.22, 0.25, 0.30)], glints=True,
        shadow=9, shadow_a=0.22,
    ),
}


def main_grey():
    """B with the grey of the owner's glassmorphism reference (2026-11-01),
    light, next to B as drawn."""
    wall = np.frombuffer(open(os.path.join(PRE, "mock-wall.rgb"), "rb").read(), np.uint8).reshape(H, W, 3)
    for part, boxes in [("today", TODAY), ("now", NOW)]:
        layer = np.frombuffer(
            open(os.path.join(PRE, f"mock-light-{part}.rgba"), "rb").read(), np.uint8
        ).reshape(H, W, 4).astype(float)
        a = composite(wall, layer, boxes, STYLES["b-glass"], False)
        b = composite(wall, layer, boxes, STYLES["b-grey"], False)
        gap = np.full((H, 8, 3), 255, np.uint8)
        Image.fromarray(np.concatenate([a, gap, b], axis=1)).save(
            os.path.join(OUT, f"glass-b-grey-{part}.png")
        )


def main():
    wall = np.frombuffer(open(os.path.join(PRE, "mock-wall.rgb"), "rb").read(), np.uint8).reshape(H, W, 3)
    for look in ["light", "dark"]:
        rows = []
        for part, boxes in [("today", TODAY), ("now", NOW)]:
            layer = np.frombuffer(
                open(os.path.join(PRE, f"mock-{look}-{part}.rgba"), "rb").read(), np.uint8
            ).reshape(H, W, 4).astype(float)
            shots = [composite(wall, layer, boxes, st, look == "dark") for st in STYLES.values()]
            gap = np.full((H, 8, 3), 255, np.uint8)
            rows.append(np.concatenate([shots[0], gap, shots[1], gap, shots[2]], axis=1))
        for part, row in zip(["today", "now"], rows):
            Image.fromarray(row).save(os.path.join(OUT, f"glass-{look}-{part}.png"))
    print("mock-ups in docs/design/mockups/glass-<light|dark>-<today|now>.png")


if __name__ == "__main__":
    import sys

    main_grey() if "grey" in sys.argv else main()
