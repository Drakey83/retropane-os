//! A person's picture from a JPEG file (contacts-rule0.md "Photos from a
//! picture file on the card, and from vCard PHOTO"): the middle square of
//! the picture, upright, 96 pixels a side, for Contacts' own small format.
//!
//! Every file is hostile input (security.md): every length and count in
//! it is checked against the bytes there, a damaged file ends in an error
//! or a partly grey picture, never a panic or a read out of bounds.
//!
//! Iterate, don't imitate (rule 0): Windows Mobile kept the JPEG it was
//! given and decoded it whole to show it (libjpeg: a full IDCT of every
//! block, a whole frame buffer); a phone makes a thumbnail with its
//! hardware decoder. Ours, for a 96-pixel disc:
//! - takes the **thumbnail a camera already put in the file** (Exif, about
//!   160 x 120, in the file's first 64 KB) when it is big enough: a
//!   12-megapixel photo becomes an avatar without reading its 4 MB;
//! - else decodes **straight to the size needed**: a block's 8 x 8
//!   frequencies give 1, 2, 4 or 8 pixels a side from only its lowest
//!   1, 4, 16 or 64 (the DC alone for a big photo: no IDCT at all), the
//!   smallest that still covers 96 pixels;
//! - **box-filters** into the square as the blocks come, each source pixel
//!   added to its one output pixel (ten bits a colour in a 32-bit word: no
//!   frame buffer, the 96 x 96 words are the caller's, a picture slot);
//! - reads a progressive file's **first scan only** (every block's DC, the
//!   whole picture at an eighth of its size);
//! - turns the picture **upright** from its Exif orientation;
//! - is **resumable**: a file bigger than the device's answer is fed a
//!   piece at a time (an MCU never half decoded: one that runs out of
//!   bytes is decoded again from the next piece).

/// The square's side.
pub const SIDE: usize = 96;
/// The caller's words for the square ([`Decoder::feed`]).
pub const WORDS: usize = SIDE * SIDE;

/// Why a file can't be read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// Not a JPEG file.
    NotJpeg,
    /// A kind this reader doesn't take: arithmetic coding, lossless,
    /// 12-bit, CMYK, or a progressive file whose first scan isn't its DC.
    Kind,
    /// The file's headers don't fit its first piece.
    Headers,
    /// Damaged.
    Damaged,
}

/// How far a [`Decoder::feed`] got.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Progress {
    /// Bytes of the piece used: feed the file again from that point.
    More(usize),
    /// The picture is whole ([`Decoder::rgb`]).
    Done,
}

/// The scaled IDCT's cosines (`C(u) cos((2x + 1) u pi / 2s)`, times 4096)
/// for 2, 4 and 8 pixels a side (the DC alone needs none).
const K2: [[i32; 2]; 2] = [[2896, 2896], [2896, -2896]];
const K4: [[i32; 4]; 4] = [
    [2896, 3784, 2896, 1567],
    [2896, 1567, -2896, -3784],
    [2896, -1567, -2896, 3784],
    [2896, -3784, 2896, -1567],
];
const K8: [[i32; 8]; 8] = [
    [2896, 4017, 3784, 3406, 2896, 2276, 1567, 799],
    [2896, 3406, 1567, -799, -2896, -4017, -3784, -2276],
    [2896, 2276, -1567, -4017, -2896, 799, 3784, 3406],
    [2896, 799, -3784, -2276, 2896, 3406, -1567, -4017],
    [2896, -799, -3784, 2276, 2896, -3406, -1567, 4017],
    [2896, -2276, -1567, 4017, -2896, -799, 3784, -3406],
    [2896, -3406, 1567, 799, -2896, 4017, -3784, 2276],
    [2896, -4017, 3784, -3406, 2896, -2276, 1567, -799],
];

/// The zigzag order's natural places.
const ZIGZAG: [u8; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

#[derive(Clone, Copy)]
struct Huff {
    syms: [u8; 256],
    maxcode: [i32; 18],
    valptr: [i32; 17],
    mincode: [i32; 17],
    ok: bool,
}

impl Huff {
    const NONE: Self = Self {
        syms: [0; 256],
        maxcode: [-1; 18],
        valptr: [0; 17],
        mincode: [0; 17],
        ok: false,
    };

    /// A table from its 16 counts and its symbols (ITU T.81 F.2.2.3).
    fn of(counts: &[u8], syms: &[u8]) -> Option<Self> {
        let mut h = Self::NONE;
        let total: usize = counts.iter().map(|&c| usize::from(c)).sum();
        if total > 256 || syms.len() < total {
            return None;
        }
        h.syms[..total].copy_from_slice(&syms[..total]);
        let (mut code, mut k) = (0i32, 0i32);
        for l in 1..=16 {
            let n = i32::from(counts[l - 1]);
            if n == 0 {
                h.maxcode[l] = -1;
            } else {
                h.valptr[l] = k;
                h.mincode[l] = code;
                code += n;
                k += n;
                h.maxcode[l] = code - 1;
            }
            // A code longer than its length's room: not a prefix code.
            if code > 1 << l {
                return None;
            }
            code <<= 1;
        }
        h.maxcode[17] = i32::MAX;
        h.ok = true;
        Some(h)
    }
}

#[derive(Clone, Copy, Default)]
struct Comp {
    id: u8,
    h: u8,
    v: u8,
    q: u8,
    dc: u8,
    ac: u8,
    pred: i32,
    /// In the scan.
    scanned: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    Start,
    Scan,
    Done,
}

/// The decoder's state between pieces.
pub struct Decoder {
    stage: Stage,
    q: [[u16; 64]; 4],
    huff: [Huff; 4],
    comps: [Comp; 3],
    n: usize,
    width: u32,
    height: u32,
    hmax: u8,
    vmax: u8,
    progressive: bool,
    /// A progressive DC scan's point transform.
    al: u8,
    /// Adobe's "no transform" (RGB, not YCbCr).
    rgb: bool,
    restart: u16,
    todo: u16,
    mcu: u32,
    mcus: u32,
    mcus_x: u32,
    bits: u32,
    nbits: u32,
    starved: bool,
    /// Zero bytes put in past the scan's data (a file cut short, or a
    /// frame bigger than its data): past a few hundred, the scan ends.
    pad: u32,
    /// Pixels a block side decoded (1, 2, 4, 8).
    s: u32,
    /// The square in the scaled picture: its corner and side, the step
    /// between pixels taken, and its side in pixels taken.
    x0: u32,
    y0: u32,
    side: u32,
    step: u32,
    taken: u32,
    orient: u8,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

fn be16(d: &[u8], at: usize) -> Option<usize> {
    Some(usize::from(*d.get(at)?) << 8 | usize::from(*d.get(at + 1)?))
}

/// The Exif block's orientation and its thumbnail's place (within `app`,
/// the APP1 segment's data after its length).
fn exif(app: &[u8]) -> (u8, Option<(usize, usize)>) {
    let Some(t) = app.strip_prefix(b"Exif\0\0") else {
        return (1, None);
    };
    let le = match t.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return (1, None),
    };
    let u16_at = |at: usize| -> Option<u32> {
        let b = t.get(at..at + 2)?;
        Some(u32::from(if le {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        }))
    };
    let u32_at = |at: usize| -> Option<u32> {
        let b: [u8; 4] = t.get(at..at + 4)?.try_into().ok()?;
        Some(if le {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    };
    let mut orient = 1;
    let mut thumb = None;
    let mut ifd = u32_at(4).unwrap_or(0) as usize;
    for k in 0..2 {
        let Some(count) = u16_at(ifd) else { break };
        let (mut off, mut len) = (None, None);
        for e in 0..count.min(64) as usize {
            let at = ifd + 2 + e * 12;
            let (Some(tag), Some(kind)) = (u16_at(at), u16_at(at + 2)) else {
                break;
            };
            let value = if kind == 3 {
                u16_at(at + 8)
            } else {
                u32_at(at + 8)
            };
            match (k, tag) {
                (0, 0x0112) => orient = value.unwrap_or(1).clamp(1, 8) as u8,
                (1, 0x0201) => off = value,
                (1, 0x0202) => len = value,
                _ => {}
            }
        }
        if let (Some(o), Some(l)) = (off, len) {
            let (o, l) = (o as usize, l as usize);
            if o.checked_add(l).is_some_and(|e| e <= t.len()) {
                // Its place in `app`: past the "Exif\0\0".
                thumb = Some((6 + o, 6 + o + l));
            }
        }
        ifd = match u32_at(ifd + 2 + count as usize * 12) {
            Some(n) if n != 0 => n as usize,
            _ => break,
        };
    }
    (orient, thumb)
}

/// A JPEG's width and height from its frame header, without decoding.
fn size_of(d: &[u8]) -> Option<(u32, u32)> {
    if d.get(..2) != Some(&[0xFF, 0xD8]) {
        return None;
    }
    let mut at = 2;
    loop {
        if *d.get(at)? != 0xFF {
            return None;
        }
        let m = *d.get(at + 1)?;
        let len = be16(d, at + 2)?;
        if matches!(m, 0xC0..=0xC2) {
            return Some((be16(d, at + 7)? as u32, be16(d, at + 5)? as u32));
        }
        at += 2 + len;
    }
}

impl Decoder {
    /// Ready for a file's first piece.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            stage: Stage::Start,
            q: [[1; 64]; 4],
            huff: [Huff::NONE; 4],
            comps: [Comp {
                id: 0,
                h: 1,
                v: 1,
                q: 0,
                dc: 0,
                ac: 0,
                pred: 0,
                scanned: false,
            }; 3],
            n: 0,
            width: 0,
            height: 0,
            hmax: 1,
            vmax: 1,
            progressive: false,
            al: 0,
            rgb: false,
            restart: 0,
            todo: 0,
            mcu: 0,
            mcus: 0,
            mcus_x: 0,
            bits: 0,
            nbits: 0,
            starved: false,
            pad: 0,
            s: 8,
            x0: 0,
            y0: 0,
            side: 1,
            step: 1,
            taken: 1,
            orient: 1,
        }
    }

    /// The next piece of the file, from where the last one's [`Progress::More`]
    /// said (the first piece holds the file's start and every header);
    /// `last` when it ends the file. Decoded pixels are added into `acc`
    /// ([`WORDS`] words, zero at the start), read with [`Decoder::rgb`].
    ///
    /// # Errors
    /// [`Error`]: the file isn't one this reads, or is damaged.
    #[inline(never)]
    pub fn feed(&mut self, data: &[u8], last: bool, acc: &mut [u32]) -> Result<Progress, Error> {
        if acc.len() < WORDS {
            return Err(Error::Damaged);
        }
        match self.stage {
            Stage::Done => Ok(Progress::Done),
            Stage::Start => {
                let (at, thumb) = self.headers(data, false)?;
                if let Some(t) = thumb {
                    // The camera's own thumbnail: decoded whole, here.
                    let orient = self.orient;
                    *self = Self::new();
                    self.orient = orient;
                    let (at, _) = self.headers(t, true)?;
                    self.stage = Stage::Scan;
                    self.scan(&t[at..], true, acc)?;
                    self.stage = Stage::Done;
                    return Ok(Progress::Done);
                }
                self.stage = Stage::Scan;
                match self.scan(&data[at..], last, acc)? {
                    Progress::More(n) => Ok(Progress::More(at + n)),
                    Progress::Done => {
                        self.stage = Stage::Done;
                        Ok(Progress::Done)
                    }
                }
            }
            Stage::Scan => {
                let p = self.scan(data, last, acc)?;
                if p == Progress::Done {
                    self.stage = Stage::Done;
                }
                Ok(p)
            }
        }
    }

    /// The headers up to the first scan's data: where it starts, and the
    /// Exif thumbnail to decode instead when it covers the square.
    fn headers<'d>(
        &mut self,
        d: &'d [u8],
        thumb: bool,
    ) -> Result<(usize, Option<&'d [u8]>), Error> {
        if d.get(..2) != Some(&[0xFF, 0xD8]) {
            return Err(Error::NotJpeg);
        }
        let mut at = 2;
        let mut use_thumb = None;
        loop {
            // Fill bytes before a marker.
            while d.get(at) == Some(&0xFF) && d.get(at + 1) == Some(&0xFF) {
                at += 1;
            }
            if d.get(at) != Some(&0xFF) {
                return Err(if at >= d.len() {
                    Error::Headers
                } else {
                    Error::Damaged
                });
            }
            let m = *d.get(at + 1).ok_or(Error::Headers)?;
            let len = be16(d, at + 2).ok_or(Error::Headers)?;
            if len < 2 {
                return Err(Error::Damaged);
            }
            let seg = d.get(at + 4..at + 2 + len).ok_or(Error::Headers)?;
            match m {
                // Exif: the orientation and the thumbnail.
                0xE1 if !thumb => {
                    let (o, t) = exif(seg);
                    self.orient = o;
                    if let Some((a, b)) = t
                        && let Some(t) = seg.get(a..b)
                        && size_of(t).is_some_and(|(w, h)| w.min(h) as usize >= SIDE)
                    {
                        use_thumb = Some(t);
                    }
                }
                // Adobe: whether three components are RGB.
                0xEE => {
                    if seg.starts_with(b"Adobe") && seg.len() >= 12 {
                        self.rgb = seg[11] == 0;
                    }
                }
                0xDB => self.tables_q(seg)?,
                0xC4 => self.tables_h(seg)?,
                0xDD => self.restart = be16(seg, 0).ok_or(Error::Damaged)? as u16,
                0xC0..=0xC2 => {
                    self.progressive = m == 0xC2;
                    self.frame(seg)?;
                    if use_thumb.is_some() {
                        return Ok((0, use_thumb));
                    }
                }
                0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => return Err(Error::Kind),
                0xDA => {
                    self.scan_header(seg)?;
                    return Ok((at + 2 + len, None));
                }
                0xD9 => return Err(Error::Damaged),
                _ => {}
            }
            at += 2 + len;
        }
    }

    fn tables_q(&mut self, seg: &[u8]) -> Result<(), Error> {
        let mut at = 0;
        while at < seg.len() {
            let pq = seg[at] >> 4;
            let t = usize::from(seg[at] & 15);
            if t > 3 {
                return Err(Error::Damaged);
            }
            let wide = pq == 1;
            let body = seg
                .get(at + 1..at + 1 + if wide { 128 } else { 64 })
                .ok_or(Error::Damaged)?;
            for k in 0..64 {
                let v = if wide {
                    u16::from(body[2 * k]) << 8 | u16::from(body[2 * k + 1])
                } else {
                    u16::from(body[k])
                };
                self.q[t][usize::from(ZIGZAG[k])] = v.max(1);
            }
            at += 1 + body.len();
        }
        Ok(())
    }

    fn tables_h(&mut self, seg: &[u8]) -> Result<(), Error> {
        let mut at = 0;
        while at < seg.len() {
            let class = seg[at] >> 4;
            let t = usize::from(seg[at] & 15);
            if class > 1 || t > 1 {
                return Err(Error::Damaged);
            }
            let counts = seg.get(at + 1..at + 17).ok_or(Error::Damaged)?;
            let total: usize = counts.iter().map(|&c| usize::from(c)).sum();
            let syms = seg.get(at + 17..at + 17 + total).ok_or(Error::Damaged)?;
            self.huff[usize::from(class) * 2 + t] = Huff::of(counts, syms).ok_or(Error::Damaged)?;
            at += 17 + total;
        }
        Ok(())
    }

    fn frame(&mut self, seg: &[u8]) -> Result<(), Error> {
        if seg.first() != Some(&8) {
            return Err(Error::Kind);
        }
        self.height = be16(seg, 1).ok_or(Error::Damaged)? as u32;
        self.width = be16(seg, 3).ok_or(Error::Damaged)? as u32;
        let n = usize::from(*seg.get(5).ok_or(Error::Damaged)?);
        if n != 1 && n != 3 {
            return Err(Error::Kind);
        }
        if self.width == 0 || self.height == 0 {
            return Err(Error::Damaged);
        }
        self.n = n;
        for (k, slot) in self.comps.iter_mut().enumerate().take(n) {
            let c = seg.get(6 + 3 * k..9 + 3 * k).ok_or(Error::Damaged)?;
            let (h, v) = (c[1] >> 4, c[1] & 15);
            if !(1..=2).contains(&h) || !(1..=2).contains(&v) || c[2] > 3 {
                return Err(Error::Kind);
            }
            *slot = Comp {
                id: c[0],
                h,
                v,
                q: c[2],
                ..Comp::default()
            };
        }
        if n == 1 {
            // One component: a block an MCU, whatever its sampling says.
            self.comps[0].h = 1;
            self.comps[0].v = 1;
        }
        self.hmax = self.comps[..n].iter().map(|c| c.h).max().unwrap_or(1);
        self.vmax = self.comps[..n].iter().map(|c| c.v).max().unwrap_or(1);
        let (bw, bh) = (8 * u32::from(self.hmax), 8 * u32::from(self.vmax));
        self.mcus_x = self.width.div_ceil(bw);
        self.mcus = self.mcus_x * self.height.div_ceil(bh);
        // Pixels a block side: the fewest that still cover the square
        // (a progressive file's first scan has only the DC).
        let short = self.width.min(self.height);
        self.s = if self.progressive {
            1
        } else {
            [1, 2, 4, 8]
                .into_iter()
                .find(|&s| short * s / 8 >= SIDE as u32)
                .unwrap_or(8)
        };
        let (w, h) = (
            (self.width * self.s).div_ceil(8),
            (self.height * self.s).div_ceil(8),
        );
        self.side = w.min(h).max(1);
        self.x0 = (w - self.side) / 2;
        self.y0 = (h - self.side) / 2;
        // Every pixel taken while the square has under two a pixel;
        // past that, every step-th (a big photo's DC: 500 a side).
        self.step = (self.side / SIDE as u32).max(1);
        self.taken = (self.side / self.step).max(1);
        Ok(())
    }

    fn scan_header(&mut self, seg: &[u8]) -> Result<(), Error> {
        let n = usize::from(*seg.first().ok_or(Error::Damaged)?);
        if n == 0 || n > self.n {
            return Err(Error::Damaged);
        }
        for c in &mut self.comps {
            c.scanned = false;
            c.pred = 0;
        }
        for k in 0..n {
            let id = *seg.get(1 + 2 * k).ok_or(Error::Damaged)?;
            let t = *seg.get(2 + 2 * k).ok_or(Error::Damaged)?;
            let c = self.comps[..self.n]
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or(Error::Damaged)?;
            c.dc = (t >> 4).min(1);
            c.ac = (t & 15).min(1);
            c.scanned = true;
        }
        let tail = seg.get(1 + 2 * n..4 + 2 * n).ok_or(Error::Damaged)?;
        if self.progressive {
            // The first scan must be every component's DC.
            if tail[0] != 0 || tail[1] != 0 || tail[2] >> 4 != 0 || n != self.n {
                return Err(Error::Kind);
            }
            self.al = tail[2] & 15;
        } else if n != self.n {
            // A baseline file in a scan per component.
            return Err(Error::Kind);
        }
        self.todo = self.restart;
        Ok(())
    }

    // ---- The entropy-coded data -----------------------------------

    /// At least 25 bits in the buffer if the piece has them; stuffed
    /// zeros past a marker, and past the piece's end (then `starved`,
    /// unless it is the file's last).
    fn fill(&mut self, d: &[u8], at: &mut usize, last: bool) {
        while self.nbits <= 24 {
            let b = match d.get(*at) {
                None => {
                    if !last {
                        self.starved = true;
                    }
                    self.pad += 1;
                    0
                }
                Some(&0xFF) => match d.get(*at + 1) {
                    Some(0) => {
                        *at += 2;
                        0xFF
                    }
                    // A marker: the scan's data ends; zeros from here.
                    Some(_) => {
                        self.pad += 1;
                        0
                    }
                    None => {
                        if !last {
                            self.starved = true;
                        }
                        self.pad += 1;
                        0
                    }
                },
                Some(&b) => {
                    *at += 1;
                    b
                }
            };
            self.bits |= u32::from(b) << (24 - self.nbits);
            self.nbits += 8;
        }
    }

    fn bit(&mut self, d: &[u8], at: &mut usize, last: bool) -> i32 {
        if self.nbits == 0 {
            self.fill(d, at, last);
        }
        let b = (self.bits >> 31) as i32;
        self.bits <<= 1;
        self.nbits -= 1;
        b
    }

    fn get(&mut self, n: u32, d: &[u8], at: &mut usize, last: bool) -> i32 {
        if n == 0 {
            return 0;
        }
        if self.nbits < n {
            self.fill(d, at, last);
        }
        let v = (self.bits >> (32 - n)) as i32;
        self.bits <<= n;
        self.nbits = self.nbits.saturating_sub(n);
        v
    }

    fn huff(&mut self, t: usize, d: &[u8], at: &mut usize, last: bool) -> Option<u8> {
        let h = &self.huff[t];
        if !h.ok {
            return None;
        }
        let mut code = 0i32;
        for l in 1..=16 {
            code = code << 1 | self.bit(d, at, last);
            let h = &self.huff[t];
            if code <= h.maxcode[l] {
                let k = h.valptr[l] + code - h.mincode[l];
                return h.syms.get(k as usize).copied();
            }
        }
        None
    }

    fn extend(v: i32, n: u32) -> i32 {
        if n == 0 {
            0
        } else if v < 1 << (n - 1) {
            v - (1 << n) + 1
        } else {
            v
        }
    }

    /// The scan's MCUs from `d` (the scan's data from where it stood).
    fn scan(&mut self, d: &[u8], last: bool, acc: &mut [u32]) -> Result<Progress, Error> {
        let mut at = 0usize;
        // One MCU's blocks, each `s x s` pixels: at most six blocks.
        let mut px = [0u8; 6 * 64];
        while self.mcu < self.mcus {
            // An MCU that runs out of bytes is decoded again from here.
            let saved = (at, self.bits, self.nbits, self.comps, self.todo);
            if self.restart != 0 && self.todo == 0 {
                // The restart marker: bits to the byte, the marker, and
                // the DC predictions from zero.
                self.bits = 0;
                self.nbits = 0;
                while d.get(at) == Some(&0xFF) && d.get(at + 1) == Some(&0xFF) {
                    at += 1;
                }
                match (d.get(at), d.get(at + 1)) {
                    (Some(0xFF), Some(0xD0..=0xD7)) => at += 2,
                    (None, _) | (Some(0xFF), None) if !last => return Ok(Progress::More(saved.0)),
                    // A marker missing: carried on (a damaged file shows
                    // what it can).
                    _ => {}
                }
                for c in &mut self.comps {
                    c.pred = 0;
                }
                self.todo = self.restart;
            }
            self.starved = false;
            let pad = self.pad;
            let ok = self.mcu_blocks(d, &mut at, last, &mut px);
            if self.starved {
                (at, self.bits, self.nbits, self.comps, self.todo) = saved;
                self.pad = pad;
                return Ok(Progress::More(at));
            }
            // The data ran out (a file cut short, a frame bigger than its
            // data, a restart marker's place): what is left stays grey.
            if self.pad > 256 {
                return Ok(Progress::Done);
            }
            if !ok {
                return Err(Error::Damaged);
            }
            self.put(&px, acc);
            self.mcu += 1;
            self.todo = self.todo.saturating_sub(1);
        }
        Ok(Progress::Done)
    }

    /// Every block of the MCU decoded into `px` at `s x s` pixels each.
    fn mcu_blocks(&mut self, d: &[u8], at: &mut usize, last: bool, px: &mut [u8; 6 * 64]) -> bool {
        let s = self.s as usize;
        let mut b = 0;
        for k in 0..self.n {
            let c = self.comps[k];
            for _ in 0..usize::from(c.h) * usize::from(c.v) {
                let mut coef = [0i32; 64];
                // DC.
                let Some(t) = self.huff(usize::from(c.dc), d, at, last) else {
                    return false;
                };
                let n = u32::from(t);
                if n > 11 {
                    return false;
                }
                let diff = Self::extend(self.get(n, d, at, last), n);
                self.comps[k].pred = self.comps[k].pred.wrapping_add(diff);
                let q = self.q[usize::from(c.q)];
                coef[0] = (self.comps[k].pred << self.al).wrapping_mul(i32::from(q[0]));
                // AC (none in a progressive DC scan): read, those under
                // the scale kept.
                if !self.progressive {
                    let mut z = 1;
                    while z < 64 {
                        let Some(rs) = self.huff(2 + usize::from(c.ac), d, at, last) else {
                            return false;
                        };
                        let (r, n) = (usize::from(rs >> 4), u32::from(rs & 15));
                        if n == 0 {
                            if r == 15 {
                                z += 16;
                                continue;
                            }
                            break;
                        }
                        z += r;
                        if z > 63 || n > 10 {
                            return false;
                        }
                        let v = Self::extend(self.get(n, d, at, last), n);
                        let nat = usize::from(ZIGZAG[z]);
                        if nat / 8 < s && nat % 8 < s {
                            coef[nat] = v.wrapping_mul(i32::from(q[nat]));
                        }
                        z += 1;
                    }
                }
                if self.starved {
                    return true;
                }
                idct(&coef, s, &mut px[b * 64..b * 64 + s * s]);
                b += 1;
            }
        }
        true
    }

    /// The MCU's pixels added into the square.
    fn put(&self, px: &[u8; 6 * 64], acc: &mut [u32]) {
        let s = self.s;
        let (mw, mh) = (u32::from(self.hmax) * s, u32::from(self.vmax) * s);
        let (mx, my) = (self.mcu % self.mcus_x, self.mcu / self.mcus_x);
        // Where each component's blocks start in `px`.
        let mut first = [0usize; 3];
        let mut b = 0;
        for (k, f) in first.iter_mut().enumerate().take(self.n) {
            *f = b;
            b += usize::from(self.comps[k].h) * usize::from(self.comps[k].v);
        }
        let sample = |k: usize, x: u32, y: u32| -> i32 {
            let c = &self.comps[k];
            // The component's pixel under MCU pixel (x, y).
            let (cx, cy) = (
                x * u32::from(c.h) / u32::from(self.hmax),
                y * u32::from(c.v) / u32::from(self.vmax),
            );
            let blk = first[k] + (cy / s * u32::from(c.h) + cx / s) as usize;
            i32::from(px[blk * 64 + ((cy % s) * s + cx % s) as usize])
        };
        for y in 0..mh {
            let sy = my * mh + y;
            if sy < self.y0 || !(sy - self.y0).is_multiple_of(self.step) {
                continue;
            }
            let ry = (sy - self.y0) / self.step;
            if ry >= self.taken {
                continue;
            }
            for x in 0..mw {
                let sx = mx * mw + x;
                if sx < self.x0 || !(sx - self.x0).is_multiple_of(self.step) {
                    continue;
                }
                let rx = (sx - self.x0) / self.step;
                if rx >= self.taken {
                    continue;
                }
                let (r, g, b) = if self.n == 1 {
                    let v = sample(0, x, y);
                    (v, v, v)
                } else if self.rgb {
                    (sample(0, x, y), sample(1, x, y), sample(2, x, y))
                } else {
                    let (yy, cb, cr) = (
                        sample(0, x, y),
                        sample(1, x, y) - 128,
                        sample(2, x, y) - 128,
                    );
                    (
                        yy + ((1436 * cr) >> 10),
                        yy - ((352 * cb + 731 * cr) >> 10),
                        yy + ((1815 * cb) >> 10),
                    )
                };
                let v = |c: i32| c.clamp(0, 255) as u32;
                // Each pixel taken into the square's pixels it covers.
                let (ox0, ox1) = span(rx, self.taken);
                let (oy0, oy1) = span(ry, self.taken);
                for oy in oy0..oy1 {
                    for ox in ox0..ox1 {
                        let (fx, fy) = turn(self.orient, ox, oy);
                        let w = &mut acc[fy * SIDE + fx];
                        *w = w.wrapping_add(v(r) << 20 | v(g) << 10 | v(b));
                    }
                }
            }
        }
    }

    /// Pixel (x, y) of the finished square, upright: red, green, blue
    /// (grey where nothing was decoded).
    #[must_use]
    pub fn rgb(&self, acc: &[u32], x: usize, y: usize) -> (u8, u8, u8) {
        let w = acc.get(y * SIDE + x).copied().unwrap_or(0);
        let (ox, oy) = unturn(self.orient, x, y);
        let n = covering(ox, self.taken) * covering(oy, self.taken);
        if n == 0 {
            return (128, 128, 128);
        }
        let c = |sh: u32| ((w >> sh & 1023) / n) as u8;
        (c(20), c(10), c(0))
    }

    /// The picture's size in pixels.
    #[must_use]
    pub const fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// The square's pixels source pixel `r` (of `taken`) covers: one when
/// shrinking, a run when growing (a picture under 96 pixels).
fn span(r: u32, taken: u32) -> (usize, usize) {
    let side = SIDE as u32;
    if taken >= side {
        let o = (r * side / taken) as usize;
        (o, o + 1)
    } else {
        (
            (r * side).div_ceil(taken) as usize,
            ((r + 1) * side).div_ceil(taken).min(side) as usize,
        )
    }
}

/// How many source pixels square pixel `o` holds.
fn covering(o: usize, taken: u32) -> u32 {
    let side = SIDE as u32;
    if taken >= side {
        let o = o as u32;
        // r with r * side / taken == o.
        let lo = (o * taken).div_ceil(side);
        let hi = ((o + 1) * taken).div_ceil(side).min(taken);
        hi.saturating_sub(lo)
    } else {
        1
    }
}

/// A square pixel turned by the Exif orientation (1-8).
const fn turn(o: u8, x: usize, y: usize) -> (usize, usize) {
    let m = SIDE - 1;
    match o {
        2 => (m - x, y),
        3 => (m - x, m - y),
        4 => (x, m - y),
        5 => (y, x),
        6 => (m - y, x),
        7 => (m - y, m - x),
        8 => (y, m - x),
        _ => (x, y),
    }
}

/// [`turn`] undone.
const fn unturn(o: u8, x: usize, y: usize) -> (usize, usize) {
    let m = SIDE - 1;
    match o {
        2 => (m - x, y),
        3 => (m - x, m - y),
        4 => (x, m - y),
        5 => (y, x),
        6 => (y, m - x),
        7 => (m - y, m - x),
        8 => (m - y, x),
        _ => (x, y),
    }
}

/// A block's `s x s` pixels from its dequantized frequencies (the
/// lowest `s x s` of them), level-shifted and clamped.
fn idct(coef: &[i32; 64], s: usize, out: &mut [u8]) {
    if s == 1 {
        out[0] = ((coef[0] + 4) / 8 + 128).clamp(0, 255) as u8;
        return;
    }
    let k = |x: usize, u: usize| -> i64 {
        i64::from(match s {
            2 => K2[x][u],
            4 => K4[x][u],
            _ => K8[x][u],
        })
    };
    // Rows: tmp[v][x] = sum over u of K[x][u] F[v][u].
    let mut tmp = [0i64; 64];
    for v in 0..s {
        for x in 0..s {
            let mut a = 0i64;
            for u in 0..s {
                a += k(x, u) * i64::from(coef[v * 8 + u]);
            }
            tmp[v * 8 + x] = a;
        }
    }
    for y in 0..s {
        for x in 0..s {
            let mut a = 0i64;
            for v in 0..s {
                a += k(y, v) * tmp[v * 8 + x];
            }
            // 1/4, and the two 4096s.
            let p = (a + (1 << 25)) >> 26;
            out[y * s + x] = (p + 128).clamp(0, 255) as u8;
        }
    }
}

#[cfg(test)]
#[path = "jpeg_tests.rs"]
mod tests;
