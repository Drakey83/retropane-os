#!/usr/bin/env python3
"""Cross-reference index over REA's decompiled Dell drivers (R82).

Reads every `local/re/ghidra-c/<module>/functions/*.c` that `rea.sh
decompile-all` wrote and lists, per module and function, what each touches:

- the board CPLD, through Windows Mobile's shared shadow table at
  `0xAC001320` (cpld.md: ten 32-bit slots for registers 0x02, 0x10, 0x12,
  0x14, 0x16, 0x18, 0x1A, 0x1C, 0x1E, 0x08), and each call of a function
  that does so with literal arguments (Dell's set/clear helpers);
- PXA270 hardware blocks named by a physical address literal (GPIO, I2C,
  clocks, the memory controller, ...);
- `KernelIoControl` (coredll ordinal 557) request codes;
- in the 2700G drivers, every literal in the 3D render block
  (`0x600`-`0x6FC`) and the TA / manager block (`0x800`-`0x87C`), with
  the line, since Dell's GLES writes registers through (offset, value)
  tables.

The output, `local/re/xref.md` and `local/re/xref.json`, stays under
`local/` with the decompiled code (the clean-room rule: Dell's code gives
hardware facts only). Run it after any new `decompile-all`:

    python tools/rea/xref.py
"""

import json
import re
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "local" / "re" / "ghidra-c"
OUT_MD = ROOT / "local" / "re" / "xref.md"
OUT_JSON = ROOT / "local" / "re" / "xref.json"

# cpld.md "Shadow at hand-over": slot address -> CPLD register.
CPLD_SLOTS = {
    0x20: 0x02, 0x24: 0x10, 0x28: 0x12, 0x2C: 0x14, 0x30: 0x16,
    0x34: 0x18, 0x38: 0x1A, 0x3C: 0x1C, 0x40: 0x1E, 0x44: 0x08,
}

# PXA270 blocks by physical base (Intel's PXA27x developer's manual).
PXA_BLOCKS = {
    0x40100000: "UART1 (FFUART)", 0x40200000: "UART2 (BTUART)",
    0x40300000: "I2C", 0x40400000: "I2S", 0x40500000: "AC97",
    0x40600000: "USB client (UDC)", 0x40700000: "UART3 (STUART)",
    0x40800000: "infrared (FICP)", 0x40900000: "RTC",
    0x40A00000: "OS timers", 0x40B00000: "PWM 0/2", 0x40C00000: "PWM 1/3",
    0x40D00000: "interrupt controller", 0x40E00000: "GPIO",
    0x40F00000: "power manager / power I2C", 0x41000000: "SSP1",
    0x41100000: "MMC / SD", 0x41300000: "clocks", 0x41400000: "mobile scaler",
    0x41500000: "keypad", 0x41600000: "USIM", 0x41700000: "SSP2",
    0x41800000: "memory stick", 0x41900000: "SSP3",
    0x44000000: "LCD controller", 0x48000000: "memory controller",
    0x4C000000: "USB host", 0x50000000: "camera",
}

# The shadow's slot order: Dell's set / clear helpers take a slot index.
SLOT_ORDER = [0x02, 0x10, 0x12, 0x14, 0x16, 0x18, 0x1A, 0x1C, 0x1E, 0x08]

GPU_MODULES = ("2700g", "gles", "pvr", "d3d", "gx", "egl")

LIT = re.compile(r"0x[0-9a-fA-F]+")
FUNC = re.compile(r"^\S.*\b(FUN_[0-9a-f]+|[A-Za-z_]\w*)\s*\(", re.M)
SLOT = re.compile(r"DAT_ac0013([0-9a-f]{2})")
IOCTL = re.compile(r"Ordinal_557\(\s*(0x[0-9a-fA-F]+|\d+)")
CALL = re.compile(r"\b(FUN_[0-9a-f]+)\(([^()]*)\)")
REG_LIT = re.compile(r"(?<![0-9a-fA-Fx])(0x(?:6[0-9a-fA-F]{2}|8[0-7][0-9a-fA-F]))\b")


def functions(module: Path):
    for f in sorted((module / "functions").glob("*.c")):
        text = f.read_text(encoding="utf-8", errors="replace")
        yield f.stem, text


def cpld_op(line: str) -> str:
    if "& ~" in line or "&~" in line:
        return "clear"
    if "|" in line:
        return "set"
    return "read/other"


def scan(module: Path):
    name = module.name
    gpu = any(k in name.lower() for k in GPU_MODULES)
    out = {
        "cpld": defaultdict(set),  # register -> {(function, op)}
        "cpld_calls": [],  # (function, helper, args, registers)
        "pxa": defaultdict(set),  # block -> {(function, literal)}
        "ioctl": defaultdict(set),  # code -> {function}
        "regs": defaultdict(list),  # offset -> [(function, line)]
        "functions": 0,
    }
    helpers = {}
    helper_ops = {}
    texts = list(functions(module))
    out["functions"] = len(texts)
    for fn, text in texts:
        regs = set()
        for line in text.splitlines():
            for m in SLOT.finditer(line):
                slot = int(m.group(1), 16)
                if slot in CPLD_SLOTS:
                    reg = CPLD_SLOTS[slot]
                    regs.add(reg)
                    out["cpld"][reg].add((fn, cpld_op(line)))
            for m in IOCTL.finditer(line):
                out["ioctl"][int(m.group(1), 0)].add(fn)
            # An address, not a float's bit pattern (0x40400000 is 3.0): a
            # register (low bits set) or a call's first argument (a map).
            firsts = {m.group(1) for m in re.finditer(r"\w\((0x[0-9a-fA-F]+)\s*,", line)}
            for lit in LIT.findall(line):
                v = int(lit, 16)
                base = v & 0xFFF00000
                low = v & 0xFFFFF
                if base in PXA_BLOCKS and (0 < low < 0x1000 or (low == 0 and lit in firsts)):
                    out["pxa"][PXA_BLOCKS[base]].add((fn, f"0x{v:08x}"))
            if gpu:
                for m in REG_LIT.finditer(line):
                    off = int(m.group(1), 16)
                    if 0x600 <= off <= 0x6FC or 0x800 <= off <= 0x87C:
                        if off % 4 == 0:
                            out["regs"][off].append((fn, line.strip()[:140]))
        if regs:
            name = f"FUN_{int(fn, 16):08x}"
            helpers[name] = sorted(regs)
            ops = {op for r in regs for (f2, op) in out["cpld"][r] if f2 == fn}
            helper_ops[name] = "clears" if "clear" in ops else "sets" if "set" in ops else "reads"
    # Calls of the CPLD-touching functions with literal arguments.
    for fn, text in texts:
        for m in CALL.finditer(text):
            h, args = m.group(1), m.group(2)
            if h in helpers and h != f"FUN_{int(fn, 16):08x}":
                a = [x.strip() for x in args.split(",")]
                if a and all(re.fullmatch(r"0x[0-9a-fA-F]+|\d+", x) for x in a):
                    out["cpld_calls"].append((fn, h, a, helpers[h], helper_ops[h]))
    return out


def main() -> int:
    if not SRC.is_dir():
        print(f"no decompiled modules under {SRC}", file=sys.stderr)
        return 1
    modules = sorted(p for p in SRC.iterdir() if (p / "functions").is_dir())
    data = {m.name: scan(m) for m in modules}
    md = [
        "# Cross-reference of Dell's decompiled drivers",
        "",
        "Generated by `tools/rea/xref.py` from `local/re/ghidra-c/` (REA,",
        "Ghidra). Hardware facts only (clean room). Function names are",
        "addresses in each module.",
        "",
        "## Modules",
        "",
        "| Module | Functions | CPLD registers | PXA blocks | KernelIoControl codes | 2700G register literals |",
        "|---|---|---|---|---|---|",
    ]
    for name, d in data.items():
        md.append(
            f"| {name} | {d['functions']} | "
            f"{', '.join(f'0x{r:02X}' for r in sorted(d['cpld'])) or '-'} | "
            f"{', '.join(sorted(d['pxa'])) or '-'} | "
            f"{', '.join(f'0x{c:08X}' for c in sorted(d['ioctl'])) or '-'} | "
            f"{len(d['regs'])} offsets |"
        )
    md += ["", "## CPLD (through the shared shadow table)", ""]
    for name, d in data.items():
        if not d["cpld"]:
            continue
        md += [f"### {name}", "", "| Register | Function | Operation |", "|---|---|---|"]
        for reg in sorted(d["cpld"]):
            for fn, op in sorted(d["cpld"][reg]):
                md.append(f"| 0x{reg:02X} | {fn} | {op} |")
        if d["cpld_calls"]:
            md += ["", "Calls of those functions with literal arguments:", "",
                   "| Caller | Helper (registers) | Arguments |", "|---|---|---|"]
            for fn, h, a, regs, op in sorted(d["cpld_calls"]):
                # A helper over most slots takes the slot index first.
                if len(regs) >= 5 and len(a) >= 2 and int(a[0], 0) < len(SLOT_ORDER):
                    reg = SLOT_ORDER[int(a[0], 0)]
                    bits = ", ".join(a[1:])
                    md.append(f"| {fn} | {h} ({op}) | register 0x{reg:02X}, bits {bits} |")
                else:
                    rs = "/".join(f"0x{r:02X}" for r in regs)
                    md.append(f"| {fn} | {h} ({op} {rs}) | {', '.join(a)} |")
        md.append("")
    md += ["## PXA270 blocks named by address", ""]
    for name, d in data.items():
        if not d["pxa"]:
            continue
        md += [f"### {name}", "", "| Block | Function | Address |", "|---|---|---|"]
        for blk in sorted(d["pxa"]):
            for fn, lit in sorted(d["pxa"][blk]):
                md.append(f"| {blk} | {fn} | {lit} |")
        md.append("")
    md += ["## KernelIoControl codes", "", "| Module | Code | Functions |", "|---|---|---|"]
    for name, d in data.items():
        for code in sorted(d["ioctl"]):
            md.append(f"| {name} | 0x{code:08X} | {', '.join(sorted(d['ioctl'][code]))} |")
    md += ["", "## 2700G register literals (render 0x600-0x6FC, TA 0x800-0x87C)", ""]
    for name, d in data.items():
        if not d["regs"]:
            continue
        md += [f"### {name}", "", "| Offset | Function | Line |", "|---|---|---|"]
        for off in sorted(d["regs"]):
            for fn, line in d["regs"][off][:12]:
                md.append(f"| 0x{off:03X} | {fn} | `{line.replace('|', '/')}` |")
            more = len(d["regs"][off]) - 12
            if more > 0:
                md.append(f"| 0x{off:03X} | ... | {more} more |")
        md.append("")
    with open(OUT_MD, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(md) + "\n")
    js = {
        name: {
            "functions": d["functions"],
            "cpld": {f"0x{r:02X}": sorted(map(list, v)) for r, v in d["cpld"].items()},
            "cpld_calls": d["cpld_calls"],
            "pxa": {k: sorted(map(list, v)) for k, v in d["pxa"].items()},
            "ioctl": {f"0x{c:08X}": sorted(v) for c, v in d["ioctl"].items()},
            "regs": {f"0x{o:03X}": v for o, v in d["regs"].items()},
        }
        for name, d in data.items()
    }
    with open(OUT_JSON, "w", encoding="utf-8", newline="\n") as f:
        json.dump(js, f, indent=1)
    print(f"{len(modules)} modules -> {OUT_MD.relative_to(ROOT)}, {OUT_JSON.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
