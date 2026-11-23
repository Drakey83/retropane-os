//! Game Mode's full tier (docs/design/game-mode-build.md G8): the frozen
//! session's compressed pages written to a preallocated swap file on the
//! card for the length of a game, and read back after it.
//!
//! What goes out is the compressed store's own frames, as they are: each is
//! already packed with compressed pages of one size class, under 64 bytes
//! wasted a page, so the card gets compressed bytes only, with no second
//! packing pass and no staging buffer. They go lowest frame first, 256 to a
//! run, each run one 1 MiB DMA transfer gathered straight from the frames
//! (the size measured fastest on the X51v, 2026-11-23: 5.0 MB/s out, 7.3
//! MB/s back, against 2.9 MB/s at 64 KiB). The page map is the set of frames
//! itself (2 KiB, one bit a frame): a frame's place in the file is its rank
//! in the set, so the record holds no table a page. Coming back, each frame
//! is put back at the frame it left ([`Frames::take`]), so every handle in
//! the page tables is right with nothing rewritten; the few a long-lived
//! kernel allocation took meanwhile come back elsewhere and are rehomed
//! ([`Moves`]). Each run carries an XXH32 chained over its frames, checked
//! on the way back (a run that fails is read once more before the return
//! is refused, with nothing lost: the record still names the file).
//!
//! Rule 0: Linux's swap writes 4 KiB pages (clusters at best) as reclaim
//! finds them, with a swap map entry and a swap-cache entry a page; zswap's
//! writeback decompresses before it writes. CE has no swap. Ours writes
//! only compressed bytes, once, in the card's fastest size, when nothing
//! else uses the card, with a 2 KiB map, and brings each frame back where
//! it was so no table is rebuilt.

use rp_kernel_mm::frames::Frames;
use rp_kernel_mm::pagetable::FrameAlloc;
use rp_kernel_mm::zpool::{AttachError, CLASSES, FrameSet, MAX_MOVES, Moves, Pages, Parked, ZPool};

use crate::xxh32;

/// Frames a run: 1 MiB.
pub const RUN_FRAMES: usize = 256;
/// Runs at most: the whole 64 MiB.
pub const MAX_RUNS: usize = rp_kernel_mm::zpool::MAX_FRAMES / RUN_FRAMES;

const PAGE: usize = 4096;

/// The card side: the preallocated swap file, its blocks found once at
/// boot. Run `n` is the megabyte at `n × 1 MiB` in the file. The frames are
/// given lowest first; a run may be short (the last). The device's card
/// gathers and scatters by DMA from the frames' physical addresses, cleaning
/// their cache lines before a write and invalidating them after a read;
/// `mem` reaches the same frames for a copy (the emulator's card, tests).
pub trait Card {
    /// Writes run `run`, gathered from `frames` in order.
    fn write_run<P: Pages>(
        &mut self,
        mem: &mut P,
        run: u32,
        frames: &[u16],
    ) -> Result<(), CardError>;
    /// Reads run `run` back, scattered into `frames` in order.
    fn read_run<P: Pages>(
        &mut self,
        mem: &mut P,
        run: u32,
        frames: &[u16],
    ) -> Result<(), CardError>;
}

/// The card refused or failed a transfer.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub struct CardError;

/// Why the tier didn't go out or come back.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum TierError {
    /// The card failed run `n` (going out: nothing freed, the store as it
    /// was; coming back: the frames given back, the record still good).
    Card(u32),
    /// Run `n` read back twice with the wrong hash (frames given back, the
    /// record still good).
    Damaged(u32),
    /// More of the store's frames are taken than may move ([`MAX_MOVES`]),
    /// or no frame is free to move one to: nothing changed.
    Occupied(u32),
    /// The store holds pages already (the game's must go first): nothing
    /// changed.
    Busy,
    /// The tier is already out, or isn't out.
    State,
    /// A record's page map doesn't hold together.
    BadMap,
}

/// The tier's page map: the store's parked state and each run's hash. The
/// kernel keeps one in a static (2.4 KiB) and in the return record
/// ([`TierMap::encode`]).
#[derive(Clone, PartialEq, Eq)]
pub struct TierMap {
    /// The store as it went out.
    pub parked: Parked,
    /// Runs written.
    pub runs: u32,
    /// Each run's hash.
    pub hashes: [u32; MAX_RUNS],
    /// Whether the tier is out (on the card, the frames freed).
    pub out: bool,
}

impl Default for TierMap {
    fn default() -> Self {
        Self::new()
    }
}

/// What went out or came back.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(test, derive(Debug))]
pub struct Moved {
    /// Frames.
    pub frames: u32,
    /// Runs (1 MiB DMA transfers).
    pub runs: u32,
    /// Frames that came back somewhere else.
    pub moved: u32,
    /// Runs read twice before their hash agreed.
    pub retried: u32,
}

/// A run's hash: XXH32 chained frame to frame (each frame's hash seeds the
/// next), seeded with the run's number so a run read from the wrong place
/// fails.
fn run_hash(mem: &mut impl Pages, run: u32, frames: &[u16]) -> u32 {
    frames
        .iter()
        .fold(run, |h, &f| xxh32(mem.page(usize::from(f)), h))
}

/// The next run's frames from `frames` (the set, lowest first) into `buf`;
/// how many.
fn next_run(frames: &mut impl Iterator<Item = usize>, buf: &mut [u16; RUN_FRAMES]) -> usize {
    let mut n = 0;
    for (slot, f) in buf.iter_mut().zip(frames) {
        *slot = f as u16;
        n += 1;
    }
    n
}

impl TierMap {
    /// Nothing out.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            parked: Parked::new(),
            runs: 0,
            hashes: [0; MAX_RUNS],
            out: false,
        }
    }

    /// Writes the store's frames to the card and frees them, leaving the
    /// store empty for the game. If the card fails, nothing is freed and
    /// the store is as it was (Game Mode then runs with the fast tier
    /// only, or is refused: the mode manager's call).
    pub fn swap_out(
        &mut self,
        zpool: &mut ZPool,
        frames: &mut impl FrameAlloc,
        mem: &mut impl Pages,
        card: &mut impl Card,
    ) -> Result<Moved, TierError> {
        if self.out {
            return Err(TierError::State);
        }
        let count = zpool.frame_set().len();
        let runs = count.div_ceil(RUN_FRAMES);
        {
            let mut buf = [0u16; RUN_FRAMES];
            let mut all = zpool.frame_set().iter();
            for (run, hash) in self.hashes[..runs].iter_mut().enumerate() {
                let n = next_run(&mut all, &mut buf);
                let r = run as u32;
                *hash = run_hash(mem, r, &buf[..n]);
                card.write_run(mem, r, &buf[..n])
                    .map_err(|_| TierError::Card(r))?;
            }
        }
        zpool.detach(&mut self.parked);
        for f in self.parked.set.iter() {
            frames.free(f);
        }
        self.runs = runs as u32;
        self.out = true;
        Ok(Moved {
            frames: count as u32,
            runs: runs as u32,
            ..Moved::default()
        })
    }

    /// Reads the store's frames back to the frames they left (those taken
    /// meanwhile to free ones, listed in `moves` for the VM's
    /// [`rehome`](rp_kernel_mm::space::Vm::rehome) of every frozen space),
    /// checks each run's hash, and puts the store back. On any failure every
    /// frame taken is given back and the map stays out, so the return can
    /// be tried again.
    pub fn swap_in<const N: usize>(
        &mut self,
        zpool: &mut ZPool,
        frames: &mut Frames<N>,
        mem: &mut impl Pages,
        card: &mut impl Card,
        moves: &mut Moves,
    ) -> Result<Moved, TierError> {
        if !self.out {
            return Err(TierError::State);
        }
        let (stored, held, _) = zpool.stats();
        if stored != 0 || held != 0 {
            return Err(TierError::Busy);
        }
        let set = &self.parked.set;
        // Before anything changes: how many must move (no room for them is
        // found below, and every take given back).
        let taken = set.iter().filter(|&f| !frames.is_free(f)).count();
        if taken > MAX_MOVES {
            return Err(TierError::Occupied(taken as u32));
        }
        *moves = Moves::new();
        // Every free one taken back first; only then are the rest moved, so
        // a frame handed out for a move is never one the set still needs.
        for f in set.iter() {
            if !frames.take(f) {
                let _ = moves.push(f, usize::MAX >> 1);
            }
        }
        let mut placed = Moves::new();
        for f in set.iter().filter(|&f| moves.get(f).is_some()) {
            match FrameAlloc::alloc(frames, 0) {
                Some(to) => {
                    let _ = placed.push(f, to);
                }
                None => {
                    give_back(frames, set, &placed, moves);
                    return Err(TierError::Occupied(taken as u32));
                }
            }
        }
        *moves = placed;
        let mut buf = [0u16; RUN_FRAMES];
        let mut done = Moved {
            frames: set.len() as u32,
            runs: self.runs,
            moved: moves.len() as u32,
            retried: 0,
        };
        let mut all = set.iter();
        for (run, &want) in self.hashes[..self.runs as usize].iter().enumerate() {
            let n = next_run(&mut all, &mut buf);
            for f in &mut buf[..n] {
                *f = moves.map(usize::from(*f)) as u16;
            }
            let r = run as u32;
            let mut ok = false;
            for attempt in 0..2 {
                if card.read_run(mem, r, &buf[..n]).is_err() {
                    give_back(frames, set, moves, moves);
                    return Err(TierError::Card(r));
                }
                if run_hash(mem, r, &buf[..n]) == want {
                    done.retried += attempt;
                    ok = true;
                    break;
                }
            }
            if !ok {
                give_back(frames, set, moves, moves);
                return Err(TierError::Damaged(r));
            }
        }
        if let Err(e) = zpool.attach(mem, &self.parked, moves) {
            give_back(frames, set, moves, moves);
            return Err(match e {
                AttachError::Busy => TierError::Busy,
                AttachError::Broken => TierError::BadMap,
            });
        }
        self.out = false;
        Ok(done)
    }

    /// Bytes [`encode`](Self::encode) writes.
    #[must_use]
    pub fn encoded_len(&self) -> usize {
        let words = self.used_words();
        FIXED + 4 * self.runs as usize + 4 * words
    }

    fn used_words(&self) -> usize {
        let w = self.parked.set.words();
        w.iter().rposition(|&x| x != 0).map_or(0, |i| i + 1)
    }

    /// Writes the map into a return-record section (`out` is
    /// [`encoded_len`](Self::encoded_len) bytes): counts, list heads, run
    /// hashes, then the set's words up to its last frame.
    pub fn encode(&self, out: &mut [u8]) {
        let words = self.used_words();
        let mut at = 0;
        let mut put = |b: &[u8]| {
            out[at..at + b.len()].copy_from_slice(b);
            at += b.len();
        };
        put(&u32::from(self.out).to_le_bytes());
        put(&self.runs.to_le_bytes());
        put(&(words as u32).to_le_bytes());
        for c in self.parked.counts {
            put(&c.to_le_bytes());
        }
        for p in self.parked.partial {
            put(&p.to_le_bytes());
        }
        put(&[0; PARTIAL_PAD]);
        for h in &self.hashes[..self.runs as usize] {
            put(&h.to_le_bytes());
        }
        for w in &self.parked.set.words()[..words] {
            put(&w.to_le_bytes());
        }
    }

    /// Reads a map back from a record section, checking that it holds
    /// together (every length bounded, the frame count and the runs agree
    /// with the set, every list head a frame of the set).
    pub fn decode(&mut self, b: &[u8]) -> Result<(), TierError> {
        if b.len() < FIXED {
            return Err(TierError::BadMap);
        }
        let word = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let out = word(0);
        let runs = word(4) as usize;
        let words = word(8) as usize;
        if out > 1 || runs > MAX_RUNS || words > FrameSet::WORDS {
            return Err(TierError::BadMap);
        }
        if b.len() != FIXED + 4 * runs + 4 * words {
            return Err(TierError::BadMap);
        }
        let mut m = Parked::new();
        for (i, c) in m.counts.iter_mut().enumerate() {
            *c = word(12 + 4 * i);
        }
        for (i, p) in m.partial.iter_mut().enumerate() {
            *p = u16::from_le_bytes([b[24 + 2 * i], b[25 + 2 * i]]);
        }
        let hashes = FIXED;
        let set_at = hashes + 4 * runs;
        for (i, w) in m.set.words_mut()[..words].iter_mut().enumerate() {
            *w = word(set_at + 4 * i);
        }
        let n = m.set.len();
        if m.counts[0] as usize != n
            || n.div_ceil(RUN_FRAMES) != runs
            || m.partial[0] != 0
            || m.partial[1..]
                .iter()
                .any(|&p| p != 0 && !m.set.contains(usize::from(p) - 1))
        {
            return Err(TierError::BadMap);
        }
        self.parked = m;
        self.runs = runs as u32;
        self.hashes = [0; MAX_RUNS];
        for (i, h) in self.hashes[..runs].iter_mut().enumerate() {
            *h = word(hashes + 4 * i);
        }
        self.out = out == 1;
        Ok(())
    }
}

/// The fixed part of an encoded map: out, runs, words, three counts, the
/// list heads padded to a word.
const PARTIAL_PAD: usize = (4 - (2 * (CLASSES + 1)) % 4) % 4;
const FIXED: usize = 24 + 2 * (CLASSES + 1) + PARTIAL_PAD;

/// Undoes a failed return's takes: each frame of the set taken back at its
/// own frame (those not in `pending`, the ones that couldn't be) and each
/// placed elsewhere (`placed`) is freed again; nothing else is touched.
fn give_back<const N: usize>(
    frames: &mut Frames<N>,
    set: &FrameSet,
    placed: &Moves,
    pending: &Moves,
) {
    for f in set.iter() {
        let at = match (pending.get(f), placed.get(f)) {
            (None, _) => f,
            (Some(_), Some(to)) => to,
            (Some(_), None) => continue,
        };
        let _ = frames.free(at);
    }
}

const _: () = assert!(PAGE * RUN_FRAMES == 1 << 20);

#[cfg(test)]
#[path = "tier_tests.rs"]
mod tests;
