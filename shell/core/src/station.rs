//! Your weather station's screen (R138, R147): the Weather header's
//! Station button opens it once the station answers. The agreed card style
//! (the owner, 2026-10-31, the "2 Now, scrolled" mock-up: "Weather station
//! was supposed to look fancier"): the station's card with its temperature,
//! a wind tile with a compass, pressure and UV on gauges, rain today,
//! lightning, and the sun's arc for the day, in the user's units.
//!
//! Iterate, don't imitate (rule 0): Tempest's own app is a phone app of
//! scrolling tiles, and WM-era station software (Cumulus, WeatherLink) a
//! desktop dashboard of bitmapped dials; ours paints its rings and arcs
//! once at start, anti-aliased by the CPU ([`dial_pixel`]: integer
//! geometry, no floating point), and only their markers move, placed by
//! the same geometry ([`on_ring`], [`on_arc`]) so a marker always sits on
//! its dial; a frame costs a few rectangles and pictures.

use rp_draw::{Argb, Cmd, Frame, Image, Rect};
use rp_weather::Line;
use rp_weather::Temp;

use super::keyboard::centre_in;
use super::{DIM, GLASS_LEFT, GLASS_RIGHT, Shell, TEXT};

/// The live dot's green.
pub(crate) const LIVE: Argb = rp_draw::look::theme(0xFF5E_E07A);
/// Lightning's amber.
const BOLT: Argb = rp_draw::look::theme(0xFFF2_B45A);

/// The dials painted at start, in [`DIALS`]'s order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dial {
    /// The wind's compass ring, with ticks at the four points.
    Ring,
    /// A half-circle gauge, coloured from low to high (UV).
    Uv,
    /// A half-circle gauge, plain (pressure).
    Gauge,
    /// The sun's arc over the horizon.
    Sun,
    /// A marker: a white dot.
    Dot,
    /// The sun on its arc.
    SunDot,
}

/// Every dial, and its size.
pub const DIALS: [(Dial, u16, u16); 6] = [
    (Dial::Ring, RING, RING),
    (Dial::Uv, GAUGE_W, GAUGE_H),
    (Dial::Gauge, GAUGE_W, GAUGE_H),
    (Dial::Sun, ARC_W, ARC_H),
    (Dial::Dot, DOT, DOT),
    (Dial::SunDot, SUN_DOT, SUN_DOT),
];
/// The words all the dials take.
pub const DIAL_WORDS: usize = {
    let mut n = 0;
    let mut i = 0;
    while i < DIALS.len() {
        n += DIALS[i].1 as usize * DIALS[i].2 as usize;
        i += 1;
    }
    n
};

/// The compass ring's box and radius (its centre is the box's).
const RING: u16 = 96;
const RING_R: i32 = 44;
/// The gauges' box; their centre on the bottom edge, radius and width.
const GAUGE_W: u16 = 120;
const GAUGE_H: u16 = 64;
const GAUGE_R: i32 = 52;
const GAUGE_HALF: i32 = 4;
/// The sun's arc: a half-circle on the horizon.
const ARC_W: u16 = 140;
const ARC_H: u16 = 70;
const ARC_R: i32 = 62;
/// The markers.
const DOT: u16 = 14;
const SUN_DOT: u16 = 22;

/// sin of 0..=90 degrees, x 16384.
const SIN: [i32; 91] = [
    0, 286, 572, 857, 1143, 1428, 1713, 1997, 2280, 2563, 2845, 3126, 3406, 3686, 3964, 4240, 4516,
    4790, 5063, 5334, 5604, 5872, 6138, 6402, 6664, 6924, 7182, 7438, 7692, 7943, 8192, 8438, 8682,
    8923, 9162, 9397, 9630, 9860, 10087, 10311, 10531, 10749, 10963, 11174, 11381, 11585, 11786,
    11982, 12176, 12365, 12551, 12733, 12911, 13085, 13255, 13421, 13583, 13741, 13894, 14044,
    14189, 14330, 14466, 14598, 14726, 14849, 14968, 15082, 15191, 15296, 15396, 15491, 15582,
    15668, 15749, 15826, 15897, 15964, 16026, 16083, 16135, 16182, 16225, 16262, 16294, 16322,
    16344, 16362, 16374, 16382, 16384,
];

/// sin and cos of `deg` (any whole degrees), x 16384.
fn sin_cos(deg: i32) -> (i32, i32) {
    let d = deg.rem_euclid(360);
    let s = |d: i32| match d {
        0..=90 => SIN[d as usize],
        91..=180 => SIN[(180 - d) as usize],
        181..=270 => -SIN[(d - 180) as usize],
        _ => -SIN[(360 - d) as usize],
    };
    (s(d), s((d + 90).rem_euclid(360)))
}

/// The offset from a ring's centre of the point `deg` round it (0 north,
/// clockwise, as a wind's direction), at radius `r`.
#[must_use]
pub fn on_ring(deg: u16, r: i32) -> (i32, i32) {
    let (s, c) = sin_cos(i32::from(deg));
    ((r * s + 8192) >> 14, -((r * c + 8192) >> 14))
}

/// The offset from a half-circle's centre of the point `t` (0-1024) along
/// it, left (0) over the top to right (1024), at radius `r`.
#[must_use]
pub fn on_arc(t: u32, r: i32) -> (i32, i32) {
    let deg = 270 + (t.min(1024) * 180 / 1024) as i32;
    on_ring(deg as u16, r)
}

/// The integer square root (floor).
fn isqrt(v: u32) -> u32 {
    let mut x = v;
    let mut y = x.div_ceil(2);
    if v < 2 {
        return v;
    }
    while y < x {
        x = y;
        y = (x + v / x) / 2;
    }
    x
}

/// How much of a pixel at distance `d16` (1/16 px) from a centre a line of
/// radius `r` and half-width `half` (px) covers, 0-255.
fn band(d16: i32, r: i32, half16: i32) -> u32 {
    let off = (d16 - r * 16).abs();
    // Half a pixel of soft edge each side.
    (((half16 + 8 - off).clamp(0, 16)) * 255 / 16) as u32
}

/// `d16` from `(cx2, cy2)` (half pixels) to the middle of pixel `(x, y)`.
fn dist16(x: u16, y: u16, cx2: i32, cy2: i32) -> i32 {
    let (dx, dy) = (2 * i32::from(x) + 1 - cx2, 2 * i32::from(y) + 1 - cy2);
    isqrt(((dx * dx + dy * dy) as u32) * 64) as i32
}

/// A low-to-high scale's colour at `t` (0-1024): green, yellow, orange,
/// red, violet (UV's).
fn scale(t: u32) -> u32 {
    const STOPS: [u32; 5] = [0x5E_E07A, 0xF2_D04A, 0xF2_9A3A, 0xE8_504A, 0xB0_5AE0];
    let t = t.min(1023) * 4;
    let (i, f) = ((t / 1024) as usize, t % 1024);
    let (a, b) = (STOPS[i], STOPS[(i + 1).min(4)]);
    let mix = |s: u32| (((a >> s) & 0xFF) * (1024 - f) + ((b >> s) & 0xFF) * f) / 1024;
    mix(16) << 16 | mix(8) << 8 | mix(0)
}

/// Dial `d`'s pixel at `(x, y)` in its box (straight-alpha ARGB): what the
/// image paints once at start.
#[must_use]
pub fn dial_pixel(d: Dial, x: u16, y: u16) -> u32 {
    let argb = |a: u32, rgb: u32| a << 24 | rgb;
    match d {
        Dial::Ring => {
            let c2 = i32::from(RING);
            let d16 = dist16(x, y, c2, c2);
            let mut a = band(d16, RING_R, 20);
            // Ticks at north, east, south, west, inside the ring.
            let (dx, dy) = (2 * i32::from(x) + 1 - c2, 2 * i32::from(y) + 1 - c2);
            let inside = (34 * 16..=41 * 16).contains(&d16);
            if inside && (dx.abs() <= 3 || dy.abs() <= 3) {
                a = a.max(200);
            }
            argb(a * 0xCC / 255, 0xCCD3DA)
        }
        Dial::Uv | Dial::Gauge => {
            let (cx2, cy2) = (i32::from(GAUGE_W), 2 * i32::from(GAUGE_H));
            let a = band(dist16(x, y, cx2, cy2), GAUGE_R, GAUGE_HALF * 16);
            if d == Dial::Uv {
                argb(a, scale(u32::from(x) * 1024 / u32::from(GAUGE_W - 1)))
            } else {
                argb(a * 0x90 / 255, 0x8C96A0)
            }
        }
        Dial::Sun => {
            let (cx2, cy2) = (i32::from(ARC_W), 2 * i32::from(ARC_H) - 8);
            let mut a = band(dist16(x, y, cx2, cy2), ARC_R, 16) * 0xB0 / 255;
            // The horizon under it.
            if y + 4 == ARC_H || y + 5 == ARC_H {
                a = a.max(0x60);
            }
            argb(a, 0xF2_C14E)
        }
        Dial::Dot => {
            let c2 = i32::from(DOT);
            let d16 = dist16(x, y, c2, c2);
            let a = ((6 * 16 + 8 - d16).clamp(0, 16) * 255 / 16) as u32;
            let edge = d16 > 4 * 16;
            argb(a, if edge { 0x1C_2126 } else { 0xFF_FFFF })
        }
        Dial::SunDot => {
            let c2 = i32::from(SUN_DOT);
            let d16 = dist16(x, y, c2, c2);
            let core = ((7 * 16 + 8 - d16).clamp(0, 16) * 255 / 16) as u32;
            // A soft glow round it.
            let glow = ((11 * 16 - d16).clamp(0, 4 * 16) * 0x50 / 64) as u32;
            argb(core.max(glow), 0xFF_D84A)
        }
    }
}

/// The dials painted, as the shell holds them.
#[derive(Clone, Copy)]
pub struct Dials {
    /// In [`DIALS`]'s order; `None` draws the screen without them.
    pub imgs: [Option<&'static Image>; DIALS.len()],
}

impl Dials {
    /// None painted.
    pub const NONE: Self = Self {
        imgs: [None; DIALS.len()],
    };

    fn get(&self, d: Dial) -> Option<&'static Image> {
        self.imgs[d as usize]
    }
}

/// The tiles (y: 100 under the header, to 592 over the softkeys).
const HERO: Rect = Rect::new(16, 100, 448, 112);
const WIND: Rect = Rect::new(16, 220, 220, 140);
const PRESSURE: Rect = Rect::new(244, 220, 220, 140);
const UV: Rect = Rect::new(16, 368, 144, 120);
const RAIN: Rect = Rect::new(168, 368, 144, 120);
const BOLTS: Rect = Rect::new(320, 368, 144, 120);
const SUN: Rect = Rect::new(16, 496, 448, 96);

impl Shell {
    /// The dials painted at start by the device.
    pub fn set_dials(&mut self, d: Dials) {
        self.dials = d;
    }

    /// Dial `d` at `(x, y)` (its box's corner), if painted.
    fn dial(&self, f: &mut Frame, d: Dial, x: i32, y: i32) {
        if let Some(img) = self.dials.get(d)
            && x >= 0
            && y >= 0
        {
            f.push(Cmd::Image {
                x: x as u16,
                y: y as u16,
                img,
            });
        }
    }

    /// A marker centred on `(cx, cy)`.
    fn marker(&self, f: &mut Frame, d: Dial, cx: i32, cy: i32) {
        let s = i32::from(if d == Dial::SunDot { SUN_DOT } else { DOT });
        self.dial(f, d, cx - s / 2, cy - s / 2);
    }

    /// A tile: glass and its name.
    fn tile(&self, f: &mut Frame, r: Rect, name: &[u8], color: Argb) {
        Self::glass(f, r, GLASS_LEFT, GLASS_RIGHT);
        f.text(r.x + 12, r.y + 24, self.fonts.label, color, name);
    }

    /// The Station screen.
    pub(crate) fn station_scene(&self, f: &mut Frame) {
        let (ui, label, big) = (self.fonts.ui, self.fonts.label, self.fonts.big);
        self.sub_header(f, b"YOUR STATION");
        let Some(n) = self.station else {
            let r = Rect::new(16, HERO.y, 448, 40);
            Self::glass(f, r, GLASS_LEFT, GLASS_RIGHT);
            centre_in(f, r, label, TEXT, b"Waiting for your station");
            return;
        };
        let rg = &self.region;
        let us = rg.temp == Temp::F;
        let dash = Line::new().push(b"--");
        let t = |v: Option<i16>| v.map_or(dash, |v| rg.temp(v));

        // The station: its name, live, the temperature large, the rest by it.
        let area = rp_weather::settings::STATION_AREA.as_bytes();
        let name = self
            .wset
            .places()
            .iter()
            .find(|p| p.area.as_bytes() == area)
            .map_or(Line::new().push(b"YOUR STATION"), |p| {
                let mut l = Line::new();
                for &c in p.name.as_bytes() {
                    l = l.push(&[c.to_ascii_uppercase()]);
                }
                l
            });
        self.tile(f, HERO, name.as_bytes(), DIM);
        let min = self.now.wrapping_sub(self.station_at) / 60_000;
        let age = if min == 0 {
            Line::new().push(b"LIVE")
        } else {
            Line::new().push(b"READ ").num(min as i32).push(b" MIN AGO")
        };
        let aw = label.width(age.as_bytes());
        let ax = HERO.x + HERO.w - 12 - aw;
        f.push(Cmd::Fill {
            r: Rect::new(ax - 14, HERO.y + 15, 7, 7),
            color: LIVE,
        });
        f.text(ax, HERO.y + 24, label, LIVE, age.as_bytes());
        self.big_temp(f, HERO.x + 14, HERO.y + 100, n.temp);
        let rows = [
            Line::new().push(b"Feels like ").push(t(n.feels).as_bytes()),
            n.humidity.map_or(dash, |h| {
                Line::new().push(b"Humidity ").num(i32::from(h)).push(b"%")
            }),
            Line::new().push(b"Dew point ").push(t(n.dew).as_bytes()),
        ];
        for (i, l) in rows.iter().enumerate() {
            f.text(
                HERO.x + 236,
                HERO.y + 52 + 26 * i as u16,
                ui,
                TEXT,
                l.as_bytes(),
            );
        }

        // Wind: its speed, where from, gusts; the compass with a dot where
        // it comes from.
        self.tile(f, WIND, b"WIND", DIM);
        let speed = n.wind.map_or(dash, |w| Line::new().num(rg.wind_speed(w)));
        f.text(WIND.x + 12, WIND.y + 84, big, TEXT, speed.as_bytes());
        let unit: &[u8] = match rg.wind {
            rp_weather::Wind::Mph => b"mph",
            rp_weather::Wind::Kmh => b"km/h",
            rp_weather::Wind::Ms => b"m/s",
        };
        let from = n.wind_from.map_or(&b""[..], rp_weather::compass);
        f.text(
            WIND.x + 12,
            WIND.y + 108,
            ui,
            TEXT,
            Line::new().push(unit).push(b" ").push(from).as_bytes(),
        );
        let gust = n
            .gust
            .map_or(dash, |g| Line::new().push(b"gusts ").num(rg.wind_speed(g)));
        f.text(WIND.x + 12, WIND.y + 130, label, DIM, gust.as_bytes());
        let (rx, ry) = (
            i32::from(WIND.x + WIND.w - RING - 8),
            i32::from(WIND.y + 36),
        );
        self.dial(f, Dial::Ring, rx, ry);
        let c = i32::from(RING) / 2;
        // N inside the ring, under its north tick.
        centre_in(
            f,
            Rect::new((rx + c - 10) as u16, (ry + 16) as u16, 20, 14),
            label,
            DIM,
            b"N",
        );
        if let Some(d) = n.wind_from {
            let (dx, dy) = on_ring(d, RING_R);
            self.marker(f, Dial::Dot, rx + c + dx, ry + c + dy);
        }

        // Pressure on its gauge (950-1050 mb), and its trend.
        self.tile(f, PRESSURE, b"PRESSURE", DIM);
        let (gx, gy) = (
            i32::from(PRESSURE.x + (PRESSURE.w - GAUGE_W) / 2),
            i32::from(PRESSURE.y + 32),
        );
        self.dial(f, Dial::Gauge, gx, gy);
        if let Some(p) = n.pressure {
            let t = (u32::from(p).clamp(9500, 10500) - 9500) * 1024 / 1000;
            let (dx, dy) = on_arc(t, GAUGE_R);
            self.marker(
                f,
                Dial::Dot,
                gx + i32::from(GAUGE_W) / 2 + dx,
                gy + i32::from(GAUGE_H) + dy,
            );
        }
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
        centre_in(
            f,
            Rect::new(PRESSURE.x, PRESSURE.y + 98, PRESSURE.w, 20),
            ui,
            TEXT,
            pressure.as_bytes(),
        );
        let trend: &[u8] = match n.trend {
            Some(1) => b"rising",
            Some(-1) => b"falling",
            Some(_) => b"steady",
            None => b"",
        };
        centre_in(
            f,
            Rect::new(PRESSURE.x, PRESSURE.y + 118, PRESSURE.w, 18),
            label,
            DIM,
            trend,
        );

        // UV on its coloured gauge (0-11+).
        self.tile(f, UV, b"UV", DIM);
        let (ux, uy) = (i32::from(UV.x + (UV.w - GAUGE_W) / 2), i32::from(UV.y + 32));
        self.dial(f, Dial::Uv, ux, uy);
        if let Some(u) = n.uv {
            let (dx, dy) = on_arc(u32::from(u.min(11)) * 1024 / 11, GAUGE_R);
            self.marker(
                f,
                Dial::Dot,
                ux + i32::from(GAUGE_W) / 2 + dx,
                uy + i32::from(GAUGE_H) + dy,
            );
        }
        let uv_word: &[u8] = match n.uv {
            Some(0..=2) => b"low",
            Some(3..=5) => b"moderate",
            Some(6..=7) => b"high",
            Some(8..=10) => b"very high",
            Some(_) => b"extreme",
            None => b"",
        };
        let uv = n.uv.map_or(dash, |u| Line::new().num(i32::from(u)));
        centre_in(
            f,
            Rect::new(UV.x, UV.y + 80, UV.w, 20),
            ui,
            TEXT,
            uv.as_bytes(),
        );
        centre_in(
            f,
            Rect::new(UV.x, UV.y + 100, UV.w, 18),
            label,
            DIM,
            uv_word,
        );

        // Rain since midnight.
        self.tile(f, RAIN, b"RAIN TODAY", DIM);
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
        centre_in(
            f,
            Rect::new(RAIN.x, RAIN.y + 52, RAIN.w, 26),
            ui,
            TEXT,
            rain.as_bytes(),
        );
        centre_in(
            f,
            Rect::new(RAIN.x, RAIN.y + 92, RAIN.w, 18),
            label,
            DIM,
            b"since midnight",
        );

        // Lightning: strikes in three hours and the last one's distance,
        // amber when there are any.
        let struck = n.strikes_3h.is_some_and(|c| c > 0);
        self.tile(f, BOLTS, b"LIGHTNING", if struck { BOLT } else { DIM });
        let strikes = n.strikes_3h.map_or(dash, |c| Line::new().num(i32::from(c)));
        centre_in(
            f,
            Rect::new(BOLTS.x, BOLTS.y + 52, BOLTS.w, 26),
            ui,
            if struck { BOLT } else { TEXT },
            strikes.as_bytes(),
        );
        let last = match (n.strikes_3h, n.strike_km) {
            (Some(c), Some(km)) if c > 0 => {
                let d = if us {
                    u32::from(km) * 6214 / 10_000
                } else {
                    u32::from(km)
                };
                Line::new()
                    .push(b"last ")
                    .num(d as i32)
                    .push(if us { b" mi" } else { b" km" })
            }
            _ => Line::new().push(b"none in 3 h"),
        };
        centre_in(
            f,
            Rect::new(BOLTS.x, BOLTS.y + 92, BOLTS.w, 18),
            label,
            DIM,
            last.as_bytes(),
        );

        // The sun: its arc for today, where it is now, sunrise and sunset,
        // and the station's sunlight.
        self.tile(f, SUN, b"SUN", DIM);
        let (sx, sy) = (i32::from(SUN.x + 90), i32::from(SUN.y + SUN.h - ARC_H - 4));
        self.dial(f, Dial::Sun, sx, sy);
        let (rise, set) = (i32::from(self.wx.sun.0), i32::from(self.wx.sun.1));
        let now = i32::from(self.time.hour) * 60 + i32::from(self.time.minute);
        if set > rise && (rise..=set).contains(&now) {
            let t = ((now - rise) * 1024 / (set - rise)) as u32;
            let (dx, dy) = on_arc(t, ARC_R);
            self.marker(
                f,
                Dial::SunDot,
                sx + i32::from(ARC_W) / 2 + dx,
                sy + i32::from(ARC_H) - 4 + dy,
            );
        }
        let hm = |m: i32| rg.time((m / 60) as u8, (m % 60) as u8, true);
        // Sunrise left of the arc, sunset right of it, on the horizon.
        let (rise_l, set_l) = (hm(rise), hm(set));
        let base = SUN.y + SUN.h - 8;
        let rx0 = (sx as u16).saturating_sub(8 + label.width(rise_l.as_bytes()));
        f.text(rx0, base, label, DIM, rise_l.as_bytes());
        f.text(sx as u16 + ARC_W + 8, base, label, DIM, set_l.as_bytes());
        let solar = n
            .solar
            .map_or(dash, |w| Line::new().num(i32::from(w)).push(b" W/m2"));
        f.text(SUN.x + 340, SUN.y + 52, ui, TEXT, solar.as_bytes());
        f.text(SUN.x + 340, SUN.y + 76, label, DIM, b"sunlight");
    }

    /// A temperature in the big digits at baseline `y` (the big font has
    /// digits, ':' and the degree sign: a minus is drawn).
    fn big_temp(&self, f: &mut Frame, x: u16, y: u16, v: Option<i16>) {
        let big = self.fonts.big;
        let Some(v) = v else {
            f.text(x, y, self.fonts.ui, TEXT, b"--");
            return;
        };
        let l = self.region.temp(v);
        let b = l.as_bytes();
        let (neg, digits) = match b.split_first() {
            Some((b'-', rest)) => (true, rest),
            _ => (false, b),
        };
        let mut x = x;
        if neg {
            f.push(Cmd::Fill {
                r: Rect::new(x, y - 30, 22, 6),
                color: TEXT,
            });
            x += 28;
        }
        f.text(x, y, big, TEXT, digits);
    }
}

/// `v` (0-99) as two digits.
fn two(v: u32) -> [u8; 2] {
    [b'0' + (v / 10) as u8, b'0' + (v % 10) as u8]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_land_on_their_dials() {
        // The compass: north up, east right, clockwise.
        assert_eq!(on_ring(0, 44), (0, -44));
        assert_eq!(on_ring(90, 44), (44, 0));
        assert_eq!(on_ring(180, 44), (0, 44));
        assert_eq!(on_ring(270, 44), (-44, 0));
        // A gauge: left at 0, top halfway, right at the end.
        assert_eq!(on_arc(0, 52), (-52, 0));
        assert_eq!(on_arc(512, 52), (0, -52));
        assert_eq!(on_arc(1024, 52), (52, 0));
        assert_eq!(on_arc(5000, 52), (52, 0), "past the end stays at it");
        // Each marker's centre is on its painted line: the ring's pixel
        // there is the line's, the dial's middle is clear.
        let c = i32::from(RING) / 2;
        for d in [0u16, 37, 90, 200, 333] {
            let (dx, dy) = on_ring(d, RING_R);
            let p = dial_pixel(Dial::Ring, (c + dx) as u16, (c + dy) as u16);
            assert!(p >> 24 > 0x80, "{d} deg: {p:08x}");
        }
        assert_eq!(dial_pixel(Dial::Ring, RING / 2, RING / 2) >> 24, 0);
        for t in [0u32, 300, 512, 900, 1024] {
            let (dx, dy) = on_arc(t, GAUGE_R);
            let (x, y) = (i32::from(GAUGE_W) / 2 + dx, i32::from(GAUGE_H) + dy);
            let p = dial_pixel(Dial::Uv, x.clamp(0, 119) as u16, y.clamp(0, 63) as u16);
            assert!(p >> 24 > 0x80, "t {t}: {p:08x}");
        }
    }

    #[test]
    fn the_uv_scale_runs_green_to_violet_and_dots_are_round() {
        let left = dial_pixel(Dial::Uv, 0, GAUGE_H - 1) & 0xFF_FFFF;
        let right = dial_pixel(Dial::Uv, GAUGE_W - 1, GAUGE_H - 1) & 0xFF_FFFF;
        assert!(
            (left >> 8) & 0xFF > (left >> 16) & 0xFF,
            "green first: {left:06x}"
        );
        assert!(
            right & 0xFF > (right >> 8) & 0xFF,
            "violet last: {right:06x}"
        );
        // A dot: solid in the middle, clear in the corners, symmetric.
        assert_eq!(dial_pixel(Dial::Dot, 7, 7) >> 24, 255);
        assert_eq!(dial_pixel(Dial::Dot, 0, 0) >> 24, 0);
        for y in 0..DOT {
            for x in 0..DOT {
                assert_eq!(
                    dial_pixel(Dial::Dot, x, y),
                    dial_pixel(Dial::Dot, DOT - 1 - x, y)
                );
            }
        }
        assert_eq!(isqrt(0), 0);
        assert_eq!(isqrt(99), 9);
        assert_eq!(isqrt(100), 10);
        assert_eq!(isqrt(u32::MAX), 65535);
    }
}
