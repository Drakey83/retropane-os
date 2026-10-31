"""Fits the device's downscale filter to the icon packs (option C, docs/design/weather-rule0.md).

    python -I tools/shell-assets/shrink_fit.py [--write]

The image keeps only each pack's big (250 px) pictures and makes every
smaller size (Hourly's 160, the cards' 66, the rows' 40) from them while it
decodes (`rp_draw::shrink`). Rule 0: a textbook filter (box, bilinear,
Lanczos) is a first draft, not the answer. This fits the filter's kernel by
least squares, one per output size, so that shrinking the big picture comes
as close as it can to the picture made straight from the master at that
size (the packs' own small pictures were made that way: `icons.py`, its
crop, then Lanczos from the master). The model is the device's own
arithmetic: premultiplied alpha, a separable kernel of radius R output
pixels sampled P times a pixel and interpolated linearly, each pixel's
weights normalised to one.

It prints the RMS (premultiplied 8-bit RGBA, against the master's) of the
fitted kernel beside box, bilinear and Lanczos-3 on the same big pictures,
for every pack found: VClouds (assets/weather/vclouds, committed) and the
owner's (local/icons/packs/<name>, never committed). `--write` writes the
kernels as Rust (drivers/draw/src/shrink_taps.rs) and the test pictures
(drivers/draw/tests/data/shrink-*.png: two VClouds conditions, their big
picture and the master's at each size) the Rust test checks the RMS on.

Run with `-I` (isolated): the pictures are data from elsewhere.
"""

import glob
import os
import sys

import numpy as np
from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BIG = (250, 176)
SIZES = (160, 66, 40)
# Samples a kernel holds per output pixel, and the radii tried (output px).
P = 8
RADII = (1.5, 2.0, 3.0)
# The test pictures: conditions of VClouds (partly cloudy day, thunder).
TEST_IDS = (3, 17)


def subject(img):
    """The picture cropped to its visible part plus a 3% margin (icons.py's)."""
    x0, y0, x1, y1 = img.getchannel("A").point(lambda a: 255 if a > 8 else 0).getbbox()
    m = int(max(x1 - x0, y1 - y0) * 0.03)
    return img.crop((max(0, x0 - m), max(0, y0 - m), min(img.width, x1 + m), min(img.height, y1 + m)))


def fit(img, size):
    """Scaled uniformly to the largest size inside `size` (icons.py's)."""
    k = min(size[0] / img.width, size[1] / img.height)
    return img.resize((max(1, round(img.width * k)), max(1, round(img.height * k))), Image.LANCZOS)


def out_size(w, h, side):
    """The device's size for `side` from a big picture `w` x `h`: the long
    side becomes `side`, rounded half up (`rp_draw::shrink::size`)."""
    m = max(w, h)
    return max(1, (w * side + m // 2) // m), max(1, (h * side + m // 2) // m)


def premul(img):
    a = np.asarray(img.convert("RGBA"), dtype=np.float64)
    out = a.copy()
    out[..., :3] = a[..., :3] * a[..., 3:4] / 255.0
    return out


def kernel_at(k, t, radius):
    """The kernel (samples `k` at 0, 1/P, ..., its last at `radius`, zero
    past it) at `t` output px, interpolated linearly, symmetric."""
    x = np.abs(t) * P
    i = np.floor(x).astype(int)
    f = x - i
    kk = np.concatenate([k, [0.0, 0.0]])
    i = np.minimum(i, len(k))
    v = kk[i] * (1 - f) + kk[np.minimum(i + 1, len(k))] * f
    return np.where(np.abs(t) < radius, v, 0.0)


_T = {}


def weights(n_in, n_out, k, radius):
    """The `n_out` x `n_in` matrix taking a row of `n_in` to `n_out`:
    output j's centre at (j + 0.5) * scale input px, taps within radius *
    scale on each side, each row normalised to one."""
    key = (n_in, n_out)
    if key not in _T:
        scale = n_in / n_out
        c = (np.arange(n_out) + 0.5)[:, None] * scale
        _T[key] = (np.arange(n_in)[None, :] + 0.5 - c) / scale
    v = kernel_at(k, _T[key], radius)
    s = v.sum(axis=1, keepdims=True)
    return np.divide(v, s, out=np.zeros_like(v), where=s != 0)


def shrink(big, dw, dh, k, radius):
    wy = weights(big.shape[0], dh, k, radius)
    wx = weights(big.shape[1], dw, k, radius)
    t = np.tensordot(wy, big, axes=(1, 0))
    out = np.tensordot(t, wx, axes=(1, 1)).transpose(0, 2, 1)
    # Clamped as the device clamps: alpha to 0-255, colour to 0-alpha
    # (straight colour at most 255). Fitting the unclamped values fitted
    # the ringing the device never shows (RMS 0.75 against 0.40 clamped).
    a = np.clip(out[..., 3:4], 0, 255)
    return np.concatenate([np.clip(out[..., :3], 0, a), a], axis=-1)


def lanczos(t, a=3):
    return np.where(np.abs(t) < a, np.sinc(t) * np.sinc(t / a), 0.0)


def textbook(radius, f):
    return np.array([f(m / P) for m in range(int(radius * P) + 1)], dtype=np.float64)


BASELINES = {
    "box": (0.5, lambda t: 1.0),
    "bilinear": (1.0, lambda t: 1.0 - abs(t)),
    "lanczos-3": (3.0, lambda t: float(lanczos(np.array(t)))),
}


def packs():
    found = [("vclouds", os.path.join(ROOT, "assets/weather/vclouds"))]
    for d in sorted(glob.glob(os.path.join(ROOT, "local/icons/packs/*"))):
        if os.path.isdir(d):
            found.append((os.path.basename(d), d))
    return found


def pictures(folder, side):
    """(big premultiplied, master's at the device's size premultiplied) for
    each condition of a pack."""
    out = []
    for i in range(1, 23):
        f = os.path.join(folder, f"{i:02}.png")
        if not os.path.exists(f):
            continue
        master = subject(Image.open(f).convert("RGBA"))
        big = fit(master, BIG)
        dw, dh = out_size(big.width, big.height, side)
        out.append((i, big, master.resize((dw, dh), Image.LANCZOS)))
    return out


def rms(sets, k, radius):
    err, n = 0.0, 0
    for big, want in sets:
        got = shrink(big, want.shape[1], want.shape[0], k, radius)
        err += ((got - want) ** 2).sum()
        n += want.size
    return (err / n) ** 0.5


def residuals(sets, k, radius):
    return np.concatenate(
        [(shrink(big, want.shape[1], want.shape[0], k, radius) - want).ravel() for big, want in sets]
    )


def fit_kernel(sets, radius, iters=12):
    """Levenberg-Marquardt over the kernel's samples (the first fixed at
    one: the weights are normalised), from Lanczos-3 cut to `radius`."""
    k = textbook(radius, lambda t: float(lanczos(np.array(t))))
    lam = 1e-2
    r = residuals(sets, k, radius)
    cost = r @ r
    for _ in range(iters):
        jac = np.empty((r.size, len(k) - 1))
        for p in range(1, len(k)):
            d = k.copy()
            d[p] += 1e-4
            jac[:, p - 1] = (residuals(sets, d, radius) - r) / 1e-4
        jtj, jtr = jac.T @ jac, jac.T @ r
        while True:
            step = np.linalg.solve(jtj + lam * np.diag(np.diag(jtj) + 1e-9), -jtr)
            k2 = k.copy()
            k2[1:] += step
            r2 = residuals(sets, k2, radius)
            if r2 @ r2 < cost:
                k, r, cost, lam = k2, r2, r2 @ r2, lam / 3
                break
            lam *= 4
            if lam > 1e8:
                return k
    return k


def main():
    write = "--write" in sys.argv
    results = {}
    for side in SIZES:
        per_pack = {name: [(premul(b), premul(t)) for _, b, t in pictures(folder, side)] for name, folder in packs()}
        every = [s for sets in per_pack.values() for s in sets]
        # Fitted on every other condition, checked on all (the others are
        # the held-out check that it learnt the filter, not the pictures).
        train = [s for sets in per_pack.values() for s in sets[::2]]
        print(f"== {side} px ({len(every)} pictures, {len(per_pack)} packs)")
        for name, (radius, f) in BASELINES.items():
            k = textbook(radius, f)
            print(f"  {name:10} " + "  ".join(f"{p} {rms(s, k, radius):5.2f}" for p, s in per_pack.items()))
        best = None
        for radius in RADII:
            k = fit_kernel(train, radius)
            e = rms(every, k, radius)
            print(f"  fit R={radius:<4} " + "  ".join(f"{p} {rms(s, k, radius):5.2f}" for p, s in per_pack.items()) + f"  (all {e:.2f})")
            # The smallest radius within 0.1 of the best: fewer taps, faster.
            if best is None or e < best[2] - 0.1:
                best = (radius, k, e)
        vc = per_pack["vclouds"]
        test = [vc[n] for n, (i, _, _) in enumerate(pictures(packs()[0][1], side)) if i in TEST_IDS]
        results[side] = (best[0], best[1], best[2], rms(test, best[1], best[0]))
        print(f"  chosen R={best[0]}: RMS {best[2]:.2f} (test pictures {results[side][3]:.2f})")
    if write:
        write_rust(results)
        write_tests()


def write_rust(results):
    lines = [
        "// Generated by tools/shell-assets/shrink_fit.py. Do not edit.",
        "",
        "use super::Kernel;",
        "",
    ]
    for side, (radius, k, e, test) in results.items():
        q = [round(v * 16384) for v in k]
        lines.append(f"/// {side} px: fitted to the masters (RMS {e:.2f} over every pack; {test:.2f} on the test pictures).")
        lines.append(f"pub static K{side}: Kernel = Kernel {{")
        lines.append(f"    side: {side},")
        lines.append(f"    radius_p: {int(radius * P)},")
        lines.append(f"    rms_x100: {int(np.ceil(test * 100))},")
        lines.append(f"    k: &[{', '.join(str(v) for v in q)}],")
        lines.append("};")
        lines.append("")
    lines.append(f"/// Every kernel, largest output first.")
    lines.append(f"pub static ALL: [&Kernel; {len(results)}] = [{', '.join(f'&K{s}' for s in results)}];")
    path = os.path.join(ROOT, "drivers/draw/src/shrink_taps.rs")
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(lines) + "\n")
    print(f"wrote {path}")


def write_tests():
    d = os.path.join(ROOT, "drivers/draw/tests/data")
    os.makedirs(d, exist_ok=True)
    folder = packs()[0][1]
    for i in TEST_IDS:
        master = subject(Image.open(os.path.join(folder, f"{i:02}.png")).convert("RGBA"))
        big = fit(master, BIG)
        big.save(os.path.join(d, f"shrink-{i:02}-big.png"), optimize=True)
        for side in SIZES:
            dw, dh = out_size(big.width, big.height, side)
            master.resize((dw, dh), Image.LANCZOS).save(os.path.join(d, f"shrink-{i:02}-{side}.png"), optimize=True)
    print(f"wrote the test pictures in {d}")


main()
