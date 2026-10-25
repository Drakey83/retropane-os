"""Designs the 2700G video scaler's shrinking filter tables offline (gate V5,
quality audit C73; CLAUDE.md rule 0).

The scaler's filter is one symmetric kernel per direction, sampled every
1/8 source pixel and stored as signed bytes (64 = a weight of one): 33
values for the horizontal filter's 8 taps, 17 for the vertical's 4
(docs/hw/2700g.md, "The filter's format"). Every output pixel uses one of
eight phases of that kernel.

Linux ships one table, cut for enlarging (shrinking with it aliases: the
X51v's P9d, one-pixel stripes left at 3/4 size); the textbook answer is a
cubic or a tent stretched by the scale (P9d: 35% of one-pixel stripes left
at 3/4 across, 39% down). Here each table is computed for this exact
structure, per scale:

- **Across (8 taps):** least squares. Every phase's frequency response
  held flat to 0.7 of the new Nyquist frequency and toward zero above it,
  weighted hardest at the source's finest detail (one-pixel lines fold back
  as the most visible stripes), each phase summing to exactly 64.
- **Down (4 taps, too few for a stop band):** least squares with one more
  exact rule per phase: its even and odd taps each sum to 32, so one-pixel
  lines become exactly flat at every phase; all the rest of the freedom
  spent on sharpness.
- Then rounded to bytes by a search that keeps every exact rule.

Enlarging keeps Linux's set (a windowed sinc, near-ideal interpolation;
right on the X51v, p9c and P9d step 7). Run:

    python tools/scaler-filter/design.py

It rewrites drivers/mbx-display/src/filter/designed.rs; the Rust tests
check the tables (exact sums, the one-pixel zero down, the stop band
across against the cubic).
"""

import os
import sys

import numpy as np

# Output / source ratios designed for, smallest first (shrinking only).
RATIOS = [(1, 2), (9, 16), (5, 8), (2, 3), (11, 16), (3, 4), (13, 16), (7, 8),
          (15, 16)]
PHASES = 8
ONE = 64
GRID = 512  # frequency samples on [0, 0.5] cycles per source pixel
PASS = 0.7  # pass edge, as a fraction of the new Nyquist frequency


def layout(n_half):
    """For a half kernel of n_half + 1 bytes (n_half = 8 x taps / 2), each
    phase's taps as (byte index, offset in source pixels), in tap order."""
    taps = n_half // 8
    out = []
    for p in range(PHASES):
        row = []
        for i in range(-taps, taps + 1):
            d8 = 8 * i - p
            if abs(d8) <= n_half:
                row.append((n_half - abs(d8), d8 / 8.0))
        out.append(row)
    return out


def system(n_half, ratio, notch):
    """The weighted least-squares rows (a, b) and the exact rules (c, d)."""
    s = ratio[0] / ratio[1]
    lay = layout(n_half)
    f = np.linspace(0.0, 0.5, GRID)
    f_stop = 0.5 * s
    f_pass = PASS * f_stop
    want = np.where(f <= f_pass, float(ONE), 0.0)
    ramp = np.clip((f - f_stop) / (0.5 - f_stop), 0, 1) ** 2
    weight = np.where(f <= f_pass, 1.0, np.where(f >= f_stop, 4.0 * (0.05 + ramp), 0.0))
    if notch:
        weight = np.where(f <= f_pass, 1.0, np.where(f >= f_stop, 1.0, 0.0))
    m = n_half + 1
    rows, target = [], []
    for row in lay:
        for part, sign in ((np.cos, 1), (np.sin, -1)):
            a = np.zeros((GRID, m))
            for k, x in row:
                a[:, k] += sign * part(2 * np.pi * f * x)
            rows.append(a * weight[:, None])
            target.append((want if part is np.cos else np.zeros(GRID)) * weight)
    rules, goals = [], []
    for row in lay:
        total = np.zeros(m)
        alternate = np.zeros(m)
        for t, (k, _) in enumerate(row):
            total[k] += 1
            alternate[k] += (-1) ** t
        rules.append(total)
        goals.append(float(ONE))
        if notch:
            rules.append(alternate)
            goals.append(0.0)
    return np.vstack(rows), np.concatenate(target), np.array(rules), np.array(goals)


def solve(a, b, c, d):
    """Least squares under the exact rules (KKT; lstsq copes with rules
    that repeat, as phases p and 8 - p share their bytes)."""
    m = a.shape[1]
    kkt = np.block([[2 * a.T @ a, c.T], [c, np.zeros((len(c), len(c)))]])
    rhs = np.concatenate([2 * a.T @ b, d])
    return np.linalg.lstsq(kkt, rhs, rcond=None)[0][:m]


def quantise(x, a, b, c, d):
    """Bytes near `x` keeping every exact rule: round, then move one byte
    one step at a time, fewest broken rules first, least error next."""
    q = np.round(x).astype(int)
    err = lambda v: float(np.sum((a @ v - b) ** 2))
    broken = lambda v: float(np.sum(np.abs(c @ v - d)))
    for _ in range(1000):
        if broken(q) == 0:
            break
        best = None
        for k in range(len(q)):
            for step in (1, -1):
                v = q.copy()
                v[k] += step
                key = (broken(v), err(v))
                if best is None or key < best[0]:
                    best = (key, v)
        if best[0][0] >= broken(q):
            raise SystemExit("cannot keep every exact rule in bytes")
        q = best[1]
    assert broken(q) == 0 and q.min() >= -128 and q.max() <= 127
    return q


def table(n_half, ratio, notch):
    a, b, c, d = system(n_half, ratio, notch)
    return quantise(solve(a, b, c, d), a, b, c, d)


def main():
    root = os.path.join(os.path.dirname(__file__), "..", "..")
    path = os.path.join(root, "drivers", "mbx-display", "src", "filter", "designed.rs")
    lines = [
        "//! Generated by tools/scaler-filter/design.py: do not edit by hand.",
        "//! The scaler's shrinking tables, computed for this hardware (see",
        "//! [`super::Filter::designed`]): across, least squares; down, least",
        "//! squares with one-pixel lines removed exactly at every phase.",
        "",
        "use super::{Filter, H_LEN, V_LEN};",
        "",
        "/// (output, source) ratio and its table, smallest ratio first.",
        f"pub const DESIGNED: [((u16, u16), Filter); {len(RATIOS)}] = [",
    ]
    for r in RATIOS:
        h = table(32, r, notch=False)
        v = table(16, r, notch=True)
        lines.append(f"    (({r[0]}, {r[1]}), Filter {{")
        lines.append(f"        vertical: {fmt(v)},")
        lines.append(f"        horizontal: {fmt(h)},")
        lines.append("    }),")
    lines.append("];")
    lines.append("")
    lines.append("const _: () = assert!(V_LEN == 17 && H_LEN == 33);")
    lines.append("")
    with open(path, "w", encoding="utf-8", newline="\n") as fh:
        fh.write("\n".join(lines))
    print("wrote", os.path.normpath(path))


def fmt(q):
    return "[" + ", ".join(str(int(v)) for v in q) + "]"


if __name__ == "__main__":
    sys.exit(main())
