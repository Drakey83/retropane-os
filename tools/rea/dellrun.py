#!/usr/bin/env python3
"""Run single functions of Dell's Windows CE ARM drivers on the PC (R91).

Unicorn Engine (2.1.4, in WSL: `~/re/unicorn-venv`, see `setup.sh`) runs a
function of a driver DLL from `local/` against fake hardware: every read
and write in a hardware window is logged in order (offset, size, value,
the instruction's address), every call into another DLL is logged and
answered (0 unless told otherwise), and the call returns its r0. So Dell's
exact register sequences and the words it builds come from its own code,
not from reading decompiled C by hand, and without the device.

The clean-room rule holds: Dell's bytes are read from `local/` and run, the
log (hardware facts) goes under `local/`, no Dell code enters the repo.

    ~/re/unicorn-venv/bin/python tools/rea/dellrun.py check

runs the self-check (Dell's textured rectangle, `ddi_2700g.dll`
`0x02118414`, against the 33-word form our renderer writes). As a library:

    d = Dell("local/drivers-a08/ddi_2700g.dll")
    d.mmio("regs", 0x50000000, 0x10000)          # a logged window
    at = d.alloc(256)                             # scratch the call uses
    r0 = d.call(0x02118414, at, other)            # up to 4 args + stack
    d.log                                         # [(kind, ...), ...]
"""

import struct
import sys

from unicorn import UC_ARCH_ARM, UC_HOOK_CODE, UC_HOOK_MEM_READ, UC_HOOK_MEM_WRITE
from unicorn import UC_HOOK_MEM_UNMAPPED, UC_MODE_ARM, Uc, UcError
from unicorn.arm_const import (
    UC_ARM_REG_CPSR, UC_ARM_REG_LR, UC_ARM_REG_PC, UC_ARM_REG_R0, UC_ARM_REG_R1,
    UC_ARM_REG_R2, UC_ARM_REG_R3, UC_ARM_REG_SP,
)

PAGE = 0x1000
# Where the harness puts what the driver didn't bring: the stack, a heap
# for the caller's structures, the import stubs and the return trap.
STACK, STACK_SIZE = 0x7000_0000, 0x10_0000
HEAP, HEAP_SIZE = 0x6000_0000, 0x40_0000
STUBS, STUBS_SIZE = 0x5F00_0000, 0x1_0000
RETURN = 0x5EFF_F000
ARGS = (UC_ARM_REG_R0, UC_ARM_REG_R1, UC_ARM_REG_R2, UC_ARM_REG_R3)


def up(n, a=PAGE):
    return (n + a - 1) // a * a


class Dell:
    """One driver DLL loaded at its preferred base, ready to call into."""

    def __init__(self, path, budget=5_000_000):
        self.data = open(path, "rb").read()
        self.uc = Uc(UC_ARCH_ARM, UC_MODE_ARM)
        self.log = []
        self.budget = budget
        self.answers = {}  # import name -> r0 (or a callable(dell) -> r0)
        self.windows = []  # (name, base, size, reads: {offset: value})
        self.heap_next = HEAP
        self._load()
        for base, size in ((STACK, STACK_SIZE), (HEAP, HEAP_SIZE), (STUBS, STUBS_SIZE), (RETURN, PAGE)):
            self.uc.mem_map(base, size)
        self.uc.hook_add(UC_HOOK_CODE, self._stub, begin=STUBS, end=STUBS + STUBS_SIZE - 1)
        self.uc.hook_add(UC_HOOK_CODE, self._ret, begin=RETURN, end=RETURN + 3)
        self.uc.hook_add(UC_HOOK_MEM_UNMAPPED, self._unmapped)

    # -- the PE image --------------------------------------------------
    def _u32(self, at):
        return struct.unpack_from("<I", self.data, at)[0]

    def _load(self):
        pe = self._u32(0x3C)
        if self.data[pe:pe + 4] != b"PE\0\0":
            raise ValueError("not a PE file")
        machine, nsec = struct.unpack_from("<HH", self.data, pe + 4)
        if machine not in (0x01C0, 0x01C2):
            raise ValueError(f"machine {machine:#06x} is not ARM")
        opt = pe + 24
        optsize = struct.unpack_from("<H", self.data, pe + 20)[0]
        self.base = self._u32(opt + 28)
        size = self._u32(opt + 56)
        self.uc.mem_map(self.base, up(size))
        self.sections = []
        for k in range(nsec):
            s = opt + optsize + 40 * k
            name = self.data[s:s + 8].rstrip(b"\0").decode(errors="replace")
            vsize, va, rawsize, raw = struct.unpack_from("<IIII", self.data, s + 8)
            self.uc.mem_write(self.base + va, self.data[raw:raw + min(rawsize, vsize or rawsize)])
            self.sections.append((name, self.base + va, vsize))
        # Imports: every slot of the address table pointed at a stub of
        # its own, named by DLL and ordinal or name.
        imp_rva = self._u32(opt + 104)
        self.stubs = {}
        n = 0
        while imp_rva:
            d = imp_rva
            ilt, _, _, name_rva, iat = struct.unpack_from("<IIIII", self.image(d, 20))
            if not name_rva:
                break
            dll = self.cstr(self.base + name_rva)
            k = 0
            while True:
                entry = struct.unpack("<I", self.image((ilt or iat) + 4 * k, 4))[0]
                if not entry:
                    break
                if entry & 0x8000_0000:
                    what = f"{dll}#{entry & 0xFFFF}"
                else:
                    what = f"{dll}!{self.cstr(self.base + entry + 2)}"
                stub = STUBS + 4 * n
                self.stubs[stub] = what
                self.uc.mem_write(self.base + iat + 4 * k, struct.pack("<I", stub))
                k += 1
                n += 1
            imp_rva += 20

    def image(self, rva, n):
        return bytes(self.uc.mem_read(self.base + rva, n))

    def cstr(self, at):
        out = b""
        while True:
            c = bytes(self.uc.mem_read(at + len(out), 1))
            if c == b"\0" or len(out) > 256:
                return out.decode(errors="replace")
            out += c

    # -- what the caller sets up ----------------------------------------
    def alloc(self, n, fill=0):
        at = self.heap_next
        self.heap_next = up(at + n, 16)
        self.uc.mem_write(at, bytes([fill]) * n)
        return at

    def put(self, at, *words):
        self.uc.mem_write(at, struct.pack(f"<{len(words)}I", *words))

    def get(self, at, n):
        return list(struct.unpack(f"<{n}I", bytes(self.uc.mem_read(at, 4 * n))))

    def mmio(self, name, base, size, reads=None):
        """A logged hardware window: every access recorded in order; reads
        answer `reads[offset]` (0 otherwise)."""
        self.uc.mem_map(base, up(size))
        self.windows.append((name, base, size, reads or {}))
        self.uc.hook_add(UC_HOOK_MEM_WRITE, self._mmio, begin=base, end=base + size - 1)
        self.uc.hook_add(UC_HOOK_MEM_READ, self._mmio, begin=base, end=base + size - 1)

    # -- running --------------------------------------------------------
    def call(self, addr, *args):
        """Calls `addr` (ARM, or Thumb with bit 0 set) with `args`: the
        first four in r0-r3, the rest on the stack. Returns r0."""
        sp = STACK + STACK_SIZE - 0x100
        rest = list(args[4:])
        sp -= 4 * len(rest)
        if rest:
            self.put(sp, *rest)
        for reg, v in zip(ARGS, args[:4]):
            self.uc.reg_write(reg, v & 0xFFFF_FFFF)
        self.uc.reg_write(UC_ARM_REG_SP, sp)
        self.uc.reg_write(UC_ARM_REG_LR, RETURN)
        self.uc.emu_start(addr, RETURN, count=self.budget)
        if self.uc.reg_read(UC_ARM_REG_PC) & ~1 != RETURN:
            raise RuntimeError(f"stopped at {self.uc.reg_read(UC_ARM_REG_PC):#010x}, not the return")
        return self.uc.reg_read(UC_ARM_REG_R0)

    def _ret(self, uc, at, size, _):
        uc.emu_stop()

    def _stub(self, uc, at, size, _):
        what = self.stubs.get(at & ~1, f"stub {at:#x}")
        args = [uc.reg_read(r) for r in ARGS]
        ans = self.answers.get(what, 0)
        r0 = ans(self, args) if callable(ans) else ans
        self.log.append(("call", what, args, r0))
        uc.reg_write(UC_ARM_REG_R0, r0 & 0xFFFF_FFFF)
        lr = uc.reg_read(UC_ARM_REG_LR)
        # Back to the caller in its own state (bit 0: Thumb).
        cpsr = uc.reg_read(UC_ARM_REG_CPSR)
        uc.reg_write(UC_ARM_REG_CPSR, (cpsr | 0x20) if lr & 1 else (cpsr & ~0x20))
        uc.reg_write(UC_ARM_REG_PC, lr & ~1)

    def _mmio(self, uc, access, at, size, value, _):
        for name, base, wsize, reads in self.windows:
            if base <= at < base + wsize:
                pc = uc.reg_read(UC_ARM_REG_PC)
                off = at - base
                if access == 17:  # UC_MEM_WRITE
                    self.log.append(("write", name, off, size, value, pc))
                else:
                    v = reads.get(off, 0)
                    uc.mem_write(at, v.to_bytes(4, "little")[:size])
                    self.log.append(("read", name, off, size, v, pc))
                return

    def _unmapped(self, uc, access, at, size, value, _):
        pc = uc.reg_read(UC_ARM_REG_PC)
        self.log.append(("unmapped", at, size, value, pc))
        return False


# -- the self-check -----------------------------------------------------------
def f32(x):
    return struct.unpack("<I", struct.pack("<f", x))[0]


def check(path="local/drivers-a08/ddi_2700g.dll"):
    """Dell's textured rectangle (`0x02118414`, the corners of the 33-word
    object `0x02117d9c` lays out) from a known rectangle: the words must sit
    where `rp_drv_mbx3d::tex::quad` puts them (R78 item 3's static
    reading), corners bottom left, top left, bottom right, top right."""
    d = Dell(path)
    obj = d.alloc(0x21 * 4, 0xEE)
    rect = d.alloc(0x80)
    left, top, right, bottom = 10.0, 20.0, 74.0, 52.0
    ul, vt, ur, vb = 0.125, 0.25, 0.75, 0.875
    d.put(rect + 0x20, f32(left), f32(top), f32(right), f32(bottom))
    d.put(rect + 0x3C, 0x0012_3480)  # the texture's address
    d.put(rect + 0x44, 0)
    d.put(rect + 0x48, f32(ul), f32(vt), f32(ur), f32(vb))
    d.put(rect + 0x58, 64, 32, 1)  # width, height; 0x60 set: no stride bit
    d.put(rect + 0x70, 0, 0)  # no filter bit; rotation 0
    d.call(0x02118414, obj, rect)
    w = d.get(obj, 0x21)
    corners = [(left, bottom, ul, vb), (left, top, ul, vt), (right, bottom, ur, vb), (right, top, ur, vt)]
    bad = []
    for k, (x, y, u, v) in enumerate(corners):
        if (w[5 + 2 * k], w[6 + 2 * k]) != (f32(x), f32(y)):
            bad.append(f"corner {k} x/y at words {5 + 2 * k}-{6 + 2 * k}: {w[5 + 2 * k]:#x} {w[6 + 2 * k]:#x}")
        if (w[22 + 3 * k], w[23 + 3 * k]) != (f32(u), f32(v)):
            bad.append(f"corner {k} u/v at words {22 + 3 * k}-{23 + 3 * k}: {w[22 + 3 * k]:#x} {w[23 + 3 * k]:#x}")
    untouched = [k for k in range(0x21) if w[k] == 0xEEEEEEEE]
    print("words:", " ".join(f"{x:08x}" for x in w))
    print("TEX0", hex(w[1]), "TEX1", hex(w[2]), "(TEX1 address field", hex((w[2] & 0x01FF_FFFF) << 7), ")")
    print("left as built by 0x02117d9c (not by this function):", untouched)
    calls = [e for e in d.log if e[0] != "write"]
    if calls:
        print("calls and faults:", calls)
    if bad:
        print("FAIL:", *bad, sep="\n  ")
        return 1
    print("OK: Dell's corners and texture coordinates sit where tex::quad puts them")
    return 0


if __name__ == "__main__":
    sys.exit(check(*sys.argv[2:]) if sys.argv[1:2] == ["check"] else print(__doc__))
