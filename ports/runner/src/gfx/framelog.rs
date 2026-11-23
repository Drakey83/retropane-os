//! R340's next measure: the 2700G's two stages frame by frame, for a
//! certain overlap rule.
//!
//! Iterate, don't imitate (rule 0). Run 354 (the fight, one kit): kit
//! 113's overlap is worth 4% of the fight (17,276 ms strict, 16,589 with
//! run 314's guess, 16,614 free) and ~20% of a play (runs 314 and 316),
//! but a render that ends under a running TA spoils the frame (event bit
//! 9, R340), and run 314's guess at the two times (the last frame's)
//! armed 871 such frames in a play. Dell's queue never overlaps; PowerVR's
//! own drivers overlap with a parameter memory sized for it and the
//! chip's progress known to them. Ours, without the chip's documents,
//! measures what a certain rule can be built from: each TA's words and
//! time, each render's start and end, and each event, as they happen in
//! the game, so the PC can fit bounds (a TA's time from its own words; a
//! render's least time) and say how many frames a rule built on them
//! would have let overlap, and whether any of them would have ended under
//! a TA. A ring of events in the runner's RAM, written to the card at
//! the end (`gfx-frames.bin`); nothing read from the chip that the driver
//! does not read already. Cost: a few stores an event.

/// An event's kind (its first word's low byte).
#[derive(Clone, Copy)]
pub enum Kind {
    /// A frame's chain armed: words fed, descriptors, 1 if a render runs.
    Arm = 1,
    /// The TA said done: its microseconds from the arm.
    TaDone = 2,
    /// A render started.
    RenderStart = 3,
    /// A render ended: its microseconds.
    RenderDone = 4,
    /// The core's event bit 9.
    Ev9 = 5,
    /// A frame's chain built at the frame call, its arm put off: words,
    /// 1 if a render runs (the moment a rule would arm it beside).
    Prepare = 6,
}

/// The file's header (`RPGFXF1\0`, then events kept, the ring's size).
const MAGIC: &[u8; 8] = b"RPGFXF1\0";
const HEAD: usize = 16;
/// Events kept (16 bytes each: kind and frame, the time in ticks, two
/// values): the last ~3,000 frames of a run.
pub const EVENTS: usize = 16_384;
/// The room the runner gives it.
pub const ROOM: usize = HEAD + EVENTS * 16;
/// The card file.
pub const FILE: &str = "gfx-frames.bin";

pub struct FrameLog {
    mem: &'static mut [u8],
    n: u32,
}

impl FrameLog {
    /// The log in `mem` (at least [`ROOM`] bytes).
    pub fn new(mem: &'static mut [u8]) -> Option<Self> {
        let mem = mem.get_mut(..ROOM)?;
        mem[..HEAD].fill(0);
        mem[..8].copy_from_slice(MAGIC);
        mem[12..16].copy_from_slice(&(EVENTS as u32).to_le_bytes());
        Some(Self { mem, n: 0 })
    }

    /// One event at `ticks` (the 3.25 MHz clock).
    pub fn put(&mut self, k: Kind, frame: u32, ticks: u32, a: u32, b: u32) {
        let at = HEAD + (self.n as usize % EVENTS) * 16;
        let w = [(frame << 8) | k as u32, ticks, a, b];
        for (i, v) in w.iter().enumerate() {
            self.mem[at + 4 * i..at + 4 * i + 4].copy_from_slice(&v.to_le_bytes());
        }
        self.n = self.n.wrapping_add(1);
    }

    /// The file's bytes (the header's count set).
    pub fn bytes(&mut self) -> &[u8] {
        let n = self.n;
        self.mem[8..12].copy_from_slice(&n.to_le_bytes());
        let used = HEAD + 16 * (n as usize).min(EVENTS);
        &self.mem[..used]
    }
}
