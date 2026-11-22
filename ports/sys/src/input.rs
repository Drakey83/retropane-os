//! A game's input: the X51v's buttons and its stylus as events a C program
//! polls (`rp_input_open`, `rp_input_poll`), turned with the picture when
//! the game plays in landscape.
//!
//! Iterate, don't imitate (rule 0). SDL gives a port a desktop's event
//! queue (key symbols of a PC keyboard, a mouse, fingers as floats from 0
//! to 1) and every port then maps the device's few buttons onto it and
//! turns the axes itself; Windows Mobile games read GAPI's key list and
//! raw stylus messages, each with its own smoothing; the DOS port hooks
//! the keyboard interrupt. Ours hands the game what the X51v has, already
//! made right once for every port:
//!
//! - **The stylus is what the person meant, in the game's own pixels.**
//!   Readings go through the OS's touch filter ([`rp_touch_cal::Filter`]:
//!   by pressure, so no position taken as the stylus lands or lifts; tuned
//!   on the owner's recorded taps) and the screen's calibration, then are
//!   turned to the game's orientation, in integers.
//! - **The D-pad turns with the picture** ([`Turn`]): held in landscape,
//!   the button under the thumb's "up" is `Up`, whichever way the device
//!   was turned. The other buttons keep their names: binding them is the
//!   game's (every key bindable).
//! - **Every event carries when it was read**, on the game's clock, so a
//!   game can weigh a drag by its speed without sampling the clock itself.
//!
//! The pad is read when the game polls and at its clock reads (as the
//! sound's mixer is woken: readiness line 6 decides the proper waker,
//! R253): a press and release between two reads is missed.

use rp_drv_pxa_keypad::{Button, Pressed};
use rp_hal_cabi::{Errno, Event, event};
use rp_touch_cal::{Calibration, Filter, Pen, Sample};

/// The X51v's screen as it is mounted: upright, in pixels.
pub const SCREEN: (u16, u16) = (480, 640);

/// How the game holds the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turn {
    /// Upright (480 x 640): nothing turned.
    None,
    /// Landscape (640 x 480) with the device's top edge at the player's
    /// left (the device turned a quarter turn anticlockwise).
    Left,
    /// Landscape with the device's top edge at the player's right.
    Right,
}

impl Turn {
    /// The turn a C program names (`RP_TURN_*` in `retropane.h`).
    #[must_use]
    pub const fn from_c(turn: u8) -> Option<Self> {
        match turn {
            event::TURN_NONE => Some(Self::None),
            event::TURN_LEFT => Some(Self::Left),
            event::TURN_RIGHT => Some(Self::Right),
            _ => None,
        }
    }

    /// The screen's size as the game sees it.
    #[must_use]
    pub const fn size(self) -> (u16, u16) {
        match self {
            Self::None => SCREEN,
            Self::Left | Self::Right => (SCREEN.1, SCREEN.0),
        }
    }

    /// An upright screen pixel as the game's pixel. Pixels outside the
    /// screen (a calibration that overshoots) are brought to its edge.
    #[must_use]
    pub fn point(self, (x, y): (u16, u16)) -> (i16, i16) {
        let (x, y) = (x.min(SCREEN.0 - 1), y.min(SCREEN.1 - 1));
        let (x, y) = match self {
            Self::None => (x, y),
            // The device's top edge is the player's left edge: down the
            // device is rightwards, and the device's right edge is the top.
            Self::Left => (y, SCREEN.0 - 1 - x),
            Self::Right => (SCREEN.1 - 1 - y, x),
        };
        (x as i16, y as i16)
    }

    /// A button's code as the player holds the device: the D-pad's four
    /// directions turned, every other button its own.
    #[must_use]
    pub const fn key(self, b: Button) -> u8 {
        let b = match (self, b) {
            (Self::Left, Button::Up) => Button::Left,
            (Self::Left, Button::Left) => Button::Down,
            (Self::Left, Button::Down) => Button::Right,
            (Self::Left, Button::Right) => Button::Up,
            (Self::Right, Button::Up) => Button::Right,
            (Self::Right, Button::Right) => Button::Down,
            (Self::Right, Button::Down) => Button::Left,
            (Self::Right, Button::Left) => Button::Up,
            (_, b) => b,
        };
        match b {
            Button::Up => event::KEY_UP,
            Button::Down => event::KEY_DOWN,
            Button::Left => event::KEY_LEFT,
            Button::Right => event::KEY_RIGHT,
            Button::Enter => event::KEY_ENTER,
            Button::Calendar => event::KEY_CALENDAR,
            Button::Contacts => event::KEY_CONTACTS,
            Button::Mail => event::KEY_MAIL,
            Button::Home => event::KEY_HOME,
            Button::Record => event::KEY_RECORD,
            Button::Wifi => event::KEY_WIFI,
        }
    }
}

/// The device's buttons and touch chip, as the image wires them.
pub trait Pad {
    /// The buttons held now.
    fn pressed(&mut self) -> Pressed;
    /// One raw reading of the stylus, if it touches the screen.
    fn pen(&mut self) -> Option<Sample>;
}

/// What [`crate::sys::System`] asks of a game's input.
pub trait Events {
    /// Starts giving events, the screen held as `turn`; what was pressed
    /// before is not reported.
    ///
    /// # Errors
    /// Open already (`EBUSY`).
    fn open(&mut self, turn: Turn, now_us: u64) -> Result<(), Errno>;
    /// Reads the pad and gives the oldest event not yet given.
    ///
    /// # Errors
    /// Not open (`EBADF`).
    fn poll(&mut self, now_us: u64) -> Result<Option<Event>, Errno>;
    /// Reads the pad if it is open and its time has come (called from the
    /// game's clock reads, so a press between two polls is kept).
    fn pump(&mut self, now_us: u64);
}

/// Events kept until the game polls: a second of presses and a long drag
/// at 30 frames a second. When full the oldest is dropped (and counted):
/// what a late game needs is what is held now, not what was.
pub const QUEUE: usize = 32;

/// The pad is read no more often than this (a stylus reading takes about
/// a millisecond of the CPU while it touches).
pub const READ_EVERY_US: u64 = 4_000;

/// R355's measure (the owner, run 312: "the mouse look with the stylus is
/// not smooth at all and very jerky"): one touch of the stylus as the pad
/// read it, said in one log line when it lifts. How often it was read
/// (the pad is read at the game's clock reads, so the readings follow the
/// game's frames, not a clock of their own), what the OS's filter made of
/// each reading, and how far the raw position moved between readings (a
/// stylus held still shows the panel's noise).
#[derive(Clone, Copy)]
struct Stroke {
    from_us: u64,
    last_us: u64,
    reads: u32,
    /// Between readings: under 6, 10, 16, 25, 50 ms, longer.
    gaps: [u32; 6],
    /// The raw step between readings (the larger axis): up to 2, 5, 10,
    /// 20, 50 units, more.
    steps: [u32; 6],
    /// The filter's answers: moves, holds, not counted.
    moves: u32,
    holds: u32,
    nothing: u32,
    last: (u16, u16),
    min: (u16, u16),
    max: (u16, u16),
}

impl Stroke {
    const NONE: Self = Self {
        from_us: 0,
        last_us: 0,
        reads: 0,
        gaps: [0; 6],
        steps: [0; 6],
        moves: 0,
        holds: 0,
        nothing: 0,
        last: (0, 0),
        min: (u16::MAX, u16::MAX),
        max: (0, 0),
    };

    fn bucket(v: u64, edges: [u64; 5]) -> usize {
        edges.iter().position(|&e| v < e).unwrap_or(5)
    }

    fn reading(&mut self, s: Sample, pen: Pen, now_us: u64) {
        if self.reads == 0 {
            self.from_us = now_us;
        } else {
            let gap = now_us.saturating_sub(self.last_us) / 1_000;
            self.gaps[Self::bucket(gap, [6, 10, 16, 25, 50])] += 1;
            let step = self.last.0.abs_diff(s.x).max(self.last.1.abs_diff(s.y));
            self.steps[Self::bucket(u64::from(step), [3, 6, 11, 21, 51])] += 1;
        }
        self.reads += 1;
        self.last_us = now_us;
        self.last = (s.x, s.y);
        self.min = (self.min.0.min(s.x), self.min.1.min(s.y));
        self.max = (self.max.0.max(s.x), self.max.1.max(s.y));
        match pen {
            Pen::Move(..) => self.moves += 1,
            Pen::Hold(..) => self.holds += 1,
            Pen::Nothing => self.nothing += 1,
            Pen::Down(..) | Pen::Up(..) => {}
        }
    }
}

/// R355 (the owner, runs 312 and 314: "the mouse look still isnt very
/// smooth"): the stylus's path while it is down, smoothed.
///
/// Measured in run 314 (44 strokes): the pad gives 191 readings a second,
/// but the OS's touch filter, built to tell a tap from a slip, passed only
/// 57% of them as moves (35% held as "the same place", 7% dropped for an
/// unsteady pressure), each move the mean of two raw readings in whole
/// pixels; 10% of the game's frames under the stylus got no move at all
/// and others several pixels at once.
///
/// Iterate, don't imitate (rule 0). Windows Mobile hands a game the touch
/// driver's tap-filtered points, and a game that wants a smooth look adds
/// its own running mean (lag at every speed); a desktop reads a mouse,
/// which has no noise to remove. Today's way for a noisy pointer is the
/// 1-euro filter (Casiez, Roussel and Vogel, CHI 2012): a low-pass whose
/// cut-off rises with the pointer's speed, so a slow stylus is steadied
/// hard and a fast one followed with next to no lag. Ours runs it on every
/// raw reading the touch filter counts (its moves and its holds: the
/// pressure rules still say when the pen is down, lifting or bouncing),
/// after a median of three that drops a single wild reading, in integers
/// (no FPU), on the pad's own units (a pixel is 5.6 to 7.5 of them, so the
/// path is kept finer than the screen), with the time between readings
/// measured, not assumed. A move is said whenever the steadied place
/// reaches another pixel.
///
/// The cut-offs are a first fit to run 314's numbers, to be judged by the
/// owner's eye and refitted from the next run's stroke lines.
#[derive(Clone, Copy)]
struct Smooth {
    /// The last readings, newest first, for the median.
    hist: [(u16, u16); 3],
    n: u8,
    /// The steadied place, in 1/256ths of a raw unit.
    at: (i64, i64),
    /// The steadied speed, in 1/256ths of a raw unit a second.
    speed: (i64, i64),
    at_us: u64,
    /// The steadied place last handed on, in raw units: the next is handed
    /// on once it is `step` raw units away (five eighths of a pixel by the
    /// calibration), so a place resting on the edge between two pixels
    /// does not flicker between them.
    anchor: (u16, u16),
    step: u16,
    /// The pixel last said to the game, and whether any move was.
    said: (i16, i16),
    moved: bool,
    live: bool,
}

/// The cut-off with the stylus at rest, in 1/1000 Hz: jitter of a unit or
/// two (a held stylus, the taps of 2026-10-13) is under a quarter pixel.
const SMOOTH_MIN_MHZ: u64 = 2_500;
/// The cut-off's rise, in 1/1000 Hz for each raw unit a second: a slow
/// look (300 units, 40 pixels a second) is followed at about 7 Hz, a quick
/// turn (3,000) at 47 Hz, where the 5 ms between readings is the lag.
const SMOOTH_RISE: u64 = 15;
/// The speed's own cut-off, in 1/1000 Hz.
const SMOOTH_SPEED_MHZ: u64 = 1_500;
/// Five eighths of a screen pixel in raw units by `cal` (at least 1): the
/// step the steadied place is handed on in.
fn smooth_step(cal: &Calibration) -> u16 {
    let (a, b) = (cal.map(1_000, 1_000), cal.map(1_064, 1_064));
    let px = (b.0 - a.0).abs().max((b.1 - a.1).abs()).max(1);
    (64 * 5 / 8 / px).clamp(1, 64) as u16
}

impl Smooth {
    const NONE: Self = Self {
        hist: [(0, 0); 3],
        n: 0,
        at: (0, 0),
        speed: (0, 0),
        at_us: 0,
        anchor: (0, 0),
        step: 1,
        said: (0, 0),
        moved: false,
        live: false,
    };

    /// The low-pass's share of the new value for a cut-off and a time
    /// step, in 1/65536ths: 2 pi f dt / (2 pi f dt + 1).
    fn share(mhz: u64, dt_us: u64) -> i64 {
        // 2 pi x 65536 x (mhz / 1000) x (dt_us / 1e6)
        let r = mhz * dt_us * 411_775 / 1_000_000_000;
        (r * 65_536 / (r + 65_536)) as i64
    }

    fn median(a: u16, b: u16, c: u16) -> u16 {
        a.max(b).min(a.min(b).max(c))
    }

    /// The pen is down at raw `p`.
    fn down(&mut self, p: (u16, u16), px: (i16, i16), step: u16, now_us: u64) {
        *self = Self {
            hist: [p; 3],
            n: 1,
            at: (i64::from(p.0) << 8, i64::from(p.1) << 8),
            speed: (0, 0),
            at_us: now_us,
            anchor: p,
            step,
            said: px,
            moved: false,
            live: true,
        };
    }

    /// One counted raw reading; the steadied place, in raw units.
    fn feed(&mut self, p: (u16, u16), now_us: u64) -> (u16, u16) {
        self.hist = [p, self.hist[0], self.hist[1]];
        self.n = (self.n + 1).min(3);
        let m = if self.n < 3 {
            p
        } else {
            (
                Self::median(self.hist[0].0, self.hist[1].0, self.hist[2].0),
                Self::median(self.hist[0].1, self.hist[1].1, self.hist[2].1),
            )
        };
        let dt = now_us.saturating_sub(self.at_us).clamp(1_000, 50_000);
        self.at_us = now_us;
        let want = (i64::from(m.0) << 8, i64::from(m.1) << 8);
        let k = Self::share(SMOOTH_SPEED_MHZ, dt);
        let v = |want: i64, at: i64, speed: i64| {
            let now = (want - at) * 1_000_000 / dt as i64;
            speed + (((now - speed) * k) >> 16)
        };
        self.speed = (
            v(want.0, self.at.0, self.speed.0),
            v(want.1, self.at.1, self.speed.1),
        );
        // one cut-off for both axes (the faster one's), so the path keeps
        // its direction
        let fast = (self.speed.0.unsigned_abs().max(self.speed.1.unsigned_abs())) >> 8;
        let a = Self::share(SMOOTH_MIN_MHZ + SMOOTH_RISE * fast.min(20_000), dt);
        self.at.0 += ((want.0 - self.at.0) * a) >> 16;
        self.at.1 += ((want.1 - self.at.1) * a) >> 16;
        let r = |v: i64| ((v + 128) >> 8).clamp(0, i64::from(u16::MAX)) as u16;
        let p = (r(self.at.0), r(self.at.1));
        if p.0.abs_diff(self.anchor.0) >= self.step || p.1.abs_diff(self.anchor.1) >= self.step {
            self.anchor = p;
        }
        self.anchor
    }
}

/// Strokes said one by one ([`Input::with_pen_log`]); after them only the
/// ones longer than a second, up to [`STROKES_MOST`] lines in all.
const STROKES_SAID: u32 = 40;
const STROKES_MOST: u32 = 160;

/// The pad's readings as events, in a queue.
pub struct Input<P: Pad> {
    pad: P,
    cal: Calibration,
    turn: Option<Turn>,
    held: Pressed,
    filter: Filter,
    queue: [Event; QUEUE],
    /// The oldest event's place and how many are kept.
    first: usize,
    len: usize,
    dropped: u32,
    read_at: u64,
    /// R355's measure: where each stroke's line goes (`None`: not kept).
    pen_log: Option<fn(core::fmt::Arguments<'_>)>,
    stroke: Stroke,
    strokes: u32,
    said: u32,
    /// R355: the stylus's path steadied ([`Smooth`]; off: the touch
    /// filter's own moves, as before).
    smooth_on: bool,
    smooth: Smooth,
}

impl<P: Pad> Input<P> {
    /// The input over `pad`, the stylus mapped to the screen by `cal`.
    pub fn new(pad: P, cal: Calibration) -> Self {
        Self {
            pad,
            cal,
            turn: None,
            held: Pressed::NONE,
            filter: Filter::new(),
            queue: [Event::NONE; QUEUE],
            first: 0,
            len: 0,
            dropped: 0,
            read_at: 0,
            pen_log: None,
            stroke: Stroke::NONE,
            strokes: 0,
            said: 0,
            smooth_on: true,
            smooth: Smooth::NONE,
        }
    }

    /// R355: the stylus's moves as the touch filter gives them, not
    /// steadied (`-rpnopensmooth`: run 314's way).
    #[must_use]
    pub fn without_pen_smooth(mut self) -> Self {
        self.smooth_on = false;
        self
    }

    /// R355's measure: each touch of the stylus said in one line of `log`
    /// when it lifts (the first 40, then the ones longer than a second).
    #[must_use]
    pub fn with_pen_log(mut self, log: fn(core::fmt::Arguments<'_>)) -> Self {
        self.pen_log = Some(log);
        self
    }

    /// The stroke that just ended, said.
    fn stroke_end(&mut self) {
        let k = core::mem::replace(&mut self.stroke, Stroke::NONE);
        let Some(log) = self.pen_log.filter(|_| k.reads > 0) else {
            return;
        };
        self.strokes += 1;
        let ms = k.last_us.saturating_sub(k.from_us) / 1_000;
        if self.said >= STROKES_MOST || (self.strokes > STROKES_SAID && ms < 1_000) {
            return;
        }
        self.said += 1;
        log(format_args!(
            "input: R355 stroke {}: {ms} ms, {} readings ({} a second); between readings under 6 ms {}, under 10 {}, under 16 {}, under 25 {}, under 50 {}, longer {}; the filter: {} moves, {} holds, {} not counted; the raw step between readings up to 2 units {}, 5 {}, 10 {}, 20 {}, 50 {}, more {}; the raw range {} x {} units (a screen pixel is about 7.5 x 5.6)",
            self.strokes,
            k.reads,
            u64::from(k.reads.saturating_sub(1)) * 1_000 / ms.max(1),
            k.gaps[0],
            k.gaps[1],
            k.gaps[2],
            k.gaps[3],
            k.gaps[4],
            k.gaps[5],
            k.moves,
            k.holds,
            k.nothing,
            k.steps[0],
            k.steps[1],
            k.steps[2],
            k.steps[3],
            k.steps[4],
            k.steps[5],
            k.max.0.saturating_sub(k.min.0),
            k.max.1.saturating_sub(k.min.1),
        ));
    }

    /// Events dropped because the game did not poll (the queue was full).
    #[must_use]
    pub const fn dropped(&self) -> u32 {
        self.dropped
    }

    /// The pad (tests set what it reads).
    pub fn pad(&mut self) -> &mut P {
        &mut self.pad
    }

    fn push(&mut self, e: Event) {
        if self.len == QUEUE {
            self.first = (self.first + 1) % QUEUE;
            self.len -= 1;
            self.dropped += 1;
        }
        self.queue[(self.first + self.len) % QUEUE] = e;
        self.len += 1;
    }

    /// One reading of the pad, as events.
    fn read(&mut self, turn: Turn, now_us: u64) {
        self.read_at = now_us;
        let now = self.pad.pressed();
        for (b, down) in self.held.changes(now) {
            self.push(Event::key(turn.key(b), down, now_us));
        }
        self.held = now;
        let raw = self.pad.pen();
        let pen = self.filter.sample(raw, (now_us / 1_000) as u32);
        if self.pen_log.is_some() {
            match raw.filter(|s| s.z1 >= rp_touch_cal::Z_MIN) {
                Some(s) => self.stroke.reading(s, pen, now_us),
                None if !self.filter.down() => self.stroke_end(),
                None => {}
            }
        }
        if self.smooth_on {
            match (pen, raw) {
                (Pen::Down(x, y), _) => {
                    let px = turn.point(self.cal.to_screen(x, y, SCREEN));
                    self.smooth.down((x, y), px, smooth_step(&self.cal), now_us);
                    self.push(Event::pen(event::PEN_DOWN, px.0, px.1, now_us));
                }
                // every reading the filter counts while the pen is down
                (Pen::Move(..) | Pen::Hold(..), Some(s)) if self.smooth.live => {
                    let (x, y) = self.smooth.feed((s.x, s.y), now_us);
                    let px = turn.point(self.cal.to_screen(x, y, SCREEN));
                    if px != self.smooth.said {
                        self.smooth.said = px;
                        self.smooth.moved = true;
                        self.push(Event::pen(event::PEN_MOVE, px.0, px.1, now_us));
                    }
                }
                (Pen::Up(x, y), _) => {
                    // lifted where the path last was (a tap: where the
                    // filter put it)
                    let px = if self.smooth.live && self.smooth.moved {
                        self.smooth.said
                    } else {
                        turn.point(self.cal.to_screen(x, y, SCREEN))
                    };
                    self.smooth = Smooth::NONE;
                    self.push(Event::pen(event::PEN_UP, px.0, px.1, now_us));
                }
                _ => {}
            }
            return;
        }
        let (kind, rx, ry) = match pen {
            Pen::Down(x, y) => (event::PEN_DOWN, x, y),
            Pen::Move(x, y) => (event::PEN_MOVE, x, y),
            Pen::Up(x, y) => (event::PEN_UP, x, y),
            Pen::Hold(..) | Pen::Nothing => return,
        };
        let (x, y) = turn.point(self.cal.to_screen(rx, ry, SCREEN));
        self.push(Event::pen(kind, x, y, now_us));
    }
}

impl<P: Pad> Events for Input<P> {
    fn open(&mut self, turn: Turn, now_us: u64) -> Result<(), Errno> {
        if self.turn.is_some() {
            return Err(Errno::Busy);
        }
        self.turn = Some(turn);
        // From now: what is held already is no press, and a touch already
        // on the screen starts when the filter next sees it land.
        self.held = self.pad.pressed();
        self.filter = Filter::new();
        self.smooth = Smooth::NONE;
        self.first = 0;
        self.len = 0;
        self.read_at = now_us;
        Ok(())
    }

    fn poll(&mut self, now_us: u64) -> Result<Option<Event>, Errno> {
        let turn = self.turn.ok_or(Errno::BadHandle)?;
        // The pad is read once an interval, by whoever comes first (this
        // poll or the clock's pump), never once a poll. Run 316's samples,
        // frame by frame: eight of its forty slowest frames of play (101
        // to 245 ms) were 49% to 66% inside this call. A game drains its
        // events in a loop until none is left; a poll that found the queue
        // empty used to read the pad again (a millisecond with the stylus
        // down), and a moving stylus gave that reading a new event, so the
        // loop went round for as long as the stylus kept moving: the look
        // itself froze the frame. Now the loop ends when the queue is
        // empty; what it hands out is never more than an interval old.
        if now_us.saturating_sub(self.read_at) >= READ_EVERY_US {
            self.read(turn, now_us);
        }
        if self.len == 0 {
            return Ok(None);
        }
        let e = self.queue[self.first];
        self.first = (self.first + 1) % QUEUE;
        self.len -= 1;
        Ok(Some(e))
    }

    fn pump(&mut self, now_us: u64) {
        if let Some(turn) = self.turn
            && now_us.saturating_sub(self.read_at) >= READ_EVERY_US
        {
            self.read(turn, now_us);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A pad that reads what the test set.
    #[derive(Default)]
    pub(crate) struct Fake {
        pub(crate) pressed: Pressed,
        pub(crate) pen: Option<Sample>,
        pub(crate) reads: u32,
    }

    impl Pad for Fake {
        fn pressed(&mut self) -> Pressed {
            self.reads += 1;
            self.pressed
        }
        fn pen(&mut self) -> Option<Sample> {
            self.pen
        }
    }

    /// A calibration whose raw readings are the upright pixels themselves.
    const PIXELS: Calibration = Calibration {
        a: rp_touch_cal::ONE,
        b: 0,
        c: 0,
        d: 0,
        e: rp_touch_cal::ONE,
        f: 0,
    };

    fn input() -> Input<Fake> {
        Input::new(Fake::default(), PIXELS)
    }

    /// Every event waiting at `now`.
    fn drain(i: &mut Input<Fake>, now: u64) -> Vec<Event> {
        let mut out = Vec::new();
        while let Some(e) = i.poll(now).unwrap() {
            out.push(e);
        }
        out
    }

    /// A firm touch at upright pixel `(x, y)`.
    fn touch(x: u16, y: u16) -> Option<Sample> {
        Some(Sample { x, y, z1: 2_000 })
    }

    /// Holds the stylus at `(x, y)` from `*now`, a reading every 5 ms,
    /// until the filter has reported something and the steadied path has
    /// come to rest (no event for 100 ms); the events.
    fn hold(i: &mut Input<Fake>, x: u16, y: u16, now: &mut u64) -> Vec<Event> {
        i.pad().pen = touch(x, y);
        let mut out = Vec::new();
        let mut quiet = 0;
        for _ in 0..400 {
            *now += 5_000;
            let got = drain(i, *now);
            quiet = if got.is_empty() { quiet + 1 } else { 0 };
            out.extend(got);
            if !out.is_empty() && quiet >= 20 {
                break;
            }
        }
        out
    }

    #[test]
    fn the_screen_s_corners_turn_with_the_picture() {
        // Upright: as they are.
        assert_eq!(Turn::None.size(), (480, 640));
        assert_eq!(Turn::None.point((0, 0)), (0, 0));
        assert_eq!(Turn::None.point((479, 639)), (479, 639));
        // The device's top at the player's left: its top-left corner is
        // the player's bottom-left, its top-right the player's top-left.
        assert_eq!(Turn::Left.size(), (640, 480));
        assert_eq!(Turn::Left.point((0, 0)), (0, 479));
        assert_eq!(Turn::Left.point((479, 0)), (0, 0));
        assert_eq!(Turn::Left.point((479, 639)), (639, 0));
        assert_eq!(Turn::Left.point((0, 639)), (639, 479));
        // The device's top at the player's right: the other way round.
        assert_eq!(Turn::Right.point((0, 0)), (639, 0));
        assert_eq!(Turn::Right.point((479, 0)), (639, 479));
        assert_eq!(Turn::Right.point((479, 639)), (0, 479));
        assert_eq!(Turn::Right.point((0, 639)), (0, 0));
        // The two turns are half a turn apart, and nothing leaves the
        // screen: a reading past its edge is brought to the edge.
        for p in [
            (0, 0),
            (17, 300),
            (479, 639),
            (480, 640),
            (u16::MAX, u16::MAX),
        ] {
            let (lx, ly) = Turn::Left.point(p);
            let (rx, ry) = Turn::Right.point(p);
            assert_eq!((lx + rx, ly + ry), (639, 479), "{p:?}");
            assert!((0..640).contains(&lx) && (0..480).contains(&ly), "{p:?}");
            let (x, y) = Turn::None.point(p);
            assert!((0..480).contains(&x) && (0..640).contains(&y), "{p:?}");
        }
    }

    #[test]
    fn the_d_pad_turns_with_the_picture_and_the_other_buttons_keep_their_names() {
        use Button::*;
        let dirs = |t: Turn| [Up, Right, Down, Left].map(|b| t.key(b));
        let (u, r, d, l) = (
            event::KEY_UP,
            event::KEY_RIGHT,
            event::KEY_DOWN,
            event::KEY_LEFT,
        );
        assert_eq!(dirs(Turn::None), [u, r, d, l]);
        // Top at the left: the button towards the device's top points left.
        assert_eq!(dirs(Turn::Left), [l, u, r, d]);
        assert_eq!(dirs(Turn::Right), [r, d, l, u]);
        // A press towards a screen edge moves towards that edge as the
        // game sees it: the D-pad agrees with the stylus. (From the
        // screen's middle, a step the button's way in upright pixels.)
        for t in [Turn::None, Turn::Left, Turn::Right] {
            for (b, (dx, dy)) in [
                (Up, (0i16, -1i16)),
                (Down, (0, 1)),
                (Left, (-1, 0)),
                (Right, (1, 0)),
            ] {
                let from = t.point((240, 320));
                let to = t.point(((240 + dx) as u16, (320 + dy) as u16));
                let moved = (to.0 - from.0, to.1 - from.1);
                let want = match t.key(b) {
                    k if k == u => (0, -1),
                    k if k == d => (0, 1),
                    k if k == l => (-1, 0),
                    _ => (1, 0),
                };
                assert_eq!(moved, want, "{t:?} {b:?}");
            }
        }
        // Every button has a code of its own, the same in every turn but
        // for the D-pad's four.
        for t in [Turn::None, Turn::Left, Turn::Right] {
            let mut codes: Vec<u8> = Button::ALL.iter().map(|&b| t.key(b)).collect();
            codes.sort_unstable();
            assert_eq!(codes, (1..=11).collect::<Vec<u8>>(), "{t:?}");
            for b in [Enter, Calendar, Contacts, Mail, Home, Record, Wifi] {
                assert_eq!(t.key(b), Turn::None.key(b));
            }
        }
        assert_eq!(Turn::from_c(0), Some(Turn::None));
        assert_eq!(Turn::from_c(1), Some(Turn::Left));
        assert_eq!(Turn::from_c(2), Some(Turn::Right));
        assert_eq!(Turn::from_c(3), None);
    }

    #[test]
    fn presses_and_releases_arrive_in_order_with_their_time() {
        let mut i = input();
        assert_eq!(i.poll(0), Err(Errno::BadHandle), "not open");
        // Held before the game opened its input: not a press.
        i.pad().pressed = Pressed::NONE.with(Button::Mail);
        assert_eq!(i.open(Turn::Left, 1_000), Ok(()));
        assert_eq!(i.open(Turn::None, 1_000), Err(Errno::Busy));
        assert_eq!(drain(&mut i, 2_000), []);
        // Up and Home go down together, Mail comes up.
        i.pad().pressed = Pressed::NONE.with(Button::Up).with(Button::Home);
        let got = drain(&mut i, 10_000);
        assert_eq!(
            got,
            [
                Event::key(event::KEY_LEFT, true, 10_000),
                Event::key(event::KEY_MAIL, false, 10_000),
                Event::key(event::KEY_HOME, true, 10_000),
            ]
        );
        i.pad().pressed = Pressed::NONE;
        assert_eq!(
            drain(&mut i, 20_000),
            [
                Event::key(event::KEY_LEFT, false, 20_000),
                Event::key(event::KEY_HOME, false, 20_000),
            ]
        );
        assert_eq!(drain(&mut i, 30_000), []);
        assert_eq!(i.dropped(), 0);
    }

    #[test]
    fn a_tap_and_a_drag_arrive_in_the_game_s_pixels() {
        for (turn, at, to) in [
            (Turn::None, (100, 200), (300, 400)),
            (Turn::Left, (200, 379), (400, 179)),
            (Turn::Right, (439, 100), (239, 300)),
        ] {
            let mut i = input();
            assert_eq!(i.open(turn, 0), Ok(()));
            let mut now = 0;
            // The stylus lands at upright (100, 200): one Down, there.
            let got = hold(&mut i, 100, 200, &mut now);
            assert_eq!(got.len(), 1, "{turn:?}: {got:?}");
            assert_eq!(
                (got[0].kind, got[0].code, got[0].x, got[0].y),
                (event::PEN, event::PEN_DOWN, at.0, at.1),
                "{turn:?}"
            );
            // (the helper waits 20 quiet readings, 100 ms, after it)
            assert_eq!(got[0].time_us, now - 100_000, "read when it was reported");
            // Dragged to upright (300, 400): Moves, the last one there.
            let got = hold(&mut i, 300, 400, &mut now);
            let last = got.last().expect("a move");
            assert_eq!(
                (last.kind, last.code, last.x, last.y),
                (event::PEN, event::PEN_MOVE, to.0, to.1),
                "{turn:?}: {got:?}"
            );
            // Lifted: one Up, where it last was.
            i.pad().pen = None;
            let mut ups = Vec::new();
            for _ in 0..20 {
                now += 5_000;
                ups.extend(drain(&mut i, now));
            }
            assert_eq!(ups.len(), 1, "{turn:?}: {ups:?}");
            assert_eq!(
                (ups[0].code, ups[0].x, ups[0].y),
                (event::PEN_UP, to.0, to.1),
                "{turn:?}"
            );
        }
    }

    /// R355: a slow steady drag with the panel's noise on it reaches the
    /// game as small even steps (never back, never a jump), and a stylus
    /// held still under the same noise says nothing.
    #[test]
    fn a_noisy_slow_drag_arrives_as_small_even_steps_and_a_held_stylus_is_quiet() {
        // the X51v's own calibration: a pixel is 5.6 to 7.5 raw units
        let mut i = Input::new(Fake::default(), Calibration::X51V_DEFAULT);
        assert_eq!(i.open(Turn::None, 0), Ok(()));
        let mut now = 0;
        let down = hold(&mut i, 1_500, 2_000, &mut now);
        assert_eq!(down.len(), 1, "{down:?}");
        let Some(base) = touch(1_500, 2_000) else {
            panic!("a touch")
        };
        // noise of +-3 raw units (under half a pixel) on every reading
        let noise = |k: u32| (k.wrapping_mul(2_654_435_761) >> 29) as i32 - 3;
        // held still for a second: nothing said
        for k in 0..200u32 {
            now += 5_000;
            i.pad().pen = Some(Sample {
                x: (i32::from(base.x) + noise(k)) as u16,
                y: (i32::from(base.y) + noise(k + 7)) as u16,
                z1: base.z1,
            });
            assert_eq!(drain(&mut i, now), [], "held still, reading {k}");
        }
        // dragged along raw x, two units a reading (about 60 pixels a
        // second), for two seconds
        let mut xs = Vec::new();
        for k in 0..400u32 {
            now += 5_000;
            i.pad().pen = Some(Sample {
                x: (i32::from(base.x) + 2 * k as i32 + noise(k)) as u16,
                y: (i32::from(base.y) + noise(k + 7)) as u16,
                z1: base.z1,
            });
            for e in drain(&mut i, now) {
                assert_eq!(e.code, event::PEN_MOVE);
                xs.push((e.x, e.y));
            }
        }
        assert!(
            xs.len() >= 30,
            "a move for nearly every pixel passed: {}",
            xs.len()
        );
        for w in xs.windows(2) {
            let (dx, dy) = (w[1].0 - w[0].0, w[1].1 - w[0].1);
            assert!(dx.abs() <= 1 && dy.abs() <= 1, "a step of one pixel: {w:?}");
        }
        // one way along the axis that moves, never back
        let end = xs[xs.len() - 1];
        let along: Vec<i16> = if (end.0 - xs[0].0).abs() < (end.1 - xs[0].1).abs() {
            xs.iter().map(|p| p.1).collect()
        } else {
            xs.iter().map(|p| p.0).collect()
        };
        let up = along[along.len() - 1] > along[0];
        for w in along.windows(2) {
            assert!(
                if up { w[1] >= w[0] } else { w[1] <= w[0] },
                "never back: {w:?}"
            );
        }
    }

    #[test]
    fn a_game_that_does_not_poll_keeps_the_newest_events_and_the_count_of_the_lost() {
        let mut i = input();
        assert_eq!(i.open(Turn::None, 0), Ok(()));
        // 40 changes seen by the clock's pump, never polled: 32 are kept.
        for n in 0..40u64 {
            i.pad().pressed = if n % 2 == 0 {
                Pressed::NONE.with(Button::Enter)
            } else {
                Pressed::NONE
            };
            i.pump((n + 1) * 10_000);
        }
        assert_eq!(i.dropped(), 8);
        let got = drain(&mut i, 400_001);
        assert_eq!(got.len(), QUEUE);
        // The oldest kept is the ninth change, the newest the fortieth.
        assert_eq!(got[0], Event::key(event::KEY_ENTER, true, 90_000));
        assert_eq!(got[QUEUE - 1], Event::key(event::KEY_ENTER, false, 400_000));
        // The pump before the input is open reads nothing.
        let mut i = input();
        i.pump(1_000_000);
        assert_eq!(i.pad().reads, 0);
    }

    #[test]
    fn the_pad_is_not_read_more_often_than_its_interval() {
        let mut i = input();
        assert_eq!(i.open(Turn::None, 0), Ok(()));
        let opened = i.pad().reads;
        // Pumps closer than the interval: no reading.
        i.pump(READ_EVERY_US - 1);
        assert_eq!(i.pad().reads, opened);
        i.pump(READ_EVERY_US);
        assert_eq!(i.pad().reads, opened + 1);
        // A poll inside the interval reads nothing again, queue empty or
        // not (run 316: a game's drain loop read the pad at every turn).
        assert_eq!(i.poll(READ_EVERY_US + 1), Ok(None));
        assert_eq!(i.pad().reads, opened + 1);
        // Once the interval is over a poll reads, and hands out what it
        // read without reading again.
        i.pad().pressed = Pressed::NONE.with(Button::Up).with(Button::Down);
        assert!(i.poll(2 * READ_EVERY_US).unwrap().is_some());
        let reads = i.pad().reads;
        assert_eq!(reads, opened + 2);
        assert!(i.poll(2 * READ_EVERY_US + 1).unwrap().is_some());
        assert_eq!(i.pad().reads, reads);
    }

    /// Run 316's class guard: a game that drains its events in a loop
    /// while the stylus moves gets out of the loop: one reading of the pad
    /// for the whole drain, however many polls it takes.
    #[test]
    fn a_drain_loop_under_a_moving_stylus_reads_the_pad_once() {
        let mut i = Input::new(Fake::default(), Calibration::X51V_DEFAULT);
        assert_eq!(i.open(Turn::None, 0), Ok(()));
        let mut now = 0;
        let down = hold(&mut i, 1_500, 2_000, &mut now);
        assert_eq!(down.len(), 1, "{down:?}");
        for k in 1..=100u16 {
            now += 5_000;
            // the stylus somewhere new at every look at the pad
            i.pad().pen = touch(1_500 + 8 * k, 2_000);
            let before = i.pad().reads;
            let mut polls = 0;
            while i.poll(now).unwrap().is_some() {
                polls += 1;
                assert!(polls < 8, "the drain goes on at reading {k}");
                i.pad().pen = touch(1_500 + 8 * k + polls, 2_000);
            }
            assert_eq!(i.pad().reads, before + 1, "one reading a drain");
        }
    }
}
