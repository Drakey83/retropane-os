"""Rebuilds what the X51v's panel showed during an opening of the Weather app
from its run log's trace (`trace of open N:` then `T` lines: ms, DSIG,
GSCADR, VSCADR, VSADR, VSCTRL, SSSIZE | the window x,y,w,h and the picture
row at its top), as a contact sheet of frames.

The owner, 2026-10-30: "why can't you see this in the logs, and if your logs
can't see it why aren't your logs better??". The trace says where the layer's
window was each refresh; this puts the pictures the image composes there, so
the movement can be looked at, not only read. The pictures are the shell's
own (`cargo xtask ui-preview`: 01-today.png before the tap, 06-weather-now.png
the app at rest), drawn by the same code as the device's but at 8 bits a
channel where the device's layer is 16-bit: shapes and timing are the
device's, colours approximate. DSIG (the display's signature of the pixels it
sent) tells which refreshes changed the picture at all.

Usage: python -I tools/trace/rebuild_open.py <run log> <open number> <out.png>
"""

import sys

from PIL import Image, ImageDraw


def trace(log, n):
    lines = open(log, encoding="utf-8", errors="replace").read().split("\n")
    i = next(k for k, l in enumerate(lines) if l.startswith(f"trace of open {n}:"))
    rows = []
    for line in lines[i + 1 :]:
        if not line.startswith("T "):
            break
        regs, meant = line.split("|")
        f = regs.split()
        x, y, w, h, row = map(int, meant.split(","))
        rows.append((int(f[1]), f[2], x, y, w, h, row))
    return rows


def main():
    log, n, out = sys.argv[1], int(sys.argv[2]), sys.argv[3]
    today = Image.open("target/ui-preview/01-today.png").convert("RGB")
    app = Image.open("target/ui-preview/06-weather-now.png").convert("RGB")
    rows = trace(log, n)
    cols = 8
    sheet = Image.new("RGB", (cols * 250, ((len(rows) + cols - 1) // cols) * 345), (20, 20, 20))
    d = ImageDraw.Draw(sheet)
    last = None
    for k, (ms, sig, x, y, w, h, row) in enumerate(rows):
        frame = today.copy()
        if w > 0 and h > 0:
            frame.paste(app.crop((x, row, x + w, row + h)), (x, y))
        cx, cy = 5 + (k % cols) * 250, (k // cols) * 345
        sheet.paste(frame.resize((240, 320)), (cx, cy + 20))
        same = " (same pixels)" if sig == last else ""
        d.text((cx, cy + 5), f"{ms} ms{same}", fill=(255, 255, 255))
        last = sig
    sheet.save(out)
    print(f"{len(rows)} refreshes -> {out}")


if __name__ == "__main__":
    main()
