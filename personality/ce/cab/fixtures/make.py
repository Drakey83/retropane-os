# Makes the synthetic cabinets the rp-cab tests read. Run from this folder:
#   python -I make.py
# Python's zlib is the MSZIP compressor: an implementation independent of
# ours, so the tests check our decoder against someone else's encoder.
# File contents come from gen(), which tests/fixtures.rs mirrors.
import struct
import zlib

WORDS = [b"golf", b"toy", b"rally", b"pocket", b"axim", b"cab", b"stunt", b"pool"]


def gen(seed, n):
    out = bytearray()
    x = seed
    while len(out) < n:
        x = (x * 1103515245 + 12345) & 0x7FFFFFFF
        out += WORDS[(x >> 16) % 8] + b" "
    return bytes(out[:n])


def checksum(data, seed):
    s = seed
    n = len(data) // 4
    for i in range(n):
        s ^= struct.unpack_from("<I", data, 4 * i)[0]
    t = 0
    for b in data[4 * n:]:
        t = (t << 8) | b
    return s ^ t


def mszip_blocks(stream, block, level_for):
    """Cuts `stream` into MSZIP blocks: each a complete raw DEFLATE stream
    whose dictionary is the previous 32 KiB of output."""
    blocks = []
    for i, at in enumerate(range(0, len(stream), block)):
        chunk = stream[at:at + block]
        hist = stream[max(0, at - 32768):at]
        kw = {"zdict": hist} if hist else {}
        c = zlib.compressobj(level_for(i), zlib.DEFLATED, -15, **kw)
        blocks.append((b"CK" + c.compress(chunk) + c.flush(), len(chunk)))
    return blocks


def stored_blocks(stream, block):
    return [(stream[a:a + block], len(stream[a:a + block])) for a in range(0, len(stream), block)]


def cabinet(folders, files, reserve=None, csum=True):
    """folders: [(compression type, [(data, uncompressed len)])];
    files: [(name, folder, offset, size)]."""
    hdr_res, fol_res, dat_res = reserve or (0, 0, 0)
    flags = 4 if reserve else 0
    head_len = 36 + (4 + hdr_res if reserve else 0)
    folders_len = len(folders) * (8 + fol_res)
    files_blob = b""
    for name, fi, off, size in files:
        files_blob += struct.pack("<IIHHHH", size, off, fi, 0x5021, 0x6000, 0x20) + name + b"\0"
    data_at = head_len + folders_len + len(files_blob)
    folder_entries = b""
    data_blob = b""
    for ctype, blocks in folders:
        folder_entries += struct.pack("<IHH", data_at + len(data_blob), len(blocks), ctype) + b"\xEE" * fol_res
        for data, ulen in blocks:
            lens = struct.pack("<HH", len(data), ulen)
            c = checksum(lens, checksum(data, 0)) if csum else 0
            data_blob += struct.pack("<I", c) + lens + b"\xDD" * dat_res + data
    total = data_at + len(data_blob)
    head = struct.pack("<4sIIIIIBBHHHHH", b"MSCF", 0, total, 0, head_len + folders_len, 0, 3, 1,
                       len(folders), len(files), flags, 0x1234, 0)
    if reserve:
        head += struct.pack("<HBB", hdr_res, fol_res, dat_res) + b"\xCC" * hdr_res
    return head + folder_entries + files_blob + data_blob


def main():
    # Two uncompressed folders, small blocks so files cross block edges.
    a, b, c = gen(1, 2500), gen(2, 10), gen(3, 1800)
    f0 = a + b
    stored = cabinet(
        [(0, stored_blocks(f0, 1000)), (0, stored_blocks(c, 700))],
        [(b"APP~1.001", 0, 0, len(a)), (b"Read me.002", 0, len(a), len(b)), (b"data.003", 1, 0, len(c))],
    )
    open("stored.mscf", "wb").write(stored)

    # One MSZIP folder: 32 KiB blocks (the window slides), the first block
    # stored (level 0), the rest dynamic Huffman with cross-block matches,
    # plus an empty file.
    big, small = gen(4, 80000), gen(5, 300)
    stream = big + small
    mszip = cabinet(
        [(1, mszip_blocks(stream, 32768, lambda i: 0 if i == 0 else 9))],
        [(b"big.001", 0, 0, len(big)), (b"small.002", 0, len(big), len(small)), (b"empty.003", 0, len(stream), 0)],
    )
    open("mszip.mscf", "wb").write(mszip)

    # Reserved areas everywhere, checksums off, small MSZIP blocks
    # (fixed Huffman likely at level 1 on short input).
    d = gen(6, 5000)
    res = cabinet(
        [(1, mszip_blocks(d, 1500, lambda i: 1))],
        [(b"res.001", 0, 0, len(d))],
        reserve=(5, 2, 3),
        csum=False,
    )
    open("reserve.mscf", "wb").write(res)


if __name__ == "__main__":
    main()
