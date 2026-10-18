//! The C6 looks test's pictures and looks (docs/design/display-calibration.md):
//! shared by the image and the emulator check, which builds its reference
//! pictures from the same functions.
//!
//! Version 4, the strength dial: photos (the card's `rp-looks` folder, or
//! the two built in when there is no card), a colour chart (the 24-patch
//! colour checker at its usual sRGB values, the six full primaries and
//! secondaries, a grey step wedge) and smooth ramps, under an S-curve of
//! strength 1.0 (none) to 4.0 through the table as found ([`dial_table`]),
//! or the original. Every look is applied in the picture and dithered
//! ([`shown`]).

#![no_std]
#![forbid(unsafe_code)]

use rp_drv_mbx_display::scanout::{GAMMA_ENTRIES, gamma, yuv_to_rgb};

/// Screen width and height.
pub const W: u32 = 480;
/// Screen height.
pub const H: u32 = 640;
/// The caption strip at the top, shown for a moment after each change.
pub const CAPTION_H: u32 = 44;

/// The backlight a look runs at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Light {
    /// As Windows Mobile left it.
    Found,
    /// Full (432 of 432).
    Full,
}

include!(concat!(env!("OUT_DIR"), "/curves.rs"));

/// Dial positions: strength 1.0 to 4.0 in quarter steps.
pub const STEPS: usize = CURVES.len();
/// Where the dial starts: 2.0, the owner's pick of the strength run.
pub const START: usize = 4;

/// Dial position `s` as hundredths: 100 (1.00) to 400 (4.00).
#[must_use]
pub const fn strength(s: usize) -> u32 {
    100 + 25 * s as u32
}

/// The table for dial position `s`: the curve, then the table as found.
#[must_use]
pub fn dial_table(s: usize, found: &[u32; GAMMA_ENTRIES]) -> [u32; GAMMA_ENTRIES] {
    let c = CURVES[s.min(STEPS - 1)];
    core::array::from_fn(|k| {
        let lane = |shift: u32| u32::from(gamma(found, c[k], shift)) << shift;
        lane(16) | lane(8) | lane(0)
    })
}

/// The measured calibration tables of verify run 1 (second pass), kept for
/// the record: looks run 1 and 2 showed them; the family never picked them.
pub const MEASURED: [(&str, [u32; GAMMA_ENTRIES]); 3] = [
    (
        "Native white",
        [
            0x0B0006, 0x29281B, 0x3B3D2B, 0x49503A, 0x556146, 0x6A7152, 0x76805E, 0x828C6C,
            0x8A997B, 0xA1A981, 0xA8B796, 0xB7C8A0, 0xC3D3B1, 0xD8E0C0, 0xE5ECCF, 0xF3F6DE,
            0xFFFFDE,
        ],
    ),
    (
        "9000 K",
        [
            0x0B0005, 0x23280B, 0x333D15, 0x435022, 0x51612C, 0x597131, 0x687F3B, 0x708A46,
            0x7A9C53, 0x86A656, 0x91B764, 0xA3C472, 0xA5D376, 0xB3DD7D, 0xC4EB89, 0xC9F799,
            0xD5FFA4,
        ],
    ),
    (
        "D65 (6500 K)",
        [
            0x0C0000, 0x262800, 0x364000, 0x4B4F00, 0x525F03, 0x626E09, 0x6B8012, 0x7C8C18,
            0x849921, 0x91A727, 0x9BB731, 0xA6C53C, 0xAECF47, 0xC1DD4B, 0xC7E959, 0xD6F062,
            0xE3FF76,
        ],
    ),
];

/// The hardware table while this test runs: straight (output = input), so
/// what reaches the panel is exactly what [`shown`] stored.
#[must_use]
pub fn straight() -> [u32; GAMMA_ENTRIES] {
    core::array::from_fn(|i| ((16 * i as u32).min(255)) * 0x01_0101)
}

/// The 4 x 4 ordered-dither thresholds (0-15).
const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// What to store for colour `rgb` at `(x, y)` under table `t`: the table
/// applied here, at full precision (linear between points 16 apart, as the
/// 2700G does, without its truncation), then an ordered dither down to the
/// panel's 64 levels a channel. The 18-bit panel keeps the top 6 bits of
/// each byte, so a calibration in the hardware table, applied after any
/// dither, would land on those levels in bands: on this panel the table
/// is applied in the picture and the hardware table is left straight. Each
/// stored byte is its level x 4 + 2, the middle of the level, so the
/// straight table's rounding in its top segment can't move it.
#[must_use]
pub fn shown(t: &[u32; GAMMA_ENTRIES], (r, g, b): (u8, u8, u8), x: u32, y: u32) -> (u8, u8, u8) {
    // The threshold in 64ths of a level: 2, 6 ... 62.
    let d = u32::from(BAYER[(y % 4) as usize][(x % 4) as usize]) * 4 + 2;
    let lane = |v: u8, shift: u32| -> u8 {
        let at = |i: usize| (t[i] >> shift) & 0xFF;
        let (i, f) = (usize::from(v >> 4), u32::from(v & 15));
        // Output in 16ths of a byte: a level is 64 of them.
        let out = at(i) * 16 + f * at(i + 1) - f * at(i);
        let level = ((out + d) / 64).min(63);
        (level * 4 + 2) as u8
    };
    (lane(r, 16), lane(g, 8), lane(b, 0))
}

/// What a page shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    /// A photo (unpacked 4:2:0).
    Photo,
    /// The colour chart.
    Chart,
    /// The ramps.
    Ramps,
}

/// The built-in photos' names (with no card).
pub const BUILT_IN: [&str; 2] = ["Pier", "Milky Way"];

/// The built-in photos, packed by `build.rs` (LZ4 blocks).
pub const PHOTOS: [&[u8]; 2] = [
    include_bytes!(concat!(env!("OUT_DIR"), "/pier.lz4")),
    include_bytes!(concat!(env!("OUT_DIR"), "/milkyway.lz4")),
];

/// A photo's unpacked size: planar 4:2:0.
pub const YUV_BYTES: usize = (W * H * 3 / 2) as usize;

/// Why a packed photo didn't unpack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unpack {
    /// The block table runs past the data.
    Header,
    /// A block didn't decode.
    Block,
    /// The blocks didn't fill exactly `out`.
    Size,
}

/// Unpacks a photo (`build.rs`'s format) into `out` (at least [`YUV_BYTES`]).
pub fn unpack(packed: &[u8], out: &mut [u8]) -> Result<(), Unpack> {
    let word = |i: usize| -> Result<usize, Unpack> {
        let b = packed.get(i * 4..i * 4 + 4).ok_or(Unpack::Header)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    };
    let n = word(0)?;
    let mut at = 4 * (1 + n);
    let mut o = 0;
    for b in 0..n {
        let size = word(1 + b)?;
        let src = packed.get(at..at + size).ok_or(Unpack::Header)?;
        let dst = out.get_mut(o..YUV_BYTES).ok_or(Unpack::Size)?;
        o += rp_lz4::decompress(src, dst).map_err(|_| Unpack::Block)?;
        at += size;
    }
    if o == YUV_BYTES && at == packed.len() {
        Ok(())
    } else {
        Err(Unpack::Size)
    }
}

/// Pixel `(x, y)` of an unpacked photo.
#[must_use]
pub fn photo_pixel(yuv: &[u8], x: u32, y: u32) -> (u8, u8, u8) {
    let (x, y) = (x as usize, y as usize);
    let (w, h) = (W as usize, H as usize);
    let c = (y / 2) * (w / 2) + x / 2;
    yuv_to_rgb(yuv[y * w + x], yuv[w * h + c], yuv[w * h + w * h / 4 + c])
}

/// The classic 24-patch colour checker, its usual sRGB values (D65): skin,
/// sky, foliage, flowers, saturated colours and a six-step grey row.
pub const CHECKER: [(u8, u8, u8); 24] = [
    (115, 82, 68),
    (194, 150, 130),
    (98, 122, 157),
    (87, 108, 67),
    (133, 128, 177),
    (103, 189, 170),
    (214, 126, 44),
    (80, 91, 166),
    (193, 90, 99),
    (94, 60, 108),
    (157, 188, 64),
    (224, 163, 46),
    (56, 61, 150),
    (70, 148, 73),
    (175, 54, 60),
    (231, 199, 31),
    (187, 86, 149),
    (8, 133, 161),
    (243, 243, 242),
    (200, 200, 200),
    (160, 160, 160),
    (122, 122, 121),
    (85, 85, 85),
    (52, 52, 52),
];

/// The chart page: the checker (6 x 4 patches of 80 x 100, 6-pixel black
/// gaps) from the top, then full red, green, blue, cyan, magenta, yellow
/// (80 x 100), then a 16-step grey wedge (30 x 140) at the bottom.
#[must_use]
pub fn chart_pixel(x: u32, y: u32) -> (u8, u8, u8) {
    const GAP: u32 = 6;
    // Inside an 80 x 100 patch, clear of its half of the gap.
    let inside = |cx: u32, cy: u32| {
        (GAP / 2..80 - GAP / 2).contains(&cx) && (GAP / 2..100 - GAP / 2).contains(&cy)
    };
    let (col, cx) = (x / 80, x % 80);
    if y < 400 {
        let (row, cy) = (y / 100, y % 100);
        if !inside(cx, cy) {
            return (0, 0, 0);
        }
        CHECKER[(row * 6 + col) as usize]
    } else if y < 500 {
        let cy = y - 400;
        if !inside(cx, cy) {
            return (0, 0, 0);
        }
        [
            (255, 0, 0),
            (0, 255, 0),
            (0, 0, 255),
            (0, 255, 255),
            (255, 0, 255),
            (255, 255, 0),
        ][col as usize]
    } else {
        let v = (x / 30).min(15) as u8 * 17;
        (v, v, v)
    }
}

/// The ramps page: eight bands of 80 rows, 0 at the left to 255 at the
/// right: grey, red, green, blue, cyan, magenta, yellow, then grey 0-63
/// stretched across the width (shadow detail).
#[must_use]
pub fn ramp_pixel(x: u32, y: u32) -> (u8, u8, u8) {
    let v = (x * 255 / (W - 1)) as u8;
    match y / 80 {
        0 => (v, v, v),
        1 => (v, 0, 0),
        2 => (0, v, 0),
        3 => (0, 0, v),
        4 => (0, v, v),
        5 => (v, 0, v),
        6 => (v, v, 0),
        _ => {
            let d = (x * 63 / (W - 1)) as u8;
            (d, d, d)
        }
    }
}

/// Pixel `(x, y)` of a page, `photo` being the unpacked photo it shows.
#[must_use]
pub fn page_pixel(page: Page, photo: &[u8], x: u32, y: u32) -> (u8, u8, u8) {
    match page {
        Page::Photo => photo_pixel(photo, x, y),
        Page::Chart => chart_pixel(x, y),
        Page::Ramps => ramp_pixel(x, y),
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec;

    #[test]
    fn both_photos_unpack_whole() {
        for p in PHOTOS {
            let mut out = vec![0u8; YUV_BYTES];
            assert_eq!(unpack(p, &mut out), Ok(()));
        }
    }

    #[test]
    fn photos_fit_a_loaded_image() {
        let packed: usize = PHOTOS.iter().map(|p| p.len()).sum();
        // The loader takes images up to 0xE8000 bytes (928 KiB): leave room
        // for the code.
        assert!(
            packed < 0xE_8000 - 64 * 1024,
            "photos packed to {packed} bytes"
        );
    }

    #[test]
    fn damaged_photos_are_refused() {
        let mut out = vec![0u8; YUV_BYTES];
        let p = PHOTOS[0];
        assert_eq!(unpack(&p[..p.len() - 1], &mut out), Err(Unpack::Header));
        assert_eq!(unpack(&p[..3], &mut out), Err(Unpack::Header));
        let mut short = vec![0u8; YUV_BYTES - 1];
        assert!(unpack(p, &mut short).is_err());
    }

    #[test]
    fn the_dial_runs_from_no_curve_to_strength_four() {
        assert_eq!(STEPS, 13);
        assert_eq!(strength(0), 100);
        assert_eq!(strength(START), 200);
        assert_eq!(strength(STEPS - 1), 400);
        let ramp: [u32; GAMMA_ENTRIES] =
            core::array::from_fn(|i| ((16 * i as u32).min(255)) * 0x01_0101);
        // 1.0 is no curve: the found table itself (point 16 is the found
        // table at 255: a straight 17-point table's squeezed 254).
        assert_eq!(dial_table(0, &ramp)[..16], ramp[..16]);
        assert_eq!(dial_table(0, &ramp)[16], 254 * 0x01_0101);
        // 2.0 is run 3's "strong", which the owner picked.
        assert_eq!(
            CURVES[START],
            [
                0, 2, 8, 18, 32, 50, 72, 98, 128, 158, 184, 206, 224, 238, 247, 253, 255
            ]
        );
        let mut prev_dark = u32::MAX;
        for s in 0..STEPS {
            let t = dial_table(s, &ramp);
            for c in [16, 8, 0] {
                assert!(
                    t.windows(2)
                        .all(|w| (w[0] >> c) & 0xFF <= (w[1] >> c) & 0xFF)
                );
            }
            assert_eq!(t[0] & 0xFF, 0);
            // The middle stays (input 128 is just above the exact middle).
            assert!((128..=130).contains(&(t[8] & 0xFF)), "{}", t[8] & 0xFF);
            assert!(t.iter().all(|w| w >> 24 == 0));
            // Stronger is darker in the shadows (or as dark).
            assert!(t[3] & 0xFF <= prev_dark);
            prev_dark = t[3] & 0xFF;
        }
        // The kept measured tables never fall.
        for (_, t) in MEASURED {
            for c in [16, 8, 0] {
                assert!(
                    t.windows(2)
                        .all(|w| (w[0] >> c) & 0xFF <= (w[1] >> c) & 0xFF)
                );
            }
        }
    }

    #[test]
    fn stored_levels_reach_the_panel_unchanged() {
        use rp_drv_mbx_display::scanout::gamma;
        let st = straight();
        for level in 0..64u32 {
            let b = (level * 4 + 2) as u8;
            assert_eq!(u32::from(gamma(&st, b, 0)) >> 2, level, "level {level}");
        }
    }

    #[test]
    fn dither_keeps_the_mean_and_exact_levels() {
        let st = straight();
        // A colour exactly on a level stays on it everywhere (below 240:
        // the 17-point table squeezes 240-255 into 240-254 by itself).
        for x in 0..4 {
            for y in 0..4 {
                assert_eq!(shown(&st, (128, 0, 236), x, y), (130, 2, 238));
            }
        }
        // Between levels the 4 x 4 cell averages to the colour: 129 is a
        // quarter of the way from level 32 to 33.
        let mean: u32 = (0..16)
            .map(|k| u32::from(shown(&st, (129, 0, 0), k % 4, k / 4).0) >> 2)
            .sum();
        assert_eq!(mean, 32 * 16 + 4);
        // White stays white; black stays black.
        assert_eq!(shown(&st, (255, 255, 255), 3, 3), (254, 254, 254));
        assert_eq!(shown(&st, (0, 0, 0), 3, 3), (2, 2, 2));
    }

    #[test]
    fn a_table_is_applied_in_the_picture() {
        // A halving table: 200 lands near level 25 (100 / 4) on average.
        let half: [u32; GAMMA_ENTRIES] =
            core::array::from_fn(|i| ((16 * i as u32).min(255) / 2) * 0x01_0101);
        let mean: u32 = (0..16)
            .map(|k| u32::from(shown(&half, (200, 200, 200), k % 4, k / 4).1) >> 2)
            .sum();
        assert_eq!(mean, 25 * 16);
    }

    #[test]
    fn chart_and_ramps_land_where_described() {
        assert_eq!(chart_pixel(40, 50), CHECKER[0]);
        assert_eq!(chart_pixel(440, 350), CHECKER[23]);
        assert_eq!(chart_pixel(0, 0), (0, 0, 0));
        assert_eq!(chart_pixel(200, 450), (0, 0, 255));
        assert_eq!(chart_pixel(479, 600), (255, 255, 255));
        assert_eq!(ramp_pixel(0, 0), (0, 0, 0));
        assert_eq!(ramp_pixel(W - 1, 0), (255, 255, 255));
        assert_eq!(ramp_pixel(W - 1, 250), (0, 0, 255));
        assert_eq!(ramp_pixel(W - 1, 639), (63, 63, 63));
    }
}
