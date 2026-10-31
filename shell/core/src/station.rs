//! Your weather station's screen (R138, the owner's pick of the S-series
//! mock-ups, 2026-10-31: "have a tempest button show up if we are connected
//! to it"): the Weather header's Station button opens it once the station
//! answers. Eight instruments as glass cards, in the user's units, and how
//! old the reading is.
//!
//! Iterate, don't imitate (rule 0): Tempest's own app is a phone app of
//! scrolling tiles, and WM-era station software (Cumulus, WeatherLink) a
//! desktop dashboard; ours is one screen of the readings a station adds to
//! a forecast (gust, pressure and its trend, rain so far, sunlight,
//! lightning), each with what it means, sized for 480 x 640.

use rp_draw::{Argb, Frame, Rect};
use rp_weather::Line;
use rp_weather::Temp;

use super::keyboard::centre_in;
use super::{DIM, GLASS_LEFT, GLASS_RIGHT, Shell, TEXT};

/// The live dot's green.
pub(crate) const LIVE: Argb = rp_draw::look::theme(0xFF5E_E07A);

/// The cards: two columns, four rows, under the header.
const TOP: u16 = 104;
const CARD_W: u16 = 220;
const CARD_H: u16 = 100;
const GAP: u16 = 8;

const fn card(i: u16) -> Rect {
    Rect::new(
        16 + (i % 2) * (CARD_W + GAP),
        TOP + (i / 2) * (CARD_H + GAP),
        CARD_W,
        CARD_H,
    )
}

impl Shell {
    /// The Station screen.
    pub(crate) fn station_scene(&self, f: &mut Frame) {
        let (ui, label, big) = (self.fonts.ui, self.fonts.label, self.fonts.big);
        self.sub_header(f, b"YOUR STATION");
        let Some(n) = self.station else {
            let r = Rect::new(16, TOP, 448, 40);
            Self::glass(f, r, GLASS_LEFT, GLASS_RIGHT);
            centre_in(f, r, label, TEXT, b"Waiting for your station");
            return;
        };
        let rg = &self.region;
        let us = rg.temp == Temp::F;
        let dash = Line::new().push(b"--");
        let t = |v: Option<i16>| v.map_or(dash, |v| rg.temp(v));
        // Pressure: inches of mercury in US units, millibars otherwise.
        let pressure = n.pressure.map_or(dash, |p| {
            if us {
                let h = (u32::from(p) * 2953 + 5000) / 10_000;
                Line::new()
                    .num((h / 100) as i32)
                    .push(b".")
                    .push(&two(h % 100))
                    .push(b" in")
            } else {
                Line::new()
                    .num(i32::from(p / 10))
                    .push(b".")
                    .num(i32::from(p % 10))
                    .push(b" mb")
            }
        });
        let trend: &[u8] = match n.trend {
            Some(1) => b"rising",
            Some(-1) => b"falling",
            Some(_) => b"steady",
            None => b"",
        };
        let rain = n.rain_today.map_or(dash, |r| {
            if us {
                let h = (u32::from(r) * 100 + 127) / 254;
                Line::new()
                    .num((h / 100) as i32)
                    .push(b".")
                    .push(&two(h % 100))
                    .push(b" in")
            } else {
                Line::new()
                    .num(i32::from(r / 10))
                    .push(b".")
                    .num(i32::from(r % 10))
                    .push(b" mm")
            }
        });
        let gust = n
            .gust
            .map_or(dash, |g| Line::new().push(b"gusts ").num(rg.wind_speed(g)));
        let strikes = n.strikes_3h.map_or(dash, |c| Line::new().num(i32::from(c)));
        let last = match (n.strikes_3h, n.strike_km) {
            (Some(c), Some(km)) if c > 0 => {
                let d = if us {
                    u32::from(km) * 6214 / 10_000
                } else {
                    u32::from(km)
                };
                Line::new().push(b"last ").num(d as i32).push(if us {
                    b" mi away"
                } else {
                    b" km away"
                })
            }
            _ => Line::new().push(b"none in 3 hours"),
        };
        let uv = n.uv.map_or(dash, |u| Line::new().num(i32::from(u)));
        let uv_word: &[u8] = match n.uv {
            Some(0..=2) => b"low",
            Some(3..=5) => b"moderate",
            Some(6..=7) => b"high",
            Some(8..=10) => b"very high",
            Some(_) => b"extreme",
            None => b"",
        };
        let solar = n
            .solar
            .map_or(dash, |w| Line::new().num(i32::from(w)).push(b" W/m2"));
        let humid = n
            .humidity
            .map_or(dash, |h| Line::new().num(i32::from(h)).push(b"%"));
        let wind = match (n.wind, n.wind_from) {
            (Some(w), Some(d)) => rg.wind(w, d),
            _ => dash,
        };
        let cards: [(&[u8], Line, Line); 8] = [
            (
                b"TEMPERATURE",
                t(n.temp),
                Line::new().push(b"feels like ").push(t(n.feels).as_bytes()),
            ),
            (
                b"HUMIDITY",
                humid,
                Line::new().push(b"dew point ").push(t(n.dew).as_bytes()),
            ),
            (b"WIND", wind, gust),
            (b"PRESSURE", pressure, Line::new().push(trend)),
            (b"RAIN TODAY", rain, Line::new().push(b"since midnight")),
            (b"UV INDEX", uv, Line::new().push(uv_word)),
            (b"SUNLIGHT", solar, Line::new().push(b"solar radiation")),
            (b"LIGHTNING", strikes, last),
        ];
        for (i, (name, value, note)) in cards.iter().enumerate() {
            let r = card(i as u16);
            Self::glass(f, r, GLASS_LEFT, GLASS_RIGHT);
            f.text(r.x + 12, r.y + 24, label, DIM, name);
            let font = if i == 0 { big } else { ui };
            let font = if font.line > 64 { ui } else { font };
            f.text(r.x + 12, r.y + 62, font, TEXT, value.as_bytes());
            f.text(r.x + 12, r.y + 88, label, DIM, note.as_bytes());
        }
        // How old the reading is.
        let r = Rect::new(16, card(7).y + CARD_H + GAP, 448, 30);
        Self::glass(f, r, GLASS_LEFT, GLASS_RIGHT);
        f.push(rp_draw::Cmd::Fill {
            r: Rect::new(r.x + 14, r.y + 12, 7, 7),
            color: LIVE,
        });
        let min = self.now.wrapping_sub(self.station_at) / 60_000;
        let age = if min == 0 {
            Line::new().push(b"Tempest, live: read just now")
        } else {
            Line::new()
                .push(b"Tempest: read ")
                .num(min as i32)
                .push(b" min ago")
        };
        f.text(r.x + 30, r.y + 21, label, TEXT, age.as_bytes());
    }
}

/// `v` (0-99) as two digits.
fn two(v: u32) -> [u8; 2] {
    [b'0' + (v / 10) as u8, b'0' + (v % 10) as u8]
}
