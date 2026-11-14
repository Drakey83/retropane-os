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

pub mod kit;

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

/// An app program.
pub trait Program {
    /// The name in the title bar.
    fn title(&self) -> &'static str;
    /// The softkeys' words (left, right); empty for none.
    fn softkeys(&self) -> (&'static str, &'static str);
    /// Draw the content area into `f` (the shell has drawn the bars and
    /// the wallpaper shows behind).
    fn draw(&mut self, f: &mut Frame, cx: &Ctx);
    /// Answer an event.
    fn event(&mut self, e: Event, cx: &Ctx) -> Reply;
    /// The keyboard to show (docked above the softkeys, 208 px), if a
    /// field is being typed, and up to three suggestions for its strip.
    fn keyboard(&self) -> Option<Field> {
        None
    }
    /// The keyboard strip's suggestions for the field being typed.
    fn suggestions(&self) -> [&[u8]; 3] {
        [&[], &[], &[]]
    }
}
