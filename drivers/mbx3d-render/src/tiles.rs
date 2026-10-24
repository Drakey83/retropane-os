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
}
