#!/usr/bin/env python3
"""R79: the video plane's 2x enlarging filter fitted to game frames.

At exactly 2x the scaler samples its half kernels (filter.rs: 1/8-pixel
steps, 64 = one) at only two phases, a quarter pixel either side of a
source pixel: horizontally the 8 values at distances 0.25, 0.75 ... 3.75,
vertically the 4 at 0.25 ... 1.75. Those 12 values are fitted by least
squares (alternating: the horizontal set with the vertical held, then the
other way, each phase's taps summing to one) so that each 240 x 320 frame,
enlarged, comes as close as possible to the native 480 x 640 render of the
same frame (`rp-gpubench`'s `r79_frames`: the host model of the 3D core,
24 warped meshes flat and textured). Seeds 0-15 train, 16-23 are held out.

Scored on the held-out frames, PSNR (dB) against native: Dell's doubling
(each pixel repeated), bilinear, mbxfb's windowed sinc (today's), the fit
(exact), and the fit rounded to the hardware's signed bytes. Writes the
rounded tables (`--out`) for the morning look; nothing is shipped.

    python tools/scaler-filter/fit2x.py target/r79 [--out fit2x.json]
"""
import json
import sys
from pathlib import Path

import numpy as np

# mbxfb's half kernels, outer edge to centre (filter.rs Filter::MBXFB).
MBXFB_V = [0, 1, 0, -1, -2, -3, -4, -3, 0, 5, 13, 23, 34, 45, 55, 61, 64]
MBXFB_H = [0, 1, 1, -1, 0, 0, 0, 0, 0, 0, 1, 2, 2, 3, 2, 1, 0, -2, -5, -7, -9,
           -10, -9, -5, 0, 7, 17, 28, 39, 49, 57, 62, 64]
H_DIST = [2, 6, 10, 14, 18, 22, 26, 30]   # eighths: 0.25 .. 3.75
V_DIST = [2, 6, 10, 14]                   # 0.25 .. 1.75


def load(path, w, h):
    raw = np.fromfile(path, dtype='<u2').reshape(h, w).astype(np.int32)
    r = ((raw >> 11) & 31) << 3
    g = ((raw >> 5) & 63) << 2
    b = (raw & 31) << 3
    return np.stack([r, g, b], -1).astype(np.float64)


def taps(n_out, n_src, dists):
    """For each output index: (source indices, which kernel value each
    uses): a 2x enlargement, pixel centres aligned (output j at source
    j / 2 - 1/4), edges clamped."""
    k = len(dists)  # taps (8 across, 4 down), each a distinct value
    half = k // 2
    idx = np.zeros((n_out, k), dtype=np.int64)
    val = np.zeros((n_out, k), dtype=np.int64)
    for j in range(n_out):
        x = j / 2 - 0.25
        base = int(np.floor(x))
        frac = x - base  # 0.75 or 0.25
        for t in range(-half + 1, half + 1):
            src = min(max(base + t, 0), n_src - 1)
            d = abs(t - frac)
            m = int(round(d * 8))
            idx[j, t + half - 1] = src
            val[j, t + half - 1] = dists.index(m)
    return idx, val


def enlarge(img, hk, vk, ht, vt):
    """Separable: across with the half-kernel values hk (len 8), down with
    vk (len 4)."""
    hidx, hval = ht
    vidx, vval = vt
    across = (img[:, hidx, :] * hk[hval][None, :, :, None]).sum(2)
    down = (across[vidx, :, :] * vk[vval][:, :, None, None]).sum(1)
    return down


def psnr(a, b):
    mse = np.mean((np.clip(a, 0, 255) - b) ** 2)
    return 10 * np.log10(255 ** 2 / mse)


def fit_side(feats, target):
    """Least squares with the values summing to 1/2 per side (each phase's
    taps sum to one: both sides' values once each, so the half sums to
    one... the phase uses every value once): solve min |F a - y| with
    sum(a) = 1 by a Lagrange multiplier."""
    a_ls = np.linalg.lstsq(feats, target, rcond=None)[0]
    ftf_inv = np.linalg.pinv(feats.T @ feats)
    one = np.ones(len(a_ls))
    lam = (one @ a_ls - 1) / (one @ ftf_inv @ one)
    return a_ls - lam * (ftf_inv @ one)


def main():
    root = Path(sys.argv[1])
    out = sys.argv[sys.argv.index('--out') + 1] if '--out' in sys.argv else None
    pairs = []
    for seed in range(24):
        for kind in ('flat', 'tex'):
            small = load(root / f's{seed}_{kind}_240.raw', 240, 320)
            big = load(root / f's{seed}_{kind}_480.raw', 480, 640)
            pairs.append((seed, kind, small, big))
    train = [p for p in pairs if p[0] < 16]
    held = [p for p in pairs if p[0] >= 16]
    ht = taps(480, 240, H_DIST)
    vt = taps(640, 320, V_DIST)

    def from_half(half, dists, centre):
        return np.array([half[centre - m] for m in dists], dtype=np.float64) / 64

    mbx_h = from_half(MBXFB_H, H_DIST, 32)
    mbx_v = from_half(MBXFB_V, V_DIST, 16)
    # Fit: alternate, starting from bilinear-ish.
    hk = np.array([0.75, 0.25, 0, 0, 0, 0, 0, 0])
    vk = np.array([0.75, 0.25, 0, 0])
    rng = np.random.default_rng(79)
    for it in range(8):
        # Horizontal values, vertical held: each output = sum_v vk * sum_h hk * src.
        rows, ys = [], []
        for _, _, small, big in train:
            sel_y = rng.integers(0, 640, 400)
            sel_x = rng.integers(0, 480, 400)
            for c in range(3):
                # Features: for value i, sum over taps using it.
                hidx, hval = ht
                vidx, vval = vt
                f = np.zeros((len(sel_y), len(hk)))
                for vt_i in range(vidx.shape[1]):
                    srow = vidx[sel_y, vt_i]
                    wv = vk[vval[sel_y, vt_i]]
                    for ht_i in range(hidx.shape[1]):
                        scol = hidx[sel_x, ht_i]
                        np.add.at(f, (np.arange(len(sel_y)), hval[sel_x, ht_i]),
                                  wv * small[srow, scol, c])
                rows.append(f)
                ys.append(big[sel_y, sel_x, c])
        hk = fit_side(np.vstack(rows), np.concatenate(ys))
        rows, ys = [], []
        for _, _, small, big in train:
            sel_y = rng.integers(0, 640, 400)
            sel_x = rng.integers(0, 480, 400)
            for c in range(3):
                hidx, hval = ht
                vidx, vval = vt
                f = np.zeros((len(sel_y), len(vk)))
                for vt_i in range(vidx.shape[1]):
                    srow = vidx[sel_y, vt_i]
                    acc = np.zeros(len(sel_y))
                    for ht_i in range(hidx.shape[1]):
                        acc += hk[hval[sel_x, ht_i]] * small[srow, hidx[sel_x, ht_i], c]
                    np.add.at(f, (np.arange(len(sel_y)), vval[sel_y, vt_i]), acc)
                rows.append(f)
                ys.append(big[sel_y, sel_x, c])
        vk = fit_side(np.vstack(rows), np.concatenate(ys))

    def quantise(k):
        q = np.round(k * 64).astype(int)
        # Each phase uses every value once: the sum must be 64.
        q[np.argmax(np.abs(k))] += 64 - q.sum()
        return q

    hq, vq = quantise(hk), quantise(vk)
    doubling = (np.array([1, 0, 0, 0, 0, 0, 0, 0.0]), np.array([1, 0, 0, 0.0]))
    # Doubling as nearest: value at 0.25 = 1 (the nearer source pixel).
    bilinear = (np.array([0.75, 0.25, 0, 0, 0, 0, 0, 0]), np.array([0.75, 0.25, 0, 0]))
    ways = {
        "Dell's doubling (nearest)": doubling,
        'bilinear': bilinear,
        "mbxfb (today's)": (mbx_h, mbx_v),
        'fitted (exact)': (hk, vk),
        'fitted (signed bytes)': (hq / 64, vq / 64),
    }
    print('held-out PSNR against native 480 x 640 (dB), seeds 16-23:')
    report = {}
    for name, (h, v) in ways.items():
        scores = {k: [] for k in ('flat', 'tex')}
        for _, kind, small, big in held:
            scores[kind].append(psnr(enlarge(small, h, v, ht, vt), big))
        trained = [psnr(enlarge(s, h, v, ht, vt), b) for _, _, s, b in train[::4]]
        report[name] = {k: float(np.mean(x)) for k, x in scores.items()}
        report[name]['train'] = float(np.mean(trained))
        print(f'  {name:28} flat {np.mean(scores["flat"]):6.2f}  textured {np.mean(scores["tex"]):6.2f}'
              f'  (train {np.mean(trained):6.2f})')
    print('fitted horizontal (0.25 .. 3.75):', hq.tolist(), 'sum', hq.sum())
    print('fitted vertical (0.25 .. 1.75):', vq.tolist(), 'sum', vq.sum())
    if out:
        json.dump({'horizontal_eighths': H_DIST, 'horizontal': hq.tolist(),
                   'vertical_eighths': V_DIST, 'vertical': vq.tolist(),
                   'psnr': report}, open(out, 'w', newline='\n'), indent=1)


if __name__ == '__main__':
    main()
