//! What the Weather app asks the network for, and what it does with the
//! answers (docs/design/weather-settings-build.md "The dock link, version
//! 2"). The shell only names what it wants; the device's loop sends the
//! line over the dock's USB serial port and hands the answer back
//! ([`Shell::next_ask`], [`Shell::net_answer`]). One question at a time;
//! each kind is wanted at most once (a bit), so repeated taps never queue
//! duplicates. The device names *what* it wants, never an address: the
//! relay maps each kind to a fixed URL (security.md: a cable peer can't
//! make the PC fetch anything else).
//!
//! The same asks will go over TLS from the device itself once it is live
//! (quality audit C95); only the transport changes, every reader stays.

use rp_weather::Line;
use rp_weather::open_meteo::{FOUND_MAX, Found};
use rp_weather::settings::{Provider, Station};
use rp_weather::store;

use super::{Rect, Screen, Shell, W};

/// What can be asked, highest priority first.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ask {
    /// The stored settings (once, at start).
    Load,
    /// Keep the settings.
    Save,
    /// A place search for the typed text.
    Find,
    /// The dock PC's place.
    Here,
    /// The dock PC's clipboard.
    Clip,
    /// The forecast for the place shown, from the provider.
    Forecast,
    /// The Tempest station's readings.
    Tempest,
}

const ORDER: [Ask; 7] = [
    Ask::Load,
    Ask::Save,
    Ask::Find,
    Ask::Here,
    Ask::Clip,
    Ask::Forecast,
    Ask::Tempest,
];

const fn bit(a: Ask) -> u8 {
    1 << a as u8
}

/// A question ready to send: the line (ending in a newline) and, for
/// Save, the record that follows it.
pub struct Question {
    /// What it is.
    pub ask: Ask,
    line: [u8; 96],
    n: usize,
    /// Save's record.
    pub body: Option<[u8; store::LEN]>,
}

impl Question {
    /// The line to send.
    #[must_use]
    pub fn line(&self) -> &[u8] {
        &self.line[..self.n]
    }

    /// The line as a log may show it: the first two words (a key after
    /// them is never logged).
    #[must_use]
    pub fn logged(&self) -> &[u8] {
        let l = self.line();
        let mut spaces = 0;
        let end = l
            .iter()
            .position(|&c| {
                spaces += usize::from(c == b' ');
                spaces == 2 && self.ask == Ask::Tempest
                    || spaces == 2 && l.starts_with(b"RPNET TEMPEST")
            })
            .unwrap_or(l.len());
        &l[..end]
    }

    fn push(&mut self, b: &[u8]) {
        for &c in b {
            if self.n < self.line.len() - 1 {
                self.line[self.n] = c;
                self.n += 1;
            }
        }
    }
}

/// `v` thousandths of a degree as `-89.861`.
fn degrees(v: i32) -> Line {
    let a = v.unsigned_abs();
    let frac = a % 1000;
    let l = Line::new()
        .push(if v < 0 { b"-" } else { b"" })
        .num((a / 1000) as i32)
        .push(b".");
    let l = if frac < 100 { l.push(b"0") } else { l };
    let l = if frac < 10 { l.push(b"0") } else { l };
    l.num(frac as i32)
}

impl Shell {
    /// Wants `a` asked (once, however often wanted).
    pub(crate) fn want(&mut self, a: Ask) {
        self.wants |= bit(a);
    }

    /// Whether `a` is wanted or on its way.
    #[must_use]
    pub fn wants(&self, a: Ask) -> bool {
        self.wants & bit(a) != 0 || self.asking == Some(a)
    }

    /// The next question to send, if none is waiting for its answer.
    pub fn next_ask(&mut self) -> Option<Question> {
        if self.asking.is_some() {
            return None;
        }
        let a = ORDER.into_iter().find(|a| self.wants & bit(*a) != 0)?;
        self.wants &= !bit(a);
        let mut q = Question {
            ask: a,
            line: [0; 96],
            n: 0,
            body: None,
        };
        q.push(b"RPNET ");
        match a {
            Ask::Load => q.push(b"LOAD"),
            Ask::Save => {
                q.push(b"SAVE ");
                q.push(Line::new().num(store::LEN as i32).as_bytes());
                q.body = Some(store::encode(&self.wset));
            }
            Ask::Find => {
                q.push(b"FIND ");
                // Only what a place name holds: the relay checks again.
                let t = &self.typed[..self.typed_n];
                for &c in t {
                    if c.is_ascii_alphanumeric() || b" -',.".contains(&c) {
                        q.push(&[c]);
                    }
                }
            }
            Ask::Here => q.push(b"HERE"),
            Ask::Clip => q.push(b"CLIP"),
            Ask::Forecast => {
                let p = self.wset.place();
                q.push(match self.wset.provider {
                    Provider::OpenMeteo => b"OPENMETEO ",
                    Provider::Nws => b"NWS ",
                    Provider::MetNo => b"METNO ",
                    Provider::Tempest => b"TEMPEST",
                });
                if self.wset.provider == Provider::Tempest {
                    self.push_key(&mut q);
                }
                if self.wset.provider != Provider::Tempest {
                    q.push(degrees(p.lat).as_bytes());
                    q.push(b" ");
                    q.push(degrees(p.lon).as_bytes());
                }
            }
            Ask::Tempest => {
                q.push(b"TEMPEST");
                self.push_key(&mut q);
            }
        }
        q.push(b"\n");
        q.line[q.n] = 0;
        self.asking = Some(a);
        Some(q)
    }

    /// The question in flight wasn't heard (the port closed under it):
    /// wanted again, nothing else changes.
    pub fn net_retry(&mut self) {
        if let Some(a) = self.asking.take() {
            self.want(a);
        }
    }

    /// The station key typed on the device, after a space (the owner,
    /// 2026-10-31: "i should just be able to put the key in and it be
    /// accepted"): the dev relay makes the one call with it and keeps
    /// nothing (quality audit C95); the device's log never prints it
    /// ([`Question::logged`]).
    fn push_key(&self, q: &mut Question) {
        if self.wset.tempest_key.is_set() {
            q.push(b" ");
            q.push(self.wset.tempest_key.expose());
        }
    }

    /// The answer to the question in flight: `ok` with its body, or not
    /// (the relay's error, or no answer in time: the question is wanted
    /// again only if it still matters, by the next refresh or tap).
    pub fn net_answer(&mut self, ok: bool, body: &[u8]) {
        let Some(a) = self.asking.take() else { return };
        self.dock = ok || self.dock;
        if !ok {
            match a {
                Ask::Find => {
                    self.find_status = "No answer: is cargo xtask net running?";
                    self.dirty_matches();
                }
                // Windows' location off, or no fix yet (a desktop without
                // Wi-Fi positioning, 2026-10-31): searching still works.
                Ask::Here => {
                    self.find_status = "Your PC doesn't know its place; search instead";
                    self.dirty_matches();
                }
                Ask::Clip => self.show_notice("Nothing to paste"),
                Ask::Forecast | Ask::Tempest => self.station_live &= a != Ask::Tempest,
                // No stored settings: the defaults' forecast.
                Ask::Load => self.want(Ask::Forecast),
                Ask::Save => {}
            }
            return;
        }
        match a {
            Ask::Load => {
                let key = self.wset.tempest_key;
                self.wset = store::decode(body, key);
                self.apply_settings();
                self.want(Ask::Forecast);
                self.dirty(Rect::new(0, 0, W, super::H));
            }
            Ask::Save => {}
            Ask::Find | Ask::Here => {
                let mut out = [Found {
                    name: rp_weather::Name::EMPTY,
                    area: rp_weather::Name::EMPTY,
                    lat: 0,
                    lon: 0,
                }; FOUND_MAX];
                let n = rp_weather::open_meteo::found(body, &mut out);
                self.found = out;
                self.n_found = n as u8;
                if a == Ask::Here && n > 0 && self.screen == Screen::AddPlace {
                    // The PC's place: kept at once (what the tap asked for).
                    return self.add_found(0);
                }
                self.find_status = if n == 0 { "Nothing found" } else { "" };
                self.dirty_matches();
            }
            Ask::Clip => {
                if self.screen == Screen::KeyEntry {
                    // The key out of whatever was copied (the owner's
                    // clipboard held 96 characters around a 36-character
                    // token, 2026-10-31): the longest run of letters,
                    // digits and dashes, 8 to 64 long.
                    match key_in(body) {
                        Some(k) => {
                            self.forget_typed();
                            self.type_bytes(k);
                            self.field_changed();
                        }
                        None => self.show_notice("No key copied"),
                    }
                } else if self.has_keyboard() {
                    // A place: the first line.
                    let line = body
                        .split(|b| *b == b'\n' || *b == b'\r')
                        .find(|l| !l.is_empty())
                        .unwrap_or(b"");
                    self.type_bytes(line);
                    self.field_changed();
                }
            }
            Ask::Forecast => {
                let place = self.wset.place().name;
                let f = match self.wset.provider {
                    Provider::OpenMeteo => rp_weather::open_meteo::forecast(body, &self.wx, place),
                    Provider::Tempest => {
                        self.adopt_station(body);
                        rp_weather::tempest::from_tempest(body, &self.wx, self.wset.place().name)
                    }
                    Provider::Nws | Provider::MetNo => None,
                };
                if let Some(f) = f {
                    self.set_forecast(f);
                    self.fetched_at = Some(self.now);
                }
                if self.wset.station == Station::Tempest && self.wset.provider != Provider::Tempest
                {
                    self.want(Ask::Tempest);
                }
            }
            Ask::Tempest => {
                self.adopt_station(body);
                // The station's reading is "now"; the forecast stays.
                if let Some(t) = rp_weather::tempest::from_tempest(body, &self.wx, self.wx.place) {
                    let mut f = self.wx;
                    f.now = t.now;
                    self.set_forecast(f);
                    self.station_live = true;
                }
            }
        }
    }

    /// The station's place, the first time it answers: kept, shown and
    /// made home, and its forecast asked for (the owner, 2026-10-31: "that
    /// should automatically switch to your location once you connect it").
    fn adopt_station(&mut self, body: &[u8]) {
        // The instruments, every answer: the Station button and screen.
        if let Some(n) = rp_weather::tempest::station_now(body) {
            let first = self.station.is_none();
            self.station = Some(n);
            self.station_at = self.now;
            self.dirty(if first || self.screen == Screen::Station {
                Rect::new(0, 0, W, super::H)
            } else {
                super::weather_app::HEAD
            });
        }
        if self.station_live {
            return;
        }
        let Some((name, lat, lon)) = rp_weather::tempest::station_place(body) else {
            return;
        };
        let p = rp_weather::settings::Place {
            name,
            area: rp_weather::Name::new("Your station"),
            lat,
            lon,
        };
        let was = self.wset.current;
        if self.wset.add_place(p)
            && (self.wset.current != was || self.wset.home != self.wset.current)
        {
            self.wset.home = self.wset.current;
            self.station_live = true;
            self.apply_settings();
            self.want(Ask::Save);
            self.want(Ask::Forecast);
        }
    }

    /// The match list's area on T2.
    fn dirty_matches(&mut self) {
        if self.screen == Screen::AddPlace {
            self.dirty(Rect::new(0, 146, W, super::keyboard::PANEL.y - 146));
        }
    }

    /// Each frame: the search when typing paused, the refresh when due.
    pub(crate) fn net_tick(&mut self) {
        self.search_due();
        let every = u32::from(self.wset.refresh_minutes()) * 60_000;
        if let Some(at) = self.fetched_at
            && every > 0
            && self.now.wrapping_sub(at) >= every
        {
            self.fetched_at = Some(self.now);
            self.want(Ask::Forecast);
        }
    }

    /// Refresh, by the softkey or a pull past the top.
    pub(crate) fn refresh(&mut self) {
        self.want(Ask::Forecast);
        self.show_notice("Refreshing");
    }

    /// The header's line about the data: where from and how old.
    pub(crate) fn data_age(&self) -> Line {
        let src = Line::new().push(self.wx.source.as_bytes());
        if self.wants(Ask::Forecast) {
            return src.push(b", updating");
        }
        match self.fetched_at {
            None => src.push(b", sample"),
            Some(at) => {
                let min = self.now.wrapping_sub(at) / 60_000;
                if min == 0 {
                    src.push(b", just now")
                } else {
                    src.push(b", ").num(min as i32).push(b" min ago")
                }
            }
        }
    }
}

/// The longest run of letters, digits and dashes in `text`, 8 to 64 long:
/// a key as services print it, out of whatever surrounds it.
fn key_in(text: &[u8]) -> Option<&[u8]> {
    text.split(|c| !(c.is_ascii_alphanumeric() || *c == b'-'))
        .filter(|t| (8..=64).contains(&t.len()))
        .max_by_key(|t| t.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_found_in_whatever_was_copied() {
        let k = b"0a1b2c3d-4e5f-6789-abcd-ef0123456789";
        let text = [
            &b"Personal access token: "[..],
            k,
            b"\r\nCreated 10/31/2026, Liberty Hill",
        ]
        .concat();
        assert_eq!(key_in(&text), Some(&k[..]));
        assert_eq!(key_in(k), Some(&k[..]));
        assert_eq!(key_in(b"no key here"), None);
    }

    #[test]
    fn degrees_keep_their_three_places_and_sign() {
        assert_eq!(degrees(37_724).as_bytes(), b"37.724");
        assert_eq!(degrees(-89_061).as_bytes(), b"-89.061");
        assert_eq!(degrees(-5).as_bytes(), b"-0.005");
        assert_eq!(degrees(180_000).as_bytes(), b"180.000");
    }

    #[test]
    fn a_tempest_question_carries_the_key_and_its_log_never_does() {
        let mut q = Question {
            ask: Ask::Tempest,
            line: [0; 96],
            n: 0,
            body: None,
        };
        q.push(
            b"RPNET TEMPEST abcd-1234
",
        );
        assert_eq!(q.logged(), b"RPNET TEMPEST");
        let mut f = Question {
            ask: Ask::Forecast,
            line: [0; 96],
            n: 0,
            body: None,
        };
        f.push(
            b"RPNET OPENMETEO 1.000 2.000
",
        );
        assert_eq!(
            f.logged(),
            b"RPNET OPENMETEO 1.000 2.000
"
        );
    }
}
