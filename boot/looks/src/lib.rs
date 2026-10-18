//! The C6 looks test's pictures and looks (docs/design/display-calibration.md):
//! shared by the image and the emulator check, which builds its reference
//! pictures from the same functions.
//!
//! Four pages, each a full 480 x 640 screen: the owner's two photos (Pier,
//! Milky Way: packed by `build.rs`), a colour chart (the 24 patches of the
//! classic colour checker at their usual sRGB values, the six full primaries
//! and secondaries, a grey step wedge) and smooth ramps (banding shows here
//! first, where a calibrated table turns a channel down). Five looks: the
//! original as found, the original at full backlight (a fair brightness
//! comparison) and the three measured calibration tables at full backlight.

#![no_std]
#![forbid(unsafe_code)]

use rp_drv_mbx_display::scanout::{GAMMA_ENTRIES, yuv_to_rgb};

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

/// One look: a name, a gamma table (`None`: as found) and a backlight.
#[derive(Clone, Copy, Debug)]
pub struct Look {
    /// Shown in the caption and the run log.
    pub name: &'static str,
    /// The 17-point table, or the one found.
    pub table: Option<[u32; GAMMA_ENTRIES]>,
    /// The backlight.
    pub light: Light,
}

/// The looks, in order (left/right). The three tables are the second pass
/// of `cargo xtask calibrate --verify` on profile run 3 (2026-10-17,
/// `local/screen-measure/c6/profile-1792284643-verify.md`): measured greys
/// within about 1.5 Delta E 2000 of BT.1886 at their white on average. Two
/// blue points are brought down to the next one, as `calibrate` now does
/// (a table must never fall): native point 15 E0 to DE, D65 point 0 6 to 0.
pub const LOOKS: [Look; 5] = [
    Look {
        name: "Original",
        table: None,
        light: Light::Found,
    },
    Look {
        name: "Original, full light",
        table: None,
        light: Light::Full,
    },
    Look {
        name: "Native white",
        table: Some([
            0x0B0006, 0x29281B, 0x3B3D2B, 0x49503A, 0x556146, 0x6A7152, 0x76805E, 0x828C6C,
            0x8A997B, 0xA1A981, 0xA8B796, 0xB7C8A0, 0xC3D3B1, 0xD8E0C0, 0xE5ECCF, 0xF3F6DE,
            0xFFFFDE,
        ]),
        light: Light::Full,
    },
    Look {
        name: "9000 K",
        table: Some([
            0x0B0005, 0x23280B, 0x333D15, 0x435022, 0x51612C, 0x597131, 0x687F3B, 0x708A46,
            0x7A9C53, 0x86A656, 0x91B764, 0xA3C472, 0xA5D376, 0xB3DD7D, 0xC4EB89, 0xC9F799,
            0xD5FFA4,
        ]),
        light: Light::Full,
    },
    Look {
        name: "D65 (6500 K)",
        table: Some([
            0x0C0000, 0x262800, 0x364000, 0x4B4F00, 0x525F03, 0x626E09, 0x6B8012, 0x7C8C18,
            0x849921, 0x91A727, 0x9BB731, 0xA6C53C, 0xAECF47, 0xC1DD4B, 0xC7E959, 0xD6F062,
            0xE3FF76,
        ]),
        light: Light::Full,
    },
];

/// Look `i`'s table, `found` being the table as found.
#[must_use]
pub fn look_table(i: usize, found: &[u32; GAMMA_ENTRIES]) -> [u32; GAMMA_ENTRIES] {
    LOOKS[i % LOOKS.len()].table.unwrap_or(*found)
}

/// The pages, in order (up/down).
pub const PAGES: [&str; 4] = ["Pier", "Milky Way", "Colour chart", "Ramps"];

/// The photos, packed by `build.rs` (LZ4 blocks).
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

/// Pixel `(x, y)` of page `page`, `photos` being the unpacked photos.
#[must_use]
pub fn page_pixel(page: usize, photos: [&[u8]; 2], x: u32, y: u32) -> (u8, u8, u8) {
    match page {
        0 | 1 => photo_pixel(photos[page], x, y),
        2 => chart_pixel(x, y),
        _ => ramp_pixel(x, y),
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
    fn looks_are_the_measured_tables() {
        let found = [0x123456; GAMMA_ENTRIES];
        assert_eq!(look_table(0, &found), found);
        assert_eq!(look_table(1, &found), found);
        assert_eq!(LOOKS[1].light, Light::Full);
        for i in 2..LOOKS.len() {
            let t = look_table(i, &found);
            assert_ne!(t, found);
            // Rising in every channel, the spare byte clear.
            for c in [16, 8, 0] {
                assert!(
                    t.windows(2)
                        .all(|w| (w[0] >> c) & 0xFF <= (w[1] >> c) & 0xFF)
                );
            }
            assert!(t.iter().all(|w| w >> 24 == 0));
        }
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
