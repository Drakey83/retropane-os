//! A scrolling text console drawn into an RGB565 framebuffer region, using
//! the 5×7 [`font`](crate::font) at an integer scale. No allocation: it draws
//! straight into the framebuffer and scrolls with `memcpy`.

use crate::fill_rect_rgb565;
use crate::font::{HEIGHT, WIDTH, glyph};

/// Where bring-up images put their boot log on the 480 × 640 screen: below
/// the color bars, inside the pattern's border. `(x, y, width, height)`.
/// `cargo xtask emu` checks the pattern outside this box and text inside it.
pub const BOOT_LOG: (usize, usize, usize, usize) = (8, 160, 464, 472);
/// Boot log glyph scale (12 × 16 pixel cells: 38 columns × 29 rows).
pub const BOOT_LOG_SCALE: usize = 2;
/// Boot log colors (RGB565): light grey on dark blue.
pub const BOOT_LOG_COLORS: (u16, u16) = (0xE73C, 0x0886);

/// A text console over part of a framebuffer.
pub struct Console<'a> {
    fb: &'a mut [u8],
    stride: usize,
    x0: usize,
    y0: usize,
    cols: usize,
    rows: usize,
    scale: usize,
    col: usize,
    row: usize,
    fg: u16,
    bg: u16,
}

impl<'a> Console<'a> {
    /// A console over the `w` × `h` pixel region at `(x, y)` of `fb` (RGB565,
    /// `stride` bytes per line), with glyphs scaled by `scale`. Clears the
    /// region to `bg`.
    pub fn new(
        fb: &'a mut [u8],
        stride: usize,
        (x, y): (usize, usize),
        (w, h): (usize, usize),
        scale: usize,
        (fg, bg): (u16, u16),
    ) -> Self {
        let scale = scale.max(1);
        fill_rect_rgb565(fb, stride, x, y, w, h, bg);
        Self {
            fb,
            stride,
            x0: x,
            y0: y,
            cols: w / ((WIDTH + 1) * scale),
            rows: h / ((HEIGHT + 1) * scale),
            scale,
            col: 0,
            row: 0,
            fg,
            bg,
        }
    }

    /// Changes the text color for what is written next.
    pub fn set_fg(&mut self, fg: u16) {
        self.fg = fg;
    }

    /// Columns and rows of text.
    #[must_use]
    pub fn size(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }

    fn cell(&self) -> (usize, usize) {
        ((WIDTH + 1) * self.scale, (HEIGHT + 1) * self.scale)
    }

    /// Writes one character, handling `\n`, `\r` and wrapping.
    pub fn put(&mut self, c: char) {
        if self.cols == 0 || self.rows == 0 {
            return;
        }
        match c {
            '\n' => self.newline(),
            '\r' => self.col = 0,
            c => {
                if self.col == self.cols {
                    self.newline();
                }
                self.draw(c);
                self.col += 1;
            }
        }
    }

    fn draw(&mut self, c: char) {
        let (cw, ch) = self.cell();
        let px = self.x0 + self.col * cw;
        let py = self.y0 + self.row * ch;
        fill_rect_rgb565(self.fb, self.stride, px, py, cw, ch, self.bg);
        let (fg, s, stride) = (self.fg, self.scale, self.stride);
        for (gy, &bits) in glyph(c).iter().enumerate() {
            for gx in 0..WIDTH {
                if bits & (1 << (WIDTH - 1 - gx)) != 0 {
                    fill_rect_rgb565(self.fb, stride, px + gx * s, py + gy * s, s, s, fg);
                }
            }
        }
    }

    fn newline(&mut self) {
        self.col = 0;
        if self.row + 1 < self.rows {
            self.row += 1;
            return;
        }
        // Scroll the text area up one line with memcpy, then clear the last line.
        let (_, ch) = self.cell();
        let width_bytes = self.cols * self.cell().0 * 2;
        let text_h = self.rows * ch;
        for y in self.y0..self.y0 + text_h - ch {
            let src = (y + ch) * self.stride + self.x0 * 2;
            self.fb
                .copy_within(src..src + width_bytes, y * self.stride + self.x0 * 2);
        }
        let last = self.y0 + text_h - ch;
        fill_rect_rgb565(
            self.fb,
            self.stride,
            self.x0,
            last,
            self.cols * self.cell().0,
            ch,
            self.bg,
        );
    }
}

impl core::fmt::Write for Console<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for c in s.chars() {
            self.put(c);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::fmt::Write;

    const W: usize = 48; // 4 columns at scale 2 (12 px each)
    const H: usize = 32; // 2 rows at scale 2 (16 px each)
    const STRIDE: usize = W * 2;

    fn px(fb: &[u8], x: usize, y: usize) -> u16 {
        u16::from_le_bytes([fb[y * STRIDE + x * 2], fb[y * STRIDE + x * 2 + 1]])
    }

    #[test]
    fn draws_glyph_pixels_scaled() {
        let mut fb = vec![0u8; STRIDE * H];
        let mut c = Console::new(&mut fb, STRIDE, (0, 0), (W, H), 2, (0xFFFF, 0x0000));
        assert_eq!(c.size(), (4, 2));
        c.put('A');
        // 'A' row 0 is .###. -> pixel x=1 (scaled: 2..4) set, x=0 clear.
        assert_eq!(px(&fb, 0, 0), 0);
        assert_eq!(px(&fb, 2, 0), 0xFFFF);
        assert_eq!(px(&fb, 3, 1), 0xFFFF);
        // Row 3 is ##### -> every glyph column set.
        assert_eq!(px(&fb, 0, 6), 0xFFFF);
        assert_eq!(px(&fb, 9, 7), 0xFFFF);
        // The spacing column (x = 10..12) stays background.
        assert_eq!(px(&fb, 10, 6), 0);
    }

    #[test]
    fn wraps_and_scrolls() {
        let mut fb = vec![0u8; STRIDE * H];
        let mut c = Console::new(&mut fb, STRIDE, (0, 0), (W, H), 2, (0xFFFF, 0x0000));
        write!(c, "----").unwrap(); // fills row 0
        write!(c, "|").unwrap(); // wraps to row 1
        write!(c, "\nZ").unwrap(); // scrolls: '|' moves to row 0, 'Z' on row 1
        // '|' is ..#.. : column x=2 -> scaled pixels 4..6, all glyph rows.
        assert_eq!(px(&fb, 4, 0), 0xFFFF);
        assert_eq!(px(&fb, 4, 12), 0xFFFF);
        // The dashes are gone (row 3 of '-' was solid at y = 6..8).
        assert_eq!(px(&fb, 0, 6), 0);
        // 'Z' row 0 is ##### on the second text line (y = 16).
        assert_eq!(px(&fb, 0, 16), 0xFFFF);
    }

    #[test]
    fn region_offset_and_background() {
        let mut fb = vec![0x55u8; STRIDE * H];
        let size = Console::new(&mut fb, STRIDE, (12, 16), (24, 16), 2, (0xFFFF, 0x1234)).size();
        assert_eq!(size, (2, 1));
        assert_eq!(px(&fb, 12, 16), 0x1234);
        assert_eq!(px(&fb, 0, 0), 0x5555); // outside the region untouched
    }
}
