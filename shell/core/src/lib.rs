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
pub mod chart;
mod keyboard;
pub mod motion;
pub mod panel;
mod panel_art;
pub mod sky;
pub mod station;
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
    if t == c || matches!(style, LightStyle::Grey) {
        return t;
    }
    let (r, g, b) = ((c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
    let a = c >> 24;
    // Dark glass (the panels) to near-opaque light glass; light lines
    // (edges, dividers) to soft dark ones.
    if (r + g + b) / 3 < 0x80 {
        let (tr, tg, tb) = match style {
            LightStyle::Tonal => (250, 236, 244),
            _ => (244, 246, 250),
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
const MENU: [&str; 10] = [
    "Today",
    "Calendar",
    "Tasks",
    "Notes",
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
/// 38 px: fourteen rows fit between the title bar and the softkeys.
const MENU_ROW: u16 = 38;

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
const MENU_LEFT: Argb = rp_draw::look::theme(0xEE08_0A0C);
const MENU_RIGHT: Argb = rp_draw::look::theme(0xE02C_3238);
const TEXT: Argb = rp_draw::look::theme(0xFFF0_F2F4);
const DIM: Argb = rp_draw::look::theme(0xFFCC_D3DA);
const FOCUS: Argb = rp_draw::look::theme(0xFF7F_D0FF);
/// A pressed button: solid slate under its label (the owner's pick of
/// three drawn, 2026-10-31: a see-through press, "I do NOT like that";
/// docs/design/mockups/pressed-look-choices.png).
const PRESSED_UNDER: Argb = rp_draw::look::theme(0xFF3C_4656);
/// The align screen's background (opaque: the wallpaper would distract).
const ALIGN_BACK: Argb = rp_draw::look::theme(0xFF0C_1014);
/// A target held long enough.
const HELD: Argb = rp_draw::look::theme(0xFF5C_E07A);

/// How long a notice stays (milliseconds).
const NOTICE_MS: u32 = 2000;

/// Dirty rectangles the shell gathers between frames.
const MAX_DAMAGE: usize = 8;

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
    /// The relay didn't answer the radar's frame list (no dock): said so
    /// on the radar, never a sample's rain instead.
    radar_offline: bool,
    /// A refresh of the radar for the same place (its map and frames kept
    /// until the new list says which still hold), and the frames moved by
    /// their time for the device to apply (P7: [`Shell::take_radar_moves`]).
    radar_refresh: bool,
    radar_moves: Option<[u8; rp_weather::radar::PAST]>,
    radar_asked: u32,
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
    /// The sky's steps so far (its clouds move a whole pixel a step).
    sky_n: u32,
    sky_forced: bool,
    /// The light look's style, and its panels' ([`soft`]).
    light_style: LightStyle,
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
            radar_offline: false,
            radar_refresh: false,
            radar_moves: None,
            radar_asked: 0,
            pack_at: 0,
            station: None,
            station_at: 0,
            history: None,
            history_at: 0,
            chart: None,
            sky_at: 0,
            sky_n: 0,
            sky_forced: false,
            light_style: LightStyle::Frosted,
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
        self.anim.is_some() || self.scroll.moving()
    }

    /// Ends any movement at once, everything in place (pictures for the
    /// preview; the device lets it run).
    pub fn settle(&mut self) {
        if self.anim.take().is_some() {
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
        self.dirty(Rect::new(0, 0, W, H));
    }

    /// The units and formats shown (the settings set them; tests and
    /// previews show a country's at once).
    pub fn set_region(&mut self, r: Region) {
        self.region = r;
        self.dirty(Rect::new(0, 0, W, H));
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
        self.dirty(Rect::new(0, 0, W, H));
    }

    /// One picture decoded (or given back, `None`): `icon` is the pack id
    /// less one. Every screen is redrawn: the pages' draw lists key the
    /// handle, so only a page that shows it is composed again.
    pub fn set_pic(&mut self, size: Size, icon: usize, pic: Option<Pic>) {
        if let Some(p) = self.icons.of_mut(size).get_mut(icon)
            && *p != pic
        {
            *p = pic;
            self.dirty(Rect::new(0, 0, W, H));
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
        self.dirty(Rect::new(0, 0, W, H));
    }

    /// Hourly's graph picture, painted by the device with
    /// [`Shell::paint_graph`] for the forecast [`Shell::graph_key`] names.
    pub fn set_graph(&mut self, pic: Option<Pic>) {
        if self.graph != pic {
            self.graph = pic;
            self.dirty(Rect::new(0, 0, W, H));
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
            if self.chart.is_some() {
                self.dirty(Rect::new(0, 0, W, H));
            }
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
        self.damaged > 0
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
        let today = [Target::Start]
            .into_iter()
            .chain(self.shows_ok().then_some(Target::Ok))
            .chain((0..back).map(|_| Target::Back))
            .chain((0..hero).map(|_| Target::Hero))
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
            Target::Quick(i) => {
                self.show_notice(not_yet(QUICK[usize::from(i).min(QUICK.len() - 1)]))
            }
            Target::SoftLeft => match self.screen {
                Screen::Today => self.show_notice(not_yet(self.softkeys().0)),
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
            },
            Target::Hero => self.open_weather(),
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
                self.set_menu(false);
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
        f.push(Cmd::HGradient {
            r: SOFTKEYS,
            left: if app { APP_SOFT_LEFT } else { BAR_LEFT },
            right: if app { APP_SOFT_RIGHT } else { BAR_RIGHT },
        });
        if app && !r.is_empty() {
            f.push(Cmd::Fill {
                r: Rect::new(W / 2, SOFTKEYS.y + 6, 1, SOFTKEYS.h - 12),
                color: APP_SOFT_DIVIDER,
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

    /// Handles one input event.
    pub fn input(&mut self, e: Input) -> Action {
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
        let before = self.damaged;
        if let Some(a) = self.chart_input(e) {
            return match a {
                Action::Nothing if self.damaged != before => Action::Redraw,
                a => a,
            };
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
            .filter(|&t| t != from)
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
        f.clear();
        for d in &self.damage[..self.damaged] {
            f.damage(*d);
        }
        self.damaged = 0;
        self.scene(f);
        self.look(f);
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
    fn look(&self, f: &mut Frame) {
        if self.light() {
            let style = self.light_style;
            f.map_colors(|c, text| {
                if sky::is_ink(c)
                    || panel::is_panel(c)
                    || c == WASH
                    || c == keyboard::DONE_TEXT
                    || c == weather_app::SHADOW_LIGHT
                {
                    // The living sky's own colours, chosen for each look,
                    // the washed wallpaper's veil, and the dark text on an
                    // accent fill (a lit switch, Done), which stays light
                    // in both looks: mirrored it was light grey on light
                    // blue (1.4:1, the light look's contrast test).
                    c
                } else if text {
                    light_text(c)
                } else {
                    light_glass(c, style)
                }
            });
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

    fn glass(&self, f: &mut Frame, r: Rect, left: Argb, right: Argb) {
        if self.panels_off {
            return;
        }
        // Every box the same glass (the owner, 2026-11-01): the tints
        // once passed here belonged to the flat look before it.
        let _ = (left, right);
        panel::draw(f, r, self.light());
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
                b"Touch the centre of the target",
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
        f.push(Cmd::HGradient {
            r: TITLE,
            left: if app { APP_TITLE_LEFT } else { BAR_LEFT },
            right: if app { APP_TITLE_RIGHT } else { BAR_RIGHT },
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
        if reveal.is_some() {
            f.clip(None);
        }
        if let Some(n) = self.notice() {
            self.centred(f, NOTICE, H - 14, ui, FOCUS, n.as_bytes());
        }

        // The Start menu over everything.
        if self.menu_open {
            let r = self.menu_rect();
            self.glass(f, r, MENU_LEFT, MENU_RIGHT);
            for i in 0..self.menu_len() {
                let e = Self::menu_entry(i);
                self.press_under(f, Target::Menu(i));
                f.text(e.x + 16, e.y + 27, ui, TEXT, self.menu_label(i).as_bytes());
                self.decorate(f, Target::Menu(i));
            }
        }
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

        // The calendar: sample lines until there is a calendar.
        self.glass(f, CALENDAR_PANEL, GLASS_LEFT, GLASS_RIGHT);
        f.text(
            CALENDAR_PANEL.x + 16,
            CALENDAR_PANEL.y + 28,
            label,
            DIM,
            b"TODAY",
        );
        f.text(
            CALENDAR_PANEL.x + CALENDAR_PANEL.w - 16 - ui.width(b"sample data"),
            CALENDAR_PANEL.y + 28,
            ui,
            DIM,
            b"sample data",
        );
        for (k, (h, m, what)) in [(11u8, 30u8, &b"Team call"[..]), (16, 0, b"Pick up parts")]
            .iter()
            .enumerate()
        {
            let y = CALENDAR_PANEL.y + 58 + k as u16 * 28;
            let when = rg.time(*h, *m, true);
            f.text(CALENDAR_PANEL.x + 16, y, ui, DIM, when.as_bytes());
            f.text(CALENDAR_PANEL.x + 112, y, ui, TEXT, what);
        }

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
