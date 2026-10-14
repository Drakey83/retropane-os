//! The MBX 2D engine of the Intel 2700G (ADR-0022): its command words, the
//! command FIFO's accounting, and a fenced command stream.
//!
//! Every word here is from docs/hw/2700g.md, "The MBX 2D engine": found in
//! Dell's driver (static analysis, no code copied) and measured on the
//! owner's X51v by `rp-mbx` where marked there. Commands are words stored
//! one after another into a 4 KiB window at `0x0FFF5000` (offset
//! [`PORT`] in the 2700G's register window); a command starts with a word
//! whose top nibble names it, and state words (surfaces, origin, key,
//! blend) stay set until replaced. The engine says nothing when it is done
//! (measured, 2026-10-14): completion is a "fence", a 1 × 1 fill of a
//! counter into a word of local memory that the CPU reads.
//!
//! Pure and host-tested; the register window comes in as a
//! [`RegisterBus`], the clock as a function, so the same code runs on the
//! device and in tests.

#![no_std]
#![forbid(unsafe_code)]

use rp_hal_api::RegisterBus;
use rp_hal_mem::load32_volatile;

/// The MBX core's registers in the 2700G register window (`0x0FFE0000`).
pub const MBX: usize = 0x4000;

/// MBX register offsets (all ones Dell's driver touches: reading any other
/// offset in the 2700G's register block can hang the bus).
pub mod reg {
    /// Core ID (`0x01020000` on the X51v).
    pub const ID: usize = 0xf00;
    /// Reset control: `0x111` asserts, 0 releases.
    pub const RESET: usize = 0x080;
    /// Interrupt status; bits 16-23 and 30 are also the command FIFO's level.
    pub const STATUS: usize = 0x12c;
    /// Interrupt enables.
    pub const ENABLE: usize = 0x130;
    /// Interrupt clear (write 1s).
    pub const CLEAR: usize = 0x134;
}

/// The 2D command window (`0x0FFF5000`) in the register window: 4 KiB,
/// written sequentially, wrapping.
pub const PORT: usize = 0x1_5000;
/// The command window's size.
pub const PORT_SIZE: usize = 0x1000;

/// A pixel format code, as the surface words carry it (bits 15-19).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format(pub u32);

impl Format {
    /// 4 bpp coverage: a solid colour drawn through it (anti-aliased text).
    pub const COVERAGE4: Self = Self(0x2_8000);
    /// RGB 5:6:5.
    pub const RGB565: Self = Self(0x5_0000);
    /// 32 bits per pixel.
    pub const BPP32: Self = Self(0x6_0000);

    /// The format with code `n` (0-31), for probing codes not yet known.
    #[must_use]
    pub const fn code(n: u32) -> Self {
        Self((n & 0x1F) << 15)
    }
}

/// Pattern copy: fill with the colour.
pub const ROP_PATCOPY: u32 = 0xF0F0;
/// Source copy.
pub const ROP_SRCCOPY: u32 = 0xCCCC;
/// Ends a batch of commands (not a completion signal).
pub const FLUSH: u32 = 0x7000_0000;
/// The FIFO's capacity in words.
pub const FIFO_WORDS: u32 = 0x1FE;
/// No scaling: the driver sends it once at start-up.
pub const SCALE_1_TO_1: u32 = 0x6080_0200;

/// Operation header bit: no colour word follows.
pub const OP_NO_COLOUR: u32 = 1 << 16;
/// Operation header bit: the blend state applies (likely: Dell's
/// anti-aliased text sets it, with ROP `0xCCCC` and a 4 bpp coverage
/// source; `rp-mbx-blend` checks it).
pub const OP_BLEND: u32 = 1 << 17;
/// Operation header bit: the colour key applies (with [`OP_NO_COLOUR`], as
/// the driver sends it).
pub const OP_KEY: u32 = 1 << 20;
// Bit 23, the driver's "y not positive", is deliberately absent: it
// doesn't make the chip copy bottom-up (measured 2026-10-14). A copy down
// over itself is one copy with a negative stride on both surfaces, or
// strips as the fallback.
/// Operation header bit: copy right to left (for a destination right of an
/// overlapping source).
pub const OP_RIGHT_LEFT: u32 = 1 << 24;

/// The blend state Dell's driver sets for anti-aliased text.
pub const BLEND_DELL_TEXT: u32 = 0x0910_0000;

/// A coordinate word: 13-bit x and y.
#[must_use]
pub fn xy(x: u16, y: u16) -> u32 {
    u32::from(x & 0x1FFF) << 16 | u32::from(y & 0x1FFF)
}

/// A destination surface word pair. `stride` is in bytes and may be
/// negative (15-bit two's complement: the surface then walks up through
/// memory; measured 2026-10-14).
#[must_use]
pub fn dest(format: Format, addr: usize, stride: i32) -> [u32; 2] {
    [
        0xA000_0000 | format.0 | (stride as u32 & 0x7FFF),
        addr as u32 & 0x03FF_FFFC,
    ]
}

/// A source surface word pair (as [`dest`]).
#[must_use]
pub fn source(format: Format, addr: usize, stride: i32) -> [u32; 2] {
    [
        0x9400_0000 | format.0 | (stride as u32 & 0x7FFF),
        addr as u32 & 0x03FF_FFFC,
    ]
}

/// The source origin: where in the source the rectangle starts.
#[must_use]
pub fn origin(sx: u16, sy: u16) -> u32 {
    0x3000_0000 | u32::from(sx & 0x1FFF) << 14 | u32::from(sy & 0x1FFF)
}

/// The colour key: source pixels whose bits under `mask` equal `key`
/// leave the destination as it was (with [`OP_KEY`]). `key(0, 0)` with no
/// `OP_KEY` is the driver's "key off".
#[must_use]
pub fn key(key: u32, mask: u32) -> [u32; 3] {
    [0x2000_0001, key, mask]
}

/// The blend state word (`0x20000004, value`): stays set until replaced,
/// across images too (the 2700G isn't reset between them).
#[must_use]
pub fn blend(value: u32) -> [u32; 2] {
    [0x2000_0004, value]
}

/// A solid fill of `[x0, x1) × [y0, y1)` with `color` into the surface at
/// local-memory offset `addr`, `stride` bytes per line: seven words.
#[must_use]
pub fn fill(
    format: Format,
    addr: usize,
    stride: usize,
    (x0, y0, x1, y1): (u16, u16, u16, u16),
    color: u32,
) -> [u32; 7] {
    let [d, a] = dest(format, addr, stride as i32);
    [
        d,
        a,
        0x8000_0000 | ROP_PATCOPY,
        color,
        xy(x0, y0),
        xy(x1, y1),
        FLUSH,
    ]
}

/// The general operation: surfaces, origin, the header (`0x80000000 | rop
/// | flags`), its colour word (left out with [`OP_NO_COLOUR`]), the
/// destination rectangle, end of batch. Ten words, or nine.
#[must_use]
pub fn blit(
    (dst_format, dst, dst_stride): (Format, usize, i32),
    (src_format, src, src_stride): (Format, usize, i32),
    (sx, sy): (u16, u16),
    (x0, y0, x1, y1): (u16, u16, u16, u16),
    rop: u32,
    flags: u32,
    colour: u32,
) -> ([u32; 10], usize) {
    let [d, da] = dest(dst_format, dst, dst_stride);
    let [s, sa] = source(src_format, src, src_stride);
    let mut w = [
        d,
        da,
        s,
        sa,
        origin(sx, sy),
        0x8000_0000 | (rop & 0xFFFF) | flags,
        colour,
        xy(x0, y0),
        xy(x1, y1),
        FLUSH,
    ];
    if flags & OP_NO_COLOUR != 0 {
        w.copy_within(7.., 6);
        return (w, 9);
    }
    (w, 10)
}

/// A copy of the `[x0, x1) × [y0, y1)` destination rectangle from the
/// source surface at `src` (`src_stride`), starting at `(sx, sy)` there.
/// `flags`: [`OP_RIGHT_LEFT`] for a rightward overlap, or [`OP_KEY`].
/// Strides as [`dest`] takes them, given as 15-bit fields. Ten words (nine
/// keyed: no colour word).
#[must_use]
pub fn copy(
    format: Format,
    (dst, dst_stride): (usize, usize),
    (src, src_stride): (usize, usize),
    origin: (u16, u16),
    rect: (u16, u16, u16, u16),
    flags: u32,
) -> ([u32; 10], usize) {
    let flags = flags | if flags & OP_KEY != 0 { OP_NO_COLOUR } else { 0 };
    blit(
        (format, dst, dst_stride as i32),
        (format, src, src_stride as i32),
        origin,
        rect,
        ROP_SRCCOPY,
        flags,
        0, // the colour word the driver always sends (ignored for a copy)
    )
}

/// The fence: writes `n` into the 32-bit word at local-memory `addr`.
#[must_use]
pub fn fence(addr: usize, n: u32) -> [u32; 7] {
    fill(Format::BPP32, addr, 4, (0, 0, 1, 1), n)
}

/// Free bytes in the command FIFO, from the status register.
#[must_use]
pub fn free_bytes(status: u32) -> u32 {
    let used = ((status & 0x00FF_0000) | ((status >> 6) & 0x0100_0000)) >> 16;
    FIFO_WORDS.saturating_sub(used) * 4
}

/// Why the engine didn't take or finish work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stuck {
    /// The FIFO stayed full for 100 ms (the status word then).
    FifoFull(u32),
    /// The fence wasn't written in time.
    Fence,
}

impl core::fmt::Display for Stuck {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            Self::FifoFull(s) => write!(f, "FIFO full (status 0x{s:08x})"),
            Self::Fence => f.write_str("fence never written"),
        }
    }
}

/// The command stream: the register window, the window's write offset and
/// the fence counter. The clock is a free-running wrapping counter and its
/// rate (differences are taken in ticks, so its wrap is harmless).
pub struct Engine<R> {
    /// The 2700G register window (the MBX core at [`MBX`], the command
    /// window at [`PORT`]).
    pub regs: R,
    at: usize,
    fence: u32,
    fence_at: usize,
    now: fn() -> u32,
    ticks_per_ms: u32,
}

impl<R: RegisterBus> Engine<R> {
    /// A stream on `regs`, its fence word at local-memory `fence_at`; the
    /// clock `now` counts `ticks_per_ms` a millisecond.
    pub fn new(regs: R, fence_at: usize, now: fn() -> u32, ticks_per_ms: u32) -> Self {
        Self {
            regs,
            at: 0,
            fence: 0,
            fence_at,
            now,
            ticks_per_ms,
        }
    }

    /// Microseconds since the tick count `t0`.
    fn us_since(&self, t0: u32) -> u32 {
        let t = u64::from((self.now)().wrapping_sub(t0));
        (t * 1000 / u64::from(self.ticks_per_ms.max(1))) as u32
    }

    /// The MBX status register.
    pub fn status(&mut self) -> u32 {
        self.regs.read32(MBX + reg::STATUS)
    }

    /// Dell's reset (core variant 2), interrupts left masked; the command
    /// window's offset starts again at 0. `delay_us` waits.
    pub fn reset(&mut self, delay_us: fn(u32)) {
        self.regs.write32(MBX + reg::RESET, 0x111);
        delay_us(2_000);
        let en = self.regs.read32(MBX + reg::ENABLE);
        self.regs.write32(MBX + reg::ENABLE, en & !0xFFFF);
        self.regs.write32(MBX + reg::RESET, 0);
        delay_us(2_000);
        self.regs.write32(MBX + reg::CLEAR, 0xFFFF);
        self.at = 0;
    }

    /// Queues `words`, waiting (up to 100 ms) for room in the FIFO.
    pub fn submit(&mut self, words: &[u32]) -> Result<(), Stuck> {
        let need = 4 * words.len() as u32;
        let t0 = (self.now)();
        loop {
            let s = self.status();
            if free_bytes(s) >= need {
                break;
            }
            if self.us_since(t0) > 100_000 {
                return Err(Stuck::FifoFull(s));
            }
        }
        for &w in words {
            self.regs.write32(PORT + self.at, w);
            self.at = (self.at + 4) % PORT_SIZE;
        }
        Ok(())
    }

    /// Queues a fence and waits (up to `limit_us`) for the engine to write
    /// it into `vram`: everything queued before it is done. Returns the
    /// wait in microseconds.
    pub fn finish(&mut self, vram: &[u8], limit_us: u32) -> Result<u32, Stuck> {
        self.finish_with(limit_us, |at| load32_volatile(vram, at))
    }

    /// [`Self::finish`], reading the fence word through `read` (local
    /// memory held by something else, e.g. a model of the engine in tests).
    pub fn finish_with(
        &mut self,
        limit_us: u32,
        mut read: impl FnMut(usize) -> Option<u32>,
    ) -> Result<u32, Stuck> {
        self.fence = self.fence.wrapping_add(1);
        let n = self.fence;
        self.submit(&fence(self.fence_at, n))?;
        let t0 = (self.now)();
        loop {
            let waited = self.us_since(t0);
            if read(self.fence_at) == Some(n) {
                return Ok(waited);
            }
            if waited > limit_us {
                return Err(Stuck::Fence);
            }
        }
    }

    /// Takes the next fence number, to queue a fence by hand.
    pub fn next_fence(&mut self) -> u32 {
        self.fence = self.fence.wrapping_add(1);
        self.fence
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use core::sync::atomic::{AtomicU32, Ordering};

    use rp_hal_mock::MockBus;

    use super::*;

    const FENCE: usize = 0x00FF_F000;

    #[test]
    fn fill_words() {
        let w = fill(Format::RGB565, 0x6_0000, 960, (40, 120, 440, 320), 0xF800);
        assert_eq!(
            w,
            [
                0xA005_03C0,
                0x0006_0000,
                0x8000_F0F0,
                0xF800,
                0x0028_0078,
                0x01B8_0140,
                0x7000_0000
            ]
        );
    }

    #[test]
    fn fence_is_one_pixel_of_32_bits() {
        let w = fence(FENCE, 7);
        assert_eq!(w[0], 0xA006_0004);
        assert_eq!(w[1], 0x00FF_F000);
        assert_eq!((w[3], w[4], w[5]), (7, 0, 0x0001_0001));
    }

    #[test]
    fn fifo_level() {
        assert_eq!(free_bytes(0), 0x1FE * 4);
        // WM's fills showed 0x00950000: 0x95 words queued.
        assert_eq!(free_bytes(0x0095_0000), (0x1FE - 0x95) * 4);
        // Bit 30 is the level's ninth bit; other bits are ignored.
        assert_eq!(free_bytes(0x4000_0004), (0x1FE - 0x100) * 4);
        assert_eq!(free_bytes(0x40FF_0000), 0);
    }

    #[test]
    fn copy_words() {
        let (w, n) = copy(
            Format::RGB565,
            (0, 960),
            (0x80_0000, 128),
            (3, 5),
            (40, 360, 104, 424),
            OP_RIGHT_LEFT,
        );
        assert_eq!(
            w[..n],
            [
                0xA005_03C0,
                0,
                0x9405_0080,
                0x0080_0000,
                0x3000_C005,
                0x8100_CCCC,
                0,
                0x0028_0168,
                0x0068_01A8,
                0x7000_0000
            ]
        );
    }

    #[test]
    fn keyed_copy_has_no_colour_word() {
        let (w, n) = copy(
            Format::RGB565,
            (0, 960),
            (0, 960),
            (0, 0),
            (0, 0, 1, 1),
            OP_KEY,
        );
        assert_eq!(n, 9);
        assert_eq!(w[5], 0x8011_CCCC, "the driver's keyed header");
        assert_eq!(w[6..9], [0, 0x0001_0001, 0x7000_0000]);
    }

    #[test]
    fn coordinates_are_13_bits() {
        assert_eq!(xy(0x2001, 0xFFFF), 0x0001_1FFF);
    }

    #[test]
    fn negative_stride_is_15_bit_twos_complement() {
        // As rp-mbx sent it on the device: 0x8000 - 960.
        assert_eq!(
            dest(Format::RGB565, 0, -960)[0],
            0xA005_0000 | (0x8000 - 960)
        );
        assert_eq!(source(Format::RGB565, 0, -128)[0] & 0x7FFF, 0x8000 - 128);
    }

    #[test]
    fn dell_text_blit() {
        // Dell's anti-aliased text (0x02100604): 4 bpp coverage source, the
        // origin's x the nibble within the byte, header 0x8002CCCC, the
        // brush colour as the colour word.
        let (w, n) = blit(
            (Format::RGB565, 0x6_0000, 960),
            (Format::COVERAGE4, 0x80_0000, 8),
            (1, 0),
            (0, 0, 16, 1),
            ROP_SRCCOPY,
            OP_BLEND,
            0xF800,
        );
        assert_eq!(n, 10);
        assert_eq!(w[2], 0x9402_8008);
        assert_eq!(w[4], 0x3000_4000, "Dell: ((sx & 1) | 0xC000) << 14");
        assert_eq!(w[5], 0x8002_CCCC);
        assert_eq!(w[6], 0xF800);
        assert_eq!(blend(BLEND_DELL_TEXT), [0x2000_0004, 0x0910_0000]);
    }

    #[test]
    fn format_codes() {
        assert_eq!(Format::code(5), Format::COVERAGE4);
        assert_eq!(Format::code(0xA), Format::RGB565);
        assert_eq!(Format::code(0xC), Format::BPP32);
    }

    // Starts just short of the wrap: timeouts must survive it.
    static NOW: AtomicU32 = AtomicU32::new(u32::MAX - 5_000);
    fn tick() -> u32 {
        NOW.fetch_add(1_000, Ordering::Relaxed)
    }

    #[test]
    fn submit_writes_sequentially_and_wraps() {
        let mut e = Engine::new(MockBus::new(), FENCE, tick, 1_000);
        e.at = PORT_SIZE - 8;
        e.submit(&[1, 2, 3]).unwrap();
        let w: std::vec::Vec<_> = e
            .regs
            .writes()
            .into_iter()
            .filter(|&(o, _)| o >= PORT)
            .collect();
        assert_eq!(
            w,
            [
                (PORT + PORT_SIZE - 8, 1),
                (PORT + PORT_SIZE - 4, 2),
                (PORT, 3)
            ]
        );
    }

    #[test]
    fn full_fifo_times_out() {
        let mut bus = MockBus::new();
        bus.preset(MBX + reg::STATUS, 0x40FF_0000);
        let mut e = Engine::new(bus, FENCE, tick, 1_000);
        assert_eq!(e.submit(&[0]), Err(Stuck::FifoFull(0x40FF_0000)));
    }

    #[test]
    fn fence_waits_for_its_number() {
        let mut e = Engine::new(MockBus::new(), FENCE, tick, 1_000);
        let mut vram = std::vec![0u8; 16 << 20];
        // Nothing writes it: stuck.
        assert_eq!(e.finish(&vram, 10_000), Err(Stuck::Fence));
        // The next fence is 2; the "engine" already wrote it.
        vram[FENCE..FENCE + 4].copy_from_slice(&2u32.to_le_bytes());
        assert!(e.finish(&vram, 10_000).is_ok());
    }
}
