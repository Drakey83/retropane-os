#!/usr/bin/env python3
"""Function-level diff of two decompiled versions of one driver (R82).

BinDiff's job without BinDiff: BinExport's newest Ghidra plugin targets
Ghidra 11.0.3, ours is 12.1.4. Each function REA wrote under
`local/re/ghidra-c/<module>/functions/` is normalized (REA's header
comment dropped; addresses, `FUN_` / `DAT_` / `LAB_` names and long hex
literals masked), then the two versions are matched: identical bodies
first, then each remaining function to its most similar partner. The
report lists the functions that changed (with a unified diff of the
normalized code), those only in one version, and how many are identical:
where a vendor fixed or worked around something between ROMs.

    python tools/rea/fdiff.py <module A> <module B> [--out local/re/<name>.md]

Module names are directories under `local/re/ghidra-c/`. Output stays
under `local/` (the clean-room rule).
"""

import argparse
import difflib
import hashlib
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "local" / "re" / "ghidra-c"

MASKS = [
    # Any Ghidra name that carries an address (FUN_, PTR_LAB_, u_Gamma_..., ...).
    (re.compile(r"(?<!\w)_?[A-Za-z]\w*?_(?:r0x)?[0-9a-fA-F]{6,}\w*"), "SYM_x"),
    (re.compile(r"\b(FUN|DAT|LAB|PTR)_x\b"), "SYM_x"),
    (re.compile(r"\b0x[0-9a-fA-F]{6,}\b"), "0xADDR"),
    (re.compile(r"\b(_?DAT|uRam|iRam)[0-9a-fA-F]{6,}\b"), "MEM"),
]


def body(text: str) -> str:
    lines = [l for l in text.splitlines() if not l.startswith("//")]
    out = "\n".join(lines).strip()
    for rx, rep in MASKS:
        out = rx.sub(rep, out)
    return out


def load(module: str) -> dict[str, str]:
    d = SRC / module / "functions"
    if not d.is_dir():
        sys.exit(f"no decompiled module {module} under {SRC}")
    return {f.stem: body(f.read_text(encoding="utf-8", errors="replace")) for f in d.glob("*.c")}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("a")
    ap.add_argument("b")
    ap.add_argument("--out")
    args = ap.parse_args()
    fa, fb = load(args.a), load(args.b)
    h = lambda s: hashlib.sha1(s.encode()).hexdigest()
    by_hash_b: dict[str, list[str]] = {}
    for k, v in fb.items():
        by_hash_b.setdefault(h(v), []).append(k)
    same, left_a = 0, []
    used_b = set()
    for k, v in fa.items():
        cand = [x for x in by_hash_b.get(h(v), []) if x not in used_b]
        if cand:
            used_b.add(cand[0])
            same += 1
        else:
            left_a.append(k)
    left_b = [k for k in fb if k not in used_b]
    changed, only_a = [], []
    pool = list(left_b)
    for k in sorted(left_a):
        best, score = None, 0.0
        for c in pool:
            r = difflib.SequenceMatcher(None, fa[k], fb[c], autojunk=False).quick_ratio()
            if r > score:
                best, score = c, r
        if best is not None and score >= 0.6:
            exact = difflib.SequenceMatcher(None, fa[k], fb[best], autojunk=False).ratio()
            changed.append((k, best, exact))
            pool.remove(best)
        else:
            only_a.append(k)
    only_b = pool
    md = [
        f"# {args.a} against {args.b}",
        "",
        f"Functions: {len(fa)} and {len(fb)}; identical (addresses masked) {same}; "
        f"changed {len(changed)}; only in the first {len(only_a)}; only in the second {len(only_b)}.",
        "",
    ]
    if changed:
        md += ["## Changed", ""]
        for a, b, r in sorted(changed, key=lambda t: t[2]):
            md += [f"### {a} -> {b} (similarity {r:.2f})", "", "```diff"]
            md += list(difflib.unified_diff(fa[a].splitlines(), fb[b].splitlines(), a, b, lineterm="", n=2))
            md += ["```", ""]
    if only_a:
        md += ["## Only in " + args.a, "", ", ".join(sorted(only_a)), ""]
    if only_b:
        md += ["## Only in " + args.b, "", ", ".join(sorted(only_b)), ""]
    text = "\n".join(md) + "\n"
    if args.out:
        with open(ROOT / args.out, "w", encoding="utf-8", newline="\n") as f:
            f.write(text)
        print(f"{same} identical, {len(changed)} changed, {len(only_a)} / {len(only_b)} unmatched -> {args.out}")
    else:
        sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
