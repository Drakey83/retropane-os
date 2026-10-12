//! The crash screen: what the loader shows when the program it ran crashed
//! or stopped responding. A red band says which, with what to do next;
//! below it, the end of the program's log, where the panic or fault message
//! is, chosen to fit so nothing scrolls (no copying a whole screen per line
//! of a long log).

use rp_hal_api::RegisterBus;

use crate::Display;
use crate::console::Console;
use crate::font::{HEIGHT, WIDTH};

/// Glyph scale (12 × 16 pixel cells: readable at arm's length).
pub const SCALE: usize = 2;
/// The band's height: two lines of text and a margin.
pub const BAND_H: usize = 2 * (HEIGHT + 1) * SCALE + 8;
/// Colors (RGB565): white on red for the band, light grey on very dark red
/// for the log.
pub const BAND_COLORS: (u16, u16) = (0xFFFF, 0xF800);
/// Log colors.
pub const LOG_COLORS: (u16, u16) = (0xE73C, 0x2000);
/// The line that says what went wrong (`PANIC ...`, `FAULT ...`): yellow.
pub const CAUSE_FG: u16 = 0xFFE0;

/// Draws the crash panel into the `w` × `h` region at the top of `fb`
/// (RGB565, `stride` bytes per line): `title` and `hint` in the band, the
/// tail of `log` below.
pub fn draw(
    fb: &mut [u8],
    stride: usize,
    (w, h): (usize, usize),
    title: &str,
    hint: &str,
    log: &[u8],
) {
    let (band, rest) = fb.split_at_mut((stride * BAND_H).min(fb.len()));
    let mut c = Console::new(band, stride, (0, 0), (w, BAND_H), SCALE, BAND_COLORS);
    let _ = core::fmt::Write::write_fmt(&mut c, format_args!(" {title}\n {hint}"));
    let body_h = h.saturating_sub(BAND_H);
    let mut c = Console::new(rest, stride, (0, 0), (w, body_h), SCALE, LOG_COLORS);
    let (cols, rows) = c.size();
    let start = tail_start(log, cols, rows);
    let tail = log[start..].trim_ascii_end();
    for (i, line) in tail.split(|&b| b == b'\n').enumerate() {
        if i > 0 {
            c.put('\n');
        }
        let cause = line.starts_with(b"PANIC") || line.starts_with(b"FAULT");
        c.set_fg(if cause { CAUSE_FG } else { LOG_COLORS.0 });
        for &b in line {
            if (0x20..=0x7E).contains(&b) {
                c.put(char::from(b));
            }
        }
    }
}

/// Where the part of `log` that fits `rows` rows of `cols` columns starts:
/// whole lines from the end, each taking as many rows as it wraps to.
#[must_use]
pub fn tail_start(log: &[u8], cols: usize, rows: usize) -> usize {
    if cols == 0 || rows == 0 {
        return log.len();
    }
    let text = log.trim_ascii_end();
    let mut used = 0;
    let mut start = text.len();
    for line in text.rsplit(|&b| b == b'\n') {
        let printable = line.iter().filter(|b| (0x20..=0x7E).contains(*b)).count();
        let need = printable.div_ceil(cols).max(1);
        if used + need > rows {
            break;
        }
        used += need;
        // `line` is a slice of `text`: where it starts.
        start = line.as_ptr() as usize - text.as_ptr() as usize;
    }
    start
}

/// The box an image's fault and panic handlers write their message into:
/// a console over the `w` × `h` region at `(x, y)` of the plane actually on
/// screen ([`Display::on_screen`]), found from the 2700G's registers `regs`
/// (read, never written) and its local memory `vram`. Every image's crash
/// path draws through here, so none can draw where nothing shows (Windows
/// Mobile leaves its plane at offset 0, not [`crate::FRAMEBUFFER_OFFSET`]).
pub fn emergency_console<B: RegisterBus>(
    regs: B,
    vram: &mut [u8],
    at: (usize, usize),
    size: (usize, usize),
    scale: usize,
    colors: (u16, u16),
) -> Console<'_> {
    let plane = Display::new(regs).on_screen(vram.len());
    let start = plane.offset.min(vram.len());
    Console::new(&mut vram[start..], plane.stride, at, size, scale, colors)
}

/// Columns of text a panel `w` pixels wide shows.
#[must_use]
pub fn columns(w: usize) -> usize {
    w / ((WIDTH + 1) * SCALE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reg;
    use rp_hal_mock::MockBus;

    #[test]
    fn emergency_box_lands_on_wm_plane_at_offset_0() {
        // Windows Mobile's plane, as measured on a real X51v (docs/hw/2700g.md).
        let mut bus = MockBus::new();
        bus.preset(reg::GSCTRL, 0x3C0E_FA7F);
        bus.preset(reg::GSCADR, 0x8000_0000);
        bus.preset(reg::GSADR, 0x0EC0_0000);
        let mut vram = vec![0u8; 16 << 20];
        let mut c = emergency_console(bus, &mut vram, (0, 576), (480, 64), 2, (0xFFFF, 0xA800));
        let _ = core::fmt::Write::write_str(&mut c, "PANIC");
        let row = 600 * 960;
        assert_eq!(
            &vram[row + 940..row + 942],
            &0xA800u16.to_le_bytes(),
            "box at offset 0"
        );
        let conventional = crate::FRAMEBUFFER_OFFSET + row;
        assert!(
            vram[conventional..conventional + 960]
                .iter()
                .all(|&b| b == 0),
            "nothing at 0x60000"
        );
    }

    #[test]
    fn the_tail_is_whole_lines_that_fit_with_wrapping() {
        let log = b"one\ntwo\nthree is long enough to wrap\nfour\n";
        // 10 columns: "three..." takes 3 rows, "four" 1.
        assert_eq!(
            &log[tail_start(log, 10, 4)..],
            b"three is long enough to wrap\nfour\n"
        );
        assert_eq!(&log[tail_start(log, 10, 3)..], b"four\n");
        assert_eq!(&log[tail_start(log, 10, 100)..], &log[..]);
        assert_eq!(tail_start(b"", 10, 5), 0);
        assert_eq!(tail_start(log, 0, 5), log.len());
    }

    #[test]
    fn draws_a_red_band_and_the_end_of_the_log() {
        let (w, h) = (480, 320);
        let stride = w * 2;
        let mut fb = vec![0u8; stride * h];
        let mut log = Vec::new();
        for i in 0..200 {
            log.extend_from_slice(format!("line {i}\n").as_bytes());
        }
        log.extend_from_slice(b"PANIC deliberate test panic\n");
        draw(
            &mut fb,
            stride,
            (w, h),
            "The program crashed",
            "Report: crashlog.txt",
            &log,
        );
        let px = |x: usize, y: usize| {
            u16::from_le_bytes([fb[y * stride + 2 * x], fb[y * stride + 2 * x + 1]])
        };
        assert_eq!(px(470, 2), BAND_COLORS.1, "band");
        assert_eq!(px(470, BAND_H + 2), LOG_COLORS.1, "log background");
        // Text in the band and in the log's last used row.
        assert!((0..w).any(|x| px(x, 8) == BAND_COLORS.0));
        let rows_used = (BAND_H..h)
            .filter(|&y| (0..w).any(|x| px(x, y) == LOG_COLORS.0))
            .count();
        assert!(rows_used > 100, "log text drawn ({rows_used} pixel rows)");
        // The cause, last, in yellow.
        assert!((BAND_H..h).any(|y| (0..w).any(|x| px(x, y) == CAUSE_FG)));
    }
}
