//! The Weather app's screens below its main one (build-ui-foundation.md
//! steps 3-5): Settings, Places, and the station key typed on an on-screen
//! keyboard (the owner, 2026-10-30: "for now I will need to do it manually
//! with a keyboard"). Each has the header's back control and Back on the
//! left softkey; Home goes home from anywhere.
//!
//! Iterate, don't imitate (rule 0): WM5's settings were dialogs of
//! drop-down lists and an OK box, and its soft keyboard (SIP) a small
//! panel of 15 px keys meant for a stylus. Ours: one row per setting, a tap
//! moves to the next choice or opens its list (no dialog, nothing to
//! confirm), and a key-entry screen whose keys are 46 x 50 px (fingertip
//! size, I3's provisional 64 px goal for the main targets) with the key
//! masked as it is typed, its last four characters shown so the owner can
//! check it.

use rp_draw::{Argb, Cmd, Frame, Rect};
use rp_weather::Line;
use rp_weather::settings::{KEY_MAX, PLACES, Secret};

use super::{DIM, FOCUS, GLASS_LEFT, GLASS_RIGHT, H, Screen, Shell, TEXT, Target, W};

/// The header's back control (every Weather screen).
pub(crate) const BACK: Rect = Rect::new(0, 36, 64, 58);
/// The settings' and places' rows.
const ROW_Y: u16 = 104;
const ROW_STEP: u16 = 58;
const ROW_H: u16 = 52;
/// The settings, in order.
pub(crate) const SETTINGS: usize = 8;

/// The key-entry screen: the field, and the keyboard's rows.
const FIELD: Rect = Rect::new(28, 144, 424, 60);
/// The panel around the field: what the key is for, the field, its count
/// and where it is kept (all of it redrawn as a key is typed).
const KEY_PANEL: Rect = Rect::new(16, 102, 448, 214);
const KEYS_Y: u16 = 330;
const KEY_W: u16 = 48;
const KEY_H: u16 = 52;
const KEY_STEP_Y: u16 = 54;
/// The keys, row by row; `\x08` backspace, `\x0E` shift, `\x0F` show or
/// hide.
const KEYROWS: [&[u8]; 4] = [
    b"1234567890",
    b"qwertyuiop",
    b"asdfghjkl-",
    b"\x0Ezxcvbnm_\x08",
];
/// The fifth row: show or hide, then the space between Cancel and Done is
/// the softkeys'.
const SHOW: u8 = 0x0F;
const SHIFT: u8 = 0x0E;
const BKSP: u8 = 0x08;
/// Keys in all.
pub(crate) const CAPS: u8 = 41;

const PRESSED_KEY: Argb = rp_draw::look::theme(0xE04A_5460);

/// A settings or places row.
const fn row(i: u8) -> Rect {
    Rect::new(16, ROW_Y + i as u16 * ROW_STEP, 448, ROW_H)
}

/// Which key cap `i` is: its byte and rectangle.
fn cap(i: u8) -> Option<(u8, Rect)> {
    let mut k = i;
    for (r, keys) in KEYROWS.iter().enumerate() {
        if usize::from(k) < keys.len() {
            let x = u16::from(k) * KEY_W;
            let y = KEYS_Y + r as u16 * KEY_STEP_Y;
            return Some((keys[usize::from(k)], Rect::new(x, y, KEY_W - 2, KEY_H)));
        }
        k -= keys.len() as u8;
    }
    (k == 0).then(|| {
        (
            SHOW,
            Rect::new(W / 2 - 80, KEYS_Y + 4 * KEY_STEP_Y, 160, KEY_H - 6),
        )
    })
}

/// The key cap that types `c` (lower case; upper case is shift then it):
/// the tests type through it.
#[cfg(test)]
pub(crate) fn cap_of(c: u8) -> Option<u8> {
    (0..CAPS).find(|&i| cap(i).is_some_and(|(b, _)| b == c.to_ascii_lowercase()))
}

impl Shell {
    /// Where a target of these screens is.
    pub(crate) fn settings_rect(&self, t: Target) -> Rect {
        match t {
            Target::Row(i) => row(i),
            Target::Cap(i) => cap(i).map_or(Rect::new(0, 0, 0, 0), |(_, r)| r),
            _ => BACK,
        }
    }

    /// The rows the screen up has.
    pub(crate) fn rows(&self) -> u8 {
        match self.screen {
            Screen::WeatherSettings => SETTINGS as u8,
            Screen::WeatherPlaces => PLACES.len() as u8,
            _ => 0,
        }
    }

    /// Opens a screen below the Weather app.
    pub(crate) fn open(&mut self, s: Screen) {
        self.screen = s;
        self.pressed = None;
        self.anim = None;
        self.gesture = super::Gesture::None;
        if s == Screen::KeyEntry {
            self.typed = [0; KEY_MAX];
            self.typed_n = 0;
            self.shift = false;
            self.show_key = false;
        }
        if self.focus.is_some() {
            self.focus = Some(match s {
                Screen::KeyEntry => Target::Cap(0),
                _ => Target::Row(0),
            });
        }
        self.dirty(Rect::new(0, 0, W, H));
    }

    /// One screen back: a screen below the app to the one above it, the
    /// app to the Today screen.
    pub(crate) fn go_back(&mut self) {
        if self.screen == Screen::KeyEntry {
            self.forget_typed();
        }
        match self.screen {
            Screen::WeatherPlaces | Screen::KeyEntry => self.open(Screen::WeatherSettings),
            Screen::WeatherSettings => {
                self.open_weather();
                self.settle();
            }
            Screen::Weather => self.go_today(),
            Screen::Today => {}
        }
    }

    /// The typed key zeroed.
    fn forget_typed(&mut self) {
        self.typed = [0; KEY_MAX];
        self.typed_n = 0;
    }

    /// A settings or places row tapped.
    pub(crate) fn row_tapped(&mut self, i: u8) {
        if self.screen == Screen::WeatherPlaces {
            self.wset.place = i % PLACES.len() as u8;
            self.apply_settings();
            self.open(Screen::WeatherSettings);
            return;
        }
        match i {
            0 => self.wset.next_region(),
            1 => self.wset.metric = !self.wset.metric,
            2 => return self.open(Screen::WeatherPlaces),
            3 => self.wset.source = self.wset.source.next(),
            4 => return self.open(Screen::KeyEntry),
            5 => {
                self.wset.refresh =
                    (self.wset.refresh + 1) % rp_weather::settings::REFRESH.len() as u8;
            }
            6 => {
                // The base set only, until packs come from the card.
                self.show_notice("Base pack only");
            }
            _ => self.wset.radar = self.wset.radar.next(),
        }
        self.apply_settings();
        self.dirty(row(i));
    }

    /// The settings into what every screen reads.
    pub(crate) fn apply_settings(&mut self) {
        let units = self.wset.units();
        if units != self.region {
            self.region = units;
            // Every number and the clock change.
            self.dirty(Rect::new(0, 0, W, H));
        }
        self.wx.place = self.wset.place_name();
        self.wx.source = if self.wset.source == rp_weather::settings::Source::Sample {
            "Sample data"
        } else {
            // No network yet: the choice is kept, the sample shown.
            "Sample (no network yet)"
        };
    }

    /// A key on the keyboard.
    pub(crate) fn cap_tapped(&mut self, i: u8) {
        let Some((b, r)) = cap(i) else { return };
        match b {
            BKSP => self.typed_n = self.typed_n.saturating_sub(1),
            SHIFT => {
                self.shift = !self.shift;
                self.dirty(Rect::new(0, KEYS_Y, W, 4 * KEY_STEP_Y));
            }
            SHOW => self.show_key = !self.show_key,
            _ if self.typed_n < KEY_MAX => {
                let c = if self.shift {
                    b.to_ascii_uppercase()
                } else {
                    b
                };
                self.typed[self.typed_n] = c;
                self.typed_n += 1;
                if self.shift {
                    self.shift = false;
                    self.dirty(Rect::new(0, KEYS_Y, W, 4 * KEY_STEP_Y));
                }
            }
            _ => self.show_notice("64 at most"),
        }
        self.typed[self.typed_n..].fill(0);
        self.dirty(KEY_PANEL);
        self.dirty(r);
    }

    /// Done on the key-entry screen: the key kept (in RAM until the key
    /// store exists, R125), the typed copy zeroed.
    pub(crate) fn key_done(&mut self) {
        match Secret::new(&self.typed[..self.typed_n]) {
            Some(k) => {
                self.wset.tempest_key.clear();
                self.wset.tempest_key = k;
                self.forget_typed();
                self.open(Screen::WeatherSettings);
                self.show_notice("Key kept");
            }
            None if self.typed_n == 0 => {
                self.wset.tempest_key.clear();
                self.open(Screen::WeatherSettings);
                self.show_notice("Key forgotten");
            }
            None => self.show_notice("No spaces"),
        }
    }

    /// The header of the screens below the app: the back control and the
    /// screen's name.
    pub(crate) fn sub_header(&self, f: &mut Frame, title: &[u8]) {
        let r = super::weather_app::HEAD;
        f.push(Cmd::HGradient {
            r,
            left: GLASS_LEFT,
            right: GLASS_RIGHT,
        });
        f.push(Cmd::Fill {
            r: Rect::new(0, r.y + r.h - 1, W, 1),
            color: super::GLASS_EDGE,
        });
        self.back_arrow(f);
        f.text(72, r.y + 37, self.fonts.label, TEXT, title);
    }

    /// The back control: a chevron, its press state and focus.
    pub(crate) fn back_arrow(&self, f: &mut Frame) {
        // A chevron of steps (no glyph needed): 12 rows to the point and
        // back, 3 px thick.
        let (cx, cy) = (26u16, BACK.y + BACK.h / 2);
        for k in 0..12u16 {
            for y in [cy - k, cy + k] {
                f.push(Cmd::Fill {
                    r: Rect::new(cx + k, y, 3, 1),
                    color: TEXT,
                });
            }
        }
        self.decorate(f, Target::Back);
    }

    /// The settings screen.
    pub(crate) fn settings_scene(&self, f: &mut Frame) {
        self.sub_header(f, b"WEATHER SETTINGS");
        let s = &self.wset;
        let refresh = s.refresh_name();
        let source = s.source.name();
        let key = s.tempest_key.masked();
        let units: &str = if s.metric { "Metric" } else { "US customary" };
        let rows: [(&[u8], &[u8]); SETTINGS] = [
            (b"REGION", s.region_info().name.as_bytes()),
            (b"UNITS", units.as_bytes()),
            (b"PLACE", s.place_name().as_bytes()),
            (b"DATA SOURCE", source.as_bytes()),
            (b"TEMPEST KEY", key.as_bytes()),
            (b"REFRESH", refresh.as_bytes()),
            (b"ICON PACK", b"VClouds"),
            (b"RADAR MAP", s.radar.name().as_bytes()),
        ];
        for (i, (name, value)) in rows.iter().enumerate() {
            self.setting_row(f, i as u8, name, value, true);
        }
    }

    fn setting_row(&self, f: &mut Frame, i: u8, name: &[u8], value: &[u8], chevron: bool) {
        let (ui, label) = (self.fonts.ui, self.fonts.label);
        let r = row(i);
        Self::glass(f, r, GLASS_LEFT, GLASS_RIGHT);
        f.text(r.x + 14, r.y + 33, label, DIM, name);
        let w = ui.width(value);
        let right = r.x + r.w - if chevron { 34 } else { 14 };
        f.text(right.saturating_sub(w), r.y + 34, ui, TEXT, value);
        if chevron {
            f.text(r.x + r.w - 24, r.y + 34, ui, DIM, b">");
        }
        self.decorate(f, Target::Row(i));
    }

    /// The places screen: the current one ticked.
    pub(crate) fn places_scene(&self, f: &mut Frame) {
        self.sub_header(f, b"PLACES");
        for (i, p) in PLACES.iter().enumerate() {
            let on = usize::from(self.wset.place) == i;
            let r = row(i as u8);
            Self::glass(f, r, GLASS_LEFT, GLASS_RIGHT);
            f.text(r.x + 14, r.y + 34, self.fonts.ui, TEXT, p.as_bytes());
            if on {
                f.text(r.x + r.w - 110, r.y + 34, self.fonts.ui, FOCUS, b"Selected");
            }
            self.decorate(f, Target::Row(i as u8));
        }
    }

    /// The key-entry screen: what it's for, the field (masked unless
    /// shown), the keyboard.
    pub(crate) fn key_scene(&self, f: &mut Frame) {
        let (ui, label) = (self.fonts.ui, self.fonts.label);
        self.sub_header(f, b"TEMPEST KEY");
        Self::glass(f, KEY_PANEL, GLASS_LEFT, GLASS_RIGHT);
        f.text(28, 130, ui, TEXT, b"Your token from the Tempest web app");
        f.push(Cmd::Fill {
            r: FIELD,
            color: rp_draw::look::theme(0xC000_0000),
        });
        f.push(Cmd::Outline {
            r: FIELD,
            color: FOCUS,
        });
        let typed = &self.typed[..self.typed_n];
        let shown = if self.show_key {
            Line::new().push(typed)
        } else {
            // Dots, then the last four as typed.
            let n = typed.len();
            let mut l = Line::new();
            for _ in 0..n.saturating_sub(4).min(24) {
                l = l.push(b"*");
            }
            l.push(&typed[n.saturating_sub(4)..])
        };
        // The end of a long key stays in view.
        let text = shown.as_bytes();
        let mut start = 0;
        while ui.width(&text[start..]) > FIELD.w - 40 {
            start += 1;
        }
        f.text(FIELD.x + 14, FIELD.y + 40, ui, TEXT, &text[start..]);
        let caret = FIELD.x + 16 + ui.width(&text[start..]);
        f.push(Cmd::Fill {
            r: Rect::new(caret, FIELD.y + 16, 2, 32),
            color: FOCUS,
        });
        let count = Line::new().num(self.typed_n as i32).push(b" characters");
        f.text(28, FIELD.y + FIELD.h + 30, ui, DIM, count.as_bytes());
        f.text(
            28,
            FIELD.y + FIELD.h + 58,
            ui,
            DIM,
            b"Kept on this device only (in memory for now)",
        );
        for i in 0..CAPS {
            let Some((b, r)) = cap(i) else { continue };
            let t = Target::Cap(i);
            let lit = (b == SHIFT && self.shift) || (b == SHOW && self.show_key);
            f.push(Cmd::Fill {
                r,
                color: if self.pressed == Some(t) || lit {
                    PRESSED_KEY
                } else {
                    super::BAR_RIGHT
                },
            });
            let name: Option<&[u8]> = match b {
                BKSP => Some(b"DEL"),
                SHIFT => Some(b"ABC"),
                SHOW if self.show_key => Some(b"HIDE KEY"),
                SHOW => Some(b"SHOW KEY"),
                _ => None,
            };
            let face = [if self.shift {
                b.to_ascii_uppercase()
            } else {
                b
            }];
            let (font, text) = match name {
                Some(n) => (label, n),
                None => (ui, &face[..]),
            };
            self.centred(f, r, r.y + 33, font, TEXT, text);
            if self.focus == Some(t) {
                f.push(Cmd::Outline { r, color: FOCUS });
            }
        }
    }
}
