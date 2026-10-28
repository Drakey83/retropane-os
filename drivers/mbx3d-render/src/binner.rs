//! A whole scene binned by the CPU (P4 step 3, docs/design/p4-ta-speed.md):
//! any number of quads, each tile's list written once, for `rp-tabench` to
//! set against the tile accelerator. The [`crate::Renderer`] keeps a
//! retained scene of [`crate::MAX_SLOTS`] quads, the interface's need; this
//! is the same objects and lists for a game's thousands.
//!
//! Two passes, a counting sort: each tile's quads counted from their
//! bounding boxes, a running sum giving every tile's list its exact place,
//! then each quad's index put there (2 bytes an entry, in SDRAM). The lists
//! then go out in tile order, end to end, so the stores into 2700G memory
//! run in address order and the core reads each list straight through: no
//! link words, nothing to overflow, quads in draw order (the sort is
//! stable). Tiles with no quad share one list. A scene binned once renders
//! again with nothing written but the start table.

use rp_drv_mbx3d::{self as m3, tex};

use crate::{Draw, Error, Kind, LocalMem, Target, draw_tiles, encode, fixed};

/// Where the background object sits, in bytes from the parameter memory's
/// start (as the renderer's).
const BG_AT: u32 = 64;
/// Where the quads' objects start.
const OBJECTS_AT: u32 = 256;
/// Words buffered before a store into 2700G memory.
const BURST: usize = 64;

/// Where a binned scene goes in 2700G memory, and its target's size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The parameter memory (4 KiB aligned; strip pointers count words
    /// from it, 21 bits: 8 MiB).
    pub params: u32,
    /// Its size in bytes: objects, then lists.
    pub params_size: u32,
    /// The region array, 8 bytes a tile.
    pub regions: u32,
    /// The target's size in pixels.
    pub w: u16,
    /// See [`Self::w`].
    pub h: u16,
}

impl Layout {
    /// The tile grid.
    #[must_use]
    pub const fn grid(&self) -> (u32, u32) {
        (self.w as u32 / m3::TILE_W, self.h as u32 / m3::TILE_H)
    }
}

/// SDRAM the binner works in, the caller's (no allocation).
pub struct Scratch<'a> {
    /// A word a tile, and one more.
    pub starts: &'a mut [u32],
    /// One entry a (quad, tile it touches): at most 65,536 quads.
    pub entries: &'a mut [u16],
    /// A word a quad: its first object's strip pointer.
    pub objects: &'a mut [u32],
}

/// What [`bin`] wrote.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Binned {
    /// Tiles in the region array (all of the grid).
    pub tiles: u32,
    /// List entries: quads summed over the tiles they touch.
    pub entries: u32,
    /// The longest list's quads.
    pub most: u32,
    /// Words of objects stored (the background's included).
    pub object_words: u32,
    /// Words of lists stored.
    pub list_words: u32,
    /// Words of the region array stored.
    pub region_words: u32,
    /// Bytes of parameter memory the scene takes.
    pub params_used: u32,
}

impl Binned {
    /// Every word stored into 2700G memory.
    #[must_use]
    pub const fn words(&self) -> u32 {
        self.object_words + self.list_words + self.region_words
    }
}

/// Words written to 2700G memory in address order, [`BURST`] at a time.
struct Out<'m, M> {
    mem: &'m mut M,
    at: u32,
    buf: [u32; BURST],
    n: usize,
    stored: u32,
}

impl<'m, M: LocalMem> Out<'m, M> {
    fn new(mem: &'m mut M, at: u32) -> Self {
        Self {
            mem,
            at,
            buf: [0; BURST],
            n: 0,
            stored: 0,
        }
    }

    fn put(&mut self, w: u32) {
        if self.n == BURST {
            self.flush();
        }
        self.buf[self.n] = w;
        self.n += 1;
    }

    fn put_all(&mut self, ws: &[u32]) {
        for &w in ws {
            self.put(w);
        }
    }

    fn flush(&mut self) {
        self.mem.store(self.at as usize, &self.buf[..self.n]);
        self.at += 4 * self.n as u32;
        self.stored += self.n as u32;
        self.n = 0;
    }

    /// Where the next word goes.
    fn next_at(&self) -> u32 {
        self.at + 4 * self.n as u32
    }
}

/// `draws` binned into `l`: the `background` colour under every tile (as
/// the renderer's, depth written at z 0) or none, each quad's two objects
/// through the renderer's encoder, every tile's list and the region array.
/// Then [`start_table`] renders it, as often as wanted.
///
/// # Errors
///
/// [`Error::NoMemory`] if `scratch` is too small or there are more than
/// 65,536 quads; [`Error::TooComplex`] if the scene doesn't fit
/// `params_size`; [`Error::Unsupported`] for a depth or alpha mode past
/// [`crate::depth`]'s eight.
pub fn bin<M: LocalMem>(
    mem: &mut M,
    l: &Layout,
    background: Option<u32>,
    draws: &[Draw],
    scratch: Scratch<'_>,
) -> Result<Binned, Error> {
    bin_with(mem, l, background, draws.len(), |i| draws[i], scratch)
}

/// [`bin`] of `n` quads that `draw` gives by index (called three times for
/// each, in order), so a scene of thousands needs no array of them: a
/// game's or a test's quads made as they are wanted.
///
/// # Errors
///
/// As [`bin`].
pub fn bin_with<M: LocalMem>(
    mem: &mut M,
    l: &Layout,
    background: Option<u32>,
    n: usize,
    draw: impl Fn(usize) -> Draw,
    scratch: Scratch<'_>,
) -> Result<Binned, Error> {
    let (cols, rows) = l.grid();
    let tiles = (cols * rows) as usize;
    if n > 1 << 16 || scratch.objects.len() < n || scratch.starts.len() <= tiles {
        return Err(Error::NoMemory);
    }
    if (0..n).map(&draw).any(|d| {
        d.depth.is_some_and(|dp| dp.mode > m3::depth::ALWAYS)
            || d.alpha.is_some_and(|a| a.func > m3::depth::ALWAYS)
    }) {
        return Err(Error::Unsupported);
    }
    let starts = &mut scratch.starts[..=tiles];

    // Pass 1: each tile's quads counted (in the slot after its own).
    starts.fill(0);
    let mut total = 0usize;
    for d in (0..n).map(&draw) {
        let r = draw_tiles(&d, cols, rows);
        for y in u32::from(r.y0)..u32::from(r.y1) {
            for x in u32::from(r.x0)..u32::from(r.x1) {
                starts[(y * cols + x) as usize + 1] += 1;
            }
        }
        if !r.is_empty() {
            total += usize::from(r.x1 - r.x0) * usize::from(r.y1 - r.y0);
        }
    }
    if total > scratch.entries.len() {
        return Err(Error::NoMemory);
    }
    // The running sum: each tile's first entry.
    let mut most = 0;
    for k in 1..=tiles {
        most = most.max(starts[k]);
        starts[k] += starts[k - 1];
    }
    // Pass 2: each quad's index into its tiles' places; after it,
    // `starts[k]` is tile k's end (tile k + 1's start).
    for (i, d) in (0..n).map(&draw).enumerate() {
        let r = draw_tiles(&d, cols, rows);
        for y in u32::from(r.y0)..u32::from(r.y1) {
            for x in u32::from(r.x0)..u32::from(r.x1) {
                let k = (y * cols + x) as usize;
                scratch.entries[starts[k] as usize] = i as u16;
                starts[k] += 1;
            }
        }
    }

    // The objects, in order: the background, then each quad's two.
    let mut objs = Out::new(&mut *mem, l.params + BG_AT);
    let bg = background.map(|c| {
        objs.put_all(&background_object(c, l.w, l.h));
        m3::STRIP_ONE_TRIANGLE | (BG_AT / 4)
    });
    objs.flush();
    objs.at = l.params + OBJECTS_AT;
    for (i, d) in (0..n).map(&draw).enumerate() {
        let (words, len, kind) = encode(&d, false, 0);
        let flags = if kind == Kind::Textured {
            tex::STRIP_GAME
        } else {
            m3::STRIP_ONE_TRIANGLE
        };
        scratch.objects[i] = flags | ((objs.next_at() - l.params) / 4);
        if objs.next_at() + 4 * len as u32 > l.params + l.params_size {
            return Err(Error::TooComplex);
        }
        objs.put_all(&words[..len]);
    }
    objs.flush();
    let object_words = objs.stored;
    let lists_at = objs.at.next_multiple_of(64);

    // The lists in tile order, then the region array, each in address
    // order. Empty tiles share the first list: the background alone.
    let end = l.params + l.params_size;
    let mut lists = Out::new(&mut *mem, lists_at);
    let empty = lists.next_at();
    lists.put_all(bg.as_slice());
    lists.put(m3::END);
    let mut region_out = RegionOut::new(l.regions);
    let mut begin = 0usize;
    for (k, &stop) in starts[..tiles].iter().enumerate() {
        let stop = stop as usize;
        let at = if stop == begin {
            empty
        } else {
            let at = lists.next_at();
            if at + 4 * (2 * (stop - begin) + 2) as u32 > end {
                return Err(Error::TooComplex);
            }
            lists.put_all(bg.as_slice());
            for &q in &scratch.entries[begin..stop] {
                let p = scratch.objects[usize::from(q)];
                let words = if p & 0xFFE0_0000 == tex::STRIP_GAME {
                    tex::WORDS as u32
                } else {
                    m3::TRIANGLE_WORDS as u32
                };
                lists.put(p);
                lists.put(p + words);
            }
            lists.put(m3::END);
            at
        };
        begin = stop;
        let (x, y) = (k as u32 % cols, k as u32 / cols);
        region_out.entries[region_out.n] = m3::region_entry(x, y, at, k + 1 == tiles);
        region_out.n += 1;
        if region_out.n == region_out.entries.len() {
            region_out.flush(lists.mem);
        }
    }
    lists.flush();
    region_out.flush(lists.mem);
    Ok(Binned {
        tiles: tiles as u32,
        entries: total as u32,
        most,
        object_words,
        list_words: lists.stored,
        region_words: 2 * tiles as u32,
        params_used: lists.at - l.params,
    })
}

/// Region entries buffered beside the lists' stream.
struct RegionOut {
    at: u32,
    entries: [[u32; 2]; BURST / 2],
    n: usize,
}

impl RegionOut {
    const fn new(at: u32) -> Self {
        Self {
            at,
            entries: [[0; 2]; BURST / 2],
            n: 0,
        }
    }

    fn flush<M: LocalMem>(&mut self, mem: &mut M) {
        mem.store(self.at as usize, self.entries[..self.n].as_flattened());
        self.at += 8 * self.n as u32;
        self.n = 0;
    }
}

/// The renderer's background: one triangle twice the target's size, depth
/// written at z 0.
fn background_object(argb: u32, w: u16, h: u16) -> [u32; m3::TRIANGLE_WORDS] {
    let (w, h) = (i32::from(w) * 2, i32::from(h) * 2);
    let f = |v: i32| f32::from_bits(fixed::f32_bits(v, 0));
    let v = [(0, 0), (w, 0), (0, h)].map(|(x, y)| m3::Vertex {
        x: f(x),
        y: f(y),
        z: f(0),
        rhw: f(1),
        argb,
    });
    let tsp = m3::depth_test(m3::TSP_DELL, m3::depth::ALWAYS, true);
    m3::triangle(m3::ISP_ALWAYS, tsp, &v)
}

/// The 3D stage's start table for a binned scene into `target` (the
/// renderer's: Dell's table, the target's format and address); the last
/// pair starts the render. The parameter memory manager must be set up
/// (`rp_drv_mbx3d::pm_init_table`) and the 3D clock on.
#[must_use]
pub fn start_table(l: &Layout, target: Target) -> [(usize, u32); 16] {
    let pic = target.picture;
    let dell = m3::start_table(
        l.regions,
        l.params,
        u32::from(pic.w),
        u32::from(pic.h),
        pic.stride,
    );
    let mut t = [(0usize, 0u32); 16];
    for (k, &(off, v)) in dell[..14].iter().enumerate() {
        t[k] = (off, if off == 0x650 { pic.format.code() } else { v });
    }
    t[14] = (0x65C, target.addr());
    t[15] = (m3::START, 1);
    t
}
