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
pub mod motion;
mod weather_app;
mod weather_settings;

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

/// The weather pictures (made from the icon pack on the PC; a missing one
/// is an outline where it goes). Indexed by pack id less one
/// ([`rp_weather::Icon::id`]).
#[derive(Clone, Copy)]
pub struct Icons {
    /// The big one in the hero panel and the app's Now page: the reading
    /// now's (partly cloudy, day, in the sample).
    pub hero: Option<&'static Image>,
    /// The forecast cards' size (66 px).
    pub card: [Option<&'static Image>; 22],
    /// The Weather app's rows (40 px).
    pub row: [Option<&'static Image>; 22],
    /// The radar's rain cells ([`rp_weather::radar::CELLS`]), made at start.
    pub radar: [Option<&'static Image>; 3],
}

impl Icons {
    /// No pictures.
    pub const NONE: Self = Self {
        hero: None,
        card: [None; 22],
        row: [None; 22],
        radar: [None; 3],
    };
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
    /// The Weather app's places.
    WeatherPlaces,
    /// The station key, typed on the on-screen keyboard.
    KeyEntry,
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
}

// ---- Layout (480 x 640, portrait) --------------------------------------

const TITLE: Rect = Rect::new(0, 0, W, 36);
/// Start, top left.
pub const START: Rect = Rect::new(0, 0, 96, 36);
/// The weather hero panel: the place, the temperature, the condition, today's
/// high and low, and the date (docs/design/ui.md "Weather and icons").
const HERO: Rect = Rect::new(16, 52, 448, 196);
/// The box of the weather icon, as in the mock-ups: 8 px past the panel's right
/// edge and 48 px above its top, over the title bar; the picture is centred
/// vertically in it and sits against its right side.
const HERO_ICON: Rect = Rect::new(16 + 448 + 8 - 250, 52 - 48, 250, 176);
/// The date at the hero's bottom right (its text depends on the day).
pub const DATE_BOX: Rect = Rect::new(280, 52 + 196 - 50, 168, 40);
/// The five-day forecast row with the room above it for the icons that
/// reach over the cards' top edge (its day names depend on the day).
pub const FORECAST_ROW: Rect = Rect::new(16, 252, 448, 88);
/// The cards' top: the row's top plus the icons' headroom.
const FORECAST_TOP: u16 = 266;
const FORECAST_H: u16 = 74;
const FORECAST_W: u16 = 83;
const FORECAST_STEP: u16 = 91;
const CALENDAR_PANEL: Rect = Rect::new(16, 352, 448, 104);
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
const NOTICE_BACK: Argb = rp_draw::look::theme(0xB000_0000);
/// The test image's extra Start menu entries, after the OS's.
const DEV_MENU: [&str; 3] = [WALLPAPER_COMPARE, ALIGN_SCREEN, BACK_TO_LOADER];
const MENU_X: u16 = 0;
const MENU_Y: u16 = 36;
const MENU_W: u16 = 280;
const MENU_ROW: u16 = 40;

// ---- Colours (the smoked glass, docs/design/ui.md) -------------

const GLASS_LEFT: Argb = rp_draw::look::theme(0xB810_1418);
const GLASS_RIGHT: Argb = rp_draw::look::theme(0x8C4C_545C);
const GLASS_EDGE: Argb = rp_draw::look::theme(0x50FF_FFFF);
const BAR_LEFT: Argb = rp_draw::look::theme(0xD800_0000);
const BAR_RIGHT: Argb = rp_draw::look::theme(0xB020_2428);
const MENU_LEFT: Argb = rp_draw::look::theme(0xE008_0A0C);
const MENU_RIGHT: Argb = rp_draw::look::theme(0xD02C_3238);
const TEXT: Argb = rp_draw::look::theme(0xFFF0_F2F4);
const DIM: Argb = rp_draw::look::theme(0xFFA8_B0B8);
const FOCUS: Argb = rp_draw::look::theme(0xFF7F_D0FF);
const PRESSED: Argb = rp_draw::look::theme(0x40FF_FFFF);
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
}

/// How far the pen may move before a press stops being a tap (px): a
/// first draft for R124's measurement (the panel's jitter at rest is 2-3
/// px after the filter).
const SLOP: i32 = 8;
/// A swipe this far across turns the page.
const SWIPE_TURN: i32 = 60;

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
    /// The Weather app's settings (RAM until the settings store exists).
    wset: rp_weather::settings::Settings,
    /// The key being typed (zeroed when done or left).
    typed: [u8; rp_weather::settings::KEY_MAX],
    typed_n: usize,
    /// The keyboard's shift, and whether the key is shown as typed.
    shift: bool,
    show_key: bool,
    /// The radar loop: the frame shown, whether it plays, when it last
    /// moved on (ms).
    radar_frame: u8,
    radar_play: bool,
    radar_at: u32,
}

impl Shell {
    /// The Today screen, all of it to be drawn.
    #[must_use]
    pub fn new(fonts: Fonts, icons: Icons, dev_exit: bool) -> Self {
        let mut s = Self {
            fonts,
            icons,
            dev_exit,
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
            wset: rp_weather::settings::Settings::DEFAULT,
            typed: [0; rp_weather::settings::KEY_MAX],
            typed_n: 0,
            shift: false,
            show_key: false,
            radar_frame: 0,
            radar_play: true,
            radar_at: 0,
        };
        s.dirty(Rect::new(0, 0, W, H));
        s
    }

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
            Target::Timeline => weather_app::TIMELINE,
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
        let caps = if self.screen == Screen::KeyEntry {
            weather_settings::CAPS
        } else {
            0
        };
        let today = [Target::Start]
            .into_iter()
            .chain((0..back).map(|_| Target::Back))
            .chain((0..hero).map(|_| Target::Hero))
            .chain((0..quick).map(Target::Quick))
            .chain((0..tabs).map(Target::Tab))
            .chain((0..self.rows()).map(Target::Row))
            .chain((0..caps).map(Target::Cap))
            .chain((0..radar).flat_map(|_| [Target::Play, Target::Timeline]))
            .chain([Target::SoftLeft, Target::SoftRight])
            // With the menu open only Start (to close it) and the menu act.
            .filter(|t| !self.menu_open || *t == Target::Start);
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
        if self.radar_step(now_ms) && self.anim.is_none() && self.notice.is_none() {
            return Action::Redraw;
        }
        if self.screen == Screen::Weather && self.scroll.step(now_ms) {
            self.dirty(weather_app::PAGE);
            if self.anim.is_none() && self.notice.is_none() {
                return Action::Redraw;
            }
        }
        if let Some(start) = self.anim {
            // A page arriving: redraw it every pass until it's in place.
            if now_ms.wrapping_sub(start) >= weather_app::ANIM_MS {
                self.anim = None;
            }
            self.dirty(weather_app::PAGE);
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
        self.time = t;
        self.dirty(TITLE_CLOCK);
        if self.screen == Screen::Weather {
            // The header's "read at" time; the Now page's sunset bar.
            self.dirty(weather_app::HEAD);
            if self.tab == 0 {
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
        if let Some(old) = self.pressed {
            self.dirty(self.rect_of(old));
        }
        if let Some(new) = p {
            self.dirty(self.rect_of(new));
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
        match t {
            Target::Start => self.set_menu(!self.menu_open),
            Target::Quick(i) => {
                self.show_notice(not_yet(QUICK[usize::from(i).min(QUICK.len() - 1)]))
            }
            Target::SoftLeft => match self.screen {
                Screen::Today => self.show_notice(not_yet(self.softkeys().0)),
                // Back (Cancel on the keyboard), I7: the left softkey and
                // the header's control do the same.
                _ => self.go_back(),
            },
            Target::SoftRight => match self.screen {
                Screen::Today => self.show_notice(not_yet(self.softkeys().1)),
                Screen::Weather => self.open(Screen::WeatherSettings),
                Screen::KeyEntry => self.key_done(),
                Screen::WeatherSettings | Screen::WeatherPlaces => {}
            },
            Target::Hero => self.open_weather(),
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
            self.anim = None;
            self.pressed = None;
            if self.focus.is_some() {
                self.focus = Some(Target::Hero);
            }
            self.dirty(Rect::new(0, 0, W, H));
        }
    }

    /// The softkeys' names on the screen up.
    fn softkeys(&self) -> (&'static str, &'static str) {
        match self.screen {
            Screen::Today => ("Calendar", "Edit"),
            Screen::Weather => ("Back", "Settings"),
            Screen::WeatherSettings | Screen::WeatherPlaces => ("Back", ""),
            Screen::KeyEntry => ("Cancel", "Done"),
        }
    }

    /// Whether a stroke from `(x, y)` is the Weather page's.
    fn on_page(&self, x: u16, y: u16) -> bool {
        self.screen == Screen::Weather && !self.menu_open && weather_app::PAGE.contains_point(x, y)
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
            self.dirty(weather_app::PAGE);
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
            Gesture::None | Gesture::Pending { .. } => {
                // Still a tap: the scroll lets go where it is.
                self.scroll.release(self.now);
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
                    self.radar_frame = weather_app::frame_at(x);
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
        f.clear();
        for d in &self.damage[..self.damaged] {
            f.damage(*d);
        }
        self.damaged = 0;
        self.scene(f);
    }

    fn glass(f: &mut Frame, r: Rect, left: Argb, right: Argb) {
        f.push(Cmd::HGradient { r, left, right });
        f.push(Cmd::Outline {
            r,
            color: GLASS_EDGE,
        });
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

    fn decorate(&self, f: &mut Frame, t: Target) {
        let r = self.rect_of(t);
        if self.pressed == Some(t) {
            f.push(Cmd::Fill { r, color: PRESSED });
        }
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

        // Title bar: Start, the clock and status at the right.
        f.push(Cmd::HGradient {
            r: TITLE,
            left: BAR_LEFT,
            right: BAR_RIGHT,
        });
        f.text(14, 25, ui, TEXT, b"Start");
        self.decorate(f, Target::Start);
        let hm = self.region.time(self.time.hour, self.time.minute, true);
        let w = ui.width(hm.as_bytes());
        f.text(W - 64 - w, 25, ui, TEXT, hm.as_bytes());
        // Battery: not read yet, drawn as an outline.
        f.push(Cmd::Outline {
            r: Rect::new(W - 46, 12, 30, 14),
            color: DIM,
        });
        f.push(Cmd::Fill {
            r: Rect::new(W - 16, 16, 3, 6),
            color: DIM,
        });

        match self.screen {
            Screen::Today => self.today_scene(f),
            Screen::Weather => self.weather_scene(f),
            Screen::WeatherSettings => self.settings_scene(f),
            Screen::WeatherPlaces => self.places_scene(f),
            Screen::KeyEntry => self.key_scene(f),
        }

        // Softkeys and the notice between them.
        f.push(Cmd::HGradient {
            r: SOFTKEYS,
            left: BAR_LEFT,
            right: BAR_RIGHT,
        });
        let (l, r) = self.softkeys();
        self.centred(f, SOFT_LEFT, H - 14, ui, TEXT, l.as_bytes());
        self.centred(f, SOFT_RIGHT, H - 14, ui, TEXT, r.as_bytes());
        self.decorate(f, Target::SoftLeft);
        self.decorate(f, Target::SoftRight);
        if let Some(n) = self.notice() {
            self.centred(f, NOTICE, H - 14, ui, FOCUS, n.as_bytes());
        }

        // The Start menu over everything.
        if self.menu_open {
            let r = self.menu_rect();
            Self::glass(f, r, MENU_LEFT, MENU_RIGHT);
            for i in 0..self.menu_len() {
                let e = Self::menu_entry(i);
                f.text(e.x + 16, e.y + 27, ui, TEXT, self.menu_label(i).as_bytes());
                self.decorate(f, Target::Menu(i));
            }
        }
    }

    /// The Today screen under the title bar: the weather hero and the next
    /// five days (from the shell's forecast), the calendar, the quick
    /// buttons.
    fn today_scene(&self, f: &mut Frame) {
        let (ui, label, big) = (self.fonts.ui, self.fonts.label, self.fonts.big);
        let (wx, rg) = (&self.wx, &self.region);
        let mut buf = [0u8; 32];

        // The weather hero (a tap opens the Weather app).
        Self::glass(f, HERO, GLASS_LEFT, GLASS_RIGHT);
        let head = upper(wx.place)
            .push(b" - ")
            .push(upper(wx.source).as_bytes());
        f.text(HERO.x + 20, HERO.y + 34, label, DIM, head.as_bytes());
        let temp = Line::new().num(rg.degrees(wx.now.temp)).push(&[DEG_BIG]);
        f.text(HERO.x + 20, HERO.y + 116, big, TEXT, temp.as_bytes());
        let icon = Icon::of(wx.now.sky);
        f.text(HERO.x + 20, HERO.y + 146, ui, TEXT, icon.words().as_bytes());
        let d = &wx.days[0];
        let hl = Line::new()
            .push(b"High ")
            .push(rg.temp(d.high).as_bytes())
            .push(b"   Low ")
            .push(rg.temp(d.low).as_bytes());
        f.text(HERO.x + 20, HERO.y + 174, ui, DIM, hl.as_bytes());
        // The icon, or its outline until the pictures exist
        // (docs/design/today-screen-plan.md).
        let pic = match (icon, self.icons.hero) {
            (Icon::PartlyDay, Some(h)) => Some(h),
            _ => self.icons.card[usize::from(icon.id() - 1)],
        };
        match pic {
            Some(img) => f.push(Cmd::Image {
                x: HERO_ICON.x + HERO_ICON.w.saturating_sub(img.w),
                y: HERO_ICON.y + HERO_ICON.h.saturating_sub(img.h) / 2,
                img,
            }),
            None => f.push(Cmd::Outline {
                r: HERO_ICON,
                color: 0x30FF_FFFF,
            }),
        }
        let date = date_line(&mut buf, self.time);
        let w = label.width(date);
        f.text(
            DATE_BOX.x + DATE_BOX.w - w,
            HERO.y + HERO.h - 20,
            label,
            TEXT,
            date,
        );
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
            Self::glass(f, r, GLASS_LEFT, GLASS_RIGHT);
            // The icon's 66 px box reaches 30 px above the card (as in the mock-ups).
            match self.icons.card[usize::from(Icon::of(d.sky).id() - 1)] {
                Some(img) => f.push(Cmd::Image {
                    x: r.x + r.w.saturating_sub(img.w) / 2,
                    y: (r.y + 36).saturating_sub(66) + 66u16.saturating_sub(img.h) / 2,
                    img,
                }),
                None => f.push(Cmd::Outline {
                    r: Rect::new(r.x + r.w / 2 - 22, r.y + 6, 44, 24),
                    color: 0x30FF_FFFF,
                }),
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
        Self::glass(f, CALENDAR_PANEL, GLASS_LEFT, GLASS_RIGHT);
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
            Self::glass(f, r, GLASS_LEFT, GLASS_RIGHT);
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
pub const NOTICES: [&str; 21] = [
    "64 at most",
    "Base pack only",
    "Key forgotten",
    "Key kept",
    "No spaces",
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

/// `MON 13 OCTOBER` into `buf` (spaced capitals are the label font's job).
fn date_line(buf: &mut [u8; 32], t: Time) -> &[u8] {
    const DAYS: [&[u8]; 7] = [b"MON", b"TUE", b"WED", b"THU", b"FRI", b"SAT", b"SUN"];
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
