//! The Weather app's screens below its main one (docs/design/weather-settings-build.md,
//! the owner's go 2026-10-31 on `mockups/settings-sheet.jpg`):
//!
//! - **T1 Settings:** one grouped list (Places, Weather from, Look); a tap
//!   changes or opens; changes apply at once (no Save), Back and ok return.
//! - **Places:** every place kept; a tap shows it; Remove mode takes one out.
//! - **T2 Add a place:** a field on K1 compact; the search runs as typing
//!   pauses (Open-Meteo's geocoder through the dock), up to four matches,
//!   and Near your dock PC.
//! - **T3 Weather from:** the forecast provider, and the weather stations.
//! - **The station key** on K1 compact (kept in RAM, R130).
//!
//! Iterate, don't imitate (rule 0): WM5's settings were dialogs of
//! drop-downs behind OK, and its weather programs a fixed city list; ours
//! is the modern grouped list with live changes and a worldwide search by
//! name or postal code, inside WM's frame (title bar, Back, softkeys).

use rp_draw::{Cmd, Frame, Rect};
use rp_weather::Line;
use rp_weather::settings::{
    Appearance, KEY_MAX, PLACES_MAX, Provider, STATION_KINDS, Secret, Station,
};

use super::keyboard::{self, Cap, centre_in};
use super::{DIM, FOCUS, GLASS_EDGE, GLASS_LEFT, GLASS_RIGHT, H, Screen, Shell, TEXT, Target, W};

/// The header's back control (every Weather screen).
pub(crate) const BACK: Rect = Rect::new(0, 36, 64, 58);
/// The content's top (under the header).
const TOP: u16 = 98;
const ROW_H: u16 = 38;
const LABEL_H: u16 = 18;
/// A provider's card on T3.
const CARD_H: u16 = 52;
/// The text field (T2 and the key).
const FIELD: Rect = Rect::new(16, 100, 448, 40);
/// T2's Near your dock PC card.
const HERE: Rect = Rect::new(16, 146, 448, 48);
/// The key screen's help panel (its count changes as a key is typed).
const KEY_HELP_Y: u16 = FIELD.y + FIELD.h + 8;
/// The most matches shown above the keyboard.
pub(crate) const MATCHES_SHOWN: u8 = 4;
/// How long typing must pause before the search goes (ms).
pub(crate) const SEARCH_PAUSE_MS: u32 = 600;

/// What a row of the screen up is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Item {
    /// T1: a place (shown now: ticked).
    Place(u8),
    /// T1: all the places (more than T1 shows).
    AllPlaces,
    /// T1 and the places list: add one.
    AddPlace,
    /// T1: the forecast's provider.
    Forecast,
    /// T1: the station.
    Station,
    /// T1: the radar's map.
    Radar,
    /// T1: how often it refreshes.
    Updates,
    /// T1: light or dark.
    Appearance,
    /// T1: the region and its units.
    Units,
    /// T1: the icon pack.
    Icons,
    /// T3: a provider.
    Provider(u8),
    /// T3: a station kind.
    StationKind(u8),
    /// T2: Near your dock PC.
    Here,
    /// T2: a match.
    Match(u8),
}

/// Up to 16 rows.
pub(crate) type Items = ([Item; 16], u8);

impl Shell {
    /// The rows of the screen up, in order.
    pub(crate) fn items(&self) -> Items {
        let mut out = ([Item::Icons; 16], 0u8);
        let mut add = |i: Item| {
            out.0[usize::from(out.1)] = i;
            out.1 += 1;
        };
        let n = self.wset.n_places;
        match self.screen {
            Screen::WeatherSettings => {
                // Up to two places, then Add (while there's room) and
                // Manage (remove one, choose home): reachable whatever the
                // count (the owner, 2026-10-31: no way to remove Seattle).
                if n <= 2 {
                    (0..n).for_each(|i| add(Item::Place(i)));
                    add(Item::AddPlace);
                } else {
                    add(Item::Place(self.wset.current));
                    let (c, h) = (self.wset.current, self.wset.home);
                    add(Item::Place(if h != c { h } else { u8::from(c == 0) }));
                }
                add(Item::AllPlaces);
                for i in [
                    Item::Forecast,
                    Item::Station,
                    Item::Radar,
                    Item::Updates,
                    Item::Appearance,
                    Item::Units,
                    Item::Icons,
                ] {
                    add(i);
                }
            }
            Screen::WeatherPlaces => {
                (0..n).for_each(|i| add(Item::Place(i)));
                if usize::from(n) < PLACES_MAX && !self.removing {
                    add(Item::AddPlace);
                }
            }
            Screen::AddPlace => {
                add(Item::Here);
                (0..self.n_found.min(MATCHES_SHOWN)).for_each(|i| add(Item::Match(i)));
            }
            Screen::WeatherFrom => {
                (0..Provider::ALL.len() as u8).for_each(|i| add(Item::Provider(i)));
                (0..STATION_KINDS.len() as u8).for_each(|i| add(Item::StationKind(i)));
            }
            _ => {}
        }
        out
    }

    /// Row `k` of the screen up: what it is and where.
    pub(crate) fn item(&self, k: u8) -> Option<(Item, Rect)> {
        let (items, n) = self.items();
        if k >= n {
            return None;
        }
        let mut y = TOP;
        let mut prev = None;
        for (j, it) in items[..usize::from(n)].iter().enumerate() {
            let sec = self.section(*it);
            if sec.is_some() && sec != prev {
                y += LABEL_H + 5;
            }
            prev = sec;
            let (r, gap) = match it {
                Item::Here => (HERE, 4),
                Item::Provider(_) => (Rect::new(16, y, 448, CARD_H), 4),
                _ => (Rect::new(16, y, 448, ROW_H), 0),
            };
            if j == usize::from(k) {
                return Some((*it, r));
            }
            y = r.y + r.h + gap;
        }
        None
    }

    /// The section label a row sits under (drawn above its first row).
    fn section(&self, it: Item) -> Option<&'static [u8]> {
        Some(match (self.screen, it) {
            (Screen::WeatherSettings, Item::Place(_) | Item::AllPlaces | Item::AddPlace) => {
                b"PLACES"
            }
            (
                Screen::WeatherSettings,
                Item::Forecast | Item::Station | Item::Radar | Item::Updates,
            ) => b"WEATHER FROM",
            (Screen::WeatherSettings, Item::Appearance | Item::Units | Item::Icons) => b"LOOK",
            (Screen::WeatherFrom, Item::Provider(_)) => b"FORECAST",
            (Screen::WeatherFrom, Item::StationKind(_)) => b"YOUR WEATHER STATION",
            (Screen::AddPlace, Item::Match(_)) => b"MATCHES",
            _ => return None,
        })
    }

    /// Where a target of these screens is.
    pub(crate) fn settings_rect(&self, t: Target) -> Rect {
        match t {
            Target::Row(i) => self.item(i).map_or(Rect::new(0, 0, 0, 0), |(_, r)| r),
            Target::Cap(i) => self.kb.cap(i).map_or(Rect::new(0, 0, 0, 0), |(_, r)| r),
            _ => BACK,
        }
    }

    /// The rows the screen up has.
    pub(crate) fn rows(&self) -> u8 {
        self.items().1
    }

    /// Whether the screen up has the keyboard.
    pub(crate) fn has_keyboard(&self) -> bool {
        matches!(self.screen, Screen::AddPlace | Screen::KeyEntry)
    }

    /// Opens a screen below the Weather app.
    pub(crate) fn open(&mut self, s: Screen) {
        if self.screen == Screen::KeyEntry && s != Screen::KeyEntry {
            self.forget_typed();
        }
        self.screen = s;
        self.pressed = None;
        self.anim = None;
        self.removing = false;
        self.gesture = super::Gesture::None;
        if self.has_keyboard() {
            self.forget_typed();
            self.kb = keyboard::Keyboard::default();
            self.n_found = 0;
            self.search_at = None;
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
        match self.screen {
            Screen::AddPlace if self.wset.n_places > 2 => self.open(Screen::WeatherPlaces),
            Screen::WeatherPlaces | Screen::AddPlace | Screen::WeatherFrom => {
                self.open(Screen::WeatherSettings);
            }
            Screen::KeyEntry => self.open(Screen::WeatherFrom),
            Screen::WeatherSettings => {
                self.open_weather();
                self.settle();
            }
            Screen::Weather => self.go_today(),
            Screen::Today => {}
        }
    }

    /// The typed text zeroed (a key is never left in RAM after its screen).
    pub(crate) fn forget_typed(&mut self) {
        self.typed = [0; KEY_MAX];
        self.typed_n = 0;
    }

    /// A row tapped.
    pub(crate) fn row_tapped(&mut self, k: u8) {
        let Some((it, r)) = self.item(k) else { return };
        match (self.screen, it) {
            (Screen::WeatherPlaces, Item::Place(i)) if self.removing => {
                if self.wset.n_places <= 1 {
                    self.show_notice("Keeps one place");
                } else {
                    self.wset.remove_place(i);
                    self.settings_changed(true);
                    self.dirty(Rect::new(0, TOP, W, H - 40 - TOP));
                }
            }
            // On the places list a tap shows it and makes it home; on T1 it
            // shows it.
            (_, Item::Place(i)) => {
                let home = self.screen == Screen::WeatherPlaces && self.wset.home != i;
                if home {
                    self.wset.home = i;
                }
                if i != self.wset.current || home {
                    self.wset.current = i;
                    self.settings_changed(true);
                }
                self.dirty(Rect::new(0, TOP, W, H - 40 - TOP));
            }
            (_, Item::AllPlaces) => self.open(Screen::WeatherPlaces),
            (_, Item::AddPlace) => self.open(Screen::AddPlace),
            (_, Item::Forecast | Item::Station) => self.open(Screen::WeatherFrom),
            (_, Item::Radar) => {
                self.wset.radar = self.wset.radar.next();
                self.settings_changed(false);
            }
            (_, Item::Updates) => {
                self.wset.refresh =
                    (self.wset.refresh + 1) % rp_weather::settings::REFRESH.len() as u8;
                self.settings_changed(false);
            }
            (_, Item::Appearance) => {
                let a = Appearance::ALL;
                let i = a
                    .iter()
                    .position(|x| *x == self.wset.appearance)
                    .unwrap_or(0);
                self.wset.appearance = a[(i + 1) % a.len()];
                self.settings_changed(false);
                self.dirty(Rect::new(0, 0, W, H));
            }
            (_, Item::Units) => {
                self.wset.next_region();
                self.settings_changed(false);
            }
            (_, Item::Icons) => {
                let n = rp_weather::settings::PACKS.len() as u8;
                self.wset.pack = (self.wset.pack + 1) % n;
                self.settings_changed(false);
            }
            (_, Item::Provider(i)) => {
                let p = Provider::ALL[usize::from(i) % Provider::ALL.len()];
                if p != self.wset.provider {
                    self.wset.provider = p;
                    self.settings_changed(true);
                }
                self.dirty(Rect::new(0, TOP, W, H - 40 - TOP));
            }
            (_, Item::StationKind(0)) => self.open(Screen::KeyEntry),
            (_, Item::StationKind(_)) => self.show_notice("Not connected yet"),
            (_, Item::Here) => {
                self.want(super::weather_net::Ask::Here);
                self.find_status = "Asking your dock PC";
                self.dirty(Rect::new(0, HERE.y, W, keyboard::PANEL.y - HERE.y));
            }
            (_, Item::Match(i)) => self.add_found(i),
        }
        self.dirty(r);
    }

    /// Match `i` kept as a place, and shown.
    pub(crate) fn add_found(&mut self, i: u8) {
        let Some(m) = self.found.get(usize::from(i)).copied() else {
            return;
        };
        let p = rp_weather::settings::Place {
            name: m.name,
            area: m.area,
            lat: m.lat,
            lon: m.lon,
        };
        if self.wset.add_place(p) {
            self.settings_changed(true);
            self.open(Screen::WeatherSettings);
        } else {
            self.show_notice("Ten places at most");
        }
    }

    /// The settings changed: the screens follow, the store keeps them, and
    /// (`refetch`) the forecast is asked for again.
    pub(crate) fn settings_changed(&mut self, refetch: bool) {
        self.apply_settings();
        self.want(super::weather_net::Ask::Save);
        if refetch {
            self.wx.place = self.wset.place().name;
            self.want(super::weather_net::Ask::Forecast);
        }
        self.dirty(Rect::new(0, TOP, W, H - 40 - TOP));
    }

    /// The settings into what every screen reads.
    pub(crate) fn apply_settings(&mut self) {
        let units = self.wset.units();
        if units != self.region {
            self.region = units;
            // Every number and the clock change.
            self.dirty(Rect::new(0, 0, W, H));
        }
        self.wx.place = self.wset.place().name;
    }

    /// A key on the keyboard.
    pub(crate) fn cap_tapped(&mut self, i: u8) {
        let Some((cap, r)) = self.kb.cap(i) else {
            return;
        };
        let before = self.kb;
        match self.kb.press(cap) {
            Cap::Char(c) => self.type_bytes(&[c]),
            Cap::Delete => self.typed_n = self.typed_n.saturating_sub(1),
            Cap::Done => return self.field_done(),
            Cap::Paste => self.show_notice("Clipboard empty"),
            Cap::FromPc => {
                self.want(super::weather_net::Ask::Clip);
                self.show_notice("Asking your PC");
            }
            Cap::Shift | Cap::Symbols => {}
        }
        self.typed[self.typed_n..].fill(0);
        if before.shift != self.kb.shift || before.symbols != self.kb.symbols {
            self.dirty(keyboard::PANEL);
        }
        self.field_changed();
        self.dirty(r);
    }

    /// Bytes into the field (printable ASCII, as many as fit).
    pub(crate) fn type_bytes(&mut self, b: &[u8]) {
        let max = if self.screen == Screen::KeyEntry {
            KEY_MAX
        } else {
            40
        };
        for &c in b.iter().filter(|c| (0x20..0x7F).contains(*c)) {
            if self.typed_n >= max {
                self.show_notice(if max == KEY_MAX {
                    "64 at most"
                } else {
                    "40 at most"
                });
                break;
            }
            self.typed[self.typed_n] = c;
            self.typed_n += 1;
        }
    }

    /// The field changed: redrawn, and on T2 the search waits for a pause.
    pub(crate) fn field_changed(&mut self) {
        self.dirty(FIELD);
        if self.screen == Screen::KeyEntry {
            self.dirty(Rect::new(0, KEY_HELP_Y, W, 76));
        }
        if self.screen == Screen::AddPlace {
            self.search_at = (self.typed_n >= 2).then_some(self.now.wrapping_add(SEARCH_PAUSE_MS));
            if self.typed_n < 2 {
                self.n_found = 0;
                self.find_status = "";
                self.dirty(Rect::new(
                    0,
                    HERE.y + HERE.h,
                    W,
                    keyboard::PANEL.y - HERE.y - HERE.h,
                ));
            }
        }
    }

    /// Done (the keyboard's or the right softkey): T2 adds the first match
    /// (or searches now), the key screen keeps the key.
    pub(crate) fn field_done(&mut self) {
        if self.screen == Screen::AddPlace {
            if self.n_found > 0 {
                return self.add_found(0);
            }
            if self.typed_n >= 2 {
                self.search_at = Some(self.now);
            }
            return;
        }
        match Secret::new(&self.typed[..self.typed_n]) {
            Some(k) => {
                self.wset.tempest_key.clear();
                self.wset.tempest_key = k;
                self.wset.station = Station::Tempest;
                self.forget_typed();
                self.settings_changed(true);
                self.open(Screen::WeatherFrom);
                self.show_notice("Key kept");
            }
            None if self.typed_n == 0 => {
                self.wset.tempest_key.clear();
                self.wset.station = Station::None;
                self.settings_changed(true);
                self.open(Screen::WeatherFrom);
                self.show_notice("Key forgotten");
            }
            None => self.show_notice("No spaces"),
        }
    }

    /// The search, when typing has paused (called each frame).
    pub(crate) fn search_due(&mut self) {
        if let Some(at) = self.search_at
            && self.now.wrapping_sub(at) < 1 << 31
        {
            self.search_at = None;
            self.want(super::weather_net::Ask::Find);
            self.find_status = "Searching";
            self.dirty(Rect::new(
                0,
                HERE.y + HERE.h,
                W,
                keyboard::PANEL.y - HERE.y - HERE.h,
            ));
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
            color: GLASS_EDGE,
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

    /// The rows and their section labels.
    fn rows_scene(&self, f: &mut Frame) {
        let (ui, label) = (self.fonts.ui, self.fonts.label);
        let (items, n) = self.items();
        let mut last = None;
        for k in 0..n {
            let Some((it, r)) = self.item(k) else {
                continue;
            };
            let sec = self.section(it);
            if sec.is_some() && sec != last {
                let s = sec.unwrap_or(b"");
                let w = label.width(s) + 12;
                let lr = Rect::new(r.x - 2, r.y - LABEL_H - 2, w, LABEL_H);
                Self::glass(f, lr, GLASS_LEFT, GLASS_RIGHT);
                centre_in(f, lr, label, TEXT, s);
            }
            last = sec;
            Self::glass(f, r, GLASS_LEFT, GLASS_RIGHT);
            self.row_body(f, items[usize::from(k)], r, (ui, label));
            self.decorate(f, Target::Row(k));
        }
    }

    /// One row's text.
    fn row_body(
        &self,
        f: &mut Frame,
        it: Item,
        r: Rect,
        (ui, label): (&'static rp_draw::Font, &'static rp_draw::Font),
    ) {
        let s = &self.wset;
        let base = r.y + (r.h + u16::from(ui.ascent) * 7 / 10) / 2;
        let right = |f: &mut Frame, text: &[u8], color| {
            let w = ui.width(text);
            f.text((r.x + r.w - 30).saturating_sub(w), base, ui, color, text);
        };
        let chevron = |f: &mut Frame| f.text(r.x + r.w - 20, base, ui, DIM, b">");
        match it {
            Item::Place(i) => {
                let p = s.places()[usize::from(i).min(s.places().len() - 1)];
                let on = i == s.current;
                if self.removing {
                    f.text(r.x + 12, base, ui, FOCUS, b"x");
                } else if on {
                    f.text(r.x + 12, base, ui, FOCUS, b"*");
                }
                f.text(r.x + 36, base, ui, TEXT, p.name.as_bytes());
                let note: &[u8] = if self.removing {
                    b"tap to remove"
                } else if i == s.home && on {
                    b"home, showing now"
                } else if on {
                    b"showing now"
                } else if i == s.home {
                    b"home"
                } else {
                    p.area.as_bytes()
                };
                right(f, note, DIM);
            }
            Item::AllPlaces => {
                f.text(
                    r.x + 36,
                    base,
                    ui,
                    TEXT,
                    Line::new()
                        .push(b"All ")
                        .num(i32::from(s.n_places))
                        .push(b" places")
                        .as_bytes(),
                );
                chevron(f);
            }
            Item::AddPlace => {
                f.text(r.x + 12, base, ui, FOCUS, b"+");
                f.text(r.x + 36, base, ui, FOCUS, b"Add a place");
                chevron(f);
            }
            Item::Forecast => {
                f.text(r.x + 12, base, ui, TEXT, b"Forecast");
                right(f, s.provider.short().as_bytes(), DIM);
                chevron(f);
            }
            Item::Station => {
                f.text(r.x + 12, base, ui, TEXT, b"Your station");
                let v: &[u8] = match s.station {
                    Station::Tempest if self.station_live => b"Tempest, live",
                    Station::Tempest => b"Tempest",
                    Station::None => b"None",
                };
                right(f, v, DIM);
                chevron(f);
            }
            Item::Radar => {
                f.text(r.x + 12, base, ui, TEXT, b"Radar map");
                right(f, s.radar.name().as_bytes(), DIM);
            }
            Item::Updates => {
                f.text(r.x + 12, base, ui, TEXT, b"Updates");
                right(f, s.refresh_name().as_bytes(), DIM);
            }
            Item::Appearance => {
                f.text(r.x + 12, base, ui, TEXT, b"Appearance");
                // A three-way switch, the setting lit.
                let mut x = r.x + r.w - 12 - 3 * 58;
                for a in Appearance::ALL {
                    let seg = Rect::new(x, r.y + 7, 58, r.h - 14);
                    if a == s.appearance {
                        f.push(Cmd::Fill {
                            r: seg,
                            color: FOCUS,
                        });
                    }
                    f.push(Cmd::Outline {
                        r: seg,
                        color: GLASS_EDGE,
                    });
                    let c = if a == s.appearance {
                        keyboard::DONE_TEXT
                    } else {
                        DIM
                    };
                    centre_in(f, seg, label, c, a.name().as_bytes());
                    x += 58;
                }
            }
            Item::Units => {
                f.text(r.x + 12, base, ui, TEXT, b"Units");
                let u = s.units();
                let t = Line::new().push(s.region_info().name.as_bytes()).push(
                    if u.temp == rp_weather::Temp::F {
                        b": F, mph"
                    } else {
                        b": C"
                    },
                );
                right(f, t.as_bytes(), DIM);
            }
            Item::Icons => {
                f.text(r.x + 12, base, ui, TEXT, b"Icons");
                let p = rp_weather::settings::PACKS
                    [usize::from(s.pack) % rp_weather::settings::PACKS.len()];
                right(f, p.as_bytes(), DIM);
            }
            Item::Provider(i) => {
                let p = Provider::ALL[usize::from(i) % Provider::ALL.len()];
                let on = p == s.provider;
                if on {
                    f.push(Cmd::Outline {
                        r: Rect::new(r.x + 1, r.y + 1, r.w - 2, r.h - 2),
                        color: FOCUS,
                    });
                }
                f.text(
                    r.x + 12,
                    r.y + 22,
                    ui,
                    if on { FOCUS } else { DIM },
                    if on { b"(*)" } else { b"( )" },
                );
                f.text(r.x + 50, r.y + 22, ui, TEXT, p.name().as_bytes());
                f.text(r.x + 50, r.y + 44, label, DIM, p.note().as_bytes());
            }
            Item::StationKind(i) => {
                let (name, needs) = STATION_KINDS[usize::from(i) % STATION_KINDS.len()];
                let tempest = i == 0;
                let on = tempest && s.station == Station::Tempest;
                f.text(r.x + 12, base, ui, FOCUS, if on { b"*" } else { b"+" });
                f.text(
                    r.x + 36,
                    base,
                    ui,
                    if on { TEXT } else { FOCUS },
                    name.as_bytes(),
                );
                let v: &[u8] = if on && self.station_live {
                    b"live"
                } else if on {
                    b"connected"
                } else {
                    needs.as_bytes()
                };
                right(f, v, DIM);
                if tempest {
                    chevron(f);
                }
            }
            Item::Here => {
                f.text(r.x + 12, r.y + 21, ui, FOCUS, b"o");
                f.text(r.x + 34, r.y + 21, ui, TEXT, b"Near your dock PC");
                f.text(
                    r.x + 34,
                    r.y + 41,
                    label,
                    DIM,
                    b"From the PC's own location service",
                );
            }
            Item::Match(i) => {
                if let Some(m) = self.found.get(usize::from(i)) {
                    f.text(r.x + 12, base, ui, TEXT, m.name.as_bytes());
                    right(f, m.area.as_bytes(), DIM);
                }
            }
        }
    }

    /// T1, the places list, T3.
    pub(crate) fn settings_scene(&self, f: &mut Frame) {
        let title: &[u8] = match self.screen {
            Screen::WeatherPlaces if self.removing => b"REMOVE A PLACE",
            Screen::WeatherPlaces => b"PLACES: TAP FOR HOME",
            Screen::WeatherFrom => b"WEATHER FROM",
            _ => b"WEATHER SETTINGS",
        };
        self.sub_header(f, title);
        self.rows_scene(f);
        if self.screen == Screen::WeatherPlaces && !self.removing {
            let r = Rect::new(16, H - 40 - 34, 448, 28);
            Self::glass(f, r, GLASS_LEFT, GLASS_RIGHT);
            centre_in(
                f,
                r,
                self.fonts.label,
                TEXT,
                b"Remove (right softkey) takes one out",
            );
        }
        if self.screen == Screen::WeatherFrom {
            let r = Rect::new(16, H - 40 - 34, 448, 28);
            Self::glass(f, r, GLASS_LEFT, GLASS_RIGHT);
            centre_in(
                f,
                r,
                self.fonts.label,
                TEXT,
                b"A station's readings replace the forecast's now",
            );
        }
    }

    /// The field: its text (masked on the key screen) and the caret.
    fn field(&self, f: &mut Frame, masked: bool) {
        let ui = self.fonts.ui;
        Self::glass(f, FIELD, GLASS_LEFT, GLASS_RIGHT);
        f.push(Cmd::Outline {
            r: FIELD,
            color: FOCUS,
        });
        let typed = &self.typed[..self.typed_n];
        let shown = if masked {
            // Dots, then the last four as typed (to check it).
            let n = typed.len();
            let mut l = Line::new();
            for _ in 0..n.saturating_sub(4).min(30) {
                l = l.push(b"*");
            }
            l.push(&typed[n.saturating_sub(4)..])
        } else {
            Line::new().push(typed)
        };
        let text = shown.as_bytes();
        let mut start = 0;
        while ui.width(&text[start..]) > FIELD.w - 30 {
            start += 1;
        }
        let base = FIELD.y + (FIELD.h + u16::from(ui.ascent) * 7 / 10) / 2;
        f.text(FIELD.x + 12, base, ui, TEXT, &text[start..]);
        let caret = FIELD.x + 13 + ui.width(&text[start..]);
        f.push(Cmd::Fill {
            r: Rect::new(caret, FIELD.y + 8, 2, FIELD.h - 16),
            color: FOCUS,
        });
    }

    /// The keyboard, its pressed key and focus.
    fn keyboard_scene(&self, f: &mut Frame) {
        let cap = |t: Option<Target>| match t {
            Some(Target::Cap(i)) => Some(i),
            _ => None,
        };
        self.kb.draw(
            f,
            (self.fonts.ui, self.fonts.label),
            cap(self.pressed),
            cap(self.focus),
            FOCUS,
        );
    }

    /// T2: the field, Near your dock PC, the matches, the keyboard.
    pub(crate) fn add_place_scene(&self, f: &mut Frame) {
        self.sub_header(f, b"ADD A PLACE");
        self.field(f, false);
        self.rows_scene(f);
        if self.n_found == 0 {
            let say: &[u8] = if self.find_status.is_empty() {
                b"Cities and ZIP or postal codes, worldwide"
            } else {
                self.find_status.as_bytes()
            };
            let r = Rect::new(16, HERE.y + HERE.h + 8, 448, 30);
            Self::glass(f, r, GLASS_LEFT, GLASS_RIGHT);
            centre_in(f, r, self.fonts.label, TEXT, say);
        }
        self.keyboard_scene(f);
    }

    /// The station key: what it is for, the field (masked), the keyboard.
    pub(crate) fn key_scene(&self, f: &mut Frame) {
        let label = self.fonts.label;
        self.sub_header(f, b"TEMPEST KEY");
        self.field(f, true);
        let help = Rect::new(16, KEY_HELP_Y, 448, 76);
        Self::glass(f, help, GLASS_LEFT, GLASS_RIGHT);
        let count = Line::new().num(self.typed_n as i32).push(b" characters");
        f.text(28, help.y + 22, label, TEXT, count.as_bytes());
        f.text(
            28,
            help.y + 44,
            label,
            DIM,
            b"tempestwx.com, Settings, Data Authorizations",
        );
        f.text(
            28,
            help.y + 66,
            label,
            DIM,
            b"Kept on this device (in memory for now)",
        );
        self.keyboard_scene(f);
    }
}

/// The settings' keyboard key count (the targets list's).
pub(crate) const CAPS: u8 = keyboard::CAPS;
