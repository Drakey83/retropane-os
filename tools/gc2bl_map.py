"""Reads a GC2BL run log (local/rp-gpubench-runGC2BL*.txt) and prints, per
scene, the wrong columns as runs, the strongest periods of the column map
(autocorrelation peaks), the fraction bins that hold the faults, and the
tile rows: the data the bilinear error's rule is fitted from.

Usage: python tools/gc2bl_map.py local/rp-gpubench-runGC2BL.txt
"""

import re
import sys


def main(path):
    scenes = {}
    for line in open(path, encoding="utf-8", errors="replace"):
        m = re.match(r"\s*GC2BL scene (\d+) \((.*?); u0 (\d+), span (\d+), v (\d+); lock .*?\): (\d+) of (\d+) renders wrong, (\d+) wrong pixels \(red \+1 (\d+), -1 (\d+), other (\d+)\)", line)
        if m:
            si = int(m.group(1))
            scenes[si] = {
                "name": m.group(2), "u0": int(m.group(3)), "span": int(m.group(4)), "v": int(m.group(5)),
                "renders": (int(m.group(6)), int(m.group(7))), "pixels": int(m.group(8)),
                "plus": int(m.group(9)), "minus": int(m.group(10)), "other": int(m.group(11)),
                "cols": [], "frac": {}, "rows": [],
            }
            continue
        m = re.match(r"\s*GC2BL scene (\d+) columns (\d+)-(\d+): \[(.*)\]", line)
        if m:
            scenes[int(m.group(1))]["cols"] += [int(v) for v in m.group(4).split(",")]
            continue
        m = re.match(r"\s*GC2BL scene (\d+) by fraction/64, red ([+-]1): \[(.*)\]", line)
        if m:
            scenes[int(m.group(1))]["frac"][m.group(2)] = [int(v) for v in m.group(3).split(",")]
            continue
        m = re.match(r"\s*GC2BL scene (\d+) by tile row: \[(.*)\]", line)
        if m:
            scenes[int(m.group(1))]["rows"] = [int(v) for v in m.group(2).split(",")]
    for si in sorted(scenes):
        s = scenes[si]
        step = s["span"] / 480 / 4096
        print(f"scene {si}: {s['name']}")
        print(f"  step {step:.6f} texel/px ({s['span']}/480 units), u0 {s['u0']} ({s['u0'] / 4096:.4f} texel), v {s['v'] / 4096:.4f} texel")
        print(f"  {s['renders'][0]} of {s['renders'][1]} renders wrong, {s['pixels']} wrong pixels: red +1 {s['plus']}, -1 {s['minus']}, other {s['other']}")
        cols = s["cols"]
        if not cols:
            print("  clean")
            continue
        wrong = [x for x, c in enumerate(cols) if c]
        runs, start = [], None
        for x in range(481):
            on = x < 480 and cols[x] > 0
            if on and start is None:
                start = x
            if not on and start is not None:
                runs.append((start, x - 1))
                start = None
        print(f"  wrong columns: {len(wrong)} of 480; runs: {runs[:40]}{' ...' if len(runs) > 40 else ''}")
        # Periods: autocorrelation of the column map.
        n = len(cols)
        mean = sum(cols) / n
        var = sum((c - mean) ** 2 for c in cols) or 1
        ac = []
        for lag in range(2, 241):
            ac.append((sum((cols[x] - mean) * (cols[x + lag] - mean) for x in range(n - lag)) / var, lag))
        ac.sort(reverse=True)
        print(f"  strongest periods (lag, correlation): {[(l, round(c, 2)) for c, l in ac[:6]]}")
        for sign in ("+1", "-1"):
            f = s["frac"].get(sign)
            if f and sum(f):
                top = sorted(range(64), key=lambda b: -f[b])[:12]
                print(f"  red {sign} by fraction/64 (bin: count): {[(b, f[b]) for b in top if f[b]]}")
        rows = s["rows"]
        if rows:
            print(f"  by tile row (0-39): {rows}")
        # The fraction at the first wrong column, by the pixel-centre convention.
        for x in wrong[:5]:
            u = s["u0"] + (2 * x + 1) * s["span"] / 960
            print(f"  column {x}: u {u / 4096:.5f} texel, fraction {(u % 4096) / 4096:.5f}")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "local/rp-gpubench-runGC2BL.txt")
