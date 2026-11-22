//! R340's measure: the TA's parameter memory manager watched in a game.
//!
//! Iterate, don't imitate (rule 0). Dell's driver never looks at the
//! manager: it runs an 8 MiB parameter memory and answers the core's
//! "memory" event (status bit 5) after the fact (`ddi_2700g`
//! `0x021192b4`); PowerVR's own drivers size the memory for the worst
//! scene and recover by a partial render. Neither says how much of the
//! memory a scene holds, which kit 113's overlap (two scenes in one
//! memory) needs to be stall-free by construction: start the next scene's
//! TA only when it provably fits beside the render in flight. Nothing
//! published names the manager's allocation pointer, so ours reads the
//! chip itself in the game: the manager's 16 KiB block (in local memory,
//! where the manager keeps its page state) and the TA block's registers
//! `0x804`-`0x85C`, at the moments that tell the format apart (set-up,
//! a scene's TA done with no render, its render done, a TA done while a
//! render runs, that render done), every report (the drift over minutes:
//! a leak shows as words that never return to the set-up's), and at a
//! failed chain (if the chip still answers). Each line in the run's log
//! says the block's words in use and changed since the set-up; the
//! snapshots go to the card at the end (`pm-snap.bin`) for the PC to
//! decode. Cost: ~2 ms of bus reads a snapshot, a few a run.

use core::fmt::Arguments;

use rp_drv_blit2d::MBX;
use rp_hal_api::RegisterBus;
use rp_hal_mem::load32_volatile;
use rp_hal_pxa27x::{Mmio, rt};
use rp_port::gfx::bus_timed_out;

/// The TA block's words read: `0x804`-`0x85C`, the words Dell's drivers
/// touch (the TA's start `0x800` left alone; `0x860`-`0x87C` are past the
/// emulator's map, where an unknown 2700G register can hang the bus).
pub const REG_AT: usize = 0x804;
pub const REG_WORDS: usize = 23;
/// The manager's block's words (16 KiB).
pub const TABLE_WORDS: usize = 4096;
/// A snapshot: tag, frame, milliseconds since the open, flags; the
/// registers; the block.
const HEAD_WORDS: usize = 4;
pub const SNAP_WORDS: usize = HEAD_WORDS + REG_WORDS + TABLE_WORDS;
pub const SLOTS: usize = 8;
/// The file's header (`RPPMSN1\0`, then slots, words a snapshot).
const MAGIC: &[u8; 8] = b"RPPMSN1\0";
pub const FILE_HEAD: usize = 16;
/// The room the runner gives it.
pub const ROOM: usize = FILE_HEAD + SLOTS * SNAP_WORDS * 4;
/// The card file.
pub const FILE: &str = "pm-snap.bin";

/// Which moment a slot keeps (its tag).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Moment {
    /// After the manager's set-up, before any scene.
    SetUp = 0,
    /// A scene's TA done, no render running (overlap off).
    TaDoneAlone = 1,
    /// That scene's render done, no TA running.
    RenderDoneAlone = 2,
    /// A scene's TA done while the last scene's render runs (overlap).
    TaDoneOverlap = 3,
    /// That render done, the next scene waiting (overlap).
    RenderDoneOverlap = 4,
    /// The last two reports (rolling).
    Report = 5,
    ReportBefore = 6,
    /// A chain that did not finish, before the core's reset.
    Failed = 7,
}

/// Flags a snapshot keeps.
pub const F_RENDERING: u32 = 1;
pub const F_TA_BUSY: u32 = 2;
pub const F_OVERLAP: u32 = 4;
pub const F_TAKEN: u32 = 8;
/// A read past the bus's time-out: the rest not read.
pub const F_CUT: u32 = 16;

/// The snapshots, in the runner's RAM until the run's end.
pub struct PmWatch {
    mem: &'static mut [u8],
    /// Moments already kept (by tag), and the frame each wants from.
    kept: u32,
    /// The set-up's block, word count in use (for the reports' lines).
    base_used: u32,
}

impl PmWatch {
    /// The watch in `mem` (at least [`ROOM`] bytes).
    pub fn new(mem: &'static mut [u8]) -> Option<Self> {
        let mem = mem.get_mut(..ROOM)?;
        mem.fill(0);
        mem[..8].copy_from_slice(MAGIC);
        mem[8..12].copy_from_slice(&(SLOTS as u32).to_le_bytes());
        mem[12..16].copy_from_slice(&(SNAP_WORDS as u32).to_le_bytes());
        Some(Self {
            mem,
            kept: 0,
            base_used: 0,
        })
    }

    const fn at(slot: usize, k: usize) -> usize {
        FILE_HEAD + (slot * SNAP_WORDS + k) * 4
    }

    fn put(&mut self, slot: usize, k: usize, v: u32) {
        let a = Self::at(slot, k);
        self.mem[a..a + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn get(&self, slot: usize, k: usize) -> u32 {
        let a = Self::at(slot, k);
        u32::from_le_bytes([
            self.mem[a],
            self.mem[a + 1],
            self.mem[a + 2],
            self.mem[a + 3],
        ])
    }

    /// Whether moment `m` is still wanted (each kept once, the reports
    /// and the failure every time).
    pub const fn wants(&self, m: Moment) -> bool {
        matches!(m, Moment::Report | Moment::Failed) || self.kept & (1 << m as u32) == 0
    }

    /// Snapshot `m` now: the TA block's registers and the manager's block
    /// at `table` in `vram`. A read past the bus's time-out ends it (a
    /// wedged chip: the rest would cost ~10 ms a word). The line said.
    pub fn take(
        &mut self,
        m: Moment,
        regs: &mut Mmio,
        vram: &[u8],
        table: u32,
        (frame, ms, flags): (u32, u32, u32),
        log: fn(Arguments<'_>),
    ) {
        if !self.wants(m) {
            return;
        }
        self.kept |= 1 << m as u32;
        let k = m as usize;
        if m == Moment::Report {
            // the last report's kept as the one before
            let (a, b) = (Self::at(k, 0), Self::at(Moment::ReportBefore as usize, 0));
            self.mem.copy_within(a..a + SNAP_WORDS * 4, b);
        }
        let mut cut = false;
        self.put(k, 0, m as u32);
        self.put(k, 1, frame);
        self.put(k, 2, ms);
        let mut nz = [(0usize, 0u32); REG_WORDS];
        let mut n = 0;
        for r in 0..REG_WORDS {
            let t0 = rt::ticks();
            let v = regs.read32(MBX + REG_AT + 4 * r);
            if bus_timed_out(rt::ticks().wrapping_sub(t0)) {
                cut = true;
                break;
            }
            self.put(k, HEAD_WORDS + r, v);
            if v != 0 {
                nz[n] = (REG_AT + 4 * r, v);
                n += 1;
            }
        }
        let base = table as usize;
        let (mut used, mut changed) = (0u32, 0u32);
        // the words that moved since the set-up, as found (index, then,
        // now): run 281 showed 2 of 4,096 moving, the manager's pointers
        let mut moved = [(0usize, 0u32, 0u32); MOVED_SAID];
        let mut nm = 0;
        if !cut {
            for w in 0..TABLE_WORDS {
                let t0 = rt::ticks();
                let Some(v) = load32_volatile(vram, base + 4 * w) else {
                    cut = true;
                    break;
                };
                if w % 256 == 0 && bus_timed_out(rt::ticks().wrapping_sub(t0)) {
                    cut = true;
                    break;
                }
                self.put(k, HEAD_WORDS + REG_WORDS + w, v);
                used += u32::from(v != 0);
                if m != Moment::SetUp {
                    let was = self.get(Moment::SetUp as usize, HEAD_WORDS + REG_WORDS + w);
                    if v != was {
                        changed += 1;
                        if nm < MOVED_SAID {
                            moved[nm] = (w, was, v);
                            nm += 1;
                        }
                    }
                }
            }
        }
        let flags = flags | F_TAKEN | if cut { F_CUT } else { 0 };
        self.put(k, 3, flags);
        // Run 286 read the block: 16-bit links, entry p the page after page
        // p (the set-up's p + 1), the last page's back to the first: a ring
        // of the free pages; entry 0 the next page the TA takes. The ring's
        // length from there is the pages free now.
        let free = if cut { None } else { Some(self.free_pages(k)) };
        if m == Moment::SetUp {
            self.base_used = used;
        }
        log(format_args!(
            "rp-gfx: R340 PM watch {k} at frame {frame} ({ms} ms): flags 0x{flags:x}; pages free {} (the ring from page 0x{:x}); the manager's block {used} words in use ({} at the set-up), {changed} changed since it{}; the TA block's words not 0: {}",
            free.map_or(-1, |f| f as i32),
            self.link(k, 0),
            self.base_used,
            Moved(&moved[..nm]),
            Regs(&nz[..n]),
        ));
        if m == Moment::SetUp && !cut {
            // the block's shape: its first and last words as set up
            let w = |i: usize| self.get(k, HEAD_WORDS + REG_WORDS + i);
            log(format_args!(
                "rp-gfx: R340 PM watch: the block at the set-up: [0..8) {:x} {:x} {:x} {:x} {:x} {:x} {:x} {:x}; [1276..1284) {:x} {:x} {:x} {:x} {:x} {:x} {:x} {:x}; [4092..4096) {:x} {:x} {:x} {:x}",
                w(0),
                w(1),
                w(2),
                w(3),
                w(4),
                w(5),
                w(6),
                w(7),
                w(1276),
                w(1277),
                w(1278),
                w(1279),
                w(1280),
                w(1281),
                w(1282),
                w(1283),
                w(4092),
                w(4093),
                w(4094),
                w(4095)
            ));
        }
    }

    /// Entry `p` of the manager's block in slot `k` (16-bit links).
    fn link(&self, k: usize, p: usize) -> u32 {
        let w = self.get(k, HEAD_WORDS + REG_WORDS + p / 2);
        if p % 2 == 0 { w & 0xFFFF } else { w >> 16 }
    }

    /// The free ring's length from entry 0's page (at most the block's
    /// entries: a ring that never closes says so by its length).
    fn free_pages(&self, k: usize) -> u32 {
        let head = self.link(k, 0) as usize;
        let (mut p, mut n) = (head, 0u32);
        while n < 2 * TABLE_WORDS as u32 {
            if p == 0 || p >= 2 * TABLE_WORDS {
                break;
            }
            p = self.link(k, p) as usize;
            n += 1;
            if p == head {
                break;
            }
        }
        n
    }

    /// The file's bytes: the header and every slot (untaken ones zero).
    pub fn bytes(&self) -> &[u8] {
        self.mem
    }
}

/// Moved words said in a line.
const MOVED_SAID: usize = 6;

/// The moved words as ` (word 12: 0x5 -> 0x9, ...)`.
struct Moved<'a>(&'a [(usize, u32, u32)]);

impl core::fmt::Display for Moved<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.0.is_empty() {
            return Ok(());
        }
        f.write_str(" (")?;
        for (k, (i, was, now)) in self.0.iter().enumerate() {
            if k > 0 {
                f.write_str(", ")?;
            }
            write!(f, "word {i}: 0x{was:x} -> 0x{now:x}")?;
        }
        f.write_str(")")
    }
}

/// The registers as `0x804=0x1 0x828=0x...` (at most 32).
struct Regs<'a>(&'a [(usize, u32)]);

impl core::fmt::Display for Regs<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.0.is_empty() {
            return f.write_str("none");
        }
        for (k, (off, v)) in self.0.iter().enumerate() {
            if k > 0 {
                f.write_str(" ")?;
            }
            write!(f, "0x{off:x}=0x{v:x}")?;
        }
        Ok(())
    }
}
