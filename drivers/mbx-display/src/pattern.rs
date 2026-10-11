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

/// Fills an RGB565 framebuffer (`stride` bytes per line) with the pattern.
pub fn fill_rgb565(buf: &mut [u8], width: u32, height: u32, stride: usize) {
    for y in 0..height {
        let row = &mut buf[y as usize * stride..][..width as usize * 2];
        for (x, px) in row.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            *px = to_rgb565(rgb(x as u32, y, width, height)).to_le_bytes();
        }
    }
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
    fn fill_respects_stride() {
        let mut buf = vec![0xAAu8; 16 * 3];
        fill_rgb565(&mut buf, 4, 3, 16);
        assert_eq!(&buf[0..2], &0xFFFFu16.to_le_bytes()); // border
        assert_eq!(buf[8], 0xAA); // padding untouched
    }
}
