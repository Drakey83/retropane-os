//! The weather model the shell shares (docs/design/weather-flows.md): the
//! home screen's hero, the desktop widget and the Weather app all read one
//! [`Forecast`], so they can never disagree.
//!
//! - **Stored in one unit, shown in the user's.** Temperatures in tenths of
//!   a degree Celsius, wind in tenths of km/h; [`Region`] turns them into
//!   what the user reads (US customary by default, metric a setting away:
//!   the international rule). Rounding is half away from zero, both ways,
//!   so a reading converted and shown never jumps by a degree.
//! - **Conditions are WMO codes** (WMO 4677, as the open feeds send them),
//!   turned into an icon-pack id ([`Icon`], the 22 names of weather.md) by
//!   one table, so any pack fits and any source fits.
//! - **No allocation, no floats** (`no_std`, the X51v has no FPU); text goes
//!   into a small fixed [`Line`].
//!
//! Iterate, don't imitate (CLAUDE.md rule 0): Windows Mobile had no weather
//! app; SPB's and HTC's drew one fixed set from one source in one unit.
//! Ours keeps the source's codes and the user's units apart, so a new
//! source, a new pack or a new region is one table, not a new app.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod alerts;
mod json;
pub mod open_meteo;
pub mod png;
pub mod radar;
mod sample;
pub mod settings;
pub mod store;
pub mod tempest;
pub mod tiles;
pub use sample::sample;

/// The pack icons the sample forecast uses, sorted: the test image bakes
/// only these (tools/shell-assets/icons.py reads this list), so it carries
/// seven small pictures, not 22 (the lean rule).
pub const SAMPLE_ICONS: [u8; 7] = [1, 2, 3, 4, 5, 9, 12];
/// Of those, the ones drawn at the forecast cards' size (the next five days
/// and the reading now): the night icons only ever show in rows.
pub const SAMPLE_CARD_ICONS: [u8; 5] = [1, 3, 5, 9, 12];

/// The degree sign in the shell's small fonts (DEL's slot,
/// tools/shell-assets/fonts.py).
pub const DEG: u8 = 0x7F;

// ---- Text ---------------------------------------------------------------

/// A short line of text built without allocating (40 bytes, more than the
/// longest line the screens show).
#[derive(Clone, Copy)]
pub struct Line {
    buf: [u8; 40],
    n: usize,
}

impl Line {
    /// Empty.
    #[must_use]
    pub const fn new() -> Self {
        Self { buf: [0; 40], n: 0 }
    }

    /// Adds bytes (what doesn't fit is dropped). Kept out of line: the
    /// screens call it dozens of times, and one copy keeps the image lean.
    #[must_use]
    #[inline(never)]
    pub fn push(mut self, s: &[u8]) -> Self {
        for &b in s {
            if self.n < self.buf.len() {
                self.buf[self.n] = b;
                self.n += 1;
            }
        }
        self
    }

    /// Adds a whole number.
    #[must_use]
    #[inline(never)]
    pub fn num(self, v: i32) -> Self {
        let mut digits = [0u8; 11];
        let mut i = digits.len();
        let mut u = v.unsigned_abs();
        loop {
            i -= 1;
            digits[i] = b'0' + (u % 10) as u8;
            u /= 10;
            if u == 0 {
                break;
            }
        }
        let s = if v < 0 { self.push(b"-") } else { self };
        s.push(&digits[i..])
    }

    /// The text.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.n]
    }
}

/// A name kept by value (a place, a region, a station): up to 32 bytes
/// of printable ASCII. Text from a source is folded into it
/// ([`Name::from_text`]): JSON escapes and UTF-8 Latin letters become their
/// plain letter (`Zürich` is `Zurich`, the screens' fonts are ASCII), and
/// anything else is dropped, so a hostile name can't carry control bytes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Name {
    buf: [u8; 32],
    n: u8,
}

impl core::fmt::Debug for Name {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

impl Name {
    /// Empty.
    pub const EMPTY: Self = Self { buf: [0; 32], n: 0 };

    /// `s` as it is (a name in the code: ASCII), cut at 32 bytes.
    #[must_use]
    pub const fn new(s: &str) -> Self {
        let b = s.as_bytes();
        let mut out = Self::EMPTY;
        while (out.n as usize) < b.len() && (out.n as usize) < out.buf.len() {
            out.buf[out.n as usize] = b[out.n as usize];
            out.n += 1;
        }
        out
    }

    /// A source's text (a JSON string's raw bytes) folded to ASCII.
    #[must_use]
    pub fn from_text(raw: &[u8]) -> Self {
        let mut out = Self::EMPTY;
        out.n = fold_into(raw, &mut out.buf) as u8;
        // No leading or trailing spaces.
        let t = out.as_str().trim();
        Self::new_bytes(t.as_bytes())
    }

    fn new_bytes(b: &[u8]) -> Self {
        let mut out = Self::EMPTY;
        let n = b.len().min(out.buf.len());
        out.buf[..n].copy_from_slice(&b[..n]);
        out.n = n as u8;
        out
    }

    /// The text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        // Only printable ASCII is ever stored.
        core::str::from_utf8(&self.buf[..usize::from(self.n)]).unwrap_or("")
    }

    /// The bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..usize::from(self.n)]
    }

    /// Whether it is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.n == 0
    }
}

/// `raw` (a JSON string's raw bytes) folded into `out` as printable ASCII:
/// escapes undone (a line break a space), UTF-8 Latin letters to their
/// plain letter, anything else dropped; the bytes written.
pub(crate) fn fold_into(raw: &[u8], out: &mut [u8]) -> usize {
    let mut n = 0;
    let mut i = 0;
    while i < raw.len() {
        let c = raw[i];
        let (fold, step) = match c {
            b'\\' => match raw.get(i + 1) {
                Some(b'u') => {
                    let cp = raw
                        .get(i + 2..i + 6)
                        .and_then(|h| core::str::from_utf8(h).ok())
                        .and_then(|h| u32::from_str_radix(h, 16).ok());
                    (cp.and_then(fold_latin), 6)
                }
                Some(&e @ (b'"' | b'/')) => (Some(e), 2),
                Some(b'n' | b't') => (Some(b' '), 2),
                _ => (None, 2),
            },
            0x20..=0x7E => (Some(c), 1),
            0xC2 | 0xC3 => (
                raw.get(i + 1)
                    .and_then(|&n| fold_latin((u32::from(c & 0x1F) << 6) | u32::from(n & 0x3F))),
                2,
            ),
            0xC4..=0xDF => (None, 2),
            0xE0..=0xEF => (None, 3),
            0xF0..=0xF7 => (None, 4),
            _ => (None, 1),
        };
        if let Some(b) = fold
            && n < out.len()
            // Runs of spaces kept to one.
            && !(b == b' ' && n > 0 && out[n - 1] == b' ')
        {
            out[n] = b;
            n += 1;
        }
        i += step;
    }
    n
}

/// A Latin-1 letter (U+00C0..U+00FF) as its plain ASCII letter; ASCII
/// itself as is; anything else `None`.
fn fold_latin(cp: u32) -> Option<u8> {
    const UPPER: &[u8; 32] = b"AAAAAAACEEEEIIIIDNOOOOOxOUUUUYTs";
    const LOWER: &[u8; 32] = b"aaaaaaaceeeeiiiidnooooo/ouuuuyty";
    match cp {
        0x20..=0x7E => Some(cp as u8),
        0xC0..=0xDF => Some(UPPER[(cp - 0xC0) as usize]).filter(|&b| b != b'x'),
        0xE0..=0xFF => Some(LOWER[(cp - 0xE0) as usize]).filter(|&b| b != b'/'),
        _ => None,
    }
}

impl Default for Line {
    fn default() -> Self {
        Self::new()
    }
}

/// `a / b` rounded half away from zero (`b` > 0).
const fn div_round(a: i32, b: i32) -> i32 {
    if a >= 0 {
        (a + b / 2) / b
    } else {
        -((-a + b / 2) / b)
    }
}

// ---- Units and regions --------------------------------------------------

/// Temperature unit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Temp {
    /// Degrees Fahrenheit.
    F,
    /// Degrees Celsius.
    C,
}

/// Wind speed unit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wind {
    /// Miles per hour.
    Mph,
    /// Kilometres per hour.
    Kmh,
    /// Metres per second (the Nordic countries' habit).
    Ms,
}

/// How the user reads numbers and times: units and the clock. US customary
/// is the default (the owner is in America); everything else is a setting.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Region {
    /// Temperature.
    pub temp: Temp,
    /// Wind.
    pub wind: Wind,
    /// 12-hour clock with AM and PM.
    pub clock12: bool,
}

impl Region {
    /// US customary: °F, mph, 12-hour clock.
    pub const US: Self = Self {
        temp: Temp::F,
        wind: Wind::Mph,
        clock12: true,
    };
    /// Metric: °C, km/h, 24-hour clock.
    pub const METRIC: Self = Self {
        temp: Temp::C,
        wind: Wind::Kmh,
        clock12: false,
    };

    /// A temperature (tenths of °C) as a whole number in this region's unit.
    #[must_use]
    pub const fn degrees(&self, c10: i16) -> i32 {
        let c10 = c10 as i32;
        match self.temp {
            Temp::C => div_round(c10, 10),
            Temp::F => div_round(c10 * 9 + 1600, 50),
        }
    }

    /// "72°".
    #[must_use]
    #[inline(never)]
    pub fn temp(&self, c10: i16) -> Line {
        Line::new().num(self.degrees(c10)).push(&[DEG])
    }

    /// A wind speed (tenths of km/h) as a whole number in this region's unit.
    #[must_use]
    pub const fn wind_speed(&self, kmh10: u16) -> i32 {
        let k = kmh10 as i32;
        match self.wind {
            Wind::Kmh => div_round(k, 10),
            // 1 mile = 1.609344 km.
            Wind::Mph => div_round(k * 1000, 16093),
            Wind::Ms => div_round(k * 10, 360),
        }
    }

    /// "8 mph NW".
    #[must_use]
    #[inline(never)]
    pub fn wind(&self, kmh10: u16, from_deg: u16) -> Line {
        let unit: &[u8] = match self.wind {
            Wind::Mph => b" mph ",
            Wind::Kmh => b" km/h ",
            Wind::Ms => b" m/s ",
        };
        Line::new()
            .num(self.wind_speed(kmh10))
            .push(unit)
            .push(compass(from_deg))
    }

    /// A time of day: "6 PM" or "18:00" (`with_minutes` adds ":05" on the
    /// 12-hour clock).
    #[must_use]
    #[inline(never)]
    pub fn time(&self, hour: u8, minute: u8, with_minutes: bool) -> Line {
        let (h, m) = (hour % 24, minute % 60);
        if self.clock12 {
            let h12 = if h % 12 == 0 { 12 } else { h % 12 };
            let mut l = Line::new().num(i32::from(h12));
            if with_minutes {
                l = l.push(b":").push(&two(m));
            }
            l.push(if h < 12 { b" AM" } else { b" PM" })
        } else {
            Line::new().push(&two(h)).push(b":").push(&two(m))
        }
    }
}

/// Two digits.
fn two(n: u8) -> [u8; 2] {
    let n = n % 100;
    [b'0' + n / 10, b'0' + n % 10]
}

/// The compass point a wind blows from (16 points).
#[must_use]
pub fn compass(deg: u16) -> &'static [u8] {
    const P: [&[u8]; 16] = [
        b"N", b"NNE", b"NE", b"ENE", b"E", b"ESE", b"SE", b"SSE", b"S", b"SSW", b"SW", b"WSW",
        b"W", b"WNW", b"NW", b"NNW",
    ];
    P[usize::from(((deg % 360) * 2 + 22) / 45 % 16)]
}

// ---- Conditions ---------------------------------------------------------

/// The sky at one moment, as a source sends it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sky {
    /// The WMO weather code (4677 family, as the open feeds use it).
    pub wmo: u8,
    /// Between sunset and sunrise.
    pub night: bool,
    /// Cloud cover, percent (splits WMO 3 into cloudy and overcast).
    pub cover: u8,
    /// Wind, tenths of km/h (strong wind under a dry sky shows as windy).
    pub wind: u16,
}

/// An icon in a weather pack: the 22 names of docs/design/weather.md, the
/// pack's file `01` to `22`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Icon {
    /// Clear sky, day.
    ClearDay = 1,
    /// Clear sky, night.
    ClearNight,
    /// Partly cloudy, day.
    PartlyDay,
    /// Partly cloudy, night.
    PartlyNight,
    /// Cloudy.
    Cloudy,
    /// Overcast.
    Overcast,
    /// Fog.
    Fog,
    /// Drizzle.
    Drizzle,
    /// Light rain.
    LightRain,
    /// Heavy rain.
    HeavyRain,
    /// Freezing rain.
    FreezingRain,
    /// Rain showers, day.
    ShowersDay,
    /// Rain showers, night.
    ShowersNight,
    /// Light snow.
    LightSnow,
    /// Heavy snow.
    HeavySnow,
    /// Sleet.
    Sleet,
    /// Snow showers, day.
    SnowShowersDay,
    /// Snow showers, night.
    SnowShowersNight,
    /// Thunderstorm.
    Thunder,
    /// Thunderstorm with hail.
    ThunderHail,
    /// Windy.
    Windy,
    /// Unknown or no data.
    Unknown,
}

/// Wind at or above this (tenths of km/h, 40 km/h: a fresh gale's start on
/// the Beaufort scale is 62, a "windy" day in forecasts is about 40) under a
/// dry, at most partly cloudy sky shows as windy.
const WINDY_KMH10: u16 = 400;
/// Cloud cover from which WMO 3 is overcast rather than cloudy.
const OVERCAST_COVER: u8 = 90;

impl Icon {
    /// The pack file's number, 1 to 22.
    #[must_use]
    pub const fn id(self) -> u8 {
        self as u8
    }

    /// From a pack file number (anything else is unknown).
    #[must_use]
    pub const fn from_id(id: u8) -> Self {
        match id {
            1 => Self::ClearDay,
            2 => Self::ClearNight,
            3 => Self::PartlyDay,
            4 => Self::PartlyNight,
            5 => Self::Cloudy,
            6 => Self::Overcast,
            7 => Self::Fog,
            8 => Self::Drizzle,
            9 => Self::LightRain,
            10 => Self::HeavyRain,
            11 => Self::FreezingRain,
            12 => Self::ShowersDay,
            13 => Self::ShowersNight,
            14 => Self::LightSnow,
            15 => Self::HeavySnow,
            16 => Self::Sleet,
            17 => Self::SnowShowersDay,
            18 => Self::SnowShowersNight,
            19 => Self::Thunder,
            20 => Self::ThunderHail,
            21 => Self::Windy,
            _ => Self::Unknown,
        }
    }

    /// The icon for a sky (weather.md's table).
    #[must_use]
    pub const fn of(s: Sky) -> Self {
        let dry_and_light = s.wmo <= 2;
        if dry_and_light && s.wind >= WINDY_KMH10 {
            return Self::Windy;
        }
        let (day, night) = match s.wmo {
            0 | 1 => (Self::ClearDay, Self::ClearNight),
            2 => (Self::PartlyDay, Self::PartlyNight),
            3 if s.cover >= OVERCAST_COVER => (Self::Overcast, Self::Overcast),
            3 => (Self::Cloudy, Self::Cloudy),
            45 | 48 => (Self::Fog, Self::Fog),
            51 | 53 | 55 => (Self::Drizzle, Self::Drizzle),
            61 => (Self::LightRain, Self::LightRain),
            63 | 65 => (Self::HeavyRain, Self::HeavyRain),
            56 | 57 | 66 | 67 => (Self::FreezingRain, Self::FreezingRain),
            80..=82 => (Self::ShowersDay, Self::ShowersNight),
            71 | 77 => (Self::LightSnow, Self::LightSnow),
            73 | 75 => (Self::HeavySnow, Self::HeavySnow),
            85 | 86 => (Self::SnowShowersDay, Self::SnowShowersNight),
            95 => (Self::Thunder, Self::Thunder),
            96 | 99 => (Self::ThunderHail, Self::ThunderHail),
            _ => (Self::Unknown, Self::Unknown),
        };
        if s.night { night } else { day }
    }

    /// What it is, in words.
    #[must_use]
    pub const fn words(self) -> &'static str {
        match self {
            Self::ClearDay => "Sunny",
            Self::ClearNight => "Clear",
            Self::PartlyDay | Self::PartlyNight => "Partly cloudy",
            Self::Cloudy => "Cloudy",
            Self::Overcast => "Overcast",
            Self::Fog => "Fog",
            Self::Drizzle => "Drizzle",
            Self::LightRain => "Light rain",
            Self::HeavyRain => "Heavy rain",
            Self::FreezingRain => "Freezing rain",
            Self::ShowersDay | Self::ShowersNight => "Showers",
            Self::LightSnow => "Light snow",
            Self::HeavySnow => "Heavy snow",
            Self::Sleet => "Sleet",
            Self::SnowShowersDay | Self::SnowShowersNight => "Snow showers",
            Self::Thunder => "Thunderstorms",
            Self::ThunderHail => "Storms and hail",
            Self::Windy => "Windy",
            Self::Unknown => "No data",
        }
    }
}

// ---- The forecast -------------------------------------------------------

/// The reading now.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Now {
    /// Temperature, tenths of °C.
    pub temp: i16,
    /// Feels like, tenths of °C.
    pub feels: i16,
    /// The sky.
    pub sky: Sky,
    /// Where the wind blows from, degrees.
    pub wind_from: u16,
    /// Relative humidity, percent.
    pub humidity: u8,
    /// UV index.
    pub uv: u8,
}

/// One hour ahead.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Hour {
    /// Hour of day, 0 to 23.
    pub hour: u8,
    /// Temperature, tenths of °C.
    pub temp: i16,
    /// The sky.
    pub sky: Sky,
    /// Chance of rain or snow, percent.
    pub precip: u8,
    /// Feels like, tenths of °C.
    pub feels: i16,
    /// Relative humidity, percent.
    pub humidity: u8,
    /// Dew point, tenths of °C.
    pub dew: i16,
    /// Wind gusts, tenths of km/h.
    pub gust: u16,
    /// Wind from, degrees.
    pub wind_from: u16,
    /// Rain or snow that hour, tenths of a mm.
    pub rain: u16,
    /// UV index.
    pub uv: u8,
    /// Visibility, tenths of a km.
    pub vis: u16,
}

/// One day ahead.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Day {
    /// 0 Monday to 6 Sunday.
    pub weekday: u8,
    /// High, tenths of °C.
    pub high: i16,
    /// Low, tenths of °C.
    pub low: i16,
    /// The day's sky.
    pub sky: Sky,
    /// Chance of rain or snow, percent.
    pub precip: u8,
}

/// Hours held: a day ahead.
pub const HOURS: usize = 24;
/// Days held: today and nine more.
pub const DAYS: usize = 10;

/// Everything the screens show for one place.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Forecast {
    /// The place's name.
    pub place: Name,
    /// The source, for the screens' credit line.
    pub source: &'static str,
    /// Sunrise and sunset today, minutes after midnight.
    pub sun: (u16, u16),
    /// The reading now.
    pub now: Now,
    /// The next hours, the current one first.
    pub hours: [Hour; HOURS],
    /// Today, then the next days.
    pub days: [Day; DAYS],
}

impl Forecast {
    /// The lowest low and highest high over the days (the 10-day bars' scale).
    #[must_use]
    pub fn range(&self) -> (i16, i16) {
        self.days.iter().fold((i16::MAX, i16::MIN), |(lo, hi), d| {
            (lo.min(d.low), hi.max(d.high))
        })
    }

    /// The one-line summary under the temperature: the sky now, and the
    /// first change in the hours ahead ("Partly cloudy, rain at 3 PM").
    #[must_use]
    pub fn summary(&self, region: &Region) -> Line {
        let now = Icon::of(self.now.sky);
        let mut l = Line::new().push(now.words().as_bytes());
        let wet = |s: Sky| matches!(s.wmo, 51..=67 | 80..=82 | 95..=99);
        if !wet(self.now.sky)
            && let Some(h) = self
                .hours
                .iter()
                .skip(1)
                .find(|h| wet(h.sky) && h.precip >= 40)
        {
            l = l
                .push(b", rain at ")
                .push(region.time(h.hour, 0, false).as_bytes());
        }
        l
    }
}

#[cfg(test)]
mod tests;
