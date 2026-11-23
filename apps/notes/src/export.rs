//! A note out as a file (notes-rule0.md, "Export"): its text as plain text
//! or Markdown, each ink block as SVG or PNG.
//!
//! Iterate, don't imitate (rule 0): Windows Mobile 5 saved a note only in
//! its own `.pwi` format, readable by nothing but Pocket Word and ActiveSync;
//! Xournal++ exports ink as SVG or PDF from a desktop's gigabytes. Ours
//! writes standard files the PC opens as they are, **streamed**: every
//! format is a pure function of the store, written through a [`Sink`], so
//! a file is first counted ([`Count`]) and then produced a piece at a time
//! into the shell's buffer ([`Window`]), and no note's length ever needs a
//! buffer its size in the program's 256 KB slot. The SVG is the exact
//! curve the screen draws: the brush's Catmull-Rom segments are cubic
//! Beziers with control points `p1 + (p2 - p0) / 6` and `p2 - (p3 - p1) / 6`,
//! so the vector file and the sheet match point for point, each run of
//! equal width one path (width from speed survives). The PNG is indexed
//! colour (coverage levels of each pen colour in the palette, see-through
//! by `tRNS`) deflated with matches at distance 1 and one row up only,
//! which is where ink's empty paper repeats: no hash table, two rows of
//! window, a fraction of a raw image.

use rp_pim::store::Store;

use crate::doc::{self, CONT};
use crate::{EXT, INK_W, PEN, block_rows, each_stroke, stroke_colour, unpack_all};

/// Where an export's bytes go.
pub trait Sink {
    /// `b` written next.
    fn put(&mut self, b: &[u8]);
}

/// Counts the bytes (a file's length before it is written).
#[derive(Default, Debug)]
pub struct Count(pub usize);

impl Sink for Count {
    fn put(&mut self, b: &[u8]) {
        self.0 += b.len();
    }
}

/// Keeps the bytes from `off` that fit `out` (one piece of a file; the
/// bytes before it are produced again and dropped).
pub struct Window<'a> {
    /// Where the piece starts in the file.
    pub off: usize,
    /// The piece.
    pub out: &'a mut [u8],
    /// Bytes seen so far.
    pub at: usize,
    /// Bytes put into `out`.
    pub got: usize,
}

impl<'a> Window<'a> {
    /// The piece of the file at `off` into `out`.
    pub fn new(off: usize, out: &'a mut [u8]) -> Self {
        Self {
            off,
            out,
            at: 0,
            got: 0,
        }
    }
}

impl Sink for Window<'_> {
    fn put(&mut self, b: &[u8]) {
        let (s, e) = (self.at, self.at + b.len());
        self.at = e;
        let lo = s.max(self.off);
        let hi = e.min(self.off + self.out.len());
        if lo < hi {
            self.out[lo - self.off..hi - self.off].copy_from_slice(&b[lo - s..hi - s]);
            self.got = self.got.max(hi - self.off);
        }
    }
}

/// A growing buffer (the host's tests and tools).
#[cfg(test)]
extern crate std;

#[cfg(test)]
impl Sink for std::vec::Vec<u8> {
    fn put(&mut self, b: &[u8]) {
        self.extend_from_slice(b);
    }
}

/// The formats.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    /// Plain text (UTF-8, CRLF lines: Notepad and every editor).
    Text,
    /// Markdown (checklists as task items, ink as images beside it).
    Markdown,
    /// One ink block as SVG.
    Svg,
    /// One ink block as PNG.
    Png,
}

impl Format {
    /// The file name's extension.
    #[must_use]
    pub const fn ext(self) -> &'static [u8] {
        match self {
            Self::Text => b".txt",
            Self::Markdown => b".md",
            Self::Svg => b".svg",
            Self::Png => b".png",
        }
    }
}

/// A file name from the note's title: letters, digits, spaces, `-` and `_`
/// kept (any other byte a `_`, so no path, device name or wildcard can be
/// formed), at most 40, `Note` for none; ink block `block` (1-based, 0 for
/// none) as `-N`; the extension. Its length in `out`.
pub fn file_name(title: &[u8], block: u8, fmt: Format, out: &mut [u8; 64]) -> usize {
    let mut n = 0;
    for &c in title.iter().take(40) {
        let ok = c.is_ascii_alphanumeric() || matches!(c, b' ' | b'-' | b'_');
        out[n] = if ok { c } else { b'_' };
        n += 1;
    }
    // No leading or trailing spaces or dots (Windows drops them).
    while n > 0 && out[n - 1] == b' ' {
        n -= 1;
    }
    if out[..n].iter().all(|&c| c == b'_' || c == b' ') {
        out[..4].copy_from_slice(b"Note");
        n = 4;
    }
    if block > 0 {
        out[n] = b'-';
        n += 1;
        if block >= 10 {
            out[n] = b'0' + block / 10;
            n += 1;
        }
        out[n] = b'0' + block % 10;
        n += 1;
    }
    for &c in fmt.ext() {
        out[n] = c;
        n += 1;
    }
    n
}

/// A number written in decimal.
fn num(s: &mut dyn Sink, v: i64) {
    let mut b = [0u8; 20];
    let mut n = b.len();
    let neg = v < 0;
    let mut u = v.unsigned_abs();
    loop {
        n -= 1;
        b[n] = b'0' + (u % 10) as u8;
        u /= 10;
        if u == 0 {
            break;
        }
    }
    if neg {
        n -= 1;
        b[n] = b'-';
    }
    s.put(&b[n..]);
}

/// A tenth-of-a-pixel value as `12.3`.
fn tenths(s: &mut dyn Sink, v: i64) {
    if v < 0 {
        s.put(b"-");
    }
    let a = v.unsigned_abs();
    num(s, (a / 10) as i64);
    if !a.is_multiple_of(10) {
        s.put(b".");
        num(s, (a % 10) as i64);
    }
}

/// Note `id`'s text as `fmt` ([`Format::Text`] or [`Format::Markdown`]),
/// `title` its title; ink blocks named as their exported images.
pub fn text(store: &Store<'_>, id: u32, title: &[u8], fmt: Format, s: &mut dyn Sink) {
    let md = fmt == Format::Markdown;
    let nl: &[u8] = if md { b"\n" } else { b"\r\n" };
    if !title.is_empty() {
        if md {
            s.put(b"# ");
        }
        s.put(title);
        s.put(nl);
        s.put(nl);
    }
    // What the line before was: 0 nothing, 1 text, 2 a checklist item.
    let mut before = 0u8;
    let mut name = [0u8; 64];
    doc::for_lines(store, id, &mut |_, l| {
        if let Some(b) = doc::ink_block(l) {
            if before != 0 {
                s.put(nl);
                if md {
                    s.put(nl);
                }
            }
            let n = file_name(title, b + 1, Format::Svg, &mut name);
            if md {
                s.put(b"![Ink ");
                num(s, i64::from(b) + 1);
                s.put(b"](");
                // Spaces in a link's target as %20 (CommonMark).
                for &c in &name[..n] {
                    s.put(if c == b' ' {
                        b"%20"
                    } else {
                        core::slice::from_ref(&c)
                    });
                }
                s.put(b")");
            } else {
                s.put(b"[Ink ");
                num(s, i64::from(b) + 1);
                s.put(b": ");
                s.put(&name[..n]);
                s.put(b"]");
            }
            before = 1;
            return true;
        }
        let (check, rest) = if let Some(t) = l.strip_prefix(b"[ ] ") {
            (Some(false), t)
        } else if let Some(t) = l.strip_prefix(b"[x] ") {
            (Some(true), t)
        } else {
            (None, l)
        };
        if let Some(rest) = rest.strip_prefix(&[CONT]) {
            // A continuation joins its paragraph.
            s.put(rest);
            return true;
        }
        match check {
            Some(done) => {
                if before != 0 {
                    s.put(nl);
                    if before == 1 && md {
                        s.put(nl);
                    }
                }
                s.put(match (md, done) {
                    (true, false) => b"- [ ] ",
                    (true, true) => b"- [x] ",
                    (false, false) => b"[ ] ",
                    (false, true) => b"[x] ",
                });
                s.put(rest);
                before = 2;
            }
            None => {
                if before != 0 {
                    s.put(nl);
                    if md {
                        s.put(nl);
                    }
                }
                s.put(rest);
                before = 1;
            }
        }
        true
    });
    if before != 0 {
        s.put(nl);
    }
}

/// The size a stroke of `style` is drawn at: its radius in 1/16 px.
fn radius16(style: u8) -> i64 {
    let base: i64 = 20;
    let r = match style & (crate::SIZE_FINE | crate::SIZE_BOLD) {
        crate::SIZE_FINE => base * 5 / 8,
        crate::SIZE_BOLD => base * 2,
        _ => base,
    };
    if style & crate::MARKER != 0 { r * 3 } else { r }
}

/// Ink block `block` of note `id` as SVG.
pub fn svg(store: &Store<'_>, id: u32, block: u8, s: &mut dyn Sink) {
    let h = i64::from(block_rows(store, id)[usize::from(block)]) * 28;
    s.put(b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"");
    num(s, i64::from(INK_W));
    s.put(b"\" height=\"");
    num(s, h);
    s.put(b"\" viewBox=\"0 0 ");
    num(s, i64::from(INK_W));
    s.put(b" ");
    num(s, h);
    s.put(b"\">\n<g fill=\"none\" stroke-linecap=\"round\" stroke-linejoin=\"round\">\n");
    let mut pts = [(0i16, 0i16); 255];
    let mut ws = [8u8; 255];
    each_stroke(store, id, &mut |packed| {
        if (packed[1] & EXT != 0 && packed[6] & 0x7F != block)
            || (packed[1] & EXT == 0 && block != 0)
        {
            return;
        }
        let u = unpack_all(packed, &mut pts, &mut ws);
        let colour = stroke_colour(u.style);
        let rad = radius16(u.style);
        let marker = u.style & crate::MARKER != 0;
        // The width of point k in tenths of a pixel.
        let wt = |k: usize| -> i64 {
            let scale = if u.widths && !marker {
                i64::from(crate::width_scale(ws[k]))
            } else {
                12
            };
            (2 * rad * scale * 10 / (16 * 12)).max(10)
        };
        let p = |k: isize| {
            let k = k.clamp(0, u.n as isize - 1) as usize;
            (i64::from(pts[k].0) * 10, i64::from(pts[k].1) * 10)
        };
        let head = |s: &mut dyn Sink, w: i64| {
            s.put(b"<path stroke=\"#");
            hex(s, colour);
            let a = colour >> 24;
            if a < 0xF0 {
                s.put(b"\" stroke-opacity=\"0.");
                num(s, i64::from(a * 100 / 255).max(1));
            }
            s.put(b"\" stroke-width=\"");
            tenths(s, w);
            s.put(b"\" d=\"M");
        };
        if u.n == 1 {
            head(s, wt(0));
            let (x, y) = p(0);
            tenths(s, x);
            s.put(b" ");
            tenths(s, y);
            s.put(b"h0\"/>\n");
            return;
        }
        // Runs of one width as one path each; a segment takes the mean of
        // its two points' widths.
        let mut open: Option<i64> = None;
        for i in 0..(u.n as isize - 1) {
            let w = (wt(i as usize) + wt(i as usize + 1)) / 2;
            let (p0, p1, p2, p3) = (p(i - 1), p(i), p(i + 1), p(i + 2));
            if open != Some(w) {
                if open.is_some() {
                    s.put(b"\"/>\n");
                }
                head(s, w);
                tenths(s, p1.0);
                s.put(b" ");
                tenths(s, p1.1);
                open = Some(w);
            }
            let c1 = (p1.0 + (p2.0 - p0.0) / 6, p1.1 + (p2.1 - p0.1) / 6);
            let c2 = (p2.0 - (p3.0 - p1.0) / 6, p2.1 - (p3.1 - p1.1) / 6);
            s.put(b"C");
            for (k, v) in [c1.0, c1.1, c2.0, c2.1, p2.0, p2.1].into_iter().enumerate() {
                if k > 0 {
                    s.put(b" ");
                }
                tenths(s, v);
            }
        }
        if open.is_some() {
            s.put(b"\"/>\n");
        }
    });
    s.put(b"</g>\n</svg>\n");
}

/// A colour's red, green and blue as six hex digits.
fn hex(s: &mut dyn Sink, c: u32) {
    const D: &[u8; 16] = b"0123456789abcdef";
    let mut b = [0u8; 6];
    for (k, out) in b.iter_mut().enumerate() {
        *out = D[((c >> (20 - 4 * k)) & 15) as usize];
    }
    s.put(&b);
}

// ---- PNG ---------------------------------------------------------------

/// The colours a PNG's palette holds: the six pens, then the marker's own
/// yellow and the six marker hues (as [`stroke_colour`] gives them).
fn palette_colours() -> [u32; 13] {
    let mut c = [0u32; 13];
    for (k, out) in c.iter_mut().enumerate() {
        *out = match k {
            0..=5 => stroke_colour(k as u8),
            6 => stroke_colour(crate::MARKER),
            _ => stroke_colour(crate::MARKER | crate::MARKER_HUE | (k as u8 - 7)),
        };
    }
    c
}

/// Rows rendered at once.
pub const BAND: usize = 8;
/// A band's indexed pixels (a filter byte before each row).
pub const BAND_BYTES: usize = BAND * (INK_W as usize + 1);

/// The PNG's working memory (kept in the program, not on its stack).
pub struct PngWork {
    /// The rows being rendered, a filter byte first in each.
    pub band: [u8; BAND_BYTES],
    /// The last row of the band before (the deflater's window).
    pub prev: [u8; INK_W as usize + 1],
}

impl PngWork {
    /// Empty.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            band: [0; BAND_BYTES],
            prev: [0; INK_W as usize + 1],
        }
    }
}

impl Default for PngWork {
    fn default() -> Self {
        Self::new()
    }
}

/// CRC-32 (IEEE) carried on over pieces.
fn crc_more(mut crc: u32, data: &[u8]) -> u32 {
    crc = !crc;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A chunk written whole: its length, type, data and CRC.
fn chunk(s: &mut dyn Sink, ty: &[u8; 4], parts: &[&[u8]]) {
    let len: usize = parts.iter().map(|p| p.len()).sum();
    s.put(&(len as u32).to_be_bytes());
    s.put(ty);
    let mut crc = crc_more(0, ty);
    for p in parts {
        s.put(p);
        crc = crc_more(crc, p);
    }
    s.put(&crc.to_be_bytes());
}

/// Deflate with the fixed codes, its bits gathered into bytes that are
/// handed out as each band's IDAT.
struct Deflate {
    bits: u64,
    nbits: u32,
    adler: (u32, u32),
}

/// The compressed bytes gathered, an IDAT each 64.
struct Out {
    b: [u8; 64],
    n: usize,
}

impl Out {
    fn push(&mut self, b: u8, s: &mut dyn Sink) {
        self.b[self.n] = b;
        self.n += 1;
        if self.n == self.b.len() {
            chunk(s, b"IDAT", &[&self.b[..self.n]]);
            self.n = 0;
        }
    }
}

impl Deflate {
    fn bits(&mut self, v: u32, n: u32, out: &mut dyn FnMut(u8)) {
        self.bits |= u64::from(v) << self.nbits;
        self.nbits += n;
        while self.nbits >= 8 {
            out(self.bits as u8);
            self.bits >>= 8;
            self.nbits -= 8;
        }
    }

    /// A Huffman code (written first bit first).
    fn code(&mut self, code: u32, len: u32, out: &mut dyn FnMut(u8)) {
        let mut r = 0;
        for k in 0..len {
            r |= ((code >> k) & 1) << (len - 1 - k);
        }
        self.bits(r, len, out);
    }

    fn lit(&mut self, v: u32, out: &mut dyn FnMut(u8)) {
        match v {
            0..=143 => self.code(0x30 + v, 8, out),
            144..=255 => self.code(0x190 + v - 144, 9, out),
            256..=279 => self.code(v - 256, 7, out),
            _ => self.code(0xC0 + v - 280, 8, out),
        }
    }

    fn len_dist(&mut self, len: u32, dist: u32, out: &mut dyn FnMut(u8)) {
        const LBASE: [u32; 29] = [
            3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99,
            115, 131, 163, 195, 227, 258,
        ];
        const LEXT: [u32; 29] = [
            0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
        ];
        const DBASE: [u32; 30] = [
            1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025,
            1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
        ];
        const DEXT: [u32; 30] = [
            0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12,
            12, 13, 13,
        ];
        let li = LBASE.iter().rposition(|&b| b <= len).unwrap_or(0);
        self.lit(257 + li as u32, out);
        self.bits(len - LBASE[li], LEXT[li], out);
        let di = DBASE.iter().rposition(|&b| b <= dist).unwrap_or(0);
        self.code(di as u32, 5, out);
        self.bits(dist - DBASE[di], DEXT[di], out);
    }

    /// `data` (the band, after `prev` in the window) compressed.
    fn band(&mut self, prev: &[u8], data: &[u8], out: &mut dyn FnMut(u8)) {
        for &b in data {
            self.adler.0 = (self.adler.0 + u32::from(b)) % 65521;
            self.adler.1 = (self.adler.1 + self.adler.0) % 65521;
        }
        let stride = INK_W as usize + 1;
        // The byte at `i` in the window of the previous row and `data`.
        let at = |i: isize| -> Option<u8> {
            if i >= 0 {
                data.get(i as usize).copied()
            } else {
                let j = prev.len() as isize + i;
                (j >= 0).then(|| prev[j as usize])
            }
        };
        let mut i = 0usize;
        while i < data.len() {
            let mut best = (0u32, 0u32);
            for dist in [1usize, stride] {
                let mut l = 0usize;
                while l < 258
                    && i + l < data.len()
                    && at(i as isize + l as isize - dist as isize) == Some(data[i + l])
                {
                    l += 1;
                }
                if l as u32 > best.0 {
                    best = (l as u32, dist as u32);
                }
            }
            if best.0 >= 3 {
                self.len_dist(best.0, best.1, out);
                i += best.0 as usize;
            } else {
                self.lit(u32::from(data[i]), out);
                i += 1;
            }
        }
    }
}

/// Ink block `block` of note `id` as an indexed-colour PNG, `work` its
/// band memory.
pub fn png(store: &Store<'_>, id: u32, block: u8, work: &mut PngWork, s: &mut dyn Sink) {
    let w = INK_W as usize;
    let h = usize::from(block_rows(store, id)[usize::from(block)]) * 28;
    s.put(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    let mut ihdr = [0u8; 13];
    ihdr[..4].copy_from_slice(&(w as u32).to_be_bytes());
    ihdr[4..8].copy_from_slice(&(h as u32).to_be_bytes());
    ihdr[8] = 8; // bits a pixel
    ihdr[9] = 3; // indexed
    chunk(s, b"IHDR", &[&ihdr]);
    // The palette: 0 clear, then 15 coverage levels of each colour.
    let colours = palette_colours();
    let mut plte = [0u8; 3 * 196];
    let mut trns = [0u8; 196];
    for (ci, &c) in colours.iter().enumerate() {
        for lv in 1..16u32 {
            let e = 1 + ci * 15 + (lv as usize - 1);
            plte[3 * e] = (c >> 16) as u8;
            plte[3 * e + 1] = (c >> 8) as u8;
            plte[3 * e + 2] = c as u8;
            trns[e] = ((c >> 24) * lv * 17 / 255) as u8;
        }
    }
    chunk(s, b"PLTE", &[&plte]);
    chunk(s, b"tRNS", &[&trns]);
    let mut d = Deflate {
        bits: 0,
        nbits: 0,
        adler: (1, 0),
    };
    let mut o = Out { b: [0; 64], n: 0 };
    // zlib's header (deflate, 32K window, no dictionary), then one final
    // block of fixed codes; the IDATs a chunk each 64 bytes as they come.
    o.push(0x78, s);
    o.push(0x01, s);
    d.bits(1, 1, &mut |b| o.push(b, s));
    d.bits(1, 2, &mut |b| o.push(b, s));
    work.prev.iter_mut().for_each(|b| *b = 0);
    let mut pts = [(0i16, 0i16); 255];
    let mut ws = [8u8; 255];
    let mut y0 = 0;
    let mut first = true;
    while y0 < h {
        let rows = BAND.min(h - y0);
        let band = &mut work.band[..rows * (w + 1)];
        band.iter_mut().for_each(|b| *b = 0);
        each_stroke(store, id, &mut |packed| {
            let sb = if packed[1] & EXT != 0 {
                packed[6] & 0x7F
            } else {
                0
            };
            if sb != block {
                return;
            }
            let u = unpack_all(packed, &mut pts, &mut ws);
            let colour = stroke_colour(u.style);
            let ci = colours.iter().position(|&c| c == colour).unwrap_or(0);
            let widths: &[u8] = if u.widths { &ws[..u.n] } else { &[] };
            crate::brush_at(
                INK_W,
                rows as u16,
                256,
                &[&pts[..u.n]],
                &[u.style],
                &[widths],
                (0, -(y0 as i32)),
                None,
                &mut |i, a, _| {
                    let lv = ((a + 8) / 17).min(15) as usize;
                    if lv == 0 {
                        return;
                    }
                    let (r, x) = (i / w, i % w);
                    let p = &mut band[r * (w + 1) + 1 + x];
                    let have = if *p == 0 {
                        0
                    } else {
                        (usize::from(*p) - 1) % 15 + 1
                    };
                    if lv > have {
                        *p = (1 + ci * 15 + lv - 1) as u8;
                    }
                },
            );
        });
        let prev: &[u8] = if first { &[] } else { &work.prev };
        d.band(prev, band, &mut |b| o.push(b, s));
        work.prev
            .copy_from_slice(&band[(rows - 1) * (w + 1)..rows * (w + 1)]);
        first = false;
        y0 += rows;
    }
    d.lit(256, &mut |b| o.push(b, s));
    // The last byte's bits, then the Adler-32.
    if d.nbits > 0 {
        let pad = 8 - d.nbits;
        d.bits(0, pad, &mut |b| o.push(b, s));
    }
    let adler = (d.adler.1 << 16) | d.adler.0;
    for b in adler.to_be_bytes() {
        o.push(b, s);
    }
    if o.n > 0 {
        chunk(s, b"IDAT", &[&o.b[..o.n]]);
    }
    chunk(s, b"IEND", &[]);
}

/// The note's ink blocks that hold strokes (a bit each, block 0 first).
#[must_use]
pub fn inked_blocks(store: &Store<'_>, id: u32) -> u128 {
    let mut m = 0u128;
    each_stroke(store, id, &mut |p| {
        let b = if p[1] & EXT != 0 { p[6] & 0x7F } else { 0 };
        m |= 1 << b;
    });
    m
}

/// [`PEN`]'s count, for the palette's tests.
pub const PENS: usize = PEN.len();
