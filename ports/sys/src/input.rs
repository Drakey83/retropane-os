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
        }
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
        self.first = 0;
        self.len = 0;
        self.read_at = now_us;
        Ok(())
    }

    fn poll(&mut self, now_us: u64) -> Result<Option<Event>, Errno> {
        let turn = self.turn.ok_or(Errno::BadHandle)?;
        if self.len == 0 || now_us.saturating_sub(self.read_at) >= READ_EVERY_US {
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
    /// until the filter has reported something; the events.
    fn hold(i: &mut Input<Fake>, x: u16, y: u16, now: &mut u64) -> Vec<Event> {
        i.pad().pen = touch(x, y);
        let mut out = Vec::new();
        for _ in 0..40 {
            *now += 5_000;
            out.extend(drain(i, *now));
            if !out.is_empty() {
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
            assert_eq!(got[0].time_us, now, "read when it was reported");
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
        // A poll with nothing queued always reads (the game asks now).
        assert_eq!(i.poll(READ_EVERY_US + 1), Ok(None));
        assert_eq!(i.pad().reads, opened + 2);
        // With events queued and the interval not over, it hands them out
        // without reading again.
        i.pad().pressed = Pressed::NONE.with(Button::Up).with(Button::Down);
        assert!(i.poll(READ_EVERY_US + 2).unwrap().is_some());
        let reads = i.pad().reads;
        assert!(i.poll(READ_EVERY_US + 3).unwrap().is_some());
        assert_eq!(i.pad().reads, reads);
    }
}
