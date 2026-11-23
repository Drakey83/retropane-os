"""The JPEG reader's test files (shell/pim/src/jpeg_tests.rs), and what each
should give: the picture's middle square, upright, as PIL makes it, at 24 x
24 (each a 4 x 4 mean of the 96 x 96 square), so a test compares the reader
with an independent decoder.

Run: python shell/pim/src/jpeg_fixtures/make.py (Pillow). The files are
committed; this only says how they were made.
"""

import io
import os
import struct

from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))


def scene(w, h, seed=0):
    """A picture with gradients, edges and a marked corner (top left red)."""
    im = Image.new("RGB", (w, h))
    px = im.load()
    for y in range(h):
        for x in range(w):
            px[x, y] = ((x * 255) // max(w - 1, 1), (y * 255) // max(h - 1, 1), ((x + y + seed) * 3) % 256)
    d = ImageDraw.Draw(im)
    d.ellipse([w // 4, h // 4, 3 * w // 4, 3 * h // 4], fill=(240, 200, 40))
    d.rectangle([0, 0, w // 3, h // 3], fill=(220, 20, 20))
    return im


def expected(im):
    """The middle square at 96, then 24 x 24 means, as bytes."""
    w, h = im.size
    s = min(w, h)
    sq = im.crop(((w - s) // 2, (h - s) // 2, (w - s) // 2 + s, (h - s) // 2 + s)).convert("RGB")
    sq = sq.resize((96, 96), Image.BOX)
    small = sq.resize((24, 24), Image.BOX)
    return bytes(small.tobytes())


def save(name, im, upright=None, **kw):
    b = io.BytesIO()
    im.save(b, "JPEG", **kw)
    data = b.getvalue()
    with open(os.path.join(HERE, name + ".jpg"), "wb") as f:
        f.write(data)
    with open(os.path.join(HERE, name + ".rgb"), "wb") as f:
        f.write(expected(upright or im))
    print(name, len(data))
    return data


def exif_with_thumb(orient, thumb):
    """An APP1 Exif block: IFD0 with the orientation, IFD1 the thumbnail."""
    le = lambda fmt, *v: struct.pack("<" + fmt, *v)
    tiff = bytearray(b"II*\x00" + le("I", 8))
    # IFD0: one entry, then the next IFD's offset.
    ifd0_at = 8
    ifd1_at = ifd0_at + 2 + 12 + 4
    tiff += le("H", 1) + le("HHI", 0x0112, 3, 1) + le("HH", orient, 0) + le("I", ifd1_at)
    thumb_at = ifd1_at + 2 + 3 * 12 + 4
    tiff += le("H", 3)
    tiff += le("HHI", 0x0103, 3, 1) + le("HH", 6, 0)
    tiff += le("HHII", 0x0201, 4, 1, thumb_at)
    tiff += le("HHII", 0x0202, 4, 1, len(thumb))
    tiff += le("I", 0)
    tiff += thumb
    return b"Exif\x00\x00" + bytes(tiff)


def main():
    save("base420", scene(200, 150), quality=85, subsampling=2)
    save("base444", scene(640, 480, 7), quality=80, subsampling=0)
    save("grey", scene(64, 64).convert("L"), upright=scene(64, 64).convert("L").convert("RGB"), quality=85)
    save("prog", scene(800, 600, 3), quality=80, progressive=True, subsampling=2)
    # Restart markers every 4 MCUs.
    im = scene(320, 240, 5)
    b = io.BytesIO()
    im.save(b, "JPEG", quality=85, restart_marker_blocks=4)
    data = b.getvalue()
    open(os.path.join(HERE, "restart.jpg"), "wb").write(data)
    open(os.path.join(HERE, "restart.rgb"), "wb").write(expected(im))
    print("restart", len(data), b"\xff\xdd" in data)
    # A camera's file: the main picture all blue, its thumbnail the scene,
    # turned (orientation 6: to be turned a quarter clockwise).
    thumb_im = scene(160, 120, 9)
    t = io.BytesIO()
    thumb_im.save(t, "JPEG", quality=80)
    main_im = Image.new("RGB", (1600, 1200), (20, 40, 220))
    m = io.BytesIO()
    main_im.save(m, "JPEG", quality=60, exif=exif_with_thumb(6, t.getvalue()))
    data = m.getvalue()
    open(os.path.join(HERE, "camera.jpg"), "wb").write(data)
    open(os.path.join(HERE, "camera.rgb"), "wb").write(expected(thumb_im.transpose(Image.Transpose.ROTATE_270)))
    print("camera", len(data))


main()
