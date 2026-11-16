//! The test runner's screen: what is running, which step of how many it
//! is on, the newest measurement and a clock that keeps going, in text
//! large enough to read from where the X51v stands in its dock.
//!
//! Iterate, don't imitate (rule 0). The other test images scroll their
//! log up the screen, and a port elsewhere leaves the screen to the
//! engine (black until its renderer is up, frozen while it loads). A
//! program under this runner may take minutes to read its files with no
//! picture of its own, so the screen is the runner's: a fixed panel whose
//! parts are redrawn only when they change (a step, a note, the clock
//! once a second: a few dozen glyphs, nothing in the frames being timed),
//! fed by the two kinds of line a program prints about itself
//! (`rp_port::progress`), so the runner still holds nothing of any game.

use core::fmt::Write;

use rp_drv_mbx_display::Display;
use rp_drv_mbx_display::console::Console;
use rp_hal_pxa27x::{Mmio, rt};
use rp_hal_x51v::MBX_VRAM_SIZE;
use rp_port::progress::{BREAK, clock};

/// The panel's colours (RGB 5-6-5): light grey on dark blue, the step in
/// yellow, the note in cyan.
const BG: u16 = 0x0886;
const FG: u16 = 0xE73C;
const LOUD: u16 = 0xFFE0;
const COOL: u16 = 0x07FF;
/// The screen, upright.
const SIZE: (usize, usize) = (480, 640);
/// Left and right of the text.
const MARGIN: usize = 6;
/// The parts, top to bottom: the row each starts at, its height, its
/// glyph scale (a glyph cell is 6 x 8 pixels times the scale).
const TITLE: (usize, usize, usize) = (8, 56, 3);
const STEP: (usize, usize, usize) = (72, 40, 4);
const WHAT: (usize, usize, usize) = (120, 200, 3);
const NOTE: (usize, usize, usize) = (328, 104, 3);
const CLOCK: (usize, usize, usize) = (440, 40, 4);
const FOOT: (usize, usize, usize) = (488, 144, 2);
/// The OS timer's ticks in a second.
const TICKS_1S: u64 = 3_250_000;

struct Panel {
    /// The 2700G's whole memory, while the panel has the screen ([`take`]
    /// hands it to the program's 3D frames).
    chip: Option<&'static mut [u8]>,
    /// The plane on screen: where it starts and its bytes a line.
    offset: usize,
    stride: usize,
    /// The two title lines, for a screen given back.
    title: (&'static str, &'static str),
    /// The timer's counter when the clock was last brought up to date,
    /// and the ticks since the panel started (the counter wraps every 22
    /// minutes).
    last: u32,
    ticks: u64,
    /// The second the clock shows.
    shown: u32,
}

impl Panel {
    /// One part of the panel, cleared, as a console; `None` while the
    /// program has the screen.
    fn part(&mut self, (y, h, scale): (usize, usize, usize), fg: u16) -> Option<Console<'_>> {
        let vram = self.chip.as_mut()?;
        let at = self.offset + y * self.stride;
        let rows = vram.get_mut(at..at + h * self.stride)?;
        Some(Console::new(
            rows,
            self.stride,
            (MARGIN, 0),
            (SIZE.0 - 2 * MARGIN, h),
            scale,
            (fg, BG),
        ))
    }

    /// The whole screen cleared, the title and the clock drawn.
    fn draw_all(&mut self) {
        let (offset, stride) = (self.offset, self.stride);
        if let Some(vram) = self.chip.as_mut()
            && let Some(fb) = vram.get_mut(offset..offset + stride * SIZE.1)
        {
            let _ = Console::new(fb, stride, (0, 0), SIZE, 1, (FG, BG));
        }
        let (first, second) = self.title;
        if let Some(mut c) = self.part(TITLE, FG) {
            let _ = write!(
                c,
                "{first}
{second}"
            );
        }
        let shown = self.shown;
        self.draw_clock(shown);
    }

    /// Seconds since the panel started, brought up to `now`.
    fn seconds(&mut self, now: u32) -> u32 {
        self.ticks += u64::from(now.wrapping_sub(self.last));
        self.last = now;
        (self.ticks / TICKS_1S) as u32
    }

    fn draw_clock(&mut self, seconds: u32) {
        self.shown = seconds;
        let (m, s) = clock(seconds);
        if let Some(mut c) = self.part(CLOCK, FG) {
            let _ = write!(c, "clock {m}:{s:02}");
        }
    }
}

static PANEL: rt::Global<Option<Panel>> = rt::Global::new(None);

/// `text` on `c`, a [`BREAK`] as a new line.
fn put(c: &mut Console<'_>, text: &[u8]) {
    for &b in text {
        c.put(if b == BREAK { '\n' } else { char::from(b) });
    }
}

/// Takes the screen and draws the panel with its two title lines; the
/// 2700G's registers (for the program's 3D frames), or `None` if there is
/// no display (the run goes on without a screen).
pub fn start(first: &'static str, second: &'static str) -> Option<Mmio> {
    let (regs, vram) = rp_image::take_mbx()?;
    let mut display = Display::new(regs);
    let (plane, _) = display.adopt(MBX_VRAM_SIZE as usize);
    let (stride, len) = (plane.stride, plane.stride * usize::from(plane.height));
    if usize::from(plane.height) < SIZE.1 || vram.len() < plane.offset + len {
        return None;
    }
    let mut panel = Panel {
        chip: Some(vram),
        offset: plane.offset,
        stride,
        title: (first, second),
        last: rt::ticks(),
        ticks: 0,
        shown: 0,
    };
    panel.draw_all();
    PANEL.with(|p| *p = Some(panel));
    Some(display.into_inner())
}

/// The 2700G's memory for the program's 3D frames, and the plane on
/// screen (where it starts, its bytes a line); the panel draws nothing
/// until [`give_back`]. `None` without a display or if taken.
pub fn take() -> Option<(&'static mut [u8], usize, usize)> {
    PANEL.with(|p| {
        let p = p.as_mut()?;
        let vram = p.chip.take()?;
        Some((vram, p.offset, p.stride))
    })
}

/// The 2700G's memory back from the program: the panel drawn again whole.
pub fn give_back(vram: &'static mut [u8]) {
    PANEL.with(|p| {
        if let Some(p) = p.as_mut() {
            p.chip = Some(vram);
            p.draw_all();
        }
    });
}

/// The step the run is on: `n` of `of`, and what it is (`text`, a `|`
/// for a new line).
pub fn step(n: u32, of: u32, text: &[u8]) {
    PANEL.with(|p| {
        let Some(p) = p else { return };
        if let Some(mut c) = p.part(STEP, LOUD) {
            let _ = write!(c, "STEP {n} of {of}");
        }
        if let Some(mut c) = p.part(WHAT, FG) {
            put(&mut c, text);
        }
    });
}

/// The run is over: said where the step was.
pub fn done(text: &[u8]) {
    PANEL.with(|p| {
        let Some(p) = p else { return };
        if let Some(mut c) = p.part(STEP, LOUD) {
            let _ = write!(c, "FINISHED");
        }
        if let Some(mut c) = p.part(WHAT, FG) {
            put(&mut c, text);
        }
    });
}

/// The newest measurement (`text`, a `|` for a new line).
pub fn note(text: &[u8]) {
    PANEL.with(|p| {
        if let Some(mut c) = p.as_mut().and_then(|p| p.part(NOTE, COOL)) {
            put(&mut c, text);
        }
    });
}

/// The small print at the bottom, written by `lines`.
pub fn foot(lines: impl FnOnce(&mut dyn Write)) {
    PANEL.with(|p| {
        if let Some(mut c) = p.as_mut().and_then(|p| p.part(FOOT, FG)) {
            lines(&mut c);
        }
    });
}

/// Brings the clock up to `now` (the timer's counter): redrawn when its
/// second has changed. Called as the run makes progress, at least every
/// few minutes (the counter's wrap). The seconds since the panel started
/// (0 without a screen).
pub fn beat(now: u32) -> u32 {
    PANEL.with(|p| {
        let Some(p) = p else { return 0 };
        let seconds = p.seconds(now);
        if seconds != p.shown {
            p.draw_clock(seconds);
        }
        seconds
    })
}
