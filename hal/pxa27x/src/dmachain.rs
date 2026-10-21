//! Descriptor chains for the DMA controller, streamed through a fixed ring
//! of slots: a transfer of any length runs on a few hundred bytes of
//! descriptors (pure logic, host-tested; [`crate::dma`] does the registers
//! and the cache).
//!
//! The controller loads a descriptor (16 bytes: next, source, target,
//! command) into its registers before it moves that descriptor's data, and
//! never reads it again. So once it has loaded descriptor `k`, `k`'s slot is
//! free for descriptor `k + slots`. The CPU tops the chain up whenever it
//! polls ([`Stream::poll`]): it writes the next descriptor with a STOP link,
//! then links the one before it to it. If the controller loaded that one
//! before the link was written, it stops after it; the next poll sees it
//! stopped short of the end and restarts it at the next descriptor. A slow
//! poller costs a pause, never a wrong byte (a device paced by its request
//! line just waits: the MMC controller stops the card clock while its FIFO
//! is full or empty, which is how programmed I/O already works).
//!
//! Where the controller is: `DDADR` (the next descriptor it will load) is
//! a slot address while the loaded descriptor links on; it names the slot,
//! and the slot plus what the CPU knows (the controller never loads a
//! descriptor not yet written, nor runs more than a ring ahead of the last
//! poll) gives the descriptor. Once it reads STOP, the controller has
//! loaded a descriptor of this transfer, so the address registers are this
//! transfer's (before that they may hold a previous transfer's, which is
//! why they are not trusted first).
//!
//! Before 2026-10-15 the chain was fixed at 32 descriptors of 4 KiB, and a
//! longer transfer was refused: the SD driver quietly read 1 MiB by
//! programmed I/O at a third of the speed (docs/research/storage-speed.md).

/// `DDADR` / descriptor link: stop after this descriptor.
pub const STOP: u32 = 1;
/// Bytes per descriptor slot.
pub const SLOT: u32 = 16;

/// A transfer, cut into descriptors of `chunk` bytes: descriptor `i` moves
/// `min(chunk, len - i * chunk)` bytes from `src + i * src_step` to
/// `dst + i * dst_step` (a step of 0: a device register, the same address
/// every time).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    /// First source address (physical).
    pub src: u32,
    /// First target address (physical).
    pub dst: u32,
    /// Source advance per descriptor (`chunk`, or 0 for a device).
    pub src_step: u32,
    /// Target advance per descriptor (`chunk`, or 0 for a device).
    pub dst_step: u32,
    /// Bytes per descriptor (under the 8,191 the length field holds).
    pub chunk: u32,
    /// Bytes in all.
    pub len: u32,
    /// The descriptor command word without its length.
    pub cmd: u32,
}

impl Shape {
    /// Descriptors in the whole transfer.
    #[must_use]
    pub const fn count(&self) -> u32 {
        self.len.div_ceil(self.chunk)
    }

    /// Descriptor `i`, with a STOP link.
    #[must_use]
    pub const fn descriptor(&self, i: u32) -> [u32; 4] {
        let piece = self.len - i * self.chunk;
        let piece = if piece < self.chunk {
            piece
        } else {
            self.chunk
        };
        [
            STOP,
            self.src + i * self.src_step,
            self.dst + i * self.dst_step,
            self.cmd | piece,
        ]
    }

    /// Bytes moved so far, from the address registers (the side that
    /// advances; capped at the length so a wild value can't run ahead).
    #[must_use]
    pub const fn moved(&self, dsadr: u32, dtadr: u32) -> u32 {
        let m = if self.dst_step != 0 {
            dtadr.wrapping_sub(self.dst)
        } else {
            dsadr.wrapping_sub(self.src)
        };
        if m < self.len { m } else { self.len }
    }

    /// Whether the shape can be streamed: something to move, whole steps,
    /// a length field that holds the chunk, and no address that wraps.
    #[must_use]
    pub fn valid(&self) -> bool {
        self.checked_count().is_some()
    }

    /// [`Shape::count`] if the shape is [`valid`](Self::valid) (one
    /// division: the ARMv5TE has none in hardware).
    fn checked_count(&self) -> Option<u32> {
        if self.len == 0
            || self.chunk == 0
            || self.chunk > 0x1FFF
            || (self.src_step != 0 && self.src_step != self.chunk)
            || (self.dst_step != 0 && self.dst_step != self.chunk)
        {
            return None;
        }
        let count = self.count();
        let end = |base: u32, step: u32| {
            base.checked_add((count - 1).saturating_mul(step))
                .and_then(|last| last.checked_add(if step == 0 { 0 } else { self.chunk }))
                .is_some()
        };
        (end(self.src, self.src_step) && end(self.dst, self.dst_step)).then_some(count)
    }
}

/// The controller's registers, as read for one poll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seen {
    /// `DDADR`.
    pub ddadr: u32,
    /// `DSADR`.
    pub dsadr: u32,
    /// `DTADR`.
    pub dtadr: u32,
    /// `DCSR` said the channel has stopped (read before the others).
    pub stopped: bool,
}

/// An address register of the channel, for [`snapshot`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Addr {
    /// `DDADR`.
    Ddadr,
    /// `DSADR`.
    Dsadr,
    /// `DTADR`.
    Dtadr,
}

/// Reads of a running channel [`snapshot`] makes at most before giving up
/// on an agreeing set (each costs a piece's time to break: never in
/// practice).
pub const SNAPSHOT_TRIES: u32 = 8;

/// The address registers for one poll, agreeing with each other. They
/// are separate reads: a running channel can load its next descriptor
/// between them, leaving `DDADR` naming one piece and `DSADR` inside the
/// next (quality audit C17: `emu-audio-s2`'s one failed ring end read
/// `DDADR` `0xA801EED0`, the link after piece 28, with `DSADR` 96 bytes
/// into piece 29). So `DDADR` is read again after the others, and all of
/// them again until it hasn't moved. `stopped` is `DCSR`'s, read before:
/// a stopped channel's registers are final and read once. After
/// [`SNAPSHOT_TRIES`] the last reading is returned as it is (the caller
/// then sees registers that make no sense, and says so).
pub fn snapshot(stopped: bool, mut read: impl FnMut(Addr) -> u32) -> Seen {
    let mut ddadr = read(Addr::Ddadr);
    let mut tries = 1;
    loop {
        let dsadr = read(Addr::Dsadr);
        let dtadr = read(Addr::Dtadr);
        let seen = Seen {
            ddadr,
            dsadr,
            dtadr,
            stopped,
        };
        if stopped || tries == SNAPSHOT_TRIES {
            return seen;
        }
        let again = read(Addr::Ddadr);
        if again == ddadr {
            return seen;
        }
        ddadr = again;
        tries += 1;
    }
}

/// What the caller does after a poll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// Still running.
    Running,
    /// Everything moved.
    Done,
    /// Stopped short of the end: start it again at this descriptor address.
    Resume(u32),
    /// The registers make no sense for this transfer: stop it, an error.
    Failed,
}

/// One transfer streaming through a ring of descriptor slots at physical
/// address `base`.
#[derive(Clone, Copy, Debug)]
pub struct Stream {
    shape: Shape,
    base: u32,
    /// Descriptors in the whole transfer ([`Shape::count`], kept).
    count: u32,
    /// Descriptors written to the ring so far.
    written: u32,
    /// Descriptors the controller has certainly loaded.
    loaded: u32,
}

impl Stream {
    /// Writes the first descriptors (as many as the ring holds, linked);
    /// the caller writes the ring back to memory and starts the controller
    /// at `base`. `None` if the shape is invalid.
    pub fn start(shape: Shape, ring: &mut [[u32; 4]], base: u32) -> Option<Self> {
        let count = shape.checked_count()?;
        if ring.is_empty() || !base.is_multiple_of(SLOT) {
            return None;
        }
        // Nothing runs yet: each descriptor is written once, already linked
        // (what `top_up` ends with, without its STOP-then-link steps).
        let n = count.min(ring.len() as u32);
        for (i, d) in (0..n).zip(ring.iter_mut()) {
            *d = shape.descriptor(i);
            if i + 1 < n {
                d[0] = base + SLOT * (i + 1);
            }
        }
        Some(Self {
            shape,
            base,
            count,
            written: n,
            loaded: 0,
        })
    }

    /// Descriptors in the whole transfer.
    #[must_use]
    pub const fn count(&self) -> u32 {
        self.count
    }

    /// The transfer.
    #[must_use]
    pub const fn shape(&self) -> &Shape {
        &self.shape
    }

    /// Descriptors written so far (diagnostics, tests).
    #[must_use]
    pub const fn written(&self) -> u32 {
        self.written
    }

    fn slot_addr(&self, slots: u32, i: u32) -> u32 {
        self.base + SLOT * (i % slots)
    }

    /// Writes descriptors into slots the controller is done with. `clean`
    /// is told each slot written, in order, and must make it visible to the
    /// controller before returning (the new descriptor before the link to
    /// it).
    fn top_up(&mut self, ring: &mut [[u32; 4]], clean: &mut dyn FnMut(usize)) {
        let slots = ring.len() as u32;
        let end = self.count.min(self.loaded + slots);
        if self.written >= end {
            return;
        }
        // Descriptor `written`'s slot, stepped rather than divided per
        // descriptor: the ARMv5TE has no divide instruction, so each `%` is
        // a library call (the emulator's SD benches lost 4-20% to them,
        // 2026-10-15).
        let mut slot = (self.written % slots) as usize;
        while self.written < end {
            ring[slot] = self.shape.descriptor(self.written);
            clean(slot);
            if self.written > 0 {
                let prev = if slot == 0 { ring.len() - 1 } else { slot - 1 };
                ring[prev][0] = self.base + SLOT * slot as u32;
                clean(prev);
            }
            self.written += 1;
            slot = if slot + 1 == ring.len() { 0 } else { slot + 1 };
        }
    }

    /// One poll: works out where the controller is from `seen`, tops the
    /// ring up, and says what to do next.
    pub fn poll(
        &mut self,
        ring: &mut [[u32; 4]],
        seen: Seen,
        clean: &mut dyn FnMut(usize),
    ) -> Next {
        let slots = ring.len() as u32;
        let chunk = self.shape.chunk;
        if seen.ddadr & STOP == 0 {
            // The next descriptor's slot: it is `loaded` plus how far the
            // slot is ahead of `loaded`'s (less than a ring: see the module
            // notes).
            let off = seen.ddadr.wrapping_sub(self.base);
            if !off.is_multiple_of(SLOT) || off / SLOT >= slots {
                return Next::Failed;
            }
            let ahead = (off / SLOT + slots - self.loaded % slots) % slots;
            self.loaded += ahead;
        } else {
            // This transfer's registers: the descriptor running (or just
            // finished) is the one the moved bytes are in, so at least the
            // ones before it, and it, were loaded.
            let moved = self.shape.moved(seen.dsadr, seen.dtadr);
            let at_least = (moved / chunk).min(self.count);
            self.loaded = self.loaded.max(at_least);
        }
        if self.loaded > self.written {
            return Next::Failed;
        }
        if !seen.stopped {
            self.top_up(ring, clean);
            return Next::Running;
        }
        // Stopped: at a STOP link, after a whole descriptor.
        let moved = self.shape.moved(seen.dsadr, seen.dtadr);
        if moved == self.shape.len {
            return Next::Done;
        }
        if seen.ddadr & STOP == 0 || !moved.is_multiple_of(chunk) {
            return Next::Failed;
        }
        let next = moved / chunk;
        self.loaded = next;
        self.top_up(ring, clean);
        if next < self.written {
            Next::Resume(self.slot_addr(slots, next))
        } else {
            Next::Failed
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    /// `snapshot` over a scripted run of register reads.
    fn scripted(stopped: bool, reads: &[(Addr, u32)]) -> (Seen, usize) {
        let mut k = 0;
        let seen = snapshot(stopped, |a| {
            let (want, v) = reads[k];
            assert_eq!(a, want, "read {k}");
            k += 1;
            v
        });
        (seen, k)
    }

    #[test]
    fn a_snapshot_of_a_running_channel_reads_ddadr_again() {
        use Addr::{Ddadr, Dsadr, Dtadr};
        let (seen, n) = scripted(
            false,
            &[(Ddadr, 0x10), (Dsadr, 0x500), (Dtadr, 7), (Ddadr, 0x10)],
        );
        assert_eq!(n, 4);
        assert_eq!((seen.ddadr, seen.dsadr, seen.dtadr), (0x10, 0x500, 7));
    }

    /// C17's reading: the next descriptor loaded between `DDADR` and
    /// `DSADR`. The torn pair is never returned.
    #[test]
    fn a_descriptor_loaded_between_the_reads_is_read_again() {
        use Addr::{Ddadr, Dsadr, Dtadr};
        let (seen, n) = scripted(
            false,
            &[
                (Ddadr, 0xA801_EED0),
                (Dsadr, 0xA880_9FE0),
                (Dtadr, 1),
                (Ddadr, 0xA801_EEE0),
                (Dsadr, 0xA880_9FE4),
                (Dtadr, 1),
                (Ddadr, 0xA801_EEE0),
            ],
        );
        assert_eq!(n, 7);
        assert_eq!((seen.ddadr, seen.dsadr), (0xA801_EEE0, 0xA880_9FE4));
        assert!(!seen.stopped);
    }

    #[test]
    fn a_stopped_channel_is_read_once() {
        use Addr::{Ddadr, Dsadr, Dtadr};
        let (seen, n) = scripted(true, &[(Ddadr, 0x11), (Dsadr, 0x600), (Dtadr, 3)]);
        assert_eq!(n, 3);
        assert!(seen.stopped);
        assert_eq!((seen.ddadr, seen.dsadr), (0x11, 0x600));
    }

    /// A `DDADR` that never settles: the last reading, after
    /// `SNAPSHOT_TRIES` passes, no more reads.
    #[test]
    fn a_snapshot_gives_up_after_its_tries() {
        let mut reads = 0u32;
        let mut d = 0u32;
        let seen = snapshot(false, |a| {
            reads += 1;
            if a == Addr::Ddadr {
                d += 0x10;
                d
            } else {
                0x700
            }
        });
        // Each pass: DSADR, DTADR; between passes DDADR; the first DDADR.
        assert_eq!(reads, 1 + 2 * SNAPSHOT_TRIES + (SNAPSHOT_TRIES - 1));
        assert_eq!(seen.ddadr, 0x10 * SNAPSHOT_TRIES);
    }
    use std::vec;
    use std::vec::Vec;

    const SLOTS: usize = 32;
    const RING: u32 = 0x1000;
    const DEVICE: u32 = 0x4110_0040;
    const BUF: u32 = 0xA010_0000;
    const INC_SRC: u32 = 1 << 31;
    const INC_TRG: u32 = 1 << 30;

    /// The DMA controller as the PXA27x manual describes it: loads a
    /// descriptor from memory (not the CPU's cache) into its registers,
    /// moves its bytes a burst at a time, follows the link or stops.
    struct Controller {
        ddadr: u32,
        dsadr: u32,
        dtadr: u32,
        left: u32,
        cmd: u32,
        running: bool,
        /// Descriptors loaded, in order, as loaded.
        loads: Vec<[u32; 4]>,
    }

    /// What the controller and the CPU see: memory (the ring as written
    /// back, the buffer), the CPU's cached ring, and a device FIFO.
    struct World {
        memory_ring: [[u32; 4]; SLOTS],
        cache_ring: [[u32; 4]; SLOTS],
        buf: Vec<u8>,
        /// Bytes the device has produced (reads) or taken (writes).
        device: Vec<u8>,
        device_at: usize,
    }

    impl Controller {
        fn new(stale: u32) -> Self {
            Self {
                ddadr: 0,
                dsadr: stale,
                dtadr: stale,
                left: 0,
                cmd: 0,
                running: false,
                loads: Vec::new(),
            }
        }

        fn start(&mut self, at: u32) {
            self.ddadr = at;
            self.running = true;
            self.left = 0;
        }

        /// One step: a load or a 32-byte burst. False once stopped.
        fn step(&mut self, w: &mut World) -> bool {
            if !self.running {
                return false;
            }
            if self.left == 0 {
                if self.ddadr & STOP != 0 {
                    self.running = false;
                    return false;
                }
                let slot = ((self.ddadr - RING) / SLOT) as usize;
                let d = w.memory_ring[slot];
                self.loads.push(d);
                self.ddadr = d[0];
                self.dsadr = d[1];
                self.dtadr = d[2];
                self.cmd = d[3];
                self.left = d[3] & 0x1FFF;
                return true;
            }
            let n = self.left.min(32);
            for _ in 0..n {
                let byte = if self.dsadr == DEVICE {
                    let b = w.device[w.device_at];
                    w.device_at += 1;
                    b
                } else {
                    w.buf[(self.dsadr - BUF) as usize]
                };
                if self.dtadr == DEVICE {
                    w.device.push(byte);
                } else {
                    w.buf[(self.dtadr - BUF) as usize] = byte;
                }
                if self.cmd & INC_SRC != 0 {
                    self.dsadr += 1;
                }
                if self.cmd & INC_TRG != 0 {
                    self.dtadr += 1;
                }
            }
            self.left -= n;
            if self.left == 0 && self.ddadr & STOP != 0 {
                self.running = false;
            }
            true
        }
    }

    /// A small deterministic generator (no float, no crates).
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            (self.0 >> 33) as u32
        }
    }

    fn pattern(i: usize) -> u8 {
        (i.wrapping_mul(131) ^ (i >> 9)) as u8
    }

    /// Runs one transfer with the CPU polling after a random number of
    /// controller steps (from none to more than a ring's worth), and the
    /// address registers left holding a stale value from "the previous
    /// transfer". Checks every byte, and that every descriptor the
    /// controller loaded was the one meant for that point.
    fn run(len: u32, to_device: bool, seed: u64, max_gap: u32) {
        let mut rng = Lcg(seed);
        let chunk = 4096;
        let shape = if to_device {
            Shape {
                src: BUF,
                dst: DEVICE,
                src_step: chunk,
                dst_step: 0,
                chunk,
                len,
                cmd: INC_SRC,
            }
        } else {
            Shape {
                src: DEVICE,
                dst: BUF,
                src_step: 0,
                dst_step: chunk,
                chunk,
                len,
                cmd: INC_TRG,
            }
        };
        let mut w = World {
            memory_ring: [[0xDEAD_BEEF; 4]; SLOTS],
            cache_ring: [[0; 4]; SLOTS],
            buf: if to_device {
                (0..len as usize).map(pattern).collect()
            } else {
                vec![0xEE; len as usize]
            },
            device: if to_device {
                Vec::new()
            } else {
                (0..len as usize).map(pattern).collect()
            },
            device_at: 0,
        };
        // Stale: the end of a previous transfer of the same buffer.
        let mut c = Controller::new(BUF + len);
        let mut s = Stream::start(shape, &mut w.cache_ring, RING).unwrap();
        w.memory_ring = w.cache_ring;
        c.start(RING);
        let mut polls = 0u32;
        loop {
            let gap = rng.next() % (max_gap + 1);
            for _ in 0..gap {
                if !c.step(&mut w) {
                    break;
                }
            }
            let seen = Seen {
                stopped: !c.running,
                ddadr: c.ddadr,
                dsadr: c.dsadr,
                dtadr: c.dtadr,
            };
            let mut cleaned = Vec::new();
            let next = s.poll(&mut w.cache_ring, seen, &mut |slot| cleaned.push(slot));
            for slot in cleaned {
                w.memory_ring[slot] = w.cache_ring[slot];
            }
            polls += 1;
            assert!(polls < 10_000_000, "never finished");
            match next {
                Next::Running => {}
                Next::Done => break,
                Next::Resume(at) => {
                    c.start(at);
                }
                Next::Failed => panic!("failed: len {len} seed {seed}"),
            }
        }
        // Every byte where it belongs.
        let moved = if to_device { &w.device } else { &w.buf };
        assert_eq!(moved.len(), len as usize);
        for (i, &b) in moved.iter().enumerate() {
            assert_eq!(b, pattern(i), "byte {i} of {len} (seed {seed})");
        }
        // Each descriptor loaded once, in order, the right one.
        assert_eq!(c.loads.len() as u32, shape.count(), "len {len}");
        for (i, d) in c.loads.iter().enumerate() {
            assert_eq!(d[1..], shape.descriptor(i as u32)[1..], "descriptor {i}");
        }
    }

    /// Every size class: one burst, under a descriptor, one, a partial
    /// last one, exactly the ring, one over, the old 128 KiB limit, 1 MiB,
    /// 4 MiB; both directions; pollers from eager to very lazy.
    #[test]
    fn every_size_moves_every_byte_whatever_the_poll_timing() {
        let sizes = [
            32,
            512,
            1024,
            4096 - 32,
            4096,
            4096 + 512,
            8192,
            31 * 4096,
            32 * 4096,
            32 * 4096 + 512,
            33 * 4096,
            64 * 4096 - 512,
            256 * 1024,
            1024 * 1024,
            4 * 1024 * 1024,
        ];
        for &len in &sizes {
            for to_device in [false, true] {
                for (seed, gap) in [(1, 1), (2, 3), (3, 200), (4, 5000), (5, 20_000)] {
                    run(len, to_device, seed + u64::from(len), gap);
                }
            }
        }
    }

    #[test]
    fn a_lazy_poller_makes_the_controller_stop_and_resume() {
        // The controller runs the whole ring between polls: it must stop
        // at the last written descriptor and be resumed, not loop round.
        run(1024 * 1024, false, 9, 1_000_000);
    }

    #[test]
    fn the_ring_holds_the_first_descriptors_linked() {
        let shape = Shape {
            src: DEVICE,
            dst: BUF,
            src_step: 0,
            dst_step: 4096,
            chunk: 4096,
            len: 3 * 4096 + 512,
            cmd: INC_TRG,
        };
        let mut ring = [[0; 4]; 8];
        let s = Stream::start(shape, &mut ring, RING).unwrap();
        assert_eq!(s.written(), 4);
        assert_eq!(ring[0], [RING + 16, DEVICE, BUF, INC_TRG | 4096]);
        assert_eq!(ring[2][0], RING + 48);
        assert_eq!(ring[3], [STOP, DEVICE, BUF + 3 * 4096, INC_TRG | 512]);
    }

    #[test]
    fn invalid_shapes_are_refused() {
        let ok = Shape {
            src: DEVICE,
            dst: BUF,
            src_step: 0,
            dst_step: 4096,
            chunk: 4096,
            len: 4096,
            cmd: 0,
        };
        let mut ring = [[0; 4]; 4];
        assert!(Stream::start(ok, &mut ring, RING).is_some());
        for bad in [
            Shape { len: 0, ..ok },
            Shape { chunk: 0, ..ok },
            Shape {
                chunk: 8192,
                dst_step: 8192,
                ..ok
            },
            Shape {
                dst_step: 512,
                ..ok
            },
            Shape {
                dst: 0xFFFF_F000,
                len: 8192,
                ..ok
            },
        ] {
            assert!(Stream::start(bad, &mut ring, RING).is_none(), "{bad:?}");
        }
        assert!(Stream::start(ok, &mut [], RING).is_none());
        assert!(Stream::start(ok, &mut ring, RING + 4).is_none());
    }

    #[test]
    fn registers_that_make_no_sense_fail_instead_of_guessing() {
        let shape = Shape {
            src: DEVICE,
            dst: BUF,
            src_step: 0,
            dst_step: 4096,
            chunk: 4096,
            len: 64 * 4096,
            cmd: INC_TRG,
        };
        let mut ring = [[0; 4]; 4];
        let mut s = Stream::start(shape, &mut ring, RING).unwrap();
        let seen = Seen {
            ddadr: 0x5000,
            dsadr: DEVICE,
            dtadr: BUF,
            stopped: false,
        };
        assert_eq!(s.poll(&mut ring, seen, &mut |_| {}), Next::Failed);
        // Stopped mid-descriptor (not at a STOP link's end).
        let mut s = Stream::start(shape, &mut ring, RING).unwrap();
        let seen = Seen {
            ddadr: STOP,
            dsadr: DEVICE,
            dtadr: BUF + 100,
            stopped: true,
        };
        assert_eq!(s.poll(&mut ring, seen, &mut |_| {}), Next::Failed);
    }
}
