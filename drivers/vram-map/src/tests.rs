//! The map and the pool.

use super::*;

#[test]
fn the_map_has_no_clashes() {
    assert_eq!(check(&MAP), Ok(()));
    // The pool runs to the fence.
    assert_eq!(POOL.end(), FENCE.at);
}

#[test]
fn a_clash_is_caught() {
    let a = r("a", 0x1000, 0x2000, 0x1000);
    let b = r("b", 0x2000, 0x1000, 0x1000);
    assert_eq!(check(&[a, b]), Err(Clash::Overlap("a", "b")));
    assert_eq!(check(&[r("c", 0x1001, 4, 4)]), Err(Clash::Misaligned("c")));
    assert_eq!(
        check(&[r("d", SIZE - 4, 8, 4)]),
        Err(Clash::OutOfRange("d"))
    );
    // The old Today screen's video plane (4:2:0, 450 KiB at 6 MiB) against
    // the image cache: the clash this crate exists for.
    let video = r("old video plane", 0x0060_0000, 480 * 640 * 3 / 2, 4);
    assert!(check(&[ROWS, video, GLYPHS]).is_err());
}

#[test]
fn the_3d_buffers_fit_the_rules_found_on_the_device() {
    // Parameter memory below 8 MiB, its block 16 KiB aligned.
    assert!(PARAMS.end() <= 0x0080_0000);
    assert_eq!(PM_TABLE.at % 0x4000, 0);
    // The pool starts 128 KiB aligned: the 3D per-tile buffer's alignment.
    assert_eq!(POOL.at % 0x2_0000, 0);
}

#[test]
fn the_pool_allocates_aligned_and_merges_on_free() {
    let mut p = Pool::new(POOL);
    let total = p.free_bytes();
    let tail = p.alloc(0x1_0000, 0x2_0000).unwrap();
    assert_eq!(tail % 0x2_0000, 0);
    let tex = p.alloc(64 * 64 * 2, 128).unwrap();
    assert_eq!(tex % 128, 0);
    assert!(tex >= tail + 0x1_0000);
    let big = p.alloc(SCREEN_BYTES, 0x1000).unwrap();
    assert_eq!(
        p.free_bytes(),
        total - 0x1_0000 - 64 * 64 * 2 - SCREEN_BYTES
    );
    // Freed in any order, the pool is whole again.
    assert!(p.free(tex, 64 * 64 * 2));
    assert!(p.free(tail, 0x1_0000));
    assert!(p.free(big, SCREEN_BYTES));
    assert_eq!(p.free_bytes(), total);
    assert_eq!(p.largest(), total);
}

#[test]
fn padding_before_an_aligned_block_stays_free() {
    let mut p = Pool::new(r("p", 0x1000, 0x10_0000, 0x1000));
    let a = p.alloc(0x100, 0x100).unwrap();
    assert_eq!(a, 0x1000);
    // Next 64 KiB-aligned block: the gap before it stays usable.
    let b = p.alloc(0x100, 0x1_0000).unwrap();
    assert_eq!(b, 0x1_0000);
    let c = p.alloc(0x100, 4).unwrap();
    assert_eq!(c, 0x1100, "the padding before b was kept free");
}

#[test]
fn double_frees_and_impossible_requests_are_refused() {
    let mut p = Pool::new(r("p", 0, 0x1000, 4));
    let a = p.alloc(0x100, 4).unwrap();
    assert!(p.free(a, 0x100));
    assert!(!p.free(a, 0x100), "double free");
    assert!(
        !p.free(0x800, 0x100),
        "never allocated: overlaps free space"
    );
    assert_eq!(p.alloc(0x2000, 4), None, "too big");
    assert_eq!(p.alloc(0x10, 3), None, "alignment not a power of two");
    assert_eq!(p.alloc(0, 4), None);
}

#[test]
fn many_small_blocks_fill_and_empty_the_pool() {
    let mut p = Pool::new(r("p", 0, 0x10000, 4));
    let mut got = [0u32; 64];
    for g in &mut got {
        *g = p.alloc(0x400, 4).unwrap();
    }
    assert_eq!(p.alloc(4, 4), None);
    // Free every other one, then the rest: fragmentation within BLOCKS.
    for g in got.iter().step_by(2).take(BLOCKS - 1) {
        assert!(p.free(*g, 0x400));
    }
    for (k, g) in got.iter().enumerate() {
        if k % 2 == 1 || k / 2 >= BLOCKS - 1 {
            assert!(p.free(*g, 0x400), "block {k}");
        }
    }
    assert_eq!(p.free_bytes(), 0x10000);
    assert_eq!(p.largest(), 0x10000);
}

#[test]
fn l1_frees_at_least_1_2_mib_and_keeps_the_32_bit_pictures() {
    // ADR-0023 Q2 L1: 16-bit screens, wallpaper and area cache; the work
    // picture and the background stay 32-bit.
    assert_eq!(SCREENS[0].size, 480 * 640 * 2);
    assert_eq!(WALLPAPER.size, SCREEN_BYTES);
    assert_eq!(CACHE.size, SCREEN_BYTES);
    assert_eq!(WORK.size, 480 * 640 * 4);
    assert_eq!(BACKGROUND.size, PICTURE32_BYTES);
    const { assert!(POOL.size >= bench32::POOL.size + 1_200 * 1024) };
    // The parameter memory and its block didn't move.
    assert_eq!((PARAMS, PM_TABLE), (bench32::PARAMS, bench32::PM_TABLE));
}

#[test]
fn the_test_images_map_is_frozen() {
    // The addresses the device runs were measured at (rp-gpubench's
    // pictures, rp-looks' screens, the bus probe's pool): never moved.
    assert_eq!(check(&bench32::MAP), Ok(()));
    let at: [u32; 14] = bench32::MAP.map(|g| g.at);
    assert_eq!(
        at,
        [
            0, 0xF_C000, 0x10_0000, 0x20_0000, 0x34_0000, 0x48_0000, 0x5C_0000, 0x60_0000,
            0x64_0000, 0x6C_0000, 0x80_0000, 0x81_0000, 0x94_0000, 0xFF_F000
        ]
    );
    assert_eq!(bench32::SCREEN_BYTES, 480 * 640 * 4);
}

#[test]
fn screen_0_alone_in_the_safe_region() {
    // C71's class fix (ADR-0034 amendment P): screen 0 at 0x0 inside the
    // safe region, the video band right after, nothing else below its end.
    assert_eq!(SCREENS[0].at, 0);
    assert!(SCREENS[0].end() <= SAFE_END);
    assert_eq!(PLANES.at, SCREENS[0].end());
    assert!(SAFE_END - SCREENS[0].end() >= 96 * 1024, "the video band's room");
    assert!(safe_holds_only(&MAP, 2));
    assert!(MAP[2..].iter().all(|g| g.at >= SAFE_END));
    // The check at its edge: a region starting one word below the end is
    // refused, one at the end accepted.
    let edge = |at| [SCREENS[0], r("x", at, 4, 4)];
    assert!(!safe_holds_only(&edge(SAFE_END - 4), 1));
    assert!(safe_holds_only(&edge(SAFE_END), 1));
    // G19's measured edge: harmless to 0xAE000, wrong from 0xB6000.
    assert_eq!(SAFE_END, 0xAE000);
}
