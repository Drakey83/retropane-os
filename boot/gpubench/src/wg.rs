//! Run WG's shared part (the Weather app's movement on the planes, sheet
//! `docs/research/sheets/weather-gpu-movement.md`, ADR-0034 C4 as changed
//! 2026-10-30, "L2"), used by the image and its host test: the pictures
//! (screen 0's wallpaper and chrome, the page layer's glass rows, the turn's
//! two-page strip), the scripted fling and rebound, the Reveal and Cascade
//! entrances as the video window's place, size and start, and the 2D
//! engine's "over" as measured. The host test holds every window state to
//! the picture it reads and composes the layouts through the scan-out
//! model (`rp_drv_mbx_display::scanout`).

use rp_drv_mbx_display::font::{self, WIDTH};

/// The screen.
pub const W: u32 = 480;
pub const H: u32 = 640;
/// The page's window (the Weather app's page area under the header and
/// tabs, over the softkeys): `VSADR` y and `VSCTRL` height.
pub const WIN_Y: u32 = 94;
pub const WIN_H: u32 = 462;
/// The transparent lines above and below the list (`motion.rs`'s
/// `BAND_MAX_PX`): the rebound shows the still wallpaper through them.
pub const MARGIN: u32 = 120;
/// The Hourly list: 24 rows.
pub const ROWS: u32 = 24;
pub const ROW_PITCH: u32 = 45;
/// The list's height (the sheet's 1,092 lines; the rows take 1,080, the
/// last 12 clear).
pub const LIST_H: u32 = 1092;
/// The tall picture: 480 x 1,332 (2.4 MiB at ARGB8888).
pub const PIC_H: u32 = LIST_H + 2 * MARGIN;
/// The furthest scroll with the window full of rows.
pub const SCROLL_MAX: i32 = (LIST_H - WIN_H) as i32;
/// The scripted fling's frames (M2: 300).
pub const FLING_FRAMES: usize = 300;
/// The turn's strip: the shown page and its neighbour side by side,
/// 960 x 462 (stride 3,840 at ARGB8888).
pub const STRIP_W: u32 = 2 * W;
/// An entrance's frames (the mock-ups' 18).
pub const ENTRANCE_FRAMES: u32 = 18;
/// Screen 0's chrome: the header (title and tabs) above the window, the
/// softkeys below it.
pub const SOFTKEYS_Y: u32 = WIN_Y + WIN_H;

/// Whether pixel `(x, y)` is lit in `s` drawn at `(x0, y0)` in the 5 x 7
/// font, `scale` times.
#[must_use]
pub fn in_text(x: u32, y: u32, s: &str, x0: u32, y0: u32, scale: u32) -> bool {
    let cell = (WIDTH as u32 + 1) * scale;
    if x < x0 || y < y0 || y >= y0 + 7 * scale {
        return false;
    }
    let k = ((x - x0) / cell) as usize;
    let Some(ch) = s.chars().nth(k) else {
        return false;
    };
    let gx = (x - x0) % cell / scale;
    let gy = ((y - y0) / scale) as usize;
    gx < WIDTH as u32 && font::glyph(ch)[gy] & (1 << (WIDTH as u32 - 1 - gx)) != 0
}

/// `0x00RRGGBB` as RGB565, each channel's top bits.
#[must_use]
pub const fn to565(c: u32) -> u16 {
    ((c >> 8) & 0xF800 | (c >> 5) & 0x07E0 | (c >> 3) & 0x001F) as u16
}

/// An RGB565 pixel as the 2D engine reads it: each channel widened by
/// repeating its top bits (`rp-mbx-render` `model::widen`, measured
/// 2026-10-17), alpha opaque.
#[must_use]
pub const fn widen(p: u16) -> u32 {
    let p = p as u32;
    let (r, g, b) = (p >> 11, (p >> 5) & 0x3F, p & 0x1F);
    0xFF00_0000 | (r << 3 | r >> 2) << 16 | (g << 2 | g >> 4) << 8 | (b << 3 | b >> 2)
}

/// The 2D engine's "over" (blend word `0x09100000`, the renderer's) of
/// `s` onto `d`, 8-bit channels, as measured (`rp-mbx-render`
/// `model::blend`: A weighed `a + (a >> 7)` of 256, saturating); the
/// result opaque.
#[must_use]
pub fn over(s: u32, d: u32) -> u32 {
    let a = s >> 24;
    let k = a + (a >> 7);
    let ch = |sh: u32| {
        let v = (((s >> sh) & 0xFF) * k + ((d >> sh) & 0xFF) * (256 - k)) >> 8;
        if v > 255 { 255 } else { v }
    };
    0xFF00_0000 | ch(16) << 16 | ch(8) << 8 | ch(0)
}

/// The still wallpaper behind the page: a dusk sky, darker at the top,
/// with a diagonal haze band and a sun disc, so a picture shown a line or
/// a pixel off reads differently (`0x00RRGGBB`).
#[must_use]
pub fn wallpaper(x: u32, y: u32) -> u32 {
    let (dx, dy) = (x as i32 - 360, y as i32 - 470);
    if dx * dx + dy * dy < 46 * 46 {
        return 0x00_FF_C8_70;
    }
    let band = (x + y) % 160;
    let haze = 24u32.saturating_sub(band);
    let r = 30 + y * 120 / H + haze;
    let g = 40 + y * 70 / H + x / 24 + haze;
    let b = 110 + y * 60 / H + haze;
    r.min(255) << 16 | g.min(255) << 8 | b.min(255)
}

/// Screen 0 (RGB565): the header with the step's two label lines, the
/// tabs and their underline, the softkeys, the wallpaper between.
#[must_use]
pub fn screen0(x: u32, y: u32, l1: &str, l2: &str) -> u16 {
    let c = if y < WIN_Y {
        if in_text(x, y, l1, 8, 8, 3) {
            0x00_FF_FF_FF
        } else if in_text(x, y, l2, 8, 40, 2) {
            0x00_FF_E0_60
        } else if (64..92).contains(&y) && in_text(x, y, "NOW  HOURLY  10 DAY  RADAR", 8, 70, 2) {
            0x00_C0_D0_E0
        } else if y >= 90 && (84..168).contains(&x) {
            0x00_40_A0_FF
        } else {
            0x00_10_18_24
        }
    } else if y >= SOFTKEYS_Y {
        if in_text(x, y, "MENU", 24, 590, 3) || in_text(x, y, "BACK", 386, 590, 3) {
            0x00_FF_FF_FF
        } else {
            0x00_10_18_24
        }
    } else {
        wallpaper(x, y)
    };
    to565(c)
}

/// The page layer's pixel (ARGB8888) at `(x, y)` of the tall picture:
/// clear margins and gaps, each row a dark glass card (alpha 0xC0, the
/// denser glass) with a soft one-pixel edge (alpha 0x60), its hour, a sun
/// and a temperature in opaque colour. `second`: the neighbour page of a
/// turn ("DAY" rows in a cooler glass).
#[must_use]
pub fn page(x: u32, y: u32, second: bool) -> u32 {
    if !(MARGIN..MARGIN + ROWS * ROW_PITCH).contains(&y) {
        return 0;
    }
    let row = (y - MARGIN) / ROW_PITCH;
    let ry = (y - MARGIN) % ROW_PITCH;
    if !(2..43).contains(&ry) || !(8..472).contains(&x) {
        return 0;
    }
    if ry == 2 || ry == 42 || x == 8 || x == 471 {
        return 0x60_A0_B0_C0;
    }
    let top = y - ry;
    let (name, temp) = if second {
        (["DAY 01", "DAY 02", "DAY 03"][(row % 3) as usize], 40 + row)
    } else {
        ("HOUR", 50 + row)
    };
    let mut t = [0u8; 4];
    let tl = digits(temp, &mut t);
    if in_text(x, y, name, 24, top + 12, 3) {
        return 0xFF_FF_FF_FF;
    }
    if !second {
        let mut n = [0u8; 4];
        if in_text(x, y, digits(row, &mut n), 120, top + 12, 3) {
            return 0xFF_FF_FF_FF;
        }
    }
    if in_text(x, y, tl, 380, top + 12, 3) {
        return 0xFF_FF_E0_A0;
    }
    if (290..320).contains(&x) && (10..36).contains(&ry) {
        return 0xFF_FF_D0_40;
    }
    let shade = ry * 2;
    if second {
        0xC0_14_30_40 + shade
    } else {
        0xC0_18_24_34 + (shade << 8)
    }
}

/// `v` in decimal (at most four digits) in `buf`.
fn digits(v: u32, buf: &mut [u8; 4]) -> &str {
    let mut n = 0;
    let mut d = 1000;
    while d > 0 {
        let q = v / d % 10;
        if q != 0 || n > 0 || d == 1 {
            buf[n] = b'0' + q as u8;
            n += 1;
        }
        d /= 10;
    }
    core::str::from_utf8(&buf[..n]).unwrap_or("")
}

/// The turn's strip at `(x, y)`, 960 x 462: the page's rows as the window
/// shows them at scroll 0 on the left, the neighbour page's on the right.
#[must_use]
pub fn strip(x: u32, y: u32) -> u32 {
    if x < W {
        page(x, y + MARGIN, false)
    } else {
        page(x - W, y + MARGIN, true)
    }
}

/// `a` to `b` eased out (fast start, slow end), step `k` of `n`.
#[must_use]
pub const fn ease_out(a: i32, b: i32, k: i32, n: i32) -> i32 {
    a + (b - a) * (k * (2 * n - k)) / (n * n)
}

/// `a` to `b` eased in and out (smoothstep), step `k` of `n`.
#[must_use]
pub const fn ease_in_out(a: i32, b: i32, k: i32, n: i32) -> i32 {
    let (k, n) = (k as i64, n as i64);
    a + ((b - a) as i64 * (3 * k * k * n - 2 * k * k * k) / (n * n * n)) as i32
}

/// The scripted fling and rebound (M2): flung down past the end into the
/// bottom margin (110 lines), back to the end, flung up past the start
/// into the top margin, back to the start; frame `f` of [`FLING_FRAMES`].
#[must_use]
pub const fn fling(f: usize) -> i32 {
    let f = f as i32;
    let over = SCROLL_MAX + 110;
    match f {
        ..100 => ease_out(0, over, f, 100),
        100..150 => ease_in_out(over, SCROLL_MAX, f - 100, 50),
        150..250 => ease_out(SCROLL_MAX, -110, f - 150, 100),
        _ => ease_in_out(-110, 0, f - 250, 50),
    }
}

/// One state of the video window: its place and size on screen, and the
/// picture's row and column shown at its top left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Win {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub row: u32,
    pub col: u32,
}

impl Win {
    /// The window's first pixel in a picture at `base`, `stride` bytes a
    /// line, `bpp` bytes a pixel.
    #[must_use]
    pub const fn start(&self, base: u32, stride: u32, bpp: u32) -> u32 {
        base + self.row * stride + self.col * bpp
    }
}

/// The page's window scrolled by `scroll` (negative: into the top margin).
#[must_use]
pub const fn page_win(scroll: i32) -> Win {
    Win {
        x: 0,
        y: WIN_Y,
        w: W,
        h: WIN_H,
        row: (MARGIN as i32 + scroll) as u32,
        col: 0,
    }
}

/// Reveal, step `k` of [`ENTRANCE_FRAMES`]: the window grows from the
/// Today card's rectangle (16, 210, 448 x 140) to the page's, the page in
/// its place behind it, read up to 52 rows lower so the card turns into
/// the page in place (the mock-up's refinement). The window's left edge
/// on 4-pixel steps (16-byte starts at ARGB8888).
#[must_use]
pub const fn reveal(k: u32) -> Win {
    let n = ENTRANCE_FRAMES as i32;
    let e = ease_out(0, 1024, k as i32, n) as u32;
    let x = (16 * (1024 - e) / 1024) & !3;
    let y = 210 - 116 * e / 1024;
    let bottom = 350 + (SOFTKEYS_Y - 350) * e / 1024;
    let low = 52 * (1024 - e) / 1024;
    Win {
        x,
        y,
        w: W - 2 * x,
        h: (bottom - y) & !1,
        row: MARGIN + (y - WIN_Y) - low,
        col: x,
    }
}

/// Cascade, step `k` of [`ENTRANCE_FRAMES`]: the page's window grows down
/// from under the tabs while the page rises 40 lines into place.
#[must_use]
pub const fn cascade(k: u32) -> Win {
    let n = ENTRANCE_FRAMES as i32;
    let e = ease_out(0, 1024, k as i32, n) as u32;
    let h = (WIN_H * e / 1024) & !1;
    Win {
        x: 0,
        y: WIN_Y,
        w: W,
        h: if h < 2 { 2 } else { h },
        row: MARGIN - 40 * (1024 - e) / 1024,
        col: 0,
    }
}

/// The turn by the start address alone: the window over the strip at
/// column `p`, a multiple of 4 (16 bytes at ARGB8888).
#[must_use]
pub const fn strip4(p: u32) -> Win {
    Win {
        x: 0,
        y: WIN_Y,
        w: W,
        h: WIN_H,
        row: 0,
        col: p & !3,
    }
}

/// The turn to the pixel (M3's 1-pixel steps): the start on the next
/// 4-pixel step and the window moved right by the 0-3 pixels left, its
/// width cut to stay even (the row cards' clear edges are 8 pixels wide,
/// so the cut never shows). Screen column `X` shows strip column `X + p`.
#[must_use]
pub const fn strip1(p: u32) -> Win {
    let c = (p + 3) & !3;
    let s = c - p;
    Win {
        x: s,
        y: WIN_Y,
        w: (W - s) & !1,
        h: WIN_H,
        row: 0,
        col: c,
    }
}

#[cfg(test)]
#[path = "wg_tests.rs"]
mod tests;
