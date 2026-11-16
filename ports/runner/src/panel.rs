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
use rp_hal_pxa27x::rt;
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
    /// The plane on screen.
    fb: &'static mut [u8],
    stride: usize,
    /// The timer's counter when the clock was last brought up to date,
    /// and the ticks since the panel started (the counter wraps every 22
    /// minutes).
    last: u32,
    ticks: u64,
    /// The second the clock shows.
    shown: u32,
}

impl Panel {
    /// One part of the panel, cleared, as a console.
    fn part(&mut self, (y, h, scale): (usize, usize, usize), fg: u16) -> Console<'_> {
        let rows = &mut self.fb[y * self.stride..(y + h) * self.stride];
        Console::new(
            rows,
            self.stride,
            (MARGIN, 0),
            (SIZE.0 - 2 * MARGIN, h),
            scale,
            (fg, BG),
        )
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
        let mut c = self.part(CLOCK, FG);
        let _ = write!(c, "clock {m}:{s:02}");
    }
}

static PANEL: rt::Global<Option<Panel>> = rt::Global::new(None);

/// `text` on `c`, a [`BREAK`] as a new line.
fn put(c: &mut Console<'_>, text: &[u8]) {
    for &b in text {
        c.put(if b == BREAK { '\n' } else { char::from(b) });
    }
}

/// Takes the screen and draws the panel with its two title lines; false
/// if there is no display (the run goes on without a screen).
pub fn start(first: &str, second: &str) -> bool {
    let Some((regs, vram)) = rp_image::take_mbx() else {
        return false;
    };
    let (plane, _) = Display::new(regs).adopt(MBX_VRAM_SIZE as usize);
    let (stride, len) = (plane.stride, plane.stride * usize::from(plane.height));
    if usize::from(plane.height) < SIZE.1 || vram.len() < plane.offset + len {
        return false;
    }
    let (_, rest) = vram.split_at_mut(plane.offset);
    let (fb, _) = rest.split_at_mut(len);
    let mut panel = Panel {
        fb,
        stride,
        last: rt::ticks(),
        ticks: 0,
        shown: 0,
    };
    // The whole screen once, then the parts.
    let _ = Console::new(&mut *panel.fb, stride, (0, 0), SIZE, 1, (FG, BG));
    {
        let mut c = panel.part(TITLE, FG);
        let _ = write!(c, "{first}\n{second}");
    }
    panel.draw_clock(0);
    PANEL.with(|p| *p = Some(panel));
    true
}

/// The step the run is on: `n` of `of`, and what it is (`text`, a `|`
/// for a new line).
pub fn step(n: u32, of: u32, text: &[u8]) {
    PANEL.with(|p| {
        let Some(p) = p else { return };
        {
            let mut c = p.part(STEP, LOUD);
            let _ = write!(c, "STEP {n} of {of}");
        }
        put(&mut p.part(WHAT, FG), text);
    });
}

/// The run is over: said where the step was.
pub fn done(text: &[u8]) {
    PANEL.with(|p| {
        let Some(p) = p else { return };
        {
            let mut c = p.part(STEP, LOUD);
            let _ = write!(c, "FINISHED");
        }
        put(&mut p.part(WHAT, FG), text);
    });
}

/// The newest measurement (`text`, a `|` for a new line).
pub fn note(text: &[u8]) {
    PANEL.with(|p| {
        if let Some(p) = p {
            put(&mut p.part(NOTE, COOL), text);
        }
    });
}

/// The small print at the bottom, written by `lines`.
pub fn foot(lines: impl FnOnce(&mut dyn Write)) {
    PANEL.with(|p| {
        if let Some(p) = p {
            lines(&mut p.part(FOOT, FG));
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
