"""hal/mem/src/trig.rs's polynomials (2026-11-16). Fits sin(pi/2 f)/f = S(u) and cos(pi/2 f) = C(u), u = f^2, f in [0, 1/2],
as Q30 integer polynomials evaluated exactly as the X51v will (Horner, each
step (acc * u) >> 32 with u in Q32, floor), and reports the worst error of the
integer pipeline against double-precision sin/cos."""
import math
import numpy as np

N = 20000
fs = np.linspace(0.0, 0.5, N)
us = fs * fs
sin_t = np.where(fs == 0, math.pi / 2, np.sin(math.pi / 2 * fs) / np.where(fs == 0, 1, fs))
cos_t = np.cos(math.pi / 2 * fs)


def lawson(target, deg, iters=200):
    w = np.ones_like(us)
    V = np.vander(us, deg + 1, increasing=True)
    for _ in range(iters):
        sw = np.sqrt(w)
        c, *_ = np.linalg.lstsq(V * sw[:, None], target * sw, rcond=None)
        err = np.abs(V @ c - target)
        w = w * err
        w /= w.sum()
    return c


def q30(c):
    return [int(round(v * (1 << 30))) for v in c]


def horner_int(coefs, u32):
    acc = coefs[-1]
    for c in reversed(coefs[:-1]):
        acc = c + ((acc * u32) >> 32)
    return acc


for deg in (4, 5):
    S = q30(lawson(sin_t, deg))
    C = q30(lawson(cos_t, deg))
    worst_s = worst_c = 0.0
    for f in np.linspace(0, 0.5, 200001):
        u32 = int(f * f * (1 << 32))
        s = horner_int(S, u32) / (1 << 30)
        c = horner_int(C, u32) / (1 << 30)
        worst_s = max(worst_s, abs(s * f - math.sin(math.pi / 2 * f)))
        worst_c = max(worst_c, abs(c - math.cos(math.pi / 2 * f)))
    print(f"deg {deg}: S {S}\n        C {C}\n  worst sin {worst_s:.3e} (2^{math.log2(worst_s):.1f}), cos {worst_c:.3e} (2^{math.log2(worst_c):.1f})")
