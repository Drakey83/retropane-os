//! The radar's rain frames on the device (R184 step 2, sheet
//! docs/research/sheets/r184-radar-loop.md, way C).
//!
//! Iterate, don't imitate: web radars (RainViewer's own example, Leaflet,
//! MapLibre) keep every frame as a layer and blend map and rain on every
//! screen refresh; until now we blended every pixel on the CPU at every
//! loop step and wrote the whole map into 2700G memory again (run
//! today-0053: ~35 ms of CPU and a ~36 ms whole-screen redraw a step). Here
//! each frame is composed once, only in the bands of rows where it rains,
//! into RGB565 band slots in 2700G memory (a few large pictures taken when
//! Radar opens, handed out 42 rows at a time: no pool fragmentation); a
//! loop step only points the renderer's wallpaper bands at that frame's
//! slots and redraws the box where the rain changed. The rain itself is kept as the PNG answers
//! that brought it (~0.1-0.4 MB for 13 frames, against 7.1 MB of 32-bit
//! frames), decoded again only when a frame is composed (a look change,
//! or Radar opened again).

use rp_draw::Rect;
use rp_weather::maplayers::{Look, rain_over};
use rp_weather::radar::PAST;
use rp_weather::tiles::{VIEW_H, VIEW_W};

/// The bands' height: the view's 462 rows in 11 bands of 42.
pub const BAND_H: u16 = 42;
/// Bands in the view.
pub const BANDS: usize = 11;
const _: () = assert!(BAND_H as usize * BANDS == VIEW_H);
const _: () = assert!(BANDS <= rp_mbx_render::BANDS);

/// Tiles in a frame (the view's 3 x 3).
pub const TILES: usize = 9;

/// A rain frame composed over the map: each rainy band's slot in the
/// radar's band pictures ([`BandPool`]), and each band's rain columns.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Composed {
    /// Per band, its slot (`None`: no rain there, the map shows).
    pub slots: [Option<u16>; BANDS],
    /// Per band, the columns it rains in (first, last).
    pub spans: [Option<(u16, u16)>; BANDS],
}

/// Bytes of the store's records before its arena: per row and tile where
/// its bytes are, each frame's row, each row's composed frame.
const AT: usize = 0;
const ORDER: usize = AT + PAST * TILES * 8;
const REC: usize = 1 + BANDS * 2 + BANDS * 4;
const COMPOSED: usize = ORDER + PAST;
/// Where the PNGs start in the store's bytes.
pub const DATA: usize = COMPOSED + PAST * REC;

/// The rain's PNG answers, by frame and tile, in one arena (compacted when
/// full), and each frame's composed bands. Everything lives in the bytes
/// it is given (from the RAM pool, not the image's stack: P6, run
/// emu-today 2026-11-04 at 85% with these tables on the stack). Each
/// frame's records sit in a row; when RainViewer's list moves on only the
/// frame-to-row map changes (13 bytes, no copies).
pub struct Store<'a> {
    bytes: &'a mut [u8],
    top: usize,
}

impl<'a> Store<'a> {
    /// An empty store in `bytes` (more than [`DATA`] long, else it keeps
    /// nothing).
    #[must_use]
    pub fn new(bytes: &'a mut [u8]) -> Self {
        let n = bytes.len().min(DATA);
        bytes[..n].fill(0);
        let mut s = Self { bytes, top: DATA };
        if s.bytes.len() > DATA {
            for f in 0..PAST {
                s.bytes[ORDER + f] = f as u8;
            }
        }
        s
    }

    fn live(&self) -> bool {
        self.bytes.len() > DATA
    }

    fn at(&self, r: usize, k: usize) -> (usize, usize) {
        let i = AT + (r * TILES + k) * 8;
        let w = |j: usize| {
            u32::from_le_bytes([
                self.bytes[j],
                self.bytes[j + 1],
                self.bytes[j + 2],
                self.bytes[j + 3],
            ]) as usize
        };
        (w(i), w(i + 4))
    }

    fn set_at(&mut self, r: usize, k: usize, o: usize, n: usize) {
        let i = AT + (r * TILES + k) * 8;
        self.bytes[i..i + 4].copy_from_slice(&(o as u32).to_le_bytes());
        self.bytes[i + 4..i + 8].copy_from_slice(&(n as u32).to_le_bytes());
    }

    fn clear_row(&mut self, r: usize) {
        let i = AT + r * TILES * 8;
        self.bytes[i..i + TILES * 8].fill(0);
    }

    /// Frame `f`'s row.
    fn row(&self, f: usize) -> usize {
        usize::from(self.bytes[ORDER + f])
    }

    /// Keeps tile `k` of frame `f`. A tile the frame already has starts the
    /// frame afresh (it is being fetched again). False if the arena can't
    /// hold it even compacted.
    pub fn put(&mut self, f: usize, k: usize, png: &[u8]) -> bool {
        if f >= PAST || k >= TILES || png.is_empty() || !self.live() {
            return false;
        }
        let r = self.row(f);
        if self.at(r, k).1 != 0 {
            self.clear_row(r);
        }
        if self.top + png.len() > self.bytes.len() {
            self.compact();
        }
        let Some(dst) = self.bytes.get_mut(self.top..self.top + png.len()) else {
            return false;
        };
        dst.copy_from_slice(png);
        self.set_at(r, k, self.top, png.len());
        self.top += png.len();
        true
    }

    /// Forgets frame `f`'s tiles.
    pub fn clear(&mut self, f: usize) {
        if f < PAST && self.live() {
            self.clear_row(self.row(f));
        }
    }

    /// Tile `k` of frame `f`, if kept.
    #[must_use]
    pub fn tile(&self, f: usize, k: usize) -> Option<&[u8]> {
        if f >= PAST || k >= TILES || !self.live() {
            return None;
        }
        let (o, n) = self.at(self.row(f), k);
        (n != 0).then(|| &self.bytes[o..o + n])
    }

    /// RainViewer's list moved on (P7, `Shell::take_radar_moves`): frame
    /// `i` is now what frame `moves[i]` was (`u8::MAX`: none yet, given an
    /// emptied row). Returns the composed frames that went, by row (a bit
    /// each): the caller takes them ([`Self::take_row`]) and gives back
    /// their band slots.
    pub fn moved(&mut self, moves: &[u8; PAST]) -> u16 {
        if !self.live() {
            return 0;
        }
        let mut old = [0u8; PAST];
        old.copy_from_slice(&self.bytes[ORDER..ORDER + PAST]);
        let mut used = 0u16;
        for &j in moves {
            if let Some(&r) = old.get(usize::from(j)) {
                used |= 1 << r;
            }
        }
        let gone = !used & ((1 << PAST) - 1);
        let mut free = gone;
        for (i, &j) in moves.iter().enumerate() {
            self.bytes[ORDER + i] = match old.get(usize::from(j)) {
                Some(&r) => r,
                None => {
                    let r = free.trailing_zeros() as u8;
                    free &= !(1 << r);
                    self.clear_row(usize::from(r));
                    r
                }
            };
        }
        gone
    }

    /// Frame `f`'s composed bands.
    #[must_use]
    pub fn composed(&self, f: usize) -> Option<Composed> {
        (f < PAST && self.live())
            .then(|| self.rec(self.row(f)))
            .flatten()
    }

    /// Frame `f`'s composed bands set (`None`: none).
    pub fn set_composed(&mut self, f: usize, c: Option<Composed>) {
        if f < PAST && self.live() {
            self.set_rec(self.row(f), c);
        }
    }

    /// Frame `f`'s composed bands, taken.
    pub fn take_composed(&mut self, f: usize) -> Option<Composed> {
        let c = self.composed(f);
        self.set_composed(f, None);
        c
    }

    /// Row `r`'s composed bands, taken ([`Self::moved`]'s rows).
    pub fn take_row(&mut self, r: usize) -> Option<Composed> {
        if r >= PAST || !self.live() {
            return None;
        }
        let c = self.rec(r);
        self.set_rec(r, None);
        c
    }

    fn rec(&self, r: usize) -> Option<Composed> {
        let b = &self.bytes[COMPOSED + r * REC..COMPOSED + (r + 1) * REC];
        if b[0] == 0 {
            return None;
        }
        let w = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
        let mut c = Composed {
            slots: [None; BANDS],
            spans: [None; BANDS],
        };
        for k in 0..BANDS {
            c.slots[k] = Some(w(1 + 2 * k)).filter(|&s| s != u16::MAX);
            let (s, e) = (w(1 + 2 * BANDS + 4 * k), w(3 + 2 * BANDS + 4 * k));
            c.spans[k] = (s <= e).then_some((s, e));
        }
        Some(c)
    }

    fn set_rec(&mut self, r: usize, c: Option<Composed>) {
        let b = &mut self.bytes[COMPOSED + r * REC..COMPOSED + (r + 1) * REC];
        b.fill(0);
        let Some(c) = c else {
            return;
        };
        b[0] = 1;
        for k in 0..BANDS {
            let s = c.slots[k].unwrap_or(u16::MAX);
            b[1 + 2 * k..3 + 2 * k].copy_from_slice(&s.to_le_bytes());
            let (a, z) = c.spans[k].unwrap_or((1, 0));
            let i = 1 + 2 * BANDS + 4 * k;
            b[i..i + 2].copy_from_slice(&a.to_le_bytes());
            b[i + 2..i + 4].copy_from_slice(&z.to_le_bytes());
        }
    }

    /// PNG bytes held.
    #[must_use]
    pub fn used(&self) -> usize {
        if !self.live() {
            return 0;
        }
        (0..PAST)
            .flat_map(|r| (0..TILES).map(move |k| (r, k)))
            .map(|(r, k)| self.at(r, k).1)
            .sum()
    }

    /// Moves every kept tile to the arena's front, in place.
    fn compact(&mut self) {
        let mut top = DATA;
        loop {
            // The kept tile lowest in the arena at or past `top`.
            let next = (0..PAST)
                .flat_map(|r| (0..TILES).map(move |k| (r, k)))
                .map(|(r, k)| (r, k, self.at(r, k)))
                .filter(|&(_, _, (o, n))| n != 0 && o >= top)
                .min_by_key(|&(_, _, (o, _))| o);
            let Some((r, k, (o, n))) = next else {
                break;
            };
            self.bytes.copy_within(o..o + n, top);
            self.set_at(r, k, top, n);
            top += n;
        }
        self.top = top;
    }
}

/// Band pictures at most (each up to [`SLOTS_PER_PIC`] bands: a picture is
/// at most 2048 rows).
pub const PICS: usize = 3;
/// Band slots in one picture (48 x 42 = 2,016 rows).
pub const SLOTS_PER_PIC: u16 = 48;
/// Band slots wanted: every band of every frame.
pub const SLOTS: u16 = (PAST * BANDS) as u16;
const _: () = assert!(SLOTS <= SLOTS_PER_PIC * PICS as u16);

/// The band slots in the radar's pictures (`H`: the graphics service's
/// picture handle): which are free, and where each lies.
pub struct BandPool<H> {
    pics: [Option<(H, u16)>; PICS],
    free: [u64; PICS],
}

impl<H: Copy> BandPool<H> {
    /// No pictures yet.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pics: [None; PICS],
            free: [0; PICS],
        }
    }

    /// Picture `h`, `bands` slots tall, added (all free); false if there
    /// is no room for another.
    pub fn add(&mut self, h: H, bands: u16) -> bool {
        let bands = bands.min(SLOTS_PER_PIC);
        let Some(k) = self.pics.iter().position(Option::is_none) else {
            return false;
        };
        self.pics[k] = Some((h, bands));
        self.free[k] = if bands == 64 {
            u64::MAX
        } else {
            (1u64 << bands) - 1
        };
        true
    }

    /// The pictures (to give back), the pool emptied.
    pub fn take_pics(&mut self) -> [Option<H>; PICS] {
        self.free = [0; PICS];
        core::array::from_fn(|k| self.pics[k].take().map(|p| p.0))
    }

    /// Slots in all.
    #[must_use]
    pub fn total(&self) -> u16 {
        self.pics.iter().flatten().map(|p| p.1).sum()
    }

    /// Slots free.
    #[must_use]
    pub fn free(&self) -> u16 {
        self.free.iter().map(|f| f.count_ones() as u16).sum()
    }

    /// A free slot, now taken.
    pub fn take(&mut self) -> Option<u16> {
        let k = self.free.iter().position(|&f| f != 0)?;
        let b = self.free[k].trailing_zeros() as u16;
        self.free[k] &= !(1 << b);
        Some(k as u16 * SLOTS_PER_PIC + b)
    }

    /// Slot `s` given back.
    pub fn give(&mut self, s: u16) {
        let (k, b) = (usize::from(s / SLOTS_PER_PIC), s % SLOTS_PER_PIC);
        if let (Some(f), Some(Some((_, n)))) = (self.free.get_mut(k), self.pics.get(k))
            && b < *n
        {
            *f |= 1 << b;
        }
    }

    /// Slot `s`'s picture and first row.
    #[must_use]
    pub fn at(&self, s: u16) -> Option<(H, u16)> {
        let (k, b) = (usize::from(s / SLOTS_PER_PIC), s % SLOTS_PER_PIC);
        let (h, n) = (*self.pics.get(k)?)?;
        (b < n).then_some((h, b * BAND_H))
    }
}

impl<H: Copy> Default for BandPool<H> {
    fn default() -> Self {
        Self::new()
    }
}

/// Per band, the columns where `rain` (the view, ARGB) has any rain
/// (first, last), or `None`.
#[must_use]
pub fn bands_of(rain: &[u32]) -> [Option<(u16, u16)>; BANDS] {
    let mut out = [None; BANDS];
    for (b, o) in out.iter_mut().enumerate() {
        let mut span: Option<(u16, u16)> = None;
        for y in b * BAND_H as usize..(b + 1) * BAND_H as usize {
            let Some(row) = rain.get(y * VIEW_W..(y + 1) * VIEW_W) else {
                break;
            };
            let first = row.iter().position(|c| c >> 24 != 0);
            let last = row.iter().rposition(|c| c >> 24 != 0);
            if let (Some(a), Some(z)) = (first, last) {
                let (a, z) = (a as u16, z as u16);
                span = Some(span.map_or((a, z), |(s, e)| (s.min(a), e.max(z))));
            }
        }
        *o = span;
    }
    out
}

/// A composed row (or a run of one): the rain (`rain`, ARGB) over the map (`map`, opaque) in
/// `look`, as RGB565 two pixels a word (the left one low), the screen's
/// top bits as the renderer's own wallpaper rows.
pub fn row565(map: &[u32], rain: &[u32], look: Look, out: &mut [u32]) {
    let px = |x: usize| {
        let m = map.get(x).copied().unwrap_or(0);
        let c = rain.get(x).copied().unwrap_or(0);
        rp_mbx_render::to565(rain_over(m, c, look))
    };
    for (i, w) in out.iter_mut().enumerate().take(VIEW_W / 2) {
        *w = u32::from(px(2 * i)) | u32::from(px(2 * i + 1)) << 16;
    }
}

/// The screen a switch from frame bands `a` to `b` changes (the view's
/// top at screen row `top`): every band either rains in, across the
/// columns either does; empty if neither has rain.
#[must_use]
pub fn changed(a: &[Option<(u16, u16)>; BANDS], b: &[Option<(u16, u16)>; BANDS], top: u16) -> Rect {
    let mut r: Option<Rect> = None;
    for k in 0..BANDS {
        let span = match (a[k], b[k]) {
            (Some((s, e)), Some((s2, e2))) => Some((s.min(s2), e.max(e2))),
            (x, y) => x.or(y),
        };
        if let Some((s, e)) = span {
            let band = Rect::new(s, top + k as u16 * BAND_H, e - s + 1, BAND_H);
            r = Some(r.map_or(band, |r| r.union(&band)));
        }
    }
    r.unwrap_or(Rect::new(0, 0, 0, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_store_keeps_moves_and_compacts_without_losing_a_tile() {
        let mut arena = vec![0u8; DATA + 64];
        let mut s = Store::new(&mut arena);
        assert!(s.put(0, 0, &[1; 10]));
        assert!(s.put(0, 1, &[2; 10]));
        assert!(s.put(1, 0, &[3; 20]));
        assert_eq!(s.used(), 40);
        // Frame 0 fetched again: its old tiles forgotten.
        assert!(s.put(0, 0, &[4; 10]));
        assert_eq!(s.tile(0, 1), None);
        // Room only after compaction (10 + 20 kept, 30 more).
        assert!(s.put(2, 0, &[5; 30]));
        assert_eq!(s.tile(0, 0), Some(&[4u8; 10][..]));
        assert_eq!(s.tile(1, 0), Some(&[3u8; 20][..]));
        assert_eq!(s.tile(2, 0), Some(&[5u8; 30][..]));
        assert!(!s.put(3, 0, &[6; 10]), "full even compacted");
        assert!(!s.put(PAST, 0, &[6; 1]) && !s.put(0, TILES, &[6; 1]));
        // Composed bands go with their frame.
        let c = |n: u16| Composed {
            slots: [Some(n); BANDS],
            spans: [Some((n, n + 1)); BANDS],
        };
        s.set_composed(0, Some(c(5)));
        s.set_composed(1, Some(c(7)));
        let mut half = c(9);
        half.slots[3] = None;
        half.spans[4] = None;
        s.set_composed(2, Some(half));
        assert_eq!(s.composed(2), Some(half), "every field kept");
        // The list moves on: frame 0 is gone, 1 and 2 move down.
        let mut m = [u8::MAX; PAST];
        m[0] = 1;
        m[1] = 2;
        let gone = s.moved(&m);
        assert_eq!(
            gone.count_ones(),
            PAST as u32 - 2,
            "every row but the two kept"
        );
        let went: Vec<_> = (0..PAST)
            .filter_map(|r| (gone & 1 << r != 0).then(|| s.take_row(r)))
            .flatten()
            .collect();
        assert_eq!(went, vec![c(5)], "frame 0's bands to give back");
        assert_eq!(s.composed(0), Some(c(7)));
        assert_eq!(s.take_composed(1), Some(half));
        assert_eq!(s.composed(1), None);
        assert_eq!(s.tile(0, 0), Some(&[3u8; 20][..]));
        assert_eq!(s.tile(1, 0), Some(&[5u8; 30][..]));
        assert_eq!(s.tile(2, 0), None);
        assert!(s.put(2, 0, &[7; 10]), "the old frame's room reused");
        assert_eq!(s.tile(1, 0), Some(&[5u8; 30][..]));
        // Too small to hold its own records: keeps nothing, never panics.
        let mut tiny = [0u8; 16];
        let mut t = Store::new(&mut tiny);
        assert!(!t.put(0, 0, &[1]));
        assert_eq!((t.tile(0, 0), t.composed(0), t.used()), (None, None, 0));
    }

    #[test]
    fn bands_find_the_rain_and_the_change_covers_both_frames() {
        let mut rain = vec![0u32; VIEW_W * VIEW_H];
        rain[5 * VIEW_W + 100] = 0x4000_FF00;
        rain[40 * VIEW_W + 300] = 0xFF00_FF00;
        rain[461 * VIEW_W + 479] = 0x0100_0000;
        let b = bands_of(&rain);
        assert_eq!(b[0], Some((100, 300)));
        assert_eq!(b[BANDS - 1], Some((479, 479)));
        assert!(b[1..BANDS - 1].iter().all(Option::is_none));
        let mut none = [None; BANDS];
        assert_eq!(changed(&none, &none, 70), Rect::new(0, 0, 0, 0));
        none[2] = Some((10, 20));
        let r = changed(&b, &none, 70);
        assert_eq!((r.x, r.y), (10, 70));
        assert_eq!((r.x + r.w, r.y + r.h), (480, 70 + 462));
    }

    #[test]
    fn the_band_pool_hands_out_each_slot_once_and_takes_it_back() {
        let mut p: BandPool<u8> = BandPool::new();
        assert_eq!(p.take(), None, "no pictures");
        assert!(p.add(7, 2) && p.add(9, 48));
        assert_eq!((p.total(), p.free()), (50, 50));
        let a = p.take().unwrap();
        let b = p.take().unwrap();
        let c = p.take().unwrap();
        assert_eq!(p.at(a), Some((7, 0)));
        assert_eq!(p.at(b), Some((7, BAND_H)));
        assert_eq!(
            p.at(c),
            Some((9, 0)),
            "the second picture once the first is full"
        );
        assert_eq!(p.at(SLOTS_PER_PIC - 1), None, "past the first's slots");
        p.give(b);
        assert_eq!(p.take(), Some(b));
        p.give(SLOTS_PER_PIC - 1);
        assert_eq!(p.free(), 47, "a slot that isn't one is never handed out");
        assert!(p.add(1, 1));
        assert!(!p.add(2, 1), "three pictures at most");
        assert_eq!(p.take_pics(), [Some(7), Some(9), Some(1)]);
        assert_eq!((p.total(), p.take()), (0, None));
    }

    /// A composed row is what the renderer's wallpaper would hold for the
    /// same map and rain: today's blend on Night, the look's factor on the
    /// light maps, truncated to RGB565.
    #[test]
    fn a_composed_row_is_the_rain_over_the_map_at_16_bits() {
        let map = [0xFF22_282Eu32; VIEW_W];
        let mut rain = [0u32; VIEW_W];
        rain[1] = 0x8C3C_BE50;
        let mut out = [0u32; VIEW_W / 2];
        row565(&map, &rain, Look::Night, &mut out);
        let m = u32::from(rp_mbx_render::to565(0xFF22_282E));
        let r = u32::from(rp_mbx_render::to565(rain_over(
            0xFF22_282E,
            0x8C3C_BE50,
            Look::Night,
        )));
        assert_eq!(out[0], m | r << 16);
        assert_eq!(out[1], m | m << 16);
        let mut paper = [0u32; VIEW_W / 2];
        row565(&map, &rain, Look::Paper, &mut paper);
        assert_ne!(paper[0], out[0], "stronger on the light map");
    }
}
