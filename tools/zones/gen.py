#!/usr/bin/env python3
"""The time-zone table (shell/pim/src/zone_table.rs) from the tz database.

Run in WSL (or any system with /usr/share/zoneinfo of tzdata 2018 or
later): `wsl python3 tools/zones/gen.py`. Reads each zone's TZif file's
footer (the POSIX TZ string: the zone's rule from now on), dedupes the
rules, and writes every zone of zone.tab and every link to one (the old
names files still carry: US/Pacific, Asia/Calcutta) with its country.

Iterate, don't imitate (rule 0): a desktop keeps the whole database with
every past change (about 400 KB compiled, glibc and Windows alike); a
calendar on the X51v needs each zone's rule from now on (past appointments
it shows were made in their own zone's rule already), so ours keeps one
14-byte rule per distinct rule (about 80) and a zone's name packed by its
area: about 9 KB for every zone on Earth.
"""

import os
import re
import sys

ROOT = os.environ.get("ZONEINFO", "/usr/share/zoneinfo")
OUT = os.path.join(os.path.dirname(__file__), "..", "..", "shell", "pim", "src", "zone_table.rs")

AREAS = ["", "Africa/", "America/", "Antarctica/", "Arctic/", "Asia/", "Atlantic/",
         "Australia/", "Europe/", "Indian/", "Pacific/", "Etc/", "US/", "Canada/"]


def footer(name):
    with open(os.path.join(ROOT, name), "rb") as f:
        data = f.read()
    if not data.startswith(b"TZif"):
        raise ValueError(name)
    tail = data.rstrip(b"\n").rsplit(b"\n", 1)[-1]
    return tail.decode()


def offset(s):
    """A POSIX offset ("5", "-3:30", "+05") as minutes EAST of UTC."""
    sign = -1
    if s[0] in "+-":
        sign = 1 if s[0] == "-" else -1
        s = s[1:]
    parts = [int(p) for p in s.split(":")]
    m = parts[0] * 60 + (parts[1] if len(parts) > 1 else 0)
    return sign * m


def at(s):
    """A transition's time ("2", "2:00", "-1", "26"), minutes."""
    sign = 1
    if s.startswith("-"):
        sign, s = -1, s[1:]
    if s.startswith("+"):
        s = s[1:]
    parts = [int(p) for p in s.split(":")]
    return sign * (parts[0] * 60 + (parts[1] if len(parts) > 1 else 0) + (1 if len(parts) > 2 and parts[2] >= 30 else 0))


NAME = r"(?:<[^>]*>|[A-Za-z]+)"
OFF = r"[+-]?\d+(?::\d+){0,2}"


def parse(tz):
    m = re.fullmatch(rf"({NAME})({OFF})(?:({NAME})({OFF})?(?:,(.+),(.+))?)?", tz)
    if not m:
        raise ValueError(tz)
    std = offset(m.group(2))
    if not m.group(3):
        return (std, std, (0, 0, 0, 0), (0, 0, 0, 0))
    dst = offset(m.group(4)) if m.group(4) else std + 60
    edges = []
    for e in (m.group(5), m.group(6)):
        if e is None:
            raise ValueError("no rule: " + tz)
        r = re.fullmatch(r"M(\d+)\.(\d)\.(\d)(?:/(.+))?", e)
        if not r:
            raise ValueError("not an Mm.w.d rule: " + tz)
        edges.append((int(r.group(1)), int(r.group(2)), int(r.group(3)),
                      at(r.group(4)) if r.group(4) else 120))
    return (std, dst, edges[0], edges[1])


def main():
    zones = []
    with open(os.path.join(ROOT, "zone.tab")) as f:
        for line in f:
            if line.startswith("#"):
                continue
            cc, _, name = line.split("\t")[:3]
            zones.append((name.strip(), name.strip(), cc))
    known = {z for z, _, _ in zones}
    cc_of = {z: c for z, _, c in zones}
    links = []
    with open(os.path.join(ROOT, "tzdata.zi")) as f:
        for line in f:
            p = line.split()
            if p and p[0] == "L" and p[1] in known and p[2] not in known:
                links.append((p[2], p[1], cc_of[p[1]]))
    zones += [("UTC", "Etc/UTC", "--"), ("Etc/UTC", "Etc/UTC", "--")]
    zones += [l for l in links if "/" in l[0] or l[0] in ("GB", "NZ", "PRC", "ROK", "Japan", "Singapore")]
    rules, index, names = [], {}, bytearray()
    skipped = []
    for name, file, cc in zones:
        try:
            r = parse(footer(file))
        except (ValueError, FileNotFoundError) as e:
            skipped.append(f"{name}: {e}")
            continue
        if r not in index:
            index[r] = len(rules)
            rules.append(r)
        area = max((k for k, a in enumerate(AREAS) if a and name.startswith(a)), default=0)
        rest = name[len(AREAS[area]):].encode()
        names += bytes([index[r], ord(cc[0]), ord(cc[1]), area, len(rest)]) + rest
    out = ["//! The time-zone table, made by tools/zones/gen.py from tzdata "
           f"({open(os.path.join(ROOT, 'tzdata.zi')).readline().split()[-1]}): do not edit.",
           "",
           "use super::{Edge, Rule};",
           "",
           "/// The zones' areas: a name's first byte past its rule and country.",
           f"pub(super) static AREAS: [&str; {len(AREAS)}] = [" + ", ".join(f'"{a}"' for a in AREAS) + "];",
           "",
           f"/// Every distinct rule ({len(rules)}).",
           f"pub(super) static RULES: [Rule; {len(rules)}] = ["]
    for std, dst, a, b in rules:
        e = lambda x: f"Edge {{ month: {x[0]}, week: {x[1]}, weekday: {x[2]}, at: {x[3]} }}"
        out.append(f"    Rule {{ std: {std}, dst: {dst}, on: {e(a)}, off: {e(b)} }},")
    out.append("];")
    out.append("")
    out.append(f"/// The zones ({len(zones) - len(skipped)}): rule, country (2), area, name length, name.")
    lines = []
    for i in range(0, len(names), 24):
        lines.append("    " + ", ".join(str(b) for b in names[i:i + 24]) + ",")
    out.append(f"pub(super) static NAMES: [u8; {len(names)}] = [")
    out += lines
    out.append("];")
    with open(OUT, "w", newline="\n") as f:
        f.write("\n".join(out) + "\n")
    print(f"{len(zones) - len(skipped)} zones, {len(rules)} rules, {len(names)} name bytes;"
          f" {len(rules) * 14 + len(names)} bytes in all")
    for s in skipped:
        print("skipped:", s, file=sys.stderr)


main()
