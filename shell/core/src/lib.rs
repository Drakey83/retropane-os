//! The shell (ADR-0021): the Today screen, the title bar with Start, the
//! Start menu and the two softkeys (docs/design/ui.md), in the smoked-glass
//! look the owner chose.
//!
//! It owns layout, focus, touch and D-pad handling and what changed; it
//! draws nothing itself. Each frame it fills an `rp_draw::Frame` (dirty
//! rectangles plus the whole scene) for whatever renderer is underneath:
//! the CPU renderer now, the MBX one after item 69, the kernel's display
//! service later. It depends on `rp-draw` only, so it can't reach pixels.
//!
//! Input is stylus and D-pad equally (owner's rule): every target can be
//! tapped and reached with the D-pad, a focus ring shows where the D-pad
//! is, Enter acts. Keys keep their Windows Mobile jobs: Home goes to the
//! Today screen.
//!
//! Placeholders say so: weather is the sample forecast (`rp-weather`) until
//! there is a network, and entries without a program yet show "Not yet" in
//! the softkey bar.
//!
//! Two screens so far: the Today screen (the home, the weather hero on top)
//! and the Weather app ([`weather_app`]), which the hero opens; both read
//! the same forecast.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod align;
pub mod apps;
mod caps;
pub mod chart;
mod keyboard;
pub mod motion;
pub mod panel;
mod panel_art;
pub mod sky;
pub mod station;
mod today_feed;
pub mod weather_app;
mod weather_net;
mod weather_settings;
pub use weather_net::{Ask, Question};

pub use weather_app::{LAYER_MARGIN, Motion, PageView, picture_rows};

use rp_draw::pics::Pic;
use rp_draw::{Argb, Cmd, Font, Frame, Image, Rect};
pub use rp_touch_cal::Calibration;
use rp_weather::{Forecast, Icon, Line, Region};

/// Screen width.
pub const W: u16 = 480;
/// Screen height.
pub const H: u16 = 640;

/// The fonts the shell draws with (made on the PC at build time).
pub struct Fonts {
    /// Menus, softkeys, small text.
    pub ui: &'static Font,
    /// Spaced capitals for labels and dates.
    pub label: &'static Font,
    /// The big light numerals (the clock, the temperature).
    pub big: &'static Font,
    /// Numerals at 36 px with their signs (`0-9 . , - + x / = %`): the
    /// calculator's display and keys (docs/design/design-language.md).
    pub num: &'static Font,
    /// 30 px: an app's large title (design-language.md version 2).
    pub title: &'static Font,
}

/// A size the shell shows a weather picture at: a class of slots in the
/// device's picture store (option C, docs/design/weather-rule0.md).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Size {
    /// The hero panel's and the Now page's (250 px).
    Hero,
    /// Hourly's card ([`weather_app::HOUR_PIC_W`]).
    Mid,
    /// The forecast cards' (66 px).
    Card,
    /// The Weather app's rows (40 px).
    Row,
}

impl Size {
    /// Every size, in [`Wanted`]'s order.
    pub const ALL: [Self; 4] = [Self::Hero, Self::Mid, Self::Card, Self::Row];

    /// Its side in px: the largest picture of the size (its store slots').
    #[must_use]
    pub const fn side(self) -> u16 {
        match self {
            Self::Hero => 250,
            Self::Mid => weather_app::HOUR_PIC_W,
            Self::Card => 66,
            Self::Row => 40,
        }
    }

    const fn at(self) -> usize {
        self as usize
    }
}

/// The weather pictures: handles to the device's picture store
/// (`rp_draw::pics`), decoded at run time at each size from the pack's big
/// pictures. The shell holds handles only, never pixels (ADR-0021 rule 1);
/// a missing one is drawn as nothing. Indexed by pack id less one
/// ([`rp_weather::Icon::id`]).
#[derive(Clone, Copy)]
pub struct Icons {
    /// The big ones (250 px) in the hero panel and the app's Now page.
    pub hero: [Option<Pic>; 22],
    /// Hourly's card ([`weather_app::HOUR_PIC_W`]).
    pub mid: [Option<Pic>; 22],
    /// The forecast cards' size (66 px).
    pub card: [Option<Pic>; 22],
    /// The Weather app's rows (40 px).
    pub row: [Option<Pic>; 22],
    /// The radar's rain cells ([`rp_weather::radar::CELLS`]), made at start.
    pub radar: [Option<&'static Image>; 3],
}

impl Icons {
    /// The big picture for `icon`: its own, else its card size (until the
    /// big one is decoded), else none.
    #[must_use]
    pub fn big(&self, icon: Icon) -> Option<Pic> {
        let i = usize::from(icon.id() - 1);
        self.hero[i].or(self.card[i])
    }

    /// The pictures of `size`.
    #[must_use]
    pub const fn of(&self, size: Size) -> &[Option<Pic>; 22] {
        match size {
            Size::Hero => &self.hero,
            Size::Mid => &self.mid,
            Size::Card => &self.card,
            Size::Row => &self.row,
        }
    }

    /// The pictures of `size`, to change.
    pub const fn of_mut(&mut self, size: Size) -> &mut [Option<Pic>; 22] {
        match size {
            Size::Hero => &mut self.hero,
            Size::Mid => &mut self.mid,
            Size::Card => &mut self.card,
            Size::Row => &mut self.row,
        }
    }

    /// No pictures.
    pub const NONE: Self = Self {
        hero: [None; 22],
        mid: [None; 22],
        card: [None; 22],
        row: [None; 22],
        radar: [None; 3],
    };
}

/// The pictures the screens show with the forecast in hand: a bit per
/// condition (pack id less one) per [`Size`]. The device decodes these
/// (and gives the rest back) when the forecast, the pack or the place
/// changes, never during touch (P1, P2: docs/design/principles.md).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Wanted(pub [u32; 4]);

impl Wanted {
    /// Whether `icon` (pack id less one) at `size` is wanted.
    #[must_use]
    pub const fn has(&self, size: Size, icon: usize) -> bool {
        icon < 22 && self.0[size.at()] >> icon & 1 != 0
    }

    /// `icon` (pack id less one) at `size` wanted.
    pub const fn add(&mut self, size: Size, icon: usize) {
        if icon < 22 {
            self.0[size.at()] |= 1 << icon;
        }
    }

    /// How many pictures are wanted.
    #[must_use]
    pub const fn count(&self) -> u32 {
        self.0[0].count_ones()
            + self.0[1].count_ones()
            + self.0[2].count_ones()
            + self.0[3].count_ones()
    }
}

/// A colour's light twin: a neutral one (its channels within 40 of each
/// other: glass, bars, text, edges) mirrored in lightness, alpha kept; any
/// other (an accent) as it is.
#[must_use]
pub const fn light_twin(c: Argb) -> Argb {
    let (r, g, b) = ((c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
    let max = if r > g {
        if r > b { r } else { b }
    } else if g > b {
        g
    } else {
        b
    };
    let min = if r < g {
        if r < b { r } else { b }
    } else if g < b {
        g
    } else {
        b
    };
    if max - min > 40 {
        return c;
    }
    // Mirrored onto 0-196, not 0-255: the glass a mid light grey rather
    // than near white, which washed out the lettering and the pictures
    // (the owner's run, 2026-11-01: "a little too bright ... make it a
    // little darker", his pick A; 0-224 was "a little better but ... still
    // too bright").
    (c & 0xFF00_0000)
        | ((255 - r) * 196 / 255) << 16
        | ((255 - g) * 196 / 255) << 8
        | ((255 - b) * 196 / 255)
}

/// How the light look keeps text readable over the photo (the owner's run,
/// 2026-11-01: grey glass was "still too bright" and "nothing is super easy
/// to see"): three approaches drawn for his pick.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LightStyle {
    /// The grey glass of the first two runs (neutrals mirrored onto 0-196).
    Grey,
    /// A: iOS's way: near-opaque frosted white glass, near-black text.
    Frosted,
    /// B: Material You's way: near-opaque glass tinted from the wallpaper
    /// (the sunset's rose).
    Tonal,
    /// C: frosted glass, and the wallpaper itself veiled light between the
    /// panels.
    Washed,
    /// Inside an app's own window, over its light backdrop
    /// (design-language.md version 3): the white window glass, the accents'
    /// window twins. Chosen by the shell where a window is up, not a
    /// setting.
    Window,
}

/// An app's own window: what fills it behind the program, so no
/// wallpaper shows through (the owner, 2026-11-14: "full screen into
/// their own full sized window, so no wallpaper behind them", run the way
/// a full-screen Windows Mobile program does). Three drawn for his pick.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WindowBody {
    /// Windows Mobile's way: one flat window colour.
    Solid,
    /// Aero's frame: the title bar's slate gradient carried down the
    /// window, left to right.
    Aero,
    /// Aero's frame tinted by the app: its accent mixed a little into the
    /// slate (Calendar's coral, Contacts' teal), so each window is its own.
    Tinted,
}

/// Where Today's tasks line goes (tasks-rule0.md: "Today screen's tasks
/// line fed from the store"); three drawn for the owner's pick
/// (R374 in docs/reminders.md), A the default until then.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TodayTasks {
    /// A: on the calendar panel's top line, right of "TODAY": "3 TASKS
    /// DUE · 1 LATE". The panel keeps its two appointment rows.
    Header,
    /// B: the panel's second row: the count, the most pressing task's
    /// title and "+2". One appointment row is left.
    Row,
    /// C: a third row under the two appointments, the panel 28 px taller
    /// (into the gap over the quick buttons).
    Extra,
}

/// The washed wallpaper's veil (C), drawn first, kept by the look.
pub(crate) const WASH: Argb = 0x9CF4_F2F6;

/// The frosted glass's opacity in the light look.
pub(crate) const LIGHT_GLASS_ALPHA: u32 = 0xB8;

/// A neutral fill's colour in the light look's `style`; accents kept.
#[must_use]
pub const fn light_glass(c: Argb, style: LightStyle) -> Argb {
    // (Dark glass's channels are under 0x80: lifted to 0xE0-0xFF, tinted.)
    let t = light_twin(c);
    if t == c {
        return light_accent_fill(c);
    }
    if matches!(style, LightStyle::Grey) {
        return t;
    }
    let (r, g, b) = ((c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
    let a = c >> 24;
    // Dark glass (the panels) to near-opaque light glass; light lines
    // (edges, dividers) to soft dark ones.
    if (r + g + b) / 3 < 0x80 {
        let (tr, tg, tb) = match style {
            // (Soft, not white: 250 and 244-250 before the owner's
            // "still seems too blinding" of 2026-11-15; the keyboard was
            // the brightest thing left, 68% of white's light.)
            LightStyle::Tonal => (204, 192, 198),
            _ => (198, 202, 209),
        };
        // As see-through as every word's 4.5:1 allows over the darkest
        // wallpaper (the owner, 2026-11-01: readable, "but there is no
        // transparence at all, is there a middle ground"): 72%, found by
        // the light look's contrast test (66% left the live green at
        // 4.43:1).
        // In proportion: the dark glass's opacity (0xC8 and over) to the
        // light's, a fading edge's lower ones scaled the same.
        let a = if a >= 0xC8 {
            LIGHT_GLASS_ALPHA
        } else {
            a * LIGHT_GLASS_ALPHA / 0xC8
        };
        a << 24
            | ((255 - r / 4) * tr / 255) << 16
            | ((255 - g / 4) * tg / 255) << 8
            | ((255 - b / 4) * tb / 255)
    } else {
        (a / 2) << 24 | 0x0030_3640
    }
}

/// A colour in the light look's `style` (`text` for lettering): the one
/// rule for the shell's screens and the hosted programs' (the owner,
/// 2026-11-14: design for both looks, everything cohesive).
#[must_use]
pub fn look_color(c: Argb, text: bool, style: LightStyle) -> Argb {
    if sky::is_ink(c)
        || panel::is_panel(c)
        || c == WASH
        || c == keyboard::DONE_TEXT
        || c == weather_app::SHADOW_LIGHT
        || rp_app::kit::kept(c)
    {
        // The living sky's own colours, chosen for each look, the washed
        // wallpaper's veil, the dark text on an accent fill (a lit switch,
        // Done), which stays light in both looks (mirrored it was light
        // grey on light blue, 1.4:1, the light look's contrast test), and
        // a program's kept colours (paper and its ink).
        c
    } else if let Some(t) = if matches!(style, LightStyle::Window) {
        rp_app::kit::window_twin(c, text)
    } else {
        rp_app::kit::light_twin(c, text)
    } {
        // An accent: its computed twin (hue and chroma kept) for the
        // glass it sits on.
        t
    } else if text {
        light_text(c)
    } else {
        light_glass(c, style)
    }
}

/// The fill colour a program's corner pictures are painted in, per light
/// style (a function, not a closure: `rp_app::Shapes` keeps a plain `fn`).
const LIGHT_FILLS: [fn(Argb) -> Argb; 5] = [
    |c| look_color(c, false, LightStyle::Grey),
    |c| look_color(c, false, LightStyle::Frosted),
    |c| look_color(c, false, LightStyle::Tonal),
    |c| look_color(c, false, LightStyle::Washed),
    |c| look_color(c, false, LightStyle::Window),
];

/// An accent fill (a card's bar, a meter, the rain bars, the temperature
/// scale) in the light look: its hue kept, darkened until it stands at 3:1
/// (WCAG's floor for marks that aren't text) against the light glass at
/// its brightest. The muted accents (2026-11-14) are tuned for the dark
/// glass; on the frosted white they washed out. Luminance with gamma
/// taken as 2, as [`light_text`]: 0.25 or under.
#[must_use]
pub const fn light_accent_fill(c: Argb) -> Argb {
    let (mut r, mut g, mut b) = ((c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
    while (2126 * r * r + 7152 * g * g + 722 * b * b) / (255 * 255) > 2500 {
        r = r * 15 / 16;
        g = g * 15 / 16;
        b = b * 15 / 16;
    }
    (c & 0xFF00_0000) | r << 16 | g << 8 | b
}

/// Text's colour in the light look: a neutral one as [`light_twin`]; a
/// light accent (the rain blue, the focus blue) darkened to 55% so it reads
/// on the white glass (the first render: cyan "Add a place" and the rain
/// percentages were faint).
#[must_use]
pub const fn light_text(c: Argb) -> Argb {
    let t = light_twin(c);
    if t != c {
        return t;
    }
    // An accent kept in its hue, darkened until it reads at 4.5:1 on the
    // frosted glass (a fixed 55% left the live green at 3.46:1: the light
    // look's contrast test, 2026-11-01): its luminance, gamma taken as 2,
    // under 0.012: the grey glass's darkest, its bottom at 93% over a
    // black wallpaper pixel, is ~0.27 (the owner, 2026-11-01: "a little
    // more transparent"; 0.05 held on the white glass before it, and at
    // 0.05 the live green read 3.55:1 on the grey).
    let (mut r, mut g, mut b) = ((c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
    while (2126 * r * r + 7152 * g * g + 722 * b * b) / (255 * 255) > 120 {
        r = r * 9 / 10;
        g = g * 9 / 10;
        b = b * 9 / 10;
    }
    (c & 0xFF00_0000) | r << 16 | g << 8 | b
}

/// Which screen is up.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    /// The Today screen (the home).
    Today,
    /// The Weather app.
    Weather,
    /// The Weather app's settings.
    WeatherSettings,
    /// The Weather app's places (and removing one).
    WeatherPlaces,
    /// Adding a place: a search by name or postal code.
    AddPlace,
    /// Where the weather comes from: providers and stations.
    WeatherFrom,
    /// The station key, typed on the on-screen keyboard.
    KeyEntry,
    /// Your weather station's instruments now.
    Station,
    /// A weather alert in full.
    Alert,
    /// An app program ([`rp_app::Program`]): the shell keeps the bars, the
    /// program draws its area (the owner, 2026-11-14: every app its own
    /// program).
    Program,
}

impl Screen {
    /// Its name, for the run log (the input trail).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Today => "Today",
            Self::Weather => "Weather",
            Self::WeatherSettings => "WeatherSettings",
            Self::WeatherPlaces => "WeatherPlaces",
            Self::AddPlace => "AddPlace",
            Self::WeatherFrom => "WeatherFrom",
            Self::KeyEntry => "KeyEntry",
            Self::Station => "Station",
            Self::Alert => "Alert",
            Self::Program => "Program",
        }
    }
}

/// The keys the shell reacts to.
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
    /// D-pad centre: act on the focused target.
    Enter,
    /// The Home button: back to the Today screen.
    Home,
    /// The Calendar button: opens Calendar (Windows Mobile's job for it).
    Calendar,
    /// The Contacts button: opens Contacts.
    Contacts,
    /// The Record button: opens Notes on a new note (Windows Mobile's
    /// opens Notes recording one; ours has no voice notes yet).
    Record,
}

impl Key {
    /// Its name, for the run log (the input trail).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Up => "Up",
            Self::Down => "Down",
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Enter => "Enter",
            Self::Home => "Home",
            Self::Calendar => "Calendar",
            Self::Contacts => "Contacts",
            Self::Record => "Record",
        }
    }
}

/// One input event.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Input {
    /// The stylus touched the screen.
    PenDown {
        /// Screen x.
        x: u16,
        /// Screen y.
        y: u16,
    },
    /// The stylus left the screen (where it last was).
    PenUp {
        /// Screen x.
        x: u16,
        /// Screen y.
        y: u16,
    },
    /// The stylus moved while down (every sample the touch filter calls a
    /// move): lists follow it, swipes are measured as it goes.
    PenMove {
        /// Screen x.
        x: u16,
        /// Screen y.
        y: u16,
    },
    /// A key went down.
    Key(Key),
    /// A raw touch reading while the pen is down: sent instead of
    /// `PenDown`/`PenUp` while [`Shell::wants_raw`] (the align screen).
    Raw {
        /// Raw X channel.
        rx: u16,
        /// Raw Y channel.
        ry: u16,
    },
    /// The pen lifted, while [`Shell::wants_raw`].
    RawUp,
}

/// What the caller should do after an input.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Nothing to show.
    Nothing,
    /// Something changed: build and draw a frame.
    Redraw,
    /// The test image's "Back to loader" entry was chosen.
    BackToLoader,
    /// App program `n` (the Start menu's organiser entries, in order:
    /// Calendar, Contacts, Tasks, Notes, the Calculator) is to be loaded
    /// and opened ([`Shell::open_program`]); only when the caller said it
    /// has programs ([`Shell::use_programs`]).
    OpenProgram(u8),
    /// The align screen finished: map touches with this from now on.
    Calibrated(Calibration),
    /// The test image's wallpaper comparison: show the wallpaper dithered
    /// to 16 bits or plainly truncated to them from now on, then redraw
    /// everything (the screens and the wallpaper are RGB565 since ADR-0023's
    /// L1, 2026-10-21; before it the other side was full depth).
    WallpaperDepth {
        /// Dithered (the wallpaper as the OS keeps it) rather than plain.
        dithered: bool,
    },
}

/// Wall-clock time as the shell shows it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Time {
    /// 0..=23.
    pub hour: u8,
    /// 0..=59.
    pub minute: u8,
    /// 0 Monday ..= 6 Sunday.
    pub weekday: u8,
    /// 1..=31.
    pub day: u8,
    /// 1..=12.
    pub month: u8,
}

/// Something that can be tapped or focused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    /// Start, top left.
    Start,
    /// A quick button on the Today screen.
    Quick(u8),
    /// The left softkey (Calendar).
    SoftLeft,
    /// The right softkey (Edit Today).
    SoftRight,
    /// A Start menu entry.
    Menu(u8),
    /// The Today screen's weather hero (opens the Weather app).
    Hero,
    /// A tab of the Weather app.
    Tab(u8),
    /// The header's back control (the Weather app and the screens below it).
    Back,
    /// A row of a settings or places list.
    Row(u8),
    /// A key of the on-screen keyboard.
    Cap(u8),
    /// The radar's play and pause.
    Play,
    /// The radar's timeline (a tap picks the frame).
    Timeline,
    /// The Weather header's Station button (a station connected).
    Station,
    /// The title bar's ok: back, as Windows Mobile's.
    Ok,
    /// The Weather header's place: its menu of places.
    PlaceMenu,
    /// A place in that menu.
    PlaceItem(u8),
    /// The Weather header's alert: the alert in full.
    Alert,
    /// A reading that opens its chart ([`chart::Reading::ALL`]'s order).
    Reading(u8),
    /// The chart's span chip (0: 12 hours, 1: 2 days).
    Span(u8),
    /// Today's calendar panel (opens Calendar).
    Agenda,
    /// The tasks line on the panel's top right (opens Tasks).
    TasksLine,
}

// ---- Layout (480 x 640, portrait) --------------------------------------

const TITLE: Rect = Rect::new(0, 0, W, 36);
/// Start, top left.
pub const START: Rect = Rect::new(0, 0, 104, 36);
/// The Start button's box and the ok box (the mix mock-up's title bar).
const START_BOX: Rect = Rect::new(6, 4, 92, 28);
const OK_BOX: Rect = Rect::new(W - 46, 4, 40, 28);
/// The title bar's box lines.
const EDGE_LIGHT: Argb = rp_draw::look::theme(0x70FF_FFFF);
/// The weather hero panel: the place, the temperature, the condition, today's
/// high and low, and the date (docs/design/ui.md "Weather and icons").
/// Exactly the Weather app's Now panel (its place and size): the app opens
/// from it by Reveal reading the app in place, so the window's wallpaper
/// meets Today's and its first refresh is the card itself (the owner's run,
/// 2026-11-01, the trace: read 28 rows lower to meet a card at y 52, the
/// app's wallpaper moved inside the window, "the background box").
const HERO: Rect = Rect::new(16, 80, 448, 206);
/// The box of the weather icon: 8 px past the panel's right edge, its top
/// 2 px under the title bar (the mock-ups' 48 px above the panel put tall
/// pictures over the clock: the owner's photo, 2026-10-31; the picture
/// keeps its size, the box moved down); the picture is centred vertically
/// in it and sits against its right side.
const HERO_ICON: Rect = Rect::new(16 + 448 + 8 - 250, 80 - 14, 250, 176);
/// The big pictures' height at most (they fit the hero's 250 x 176 box):
/// the picture store's slots for them are that, not square.
pub const HERO_PIC_H: u16 = HERO_ICON.h;
/// The hero's place line stops short of the icon.
pub(crate) const HERO_LABEL_W: u16 = HERO_ICON.x - HERO.x - 28;
/// The date at the hero's bottom right (its text depends on the day).
pub const DATE_BOX: Rect = Rect::new(280, 80 + 176 - 24, 170, 40);
/// The five-day forecast row with the room above it for the icons that
/// reach over the cards' top edge (its day names depend on the day).
pub const FORECAST_ROW: Rect = Rect::new(16, 292, 448, 88);
/// The cards' top: the row's top plus the icons' headroom.
const FORECAST_TOP: u16 = 306;
const FORECAST_H: u16 = 74;
const FORECAST_W: u16 = 83;
const FORECAST_STEP: u16 = 91;
const CALENDAR_PANEL: Rect = Rect::new(16, 392, 448, 104);
/// The tasks line: the calendar panel's top right, beside "TODAY".
const TASKS_LINE: Rect = Rect::new(
    CALENDAR_PANEL.x + 152,
    CALENDAR_PANEL.y + 6,
    CALENDAR_PANEL.w - 158,
    32,
);
const SOFTKEYS: Rect = Rect::new(0, H - 40, W, 40);
const SOFT_LEFT: Rect = Rect::new(0, H - 40, 160, 40);
const SOFT_RIGHT: Rect = Rect::new(W - 160, H - 40, 160, 40);
/// Where the "Not yet" notice and the title clock sit.
const NOTICE: Rect = Rect::new(160, H - 40, 160, 40);
/// The title bar's clock and status (depends on the time).
pub const TITLE_CLOCK: Rect = Rect::new(W - 170, 0, 170, 36);
/// The quick buttons (docs/design/ui.md "A PDA, proudly").
const QUICK: [&str; 5] = ["Note", "Record", "Reader", "Media", "Games"];
const QUICK_Y: u16 = 532;
const QUICK_W: u16 = 84;
const QUICK_H: u16 = 54;

fn quick_rect(i: u8) -> Rect {
    Rect::new(16 + u16::from(i) * 91, QUICK_Y, QUICK_W, QUICK_H)
}

/// The degree sign in the big font (Latin-1's place; the small fonts have it
/// at DEL's slot, [`rp_weather::DEG`]).
const DEG_BIG: u8 = 0xB0;

/// The Start menu (docs/design/ui.md), "Back to loader" only in test images.
const MENU: [&str; 12] = [
    "Today",
    "Calendar",
    "Contacts",
    "Tasks",
    "Notes",
    "Calculator",
    "Documents",
    "File Explorer",
    "Inbox",
    "Browser",
    "Programs",
    "Settings",
];
const BACK_TO_LOADER: &str = "Back to loader";
/// The test image's way to the align screen (the OS's is Settings).
const ALIGN_SCREEN: &str = "Align screen";
/// The test image's wallpaper comparison (the owner's side-by-side check
/// of a 16-bit wallpaper, 2026-10-17): the wallpaper with nothing over it,
/// where a tap switches between dithered and plain 16 bits (32 and 16 bits
/// before ADR-0023's L1; quality audit C48) and Home goes back to the
/// Today screen with the one chosen. One entry: 13 rows is all the menu
/// has between the title bar and the softkeys.
const WALLPAPER_COMPARE: &str = "Wallpaper compare";
/// The notice naming the wallpaper after a switch (it goes after
/// NOTICE_MS).
const SHOWING_DITHERED: &str = "16-bit dithered";
const SHOWING_PLAIN: &str = "16-bit plain";
/// The notice's backing in the wallpaper-only view (readable on any picture).
const NOTICE_BACK: Argb = rp_draw::look::theme(0xD000_0000);
/// The test image's extra Start menu entries, after the OS's.
/// The test image's sky test ([`sky`]): each tap the next weather.
const SKY_TEST: &str = "Sky test";
const DEV_MENU: [&str; 4] = [WALLPAPER_COMPARE, SKY_TEST, ALIGN_SCREEN, BACK_TO_LOADER];
const MENU_X: u16 = 0;
const MENU_Y: u16 = 36;
const MENU_W: u16 = 280;
/// 33 px: the test image's sixteen rows fit between the title bar and the
/// softkeys, and the menu's area open and closed (280 x 536, RGB565) both
/// fit the renderer's area cache (a screen's bytes): at 34 px they were
/// 618 KB of its 614 KB, each pushed the other out and every opening was
/// drawn again (rp-today's test `gpu_today_matches_the_cpu_renderer`
/// counts the cache's hits).
const MENU_ROW: u16 = 33;

// ---- Colours (the smoked glass, docs/design/ui.md) -------------

// Every glass made denser (the owner, 2026-10-30: "make this glass
// background a little more opaque, it makes it too hard to read the
// letters, do it for all of the glass"): panels 72-55% to 85-74%, bars
// 85-69% to 91-82%, menus 88-82% to 93-88%.

const GLASS_LEFT: Argb = rp_draw::look::theme(0xE010_1418);
const GLASS_RIGHT: Argb = rp_draw::look::theme(0xC844_4C54);
const GLASS_EDGE: Argb = rp_draw::look::theme(0x50FF_FFFF);
const BAR_LEFT: Argb = rp_draw::look::theme(0xE800_0000);
const BAR_RIGHT: Argb = rp_draw::look::theme(0xD020_2428);
/// The apps' chrome, the agreed mix mock-up's colours (read from its
/// pixels): the title bar, the softkeys and their divider, opaque.
const APP_TITLE_LEFT: Argb = rp_draw::look::theme(0xFF12_171D);
const APP_TITLE_RIGHT: Argb = rp_draw::look::theme(0xFF1D_262F);
const APP_SOFT_LEFT: Argb = rp_draw::look::theme(0xFF02_0305);
const APP_SOFT_RIGHT: Argb = rp_draw::look::theme(0xFF1A_1D22);
const APP_SOFT_DIVIDER: Argb = rp_draw::look::theme(0xFF59_5D60);
/// The picture store's owner tag for the app program open
/// (`rp_draw::pics::Pics::set_owner`).
const PROGRAM_PICS: u8 = 1;

/// An app's bars in the light look: solid, not glass (the owner,
/// 2026-11-14: the title bar "being transparent on light mode" in a full
/// screen app). Aero's frosted blue-white, cooling to the right, the
/// softkeys a shade under the title; kept by the look's pass.
const APP_TITLE_LIGHT: (Argb, Argb) = (
    rp_app::kit::keep(rp_draw::look::theme(0xFFA8_B0BA)),
    rp_app::kit::keep(rp_draw::look::theme(0xFF98_A2AE)),
);
const APP_SOFT_LIGHT: (Argb, Argb) = (
    rp_app::kit::keep(rp_draw::look::theme(0xFF9E_A7B2)),
    rp_app::kit::keep(rp_draw::look::theme(0xFF8E_98A4)),
);
const APP_SOFT_DIVIDER_LIGHT: Argb = rp_app::kit::keep(rp_draw::look::theme(0xFF70_7C88));
const MENU_LEFT: Argb = rp_draw::look::theme(0xEE08_0A0C);
const MENU_RIGHT: Argb = rp_draw::look::theme(0xE02C_3238);
const TEXT: Argb = rp_draw::look::theme(0xFFF0_F2F4);
const DIM: Argb = rp_draw::look::theme(0xFFCC_D3DA);
const FOCUS: Argb = rp_draw::look::theme(0xFF7F_D0FF);
/// A pressed button: solid slate under its label (the owner's pick of
/// three drawn, 2026-10-31: a see-through press, "I do NOT like that";
/// docs/design/mockups/pressed-look-choices.png).
const PRESSED_UNDER: Argb = rp_draw::look::theme(0xFF3C_4656);
/// An app's window body (left, right), each look's own: the dark one
/// a shade under the title bar's slate, the light one Aero's frosted white
/// cooling to the right.
const BODY_DARK: (Argb, Argb) = (
    rp_draw::look::theme(0xFF0E_1216),
    rp_draw::look::theme(0xFF1A_2027),
);
const BODY_LIGHT: (Argb, Argb) = (
    rp_draw::look::theme(0xFF9C_A2AA),
    rp_draw::look::theme(0xFF8E_96A0),
);

/// `a` toward `b`, `k` of `n`, opaque.
fn mix(a: Argb, b: Argb, k: u32, n: u32) -> Argb {
    let ch = |s: u32| {
        let (x, y) = ((a >> s) & 0xFF, (b >> s) & 0xFF);
        ((x * (n - k) + y * k) / n) << s
    };
    0xFF00_0000 | ch(16) | ch(8) | ch(0)
}

/// The align screen's background (opaque: the wallpaper would distract).
const ALIGN_BACK: Argb = rp_draw::look::theme(0xFF0C_1014);
/// A target held long enough.
const HELD: Argb = rp_draw::look::theme(0xFF5C_E07A);

/// How long a notice stays (milliseconds).
const NOTICE_MS: u32 = 2000;

/// Dirty rectangles the shell gathers between frames.
const MAX_DAMAGE: usize = 8;

/// Words [`Shell::keep_lists`] takes: two frames' command keys.
pub const SHOWN_WORDS: usize = 2 * LIST_WORDS;

/// Words one frame's command keys take ([`Shell::keep_today_list`]).
pub const LIST_WORDS: usize = rp_draw::MAX_CMDS * rp_draw::changes::KEY_WORDS;

/// What a stroke on the Weather app's page has turned out to be.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Gesture {
    /// No stroke on the page.
    None,
    /// Down, not yet moved past the slop: may still be a tap.
    Pending {
        /// Where it went down.
        x0: u16,
        /// See `x0`.
        y0: u16,
    },
    /// Up or down: the list follows the pen.
    Scroll,
    /// Across: turns the page when it lifts far enough; the tab's
    /// underline follows it meanwhile.
    Swipe {
        /// Where it went down.
        x0: u16,
        /// How far it has gone (negative: left).
        dx: i16,
    },
    /// Along the chart's line: the marker follows the pen.
    Scrub,
    /// Down from the chart sheet's top: far enough closes it.
    Sheet {
        /// Where it went down.
        y0: u16,
    },
}

/// How far the pen may move before a press stops being a tap (px): a
/// first draft for R124's measurement (the panel's jitter at rest is 2-3
/// px after the filter).
const SLOP: i32 = 8;
/// A swipe this far across turns the page.
const SWIPE_TURN: i32 = 60;
/// How far past the top a pull must go to refresh (px; T4's 70 px drawn,
/// the band halves the pen's travel past an end).
const PULL_REFRESH: i32 = 56;

/// The shell's state.
pub struct Shell {
    /// The organiser apps' state ([`apps`]).
    apps: apps::Apps,
    /// What Today's calendar panel shows, worked from the store when it
    /// changes ([`today_feed`]).
    feed: core::cell::RefCell<today_feed::Feed>,
    /// The device's offset from UTC, minutes (the clock file's).
    utc_offset: i32,
    /// The clipboard programs share.
    clip: core::cell::RefCell<rp_app::Text<64>>,
    /// Programs' rounded shapes' corners, painted once per colour.
    shapes: rp_app::Shapes,
    /// The app program open, if any ([`Screen::Program`]).
    program: core::cell::RefCell<Option<&'static mut dyn rp_app::Program>>,
    /// The organiser's entries open app programs (the caller loads them)
    /// rather than the apps built into the shell.
    programs: bool,
    /// The next program's intent (the Note button: a new note).
    intent: u8,
    /// When the open program's motion wants its next step (ms).
    prog_anim: Option<u32>,
    /// An app opening out of Today: when it began (ms) and the rectangle
    /// its window grows from ([`Self::prog_window`]).
    prog_open: Option<(u32, Rect, u32)>,
    /// Where the stylus last left the screen (`None` after a key): what
    /// an app's window opens out of.
    tap: Option<(u16, u16)>,
    /// The opening has not had its first step yet.
    prog_fresh: bool,
    /// The Start menu's sheet an app was picked from (what its window
    /// opens out of), and whether an app was left with no frame drawn
    /// since (it is still on the screen): another opened now comes over
    /// it.
    open_from: Option<Rect>,
    left_app: bool,
    /// The field the program's keyboard was set for, and the last
    /// character typed (the automatic capitals).
    prog_field: Option<rp_app::Field>,
    prog_last: u8,
    /// The picture store programs paint their avatars and marks into.
    app_pics: Option<&'static rp_draw::pics::Pics<'static>>,
    /// The forecast by day, lent to programs (the calendar's days).
    forecast: [rp_app::DayWeather; 10],
    forecast_n: u8,
    fonts: Fonts,
    icons: Icons,
    /// Whether the Start menu offers "Back to loader" (test images only).
    dev_exit: bool,
    menu_open: bool,
    focus: Option<Target>,
    pressed: Option<Target>,
    time: Time,
    notice: Option<(&'static str, u32)>,
    now: u32,
    damage: [Rect; MAX_DAMAGE],
    damaged: usize,
    /// Data changed that may show anywhere (a picture, a forecast, a radar
    /// frame): the next frame finds where by comparing its draw list with
    /// the last one drawn ([`rp_draw::changes`], R191), not by drawing the
    /// whole screen.
    restated: bool,
    /// The last two frames' command keys ([`SHOWN_WORDS`], the device's
    /// RAM pool: its zeroed RAM budget had no 8 KB left), a list in each
    /// half: `shown_at` names the half holding the last frame drawn,
    /// `shown_n` its length (`None`: unknown, or no lists given, and a
    /// restatement draws whole).
    shown: Option<&'static mut [u32]>,
    shown_at: usize,
    shown_n: Option<usize>,
    /// Today's last list as it was left ([`LIST_WORDS`], the device's RAM
    /// pool) and its length: a return to Today marks only where its list
    /// differs from it, the device starting the frame from Today's kept
    /// picture (the owner's ruling of 2026-11-12 on R191). `None`: the
    /// caller keeps no picture, and a return is drawn whole.
    today_kept: Option<&'static mut [u32]>,
    today_kept_n: Option<usize>,
    /// The screen of the last frame built (`None`: none yet).
    drawn_screen: Option<Screen>,
    /// The last frame left Today (the caller keeps the picture shown), or
    /// returned to it marking only what differs from the kept list (the
    /// caller starts it from that picture or marks the whole screen).
    left_today: bool,
    from_kept: bool,
    /// The align screen, while it runs (over everything).
    align: Option<align::Align>,
    /// The test image's comparison: the wallpaper plainly truncated (not
    /// dithered).
    wall_plain: bool,
    /// The test image's comparison: the wallpaper alone, nothing over it.
    bare: bool,
    /// The screen up.
    screen: Screen,
    /// The forecast every screen reads (the sample until there is a network).
    wx: Forecast,
    /// How numbers and times are shown (US customary by default).
    region: Region,
    /// The Weather app's page: 0 Now, 1 Hourly, 2 10 Day.
    tab: u8,
    /// The page's scroll (each page starts at its top).
    scroll: motion::Scroll,
    /// What the pen is doing on the Weather app's page.
    gesture: Gesture,
    /// When the page's entrance started (none: everything in place).
    anim: Option<u32>,
    /// How pages arrive, and whether this entrance came from the hero.
    motion: weather_app::Motion,
    expanding: bool,
    /// The Weather app's settings (kept through the dock: `Ask::Save`).
    wset: rp_weather::settings::Settings,
    /// The field being typed (a place's name, or the key: zeroed when done
    /// or left).
    typed: [u8; rp_weather::settings::KEY_MAX],
    typed_n: usize,
    /// The keyboard (K1 compact): shift and the symbols layer.
    kb: keyboard::Keyboard,
    /// The places list in Remove mode.
    removing: bool,
    /// The place search's matches, how many, when it is due, and what it
    /// says while there are none.
    found: [rp_weather::open_meteo::Found; rp_weather::open_meteo::FOUND_MAX],
    n_found: u8,
    search_at: Option<u32>,
    find_status: &'static str,
    /// The dock's questions wanted (bits of [`Ask`]) and the one in flight.
    wants: u16,
    asking: Option<Ask>,
    /// The hour Hourly's card shows.
    hour_pick: u8,
    /// The header's place menu is open.
    place_menu: bool,
    /// The weather alerts in force for the place shown.
    alerts: [rp_weather::alerts::Alert; rp_weather::alerts::ALERTS_MAX],
    n_alerts: u8,
    /// Hourly's graph, one store picture ([`Shell::paint_graph`]).
    graph: Option<Pic>,
    /// The open chart's line, a picture of its own ([`Shell::paint_chart_pic`]):
    /// in Hourly's slot, opening a chart repainted Hourly's line away and
    /// rebuilt Hourly's page around the chart's (~190 ms each, the owner's
    /// run of 2026-11-01).
    chart_pic: Option<Pic>,
    /// The Station screen's dials (painted at start by the device).
    dials: station::Dials,
    /// The real radar (R184 step 2): its frames' times (Unix, oldest
    /// first), how many, the place it was asked for, whether the map is in,
    /// which rain frames are complete (a bit each), and the frame being
    /// fetched with its next tile (the newest missing first).
    radar_times: [u32; rp_weather::radar::PAST],
    radar_n: u8,
    radar_for: Option<(i32, i32)>,
    radar_map: bool,
    radar_have: u16,
    radar_tile: Option<(u8, u8)>,
    /// Rain frames the relay refused twice (shown as missing, never dry:
    /// run today-0054), those asked again once, and those the device has
    /// composed and can show (the loop waits for every one).
    radar_lost: u16,
    radar_retried: u16,
    radar_ready: u16,
    /// The device has no 2700G memory to compose the frame shown (audit
    /// C97): the radar says the rain is unavailable, never a dry map.
    radar_no_room: bool,
    /// The relay didn't answer the radar's frame list (no dock): said so
    /// on the radar, never a sample's rain instead.
    radar_offline: bool,
    /// A refresh of the radar for the same place (its map and frames kept
    /// until the new list says which still hold), and the frames moved by
    /// their time for the device to apply (P7: [`Shell::take_radar_moves`]).
    radar_refresh: bool,
    radar_moves: Option<[u8; rp_weather::radar::PAST]>,
    radar_asked: u32,
    /// The place whose map is in (kept across a refused frame list for the
    /// same place: the device's layers are still that place's), the
    /// device's copy of the place's map (R184 step 3: on its card, read
    /// and checked with the relay once the rain is in) and whether the
    /// question in flight is the map's.
    radar_map_for: Option<(i32, i32)>,
    radar_card: weather_net::CardMap,
    asked_map: bool,
    /// The default icon pack's offset asked next ([`Ask::Pack`]).
    pack_at: u32,
    /// The station's instruments and when they came (ms).
    station: Option<rp_weather::tempest::StationNow>,
    station_at: u32,
    /// The station's last day (for its charts) and when it came (ms).
    history: Option<rp_weather::tempest::History>,
    history_at: u32,
    /// The chart sheet open, if any (N3, N4: [`chart`]).
    chart: Option<chart::Chart>,
    /// When the living sky last moved (ms), and the previews' word that it
    /// moves with the sample forecast ([`sky`]).
    sky_at: u32,
    /// The sky's steps so far (its clouds move a whole pixel a step), and
    /// how many of them changed what is on screen (the device's log: the
    /// owner, 2026-11-06, "it doesnt do the animation at all").
    sky_n: u32,
    sky_moved: u32,
    sky_forced: bool,
    /// The sky's weather at its last step (R183: a change, rain to dry,
    /// redraws the whole card once, so no streak of the old weather stays).
    sky_was: Option<(u8, bool, bool, u8)>,
    /// The light look's style, and its panels' ([`soft`]).
    light_style: LightStyle,
    /// What fills an app's window behind it ([`WindowBody`]).
    window_body: WindowBody,
    /// Where Today's tasks line goes.
    today_tasks: TodayTasks,
    /// The floor of the last frame made ([`Self::floor`]).
    floor_was: Option<(Rect, Argb, Argb)>,
    /// The host paints window backdrops ([`Self::use_backdrops`]).
    backdrops: bool,
    /// The previews' panels left out (mock-ups composite their own).
    panels_off: bool,
    /// The test image's sky test: 0 the forecast's sky, else the weather
    /// it shows ([`sky`]).
    sky_test: u8,
    /// When the forecast last came (ms), whether the dock ever answered,
    /// and whether the station's readings are in.
    fetched_at: Option<u32>,
    dock: bool,
    station_live: bool,
    /// The radar loop: the frame shown, whether it plays, when it last
    /// moved on (ms).
    radar_frame: u8,
    radar_play: bool,
    radar_at: u32,
    /// The plane path (sheet weather-gpu-movement.md, ADR-0034 C4 as
    /// changed, L2): the Weather pages on the page layer, screen 0 their
    /// chrome. Set by the device; the preview draws everything in 2D.
    planes: bool,
    /// Whether the last frame left the page to the layer (a change redraws
    /// the page's area on screen 0).
    layer_was: bool,
    /// While a page is drawn into its picture: the picture's rows drawn
    /// (top, bottom), in picture rows.
    band: Option<(i32, i32)>,
    /// Reveal's content glides up into place (the mock-up's way).
    reveal_glide: bool,
}

impl Shell {
    /// The Today screen, all of it to be drawn.
    #[must_use]
    pub fn new(fonts: Fonts, icons: Icons, dev_exit: bool) -> Self {
        let mut s = shell_start!(fonts, icons, dev_exit; value);
        s.dirty(Rect::new(0, 0, W, H));
        s
    }

    /// [`Shell::new`] made straight into `slot` (the device's, in `.bss`), a
    /// field at a time: the Shell is 8.5 KB, and made on the stack and
    /// moved it took a 15.8 KB frame at start (then 8.9 KB through
    /// `slot.write`), which with `main`'s 8.7 KB overflowed the image's
    /// stack (today-0036, -0037 on the X51v). `cargo xtask stacks` (P6)
    /// keeps every such frame under 4 KB.
    pub fn new_in(
        slot: &mut core::mem::MaybeUninit<Self>,
        fonts: Fonts,
        icons: Icons,
        dev_exit: bool,
    ) -> &mut Self {
        let s = shell_start!(fonts, icons, dev_exit; slot,);
        s.dirty(Rect::new(0, 0, W, H));
        s
    }
}

/// Every field's start, made as a value or straight into a slot
/// (`rp_hal_mem::in_place!`): one list for [`Shell::new`] and
/// [`Shell::new_in`].
macro_rules! shell_start {
    ($fonts:expr, $icons:expr, $dev_exit:expr; $($how:tt)*) => {
        rp_hal_mem::in_place!($($how)* Shell {
            apps: apps::Apps::new(),
            feed: core::cell::RefCell::new(today_feed::Feed::new()),
            utc_offset: 0,
            clip: core::cell::RefCell::new(rp_app::Text::EMPTY),
            shapes: rp_app::Shapes::new(),
            program: core::cell::RefCell::new(None),
            programs: false,
            intent: 0,
            prog_anim: None,
            prog_open: None,
            tap: None,
            prog_fresh: false,
            open_from: None,
            left_app: false,
            prog_field: None,
            prog_last: b' ',
            app_pics: None,
            forecast: [rp_app::DayWeather {
                day: 0,
                high: 0,
                low: 0,
                icon: None,
            }; 10],
            forecast_n: 0,
            fonts: $fonts,
            icons: $icons,
            dev_exit: $dev_exit,
            menu_open: false,
            focus: None,
            pressed: None,
            time: Time::default(),
            notice: None,
            now: 0,
            damage: [Rect::default(); MAX_DAMAGE],
            damaged: 0,
            restated: false,
            shown: None,
            shown_at: 0,
            shown_n: None,
            today_kept: None,
            today_kept_n: None,
            drawn_screen: None,
            left_today: false,
            from_kept: false,
            align: None,
            wall_plain: false,
            bare: false,
            screen: Screen::Today,
            wx: rp_weather::sample(),
            region: Region::US,
            tab: 0,
            scroll: motion::Scroll::new(0),
            gesture: Gesture::None,
            anim: None,
            motion: weather_app::Motion::Stagger,
            expanding: false,
            wset: rp_weather::settings::Settings::DEFAULT,
            typed: [0; rp_weather::settings::KEY_MAX],
            typed_n: 0,
            kb: keyboard::Keyboard::default(),
            removing: false,
            found: @each rp_weather::open_meteo::Found {
                name: rp_weather::Name::EMPTY,
                area: rp_weather::Name::EMPTY,
                lat: 0,
                lon: 0,
            },
            n_found: 0,
            search_at: None,
            find_status: "",
            // The stored settings first, then the forecast they name.
            wants: 1 << Ask::Load as u16,
            asking: None,
            hour_pick: 0,
            place_menu: false,
            alerts: @each rp_weather::alerts::Alert::NONE,
            n_alerts: 0,
            graph: None,
            chart_pic: None,
            dials: station::Dials::NONE,
            radar_times: [0; rp_weather::radar::PAST],
            radar_n: 0,
            radar_for: None,
            radar_map: false,
            radar_have: 0,
            radar_tile: None,
            radar_lost: 0,
            radar_retried: 0,
            radar_ready: 0,
            radar_no_room: false,
            radar_offline: false,
            radar_refresh: false,
            radar_moves: None,
            radar_asked: 0,
            radar_map_for: None,
            radar_card: weather_net::CardMap::Unknown,
            asked_map: false,
            pack_at: 0,
            station: None,
            station_at: 0,
            history: None,
            history_at: 0,
            chart: None,
            sky_at: 0,
            sky_was: None,
            sky_n: 0,
            sky_moved: 0,
            sky_forced: false,
            light_style: LightStyle::Frosted,
            window_body: WindowBody::Tinted,
            today_tasks: TodayTasks::Header,
            floor_was: None,
            backdrops: false,
            panels_off: false,
            sky_test: 0,
            fetched_at: None,
            dock: false,
            station_live: false,
            radar_frame: 0,
            radar_play: true,
            radar_at: 0,
            planes: false,
            layer_was: false,
            band: None,
            reveal_glide: true,
        })
    };
}
use shell_start;

impl Shell {
    /// The focused target, if the D-pad has been used.
    #[must_use]
    pub fn focus(&self) -> Option<Target> {
        self.focus
    }

    /// Whether the Start menu is open.
    #[must_use]
    pub fn menu_open(&self) -> bool {
        self.menu_open
    }

    /// The screen up.
    #[must_use]
    pub fn screen(&self) -> Screen {
        self.screen
    }

    /// The Weather app's page (0 Now, 1 Hourly, 2 10 Day) and how far it
    /// is scrolled (px; negative or past its end while pulled).
    #[must_use]
    pub fn weather_page(&self) -> (u8, i32) {
        (self.tab, self.scroll.offset())
    }

    /// While the Weather app opens by Reveal, the window it shows through
    /// and how many rows lower the app is read inside it (the plane path's
    /// video window and start offset; the preview composes that look).
    #[must_use]
    pub fn reveal_window(&self) -> Option<(Rect, u16)> {
        self.reveal().filter(|_| self.screen == Screen::Weather)
    }

    /// Whether something moves by itself (a page's entrance, a fling or a
    /// rebound): the caller keeps drawing frames.
    #[must_use]
    pub fn animating(&self) -> bool {
        self.anim.is_some() || self.scroll.moving() || self.prog_open.is_some()
    }

    /// Ends any movement at once, everything in place (pictures for the
    /// preview; the device lets it run).
    pub fn settle(&mut self) {
        if self.anim.take().is_some() | self.prog_open.take().is_some() {
            self.dirty(Rect::new(0, 0, W, H));
        }
        if self.scroll.moving() {
            let at = self.scroll.offset().clamp(0, self.scroll.max());
            self.scroll.set(at);
            self.dirty(weather_app::PAGE);
        }
    }

    /// The notice showing, if any.
    #[must_use]
    pub fn notice(&self) -> Option<&'static str> {
        self.notice.map(|(n, _)| n)
    }

    fn dirty(&mut self, r: Rect) {
        if r.is_empty() {
            return;
        }
        if self.damaged < MAX_DAMAGE {
            self.damage[self.damaged] = r;
            self.damaged += 1;
        } else {
            // Out of room: everything so far in one rectangle.
            let all = self.damage.iter().fold(r, |a, d| a.union(d));
            self.damage[0] = all;
            self.damaged = 1;
        }
    }

    /// The forecast every screen reads.
    #[must_use]
    pub fn forecast(&self) -> &rp_weather::Forecast {
        &self.wx
    }

    /// A new forecast (a fetch through the dock's relay, R132): every
    /// screen that shows it is redrawn; the Weather pages' pictures are
    /// checked against their draw lists and rebuilt.
    pub fn set_forecast(&mut self, f: rp_weather::Forecast) {
        self.wx = f;
        self.restate();
    }

    /// The units and formats shown (the settings set them; tests and
    /// previews show a country's at once).
    pub fn set_region(&mut self, r: Region) {
        self.region = r;
        self.restate();
    }

    /// The icon pack the settings name (the device loads it from the card
    /// and hands it over with [`Shell::set_icons`]).
    #[must_use]
    pub fn icon_pack(&self) -> u8 {
        self.wset.pack
    }

    /// New pictures (a whole set at once: the previews' and the tests'):
    /// every screen redrawn with them.
    pub fn set_icons(&mut self, icons: Icons) {
        self.icons = icons;
        self.restate();
    }

    /// One picture decoded (or given back, `None`): `icon` is the pack id
    /// less one. Every screen is redrawn: the pages' draw lists key the
    /// handle, so only a page that shows it is composed again.
    pub fn set_pic(&mut self, size: Size, icon: usize, pic: Option<Pic>) {
        if let Some(p) = self.icons.of_mut(size).get_mut(icon)
            && *p != pic
        {
            *p = pic;
            self.restate();
        }
    }

    /// The picture held for `icon` (pack id less one) at `size`.
    #[must_use]
    pub fn pic(&self, size: Size, icon: usize) -> Option<Pic> {
        self.icons.of(size).get(icon).copied().flatten()
    }

    /// Whether the title bar shows Windows Mobile's "ok" (closes the screen).
    /// Not in the Weather app: back and Home do its job (the owner's ruling,
    /// 2026-10-31, R162: "We dont need this ... atleast for the weather app,
    /// it might make sense on a different app"). Every screen so far is the
    /// Weather app's or Today, so none shows it; an app that wants it says
    /// so here.
    const fn shows_ok(&self) -> bool {
        false
    }

    /// The pictures the screens show with this forecast: the hero's and
    /// the Now page's big one, the five forecast cards, Hourly's card for
    /// any hour picked and the rows of its 24 hours and of 10 Day. Every
    /// hour's card picture is listed (a pick never waits on a decode, P1).
    #[must_use]
    pub fn wanted(&self) -> Wanted {
        let mut w = Wanted::default();
        let id = |sky| usize::from(Icon::of(sky).id() - 1);
        w.add(Size::Hero, id(self.wx.now.sky));
        // The sky the hero draws ([`Self::sky_now`]: the sky test's while
        // it is on). Its big picture was never asked for, so the hero
        // showed the forecast card's 66 px one in its place (the owner on
        // the X51v, 2026-11-14: "turned on the sky test and the today
        // screen icon turned tiny", R207). The forecast's stays wanted
        // beside it (the store has two big slots): back from the test
        // without a decode.
        w.add(Size::Hero, id(self.sky_now()));
        for d in &self.wx.days[1..6] {
            w.add(Size::Card, id(d.sky));
        }
        for h in &self.wx.hours {
            w.add(Size::Mid, id(h.sky));
            w.add(Size::Row, id(h.sky));
        }
        for d in &self.wx.days {
            w.add(Size::Row, id(d.sky));
        }
        w
    }

    /// The Today screen's five forecast cards: each day's condition (pack
    /// id less one) and the picture its card shows (`None`: the card is
    /// drawn without one). For the run log (R181: a card shown without its
    /// icon said nothing).
    #[must_use]
    pub fn today_cards(&self) -> [(u8, Option<Pic>); 5] {
        let mut out = [(0, None); 5];
        for (o, d) in out.iter_mut().zip(&self.wx.days[1..6]) {
            let id = Icon::of(d.sky).id() - 1;
            *o = (id, self.icons.card[usize::from(id)]);
        }
        out
    }

    /// The first alert in force's name (the header's chip).
    pub(crate) fn alert_name(&self) -> Option<&[u8]> {
        (self.n_alerts > 0).then(|| self.alerts[0].event.as_bytes())
    }

    /// Where the header's alert chip is (after the place and its dots).
    pub(crate) fn alert_rect(&self) -> Rect {
        self.alert_chip().map_or(Rect::new(0, 0, 0, 0), |(r, _)| r)
    }

    /// The header's alert chip and the part of the alert's name that fits
    /// it: it ends before the STATION chip (or the reading's time), a long
    /// name cut with ".." (the owner's run, 2026-10-31: both chips shown).
    pub(crate) fn alert_chip(&self) -> Option<(Rect, Line)> {
        let (ui, label) = (self.fonts.ui, self.fonts.label);
        let name = self.alert_name()?;
        let mx = weather_app::PLACE_BTN.x + 14 + ui.width(self.wx.place.as_bytes());
        let x = mx + 22 + 14 * u16::from(self.wset.n_places.min(10)) + 8;
        let end = if self.station.is_some() {
            weather_app::STATION_BTN.x
        } else {
            W - 12 - label.width(self.read_at().as_bytes())
        }
        .saturating_sub(8);
        let room = end.saturating_sub(x + 34);
        let mut n = name.len();
        while n > 0 && label.width(&name[..n]) > room {
            n -= 1;
        }
        let text = if n < name.len() {
            let mut k = n;
            while k > 0 && label.width(&name[..k]) + label.width(b"..") > room {
                k -= 1;
            }
            Line::new().push(&name[..k]).push(b"..")
        } else {
            Line::new().push(name)
        };
        (room > 0).then(|| {
            (
                Rect::new(
                    x,
                    weather_app::HEAD.y + 5,
                    label.width(text.as_bytes()) + 34,
                    24,
                ),
                text,
            )
        })
    }

    /// When the forecast was read, as a clock time ("sample" before any).
    pub(crate) fn read_at(&self) -> Line {
        match self.fetched_at {
            None if self.wants(Ask::Forecast) => Line::new().push(b"updating"),
            None => Line::new().push(b"sample"),
            Some(at) => {
                let ago = (self.now.wrapping_sub(at) / 60_000) as i32;
                let m = (i32::from(self.time.hour) * 60 + i32::from(self.time.minute) - ago)
                    .rem_euclid(24 * 60);
                self.region.time((m / 60) as u8, (m % 60) as u8, true)
            }
        }
    }

    /// Place `i` in the header's menu.
    const fn place_item(i: u8) -> Rect {
        Rect::new(
            8,
            weather_app::HEAD.y + weather_app::HEAD.h + i as u16 * 42,
            280,
            42,
        )
    }

    /// The header's place menu over the page.
    pub(crate) fn place_menu_scene(&self, f: &mut Frame) {
        let ui = self.fonts.ui;
        let n = self.wset.n_places;
        let r = Rect::new(
            8,
            weather_app::HEAD.y + weather_app::HEAD.h,
            280,
            u16::from(n) * 42,
        );
        self.glass(f, r, MENU_LEFT, MENU_RIGHT);
        for i in 0..n {
            let e = Self::place_item(i);
            let p = self.wset.places()[usize::from(i)];
            let color = if i == self.wset.current { FOCUS } else { TEXT };
            self.press_under(f, Target::PlaceItem(i));
            f.text(e.x + 16, e.y + 28, ui, color, p.name.as_bytes());
            self.decorate(f, Target::PlaceItem(i));
        }
    }

    /// An alert in full: its name, how severe and until when, the headline
    /// and its text, wrapped to the screen.
    pub(crate) fn alert_scene(&self, f: &mut Frame) {
        let (ui, label) = (self.fonts.ui, self.fonts.label);
        let r = Rect::new(12, 46, 456, H - 40 - 56);
        self.glass(f, r, MENU_LEFT, MENU_RIGHT);
        let Some(a) = self.alerts.first().filter(|_| self.n_alerts > 0) else {
            f.text(r.x + 16, r.y + 34, ui, TEXT, b"No alerts in force");
            return;
        };
        f.text(
            r.x + 16,
            r.y + 34,
            ui,
            weather_app::ALERT_TEXT,
            a.event.as_bytes(),
        );
        let sev: &[u8] = match a.severity {
            rp_weather::alerts::Severity::Extreme => b"EXTREME",
            rp_weather::alerts::Severity::Severe => b"SEVERE",
            rp_weather::alerts::Severity::Moderate => b"MODERATE",
            rp_weather::alerts::Severity::Minor => b"MINOR",
            rp_weather::alerts::Severity::Unknown => b"",
        };
        // Until "2 NOVEMBER 11:00 AM", from the service's local ISO time
        // ("2026-11-02T11:00:00-07:00"); anything else as it came.
        let u = a.until.as_bytes();
        let num = |r: core::ops::Range<usize>| {
            u.get(r)?
                .iter()
                .try_fold(0u8, |n, &c| c.is_ascii_digit().then(|| n * 10 + (c - b'0')))
        };
        let until = match (num(5..7), num(8..10), num(11..13), num(14..16)) {
            (Some(mo @ 1..=12), Some(d @ 1..=31), Some(h @ 0..=23), Some(mi @ 0..=59)) => {
                Line::new()
                    .num(i32::from(d))
                    .push(b" ")
                    .push(MONTHS[usize::from(mo - 1)])
                    .push(b" ")
                    .push(self.region.time(h, mi, true).as_bytes())
            }
            _ => Line::new().push(u.get(..16).unwrap_or(u)),
        };
        let meta = Line::new()
            .push(sev)
            .push(b"  UNTIL ")
            .push(until.as_bytes());
        f.text(r.x + 16, r.y + 58, label, DIM, meta.as_bytes());
        let mut y = r.y + 92;
        for (text, font, color) in [
            (a.headline.as_bytes(), ui, TEXT),
            (a.text.as_bytes(), ui, DIM),
        ] {
            let mut rest = text;
            while !rest.is_empty() && y + 8 < r.y + r.h {
                // Words to the line's width.
                let mut end = rest.len();
                while font.width(&rest[..end]) > r.w - 32 {
                    end = rest[..end]
                        .iter()
                        .rposition(|&b| b == b' ')
                        .unwrap_or(end - 1);
                }
                f.text(r.x + 16, y, font, color, &rest[..end]);
                rest = rest[end..].strip_prefix(b" ").unwrap_or(&rest[end..]);
                y += 26;
            }
            y += 12;
        }
        if self.n_alerts > 1 {
            let more = Line::new()
                .num(i32::from(self.n_alerts - 1))
                .push(b" more in force");
            f.text(r.x + 16, r.y + r.h - 12, label, DIM, more.as_bytes());
        }
    }

    /// The app's name in the title bar.
    fn title(&self) -> &'static str {
        match self.screen {
            Screen::Today => "",
            Screen::Weather => "Weather",
            Screen::WeatherSettings => "Weather settings",
            Screen::WeatherPlaces => "Places",
            Screen::AddPlace => "Add a place",
            Screen::WeatherFrom => "Weather from",
            Screen::KeyEntry => "Station key",
            Screen::Station => "Your station",
            Screen::Alert => "Weather alert",
            Screen::Program => self.program.borrow().as_ref().map_or("", |p| p.title()),
        }
    }

    /// The alerts in force as the service answers them (NWS GeoJSON), as
    /// the dock's relay brings them: for the PC's previews, which have no
    /// relay.
    pub fn set_alerts_from_nws(&mut self, json: &[u8]) {
        self.n_alerts = rp_weather::alerts::from_nws(json, &mut self.alerts) as u8;
        self.dirty(weather_app::HEAD);
    }

    /// A station's reading as Tempest answers it: for the PC's previews,
    /// which have no relay.
    pub fn set_station_from_tempest(&mut self, json: &[u8]) {
        self.station = rp_weather::tempest::station_now(json);
        self.station_at = self.now;
        self.restate();
    }

    /// Hourly's graph picture, painted by the device with
    /// [`Shell::paint_graph`] for the forecast [`Shell::graph_key`] names.
    pub fn set_graph(&mut self, pic: Option<Pic>) {
        if self.graph != pic {
            self.graph = pic;
            self.restate();
        }
    }

    /// Hourly's graph picture held.
    #[must_use]
    pub fn graph(&self) -> Option<Pic> {
        self.graph
    }

    /// The size of Hourly's graph picture.
    #[must_use]
    pub fn graph_size(&self) -> (u16, u16) {
        weather_app::GRAPH_PIC
    }

    /// The open chart's line picture held.
    #[must_use]
    pub fn chart_pic(&self) -> Option<Pic> {
        self.chart_pic
    }

    /// The open chart's line picture, painted by the device with
    /// [`Shell::paint_chart_pic`] for the chart [`Shell::chart_pic_key`]
    /// names.
    pub fn set_chart_pic(&mut self, pic: Option<Pic>) {
        if self.chart_pic != pic {
            self.chart_pic = pic;
            self.restate();
        }
    }

    /// What the open chart's line shows (`None`: no chart open): a new key
    /// is a new picture to paint.
    #[must_use]
    pub fn chart_pic_key(&self) -> Option<u32> {
        self.chart.map(|c| self.chart_key(c))
    }

    /// What Hourly's graph picture shows (its hours' temperatures): a new
    /// key is a new picture to paint.
    #[must_use]
    pub fn graph_key(&self) -> u32 {
        self.wx.hours[..12].iter().fold(0x811C_9DC5u32, |h, x| {
            (h ^ u32::from(x.temp.cast_unsigned())).wrapping_mul(0x0100_0193)
        })
    }

    /// An icon pack is being read from the card (about two seconds the
    /// first time): said at once, so the tap is seen to have worked.
    pub fn icons_loading(&mut self) {
        self.show_notice("Loading icons");
    }

    /// The pack the settings name couldn't be read: back to the default.
    pub fn icon_pack_failed(&mut self) {
        self.wset.pack = 0;
        self.show_notice("Pack not on the card");
        self.want(Ask::Save);
        self.dirty(Rect::new(0, 0, W, H));
    }

    /// Marks the whole screen to be drawn (the device's hand-back from a
    /// picture composed ahead to the renderer's own screens).
    pub fn redraw_all(&mut self) {
        self.dirty(Rect::new(0, 0, W, H));
    }

    /// Marks `r` to be drawn: the device changed what lies under it (the
    /// radar's wallpaper bands switched to another frame, R184 step 2).
    pub fn redraw(&mut self, r: Rect) {
        self.dirty(r);
    }

    /// Whether anything waits to be drawn.
    #[must_use]
    pub fn needs_frame(&self) -> bool {
        self.damaged > 0 || self.restated
    }

    /// Data changed that may show on any screen, or none: the next frame
    /// draws only where its draw list differs from the last one's (R191:
    /// marked whole, Today was drawn whole for each icon decoded at
    /// start-up and Settings for each forecast and radar answer, though
    /// nothing on it changed, run today-0115). Never for a change under
    /// the list (the wallpaper, the radar map): those mark their area.
    fn restate(&mut self) {
        self.restated = true;
    }

    /// Room for the last two frames' command keys ([`SHOWN_WORDS`] words
    /// or more), so a restatement draws only what changed. Without it every
    /// restatement draws the whole screen.
    pub fn keep_lists(&mut self, words: &'static mut [u32]) {
        if words.len() >= SHOWN_WORDS {
            self.shown = Some(words);
            self.shown_n = None;
        }
    }

    /// Room for Today's last list as it is left ([`LIST_WORDS`] words or
    /// more), for a caller that keeps Today's picture as it leaves
    /// ([`Self::take_left_today`]) and starts the return from it
    /// ([`Self::take_from_kept`]): the return then marks only where Today's
    /// list differs from the kept one (the owner's ruling of 2026-11-12 on
    /// R191: the return was drawn whole, 706 blits, 55.5 ms on the X51v).
    /// Needs [`Self::keep_lists`] too.
    pub fn keep_today_list(&mut self, words: &'static mut [u32]) {
        if words.len() >= LIST_WORDS {
            self.today_kept = Some(words);
            self.today_kept_n = None;
        }
    }

    /// While another screen is up: Today's frame as a return would draw it
    /// now (the menu closed, nothing focused), built into `f` with the
    /// whole screen marked, the shell's state unchanged; and whether the
    /// list kept as Today was left still matches it. `None` on Today or
    /// with no kept list. Stale, the caller composes `f` into the kept
    /// picture and calls [`Self::today_composed`] (the owner's ruling of
    /// 2026-11-12 on R191, the Reveal pattern: run today-0139 kept Today in
    /// the light look and came back in the dark, half the screen redrawn).
    pub fn today_ahead(&mut self, f: &mut Frame) -> Option<bool> {
        if self.screen == Screen::Today || self.today_kept.is_none() {
            return None;
        }
        let saved = (self.screen, self.menu_open, self.focus, self.pressed);
        (self.screen, self.menu_open, self.focus, self.pressed) =
            (Screen::Today, false, None, None);
        f.clear();
        f.damage(Rect::new(0, 0, W, H));
        self.scene(f);
        self.look(f);
        (self.screen, self.menu_open, self.focus, self.pressed) = saved;
        let k = rp_draw::changes::KEY_WORDS;
        let kept = self.today_kept.as_deref()?;
        Some(
            self.today_kept_n
                .is_some_and(|m| rp_draw::changes::same_keys(f, &kept[..m * k])),
        )
    }

    /// `f` (from [`Self::today_ahead`]) was composed into the kept picture:
    /// a return now marks only where Today differs from it.
    pub fn today_composed(&mut self, f: &Frame) {
        if let Some(kept) = self.today_kept.as_deref_mut() {
            self.today_kept_n = rp_draw::changes::cmd_keys(f, kept);
        }
    }

    /// Whether the last frame left Today (once): the caller keeps the
    /// picture shown before that frame draws.
    pub fn take_left_today(&mut self) -> bool {
        core::mem::take(&mut self.left_today)
    }

    /// Whether the last frame returned to Today marking only where its list
    /// differs from the one kept as it was left (once): the caller starts
    /// that frame from the picture kept then, or, if it can't, marks the
    /// whole screen.
    pub fn take_from_kept(&mut self) -> bool {
        core::mem::take(&mut self.from_kept)
    }

    fn menu_len(&self) -> u8 {
        Self::menu_entries(self.dev_exit)
    }

    /// How many entries the Start menu has (`dev`: a test image's).
    #[must_use]
    pub const fn menu_entries(dev: bool) -> u8 {
        MENU.len() as u8 + if dev { DEV_MENU.len() as u8 } else { 0 }
    }

    fn menu_rect(&self) -> Rect {
        Rect::new(
            MENU_X,
            MENU_Y,
            MENU_W,
            u16::from(self.menu_len()) * MENU_ROW + 8,
        )
    }

    /// Where Start menu entry `i` is.
    #[must_use]
    pub fn menu_entry(i: u8) -> Rect {
        Rect::new(
            MENU_X + 4,
            MENU_Y + 4 + u16::from(i) * MENU_ROW,
            MENU_W - 8,
            MENU_ROW,
        )
    }

    fn menu_label(&self, i: u8) -> &'static str {
        let i = usize::from(i);
        MENU.get(i)
            .or_else(|| DEV_MENU.get(i - MENU.len()))
            .copied()
            .unwrap_or(BACK_TO_LOADER)
    }

    /// Whether the wallpaper-only view is up (the test image's comparison).
    #[must_use]
    pub fn wallpaper_only(&self) -> bool {
        self.bare
    }

    /// The notice naming the comparison's wallpaper.
    fn show_depth(&mut self) {
        self.show_notice(if self.wall_plain {
            SHOWING_PLAIN
        } else {
            SHOWING_DITHERED
        });
    }

    /// Switches the comparison's wallpaper (dithered or plain): everything
    /// is redrawn over the new wallpaper, and a notice names it.
    fn switch_depth(&mut self) -> Action {
        self.wall_plain = !self.wall_plain;
        self.show_depth();
        self.dirty(Rect::new(0, 0, W, H));
        Action::WallpaperDepth {
            dithered: !self.wall_plain,
        }
    }

    /// Where a target is on screen.
    #[must_use]
    pub fn rect_of(&self, t: Target) -> Rect {
        match t {
            Target::Start => START,
            Target::Quick(i) => quick_rect(i),
            Target::SoftLeft => SOFT_LEFT,
            Target::SoftRight => SOFT_RIGHT,
            Target::Menu(i) => Self::menu_entry(i),
            Target::Hero => HERO,
            Target::Tab(i) => weather_app::tab_rect(i),
            Target::Back | Target::Row(_) | Target::Cap(_) => self.settings_rect(t),
            Target::Play => weather_app::PLAY,
            Target::Station => weather_app::STATION_BTN,
            Target::Ok => OK_BOX,
            Target::PlaceMenu => weather_app::PLACE_BTN,
            Target::PlaceItem(i) => Self::place_item(i),
            Target::Alert => self.alert_rect(),
            Target::Timeline => weather_app::TIMELINE,
            Target::Reading(i) => self.reading_rect(i),
            Target::Span(i) => chart::span_chip(i),
            Target::Agenda => self.panel_rects().0,
            Target::TasksLine => self.panel_rects().1,
        }
    }

    /// The targets that can be tapped or focused now, front first.
    fn targets(&self) -> impl Iterator<Item = Target> + '_ {
        let menu = (0..self.menu_len())
            .filter(|_| self.menu_open)
            .map(Target::Menu);
        let (hero, quick, tabs) = match self.screen {
            Screen::Today => (1, QUICK.len() as u8, 0),
            Screen::Weather => (0, 0, weather_app::TAB_NAMES.len() as u8),
            _ => (0, 0, 0),
        };
        let back = u8::from(self.screen != Screen::Today);
        let radar = u8::from(self.screen == Screen::Weather && self.tab == weather_app::RADAR);
        let caps = if self.has_keyboard() {
            weather_settings::CAPS
        } else {
            0
        };
        // The organiser apps' controls and the reminder banner's buttons
        // before what they may lie over (the banner over Today's hero).
        let today = [Target::Start]
            .into_iter()
            .chain(self.shows_ok().then_some(Target::Ok))
            .chain((0..back).map(|_| Target::Back))
            .chain((0..hero).map(|_| Target::Hero))
            // The tasks line before the panel it lies on.
            .chain((0..hero).flat_map(|_| [Target::TasksLine, Target::Agenda]))
            .chain((0..quick).map(Target::Quick))
            .chain((0..tabs).map(Target::Tab))
            .chain(
                (self.screen == Screen::Weather && self.station.is_some())
                    .then_some(Target::Station),
            )
            .chain((self.screen == Screen::Weather).then_some(Target::PlaceMenu))
            .chain(
                (self.screen == Screen::Weather && self.alert_name().is_some())
                    .then_some(Target::Alert),
            )
            .chain(
                (0..self.wset.n_places)
                    .filter(|_| self.place_menu && self.screen == Screen::Weather)
                    .map(Target::PlaceItem),
            )
            .chain((0..self.rows()).map(Target::Row))
            .chain((0..caps).map(Target::Cap))
            .chain((0..radar).flat_map(|_| [Target::Play, Target::Timeline]))
            .chain(
                (0..8)
                    .filter(|&i| self.reading_shown(i))
                    .map(Target::Reading),
            )
            .chain((0..2).filter(|_| self.chart.is_some()).map(Target::Span))
            .chain([Target::SoftLeft, Target::SoftRight])
            // With the menu open only Start (to close it) and the menu act;
            // with a chart open, Start, its chips and the softkeys.
            .filter(|t| !self.menu_open || *t == Target::Start)
            .filter(|t| {
                self.chart.is_none()
                    || matches!(
                        t,
                        Target::Start | Target::Span(_) | Target::SoftLeft | Target::SoftRight
                    )
            });
        menu.chain(today)
    }

    fn hit(&self, x: u16, y: u16) -> Option<Target> {
        self.targets()
            .find(|&t| self.rect_of(t).contains_point(x, y))
    }

    /// Advances time to `now_ms` (any monotonic millisecond count): a
    /// notice past its time goes.
    pub fn tick(&mut self, now_ms: u32) -> Action {
        self.now = now_ms;
        self.net_tick();
        // An app's window opening: its next step, then the whole screen
        // once it is open.
        if let Some((start, from, ms)) = self.prog_open {
            // It begins at the first step after the program was read from
            // the card (0.14 s on the X51v), not at the touch.
            let start = if core::mem::take(&mut self.prog_fresh) {
                self.prog_open = Some((now_ms, from, ms));
                now_ms
            } else {
                start
            };
            if now_ms.wrapping_sub(start) >= ms || self.screen != Screen::Program {
                self.prog_open = None;
                self.dirty(Rect::new(0, 0, W, H));
            } else if let Some(w) = self.prog_window() {
                self.dirty(w);
            }
            return Action::Redraw;
        }
        // A program's motion: its next step when due.
        if let Some(due) = self.prog_anim
            && now_ms.wrapping_sub(due) < 1 << 31
        {
            self.prog_anim = None;
            if self.screen == Screen::Program
                && self.program_event(rp_app::Event::Tick) != Action::Nothing
            {
                return Action::Redraw;
            }
        }
        if self.radar_step(now_ms) && self.anim.is_none() && self.notice.is_none() {
            return Action::Redraw;
        }
        if self.sky_step(now_ms) && self.notice.is_none() {
            return Action::Redraw;
        }
        if self.screen == Screen::Weather && self.scroll.step(now_ms) {
            self.page_moved();
            if self.anim.is_none() && self.notice.is_none() {
                return Action::Redraw;
            }
        }
        if let Some(start) = self.anim {
            // A page arriving: redraw it every pass until it's in place.
            let area = self.page_area();
            // On the planes an entrance moves the layer's window only.
            let planes = self.layer_tab().is_some() || self.reveal_plane().is_some();
            if now_ms.wrapping_sub(start) >= weather_app::ANIM_MS {
                self.anim = None;
                self.expanding = false;
            }
            if !planes {
                self.dirty(area);
            }
            if self.notice.is_none() {
                return Action::Redraw;
            }
        }
        match self.notice {
            Some((_, until)) if now_ms.wrapping_sub(until) < 1 << 31 => {
                self.notice = None;
                self.dirty(NOTICE);
                Action::Redraw
            }
            _ => Action::Nothing,
        }
    }

    /// Sets the time shown; only what changed is redrawn.
    pub fn set_time(&mut self, t: Time) -> Action {
        if t == self.time {
            return Action::Nothing;
        }
        let date_changed =
            (t.weekday, t.day, t.month) != (self.time.weekday, self.time.day, self.time.month);
        let was_light = self.light();
        self.time = t;
        self.dirty(TITLE_CLOCK);
        let _ = date_changed;
        self.reminders_tick();
        // Auto's change at sunrise or sunset: every screen in the other look.
        if self.light() != was_light {
            self.dirty(Rect::new(0, 0, W, H));
        }
        if self.screen == Screen::Weather {
            // The header's "read at" time; the Now page's sunset bar.
            self.dirty(weather_app::HEAD);
            // On the planes the Now page's picture is checked against its
            // draw list when this frame is drawn (the device's).
            if self.tab == 0 && self.layer_tab().is_none() {
                self.dirty(weather_app::PAGE);
            }
        }
        if date_changed {
            self.dirty(DATE_BOX);
            self.dirty(FORECAST_ROW);
        }
        Action::Redraw
    }

    fn set_focus(&mut self, f: Option<Target>) {
        if f == self.focus {
            return;
        }
        if let Some(old) = self.focus {
            self.dirty(self.rect_of(old));
        }
        if let Some(new) = f {
            self.dirty(self.rect_of(new));
        }
        self.focus = f;
    }

    fn set_pressed(&mut self, p: Option<Target>) {
        if p == self.pressed {
            return;
        }
        for t in [self.pressed, p].into_iter().flatten() {
            self.dirty(self.rect_of(t));
            // A key's enlargement above it, and what it covered.
            if let Target::Cap(i) = t
                && let Some(r) = self.kb.pop_rect(i)
            {
                self.dirty(r);
            }
        }
        self.pressed = p;
    }

    /// Shows `what` in the notice for a moment (the caller's word, as a
    /// program that could not be opened).
    pub fn tell(&mut self, what: &'static str) {
        self.show_notice(what);
    }

    fn show_notice(&mut self, what: &'static str) {
        self.notice = Some((what, self.now.wrapping_add(NOTICE_MS)));
        self.dirty(NOTICE);
    }

    fn set_menu(&mut self, open: bool) {
        if open == self.menu_open {
            return;
        }
        // The menu's area, before and after (the Today screen shows again).
        self.dirty(self.menu_rect());
        self.dirty(START);
        self.menu_open = open;
        if matches!(self.focus, Some(Target::Menu(_))) || open {
            let f = if open {
                Some(Target::Menu(0))
            } else {
                Some(Target::Start)
            };
            // Keep the D-pad where the user is only if they were using it.
            let f = if self.focus.is_some() { f } else { None };
            self.set_focus(f);
        }
    }

    fn activate(&mut self, t: Target) -> Action {
        // The chart's softkeys: Close, and the other span.
        if let Some(c) = self.chart {
            match t {
                Target::SoftLeft => {
                    self.close_chart();
                    return Action::Redraw;
                }
                Target::SoftRight => {
                    self.set_chart_span(1 - c.span);
                    return Action::Redraw;
                }
                _ => {}
            }
        }
        match t {
            Target::Start => self.set_menu(!self.menu_open),
            // The first quick button starts a note (the Notes program).
            Target::Quick(0) if self.programs => {
                self.intent = rp_app::INTENT_NEW;
                return Action::OpenProgram(3);
            }
            Target::Quick(i) => {
                self.show_notice(not_yet(QUICK[usize::from(i).min(QUICK.len() - 1)]))
            }
            Target::SoftLeft => match self.screen {
                // Today's left softkey opens Calendar (the Calendar program).
                Screen::Today if self.programs => return Action::OpenProgram(0),
                Screen::Today => self.show_notice(not_yet("Calendar")),
                Screen::Program => {
                    self.program_event(rp_app::Event::Key(rp_app::Key::SoftLeft));
                }
                // Refresh (T4); the header's control goes back.
                Screen::Weather => self.refresh(),
                // Back (Cancel on the keyboard), I7: the left softkey and
                // the header's control do the same.
                _ => self.go_back(),
            },
            Target::SoftRight => match self.screen {
                Screen::Today => self.show_notice(not_yet(self.softkeys().1)),
                Screen::Weather => self.open(Screen::WeatherSettings),
                Screen::KeyEntry | Screen::AddPlace => self.field_done(),
                Screen::WeatherPlaces => {
                    self.removing = !self.removing;
                    self.dirty(Rect::new(0, 0, W, H));
                }
                Screen::WeatherSettings | Screen::WeatherFrom | Screen::Alert => {}
                Screen::Station => self.refresh(),
                Screen::Program => {
                    self.program_event(rp_app::Event::Key(rp_app::Key::SoftRight));
                }
            },
            Target::Hero => self.open_weather(),
            // The calendar panel opens Calendar and its tasks line Tasks
            // (ui.md: each one tap from Today).
            Target::Agenda if self.programs => return Action::OpenProgram(0),
            Target::Agenda => self.show_notice(not_yet("Calendar")),
            Target::TasksLine if self.programs => return Action::OpenProgram(2),
            Target::TasksLine => self.show_notice(not_yet("Tasks")),
            Target::Station => self.open(Screen::Station),
            Target::Ok => self.go_back(),
            Target::PlaceMenu => {
                self.place_menu = !self.place_menu;
                self.dirty(Rect::new(0, 0, W, H));
            }
            Target::PlaceItem(i) => {
                self.place_menu = false;
                if i != self.wset.current {
                    self.wset.current = i;
                    self.settings_changed(true);
                }
                self.dirty(Rect::new(0, 0, W, H));
            }
            Target::Alert => self.open(Screen::Alert),
            Target::Reading(i) => self.open_chart(self.reading_of(i)),
            Target::Span(i) => self.set_chart_span(i),
            Target::Tab(i) => self.set_tab(i),
            Target::Back => self.go_back(),
            Target::Row(i) => self.row_tapped(i),
            Target::Cap(i) => self.cap_tapped(i),
            Target::Play => {
                self.radar_play = !self.radar_play;
                self.radar_at = self.now;
                self.dirty(weather_app::PAGE);
            }
            Target::Timeline => {
                // A pick by the D-pad's Enter keeps the frame; a tap's x is
                // taken in `PenUp`.
                self.radar_play = false;
                self.dirty(weather_app::PAGE);
            }
            Target::Menu(i) => {
                let label = self.menu_label(i);
                if label == BACK_TO_LOADER {
                    return Action::BackToLoader;
                }
                if label == ALIGN_SCREEN {
                    self.start_align();
                    return Action::Redraw;
                }
                if label == SKY_TEST {
                    self.set_menu(false);
                    self.next_sky_test();
                    return Action::Redraw;
                }
                if label == WALLPAPER_COMPARE {
                    self.set_menu(false);
                    self.set_focus(None);
                    self.pressed = None;
                    self.bare = true;
                    self.show_depth();
                    self.dirty(Rect::new(0, 0, W, H));
                    return Action::Redraw;
                }
                let sheet = self.menu_rect();
                self.set_menu(false);
                if self.programs
                    && let Some(n) = ["Calendar", "Contacts", "Tasks", "Notes", "Calculator"]
                        .iter()
                        .position(|&p| p == label)
                {
                    // (It opens out of the menu's sheet.)
                    self.open_from = Some(sheet);
                    return Action::OpenProgram(n as u8);
                }
                if label == "Today" {
                    self.go_today();
                } else {
                    self.show_notice(not_yet(label));
                }
            }
        }
        Action::Redraw
    }

    /// Back to the Today screen (from anywhere).
    fn go_today(&mut self) {
        if self.screen == Screen::Program {
            self.left_app = true;
            *self.program.borrow_mut() = None;
            // What it asked the picture store for goes with it.
            if let Some(pics) = self.app_pics {
                pics.free_owned(PROGRAM_PICS);
            }
            self.shapes.forget();
        }
        if self.screen != Screen::Today {
            // A key half typed is forgotten on the way.
            self.typed = [0; rp_weather::settings::KEY_MAX];
            self.typed_n = 0;
            self.screen = Screen::Today;
            self.place_menu = false;
            self.chart = None;
            self.anim = None;
            self.pressed = None;
            if self.focus.is_some() {
                self.focus = Some(Target::Hero);
            }
            self.dirty(Rect::new(0, 0, W, H));
        }
    }

    /// The softkey bar with labels `(l, r)`.
    fn softkey_bar(&self, f: &mut Frame, (l, r): (&'static str, &'static str)) {
        let ui = self.fonts.ui;
        let app = self.screen != Screen::Today;
        let solid = self.solid_bars();
        let (left, right) = if solid {
            APP_SOFT_LIGHT
        } else if app {
            (APP_SOFT_LEFT, APP_SOFT_RIGHT)
        } else {
            (BAR_LEFT, BAR_RIGHT)
        };
        f.push(Cmd::HGradient {
            r: SOFTKEYS,
            left,
            right,
        });
        if app && !r.is_empty() {
            f.push(Cmd::Fill {
                r: Rect::new(W / 2, SOFTKEYS.y + 6, 1, SOFTKEYS.h - 12),
                color: if solid {
                    APP_SOFT_DIVIDER_LIGHT
                } else {
                    APP_SOFT_DIVIDER
                },
            });
        }
        self.press_under(f, Target::SoftLeft);
        self.press_under(f, Target::SoftRight);
        self.centred(f, SOFT_LEFT, H - 14, ui, TEXT, l.as_bytes());
        self.centred(f, SOFT_RIGHT, H - 14, ui, TEXT, r.as_bytes());
        self.decorate(f, Target::SoftLeft);
        self.decorate(f, Target::SoftRight);
    }

    /// The softkeys' names on the screen up.
    fn softkeys(&self) -> (&'static str, &'static str) {
        if let Some(k) = self.chart_softkeys() {
            return k;
        }
        match self.screen {
            Screen::Today => ("Calendar", "Edit"),
            // T4: Refresh, and Menu (the settings list); the header's
            // control goes back.
            Screen::Weather => ("Refresh", "Menu"),
            Screen::WeatherSettings | Screen::WeatherFrom => ("Back", ""),
            Screen::WeatherPlaces if self.removing => ("Back", "Done"),
            Screen::WeatherPlaces => ("Back", "Remove"),
            Screen::AddPlace => ("Cancel", "Add"),
            Screen::KeyEntry => ("Cancel", "Done"),
            Screen::Station => ("Back", "Refresh"),
            Screen::Alert => ("Back", ""),
            Screen::Program => self
                .program
                .borrow()
                .as_ref()
                .map_or(("Back", ""), |p| p.softkeys()),
        }
    }

    /// Whether a stroke from `(x, y)` is the Weather page's.
    fn on_page(&self, x: u16, y: u16) -> bool {
        self.screen == Screen::Weather
            && !self.menu_open
            && self.chart.is_none()
            && weather_app::PAGE.contains_point(x, y)
    }

    /// The pen moved: a press slid off its target loses its press state;
    /// on the Weather page, past the slop a stroke becomes a scroll (up or
    /// down) or a swipe (across), and stops being a tap.
    fn pen_move(&mut self, x: u16, y: u16) {
        if let Some(p) = self.pressed
            && !self.rect_of(p).contains_point(x, y)
        {
            self.set_pressed(None);
        }
        match self.gesture {
            Gesture::None => {}
            Gesture::Pending { x0, y0 } => {
                let (dx, dy) = (i32::from(x) - i32::from(x0), i32::from(y) - i32::from(y0));
                if dy.abs() >= SLOP && dy.abs() >= dx.abs() && self.tab == weather_app::RADAR {
                    // The map doesn't scroll: a stroke there is nothing.
                    self.gesture = Gesture::None;
                    self.scroll.release(self.now);
                } else if dy.abs() >= SLOP && dy.abs() >= dx.abs() {
                    self.gesture = Gesture::Scroll;
                    self.set_pressed(None);
                    self.scroll_to_pen(y);
                } else if dx.abs() >= SLOP {
                    self.gesture = Gesture::Swipe { x0, dx: 0 };
                    self.set_pressed(None);
                    self.pen_move(x, y);
                }
            }
            Gesture::Scroll => self.scroll_to_pen(y),
            // The chart's own ([`Shell::chart_input`]).
            Gesture::Scrub | Gesture::Sheet { .. } => {}
            Gesture::Swipe { x0, dx } => {
                let d = (i32::from(x) - i32::from(x0)).clamp(-i32::from(W), i32::from(W)) as i16;
                if d != dx {
                    self.gesture = Gesture::Swipe { x0, dx: d };
                    self.dirty(weather_app::TABS);
                }
            }
        }
    }

    /// The list follows the pen to `y`.
    fn scroll_to_pen(&mut self, y: u16) {
        let before = self.scroll.offset();
        self.scroll.drag(y, self.now);
        if self.scroll.offset() != before {
            self.page_moved();
        }
    }

    /// The pen lifted at `(x, y)` from a stroke on the page: `Some` if it
    /// was a scroll or a swipe (not a tap).
    fn pen_lift(&mut self, x: u16, y: u16) -> Option<Action> {
        if let Gesture::Pending { .. } = self.gesture {
            // No moves reported between down and up (a quick stroke): the
            // lift point is the move.
            self.pen_move(x, y);
        }
        let g = core::mem::replace(&mut self.gesture, Gesture::None);
        match g {
            Gesture::Scroll => {
                // T4: let go pulled past the top's rebound: refresh.
                if self.scroll.offset() <= -PULL_REFRESH {
                    self.refresh();
                }
                self.scroll.release(self.now);
                Some(Action::Redraw)
            }
            Gesture::Swipe { dx, .. } => {
                self.dirty(weather_app::TABS);
                if i32::from(dx).abs() >= SWIPE_TURN {
                    let t = if dx < 0 {
                        self.tab + 1
                    } else {
                        self.tab.saturating_sub(1)
                    };
                    self.set_tab(t);
                }
                Some(Action::Redraw)
            }
            // A press that didn't start on the page (another screen, the
            // menu over it) is never a tap on the page: Hourly scrolled, then
            // Home, the Start menu's last entry fell on a grid cell and
            // picked an hour instead of going back to the loader
            // (emu-today, 2026-10-31).
            Gesture::None | Gesture::Scrub | Gesture::Sheet { .. } => None,
            Gesture::Pending { .. } => {
                // Still a tap: the scroll lets go where it is.
                self.scroll.release(self.now);
                // On Hourly's graph: that hour into the card.
                if let Some(h) = self.hour_at(x, y) {
                    self.hour_pick = h;
                    self.dirty(weather_app::PAGE);
                    return Some(Action::Redraw);
                }
                None
            }
        }
    }

    /// How far a swipe in progress has gone (px, negative: left), for the
    /// tabs' underline.
    pub(crate) fn swipe_dx(&self) -> i16 {
        match self.gesture {
            Gesture::Swipe { dx, .. } => dx,
            _ => 0,
        }
    }

    /// Starts the align screen (over everything until it finishes or Home).
    pub fn start_align(&mut self) {
        self.menu_open = false;
        self.pressed = None;
        self.focus = None;
        self.align = Some(align::Align::new());
        self.dirty(Rect::new(0, 0, W, H));
    }

    /// Whether the align screen is up.
    #[must_use]
    pub fn aligning(&self) -> bool {
        self.align.is_some()
    }

    /// Whether the shell wants raw touch readings ([`Input::Raw`]) rather
    /// than screen points: while the align screen is up.
    #[must_use]
    pub fn wants_raw(&self) -> bool {
        self.aligning()
    }

    fn align_input(&mut self, e: Input) -> Action {
        let Some(a) = self.align.as_mut() else {
            return Action::Nothing;
        };
        let step = match e {
            Input::Raw { rx, ry } => a.reading(rx, ry),
            Input::RawUp => a.lift(),
            Input::Key(Key::Home) => {
                // Leaves with the calibration it had.
                self.align = None;
                self.dirty(Rect::new(0, 0, W, H));
                return Action::Redraw;
            }
            _ => align::Step::Same,
        };
        match step {
            align::Step::Same => Action::Nothing,
            align::Step::Changed => {
                self.dirty(Rect::new(0, 0, W, H));
                Action::Redraw
            }
            align::Step::Done(cal) => {
                self.align = None;
                self.dirty(Rect::new(0, 0, W, H));
                Action::Calibrated(cal)
            }
        }
    }

    /// Opens the app program `p` ([`Screen::Program`]).
    pub fn open_program(&mut self, p: &'static mut dyn rp_app::Program) {
        // Out of Today its window opens as Weather's does (the owner on
        // the X51v, 2026-11-15: "entering full screen app with animation
        // doesnt happen with these apps like how it does for the weather
        // app"): it grows from what was touched (the Start menu's sheet,
        // the tile under the stylus, the screen's middle for a button) to
        // the whole window. Weather's page is a picture on the video
        // plane, which an app's live screen is not; an app's window is
        // the renderer's floor and a clip, and only the window is drawn
        // each step: Today stays on the screen round it, not drawn again.
        // From one app to another (its button, pressed in another app:
        // the owner, 2026-11-15, "the switching animation from one full
        // screen app to another needs to make more sense"; it grew from a
        // square in the middle of the old app): the new app comes over
        // the old one from the right edge, the old one left on the screen
        // until it is covered, as a page is turned.
        let from_app = self.screen == Screen::Program || self.left_app;
        self.left_app = false;
        let from = if let Some(menu) = self.open_from.take() {
            menu
        } else if from_app {
            Rect::new(W - 8, TITLE.h, 8, H - TITLE.h)
        } else {
            let (x, y) = self.tap.unwrap_or((W / 2, H / 2));
            Rect::new(
                x.saturating_sub(48).min(W - 96),
                y.saturating_sub(48).clamp(TITLE.h, H - 96),
                96,
                96,
            )
        };
        let opening = (self.screen == Screen::Today || from_app) && self.planes;
        self.set_menu(false);
        // Its pictures are its own from here, given back when it closes.
        if let Some(pics) = self.app_pics {
            pics.free_owned(PROGRAM_PICS);
        }
        self.shapes.forget();
        *self.program.borrow_mut() = Some(p);
        self.screen = Screen::Program;
        self.prog_field = None;
        self.program_keyboard();
        self.pressed = None;
        self.focus = None;
        if opening {
            // The title and the window's first step; the rest as it grows.
            // (Over another app in half the time: a switch, not an
            // entrance.)
            let ms = if from_app {
                weather_app::ANIM_MS / 2
            } else {
                weather_app::ANIM_MS
            };
            self.prog_open = Some((self.now, from, ms));
            self.prog_fresh = true;
            // (What is on the screen stays round the window: nothing
            // else is drawn again until it is open.)
            self.damaged = 0;
            self.dirty(TITLE);
            self.dirty(from);
        } else {
            self.prog_open = None;
            self.dirty(Rect::new(0, 0, W, H));
        }
    }

    /// While an app opens out of Today: the window it shows through so
    /// far (smoothstep, as Weather's Reveal: from rest, fastest half-way).
    #[must_use]
    pub fn prog_window(&self) -> Option<Rect> {
        let (start, from, ms) = self.prog_open.filter(|_| self.screen == Screen::Program)?;
        let t = i64::from(self.now.wrapping_sub(start).min(ms)) * 1024 / i64::from(ms);
        let left = (1024 - t * t * (3 * 1024 - 2 * t) / (1024 * 1024)) as i32;
        let lerp =
            |a: u16, b: u16| (i32::from(b) + (i32::from(a) - i32::from(b)) * left / 1024) as u16;
        Some(Rect::new(
            lerp(from.x, 0),
            lerp(from.y, TITLE.h),
            lerp(from.w, W),
            lerp(from.h, H - TITLE.h),
        ))
    }

    /// The look's setting (Light, Dark, Auto with the sun): the previews
    /// draw each.
    pub fn set_appearance(&mut self, a: rp_weather::settings::Appearance) {
        self.wset.appearance = a;
        self.dirty(Rect::new(0, 0, W, H));
    }

    /// The keyboard follows the program's field (design-language.md
    /// version 2): the pad for numbers, ".com" for e-mail, capitals ready
    /// at the start of a name, an address or a sentence.
    fn program_keyboard(&mut self) {
        let field = self.program.borrow().as_ref().and_then(|p| p.keyboard());
        if field != self.prog_field {
            self.prog_field = field;
            self.kb = keyboard::Keyboard {
                pad: matches!(field, Some(rp_app::Field::Phone | rp_app::Field::Number)),
                email: field == Some(rp_app::Field::Email),
                shift: matches!(
                    field,
                    Some(rp_app::Field::Name | rp_app::Field::Address | rp_app::Field::Sentence)
                ),
                symbols: false,
            };
            self.dirty(Rect::new(0, 0, W, H));
        }
    }

    /// What the next program opened is for ([`rp_app::Opening::intent`]),
    /// taken once.
    pub fn take_intent(&mut self) -> u8 {
        core::mem::take(&mut self.intent)
    }

    /// The organiser's Start menu entries open app programs from now on
    /// ([`Action::OpenProgram`]).
    pub fn use_programs(&mut self, yes: bool) {
        self.programs = yes;
    }

    /// The device's offset from UTC (minutes): online calendars' UTC
    /// times are moved by it.
    pub fn set_utc_offset(&mut self, minutes: i32) {
        self.utc_offset = minutes;
    }

    /// The picture store programs paint into (the renderer's own).
    pub fn give_pics(&mut self, pics: &'static rp_draw::pics::Pics<'static>) {
        self.app_pics = Some(pics);
    }

    /// The Weather app's row-size picture for icon `id` (1-22), lent to
    /// programs (the calendar's days).
    #[must_use]
    pub fn row_icon(&self, id: u8) -> Option<Pic> {
        self.icons
            .row
            .get(usize::from(id.checked_sub(1)?))
            .copied()
            .flatten()
    }

    /// The forecast by day, as programs are lent it.
    pub fn set_app_forecast(&mut self, days: &[rp_app::DayWeather]) {
        let n = days.len().min(self.forecast.len());
        self.forecast[..n].copy_from_slice(&days[..n]);
        self.forecast_n = n as u8;
    }

    /// What a program is lent each call.
    fn program_fonts(&self) -> rp_app::Fonts {
        rp_app::Fonts {
            ui: self.fonts.ui,
            label: self.fonts.label,
            big: self.fonts.big,
            num: self.fonts.num,
            title: self.fonts.title,
        }
    }

    fn program_look(&self) -> rp_app::Look {
        let light = self.light();
        // In its own window over the backdrop: the window's glass and, in
        // the light look, the white gel keys (design-language.md version 3).
        let window = self.backdrops;
        rp_app::Look {
            light,
            panel: if window {
                panel::window(light)
            } else {
                panel::art(light)
            },
            pressed: if window {
                panel::window_pressed(light)
            } else {
                panel::pressed(light)
            },
            // The graphite keys read as holes in the frosted white: the
            // light look's keys are its glass, a shade down under the pen;
            // the chosen one is the blue glow in both.
            key: match (light, window) {
                (true, true) => &panel_art::KEY_LIGHT,
                (true, false) => panel::art(true),
                (false, _) => &panel_art::KEY,
            },
            key_down: match (light, window) {
                (true, true) => &panel_art::KEY_LIGHT_DOWN,
                (true, false) => panel::pressed(true),
                (false, _) => &panel_art::KEY_DOWN,
            },
            on: &panel_art::ON,
        }
    }

    fn program_now(&self) -> rp_app::Now {
        rp_app::Now {
            day: self.today(),
            minute: u16::from(self.time.hour) * 60 + u16::from(self.time.minute),
        }
    }

    /// A program's keyboard and its strip's suggestions, when it types
    /// (its area is drawn after the look's pass, [`Self::program_content`]).
    fn program_scene(&self, f: &mut Frame) {
        let mut words: [[u8; 24]; 3] = [[0; 24]; 3];
        let mut lens = [0usize; 3];
        let typing = {
            let mut p = self.program.borrow_mut();
            let Some(p) = p.as_mut() else { return };
            for (k, w) in p.suggestions().iter().enumerate() {
                let n = w.len().min(24);
                words[k][..n].copy_from_slice(&w[..n]);
                lens[k] = n;
            }
            p.keyboard()
        };
        if typing.is_some() {
            let pressed = match self.pressed {
                Some(Target::Cap(i)) => Some(i),
                _ => None,
            };
            let focus = match self.focus {
                Some(Target::Cap(i)) => Some(i),
                _ => None,
            };
            self.kb.draw_named(
                f,
                (self.fonts.ui, self.fonts.label),
                pressed,
                focus,
                FOCUS,
                b"DONE",
            );
            for (k, w) in words.iter().enumerate() {
                // The suggestion keys sit at a different index on the pad.
                let at = (0..keyboard::CAPS).find_map(|i| match self.kb.cap(i) {
                    Some((keyboard::Cap::Suggest(j), r)) if usize::from(j) == k => Some(r),
                    _ => None,
                });
                if let Some(r) = at
                    && lens[k] > 0
                {
                    // Cut to the key at a character with "..", never over
                    // its neighbour.
                    let ui = self.fonts.ui;
                    let mut n = lens[k];
                    let mut cut = [0u8; 26];
                    while n > 0 && ui.width(&w[..n]) + 6 > r.w {
                        n -= 1;
                    }
                    let shown: &[u8] = if n < lens[k] {
                        let n = n.saturating_sub(1);
                        cut[..n].copy_from_slice(&w[..n]);
                        cut[n..n + 2].copy_from_slice(b"..");
                        &cut[..n + 2]
                    } else {
                        &w[..n]
                    };
                    keyboard::centre_in(f, r, ui, TEXT, shown);
                }
            }
        }
    }

    /// Runs `f` (a call into the app program) with the picture store
    /// naming the program as the owner of what it asks for: only what the
    /// program itself asks for is its own (the shell's pictures made while
    /// an app is up stay the shell's).
    fn owning<R>(&self, f: impl FnOnce() -> R) -> R {
        if let Some(pics) = self.app_pics {
            pics.set_owner(PROGRAM_PICS);
        }
        let r = f();
        if let Some(pics) = self.app_pics {
            pics.set_owner(0);
        }
        r
    }

    /// The open program's area, drawn after the look's pass: a program
    /// chooses its own colours (its rounded shapes' corners are pictures
    /// the pass can't recolour, so the pass would split body from corner).
    fn program_content(&self, f: &mut Frame) {
        if self.screen != Screen::Program {
            return;
        }
        // (Before the program is borrowed: the style asks it its accent.)
        let style = self.style_now();
        let (fonts, look) = (self.program_fonts(), self.program_look());
        let cx = rp_app::Ctx {
            fonts: &fonts,
            look: &look,
            now: self.program_now(),
            pics: self.app_pics,
            forecast: &self.forecast[..usize::from(self.forecast_n)],
            us: true,
            utc_offset: self.utc_offset,
            shapes: &self.shapes,
            clip: &self.clip,
            pim: Some(&self.apps.store),
        };
        let mut p = self.program.borrow_mut();
        let Some(p) = p.as_mut() else { return };
        // Under the keyboard when one is up.
        let area = if p.keyboard().is_some() {
            Rect::new(
                0,
                rp_app::CONTENT.y,
                W,
                keyboard::PANEL.y - rp_app::CONTENT.y,
            )
        } else {
            rp_app::CONTENT
        };
        // In the light look the program's colours take the same rule as
        // the shell's (its own pass ran before the program drew), its
        // corner pictures painted to match.
        let light = self.light();
        self.shapes.set_paint(if light {
            LIGHT_FILLS[match style {
                LightStyle::Grey => 0,
                LightStyle::Frosted => 1,
                LightStyle::Tonal => 2,
                LightStyle::Washed => 3,
                LightStyle::Window => 4,
            }]
        } else {
            |c| c
        });
        // Its own window, no wallpaper behind: the renderer's floor
        // ([`Self::floor`]).
        let start = f.len();
        f.clip(Some(
            self.prog_window().map_or(area, |w| w.intersect(&area)),
        ));
        self.owning(|| p.draw(f, &cx));
        f.clip(None);
        if light {
            f.map_colors_from(start, |c, text| look_color(c, text, style));
        }
    }

    /// The answer to the open program's network question.
    pub(crate) fn program_net(&mut self, ok: bool, body: &[u8]) {
        let (fonts, look) = (self.program_fonts(), self.program_look());
        let reply = {
            let cx = rp_app::Ctx {
                fonts: &fonts,
                look: &look,
                now: self.program_now(),
                pics: self.app_pics,
                forecast: &self.forecast[..usize::from(self.forecast_n)],
                us: true,
                utc_offset: self.utc_offset,
                shapes: &self.shapes,
                clip: &self.clip,
                pim: Some(&self.apps.store),
            };
            let mut p = self.program.borrow_mut();
            match p.as_mut() {
                Some(p) => self.owning(|| p.net_answer(ok, body, &cx)),
                None => rp_app::Reply::Nothing,
            }
        };
        if reply != rp_app::Reply::Nothing {
            self.dirty(Rect::new(0, 0, W, H));
        }
    }

    /// The open program's body for its question in flight (a `PUT` to the
    /// PC or a `CARDPUT`): its bytes from `at` into `out`, how many (0: no
    /// more).
    pub fn program_body(&mut self, at: usize, out: &mut [u8]) -> usize {
        let (fonts, look) = (self.program_fonts(), self.program_look());
        let cx = rp_app::Ctx {
            fonts: &fonts,
            look: &look,
            now: self.program_now(),
            pics: self.app_pics,
            forecast: &self.forecast[..usize::from(self.forecast_n)],
            us: true,
            utc_offset: self.utc_offset,
            shapes: &self.shapes,
            clip: &self.clip,
            pim: Some(&self.apps.store),
        };
        let mut p = self.program.borrow_mut();
        p.as_mut().map_or(0, |p| p.net_body(at, out, &cx))
    }

    /// The open program's question for the device's own card
    /// ([`rp_app::Program::card_ask`]), taken: its name, argument, and (a
    /// `CARDPUT`) the body's length. The device answers it with
    /// [`Shell::card_answer`]; no dock is needed.
    pub fn card_ask(&mut self) -> Option<(&'static [u8], rp_app::Text<64>, usize)> {
        let ask = self.program.borrow_mut().as_mut()?.card_ask()?;
        let len = if ask.what == b"CARDPUT" {
            ask.arg
                .get()
                .split(|&c| c == b' ')
                .next()
                .and_then(|w| core::str::from_utf8(w).ok()?.parse().ok())
                .unwrap_or(0)
        } else {
            0
        };
        Some((ask.what, ask.arg, len))
    }

    /// The device's answer to the open program's card question.
    pub fn card_answer(&mut self, ok: bool, body: &[u8]) {
        self.program_net(ok, body);
    }

    /// Sends `e` to the open program and does what it answers.
    fn program_event(&mut self, e: rp_app::Event) -> Action {
        let (fonts, look) = (self.program_fonts(), self.program_look());
        let reply = {
            let cx = rp_app::Ctx {
                fonts: &fonts,
                look: &look,
                now: self.program_now(),
                pics: self.app_pics,
                forecast: &self.forecast[..usize::from(self.forecast_n)],
                us: true,
                utc_offset: self.utc_offset,
                shapes: &self.shapes,
                clip: &self.clip,
                pim: Some(&self.apps.store),
            };
            let mut p = self.program.borrow_mut();
            match p.as_mut() {
                Some(p) => self.owning(|| p.event(e, &cx)),
                None => rp_app::Reply::Nothing,
            }
        };
        self.program_keyboard();
        self.reminders_follow();
        self.program_caps();
        match reply {
            rp_app::Reply::Nothing => Action::Nothing,
            rp_app::Reply::Redraw => {
                self.dirty(Rect::new(0, 0, W, H));
                Action::Redraw
            }
            rp_app::Reply::Animate => {
                // The next step about a frame later (the 33 ms rule).
                self.prog_anim = Some(self.now.wrapping_add(30));
                self.dirty(Rect::new(0, 0, W, H));
                Action::Redraw
            }
            rp_app::Reply::Close => {
                self.go_today();
                Action::Redraw
            }
        }
    }

    /// The pen in a program's area and keyboard, and the D-pad, while a
    /// program is open; `None` leaves the input to the shell (the bars,
    /// Start, Home, the softkeys).
    fn program_input(&mut self, e: Input) -> Option<Action> {
        if self.screen != Screen::Program || self.menu_open {
            return None;
        }
        self.program_keyboard();
        // The key shown large goes with the stylus, wherever it lifts.
        if matches!(e, Input::PenUp { .. }) && matches!(self.pressed, Some(Target::Cap(_))) {
            self.set_pressed(None);
        }
        let field = self.prog_field;
        let typing = field.is_some();
        let area = |x: u16, y: u16| {
            rp_app::CONTENT.contains_point(x, y)
                && !(typing && keyboard::PANEL.contains_point(x, y))
        };
        match e {
            Input::PenDown { x, y } if typing && keyboard::PANEL.contains_point(x, y) => {
                let cap = (0..keyboard::CAPS)
                    .find(|&i| self.kb.cap(i).is_some_and(|(_, r)| r.contains_point(x, y)));
                let cap = cap.and_then(|i| self.kb.cap(i).map(|c| (i, c.0)))?;
                // The key under the stylus shown large above it while it
                // is down, as in Weather's settings (an app's keyboard
                // types on the touch and never said which key was pressed:
                // the owner on the X51v, 2026-11-15, "it used to show up
                // in a box above the key ... i liked that and now its
                // gone").
                self.set_pressed(Some(Target::Cap(cap.0)));
                let before = self.kb;
                let ev = match self.kb.press(cap.1) {
                    keyboard::Cap::Char(c) => {
                        // Capitals at each word (names, addresses) and each
                        // sentence; the shift comes back by itself.
                        let next = match field {
                            Some(rp_app::Field::Name | rp_app::Field::Address) => c == b' ',
                            Some(rp_app::Field::Sentence) => {
                                (c == b' ' && matches!(self.prog_last, b'.' | b'!' | b'?'))
                                    || c == b'\n'
                            }
                            _ => false,
                        };
                        self.prog_last = c;
                        if next {
                            self.kb.shift = true;
                        }
                        Some(rp_app::Event::Char(c))
                    }
                    keyboard::Cap::Com => {
                        for &c in b".co" {
                            self.program_event(rp_app::Event::Char(c));
                        }
                        Some(rp_app::Event::Char(b'm'))
                    }
                    keyboard::Cap::Suggest(n) => Some(rp_app::Event::Suggestion(n)),
                    // The shared clipboard typed into the field.
                    keyboard::Cap::Paste => {
                        let clip = *self.clip.borrow();
                        let text = clip.get();
                        if text.is_empty() {
                            self.show_notice("Clipboard empty");
                            None
                        } else {
                            for &c in &text[..text.len() - 1] {
                                self.program_event(rp_app::Event::Char(c));
                            }
                            Some(rp_app::Event::Char(text[text.len() - 1]))
                        }
                    }
                    keyboard::Cap::Delete => Some(rp_app::Event::Delete),
                    keyboard::Cap::Done => Some(rp_app::Event::Done),
                    _ => None,
                };
                if before.shift != self.kb.shift || before.symbols != self.kb.symbols {
                    self.dirty(keyboard::PANEL);
                }
                Some(ev.map_or(Action::Redraw, |ev| self.program_event(ev)))
            }
            Input::PenUp { x, y } if typing && keyboard::PANEL.contains_point(x, y) => {
                Some(Action::Nothing)
            }
            Input::PenDown { x, y } if area(x, y) => {
                Some(self.program_event(rp_app::Event::PenDown { x, y }))
            }
            Input::PenMove { x, y } if area(x, y) => {
                Some(self.program_event(rp_app::Event::PenMove { x, y }))
            }
            Input::PenUp { x, y } if area(x, y) => {
                Some(self.program_event(rp_app::Event::PenUp { x, y }))
            }
            // The Start key from inside an app (the owner on the X51v,
            // 2026-11-15: "you cannot select the start menu with the
            // dpad"): the app has the arrows, so Start was out of their
            // reach. Up, where the app has nothing higher to go to, lands
            // on Start; there the centre opens the menu and down gives the
            // arrows back to the app.
            Input::Key(k) if self.focus == Some(Target::Start) => Some(match k {
                Key::Enter => self.activate(Target::Start),
                Key::Down => {
                    self.set_focus(None);
                    Action::Redraw
                }
                _ => Action::Nothing,
            }),
            Input::Key(Key::Up) => {
                let a = self.program_event(rp_app::Event::Key(rp_app::Key::Up));
                if a == Action::Nothing && self.prog_field.is_none() {
                    self.set_focus(Some(Target::Start));
                    return Some(Action::Redraw);
                }
                Some(a)
            }
            Input::Key(k) => {
                let k = match k {
                    Key::Up => rp_app::Key::Up,
                    Key::Down => rp_app::Key::Down,
                    Key::Left => rp_app::Key::Left,
                    Key::Right => rp_app::Key::Right,
                    Key::Enter => rp_app::Key::Enter,
                    Key::Home | Key::Calendar | Key::Contacts | Key::Record => return None,
                };
                Some(self.program_event(rp_app::Event::Key(k)))
            }
            _ => None,
        }
    }

    /// Handles one input event.
    pub fn input(&mut self, e: Input) -> Action {
        match e {
            Input::PenUp { x, y } => self.tap = Some((x, y)),
            Input::Key(_) => self.tap = None,
            _ => {}
        }
        self.open_from = None;
        if self.align.is_some() {
            return self.align_input(e);
        }
        if self.bare {
            // The wallpaper alone: a tap switches the depth, Home goes back.
            return match e {
                Input::PenUp { .. } => self.switch_depth(),
                Input::Key(Key::Home) => {
                    self.bare = false;
                    self.show_depth();
                    self.dirty(Rect::new(0, 0, W, H));
                    Action::Redraw
                }
                _ => Action::Nothing,
            };
        }
        // The program buttons (the owner, 2026-11-14: "physical buttons
        // that are supposed to bring up apps, home button works but the
        // others dont"): from any screen, the app open closed first.
        if let Input::Key(k @ (Key::Calendar | Key::Contacts | Key::Record)) = e {
            if !self.programs {
                return Action::Nothing;
            }
            // Its own button, pressed while the app is open, is the app's
            // to answer (Calendar steps through its views, as Windows
            // Mobile's does; Contacts goes back to its list): it is not
            // closed and opened again.
            let own = match k {
                Key::Calendar => "Calendar",
                Key::Contacts => "Contacts",
                _ => "",
            };
            if self.screen == Screen::Program
                && self
                    .program
                    .borrow()
                    .as_ref()
                    .is_some_and(|p| p.title() == own)
            {
                return self.program_event(rp_app::Event::Key(rp_app::Key::App));
            }
            self.set_menu(false);
            self.go_today();
            let n = match k {
                Key::Calendar => 0,
                Key::Contacts => 1,
                _ => {
                    self.intent = rp_app::INTENT_NEW;
                    3
                }
            };
            return Action::OpenProgram(n);
        }
        // The reminder banner over any screen, before what lies under it
        // (an app program's area took its touches: its keys were dead over
        // an open app). The Start menu lies over the banner.
        if !self.menu_open
            && let Some(a) = self.banner_input(e)
        {
            return a;
        }
        let before = self.damaged;
        if let Some(a) = self.chart_input(e) {
            return match a {
                Action::Nothing if self.damaged != before => Action::Redraw,
                a => a,
            };
        }
        // An app program's area, its keyboard and its keys.
        if let Some(a) = self.program_input(e) {
            return a;
        }
        let act = match e {
            Input::PenDown { x, y } => {
                self.gesture = Gesture::None;
                if self.on_page(x, y) {
                    // A touch catches a moving list; it may still be a tap.
                    let moving = self.scroll.moving();
                    self.scroll.press(y, self.now);
                    self.gesture = if moving {
                        Gesture::Scroll
                    } else {
                        Gesture::Pending { x0: x, y0: y }
                    };
                }
                let t = self.hit(x, y);
                if t.is_none() && self.menu_open && !self.menu_rect().contains_point(x, y) {
                    // A tap outside the open menu closes it.
                    self.set_menu(false);
                }
                // The place menu the same way: left open, it kept the page
                // off its layer and stayed drawn behind Today and the app
                // (the owner's run, 2026-10-31). The press starts no page
                // gesture: it only closes the menu.
                if self.place_menu && !matches!(t, Some(Target::PlaceItem(_) | Target::PlaceMenu)) {
                    self.place_menu = false;
                    self.gesture = Gesture::None;
                    self.dirty(Rect::new(0, 0, W, H));
                }
                self.set_pressed(t);
                Action::Nothing
            }
            Input::PenMove { x, y } => {
                self.pen_move(x, y);
                Action::Nothing
            }
            Input::PenUp { x, y } => {
                if let Some(a) = self.pen_lift(x, y) {
                    self.set_pressed(None);
                    return a;
                }
                let t = self.pressed;
                self.set_pressed(None);
                if t == Some(Target::Timeline) && weather_app::TIMELINE.contains_point(x, y) {
                    self.radar_frame = weather_app::frame_at(x, self.radar_frames());
                }
                match t {
                    Some(t) if self.rect_of(t).contains_point(x, y) => self.activate(t),
                    _ => Action::Nothing,
                }
            }
            Input::Key(Key::Home) => {
                self.set_menu(false);
                self.set_focus(None);
                self.go_today();
                Action::Nothing
            }
            // Raw readings matter only to the align screen.
            Input::Raw { .. } | Input::RawUp => Action::Nothing,
            Input::Key(Key::Enter) => match self.focus {
                Some(t) => self.activate(t),
                None => {
                    self.set_focus(Some(Target::Start));
                    Action::Nothing
                }
            },
            // In the Weather app left and right turn the page, and up and
            // down scroll the Hourly list (WM's keys: the D-pad moves within
            // what is shown).
            Input::Key(k @ (Key::Left | Key::Right))
                if self.screen == Screen::Weather && !self.menu_open =>
            {
                let t = if k == Key::Left {
                    self.tab.saturating_sub(1)
                } else {
                    self.tab + 1
                };
                self.set_tab(t);
                if self.focus.is_some() {
                    self.set_focus(Some(Target::Tab(self.tab)));
                }
                Action::Nothing
            }
            Input::Key(k @ (Key::Up | Key::Down))
                if self.screen == Screen::Weather && self.scroll.max() > 0 && !self.menu_open =>
            {
                self.scroll_by(if k == Key::Up { -1 } else { 1 });
                Action::Nothing
            }
            Input::Key(k) => {
                let next = match self.focus {
                    None => Some(if self.menu_open {
                        Target::Menu(0)
                    } else {
                        Target::Start
                    }),
                    Some(f) => self.neighbour(f, k).or(Some(f)),
                };
                self.set_focus(next);
                Action::Nothing
            }
        };
        match act {
            Action::Nothing if self.damaged != before => Action::Redraw,
            a => a,
        }
    }

    /// The nearest target from `from` in the direction of `k`.
    fn neighbour(&self, from: Target, k: Key) -> Option<Target> {
        // The tasks line lies on the calendar panel: up and down reach the
        // panel, right from it the line, left from the line the panel back;
        // from the line, up and down go on as from the panel.
        let from = match (from, k) {
            (Target::Agenda, Key::Right) => return Some(Target::TasksLine),
            (Target::TasksLine, Key::Left) => return Some(Target::Agenda),
            (Target::TasksLine, Key::Right) => return None,
            (Target::TasksLine, _) => Target::Agenda,
            _ => from,
        };
        let c = |r: Rect| {
            (
                i32::from(r.x) + i32::from(r.w) / 2,
                i32::from(r.y) + i32::from(r.h) / 2,
            )
        };
        let (fx, fy) = c(self.rect_of(from));
        // How far a point is from a span: 0 inside it. Up and down measure
        // a wide target (the hero) from its nearest edge, so it is reachable
        // straight down or up from anything above or below it.
        let off = |p: i32, a: u16, w: u16| {
            let (a, b) = (i32::from(a), i32::from(a) + i32::from(w));
            if p < a {
                a - p
            } else if p > b {
                b - p
            } else {
                0
            }
        };
        self.targets()
            .filter(|&t| t != from && t != Target::TasksLine)
            .filter_map(|t| {
                let r = self.rect_of(t);
                let (x, y) = c(r);
                let (dx, dy) = (x - fx, y - fy);
                let (along, across) = match k {
                    Key::Up => (-dy, off(fx, r.x, r.w)),
                    Key::Down => (dy, off(fx, r.x, r.w)),
                    // Left and right stay on their row: centres, as before.
                    Key::Left => (-dx, dy),
                    Key::Right => (dx, dy),
                    _ => return None,
                };
                // In that direction; nearest, straightest. Up and down accept
                // 45 degrees; left and right stay near their row (the quick
                // row sits just above the softkeys: Left from the right
                // softkey must reach the left one, not a quick button).
                let wide = if matches!(k, Key::Left | Key::Right) {
                    4
                } else {
                    1
                };
                (along > 0 && wide * across.abs() <= along).then_some((along + 2 * across.abs(), t))
            })
            .min_by_key(|&(d, _)| d)
            .map(|(_, t)| t)
    }

    /// Fills `f` with the changed rectangles and the whole scene, and
    /// forgets the changes.
    pub fn frame(&mut self, f: &mut Frame) {
        // The page handed to the layer or back: its area on screen 0 is
        // the wallpaper or the page.
        let layer = self.layer_tab().is_some();
        if layer != self.layer_was {
            self.layer_was = layer;
            self.dirty(weather_app::PAGE);
        }
        // Leaving Today, or back to it (the owner's ruling of 2026-11-12
        // on R191): back, with Today's list kept as it was left, the frame
        // marks only where its list differs from that one (the caller
        // starts it from the picture kept then), not the change of screen.
        let was = self.drawn_screen.replace(self.screen);
        let today = self.screen == Screen::Today;
        let leaving = !today && was == Some(Screen::Today);
        let from_kept = today
            && was.is_some_and(|s| s != Screen::Today)
            && self.today_kept_n.is_some()
            && self.shown.is_some();
        f.clear();
        if !from_kept {
            for d in &self.damage[..self.damaged] {
                f.damage(*d);
            }
        }
        self.damaged = 0;
        // An app's own window under it (R201), carried by the frame; where
        // it came, went or changed, everything under it is new (not on the
        // way back to Today's kept picture, already right).
        let floor = self.floor();
        // (An app opening: its window's steps are marked as it grows.)
        if floor != self.floor_was && !from_kept && self.prog_open.is_none() {
            f.damage(Rect::new(0, 0, W, H));
        }
        self.floor_was = floor;
        f.set_floor(floor);
        f.set_floor_picture(self.backdrops && floor.is_some());
        let keys = self.softkeys();
        self.left_app = false;
        self.scene(f);
        self.look(f);
        self.program_content(f);
        // A program that changed its screen as it drew (Notes opened for
        // a new note: its softkey read "New" on the note until something
        // else redrew the bar, found by the apps check, 2026-11-15): the
        // frame made again, once, with the bar it has now.
        if self.softkeys() != keys {
            self.dirty(Rect::new(0, 0, W, H));
            self.drawn_screen = was;
            return self.frame(f);
        }
        if self.screen == Screen::Program {
            // Over the app: its overlays, in the look's colours.
            let start = f.len();
            self.overlays(f);
            if self.light() {
                let style = self.style_now();
                f.map_colors_from(start, |c, text| look_color(c, text, style));
            }
        }
        // The list kept for the next frame, and a restatement's areas:
        // where this list differs from the last one drawn.
        let restated = core::mem::take(&mut self.restated);
        let Some(shown) = self.shown.as_deref_mut() else {
            if restated {
                f.damage(Rect::new(0, 0, W, H));
            }
            return;
        };
        let (lo, hi) = shown.split_at_mut(LIST_WORDS);
        let (last, now) = if self.shown_at == 0 {
            (lo, hi)
        } else {
            (hi, lo)
        };
        let k = rp_draw::changes::KEY_WORDS;
        let n = rp_draw::changes::cmd_keys(f, now);
        if leaving {
            // Today's last list, as the picture the caller keeps shows it.
            self.left_today = true;
            self.today_kept_n = None;
            if let (Some(kept), Some(m)) = (self.today_kept.as_deref_mut(), self.shown_n) {
                kept[..m * k].copy_from_slice(&last[..m * k]);
                self.today_kept_n = Some(m);
            }
        }
        if from_kept {
            let kept = self.today_kept.as_deref().unwrap_or_default();
            match (self.today_kept_n, n) {
                (Some(m), Some(n)) => {
                    rp_draw::changes::changed_areas(&kept[..m * k], &now[..n * k], |r| {
                        f.damage(r);
                    });
                    self.from_kept = true;
                }
                _ => f.damage(Rect::new(0, 0, W, H)),
            }
        } else if restated {
            match (self.shown_n, n) {
                (Some(m), Some(n)) => {
                    rp_draw::changes::changed_areas(&last[..m * k], &now[..n * k], |r| {
                        f.damage(r);
                    });
                }
                _ => f.damage(Rect::new(0, 0, W, H)),
            }
        }
        self.shown_at ^= 1;
        self.shown_n = n;
    }

    /// Whether the light look is on: the settings' Light, or Auto between
    /// sunrise and sunset at the place shown (ui.md "Light and dark":
    /// decided by the owner, both modes, a toggle and a change with the time
    /// of day).
    #[must_use]
    pub fn light(&self) -> bool {
        match self.wset.appearance {
            rp_weather::settings::Appearance::Light => true,
            rp_weather::settings::Appearance::Dark => false,
            rp_weather::settings::Appearance::Auto => {
                let m = u16::from(self.time.hour) * 60 + u16::from(self.time.minute);
                m >= self.wx.sun.0 && m < self.wx.sun.1
            }
        }
    }

    /// The look on a frame just drawn: in the light look every neutral
    /// colour (glass, bars, text, edges) turned to its light twin (ui.md:
    /// frosted white glass and dark text over the same photo); accents
    /// (the focus blue, rain, the temperature colours) and pictures keep
    /// theirs. One switch for every screen, so none is forgotten in one
    /// look (the owner's run, 2026-10-31: Light and Dark changed nothing,
    /// R160).
    /// The light style of the screen up: the window's where an app's own
    /// window is under it with its backdrop, the setting elsewhere.
    fn style_now(&self) -> LightStyle {
        if self.windowed() {
            LightStyle::Window
        } else {
            self.light_style
        }
    }

    fn look(&self, f: &mut Frame) {
        if self.light() {
            let style = self.style_now();
            f.map_colors(|c, text| look_color(c, text, style));
        }
    }

    /// The previews' screens without their panels (the mock-ups of panel
    /// designs composite theirs under the words).
    pub fn preview_panels_off(&mut self, off: bool) {
        self.panels_off = off;
        self.dirty(Rect::new(0, 0, W, H));
    }

    /// The light look's style (the owner's pick of the three drawn,
    /// 2026-11-01; the previews draw each).
    pub fn set_light_style(&mut self, style: LightStyle) {
        self.light_style = style;
        self.dirty(Rect::new(0, 0, W, H));
    }

    /// Where Today's tasks line goes (the previews draw each).
    pub fn set_today_tasks(&mut self, t: TodayTasks) {
        self.today_tasks = t;
        self.dirty(Rect::new(0, 0, W, H));
    }

    /// Today's calendar panel and its tasks line, as the tasks line's
    /// place has them.
    fn panel_rects(&self) -> (Rect, Rect) {
        let p = CALENDAR_PANEL;
        match self.today_tasks {
            TodayTasks::Header => (p, TASKS_LINE),
            TodayTasks::Row => (p, Rect::new(p.x + 4, p.y + 66, p.w - 8, 30)),
            TodayTasks::Extra => {
                let p = Rect::new(p.x, p.y, p.w, p.h + 28);
                (p, Rect::new(p.x + 4, p.y + 94, p.w - 8, 30))
            }
        }
    }

    /// What fills an app's window behind it (the previews draw each).
    pub fn set_window_body(&mut self, b: WindowBody) {
        self.window_body = b;
        self.dirty(Rect::new(0, 0, W, H));
    }

    /// The host paints each window's backdrop (design-language.md version
    /// 3, light behind the glass: `rp_app::aurora`) into the renderer's
    /// floor picture: before drawing a frame whose `floor_picture` is set,
    /// it paints [`Self::backdrop_key`]'s if that isn't the one painted.
    pub fn use_backdrops(&mut self, yes: bool) {
        self.backdrops = yes;
        self.dirty(Rect::new(0, 0, W, H));
    }

    /// The backdrop the screen up wants: its tint and whether the look is
    /// light; `None` where there is no window (Today, the radar).
    #[must_use]
    pub fn backdrop_key(&self) -> Option<(Argb, bool)> {
        self.floor()?;
        Some((self.window_tint()?, self.light()))
    }

    /// The tint of the window up: the app's accent, Weather's sky.
    fn window_tint(&self) -> Option<Argb> {
        match self.screen {
            Screen::Today => None,
            Screen::Program => self.program.borrow().as_ref().and_then(|p| p.accent()),
            _ => Some(rp_app::kit::tint::WEATHER),
        }
    }

    /// An app's bars drawn solid in the light look (the dark look's are
    /// opaque already; the light look's pass would make them glass).
    fn solid_bars(&self) -> bool {
        (self.floor().is_some() || self.radar_window().is_some()) && self.light()
    }

    /// The open app's window body for the renderer (`set_floor` on
    /// both): its area and its row's left and right colours, opaque, in
    /// the look chosen; `None` off an app (the wallpaper). The renderer
    /// puts it wherever it would read the wallpaper, so the program's glass
    /// blends over the window, not the photo, at no cost per frame.
    #[must_use]
    pub fn floor(&self) -> Option<(Rect, Argb, Argb)> {
        // Weather full screen too (the owner, 2026-11-14: "weather app,
        // when its full screen, still shows the wallpaper"), but not over
        // the radar (its map is the backdrop) or while it opens out of
        // Today (Reveal's window over the Today screen).
        let weather = self.screen != Screen::Today && self.screen != Screen::Program;
        if self.screen == Screen::Today
            || weather && (self.backdrop().is_some() || self.reveal_window().is_some())
        {
            return None;
        }
        let light = self.light();
        let (left, right) = if light { BODY_LIGHT } else { BODY_DARK };
        let (left, right) = match self.window_body {
            WindowBody::Solid => (left, left),
            WindowBody::Aero => (left, right),
            WindowBody::Tinted => {
                let tint = self.window_tint().unwrap_or(left);
                let k = if light { 6 } else { 10 };
                (mix(left, tint, k, 100), mix(right, tint, k, 100))
            }
        };
        // The whole screen (the light look's glass bars too), or the
        // window an app opens through.
        Some((
            self.prog_window().unwrap_or(Rect::new(0, 0, W, H)),
            left,
            right,
        ))
    }

    fn glass(&self, f: &mut Frame, r: Rect, left: Argb, right: Argb) {
        if self.panels_off {
            return;
        }
        // Every box the same glass (the owner, 2026-11-01): the tints
        // once passed here belonged to the flat look before it.
        let _ = (left, right);
        self.card(f, r);
    }

    /// Whether the screen up is in its own window over a backdrop the
    /// host paints (design-language.md version 3).
    pub(crate) fn windowed(&self) -> bool {
        // (The radar's window is painted round its map: no floor.)
        self.backdrops && (self.floor().is_some() || self.backdrop().is_some())
    }

    /// A glass card in the look: the window's thin glass in a window, the
    /// wallpaper's elsewhere (one place decides, for every shell screen).
    pub(crate) fn card(&self, f: &mut Frame, r: Rect) {
        if self.windowed() {
            panel::draw_window(f, r, self.light());
        } else {
            panel::draw(f, r, self.light());
        }
    }

    fn centred(
        &self,
        f: &mut Frame,
        r: Rect,
        baseline: u16,
        font: &'static Font,
        color: Argb,
        text: &[u8],
    ) {
        let w = font.width(text);
        let x = r.x + r.w.saturating_sub(w) / 2;
        f.text(x, baseline, font, color, text);
    }

    /// A pressed button's solid fill, drawn under its label (every button
    /// but the weather card, which darkens).
    fn press_under(&self, f: &mut Frame, t: Target) {
        if self.pressed == Some(t) {
            f.push(Cmd::Fill {
                r: self.rect_of(t),
                color: PRESSED_UNDER,
            });
        }
    }

    fn decorate(&self, f: &mut Frame, t: Target) {
        let r = self.rect_of(t);
        // The weather card opens the app: pressed, its glass darkens
        // (`today_scene`; a pale fill over its glass read as the screen
        // flashing white, the owner's photo 2026-10-30, and a dark one
        // replaced the card, words and picture, with the wallpaper, his
        // video 2026-11-01). Every other button's press is solid, under its
        // label (`press_under`).
        if self.focus == Some(t) {
            f.push(Cmd::Outline { r, color: FOCUS });
            f.push(Cmd::Outline {
                r: Rect::new(
                    r.x + 1,
                    r.y + 1,
                    r.w.saturating_sub(2),
                    r.h.saturating_sub(2),
                ),
                color: FOCUS,
            });
        }
    }

    fn align_scene(&self, f: &mut Frame, a: &align::Align) {
        let (ui, label) = (self.fonts.ui, self.fonts.label);
        let all = Rect::new(0, 0, W, H);
        f.push(Cmd::Fill {
            r: all,
            color: ALIGN_BACK,
        });
        self.centred(f, all, 150, label, TEXT, b"ALIGN SCREEN");
        let (line1, line2): (&[u8], &[u8]) = match a.stage() {
            align::Stage::Target(_) => (
                b"Touch the center of the target",
                b"and hold until it turns green.",
            ),
            align::Stage::Check(_) => (b"Now touch the dot to check.", b""),
        };
        self.centred(f, all, 200, ui, TEXT, line1);
        self.centred(f, all, 226, ui, TEXT, line2);
        let mut buf = [0u8; 32];
        if let align::Stage::Target(i) = a.stage() {
            let n = b'1' + i;
            let s = b"Point 0 of 5";
            buf[..s.len()].copy_from_slice(s);
            buf[6] = n;
            self.centred(f, all, 262, ui, DIM, &buf[..s.len()]);
        }
        if let Some(note) = a.note() {
            self.centred(f, all, 400, ui, FOCUS, note.as_bytes());
        }
        self.centred(f, all, 610, ui, DIM, b"Home: leave without changing");
        let (x, y) = a.target();
        let color = if a.held() { HELD } else { TEXT };
        match a.stage() {
            align::Stage::Target(_) => {
                // A crosshair: the centre is the point.
                f.push(Cmd::Fill {
                    r: Rect::new(x - 14, y - 1, 29, 3),
                    color,
                });
                f.push(Cmd::Fill {
                    r: Rect::new(x - 1, y - 14, 3, 29),
                    color,
                });
                f.push(Cmd::Outline {
                    r: Rect::new(x - 8, y - 8, 17, 17),
                    color,
                });
            }
            align::Stage::Check(_) => {
                f.push(Cmd::Fill {
                    r: Rect::new(x - 4, y - 4, 9, 9),
                    color,
                });
            }
        }
    }

    fn scene(&self, f: &mut Frame) {
        if let Some(a) = &self.align {
            self.align_scene(f, a);
            return;
        }
        if self.bare {
            // Nothing over the wallpaper but the depth's notice, for its
            // two seconds.
            if let Some(n) = self.notice() {
                f.push(Cmd::Fill {
                    r: NOTICE,
                    color: NOTICE_BACK,
                });
                self.centred(f, NOTICE, H - 14, self.fonts.ui, TEXT, n.as_bytes());
            }
            return;
        }
        let ui = self.fonts.ui;
        if self.light() && self.light_style == LightStyle::Washed {
            f.push(Cmd::Fill {
                r: Rect::new(0, 0, W, H),
                color: WASH,
            });
        }

        // Title bar (the agreed mix mock-up's, Windows Mobile's layout): a
        // boxed Start, the app's name, the status and time, and ok (back)
        // in an app.
        let app = self.screen != Screen::Today;
        let (left, right) = if self.solid_bars() {
            APP_TITLE_LIGHT
        } else if app {
            (APP_TITLE_LEFT, APP_TITLE_RIGHT)
        } else {
            (BAR_LEFT, BAR_RIGHT)
        };
        f.push(Cmd::HGradient {
            r: TITLE,
            left,
            right,
        });
        let label = self.fonts.label;
        self.press_under(f, Target::Start);
        f.push(Cmd::Outline {
            r: START_BOX,
            color: EDGE_LIGHT,
        });
        f.push(Cmd::Outline {
            r: Rect::new(START_BOX.x + 6, START_BOX.y + 5, 16, 16),
            color: TEXT,
        });
        keyboard::centre_in(
            f,
            Rect::new(START_BOX.x + 6, START_BOX.y + 5, 16, 16),
            label,
            TEXT,
            b"R",
        );
        f.text(START_BOX.x + 30, 25, ui, TEXT, b"Start");
        self.decorate(f, Target::Start);
        if app {
            f.text(
                START_BOX.x + START_BOX.w + 16,
                25,
                ui,
                TEXT,
                self.title().as_bytes(),
            );
            if self.shows_ok() {
                self.press_under(f, Target::Ok);
                f.push(Cmd::Outline {
                    r: OK_BOX,
                    color: EDGE_LIGHT,
                });
                keyboard::centre_in(f, OK_BOX, ui, TEXT, b"ok");
                self.decorate(f, Target::Ok);
            }
        }
        let right = if self.shows_ok() {
            OK_BOX.x - 10
        } else {
            W - 10
        };
        let hm = self.region.time(self.time.hour, self.time.minute, true);
        let w = ui.width(hm.as_bytes());
        f.text(right - w, 25, ui, TEXT, hm.as_bytes());
        // Battery: not read yet, drawn as an outline.
        let bx = right - w - 44;
        f.push(Cmd::Outline {
            r: Rect::new(bx, 12, 28, 13),
            color: DIM,
        });
        f.push(Cmd::Fill {
            r: Rect::new(bx + 28, 16, 3, 5),
            color: DIM,
        });
        // Online through the dock: a ring with a dot (only when it is).
        if self.dock {
            let c = Rect::new(bx - 26, 11, 15, 15);
            f.push(Cmd::Outline { r: c, color: TEXT });
            f.push(Cmd::Fill {
                r: Rect::new(c.x + 5, c.y + 5, 5, 5),
                color: TEXT,
            });
        }

        // On the plane path the app is drawn whole (aside) and the window
        // is the video plane's.
        let reveal = self
            .reveal_window()
            .map(|(r, _)| r)
            .filter(|_| !self.planes);
        // A chart open: what is under its sheet isn't drawn (its pictures
        // would take the 2D engine's cache from the chart's).
        if self.chart.is_some() {
            f.clip(Some(Rect::new(0, 36, W, chart::SHEET.y - 36)));
        }
        match self.screen {
            Screen::Today => self.today_scene(f),
            Screen::Weather => {
                // Reveal on the 2D renderer: the app inside the opening
                // window over the wallpaper. On the X51v's plane path the
                // window shows the app's picture over the Today screen
                // (sheet weather-gpu-movement.md); the preview composes
                // that look from these frames.
                if reveal.is_some() {
                    f.clip(reveal);
                }
                self.weather_scene(f, reveal);
                if self.place_menu {
                    f.clip(None);
                    self.place_menu_scene(f);
                }
            }
            Screen::WeatherSettings | Screen::WeatherPlaces | Screen::WeatherFrom => {
                self.settings_scene(f);
            }
            Screen::AddPlace => self.add_place_scene(f),
            Screen::KeyEntry => self.key_scene(f),
            Screen::Station => self.station_scene(f),
            Screen::Alert => self.alert_scene(f),
            Screen::Program => {
                f.clip(self.prog_window());
                self.program_scene(f);
            }
        }
        if self.chart.is_some() {
            f.clip(None);
        }
        // A reading pressed or focused, then the chart's sheet over all.
        for i in 0..8 {
            if self.reading_shown(i) {
                self.decorate(f, Target::Reading(i));
            }
        }
        if let Some(c) = self.chart {
            self.chart_scene(f, c);
        }

        // Softkeys and the notice between them.
        self.softkey_bar(f, self.softkeys());
        if reveal.is_some() || self.prog_window().is_some() {
            f.clip(None);
        }
        if let Some(n) = self.notice() {
            self.centred(f, NOTICE, H - 14, ui, FOCUS, n.as_bytes());
        }

        // Over an app the overlays are drawn after its content
        // ([`Self::frame`]): the app's own window would cover them (the
        // owner, 2026-11-14: "the full screen app is covering the start
        // menu").
        if self.screen != Screen::Program {
            self.overlays(f);
        }
    }

    /// What goes over whatever is up: a reminder's banner (the
    /// notification centre's, docs/design/calendar-rule0.md) and the Start
    /// menu over everything.
    fn overlays(&self, f: &mut Frame) {
        let ui = self.fonts.ui;
        self.banner_scene(f);
        if self.menu_open {
            let r = self.menu_rect();
            self.glass(f, r, MENU_LEFT, MENU_RIGHT);
            for i in 0..self.menu_len() {
                let e = Self::menu_entry(i);
                self.press_under(f, Target::Menu(i));
                f.text(e.x + 16, e.y + 24, ui, TEXT, self.menu_label(i).as_bytes());
                self.decorate(f, Target::Menu(i));
            }
        }
    }

    /// Today's calendar panel from the store ([`today_feed`]), in the T1
    /// layout (today-screen-plan.md): "TODAY" and up to two rows of a time
    /// and a title; what is left of today first, all day first, then
    /// tomorrow's in the room they leave; on the top right the tasks line,
    /// "3 TASKS DUE · 1 LATE". A tap on the panel opens Calendar, on the
    /// tasks line Tasks.
    fn calendar_panel(&self, f: &mut Frame) {
        let (ui, label) = (self.fonts.ui, self.fonts.label);
        let (p, line) = self.panel_rects();
        let now = self.program_now();
        let feed = {
            let mut feed = self.feed.borrow_mut();
            let store = self.apps.store.try_borrow().ok();
            feed.update(store.as_ref().and_then(|s| s.as_ref()), now.day, now.minute);
            *feed
        };
        if self.pressed == Some(Target::Agenda) && !self.panels_off {
            panel::draw_pressed(f, p, self.light());
        } else {
            self.glass(f, p, GLASS_LEFT, GLASS_RIGHT);
        }
        // B gives the second row to the tasks.
        let cap = if self.today_tasks == TodayTasks::Row {
            1
        } else {
            2
        };
        let all = &feed.rows[..usize::from(feed.n)];
        let rows = &all[..all.len().min(cap)];
        let more =
            u16::from(feed.more) + all[rows.len()..].iter().filter(|r| !r.tomorrow).count() as u16;
        let head: &[u8] = if !rows.is_empty() && rows.iter().all(|r| r.tomorrow) {
            b"TOMORROW"
        } else {
            b"TODAY"
        };
        f.text(p.x + 16, p.y + 28, label, DIM, head);
        // The tasks line, right-aligned: the count due, then the late ones
        // in the late colour (the words say it too: colour never alone).
        if feed.ready && self.today_tasks != TodayTasks::Header {
            // B and C: a row of its own: the count, the most pressing
            // task, the late ones.
            self.press_under(f, Target::TasksLine);
            let y = line.y + 22;
            let due = if feed.due == 0 {
                Line::new().push(b"No tasks")
            } else {
                Line::new().num(i32::from(feed.due)).push(b" due")
            };
            f.text(p.x + 16, y, ui, DIM, due.as_bytes());
            let mut room = p.w - 112 - 16;
            if feed.late > 0 {
                let late = Line::new().num(i32::from(feed.late)).push(b" late");
                let lw = ui.width(late.as_bytes());
                f.text(
                    p.x + p.w - 16 - lw,
                    y,
                    ui,
                    rp_app::kit::LATE,
                    late.as_bytes(),
                );
                room -= lw + 12;
            }
            if feed.due == 0 {
                f.text(p.x + 112, y, ui, DIM, b"All done for today");
            } else {
                rp_app::kit::fitted(f, p.x + 112, y, ui, TEXT, feed.next_task.title(), room);
            }
            self.decorate(f, Target::TasksLine);
        } else if feed.ready {
            self.press_under(f, Target::TasksLine);
            let mut due = Line::new();
            if feed.due == 0 {
                due = due.push(b"NO TASKS DUE");
            } else {
                due = due.num(i32::from(feed.due)).push(if feed.due == 1 {
                    b" TASK DUE"
                } else {
                    b" TASKS DUE"
                });
            }
            let late = if feed.late > 0 {
                Line::new()
                    .push(b"  \x80  ")
                    .num(i32::from(feed.late))
                    .push(b" LATE")
            } else {
                Line::new()
            };
            let right = line.x + line.w - 10;
            let lw = label.width(late.as_bytes());
            let dw = label.width(due.as_bytes());
            let x = right - lw - dw;
            f.text(x, p.y + 28, label, DIM, due.as_bytes());
            f.text(x + dw, p.y + 28, label, rp_app::kit::LATE, late.as_bytes());
            self.decorate(f, Target::TasksLine);
        }
        // The rows: a time (or "All day", or "Tomorrow" beside today's)
        // and the title, cut to the panel.
        let mixed = rows.iter().any(|r| !r.tomorrow);
        for (k, r) in rows.iter().enumerate() {
            let y = p.y + 58 + k as u16 * 28;
            let when = match r.at {
                _ if r.tomorrow && mixed => Line::new().push(b"Tomorrow"),
                Some(m) => self.region.time((m / 60) as u8, (m % 60) as u8, true),
                None => Line::new().push(b"All day"),
            };
            f.text(p.x + 16, y, ui, DIM, when.as_bytes());
            let mut room = p.w - 112 - 16;
            // "+2" after the last row: today's that did not fit.
            if k + 1 == rows.len() && more > 0 {
                let more = Line::new().push(b"+").num(i32::from(more));
                let mw = ui.width(more.as_bytes());
                f.text(p.x + p.w - 16 - mw, y, ui, DIM, more.as_bytes());
                room -= mw + 12;
            }
            rp_app::kit::fitted(f, p.x + 112, y, ui, TEXT, r.title(), room);
        }
        if rows.is_empty() {
            let words: &[u8] = if feed.ready {
                b"Nothing more today"
            } else {
                b"Calendar loading"
            };
            f.text(p.x + 16, p.y + 58, ui, DIM, words);
        }
        self.decorate(f, Target::Agenda);
    }

    /// The Today screen under the title bar: the weather hero and the next
    /// five days (from the shell's forecast), the calendar, the quick
    /// buttons.
    fn today_scene(&self, f: &mut Frame) {
        let (ui, label) = (self.fonts.ui, self.fonts.label);
        let (wx, rg) = (&self.wx, &self.region);

        // The weather hero (a tap opens the Weather app), the living sky
        // behind its words and picture.
        if self.pressed == Some(Target::Hero) && !self.panels_off {
            panel::draw_pressed(f, HERO, self.light());
        } else {
            self.glass(f, HERO, GLASS_LEFT, GLASS_RIGHT);
        }
        self.sky_scene(f);
        // The card, as the Weather app's Now panel draws it (Reveal opens
        // from one to the other with nothing changing).
        self.weather_card(f, HERO.x, i32::from(HERO.y), 0);
        self.decorate(f, Target::Hero);

        // The next five days: the card, its icon over the card's top edge,
        // the day name and the high and low.
        const DAYS: [&[u8]; 7] = [b"MON", b"TUE", b"WED", b"THU", b"FRI", b"SAT", b"SUN"];
        for (i, d) in wx.days[1..6].iter().enumerate() {
            let r = Rect::new(
                FORECAST_ROW.x + i as u16 * FORECAST_STEP,
                FORECAST_TOP,
                FORECAST_W,
                FORECAST_H,
            );
            self.glass(f, r, GLASS_LEFT, GLASS_RIGHT);
            // The icon's 66 px box reaches 30 px above the card (as in the mock-ups).
            if let Some(pic) = self.icons.card[usize::from(Icon::of(d.sky).id() - 1)] {
                f.push(Cmd::Pic {
                    x: r.x + r.w.saturating_sub(pic.w) / 2,
                    y: ((r.y + 36).saturating_sub(66) + 66u16.saturating_sub(pic.h) / 2)
                        .cast_signed(),
                    pic,
                });
            }
            self.centred(
                f,
                r,
                r.y + 44,
                label,
                DIM,
                DAYS[usize::from(d.weekday.min(6))],
            );
            let hl = rg.temp(d.high).push(b" ").push(rg.temp(d.low).as_bytes());
            self.centred(f, r, r.y + r.h - 8, ui, TEXT, hl.as_bytes());
        }

        self.calendar_panel(f);

        // Quick buttons.
        for (i, name) in QUICK.iter().enumerate() {
            let t = Target::Quick(i as u8);
            let r = self.rect_of(t);
            self.glass(f, r, GLASS_LEFT, GLASS_RIGHT);
            self.press_under(f, t);
            self.centred(f, r, r.y + r.h - 14, ui, TEXT, name.as_bytes());
            self.decorate(f, t);
        }
    }
}

/// Text in capitals (the label font's spaced capitals).
pub(crate) fn upper(s: &str) -> Line {
    let mut l = Line::new();
    for b in s.bytes() {
        l = l.push(&[b.to_ascii_uppercase()]);
    }
    l
}

/// Every notice the shell shows: each must fit the notice's area between
/// the softkeys in the real font (rp-today's test; 2026-10-30: two
/// settings notices ran past it and were left on screen).
pub const NOTICES: [&str; 31] = [
    "40 at most",
    "64 at most",
    "Asking your PC",
    "Clipboard empty",
    "Keeps one place",
    "Key forgotten",
    "Key kept",
    "Loading icons",
    "No key copied",
    "No spaces",
    "Not connected yet",
    "Not yet: Browser",
    "Not yet: Calendar",
    "Not yet: Documents",
    "Not yet: Edit",
    "Not yet: Files",
    "Not yet: Games",
    "Not yet: Inbox",
    "Not yet: Media",
    "Not yet: Menu",
    "Not yet: Notes",
    "Not yet: Places",
    "Not yet: Programs",
    "Not yet: Reader",
    "Not yet: Record",
    "Not yet: Settings",
    "Not yet: Tasks",
    "Nothing to paste",
    "Pack not on the card",
    "Refreshing",
    "Ten places at most",
];

/// The notice's area's width.
pub const NOTICE_W: u16 = NOTICE.w;

/// "Not yet: <name>" for the entries without a program yet.
fn not_yet(name: &str) -> &'static str {
    match name {
        "Note" | "Notes" => "Not yet: Notes",
        "Record" => "Not yet: Record",
        "Reader" => "Not yet: Reader",
        "Media" => "Not yet: Media",
        "Games" => "Not yet: Games",
        "Calendar" => "Not yet: Calendar",
        "Edit" => "Not yet: Edit",
        "Tasks" => "Not yet: Tasks",
        "Documents" => "Not yet: Documents",
        "File Explorer" => "Not yet: Files",
        "Inbox" => "Not yet: Inbox",
        "Browser" => "Not yet: Browser",
        "Programs" => "Not yet: Programs",
        "Settings" => "Not yet: Settings",
        "Places" => "Not yet: Places",
        "Menu" => "Not yet: Menu",
        _ => "Not yet",
    }
}

/// The months' names (the label font spaces capitals).
const MONTHS: [&[u8]; 12] = [
    b"JANUARY",
    b"FEBRUARY",
    b"MARCH",
    b"APRIL",
    b"MAY",
    b"JUNE",
    b"JULY",
    b"AUGUST",
    b"SEPTEMBER",
    b"OCTOBER",
    b"NOVEMBER",
    b"DECEMBER",
];

/// `MON 13 OCTOBER` into `buf` (spaced capitals are the label font's job).
pub(crate) fn date_line(buf: &mut [u8; 32], t: Time) -> &[u8] {
    const DAYS: [&[u8]; 7] = [b"MON", b"TUE", b"WED", b"THU", b"FRI", b"SAT", b"SUN"];
    let day = DAYS[usize::from(t.weekday.min(6))];
    let month = MONTHS[usize::from(t.month.clamp(1, 12) - 1)];
    let d = t.day.clamp(1, 31);
    let digits = [b'0' + d / 10, b'0' + d % 10];
    let dd: &[u8] = if d >= 10 { &digits } else { &digits[1..] };
    let mut n = 0;
    for part in [day, b" ", dd, b" ", month] {
        buf[n..n + part.len()].copy_from_slice(part);
        n += part.len();
    }
    &buf[..n]
}

#[cfg(test)]
mod tests;
