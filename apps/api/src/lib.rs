//! The interface between the shell and an app program, and the shared app
//! kit of [design-language.md](../../../docs/design/design-language.md)
//! version 2.
//!
//! The owner, 2026-11-14: every app is its own program, loaded when opened
//! and released when left. The shell keeps the title bar and the softkeys
//! (the frame every screen shares) and hands the app the content area
//! ([`CONTENT`]), the fonts, the look, the clock and the picture store
//! ([`Ctx`]). The app draws into a [`Frame`] and answers [`Event`]s
//! ([`Program`]). Nothing here knows how the app got into memory: today the
//! shell holds it directly, next the program is loaded from internal
//! storage behind the same trait, and under the kernel it becomes a
//! process whose calls cross IPC (ADR-0016) with the same shapes.
//!
//! Iterate, don't imitate (rule 0): Windows Mobile's apps each drew their
//! own window through GWES and the menu bar was theirs; Android hands an
//! app the whole screen. Ours hands it only its area and keeps the bars,
//! so every app gets the same Start, clock and softkeys for free and an
//! app can't break them; the kit makes the version 2 look one call each,
//! so five apps cost one look.

#![no_std]
#![forbid(unsafe_code)]

pub mod aurora;
pub mod kit;
pub mod timepad;

pub use rp_draw::pics::{Pic, Pics};
pub use rp_draw::{Argb, Cmd, Font, Frame, PanelArt, Rect};

/// The screen.
pub const W: u16 = 480;
/// The screen's height.
pub const H: u16 = 640;
/// The app's area: under the title bar, above the softkey bar.
pub const CONTENT: Rect = Rect::new(0, 36, W, 564);
/// The side margin and the width inside it.
pub const MARGIN: u16 = 16;
/// The content's width inside the margins.
pub const WIDE: u16 = W - 2 * MARGIN;

/// The fonts the shell lends (design-language.md "Type").
#[derive(Clone, Copy)]
pub struct Fonts {
    /// Didact Gothic 18 px: everything read as words.
    pub ui: &'static Font,
    /// Spaced capitals: section labels, chips, weekdays.
    pub label: &'static Font,
    /// The big light numerals.
    pub big: &'static Font,
    /// 36 px numerals and signs (the calculator).
    pub num: &'static Font,
    /// 30 px: the large title, a contact's name.
    pub title: &'static Font,
}

/// The look the shell lends: light or dark and the glass's art.
#[derive(Clone, Copy)]
pub struct Look {
    /// The light look is on.
    pub light: bool,
    /// The glass panel.
    pub panel: &'static PanelArt,
    /// The glass panel under the pen.
    pub pressed: &'static PanelArt,
    /// An Aero glass key or chip (8 px corners, the gloss line).
    pub key: &'static PanelArt,
    /// A key under the pen.
    pub key_down: &'static PanelArt,
    /// The chosen chip, the main key: the blue glow glass.
    pub on: &'static PanelArt,
}

/// The wall clock as days and minutes, in the device's zone.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Now {
    /// Days since 1 January 1970.
    pub day: i32,
    /// Minutes since midnight.
    pub minute: u16,
}

/// What the app is given each call.
#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    /// The fonts.
    pub fonts: &'a Fonts,
    /// The look.
    pub look: &'a Look,
    /// The clock.
    pub now: Now,
    /// The picture store the renderer draws from (avatars, marks), if any.
    pub pics: Option<&'a Pics<'static>>,
    /// The forecast the Weather app already holds, by day (the calendar's
    /// days show it: no fetch of the app's own).
    pub forecast: &'a [DayWeather],
    /// The region writes dates month first (the US) and starts the week on
    /// Sunday.
    pub us: bool,
    /// The device's offset from UTC, minutes (a time kept in UTC, as an
    /// online calendar's, is moved by it).
    pub utc_offset: i32,
    /// The rounded shapes' corner pictures, painted once per colour and
    /// kept by the shell ([`Shapes`]).
    pub shapes: &'a Shapes,
    /// The clipboard every program shares (Copy puts text on it; the
    /// keyboard's Paste types it into the field being typed).
    pub clip: &'a core::cell::RefCell<Text<64>>,
    /// The organiser's one store (docs/design/pim-store.md), when it is
    /// given and loaded: read while drawing, changed while answering.
    pub pim: Option<&'a core::cell::RefCell<Option<rp_pim::store::Store<'static>>>>,
}

/// One day of the forecast, in the units the user chose.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DayWeather {
    /// Days since 1 January 1970.
    pub day: i32,
    /// The high.
    pub high: i16,
    /// The low.
    pub low: i16,
    /// The weather's icon at row size, if the store holds it.
    pub icon: Option<Pic>,
}

/// The keys an app sees (Home and Start stay the shell's).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    /// D-pad up.
    Up,
    /// D-pad down.
    Down,
    /// D-pad left.
    Left,
    /// D-pad right.
    Right,
    /// D-pad centre.
    Enter,
    /// The left softkey.
    SoftLeft,
    /// The right softkey.
    SoftRight,
    /// The app's own program button, pressed while the app is open (the
    /// Calendar button in Calendar: Windows Mobile's cycles its views).
    App,
}

/// One event for the app.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Event {
    /// The stylus touched its area.
    PenDown {
        /// Screen x.
        x: u16,
        /// Screen y.
        y: u16,
    },
    /// The stylus moved while down.
    PenMove {
        /// Screen x.
        x: u16,
        /// Screen y.
        y: u16,
    },
    /// The stylus left the screen.
    PenUp {
        /// Screen x.
        x: u16,
        /// Screen y.
        y: u16,
    },
    /// A key went down.
    Key(Key),
    /// The minute changed, or motion the app started wants its next step.
    Tick,
    /// The keyboard typed a character (ASCII; capitals as the field's
    /// kind decided).
    Char(u8),
    /// The keyboard's delete.
    Delete,
    /// The keyboard's Done (or Return in a sentence field).
    Done,
    /// The keyboard strip's suggestion `n` was tapped.
    Suggestion(u8),
}

/// What a text field holds: the keyboard follows it (design-language.md
/// "The keyboard knows the field").
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    /// A name, a title, a place: capitals at each word.
    Name,
    /// An address line: capitals at each word, numbers in reach.
    Address,
    /// Sentences: capitals at each sentence.
    Sentence,
    /// A phone number: the number pad, formatted as typed.
    Phone,
    /// An e-mail address: `@` and `.com`, no capitals.
    Email,
    /// A number: the pad with `.` and `-`.
    Number,
    /// A link: `/ . :`, no capitals.
    Link,
    /// A typed line read by a parser (quick add, search): no capitals.
    Line,
}

/// What the shell should do after an event.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reply {
    /// Nothing changed.
    Nothing,
    /// Draw a frame.
    Redraw,
    /// Draw a frame, and send a Tick about 30 ms later (motion).
    Animate,
    /// The app is done: back to where it was opened from.
    Close,
}

/// The corner pictures of rounded shapes (`kit::round_rect`): a 24 px
/// disc per colour, painted once into the picture store and kept for every
/// app that draws that colour.
pub struct Shapes {
    discs: core::cell::RefCell<[(Argb, Option<Pic>); 32]>,
    /// The look's colour for a fill (the shell's light look maps every
    /// colour a program draws; a corner picture is painted in the colour
    /// its fills will have, so a shape stays one colour edge to edge).
    paint: core::cell::Cell<fn(Argb) -> Argb>,
}

impl Shapes {
    /// None painted yet.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            discs: core::cell::RefCell::new([(0, None); 32]),
            paint: core::cell::Cell::new(same),
        }
    }

    /// Forgets every disc (their pictures were given back: an app program
    /// closed).
    pub fn forget(&self) {
        if let Ok(mut d) = self.discs.try_borrow_mut() {
            *d = [(0, None); 32];
        }
    }

    /// The look's fill colour, set by the shell before a program draws
    /// (the identity in the dark look).
    pub fn set_paint(&self, paint: fn(Argb) -> Argb) {
        self.paint.set(paint);
    }

    /// The disc of colour `c` (in the look's paint), painted the first
    /// time it is asked for.
    pub fn disc(&self, pics: &Pics<'static>, c: Argb) -> Option<Pic> {
        let c = (self.paint.get())(c);
        let mut d = self.discs.try_borrow_mut().ok()?;
        if let Some((_, p)) = d.iter().find(|(k, p)| *k == c && p.is_some()) {
            return *p;
        }
        let free = d.iter().position(|(_, p)| p.is_none())?;
        let p = kit::flat_disc(pics, kit::CORNER * 2, c);
        d[free] = (c, p);
        p
    }
}

/// A colour as it is (the dark look's paint).
fn same(c: Argb) -> Argb {
    c
}

impl Default for Shapes {
    fn default() -> Self {
        Self::new()
    }
}

/// A short text kept with a list's entry, cut to `N` bytes: a store's own
/// bytes can't be held across a change, so views copy what they show.
#[derive(Clone, Copy, Debug)]
pub struct Text<const N: usize> {
    b: [u8; N],
    n: usize,
}

impl<const N: usize> Text<N> {
    /// No text.
    pub const EMPTY: Self = Self { b: [0; N], n: 0 };

    /// `s`, cut to `N` bytes.
    #[must_use]
    pub fn of(s: &[u8]) -> Self {
        let mut t = Self::EMPTY;
        let n = s.len().min(N);
        t.b[..n].copy_from_slice(&s[..n]);
        t.n = n;
        t
    }

    /// The bytes.
    #[must_use]
    pub fn get(&self) -> &[u8] {
        &self.b[..self.n]
    }

    /// Nothing in it.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.n == 0
    }
}

/// What the shell hands an app program's entry when it opens it, and the
/// program it hands back (`rp_hal_user::app_entry!`).
///
/// The shell and the program are built from the same sources by the same
/// compiler in one `cargo xtask` run, so the types they share have one
/// layout; [`Opening::layout`] is checked anyway, and a program built
/// apart from its shell refuses to open rather than read a wrong shape.
pub struct Opening {
    /// The shell's [`Opening::layout`].
    pub layout: [u32; 6],
    /// The program's own, written when it refuses (the shell logs both).
    pub theirs: [u32; 6],
    /// Today, days since 1970 (the app's first screen).
    pub today: i32,
    /// What the opening is for: [`INTENT_NEW`] (the Today screen's Note
    /// button: a new note at once), else 0.
    pub intent: u8,
    /// The program, when the entry agreed to open.
    pub program: Option<&'static mut dyn Program>,
}

impl Opening {
    /// The sizes of the types the shell and a program share.
    #[must_use]
    pub const fn layout() -> [u32; 6] {
        [
            core::mem::size_of::<Frame>() as u32,
            core::mem::size_of::<Ctx>() as u32,
            core::mem::size_of::<Event>() as u32,
            core::mem::size_of::<Fonts>() as u32,
            core::mem::size_of::<rp_draw::Cmd>() as u32,
            core::mem::size_of::<Look>() as u32,
        ]
    }

    /// An opening record for `today`.
    #[must_use]
    pub const fn new(today: i32) -> Self {
        Self {
            layout: Self::layout(),
            theirs: [0; 6],
            today,
            intent: 0,
            program: None,
        }
    }

    /// Whether the shell's shapes are this program's; when not, the
    /// program's own are written for the shell to log.
    pub fn fits(&mut self) -> bool {
        self.theirs = Self::layout();
        self.layout == self.theirs
    }
}

/// A question an app puts to the network (the shell sends it over
/// whatever connection there is, one at a time): what, by name, and its
/// argument. The app names what it wants, never an address (security.md:
/// the relay, or later the device's own client, maps each name to a fixed
/// service).
#[derive(Clone, Copy, Debug)]
pub struct NetAsk {
    /// `ADDR` (address suggestions), `RATES` (money), `ICAL` (a calendar
    /// link by its number).
    pub what: &'static [u8],
    /// Its argument: the typed text, a number.
    pub arg: Text<64>,
}

/// [`Opening::intent`]: start something new at once.
pub const INTENT_NEW: u8 = 1;

/// An app program.
pub trait Program {
    /// The name in the title bar.
    fn title(&self) -> &'static str;
    /// The softkeys' words (left, right); empty for none.
    fn softkeys(&self) -> (&'static str, &'static str);
    /// Draw the content area into `f` (the shell has drawn the bars, and
    /// the app's own window is under it: no wallpaper, R201).
    fn draw(&mut self, f: &mut Frame, cx: &Ctx);
    /// Answer an event.
    fn event(&mut self, e: Event, cx: &Ctx) -> Reply;
    /// The app's accent (its window may be tinted by it); none by default.
    fn accent(&self) -> Option<Argb> {
        None
    }
    /// The keyboard to show (docked above the softkeys, 208 px), if a
    /// field is being typed, and up to three suggestions for its strip.
    fn keyboard(&self) -> Option<Field> {
        None
    }
    /// The keyboard strip's suggestions for the field being typed.
    fn suggestions(&self) -> [&[u8]; 3] {
        [&[], &[], &[]]
    }
    /// A question for the network, if the app has one (taken: asked once).
    fn net_ask(&mut self) -> Option<NetAsk> {
        None
    }
    /// The answer to the app's last question (`ok` false: no connection or
    /// the service refused).
    fn net_answer(&mut self, ok: bool, body: &[u8], cx: &Ctx) -> Reply {
        let _ = (ok, body, cx);
        Reply::Nothing
    }
    /// A question for the device's own card, if the app has one (taken):
    /// `CARDFILES` (the files of a kind: `vcf`, `ics`, `pic`, as "n|name"
    /// lines), `CARDFILE` ("n kind": file n's bytes), `CARDPUT` ("length
    /// name": the body, [`Program::net_body`], kept as the card's
    /// `RetroPane\name`). Answered through [`Program::net_answer`], as the
    /// network's are; the card needs no dock.
    fn card_ask(&mut self) -> Option<NetAsk> {
        None
    }
    /// The body of the question in flight that sends one (the network's
    /// `PUT`, the card's `CARDPUT`): its bytes from `at` into `out`, how
    /// many (0: no more). Asked in order, `at` rising, so an app writes
    /// a file a piece at a time and never holds it whole.
    fn net_body(&mut self, at: usize, out: &mut [u8], cx: &Ctx) -> usize {
        let _ = (at, out, cx);
        0
    }
}
