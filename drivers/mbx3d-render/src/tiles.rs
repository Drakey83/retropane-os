//! Sets of tiles (8 x 16 pixels, measured: `rp-mbx3d` version 3) as bits:
//! 300 bytes for a whole 480 x 640 screen.

use rp_drv_mbx3d::{TILE_H, TILE_W};

/// Tiles a target may have: 480 x 640 in either orientation.
pub const MAX_TILES: usize = 2400;
const WORDS: usize = MAX_TILES.div_ceil(32);

/// A rectangle of tiles, `x1`/`y1` exclusive; empty when `x0 >= x1` or
/// `y0 >= y1`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TileRect {
    /// First column.
    pub x0: u8,
    /// First row.
    pub y0: u8,
    /// One past the last column.
    pub x1: u8,
    /// One past the last row.
    pub y1: u8,
}

impl TileRect {
    /// No tiles.
    pub const EMPTY: Self = Self {
        x0: 0,
        y0: 0,
        x1: 0,
        y1: 0,
    };

    /// Whether it holds no tile.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }

    /// Whether it holds tile `(x, y)`.
    #[must_use]
    pub const fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.x0 as u32 && x < self.x1 as u32 && y >= self.y0 as u32 && y < self.y1 as u32
    }

    /// The tiles of a `cols` x `rows` grid that pixels `[px0, px1)` x
    /// `[py0, py1)` touch (pixels may lie outside the grid: clipped).
    #[must_use]
    pub fn of_pixels(px0: i32, py0: i32, px1: i32, py1: i32, cols: u32, rows: u32) -> Self {
        if px0 >= px1 || py0 >= py1 {
            return Self::EMPTY;
        }
        let (w, h) = (TILE_W as i32, TILE_H as i32);
        let clip = |v: i32, n: u32| v.clamp(0, n as i32) as u8;
        let r = Self {
            x0: clip(px0.div_euclid(w), cols),
            y0: clip(py0.div_euclid(h), rows),
            x1: clip((px1 + w - 1).div_euclid(w), cols),
            y1: clip((py1 + h - 1).div_euclid(h), rows),
        };
        if r.is_empty() { Self::EMPTY } else { r }
    }
}

/// A set of tiles of a grid `cols` wide, as bits in row order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tiles {
    bits: [u32; WORDS],
}

impl Default for Tiles {
    fn default() -> Self {
        Self::new()
    }
}

impl Tiles {
    /// No tiles.
    #[must_use]
    pub const fn new() -> Self {
        Self { bits: [0; WORDS] }
    }

    /// Adds tile number `k` (`y * cols + x`).
    pub fn add(&mut self, k: usize) {
        if k < MAX_TILES {
            self.bits[k / 32] |= 1 << (k % 32);
        }
    }

    /// Whether tile number `k` is in.
    #[must_use]
    pub fn has(&self, k: usize) -> bool {
        k < MAX_TILES && self.bits[k / 32] & (1 << (k % 32)) != 0
    }

    /// Adds every tile of `r` in a grid `cols` wide.
    pub fn add_rect(&mut self, r: TileRect, cols: u32) {
        if r.is_empty() {
            return;
        }
        for y in u32::from(r.y0)..u32::from(r.y1) {
            for x in u32::from(r.x0)..u32::from(r.x1) {
                self.add((y * cols + x) as usize);
            }
        }
    }

    /// Adds the first `n` tiles.
    pub fn add_first(&mut self, n: usize) {
        let n = n.min(MAX_TILES);
        for w in &mut self.bits[..n / 32] {
            *w = u32::MAX;
        }
        if !n.is_multiple_of(32) {
            self.bits[n / 32] |= (1 << (n % 32)) - 1;
        }
    }

    /// Adds every tile of `other`.
    pub fn add_all(&mut self, other: &Self) {
        for (a, b) in self.bits.iter_mut().zip(other.bits.iter()) {
            *a |= b;
        }
    }

    /// Removes every tile.
    pub fn clear(&mut self) {
        self.bits = [0; WORDS];
    }

    /// How many tiles are in.
    #[must_use]
    pub fn count(&self) -> u32 {
        self.bits.iter().map(|w| w.count_ones()).sum()
    }

    /// Whether no tile is in.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bits.iter().all(|&w| w == 0)
    }

    /// The tile numbers in, a `cols`-wide grid of `rows` read row by row
    /// from the top, each row from its right end.
    pub fn iter_cols_rev(&self, cols: usize, rows: usize) -> impl Iterator<Item = usize> + '_ {
        (0..rows)
            .flat_map(move |y| (0..cols).rev().map(move |x| y * cols + x))
            .filter(move |&k| self.has(k))
    }

    /// The tile numbers in, descending (the reverse of [`Self::iter`]).
    pub fn iter_rev(&self) -> impl Iterator<Item = usize> + '_ {
        self.bits.iter().enumerate().rev().flat_map(|(i, &w)| {
            let mut w = w;
            core::iter::from_fn(move || {
                if w == 0 {
                    return None;
                }
                let b = 31 - w.leading_zeros() as usize;
                w &= !(1 << b);
                Some(i * 32 + b)
            })
        })
    }

    /// The tile numbers in, ascending (row order).
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.bits.iter().enumerate().flat_map(|(i, &w)| {
            let mut w = w;
            core::iter::from_fn(move || {
                if w == 0 {
                    return None;
                }
                let b = w.trailing_zeros() as usize;
                w &= w - 1;
                Some(i * 32 + b)
            })
        })
    }
}

/// `n / cols` for `n < 128 * 128` and `cols` in 1..=128, by a multiply:
/// the XScale has no divide instruction (each `/` is a library call of
/// tens of instructions). `recip` is [`recip`]`(cols)`.
#[must_use]
pub(crate) const fn div_cols(n: u32, recip: u32) -> u32 {
    ((n as u64 * recip as u64) >> 31) as u32
}

/// The multiplier [`div_cols`] takes: `2^31 / cols`, plus one (fits 32
/// bits for every `cols`, one included; exact while `n x cols < 2^31 /
/// cols`, here 16,384 x 128 < 2^31 / 128).
#[must_use]
pub(crate) const fn recip(cols: u32) -> u32 {
    (1u32 << 31) / cols + 1
}

/// Each tile of `todo` in `order` (of a `cols` x `rows` grid), as its
/// number `k = y * cols + x` with `x` and `y`, given to `f` until it says
/// stop (`None`). The order the renderer's region array lists (R88: no
/// division per tile; the old walk did four or more, ~2 us a tile with the
/// rest on the X51v, `rp-gpubench` GC2CP).
pub(crate) fn walk(
    order: crate::TileOrder,
    todo: &Tiles,
    (cols, rows): (u32, u32),
    mut f: impl FnMut(usize, u32, u32) -> Option<()>,
) -> Option<()> {
    use crate::TileOrder as O;
    let n = cols * rows;
    let mut each = |x: u32, y: u32| -> Option<()> {
        let k = (y * cols + x) as usize;
        if todo.has(k) { f(k, x, y) } else { Some(()) }
    };
    match order {
        O::Rows => {
            // The set bits only, a word at a time (a button's few tiles
            // cost a few steps), the row kept as the numbers rise.
            let (mut y, mut start) = (0, 0);
            for (i, &word) in todo.bits.iter().enumerate() {
                let mut w = word;
                while w != 0 {
                    let k = i as u32 * 32 + w.trailing_zeros();
                    w &= w - 1;
                    if k >= n {
                        return Some(());
                    }
                    while k >= start + cols {
                        start += cols;
                        y += 1;
                    }
                    f(k as usize, k - start, y)?;
                }
            }
        }
        O::Reversed => {
            let (mut y, mut start) = (rows - 1, (rows - 1) * cols);
            for (i, &word) in todo.bits.iter().enumerate().rev() {
                let mut w = word;
                while w != 0 {
                    let b = 31 - w.leading_zeros();
                    w &= !(1 << b);
                    let k = i as u32 * 32 + b;
                    if k >= n {
                        continue;
                    }
                    while k < start {
                        start -= cols;
                        y -= 1;
                    }
                    f(k as usize, k - start, y)?;
                }
            }
        }
        O::ColumnsReversed => {
            for y in 0..rows {
                for x in (0..cols).rev() {
                    each(x, y)?;
                }
            }
        }
        O::Columns => {
            for x in (0..cols).rev() {
                for y in (0..rows).rev() {
                    each(x, y)?;
                }
            }
        }
        O::Checker => {
            // From the last tile back, the even squares (x + y even), then
            // the odd ones.
            for pass in 0..2 {
                for y in (0..rows).rev() {
                    let mut x = cols - 1;
                    if (x + y) & 1 != pass {
                        if x == 0 {
                            continue;
                        }
                        x -= 1;
                    }
                    loop {
                        each(x, y)?;
                        if x < 2 {
                            break;
                        }
                        x -= 2;
                    }
                }
            }
        }
        O::Scrambled => {
            // `(n - 1 - i) * SCRAMBLE % n`, stepped down by the step mod n.
            let step = crate::SCRAMBLE as u32 % n;
            let mut v = ((n - 1) as u64 * crate::SCRAMBLE as u64 % u64::from(n)) as u32;
            let r = recip(cols);
            for _ in 0..n {
                let y = div_cols(v, r);
                each(v - y * cols, y)?;
                v = if v >= step { v - step } else { v + n - step };
            }
        }
        O::Interleaved => {
            for y in 0..rows {
                for x in (0..cols).step_by(2).chain((1..cols).step_by(2)) {
                    each(x, y)?;
                }
            }
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reverse_order_is_every_tile_once_descending() {
        // Each end of a word, the words' seams, a middle one, the last.
        let set = [0, 1, 31, 32, 33, 63, 64, 500, 2399];
        let mut t = Tiles::new();
        for k in set {
            t.bits[k / 32] |= 1 << (k % 32);
        }
        assert!(t.iter().eq(set.iter().copied()));
        assert!(t.iter_rev().eq(set.iter().rev().copied()));
        assert_eq!(Tiles::new().iter_rev().count(), 0);
        // Every tile of a full grid, once each, in reverse.
        let mut all = Tiles::new();
        all.add_first(2400);
        assert!(all.iter_rev().eq((0..2400).rev()));
    }

    #[test]
    fn columns_reversed_keeps_rows_top_down_and_reads_each_from_the_right() {
        // A 60 x 40 grid: row 0 from tile 59 down to 0, then row 1 from 119.
        let mut all = Tiles::new();
        all.add_first(2400);
        let order: [usize; 4] = [59, 58, 57, 56];
        assert!(all.iter_cols_rev(60, 40).take(4).eq(order));
        assert_eq!(all.iter_cols_rev(60, 40).nth(60), Some(119));
        assert_eq!(all.iter_cols_rev(60, 40).last(), Some(2340));
        assert_eq!(all.iter_cols_rev(60, 40).count(), 2400);
        // Only the tiles in, each once.
        let mut t = Tiles::new();
        for k in [0, 59, 60, 61, 2399] {
            t.bits[k / 32] |= 1 << (k % 32);
        }
        assert!(t.iter_cols_rev(60, 40).eq([59, 0, 61, 60, 2399]));
    }

    #[test]
    fn pixels_to_tiles() {
        // A 10 x 10 px square at (20, 20): columns 2-3, row 1.
        let r = TileRect::of_pixels(20, 20, 30, 30, 60, 40);
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (2, 1, 4, 2));
        // Edges on tile boundaries take no extra tile.
        let r = TileRect::of_pixels(8, 16, 16, 32, 60, 40);
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (1, 1, 2, 2));
        // Off the grid: clipped, or empty.
        let r = TileRect::of_pixels(-100, -5, 2000, 700, 60, 40);
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (0, 0, 60, 40));
        assert!(TileRect::of_pixels(500, 0, 600, 10, 60, 40).is_empty());
        assert!(TileRect::of_pixels(-20, 0, -1, 10, 60, 40).is_empty());
        assert!(
            TileRect::of_pixels(5, 5, 5, 9, 60, 40).is_empty(),
            "no width"
        );
    }

    #[test]
    fn sets() {
        let mut t = Tiles::new();
        assert!(t.is_empty());
        t.add_rect(TileRect::of_pixels(0, 0, 16, 32, 60, 40), 60);
        assert_eq!(t.iter().collect::<std::vec::Vec<_>>(), [0, 1, 60, 61]);
        t.add(2399);
        t.add(2400);
        assert_eq!(t.count(), 5);
        assert!(t.has(2399) && !t.has(2400) && !t.has(2));
        let mut all = Tiles::new();
        all.add_first(2400);
        assert_eq!(all.count(), 2400);
        let mut some = Tiles::new();
        some.add_first(33);
        assert_eq!(some.count(), 33);
        assert!(some.has(32) && !some.has(33));
        some.add_all(&t);
        assert_eq!(
            some.count(),
            33 + 3,
            "60, 61 and 2399 added; 0 and 1 already in"
        );
        some.clear();
        assert!(some.is_empty());
    }

    #[test]
    fn the_division_free_walk_lists_what_the_old_walks_listed() {
        use crate::{TileOrder as O, order_tile};
        // The old walks (row order and its reverse from the bit iterators,
        // columns reversed from its own, the rest from `order_tile`), each
        // tile then split by division.
        let old =
            |order: O, todo: &Tiles, cols: u32, rows: u32| -> std::vec::Vec<(usize, u32, u32)> {
                let (c, r) = (cols as usize, rows as usize);
                let ks: std::vec::Vec<usize> = match order {
                    O::Rows => todo.iter().collect(),
                    O::Reversed => todo.iter_rev().collect(),
                    O::ColumnsReversed => todo.iter_cols_rev(c, r).collect(),
                    _ => (0..2 * c * r)
                        .filter_map(|i| order_tile(order, i, c, r))
                        .filter(|&k| todo.has(k))
                        .collect(),
                };
                ks.into_iter()
                    .filter(|&k| k < c * r)
                    .map(|k| (k, k as u32 % cols, k as u32 / cols))
                    .collect()
            };
        let orders = [
            O::Rows,
            O::Reversed,
            O::ColumnsReversed,
            O::Columns,
            O::Checker,
            O::Scrambled,
            O::Interleaved,
        ];
        let mut seed = 0x1234_5678u32;
        for (cols, rows) in [
            (60, 40),
            (60, 20),
            (30, 20),
            (40, 60),
            (15, 60),
            (1, 1),
            (7, 3),
            (1, 9),
            (9, 1),
            (2, 2),
        ] {
            for density in [0, 1, 7, 32, 100] {
                let mut todo = Tiles::new();
                for k in 0..(cols * rows) as usize {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    if (seed >> 8) % 100 < density || density == 100 {
                        todo.add(k);
                    }
                }
                for order in orders {
                    let mut got = std::vec::Vec::new();
                    walk(order, &todo, (cols, rows), |k, x, y| {
                        got.push((k, x, y));
                        Some(())
                    });
                    assert_eq!(
                        got,
                        old(order, &todo, cols, rows),
                        "{order:?} {cols} x {rows} at {density}%"
                    );
                }
            }
        }
        // Stopping: `None` from the callback ends the walk there.
        let mut all = Tiles::new();
        all.add_first(2400);
        let mut seen = 0;
        let r = walk(O::Checker, &all, (60, 40), |_, _, _| {
            seen += 1;
            (seen < 5).then_some(())
        });
        assert_eq!((r, seen), (None, 5));
    }

    #[test]
    fn division_by_multiply_is_exact_for_every_grid() {
        for cols in 1..=128u32 {
            let r = recip(cols);
            for n in 0..cols * 128 {
                assert_eq!(div_cols(n, r), n / cols, "{n} / {cols}");
            }
        }
    }
}
