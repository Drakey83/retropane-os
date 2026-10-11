//! A deterministic test pattern: eight vertical color bars with a white
//! one-pixel border. RetroPane images draw it, and `cargo xtask emu` checks the
//! emulator's screenshot against the same function.

/// Bar colors, left to right, as `(r, g, b)` with 8 bits per channel.
pub const BARS: [(u8, u8, u8); 8] = [
    (255, 255, 255), // white
    (255, 255, 0),   // yellow
    (0, 255, 255),   // cyan
    (0, 255, 0),     // green
    (255, 0, 255),   // magenta
    (255, 0, 0),     // red
    (0, 0, 255),     // blue
    (0, 0, 0),       // black
];

/// The pattern's color at `(x, y)` on a `width` × `height` screen.
#[must_use]
pub fn rgb(x: u32, y: u32, width: u32, height: u32) -> (u8, u8, u8) {
    if x == 0 || y == 0 || x + 1 == width || y + 1 == height {
        return (255, 255, 255);
    }
    BARS[(x * 8 / width.max(1)) as usize % 8]
}

/// Packs a color as RGB565.
#[must_use]
pub const fn to_rgb565((r, g, b): (u8, u8, u8)) -> u16 {
    ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)
}

/// The widest screen whose bar row [`fill_rgb565`] builds outside the
/// framebuffer (VGA's long side; 1.25 KiB of stack, well under a page).
pub const ROW_MAX: usize = 640;

/// Fills an RGB565 framebuffer (`stride` bytes per line) with the pattern.
///
/// Matches [`rgb`] pixel for pixel, but builds the bar row once (each bar as a
/// solid span) and copies it down the screen: no per-pixel arithmetic.
/// The row is built in a buffer in ordinary RAM, not in the framebuffer:
/// on the real X51v reading the 2700G's memory is slow (the probe measured
/// the copy-within-the-framebuffer version at 4.2x the time of a plain
/// fill), so the framebuffer is only ever written. Screens wider than
/// [`ROW_MAX`] pixels fall back to copying within the framebuffer.
/// Benchmarked as `fb_pattern_vga` (`cargo xtask bench`).
pub fn fill_rgb565(buf: &mut [u8], width: u32, height: u32, stride: usize) {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 {
        return;
    }
    let white = to_rgb565((255, 255, 255)).to_le_bytes();
    let row_bytes = w * 2;
    let fill = |row: &mut [u8], color: [u8; 2]| {
        for px in row.as_chunks_mut::<2>().0 {
            *px = color;
        }
    };

    let bars = |row: &mut [u8]| {
        for (i, &bar) in BARS.iter().enumerate() {
            // Pixels x with x * 8 / w == i.
            let start = (i * w).div_ceil(8);
            let end = ((i + 1) * w).div_ceil(8).min(w);
            fill(&mut row[start * 2..end * 2], to_rgb565(bar).to_le_bytes());
        }
        row[..2].copy_from_slice(&white);
        row[row_bytes - 2..].copy_from_slice(&white);
    };

    // Rows 1..h-1: bars with white edges.
    if h > 2 {
        if w <= ROW_MAX {
            let mut src = [0u8; ROW_MAX * 2];
            let src = &mut src[..row_bytes];
            bars(src);
            for y in 1..h - 1 {
                buf[y * stride..y * stride + row_bytes].copy_from_slice(src);
            }
        } else {
            bars(&mut buf[stride..stride + row_bytes]);
            for y in 2..h - 1 {
                buf.copy_within(stride..stride + row_bytes, y * stride);
            }
        }
    }
    fill(&mut buf[..row_bytes], white);
    fill(
        &mut buf[(h - 1) * stride..(h - 1) * stride + row_bytes],
        white,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn border_is_white_and_bars_cover_the_width() {
        assert_eq!(rgb(0, 10, 480, 640), (255, 255, 255));
        assert_eq!(rgb(479, 10, 480, 640), (255, 255, 255));
        assert_eq!(rgb(30, 10, 480, 640), BARS[0]);
        assert_eq!(rgb(30 + 60 * 5, 10, 480, 640), BARS[5]);
        assert_eq!(rgb(470, 10, 480, 640), BARS[7]);
    }

    #[test]
    fn rgb565_packing() {
        assert_eq!(to_rgb565((255, 255, 255)), 0xFFFF);
        assert_eq!(to_rgb565((255, 0, 0)), 0xF800);
        assert_eq!(to_rgb565((0, 255, 0)), 0x07E0);
        assert_eq!(to_rgb565((0, 0, 255)), 0x001F);
    }

    #[test]
    fn fast_fill_matches_the_reference_pixel_for_pixel() {
        for &(w, h) in &[
            (480, 640),
            (640, 480),
            (641, 5), // wider than ROW_MAX: the copy-within fallback
            (241, 7),
            (8, 3),
            (9, 4),
            (3, 1),
            (1, 1),
            (17, 2),
        ] {
            let stride = (w as usize * 2).div_ceil(16) * 16;
            let mut buf = vec![0u8; stride * h as usize];
            fill_rgb565(&mut buf, w, h, stride);
            for y in 0..h {
                for x in 0..w {
                    let i = y as usize * stride + x as usize * 2;
                    let got = u16::from_le_bytes([buf[i], buf[i + 1]]);
                    assert_eq!(got, to_rgb565(rgb(x, y, w, h)), "{w}x{h} at ({x},{y})");
                }
            }
        }
    }

    #[test]
    fn fill_respects_stride() {
        let mut buf = vec![0xAAu8; 16 * 3];
        fill_rgb565(&mut buf, 4, 3, 16);
        assert_eq!(&buf[0..2], &0xFFFFu16.to_le_bytes()); // border
        assert_eq!(buf[8], 0xAA); // padding untouched
    }
}
