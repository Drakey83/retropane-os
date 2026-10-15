//! The 2700G's MVED1 video decoder (ADR-0025): register values, the
//! frame-conversion job, and the test frame with its host references.
//!
//! Everything here comes from the static survey (docs/hw/2700g.md, "The
//! MVED1 video decoder"; found in Windows Mobile's `pvrvadd.dll`, no code
//! copied) and is checked on the device by `rp-mved`, stage by stage:
//!
//! - **S0/S1:** the clock (`M24CLK`), the three registers Dell's library
//!   reads ([`READABLE`]: reading any other may hang the bus), the reset;
//! - **S2:** a 64 x 64 planar 4:2:0 test frame ([`test_frame_byte`])
//!   converted by the chip to YUY2 ([`convert_yuy2`]) and to its 3-byte
//!   format ([`convert_rgb3`]), compared with what the host computes
//!   ([`yuy2_byte`], [`rgb3_byte`]) under each hypothesis still open (the
//!   chroma order, RGB or BGR).
//!
//! **Security:** the unit is a bus master inside the 2700G's memory, which
//! also holds the visible screen. A job is only ever built by
//! [`convert_yuy2`] / [`convert_rgb3`], which check every address, stride
//! and size against the field widths and the memory's length, and refuse
//! overlapping source and destination, so a job can't be aimed outside the
//! frames it names.
//!
//! Pure: no hardware access; the test image writes these values.

#![no_std]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests;

/// The decoder's registers, as an offset into the 2700G register window
/// RetroPane maps (`0x0FFE0000`): `0x0FFE3000`.
pub const MVED: usize = 0x3000;
/// The decoder's clock, in the system block (offset `0x050`): bit 0 the
/// enable, bits 2-1 the divider - 1 (Linux mbxfb). Windows Mobile idles at
/// `0x2` (divider 2, off).
pub const M24CLK: usize = 0x050;
/// The two command windows (motion-compensation commands, then
/// coefficients or residuals), offsets in the register window. **Write
/// only: never read.** Unused by S0-S2.
pub const WINDOWS: core::ops::Range<usize> = 0x1_0000..0x1_2000;

/// Register offsets from [`MVED`] (docs/hw/2700g.md, "Registers").
pub mod reg {
    /// Reset pulse: 1, then 0.
    pub const RESET: usize = 0x00;
    /// Conversion source (a 4:2:0 frame), address / 8.
    pub const CONV_SRC: usize = 0x30;
    /// Conversion destination for mode 1 (YUY2), address / 8.
    pub const CONV_DST: usize = 0x34;
    /// Conversion size, `width | height << 11`.
    pub const CONV_SIZE: usize = 0x38;
    /// Conversion strides: mode 1 `(dst stride / 4) << 9 | src stride / 8`;
    /// mode 3 `src stride / 8`.
    pub const CONV_STRIDES: usize = 0x3C;
    /// Conversion destination for mode 3, address / 8.
    pub const CONV_DST3: usize = 0x40;
    /// Conversion destination stride for mode 3, stride / 8.
    pub const CONV_DST3_STRIDE: usize = 0x44;
    /// Starts a conversion: 1 = YUY2, 3 = 3 bytes a pixel.
    pub const CONV_START: usize = 0x48;
    /// FIFO levels (read; see [`super::Fifo`]).
    pub const FIFO: usize = 0x50;
    /// Status (read): [`super::CONVERT_DONE`], [`super::FRAME_DONE`].
    pub const STATUS: usize = 0x80;
    /// Acknowledge: the status bits written as 1 are cleared.
    pub const ACK: usize = 0x88;
    /// Capabilities (read), low 16 bits; bits 0-1 reported to the decoder.
    pub const CAPS: usize = 0xA0;
}

/// The only registers Dell's library reads; the bus-hang rule: read
/// nothing else in the block.
pub const READABLE: [usize; 3] = [reg::FIFO, reg::STATUS, reg::CAPS];

/// Whether `off` (from [`MVED`]) may be read.
#[must_use]
pub fn may_read(off: usize) -> bool {
    READABLE.contains(&off)
}

/// Status bit 0: a conversion finished.
pub const CONVERT_DONE: u32 = 1 << 0;
/// Status bit 1: a frame finished.
pub const FRAME_DONE: u32 = 1 << 1;

/// The reset, in order: the pulse, then both status bits acknowledged.
pub const RESET_SEQUENCE: [(usize, u32); 3] = [
    (reg::RESET, 1),
    (reg::RESET, 0),
    (reg::ACK, CONVERT_DONE | FRAME_DONE),
];

/// `M24CLK` with the decoder's clock on, keeping the divider found. A
/// divider field of 0 (divide by 1, never seen) is replaced by Windows
/// Mobile's 2, so the unit never runs faster than Dell's setting; the
/// higher bits are kept. WM's idle `0x2` gives `0x3`.
#[must_use]
pub const fn m24clk_on(found: u32) -> u32 {
    if found & 0x6 == 0 {
        (found & !0x7) | 0x3
    } else {
        found | 1
    }
}

/// `0x50` decoded: the three FIFOs' levels (entries in use).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fifo {
    /// Command FIFO, bits 12-6 (31 entries).
    pub command: u32,
    /// Residual FIFO, bits 20-13 (63 entries).
    pub residual: u32,
    /// Coefficient FIFO, bits 25-21 (31 entries).
    pub coefficient: u32,
}

impl Fifo {
    /// Command FIFO capacity, entries.
    pub const COMMAND: u32 = 31;
    /// Residual FIFO capacity, entries.
    pub const RESIDUAL: u32 = 63;
    /// Coefficient FIFO capacity, entries.
    pub const COEFFICIENT: u32 = 31;

    /// Decodes a `0x50` read.
    #[must_use]
    pub const fn from_reg(v: u32) -> Self {
        Self {
            command: (v >> 6) & 0x7F,
            residual: (v >> 13) & 0xFF,
            coefficient: (v >> 21) & 0x1F,
        }
    }

    /// All three empty.
    #[must_use]
    pub const fn empty(&self) -> bool {
        self.command == 0 && self.residual == 0 && self.coefficient == 0
    }

    /// Free command space in words ((capacity - level) x 2, Dell's rule);
    /// 0 if the level reads above the capacity.
    #[must_use]
    pub const fn command_room(&self) -> u32 {
        Self::COMMAND.saturating_sub(self.command) * 2
    }
}

/// A planar 4:2:0 frame as the decoder keeps it: the luma plane (stride x
/// height), then one chroma plane of height / 2 rows with the same
/// stride, the two chroma components side by side (the second at stride
/// / 2; IMC2-style). Which component comes first is what S2 finds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame420 {
    /// Width in pixels (even).
    pub width: u32,
    /// Height in pixels (even).
    pub height: u32,
    /// Bytes from one row to the next: a multiple of 256.
    pub stride: u32,
}

impl Frame420 {
    /// A frame with the stride rounded up to 256 bytes, as Dell allocates;
    /// None for an odd or zero size or one past the 11-bit size fields.
    #[must_use]
    pub const fn new(width: u32, height: u32) -> Option<Self> {
        if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return None;
        }
        if width > MAX_SIZE || height > MAX_SIZE {
            return None;
        }
        Some(Self {
            width,
            height,
            stride: width.div_ceil(256) * 256,
        })
    }

    /// Bytes the frame takes: stride x height x 3 / 2.
    #[must_use]
    pub const fn bytes(&self) -> usize {
        self.stride as usize * self.height as usize * 3 / 2
    }

    /// Offset of the chroma plane.
    #[must_use]
    pub const fn chroma(&self) -> usize {
        self.stride as usize * self.height as usize
    }
}

/// The size fields' limit (11 bits each).
pub const MAX_SIZE: u32 = 0x7FF;

/// The test frame's luma: a gradient, distinct across a row and down a
/// column, inside studio range (16..=205 for 64 x 64).
#[must_use]
pub const fn luma(x: u32, y: u32) -> u8 {
    (16 + 2 * (x & 63) + (y & 63)) as u8
}

/// The test frame's first chroma component (the left half of the chroma
/// plane) at chroma column `cx`: `0x40` rising by 2. Constant down a
/// column, so a chip that interpolates chroma vertically gives the same
/// bytes; rising across, so a swapped or mirrored half shows.
#[must_use]
pub const fn chroma_first(cx: u32) -> u8 {
    (0x40 + 2 * (cx & 31)) as u8
}

/// The test frame's second chroma component (the right half): `0xC0`
/// falling by 2. Never equal to any [`chroma_first`] value.
#[must_use]
pub const fn chroma_second(cx: u32) -> u8 {
    (0xC0 - 2 * (cx & 31)) as u8
}

/// The test frame's byte at offset `off` into `f` (padding bytes 0).
#[must_use]
pub const fn test_frame_byte(f: &Frame420, off: usize) -> u8 {
    let stride = f.stride as usize;
    let (row, col) = (off / stride, off % stride);
    let (w, h) = (f.width as usize, f.height as usize);
    if row < h {
        return if col < w {
            luma(col as u32, row as u32)
        } else {
            0
        };
    }
    if row >= h + h / 2 {
        return 0;
    }
    let half = stride / 2;
    if col < w / 2 {
        chroma_first(col as u32)
    } else if col >= half && col < half + w / 2 {
        chroma_second((col - half) as u32)
    } else {
        0
    }
}

/// The test frame's 32-bit word at word-aligned offset `off`
/// (little-endian, as the CPU stores it into 2700G memory).
#[must_use]
pub const fn test_frame_word(f: &Frame420, off: usize) -> u32 {
    u32::from_le_bytes([
        test_frame_byte(f, off),
        test_frame_byte(f, off + 1),
        test_frame_byte(f, off + 2),
        test_frame_byte(f, off + 3),
    ])
}

/// Which chroma half the chip takes as U (Cb): the open question S2
/// answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChromaOrder {
    /// The left half is U, the right half V.
    UFirst,
    /// The left half is V, the right half U.
    VFirst,
}

impl ChromaOrder {
    /// Both hypotheses.
    pub const ALL: [Self; 2] = [Self::UFirst, Self::VFirst];

    /// (U, V) of the test frame at chroma column `cx`.
    #[must_use]
    pub const fn uv(self, cx: u32) -> (u8, u8) {
        let (a, b) = (chroma_first(cx), chroma_second(cx));
        match self {
            Self::UFirst => (a, b),
            Self::VFirst => (b, a),
        }
    }

    /// For the log.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::UFirst => "U first",
            Self::VFirst => "V first",
        }
    }
}

/// The YUY2 byte (`Y0 U Y1 V`) the conversion of the test frame should
/// give at byte `xb` of row `y`, under `order`.
#[must_use]
pub const fn yuy2_byte(order: ChromaOrder, xb: u32, y: u32) -> u8 {
    let x = xb / 2;
    let (u, v) = order.uv(x / 2);
    match xb % 4 {
        0 | 2 => luma(x, y),
        1 => u,
        _ => v,
    }
}

/// BT.601 studio range to RGB, the rule `rp_drv_mbx_display::scanout`
/// and the emulator use (docs/hw/2700g.md, "The mixing rule").
#[must_use]
pub const fn rgb601(y: u8, u: u8, v: u8) -> [u8; 3] {
    let c = y as i32 - 16;
    let d = u as i32 - 128;
    let e = v as i32 - 128;
    [
        clamp((298 * c + 409 * e + 128) >> 8),
        clamp((298 * c - 100 * d - 208 * e + 128) >> 8),
        clamp((298 * c + 516 * d + 128) >> 8),
    ]
}

const fn clamp(v: i32) -> u8 {
    if v < 0 {
        0
    } else if v > 255 {
        255
    } else {
        v as u8
    }
}

/// A hypothesis for the 3-byte format (mode 3): the chroma order and the
/// byte order in memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb3 {
    /// Which chroma half is U.
    pub order: ChromaOrder,
    /// Bytes B, G, R in memory (as a little-endian 24-bit `0xRRGGBB`)
    /// rather than R, G, B.
    pub bgr: bool,
}

impl Rgb3 {
    /// Every hypothesis.
    pub const ALL: [Self; 4] = [
        Self {
            order: ChromaOrder::UFirst,
            bgr: true,
        },
        Self {
            order: ChromaOrder::UFirst,
            bgr: false,
        },
        Self {
            order: ChromaOrder::VFirst,
            bgr: true,
        },
        Self {
            order: ChromaOrder::VFirst,
            bgr: false,
        },
    ];
}

impl Rgb3 {
    /// For the log.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match (self.bgr, self.order) {
            (false, ChromaOrder::UFirst) => "RGB, U first",
            (true, ChromaOrder::UFirst) => "BGR, U first",
            (false, ChromaOrder::VFirst) => "RGB, V first",
            (true, ChromaOrder::VFirst) => "BGR, V first",
        }
    }
}

/// The byte the mode-3 conversion of the test frame would give at byte
/// `xb` of row `y` if it is BT.601 RGB under `h`.
#[must_use]
pub const fn rgb3_byte(h: Rgb3, xb: u32, y: u32) -> u8 {
    let x = xb / 3;
    let (u, v) = h.order.uv(x / 2);
    let rgb = rgb601(luma(x, y), u, v);
    let k = (xb % 3) as usize;
    if h.bgr { rgb[2 - k] } else { rgb[k] }
}

/// A conversion job's frames: where the 4:2:0 source is and where the
/// result goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Convert {
    /// The source frame's layout.
    pub frame: Frame420,
    /// The source's byte offset in 2700G memory.
    pub src: usize,
    /// The destination's byte offset in 2700G memory.
    pub dst: usize,
    /// The destination's stride in bytes.
    pub dst_stride: u32,
}

/// Why a job was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobError {
    /// An address isn't a multiple of 8 (the registers take address / 8).
    Unaligned,
    /// The source stride isn't a multiple of 256, or a stride is too small
    /// for the width.
    Stride,
    /// A value doesn't fit its register field.
    Field,
    /// A frame reaches past the end of 2700G memory.
    OutOfMemory,
    /// The source and destination overlap.
    Overlap,
}

impl JobError {
    /// For the log.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Unaligned => "an address isn't a multiple of 8",
            Self::Stride => "a stride is wrong for the frame",
            Self::Field => "a value doesn't fit its register",
            Self::OutOfMemory => "a frame reaches past 2700G memory",
            Self::Overlap => "source and destination overlap",
        }
    }
}

/// Destination strides are rounded to 32 bytes (Dell's allocation types
/// 1 and 2); the job asks for at least that alignment.
pub const DST_STRIDE_ALIGN: u32 = 32;

impl Convert {
    /// The destination's extent for `bpp` bytes a pixel.
    const fn dst_end(&self, bpp: u32) -> Option<usize> {
        let rows = self.frame.height as usize - 1;
        match (self.dst_stride as usize).checked_mul(rows) {
            Some(v) => match v.checked_add(self.frame.width as usize * bpp as usize) {
                Some(n) => self.dst.checked_add(n),
                None => None,
            },
            None => None,
        }
    }

    /// Every check both modes share; the destination's end.
    fn check(&self, bpp: u32, vram_len: usize) -> Result<usize, JobError> {
        let f = &self.frame;
        if f.width == 0
            || f.height == 0
            || !f.width.is_multiple_of(2)
            || !f.height.is_multiple_of(2)
        {
            return Err(JobError::Field);
        }
        if f.width > MAX_SIZE || f.height > MAX_SIZE {
            return Err(JobError::Field);
        }
        if !self.src.is_multiple_of(8) || !self.dst.is_multiple_of(8) {
            return Err(JobError::Unaligned);
        }
        if !f.stride.is_multiple_of(256) || f.stride < f.width {
            return Err(JobError::Stride);
        }
        if !self.dst_stride.is_multiple_of(DST_STRIDE_ALIGN) || self.dst_stride < f.width * bpp {
            return Err(JobError::Stride);
        }
        let src_end = self
            .src
            .checked_add(f.bytes())
            .ok_or(JobError::OutOfMemory)?;
        let dst_end = self.dst_end(bpp).ok_or(JobError::OutOfMemory)?;
        if src_end > vram_len || dst_end > vram_len {
            return Err(JobError::OutOfMemory);
        }
        if self.src < dst_end && self.dst < src_end {
            return Err(JobError::Overlap);
        }
        // Addresses / 8 into 32-bit registers: 16 MiB fits; a larger
        // `vram_len` still can't overflow the field.
        if src_end / 8 > u32::MAX as usize || dst_end / 8 > u32::MAX as usize {
            return Err(JobError::Field);
        }
        Ok(dst_end)
    }

    /// The bytes the destination takes for `bpp` bytes a pixel: what the
    /// test image compares and guards.
    #[must_use]
    pub const fn dst_bytes(&self, bpp: u32) -> usize {
        match self.dst_end(bpp) {
            Some(e) => e - self.dst,
            None => 0,
        }
    }
}

const fn size_word(f: &Frame420) -> u32 {
    f.width | f.height << 11
}

/// A conversion to YUY2 (mode 1): the register writes in order, the start
/// last. Checked against `vram_len` (see [`JobError`]).
pub fn convert_yuy2(job: &Convert, vram_len: usize) -> Result<[(usize, u32); 5], JobError> {
    job.check(2, vram_len)?;
    let src_field = job.frame.stride / 8;
    let dst_field = job.dst_stride / 4;
    // The source stride takes bits 8-0, the destination's the rest.
    if src_field > 0x1FF || dst_field > u32::MAX >> 9 {
        return Err(JobError::Field);
    }
    Ok([
        (reg::CONV_SRC, (job.src / 8) as u32),
        (reg::CONV_DST, (job.dst / 8) as u32),
        (reg::CONV_SIZE, size_word(&job.frame)),
        (reg::CONV_STRIDES, dst_field << 9 | src_field),
        (reg::CONV_START, 1),
    ])
}

/// A conversion to the 3-byte format (mode 3): the register writes in
/// order, the start last. Checked against `vram_len`.
pub fn convert_rgb3(job: &Convert, vram_len: usize) -> Result<[(usize, u32); 6], JobError> {
    job.check(3, vram_len)?;
    if job.frame.stride / 8 > 0x1FF {
        return Err(JobError::Field);
    }
    Ok([
        (reg::CONV_SRC, (job.src / 8) as u32),
        (reg::CONV_DST3, (job.dst / 8) as u32),
        (reg::CONV_SIZE, size_word(&job.frame)),
        (reg::CONV_STRIDES, job.frame.stride / 8),
        (reg::CONV_DST3_STRIDE, job.dst_stride / 8),
        (reg::CONV_START, 3),
    ])
}

/// How a read-back destination compares with a reference: bytes that
/// differ, the largest difference, the first differing byte.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Diff {
    /// Bytes compared.
    pub bytes: u32,
    /// Bytes that differ by more than the tolerance.
    pub wrong: u32,
    /// The largest difference seen.
    pub max: u8,
    /// The first byte past the tolerance: (byte in row, row, got, wanted).
    pub first: Option<(u32, u32, u8, u8)>,
}

impl Diff {
    /// Adds one byte at (`xb`, `y`).
    pub fn add(&mut self, xb: u32, y: u32, got: u8, want: u8, tolerance: u8) {
        let d = got.abs_diff(want);
        self.bytes += 1;
        self.max = self.max.max(d);
        if d > tolerance {
            self.wrong += 1;
            if self.first.is_none() {
                self.first = Some((xb, y, got, want));
            }
        }
    }

    /// Every byte within the tolerance (and at least one compared).
    #[must_use]
    pub const fn matches(&self) -> bool {
        self.bytes > 0 && self.wrong == 0
    }
}
