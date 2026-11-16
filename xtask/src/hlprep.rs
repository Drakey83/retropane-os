//! The PC's preparation of a game's files for the X51v: the first piece,
//! the menu's background made for the screen (docs/reminders.md R275; the
//! owner, 2026-11-15: "do exactly 640 x 480", then "is there no rule 0
//! option?").
//!
//! Iterate, don't imitate (rule 0). What the engine does: at every start
//! it loads the menu background's pieces at the size they were drawn for
//! (Half-Life's anniversary copy: 105 pieces of 256 x 256, a 3840 x 1600
//! picture; its older 800 x 600 set is black in that copy), keeps them all
//! as textures (13.1 MiB in the software renderer, measured: more than
//! the X51v's game heap had left), cuts each colour to 16 bits by dropping
//! its low bits (bands in a dark glow), and stretches every piece onto the
//! screen in every frame.
//!
//! Ours: the work is done once, here, on the PC, where time and memory
//! cost nothing, and the device loads one file made for its screen:
//!
//! - the picture is brought to exactly 640 x 480 with an area average in
//!   linear light (each screen pixel is the true mean of the light of the
//!   part of the picture it covers: averaging the stored values instead
//!   darkens fine detail), keeping its shape as the menu itself places it
//!   (one scale factor, what hangs over cut off equally at both ends);
//! - it is brought to the screen's 16-bit colours by error diffusion, so
//!   a smooth glow stays smooth to the eye instead of stepping in bands;
//! - it is written as an ordinary picture file whose colours are already
//!   exact 16-bit values, so the engine's own loader takes it, its cut to
//!   16 bits changes nothing, and the device needs no code of ours to
//!   scale or decode: about 0.6 MiB as a texture, one read at a start,
//!   drawn one pixel to one.
//!
//! The owner's files are read and never changed; the result goes beside
//! them on the card ([`BACKGROUND`]). It changes the picture (the dither),
//! so it is shown to the owner against the engine's own frame before it
//! is kept (R269).

use std::path::Path;

/// The X51v's screen as a game holds it.
pub const SCREEN: (u32, u32) = (640, 480);
/// Where the prepared background goes, from the game's own folder: the
/// menu looks for it there (ports/xash/patches/mainui.patch).
pub const BACKGROUND: &str = "retropane/background.tga";

/// One piece of a background: its file (from the game's folder) and where
/// its top left corner lies in the picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    pub file: String,
    pub x: u32,
    pub y: u32,
}

/// A background's layout file (`resource/BackgroundLayout.txt`): the
/// picture's size, then each piece as `file fit|scaled x y`. `None` if it
/// is not one.
pub fn parse_layout(text: &str) -> Option<((u32, u32), Vec<Piece>)> {
    let mut words = text
        .lines()
        .map(|l| l.split("//").next().unwrap_or(""))
        .flat_map(str::split_whitespace);
    if words.next()? != "resolution" {
        return None;
    }
    let size = (words.next()?.parse().ok()?, words.next()?.parse().ok()?);
    let mut pieces = Vec::new();
    while let Some(file) = words.next() {
        let _how = words.next()?;
        pieces.push(Piece {
            file: file.to_string(),
            x: words.next()?.parse().ok()?,
            y: words.next()?.parse().ok()?,
        });
    }
    (size.0 > 0 && size.1 > 0 && !pieces.is_empty()).then_some((size, pieces))
}

/// A picture: red, green, blue bytes, rows from the top.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub w: u32,
    pub h: u32,
    pub rgb: Vec<u8>,
}

/// An uncompressed true-colour TGA as the game's background pieces are:
/// 24 or 32 bits a pixel, blue first, rows from the bottom up whatever
/// its header says (the engine loads them so: "Steam background
/// completely ignore tga attribute 0x20"). `None` for anything else.
pub fn decode_piece(tga: &[u8]) -> Option<Picture> {
    if tga.len() < 18 || tga[1] != 0 || tga[2] != 2 {
        return None;
    }
    let (w, h) = (
        u32::from(u16::from_le_bytes([tga[12], tga[13]])),
        u32::from(u16::from_le_bytes([tga[14], tga[15]])),
    );
    let bytes = match tga[16] {
        24 => 3,
        32 => 4,
        _ => return None,
    };
    let from = 18 + tga[0] as usize;
    let pixels = tga.get(from..from + (w * h) as usize * bytes)?;
    if w == 0 || h == 0 {
        return None;
    }
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for row in pixels.chunks_exact(w as usize * bytes).rev() {
        for p in row.chunks_exact(bytes) {
            rgb.extend([p[2], p[1], p[0]]);
        }
    }
    Some(Picture { w, h, rgb })
}

/// `piece` put into `canvas` with its top left corner at (`x`, `y`); what
/// hangs over the canvas's edge is left off (a layout's last pieces do).
pub fn put(canvas: &mut Picture, piece: &Picture, x: u32, y: u32) {
    for py in 0..piece.h.min(canvas.h.saturating_sub(y)) {
        let n = piece.w.min(canvas.w.saturating_sub(x)) as usize * 3;
        let from = (py * piece.w) as usize * 3;
        let to = ((y + py) * canvas.w + x) as usize * 3;
        canvas.rgb[to..to + n].copy_from_slice(&piece.rgb[from..from + n]);
    }
}

/// A stored colour value as light (sRGB's own curve), 0 to 1.
fn light(v: u8) -> f64 {
    let c = f64::from(v) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Light, 0 to 1, as a stored colour value, 0 to 255 (not rounded).
fn stored(l: f64) -> f64 {
    let l = l.clamp(0.0, 1.0);
    255.0
        * if l <= 0.003_130_8 {
            l * 12.92
        } else {
            1.055 * l.powf(1.0 / 2.4) - 0.055
        }
}

/// `src` scaled so that it covers `w` x `h`, centred: one factor for both
/// axes, what hangs over on one cut off equally at both ends. Each target
/// pixel is the mean, in light, of the source area it covers; the result
/// as stored values, three a pixel, not yet rounded.
pub fn cover(src: &Picture, w: u32, h: u32) -> Vec<f64> {
    let (sw, sh, dw, dh) = (
        f64::from(src.w),
        f64::from(src.h),
        f64::from(w),
        f64::from(h),
    );
    let scale = (dw / sw).max(dh / sh);
    // A target pixel's side in source pixels, and where the target starts.
    let step = 1.0 / scale;
    let (x0, y0) = ((sw - dw * step) / 2.0, (sh - dh * step) / 2.0);
    let table: Vec<f64> = (0..=255).map(light).collect();
    let mut out = Vec::with_capacity((w * h * 3) as usize);
    for oy in 0..h {
        let (top, bottom) = (y0 + f64::from(oy) * step, y0 + f64::from(oy + 1) * step);
        for ox in 0..w {
            let (left, right) = (x0 + f64::from(ox) * step, x0 + f64::from(ox + 1) * step);
            let (mut sum, mut area) = ([0.0f64; 3], 0.0);
            let mut y = top.floor().max(0.0) as u32;
            while f64::from(y) < bottom && y < src.h {
                let wy = bottom.min(f64::from(y + 1)) - top.max(f64::from(y));
                let mut x = left.floor().max(0.0) as u32;
                while f64::from(x) < right && x < src.w {
                    let wx = right.min(f64::from(x + 1)) - left.max(f64::from(x));
                    let at = (y * src.w + x) as usize * 3;
                    if wx > 0.0 && wy > 0.0 {
                        for (s, &v) in sum.iter_mut().zip(&src.rgb[at..at + 3]) {
                            *s += wx * wy * table[v as usize];
                        }
                        area += wx * wy;
                    }
                    x += 1;
                }
                y += 1;
            }
            out.extend(sum.map(|s| if area > 0.0 { stored(s / area) } else { 0.0 }));
        }
    }
    out
}

/// A stored value, 0 to 255, as the nearest of a 16-bit colour's levels
/// (`bits` of them: 5 for red and blue, 6 for green), written back as
/// eight bits whose top bits are that level: the engine's cut to 16 bits
/// (`value * levels / 256`) gives the level back unchanged.
fn level(v: f64, bits: u32) -> u8 {
    let top = (1u32 << bits) - 1;
    let n = ((v / 255.0 * f64::from(top))
        .round()
        .clamp(0.0, f64::from(top))) as u32;
    ((n << (8 - bits)) | (n >> (2 * bits - 8))) as u8
}

/// `picture` (stored values, three a pixel, `w` wide) in the screen's
/// 16-bit colours (5, 6 and 5 bits), each as eight bits ([`level`]), by
/// error diffusion (Floyd and Steinberg's weights, each row in the
/// opposite direction to the last): what a pixel loses to its nearest
/// level is handed to its neighbours not yet done, so an area's mean
/// colour is kept and a slow change becomes fine grain, not bands. A
/// pixel that is already a level (black, most of all) is left exact.
pub fn to_screen_colours(picture: &[f64], w: u32) -> Vec<u8> {
    let w = w as usize;
    let h = picture.len() / 3 / w;
    let mut work = picture.to_vec();
    let mut out = vec![0u8; w * h * 3];
    for y in 0..h {
        let forward = y % 2 == 0;
        for step in 0..w {
            let x = if forward { step } else { w - 1 - step };
            for c in 0..3 {
                let at = (y * w + x) * 3 + c;
                let q = level(work[at], if c == 1 { 6 } else { 5 });
                out[at] = q;
                let error = work[at] - f64::from(q);
                let ahead = if forward { x + 1 } else { x.wrapping_sub(1) };
                let behind = if forward { x.wrapping_sub(1) } else { x + 1 };
                let mut give = |x: usize, y: usize, part: f64| {
                    if x < w && y < h {
                        work[(y * w + x) * 3 + c] += error * part / 16.0;
                    }
                };
                give(ahead, y, 7.0);
                give(behind, y + 1, 3.0);
                give(x, y + 1, 5.0);
                give(ahead, y + 1, 1.0);
            }
        }
    }
    out
}

/// `rgb` (`w` x `h`, rows from the top) as a 24-bit TGA file, rows from
/// the bottom up, blue first: the form every loader of the engine's takes.
pub fn encode_tga(w: u32, h: u32, rgb: &[u8]) -> Vec<u8> {
    let mut file = vec![0u8; 18];
    file[2] = 2;
    file[12..14].copy_from_slice(&(w as u16).to_le_bytes());
    file[14..16].copy_from_slice(&(h as u16).to_le_bytes());
    file[16] = 24;
    for row in rgb.chunks_exact(w as usize * 3).rev() {
        for p in row.as_chunks::<3>().0 {
            file.extend([p[2], p[1], p[0]]);
        }
    }
    file
}

/// The menu background of the game in `game` (the folder that holds its
/// `resource`), made for the X51v's screen: the file for [`BACKGROUND`],
/// and which layout it was made from. `None` if the game has no
/// background in pieces. The HD layout first, as the menu itself prefers
/// it for a game that has one.
///
/// # Errors
/// A layout that names a piece that is not there or is not a plain
/// picture.
pub fn background(game: &Path) -> Result<Option<(Vec<u8>, &'static str)>, String> {
    for layout in [
        "resource/HD_BackgroundLayout.txt",
        "resource/BackgroundLayout.txt",
    ] {
        let Ok(text) = std::fs::read_to_string(game.join(layout)) else {
            continue;
        };
        let Some(((w, h), pieces)) = parse_layout(&text) else {
            return Err(format!("{layout}: not a background's layout"));
        };
        let mut canvas = Picture {
            w,
            h,
            rgb: vec![0; (w * h * 3) as usize],
        };
        for piece in &pieces {
            let path = game.join(&piece.file);
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let picture = decode_piece(&bytes).ok_or_else(|| {
                format!("{}: not an uncompressed 24 or 32 bit TGA", path.display())
            })?;
            put(&mut canvas, &picture, piece.x, piece.y);
        }
        // A set that is all black is no picture (the anniversary copy's
        // 800 x 600 one): the next layout, or none.
        if canvas.rgb.iter().all(|&v| v == 0) {
            continue;
        }
        let screen = to_screen_colours(&cover(&canvas, SCREEN.0, SCREEN.1), SCREEN.0);
        return Ok(Some((encode_tga(SCREEN.0, SCREEN.1, &screen), layout)));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: u32, h: u32, v: [u8; 3]) -> Picture {
        Picture {
            w,
            h,
            rgb: v.repeat((w * h) as usize),
        }
    }

    #[test]
    fn a_layout_gives_its_size_and_each_piece_s_place() {
        let text = "resolution\t800\t600\n\nresource/background/800_1_a_loading.tga\t\tfit\t\t0\t0\nresource/background/800_1_b_loading.tga\t\tfit\t\t256\t0 // a note\n\nresource/background/800_2_a_loading.tga scaled 0 256\n";
        let (size, pieces) = parse_layout(text).expect("a layout");
        assert_eq!(size, (800, 600));
        assert_eq!(pieces.len(), 3);
        assert_eq!(
            pieces[1],
            Piece {
                file: "resource/background/800_1_b_loading.tga".into(),
                x: 256,
                y: 0
            }
        );
        assert_eq!((pieces[2].x, pieces[2].y), (0, 256));
        // Not a layout: another first word, no size, a piece cut short, no
        // pieces, a size of nothing.
        assert_eq!(parse_layout("size 800 600\na.tga fit 0 0\n"), None);
        assert_eq!(parse_layout("resolution 800\n"), None);
        assert_eq!(parse_layout("resolution 800 600\na.tga fit 0\n"), None);
        assert_eq!(parse_layout("resolution 800 600\n"), None);
        assert_eq!(parse_layout("resolution 0 600\na.tga fit 0 0\n"), None);
        assert_eq!(parse_layout(""), None);
    }

    /// A TGA of `w` x `h` with `bytes` a pixel: the pixel in file row
    /// `r`, column `c` is blue `r`, green `c`, red 200.
    fn tga(w: u16, h: u16, bytes: u8, attributes: u8) -> Vec<u8> {
        let mut f = vec![0u8; 18];
        f[2] = 2;
        f[12..14].copy_from_slice(&w.to_le_bytes());
        f[14..16].copy_from_slice(&h.to_le_bytes());
        f[16] = bytes * 8;
        f[17] = attributes;
        for r in 0..h {
            for c in 0..w {
                f.extend([r as u8, c as u8, 200]);
                if bytes == 4 {
                    f.push(255);
                }
            }
        }
        f
    }

    #[test]
    fn a_piece_is_read_with_its_last_row_on_top_and_red_first() {
        for (bytes, attributes) in [(3, 0), (4, 8), (3, 0x20)] {
            let p = decode_piece(&tga(3, 2, bytes, attributes)).expect("a piece");
            assert_eq!((p.w, p.h), (3, 2));
            // The file's second row is the picture's top one, whatever
            // the header's flag says: red 200, green the column, blue 1.
            assert_eq!(&p.rgb[..9], [200, 0, 1, 200, 1, 1, 200, 2, 1]);
            assert_eq!(&p.rgb[9..12], [200, 0, 0]);
        }
        // Not a plain picture: packed, with a palette, 16 bits, cut short.
        let good = tga(3, 2, 3, 0);
        for change in [(2usize, 10u8), (1, 1), (16, 16)] {
            let mut bad = good.clone();
            bad[change.0] = change.1;
            assert_eq!(decode_piece(&bad), None, "{change:?}");
        }
        assert_eq!(decode_piece(&good[..good.len() - 1]), None);
        assert_eq!(decode_piece(&good[..10]), None);
        // The file's own notes after the pixels are not pixels.
        let mut longer = good.clone();
        longer.extend([9u8; 26]);
        assert_eq!(decode_piece(&longer), decode_piece(&good));
    }

    #[test]
    fn a_piece_goes_where_the_layout_says_and_its_overhang_is_left_off() {
        let mut canvas = flat(5, 4, [0, 0, 0]);
        put(&mut canvas, &flat(2, 2, [9, 9, 9]), 1, 1);
        put(&mut canvas, &flat(3, 3, [7, 7, 7]), 3, 2);
        let at = |x: usize, y: usize| canvas.rgb[(y * 5 + x) * 3];
        let rows: Vec<Vec<u8>> = (0..4).map(|y| (0..5).map(|x| at(x, y)).collect()).collect();
        assert_eq!(
            rows,
            [
                [0, 0, 0, 0, 0],
                [0, 9, 9, 0, 0],
                [0, 9, 9, 7, 7],
                [0, 0, 0, 7, 7]
            ]
        );
        // Wholly outside: nothing, and no fault.
        put(&mut canvas, &flat(2, 2, [1, 1, 1]), 5, 0);
        put(&mut canvas, &flat(2, 2, [1, 1, 1]), 0, 9);
        assert_eq!(canvas.rgb[4 * 3], 0);
    }

    #[test]
    fn one_colour_stays_that_colour_at_any_size() {
        for (sw, sh) in [(800, 600), (3840, 1600), (64, 64)] {
            let out = cover(&flat(sw, sh, [200, 100, 7]), 64, 48);
            for p in out.as_chunks::<3>().0 {
                assert!((p[0] - 200.0).abs() < 1e-6, "{sw} x {sh}: {p:?}");
                assert!((p[1] - 100.0).abs() < 1e-6);
                assert!((p[2] - 7.0).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn a_smaller_picture_is_the_mean_of_the_light_not_of_the_stored_values() {
        // Black and white side by side, halved: half the light is stored
        // as 188, not as the 128 a mean of the stored values would give.
        let src = Picture {
            w: 2,
            h: 1,
            rgb: vec![0, 0, 0, 255, 255, 255],
        };
        let out = cover(&src, 1, 1);
        assert!((out[0] - 187.5).abs() < 0.5, "{}", out[0]);
        // A pixel covered by one and a quarter source pixels takes each
        // by the area it covers (5 to 4, as 800 to 640): the first is
        // (1 * a + 0.25 * b) / 1.25 in light.
        let row = [0u8, 80, 160, 240, 80];
        let src = Picture {
            w: 5,
            h: 5,
            rgb: (0..5)
                .flat_map(|_| row.iter().flat_map(|&v| [v, v, v]))
                .collect(),
        };
        let out = cover(&src, 4, 4);
        let want = stored((light(0) + 0.25 * light(80)) / 1.25);
        assert!((out[0] - want).abs() < 1e-9);
        let want = stored((0.25 * light(240) + light(80)) / 1.25);
        assert!((out[9] - want).abs() < 1e-9);
    }

    #[test]
    fn the_picture_keeps_its_shape_and_loses_the_same_from_both_ends() {
        // 3840 x 1600 onto 640 x 480: the factor is 0.3 (the heights), so
        // 2133 of the 3840 columns are seen, 853 cut off each side. A
        // picture whose left half is dark and right half light meets in
        // the middle of the screen; its outer sixths are never seen.
        let mut src = flat(3840, 1600, [10, 10, 10]);
        for y in 0..1600usize {
            for x in 0..3840usize {
                let v = if !(800..3040).contains(&x) {
                    250
                } else if x < 1920 {
                    40
                } else {
                    90
                };
                src.rgb[(y * 3840 + x) * 3] = v;
            }
        }
        let out = cover(&src, 640, 480);
        let at = |x: usize, y: usize| out[(y * 640 + x) * 3];
        assert!((at(0, 0) - 40.0).abs() < 1e-6);
        assert!((at(319, 240) - 40.0).abs() < 1e-6);
        assert!((at(320, 240) - 90.0).abs() < 1e-6);
        assert!((at(639, 479) - 90.0).abs() < 1e-6);
        // Not mirrored, not turned: a mark in the source's top left
        // corner of what is seen lands top left.
        let mut src = flat(800, 600, [0, 0, 0]);
        for y in 0..100usize {
            for x in 0..100usize {
                src.rgb[(y * 800 + x) * 3] = 255;
            }
        }
        let out = cover(&src, 640, 480);
        let at = |x: usize, y: usize| out[(y * 640 + x) * 3];
        assert!(at(0, 0) > 254.0 && at(79, 79) > 254.0);
        assert!(at(81, 0) < 1.0 && at(0, 81) < 1.0 && at(639, 479) < 1.0);
    }

    #[test]
    fn the_screen_s_colours_keep_black_exact_levels_and_an_area_s_mean() {
        // The engine's cut to 16 bits gives each level back unchanged.
        for bits in [5u32, 6] {
            for n in 0..(1u32 << bits) {
                let v = f64::from(n) * 255.0 / f64::from((1u32 << bits) - 1);
                let q = level(v, bits);
                assert_eq!(u32::from(q) * (1 << bits) / 256, n, "{bits} bits");
            }
        }
        assert_eq!(
            (level(0.0, 5), level(255.0, 5), level(255.0, 6)),
            (0, 255, 255)
        );
        assert_eq!((level(-9.0, 5), level(999.0, 6)), (0, 255));
        // Black stays black, everywhere: nothing is spread into it.
        let black = to_screen_colours(&vec![0.0; 64 * 8 * 3], 64);
        assert!(black.iter().all(|&v| v == 0));
        // A flat colour between two levels (red 4 is half way from 0 to
        // the first level, 8): half the pixels take each, none another,
        // and the mean is kept.
        let flat: Vec<f64> = [4.0, 4.0, 4.0].repeat(64 * 64);
        let out = to_screen_colours(&flat, 64);
        let reds: Vec<u8> = out.as_chunks::<3>().0.iter().map(|p| p[0]).collect();
        assert!(reds.iter().all(|&v| v == 0 || v == 8), "{:?}", &reds[..8]);
        let mean = reds.iter().map(|&v| f64::from(v)).sum::<f64>() / reds.len() as f64;
        assert!((mean - 4.0).abs() < 0.2, "{mean}");
        // A slow ramp does not step in bands: no run of one level longer
        // than a few pixels where the ramp crosses between two.
        let ramp: Vec<f64> = (0..64)
            .flat_map(|_| (0..256).flat_map(|x| [x as f64 / 8.0; 3]))
            .collect();
        let out = to_screen_colours(&ramp, 256);
        let row: Vec<u8> = out[32 * 256 * 3..33 * 256 * 3]
            .as_chunks::<3>()
            .0
            .iter()
            .map(|p| p[0])
            .collect();
        // Between x = 16 and 48 the true value runs 2 to 6: both 0 and 8
        // appear there, mixed.
        let mid = &row[16..48];
        assert!(mid.contains(&0) && mid.contains(&8));
        let changes = mid.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(changes >= 6, "{changes} changes: {mid:?}");
    }

    #[test]
    fn the_file_is_a_plain_tga_that_reads_back_as_the_picture() {
        let rgb: Vec<u8> = (0..4 * 3 * 3).map(|i| i as u8).collect();
        let file = encode_tga(4, 3, &rgb);
        assert_eq!(file.len(), 18 + 4 * 3 * 3);
        assert_eq!(
            (file[2], file[12], file[14], file[16], file[17]),
            (2, 4, 3, 24, 0)
        );
        // Its first pixel is the picture's bottom left, blue first.
        assert_eq!(&file[18..21], [rgb[26], rgb[25], rgb[24]]);
        let back = decode_piece(&file).expect("a plain picture");
        assert_eq!((back.w, back.h, back.rgb), (4, 3, rgb));
    }

    #[test]
    fn a_game_s_background_is_made_from_its_layout_and_a_black_set_is_no_picture() {
        let dir = std::env::temp_dir().join(format!("rp-hlprep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("resource/background")).expect("folders");
        // No layout: no background, no error.
        assert_eq!(background(&dir), Ok(None));
        // The plain set, all black; an HD set with a picture.
        let piece = |v: u8| {
            let mut f = tga(64, 64, 3, 0);
            for b in &mut f[18..] {
                *b = v;
            }
            f
        };
        std::fs::write(dir.join("resource/background/black.tga"), piece(0)).expect("file");
        std::fs::write(dir.join("resource/background/lit.tga"), piece(90)).expect("file");
        std::fs::write(
            dir.join("resource/BackgroundLayout.txt"),
            "resolution 64 48\nresource/background/black.tga fit 0 0\n",
        )
        .expect("file");
        assert_eq!(background(&dir), Ok(None));
        std::fs::write(
            dir.join("resource/HD_BackgroundLayout.txt"),
            "resolution 128 64\nresource/background/lit.tga fit 0 0\nresource/background/lit.tga fit 64 0\n",
        )
        .expect("file");
        let (file, layout) = background(&dir).expect("made").expect("a picture");
        assert_eq!(layout, "resource/HD_BackgroundLayout.txt");
        let back = decode_piece(&file).expect("a plain picture");
        assert_eq!((back.w, back.h), SCREEN);
        // 90 lies between two levels: every value is one of them, each
        // colour exact for the screen.
        for p in back.rgb.as_chunks::<3>().0 {
            assert!(p[0] == 82 || p[0] == 90, "{p:?}");
            assert_eq!(u32::from(p[1]) * 64 / 256 * 4, u32::from(p[1]) & !3);
        }
        // A piece named and not there is an error, not a hole.
        std::fs::write(
            dir.join("resource/HD_BackgroundLayout.txt"),
            "resolution 128 64\nresource/background/absent.tga fit 0 0\n",
        )
        .expect("file");
        assert!(background(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
