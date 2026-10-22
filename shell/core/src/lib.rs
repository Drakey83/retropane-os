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
//! Placeholders say so: weather is a fixed reading, and entries without a
//! program yet show "Not yet" in the softkey bar.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod align;

use rp_draw::{Argb, Cmd, Font, Frame, Image, Rect};
pub use rp_touch_cal::Calibration;

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

/// The weather pictures the Today screen draws (made from the icon pack on
/// the PC; none yet means an outline where each goes).
#[derive(Clone, Copy, Default)]
pub struct Icons {
    /// The big one in the hero panel.
    pub hero: Option<&'static Image>,
    /// The five in the forecast row.
    pub forecast: [Option<&'static Image>; 5],
}

impl Icons {
    /// No pictures.
    pub const NONE: Self = Self {
        hero: None,
        forecast: [None; 5],
    };
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
pub const TITLE_CLOCK: Rect = Rect::new(W - 140, 0, 140, 36);
/// The quick buttons (docs/design/ui.md "A PDA, proudly").
const QUICK: [&str; 5] = ["Note", "Record", "Reader", "Media", "Games"];
const QUICK_Y: u16 = 532;
const QUICK_W: u16 = 84;
const QUICK_H: u16 = 54;

fn quick_rect(i: u8) -> Rect {
    Rect::new(16 + u16::from(i) * 91, QUICK_Y, QUICK_W, QUICK_H)
}

/// The degree sign in the small fonts (DEL's slot, tools/shell-assets/fonts.py);
/// the big font has it at the Latin-1 place.
const DEG: u8 = 0x7F;
const DEG_BIG: u8 = 0xB0;

/// Placeholder weather until there is a network: today's reading and a
/// five-day forecast as (high, low), drawn with "sample data" beside them.
const SAMPLE_NOW: u8 = 72;
const SAMPLE_HIGH_LOW: (u8, u8) = (75, 52);
const SAMPLE_FORECAST: [(u8, u8); 5] = [(73, 52), (63, 48), (61, 48), (59, 46), (70, 50)];

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
        }
    }

    /// The targets that can be tapped or focused now, front first.
    fn targets(&self) -> impl Iterator<Item = Target> + '_ {
        let menu = (0..self.menu_len())
            .filter(|_| self.menu_open)
            .map(Target::Menu);
        let today = [Target::Start]
            .into_iter()
            .chain((0..QUICK.len() as u8).map(Target::Quick))
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
            Target::SoftLeft => self.show_notice(not_yet("Calendar")),
            Target::SoftRight => self.show_notice(not_yet("Edit")),
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
                if label != "Today" {
                    self.show_notice(not_yet(label));
                }
            }
        }
        Action::Redraw
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
                let t = self.hit(x, y);
                if t.is_none() && self.menu_open && !self.menu_rect().contains_point(x, y) {
                    // A tap outside the open menu closes it.
                    self.set_menu(false);
                }
                self.set_pressed(t);
                Action::Nothing
            }
            Input::PenUp { x, y } => {
                let t = self.pressed;
                self.set_pressed(None);
                match t {
                    Some(t) if self.rect_of(t).contains_point(x, y) => self.activate(t),
                    _ => Action::Nothing,
                }
            }
            Input::Key(Key::Home) => {
                self.set_menu(false);
                self.set_focus(None);
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
        self.targets()
            .filter(|&t| t != from)
            .filter_map(|t| {
                let (x, y) = c(self.rect_of(t));
                let (dx, dy) = (x - fx, y - fy);
                let (along, across) = match k {
                    Key::Up => (-dy, dx),
                    Key::Down => (dy, dx),
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
        let (ui, label, big) = (self.fonts.ui, self.fonts.label, self.fonts.big);
        let mut buf = [0u8; 32];

        // Title bar: Start, the clock and status at the right.
        f.push(Cmd::HGradient {
            r: TITLE,
            left: BAR_LEFT,
            right: BAR_RIGHT,
        });
        f.text(14, 25, ui, TEXT, b"Start");
        self.decorate(f, Target::Start);
        let hm = hh_mm(&mut buf, self.time);
        let w = ui.width(hm);
        f.text(W - 64 - w, 25, ui, TEXT, hm);
        // Battery: not read yet, drawn as an outline.
        f.push(Cmd::Outline {
            r: Rect::new(W - 46, 12, 30, 14),
            color: DIM,
        });
        f.push(Cmd::Fill {
            r: Rect::new(W - 16, 16, 3, 6),
            color: DIM,
        });

        // The weather hero.
        Self::glass(f, HERO, GLASS_LEFT, GLASS_RIGHT);
        f.text(
            HERO.x + 20,
            HERO.y + 34,
            label,
            DIM,
            b"YOUR CITY - SAMPLE DATA",
        );
        let temp = [b'0' + SAMPLE_NOW / 10, b'0' + SAMPLE_NOW % 10, DEG_BIG];
        f.text(HERO.x + 20, HERO.y + 116, big, TEXT, &temp);
        f.text(HERO.x + 20, HERO.y + 146, ui, TEXT, b"Partly cloudy");
        let (hi, lo) = SAMPLE_HIGH_LOW;
        let n = high_low(&mut buf, hi, lo);
        f.text(HERO.x + 20, HERO.y + 174, ui, DIM, &buf[..n]);
        // The icon, or its outline until the pictures exist
        // (docs/design/today-screen-plan.md).
        match self.icons.hero {
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

        // The five-day forecast: the card, its icon over the card's top edge,
        // the day name and the high and low.
        for (i, (hi, lo)) in SAMPLE_FORECAST.iter().enumerate() {
            let r = Rect::new(
                FORECAST_ROW.x + i as u16 * FORECAST_STEP,
                FORECAST_TOP,
                FORECAST_W,
                FORECAST_H,
            );
            Self::glass(f, r, GLASS_LEFT, GLASS_RIGHT);
            // The icon's 66 px box reaches 30 px above the card (as in the mock-ups).
            match self.icons.forecast[i] {
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
            let day = day_name(self.time.weekday, i as u8 + 1);
            self.centred(f, r, r.y + 44, label, DIM, day);
            let n = high_low_short(&mut buf, *hi, *lo);
            self.centred(f, r, r.y + r.h - 8, ui, TEXT, &buf[..n]);
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
        for (k, (when, what)) in [
            (&b"11:30"[..], &b"Team call"[..]),
            (b"16:00", b"Pick up parts"),
        ]
        .iter()
        .enumerate()
        {
            let y = CALENDAR_PANEL.y + 58 + k as u16 * 28;
            f.text(CALENDAR_PANEL.x + 16, y, ui, DIM, when);
            f.text(CALENDAR_PANEL.x + 80, y, ui, TEXT, what);
        }

        // Quick buttons.
        for (i, name) in QUICK.iter().enumerate() {
            let t = Target::Quick(i as u8);
            let r = self.rect_of(t);
            Self::glass(f, r, GLASS_LEFT, GLASS_RIGHT);
            self.centred(f, r, r.y + r.h - 14, ui, TEXT, name.as_bytes());
            self.decorate(f, t);
        }

        // Softkeys and the notice between them.
        f.push(Cmd::HGradient {
            r: SOFTKEYS,
            left: BAR_LEFT,
            right: BAR_RIGHT,
        });
        self.centred(f, SOFT_LEFT, H - 14, ui, TEXT, b"Calendar");
        self.centred(f, SOFT_RIGHT, H - 14, ui, TEXT, b"Edit");
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
}

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
        _ => "Not yet",
    }
}

/// `HH:MM` into `buf`.
fn hh_mm(buf: &mut [u8; 32], t: Time) -> &[u8] {
    let (h, m) = (t.hour.min(23), t.minute.min(59));
    buf[..5].copy_from_slice(&[
        b'0' + h / 10,
        b'0' + h % 10,
        b':',
        b'0' + m / 10,
        b'0' + m % 10,
    ]);
    &buf[..5]
}

/// "High 75° Low 52°" into `buf` (the degree sign is the small fonts' own).
fn high_low(buf: &mut [u8; 32], hi: u8, lo: u8) -> usize {
    let mut n = 0;
    for part in [
        &b"High "[..],
        &two(hi),
        &[DEG],
        b"   Low ",
        &two(lo),
        &[DEG],
    ] {
        buf[n..n + part.len()].copy_from_slice(part);
        n += part.len();
    }
    n
}

/// "73° 52°" into `buf` (a forecast card).
fn high_low_short(buf: &mut [u8; 32], hi: u8, lo: u8) -> usize {
    let mut n = 0;
    for part in [&two(hi)[..], &[DEG, b' '], &two(lo), &[DEG]] {
        buf[n..n + part.len()].copy_from_slice(part);
        n += part.len();
    }
    n
}

/// Two digits of `n` (0..=99).
fn two(n: u8) -> [u8; 2] {
    let n = n.min(99);
    [b'0' + n / 10, b'0' + n % 10]
}

/// The name of the weekday `ahead` days after `weekday` (0 Monday).
fn day_name(weekday: u8, ahead: u8) -> &'static [u8] {
    const DAYS: [&[u8]; 7] = [b"MON", b"TUE", b"WED", b"THU", b"FRI", b"SAT", b"SUN"];
    DAYS[usize::from((weekday.min(6) + ahead) % 7)]
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
