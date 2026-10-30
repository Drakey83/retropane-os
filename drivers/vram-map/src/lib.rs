//! The 2700G's 16 MiB of local memory (ADR-0027): every fixed region in one
//! map, checked for overlaps and alignment, and a pool allocator for the
//! space left (textures, 3D buffers, video frames).
//!
//! Why one map: the 2D engine and the 3D core can only read this memory
//! (measured: the engine can't reach system RAM), so every surface lives
//! here, and before this crate each image kept its own constants (the old
//! Today screen's video plane and the 2D renderer's image cache both sat at
//! 6 MiB, 2026-10-15). A clash now fails a test instead of corrupting a
//! picture.
//!
//! Pure: no hardware access, no allocation.
//!
//! Watch item (ADR-0027): the pool is shared by textures, 3D buffers, depth
//! and video frames; a 3D game and video at once will press on it, so the
//! graphics service arbitrates (quotas, refusals reported, no silent
//! eviction).

#![no_std]
#![forbid(unsafe_code)]

/// The size of 2700G local memory.
pub const SIZE: u32 = 0x0100_0000;

/// One screen picture, RGB565 (480 x 640 x 2): ADR-0023's screens are
/// 16-bit since the owner's 2026-10-21 vote.
pub const SCREEN_BYTES: u32 = 480 * 2 * 640;

/// One 32-bit picture (480 x 640 x 4): the work picture and the
/// background, where every blend happens at 8 bits a channel.
pub const PICTURE32_BYTES: u32 = 480 * 4 * 640;

/// A fixed region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    /// What it holds.
    pub name: &'static str,
    /// Its first byte.
    pub at: u32,
    /// Its size in bytes.
    pub size: u32,
    /// The alignment its user needs.
    pub align: u32,
}

impl Region {
    /// One past its last byte.
    #[must_use]
    pub const fn end(&self) -> u32 {
        self.at + self.size
    }
}

const fn r(name: &'static str, at: u32, size: u32, align: u32) -> Region {
    Region {
        name,
        at,
        size,
        align,
    }
}

/// The end of the safe region (ADR-0034 amendment P, rules 1-2; quality
/// audit C71): while the 3D core draws, the display reads only below it,
/// or what the core writes comes out wrong (G16-G20, C71's runs: at
/// every clock). 704 KiB, the 2700G5's on-die memory: R23E measured the
/// edge three times on the X51v (gpubench-0226, -0227, -0233: a shown
/// screen ending at `0xB0000` right at 74.75 / 1 and 149.50 / 2, one
/// ending at `0xB0400` wrong at 74.75 / 1), the same edge where render
/// targets turn slower (R23T); raised from G19's `0xAE000` by the owner,
/// 2026-10-30. Ruled for divide 2: at 143.00 / 1 the edge isn't readable
/// yet (R120). The smallest proven until each device's self-test measures
/// its own (amendment P, P3).
pub const SAFE_END: u32 = 0x000B_0000;

/// The loader's and Windows Mobile's display planes, after the screen:
/// the video band inside the safe region (amendment P rule 1, 104 KiB)
/// and the rest of the first MiB. The loader's own picture at `0x60000`
/// is under screen 0 from the moment an image shows its screen.
pub const PLANES: Region = r(
    "video band and loader planes",
    SCREEN_BYTES,
    0x000F_C000 - SCREEN_BYTES,
    4,
);
/// The 3D parameter memory (objects, lists) on die while a scene is open
/// (R116, the owner's pick C, 2026-10-29): the first 704 KiB are faster
/// memory (R23T on the X51v: a target there renders 15% faster; the lists
/// there save ~0.31 ms a render, R114 against R23T). The video band's
/// place inside the safe region, which the display doesn't read while the
/// 3D core draws (ADR-0034 amendment P), so time-shared with [`PLANES`]
/// and not in [`MAP`]: a program showing the video band with a scene open
/// keeps the lists in [`PARAMS`] (`Graphics::lists_on_die(false)`).
pub const LISTS_ON_DIE: Region = r(
    "3D parameter memory on die (the video band's place)",
    SCREEN_BYTES,
    SAFE_END - SCREEN_BYTES,
    0x1000,
);

/// 3D parameter memory manager's 16 KiB block (16 KiB aligned).
/// **Proposed** (ADR-0026/0027): placement rules only partly known.
pub const PM_TABLE: Region = r("3D memory manager block", 0x000F_C000, 0x4000, 0x4000);
/// 3D parameter memory: objects and lists (4 KiB aligned, kept low: the
/// manager's pages misbehaved above 8 MiB, `rp-mbx3d` version 14).
/// **Proposed.**
pub const PARAMS: Region = r("3D parameter memory", 0x0010_0000, 0x0010_0000, 0x1000);
/// ADR-0023's two screen pictures, RGB565 (option L1, 2026-10-21).
/// Screen 0 at `0x0`, inside the safe region ([`SAFE_END`]): the one
/// shown, and drawn by the 3D core, while the 3D core works (ADR-0034
/// amendment P, P1 C; Windows Mobile's place, G17). Screen 1 outside:
/// its job under P1 C is the owner's ruling (docs/design/c71-screen-place.md:
/// R-A drops it, R-B keeps it as the 2D renderer's back picture).
pub const SCREENS: [Region; 2] = [
    r("screen 0", 0, SCREEN_BYTES, 0x1000),
    r("screen 1", 0x0029_6000, SCREEN_BYTES, 0x1000),
];
/// The 2D renderer's work picture, 32-bit: each changed rectangle is
/// composed here, then converted into a screen (ADR-0023 Q1 A).
pub const WORK: Region = r("work picture", 0x0032_C000, PICTURE32_BYTES, 0x1000);
/// The wallpaper, RGB565 (quantised once on the CPU; the engine widens it
/// by repeating the top bits, measured 2026-10-17).
pub const WALLPAPER: Region = r("wallpaper", 0x0045_8000, SCREEN_BYTES, 0x1000);
/// The 2D renderer's row ring (64 rows of 2.5 KiB).
pub const ROWS: Region = r("row ring", 0x004E_E000, 64 * 2560, 4);
/// The 2D renderer's image cache.
pub const IMAGES: Region = r("image cache", 0x0051_6000, 256 * 1024, 4);
/// The glyph atlases.
pub const GLYPHS: Region = r("glyph atlases", 0x0055_6000, 512 * 1024, 4);
/// The background: wallpaper and hoisted glass, 32-bit.
pub const BACKGROUND: Region = r("background", 0x005D_6000, PICTURE32_BYTES, 0x1000);
/// The dither pattern's four rows (32-bit, the work picture's stride
/// apart).
pub const DITHER: Region = r("dither rows", 0x0070_2000, 4 * 480 * 4, 4);
/// Finished areas kept for reuse: screen pixels, RGB565.
pub const CACHE: Region = r("area cache", 0x0070_4000, SCREEN_BYTES, 0x1000);
/// The 2D engine's fence word (its page).
pub const FENCE: Region = r("2D fence", 0x00FF_F000, 0x1000, 4);

/// Every fixed region, in address order.
pub const MAP: [Region; 15] = [
    SCREENS[0], PLANES, PM_TABLE, PARAMS, SCREENS[1], WORK, WALLPAPER, ROWS, IMAGES, GLYPHS,
    BACKGROUND, DITHER, CACHE, POOL, FENCE,
];

/// Screen 0 inside the safe region, the video band after it, and nothing
/// else of the map there (amendment P rules 1 and 5).
const _: () = assert!(SCREENS[0].at == 0 && SCREENS[0].end() <= SAFE_END);
const _: () = assert!(safe_holds_only(&MAP, 2));

/// Whether only the first `n` regions of `map` (in address order) start
/// below [`SAFE_END`].
#[must_use]
pub const fn safe_holds_only(map: &[Region], n: usize) -> bool {
    let mut k = n;
    while k < map.len() {
        if map[k].at < SAFE_END {
            return false;
        }
        k += 1;
    }
    true
}

/// The space handed out at run time (textures, the 3D core's per-tile
/// buffer and region arrays, depth, video frames): from the area cache's
/// end (rounded up to 128 KiB, the 3D per-tile buffer's alignment) to the
/// fence, ~8.4 MiB (6.75 MiB with 32-bit screens).
pub const POOL: Region = r("pool", 0x007A_0000, 0x00FF_F000 - 0x007A_0000, 0x2_0000);

/// The map as it was with 32-bit screens (ADR-0023 before its
/// 2026-10-21 amendment), kept unchanged for the test images measured
/// with it (`rp-gpubench`, `rp-mbx3d-api`, `rp-looks`, the display and
/// bus probes): their device runs stay comparable, byte for byte. The OS
/// (the renderers, the graphics service, the shell) uses the crate's own
/// map. Never both in one image's drawing: the two overlap.
pub mod bench32 {
    use super::{Region, r};

    /// One picture of the screen, XRGB8888 (480 x 640 x 4).
    pub const SCREEN_BYTES: u32 = 480 * 4 * 640;

    /// The loader's and Windows Mobile's display planes (below 1 MiB; the
    /// loader's at `0x60000`). Kept while the loader may still show them.
    pub const PLANES: Region = r("loader and WM planes", 0, 0x000F_C000, 4);
    /// 3D parameter memory manager's 16 KiB block (16 KiB aligned).
    /// **Proposed** (ADR-0026/0027): placement rules only partly known.
    pub const PM_TABLE: Region = r("3D memory manager block", 0x000F_C000, 0x4000, 0x4000);
    /// 3D parameter memory: objects and lists (4 KiB aligned, kept low: the
    /// manager's pages misbehaved above 8 MiB, `rp-mbx3d` version 14).
    /// **Proposed.**
    pub const PARAMS: Region = r("3D parameter memory", 0x0010_0000, 0x0010_0000, 0x1000);
    /// ADR-0023's two 32-bit screen pictures (before the 16-bit amendment).
    pub const SCREENS: [Region; 2] = [
        r("screen 0", 0x0020_0000, SCREEN_BYTES, 0x1000),
        r("screen 1", 0x0034_0000, SCREEN_BYTES, 0x1000),
    ];
    /// The wallpaper, XRGB8888.
    pub const WALLPAPER: Region = r("wallpaper", 0x0048_0000, SCREEN_BYTES, 0x1000);
    /// The 2D renderer's row ring (64 rows of 2.5 KiB).
    pub const ROWS: Region = r("row ring", 0x005C_0000, 64 * 2560, 4);
    /// The 2D renderer's image cache.
    pub const IMAGES: Region = r("image cache", 0x0060_0000, 256 * 1024, 4);
    /// The glyph atlases.
    pub const GLYPHS: Region = r("glyph atlases", 0x0064_0000, 512 * 1024, 4);
    /// The background: wallpaper and hoisted glass.
    pub const BACKGROUND: Region = r("background", 0x006C_0000, SCREEN_BYTES, 0x1000);
    /// The dither pattern's four rows (screen stride apart).
    pub const DITHER: Region = r("dither rows", 0x0080_0000, 4 * 480 * 4, 4);
    /// Finished areas kept for reuse.
    pub const CACHE: Region = r("area cache", 0x0081_0000, SCREEN_BYTES, 0x1000);
    /// The 2D engine's fence word (its page).
    pub const FENCE: Region = r("2D fence", 0x00FF_F000, 0x1000, 4);

    /// Every fixed region, in address order.
    pub const MAP: [Region; 14] = [
        PLANES, PM_TABLE, PARAMS, SCREENS[0], SCREENS[1], WALLPAPER, ROWS, IMAGES, GLYPHS,
        BACKGROUND, DITHER, CACHE, POOL, FENCE,
    ];

    /// The space handed out at run time (textures, the 3D core's per-tile
    /// buffer and region arrays, depth, video frames): from the area cache's
    /// end (rounded up to 128 KiB, the 3D per-tile buffer's alignment) to the
    /// fence, 6.75 MiB.
    pub const POOL: Region = r("pool", 0x0094_0000, 0x00FF_F000 - 0x0094_0000, 0x2_0000);

    const _: () = assert!(super::check(&MAP).is_ok());
}

/// Why a map is wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clash {
    /// The region runs past the end of local memory.
    OutOfRange(&'static str),
    /// The region isn't at its alignment.
    Misaligned(&'static str),
    /// Two regions overlap (or are out of order).
    Overlap(&'static str, &'static str),
}

/// Checks that every region of `map` fits, is aligned, and that they're in
/// address order without overlaps.
pub const fn check(map: &[Region]) -> Result<(), Clash> {
    let mut k = 0;
    while k < map.len() {
        let g = map[k];
        if g.at as u64 + g.size as u64 > SIZE as u64 {
            return Err(Clash::OutOfRange(g.name));
        }
        if g.align == 0 || !g.at.is_multiple_of(g.align) {
            return Err(Clash::Misaligned(g.name));
        }
        if k > 0 && map[k - 1].end() > g.at {
            return Err(Clash::Overlap(map[k - 1].name, g.name));
        }
        k += 1;
    }
    Ok(())
}

/// 2700G local memory as the CPU sees it (the 2D and 3D renderers write
/// it through this, so the same code runs on the device and against a
/// model in tests).
pub trait LocalMem {
    /// Stores `words` from byte offset `at`.
    fn store(&mut self, at: usize, words: &[u32]);
    /// The word at byte offset `at`.
    fn load(&self, at: usize) -> Option<u32>;
    /// Makes every store so far visible to the engine (drains the CPU's
    /// write buffer on the device).
    fn sync(&mut self);
}

// The map is checked when anything using it builds.
const _: () = assert!(check(&MAP).is_ok());

/// Blocks a [`Pool`] can track.
pub const BLOCKS: usize = 32;

/// A first-fit allocator over a region, with no heap: up to [`BLOCKS`]
/// free blocks, in address order, merged when freed next to each other.
#[derive(Clone, Debug)]
pub struct Pool {
    free: [(u32, u32); BLOCKS],
    n: usize,
}

impl Pool {
    /// A pool over `region`, all free.
    #[must_use]
    pub const fn new(region: Region) -> Self {
        let mut free = [(0, 0); BLOCKS];
        free[0] = (region.at, region.size);
        Self { free, n: 1 }
    }

    /// Bytes free in all.
    #[must_use]
    pub fn free_bytes(&self) -> u32 {
        self.free[..self.n].iter().map(|b| b.1).sum()
    }

    /// The largest free block.
    #[must_use]
    pub fn largest(&self) -> u32 {
        self.free[..self.n].iter().map(|b| b.1).max().unwrap_or(0)
    }

    /// `size` bytes at a multiple of `align` (a power of two), or `None`
    /// if no free block fits (or the free list is full).
    pub fn alloc(&mut self, size: u32, align: u32) -> Option<u32> {
        if size == 0 || !align.is_power_of_two() {
            return None;
        }
        for k in 0..self.n {
            let (at, len) = self.free[k];
            let start = at.checked_add(align - 1)? & !(align - 1);
            let pad = start - at;
            if pad.checked_add(size)? > len {
                continue;
            }
            let rest = len - pad - size;
            match (pad > 0, rest > 0) {
                (false, false) => self.remove(k),
                (false, true) => self.free[k] = (start + size, rest),
                (true, false) => self.free[k] = (at, pad),
                (true, true) => {
                    if self.n == BLOCKS {
                        return None;
                    }
                    self.free[k] = (at, pad);
                    self.insert(k + 1, (start + size, rest));
                }
            }
            return Some(start);
        }
        None
    }

    /// Gives back `size` bytes at `at` (as `alloc` returned them). Returns
    /// false, changing nothing, if they overlap free space (a double free)
    /// or the free list is full.
    pub fn free(&mut self, at: u32, size: u32) -> bool {
        let Some(end) = at.checked_add(size) else {
            return false;
        };
        if size == 0 {
            return false;
        }
        let k = self.free[..self.n]
            .iter()
            .position(|b| b.0 >= at)
            .unwrap_or(self.n);
        let before = k.checked_sub(1).map(|j| self.free[j]);
        let after = (k < self.n).then(|| self.free[k]);
        if before.is_some_and(|b| b.0 + b.1 > at) || after.is_some_and(|a| a.0 < end) {
            return false;
        }
        let joins_before = before.is_some_and(|b| b.0 + b.1 == at);
        let joins_after = after.is_some_and(|a| a.0 == end);
        match (joins_before, joins_after) {
            (true, true) => {
                self.free[k - 1].1 += size + self.free[k].1;
                self.remove(k);
            }
            (true, false) => self.free[k - 1].1 += size,
            (false, true) => self.free[k] = (at, size + self.free[k].1),
            (false, false) => {
                if self.n == BLOCKS {
                    return false;
                }
                self.insert(k, (at, size));
            }
        }
        true
    }

    fn remove(&mut self, k: usize) {
        self.free.copy_within(k + 1..self.n, k);
        self.n -= 1;
    }

    fn insert(&mut self, k: usize, b: (u32, u32)) {
        self.free.copy_within(k..self.n, k + 1);
        self.free[k] = b;
        self.n += 1;
    }
}

#[cfg(test)]
mod tests;
