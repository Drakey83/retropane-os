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

use rp_draw::{Argb, Cmd, Font, Frame, Rect};
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
/// The clock and date panel (its contents depend on the time).
pub const CLOCK_PANEL: Rect = Rect::new(16, 56, 448, 150);
const WEATHER_PANEL: Rect = Rect::new(16, 222, 448, 110);
const CALENDAR_PANEL: Rect = Rect::new(16, 348, 448, 52);
const SOFTKEYS: Rect = Rect::new(0, H - 40, W, 40);
const SOFT_LEFT: Rect = Rect::new(0, H - 40, 160, 40);
const SOFT_RIGHT: Rect = Rect::new(W - 160, H - 40, 160, 40);
/// Where the "Not yet" notice and the title clock sit.
const NOTICE: Rect = Rect::new(160, H - 40, 160, 40);
/// The title bar's clock and status (depends on the time).
pub const TITLE_CLOCK: Rect = Rect::new(W - 140, 0, 140, 36);
/// The big clock's baseline from the top of its panel.
const BIG_CLOCK_BASELINE: u16 = 92;

/// The quick buttons (docs/design/ui.md "A PDA, proudly").
const QUICK: [&str; 5] = ["Note", "Record", "Reader", "Media", "Games"];
const QUICK_Y: u16 = 416;
const QUICK_SIZE: u16 = 82;

fn quick_rect(i: u8) -> Rect {
    Rect::new(16 + u16::from(i) * 90, QUICK_Y, QUICK_SIZE, QUICK_SIZE)
}

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
/// The test image's extra Start menu entries, after the OS's.
const DEV_MENU: [&str; 2] = [ALIGN_SCREEN, BACK_TO_LOADER];
const MENU_X: u16 = 0;
const MENU_Y: u16 = 36;
const MENU_W: u16 = 280;
const MENU_ROW: u16 = 40;

// ---- Colours (the VClouds smoked glass, docs/design/ui.md) -------------

const GLASS_LEFT: Argb = 0xB810_1418;
const GLASS_RIGHT: Argb = 0x8C4C_545C;
const GLASS_EDGE: Argb = 0x50FF_FFFF;
const BAR_LEFT: Argb = 0xD800_0000;
const BAR_RIGHT: Argb = 0xB020_2428;
const MENU_LEFT: Argb = 0xE008_0A0C;
const MENU_RIGHT: Argb = 0xD02C_3238;
const TEXT: Argb = 0xFFF0_F2F4;
const DIM: Argb = 0xFFA8_B0B8;
const FOCUS: Argb = 0xFF7F_D0FF;
const PRESSED: Argb = 0x40FF_FFFF;
/// The align screen's background (opaque: the wallpaper would distract).
const ALIGN_BACK: Argb = 0xFF0C_1014;
/// A target held long enough.
const HELD: Argb = 0xFF5C_E07A;

/// How long a notice stays (milliseconds).
const NOTICE_MS: u32 = 2000;

/// Dirty rectangles the shell gathers between frames.
const MAX_DAMAGE: usize = 8;

/// The shell's state.
pub struct Shell {
    fonts: Fonts,
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
}

impl Shell {
    /// The Today screen, all of it to be drawn.
    #[must_use]
    pub fn new(fonts: Fonts, dev_exit: bool) -> Self {
        let mut s = Self {
            fonts,
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
        // The big clock's box before and after (its width can change).
        let old = self.big_clock_rect(self.time);
        self.time = t;
        self.dirty(TITLE_CLOCK);
        if date_changed {
            self.dirty(CLOCK_PANEL);
        } else {
            self.dirty(old.union(&self.big_clock_rect(t)));
        }
        Action::Redraw
    }

    /// The box the big clock's text takes for `t` (with the text margin).
    fn big_clock_rect(&self, t: Time) -> Rect {
        let mut buf = [0u8; 32];
        let font = self.fonts.big;
        let w = font.width(hh_mm(&mut buf, t));
        let x = CLOCK_PANEL.x + CLOCK_PANEL.w.saturating_sub(w) / 2;
        let top = (CLOCK_PANEL.y + BIG_CLOCK_BASELINE).saturating_sub(u16::from(font.ascent) + 4);
        Rect::new(x.saturating_sub(4), top, w + 8, u16::from(font.line) + 8).intersect(&CLOCK_PANEL)
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
                // In that direction, within 45 degrees; nearest, straightest.
                (along > 0 && across.abs() <= along).then_some((along + 2 * across.abs(), t))
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

        // The clock and date.
        Self::glass(f, CLOCK_PANEL, GLASS_LEFT, GLASS_RIGHT);
        let hm = hh_mm(&mut buf, self.time);
        self.centred(
            f,
            CLOCK_PANEL,
            CLOCK_PANEL.y + BIG_CLOCK_BASELINE,
            big,
            TEXT,
            hm,
        );
        let date = date_line(&mut buf, self.time);
        self.centred(f, CLOCK_PANEL, CLOCK_PANEL.y + 130, label, DIM, date);

        // The weather: a placeholder until there's a network.
        Self::glass(f, WEATHER_PANEL, GLASS_LEFT, GLASS_RIGHT);
        f.text(
            WEATHER_PANEL.x + 24,
            WEATHER_PANEL.y + 74,
            big,
            TEXT,
            b"18\xb0",
        );
        f.text(
            WEATHER_PANEL.x + 200,
            WEATHER_PANEL.y + 48,
            label,
            TEXT,
            b"PARTLY CLOUDY",
        );
        f.text(
            WEATHER_PANEL.x + 200,
            WEATHER_PANEL.y + 78,
            ui,
            DIM,
            b"(sample: no network yet)",
        );

        // The calendar line.
        Self::glass(f, CALENDAR_PANEL, GLASS_LEFT, GLASS_RIGHT);
        f.text(
            CALENDAR_PANEL.x + 16,
            CALENDAR_PANEL.y + 33,
            ui,
            TEXT,
            b"No appointments today",
        );

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
