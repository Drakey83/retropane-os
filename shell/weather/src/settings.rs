//! The Weather app's settings (build-ui-foundation.md step 4; the owner's
//! direction in weather-flows.md): the region (which sets every default as
//! phones do), then the units system on its own, the place, the data
//! source, the refresh, the icon pack and the radar's map. Plain values the
//! shell shows and the tests check, kept by [`crate::store`] (the key
//! never: R130). The screens are docs/design/weather-settings-build.md's
//! T1-T4 (the owner's go, 2026-10-31).
//!
//! The international rule (owner, 2026-10-30): any country works, US
//! defaults stay; units are named for the system ("US customary",
//! "Metric"), never for a country.

use crate::{Name, Region, Temp, Wind};

/// A region: its name and what it sets.
#[derive(Clone, Copy, Debug)]
pub struct RegionInfo {
    /// As shown.
    pub name: &'static str,
    /// Its default units system.
    pub metric: bool,
    /// Wind under Metric (the UK keeps mph, the Nordics m/s).
    pub wind: Wind,
    /// 12-hour clock with AM and PM.
    pub clock12: bool,
}

/// The regions offered (a first set; each a row, so more cost nothing).
pub const REGIONS: [RegionInfo; 8] = [
    RegionInfo {
        name: "United States",
        metric: false,
        wind: Wind::Kmh,
        clock12: true,
    },
    RegionInfo {
        name: "Canada",
        metric: true,
        wind: Wind::Kmh,
        clock12: true,
    },
    RegionInfo {
        name: "United Kingdom",
        metric: true,
        wind: Wind::Mph,
        clock12: false,
    },
    RegionInfo {
        name: "Australia",
        metric: true,
        wind: Wind::Kmh,
        clock12: true,
    },
    RegionInfo {
        name: "Germany",
        metric: true,
        wind: Wind::Kmh,
        clock12: false,
    },
    RegionInfo {
        name: "Norway",
        metric: true,
        wind: Wind::Ms,
        clock12: false,
    },
    RegionInfo {
        name: "Japan",
        metric: true,
        wind: Wind::Ms,
        clock12: false,
    },
    RegionInfo {
        name: "India",
        metric: true,
        wind: Wind::Kmh,
        clock12: true,
    },
];

/// A place the forecast is for: found by search (Open-Meteo's geocoder)
/// or the dock PC's location.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Place {
    /// Its name ("Perryville").
    pub name: Name,
    /// Its state or region and country code ("Missouri, US").
    pub area: Name,
    /// Latitude, thousandths of a degree.
    pub lat: i32,
    /// Longitude, thousandths of a degree.
    pub lon: i32,
}

impl Place {
    /// None.
    pub const NONE: Self = Self {
        name: Name::EMPTY,
        area: Name::EMPTY,
        lat: 0,
        lon: 0,
    };

    /// Whether it is a real one: named, its position in range.
    #[must_use]
    pub const fn valid(&self) -> bool {
        !self.name.is_empty() && self.lat.abs() <= 90_000 && self.lon.abs() <= 180_000
    }
}

/// The most places kept (WF8: up to 10, one home).
pub const PLACES_MAX: usize = 10;

/// The place a fresh install starts with (the sample forecast's).
pub const SEATTLE: Place = Place {
    name: Name::new("Seattle"),
    area: Name::new("Washington, US"),
    lat: 47_606,
    lon: -122_332,
};

/// Where the forecast comes from (T3; R125 checks each one's terms before
/// the sources ADR).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Provider {
    /// Free, worldwide, no account; hourly, 16 days.
    OpenMeteo,
    /// The US National Weather Service: free, US only, official alerts.
    Nws,
    /// MET Norway: free, worldwide, strong in Europe.
    MetNo,
    /// Tempest's own forecast, tuned to the station (needs a Tempest).
    Tempest,
}

impl Provider {
    /// All, in the screen's order.
    pub const ALL: [Self; 4] = [Self::OpenMeteo, Self::Nws, Self::MetNo, Self::Tempest];

    /// As shown.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::OpenMeteo => "Open-Meteo",
            Self::Nws => "National Weather Service",
            Self::MetNo => "MET Norway",
            Self::Tempest => "Tempest's forecast",
        }
    }

    /// What it gives, as the screen says.
    #[must_use]
    pub const fn note(self) -> &'static str {
        match self {
            Self::OpenMeteo => "Free, worldwide, no account; 10 days",
            Self::Nws => "Free, US only; official alerts",
            Self::MetNo => "Free, worldwide; strong in Europe",
            Self::Tempest => "Tuned to your station; needs a Tempest",
        }
    }

    /// The short name the settings list shows.
    #[must_use]
    pub const fn short(self) -> &'static str {
        match self {
            Self::OpenMeteo => "Open-Meteo, free",
            Self::Nws => "NWS",
            Self::MetNo => "MET Norway",
            Self::Tempest => "Tempest",
        }
    }

    /// From its stored byte (an unknown one is the default).
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Nws,
            2 => Self::MetNo,
            3 => Self::Tempest,
            _ => Self::OpenMeteo,
        }
    }
}

/// A personal weather station whose live readings replace the forecast's
/// "now" (T3's list).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Station {
    /// None.
    None,
    /// A Tempest, through its owner's token.
    Tempest,
}

/// The stations T3 lists, and what each needs to connect.
pub const STATION_KINDS: [(&str, &str); 5] = [
    ("Tempest", "personal access token"),
    ("Ecowitt", "your network, no account"),
    ("Ambient Weather", "account key"),
    ("Davis WeatherLink", "account key"),
    ("Weather Underground", "station ID"),
];

/// Light or dark (ui.md "Light and dark": a core decision).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Appearance {
    /// Light by day, dark by night.
    Auto,
    /// Always light.
    Light,
    /// Always dark.
    Dark,
}

impl Appearance {
    /// All, in the switch's order.
    pub const ALL: [Self; 3] = [Self::Auto, Self::Light, Self::Dark];

    /// As shown.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }
}

/// Refresh intervals offered, minutes (0: by hand only). WF4's
/// provisional 3 hours is the default.
pub const REFRESH: [u16; 5] = [15, 30, 60, 180, 0];

/// The radar's base map.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RadarMap {
    /// Ink by day, Night by night (ui.md "Light and dark").
    Auto,
    /// Dark land and water, under the smoked glass.
    Night,
    /// Black water, white land.
    Ink,
}

impl RadarMap {
    /// As shown.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Night => "Night",
            Self::Ink => "Ink",
        }
    }

    /// The next choice.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Auto => Self::Night,
            Self::Night => Self::Ink,
            Self::Ink => Self::Auto,
        }
    }
}

/// The longest key kept (Tempest's personal access tokens are 36).
pub const KEY_MAX: usize = 64;

/// A secret (the station's key): never shown in full, never formatted by
/// `Debug`, cleared when replaced. Held in RAM only until the device's
/// protected key store exists (R125).
#[derive(Clone, Copy)]
pub struct Secret {
    bytes: [u8; KEY_MAX],
    len: u8,
}

impl core::fmt::Debug for Secret {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Secret({} bytes)", self.len)
    }
}

impl Secret {
    /// None set.
    pub const NONE: Self = Self {
        bytes: [0; KEY_MAX],
        len: 0,
    };

    /// `text` as the secret (printable ASCII only, at most [`KEY_MAX`]).
    #[must_use]
    pub fn new(text: &[u8]) -> Option<Self> {
        if text.is_empty() || text.len() > KEY_MAX || !text.iter().all(|b| (0x21..0x7F).contains(b))
        {
            return None;
        }
        let mut s = Self::NONE;
        s.bytes[..text.len()].copy_from_slice(text);
        s.len = text.len() as u8;
        Some(s)
    }

    /// Whether one is set.
    #[must_use]
    pub const fn is_set(&self) -> bool {
        self.len > 0
    }

    /// Its length.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    /// Whether it is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Forgets it (every byte zeroed).
    pub fn clear(&mut self) {
        self.bytes = [0; KEY_MAX];
        self.len = 0;
    }

    /// As shown: dots, then its last four characters (so the owner can tell
    /// which key it is), e.g. `•••• 1a2b`; "Not set" when empty.
    #[must_use]
    pub fn masked(&self) -> crate::Line {
        if !self.is_set() {
            return crate::Line::new().push(b"Not set");
        }
        let n = self.len();
        let tail = &self.bytes[n.saturating_sub(4)..n];
        crate::Line::new().push(b"****").push(b" ").push(tail)
    }

    /// The bytes, for the code that sends it to its own service only.
    #[must_use]
    pub fn expose(&self) -> &[u8] {
        &self.bytes[..self.len()]
    }
}

/// The Weather app's settings.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// Index into [`REGIONS`].
    pub region: u8,
    /// Metric rather than US customary.
    pub metric: bool,
    /// The places, the first `n_places` of them used.
    pub places: [Place; PLACES_MAX],
    /// How many places.
    pub n_places: u8,
    /// The place shown now.
    pub current: u8,
    /// The home place (the Today screen's).
    pub home: u8,
    /// The forecast's provider.
    pub provider: Provider,
    /// The station whose readings are "now".
    pub station: Station,
    /// Index into [`REFRESH`].
    pub refresh: u8,
    /// The icon pack's index (0: VClouds, the base set; packs on the card
    /// later).
    pub pack: u8,
    /// The radar's map.
    pub radar: RadarMap,
    /// Light or dark.
    pub appearance: Appearance,
    /// The Tempest station's key (never stored: R130).
    pub tempest_key: Secret,
}

impl Default for Settings {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl Settings {
    /// The defaults: United States, US customary, Seattle, Open-Meteo, no
    /// station, every 3 hours (WF4), VClouds, the Night map, Auto.
    pub const DEFAULT: Self = Self {
        region: 0,
        metric: false,
        places: {
            let mut p = [Place::NONE; PLACES_MAX];
            p[0] = SEATTLE;
            p
        },
        n_places: 1,
        current: 0,
        home: 0,
        provider: Provider::OpenMeteo,
        station: Station::None,
        refresh: 3,
        pack: 0,
        // The owner, 2026-10-30, after the X51v run: "the dark map looks
        // way better then the ink map".
        radar: RadarMap::Night,
        appearance: Appearance::Auto,
        tempest_key: Secret::NONE,
    };

    /// The region's details.
    #[must_use]
    pub fn region_info(&self) -> RegionInfo {
        REGIONS[usize::from(self.region) % REGIONS.len()]
    }

    /// Moves to the next region; its default units come with it.
    pub fn next_region(&mut self) {
        self.region = (self.region + 1) % REGIONS.len() as u8;
        self.metric = self.region_info().metric;
    }

    /// How numbers and times are shown under these settings.
    #[must_use]
    pub fn units(&self) -> Region {
        let r = self.region_info();
        if self.metric {
            Region {
                temp: Temp::C,
                wind: r.wind,
                clock12: r.clock12,
            }
        } else {
            Region {
                temp: Temp::F,
                wind: Wind::Mph,
                clock12: r.clock12,
            }
        }
    }

    /// The refresh interval in minutes (0: by hand).
    #[must_use]
    pub fn refresh_minutes(&self) -> u16 {
        REFRESH[usize::from(self.refresh) % REFRESH.len()]
    }

    /// The refresh as the settings list shows it.
    #[must_use]
    pub fn refresh_name(&self) -> &'static str {
        match self.refresh_minutes() {
            15 => "15 min, pull to refresh",
            30 => "30 min, pull to refresh",
            60 => "hourly, pull to refresh",
            180 => "3 hours, pull to refresh",
            _ => "pull to refresh",
        }
    }

    /// The places in use.
    #[must_use]
    pub fn places(&self) -> &[Place] {
        &self.places[..usize::from(self.n_places).min(PLACES_MAX)]
    }

    /// The place shown now.
    #[must_use]
    pub fn place(&self) -> Place {
        self.places()
            .get(usize::from(self.current))
            .copied()
            .unwrap_or(SEATTLE)
    }

    /// Adds `p` and shows it (or shows it, if already kept: the same name
    /// and area). `false` when all [`PLACES_MAX`] are taken or `p` isn't a
    /// real place.
    pub fn add_place(&mut self, p: Place) -> bool {
        if let Some(i) = self
            .places()
            .iter()
            .position(|q| q.name == p.name && q.area == p.area)
        {
            self.current = i as u8;
            return true;
        }
        let n = usize::from(self.n_places);
        if n >= PLACES_MAX || !p.valid() {
            return false;
        }
        self.places[n] = p;
        self.n_places += 1;
        self.current = n as u8;
        true
    }

    /// Removes place `i` (never the last one); the current and home ones
    /// keep pointing at the same places (or the one before, if theirs went).
    pub fn remove_place(&mut self, i: u8) {
        let n = usize::from(self.n_places);
        let i = usize::from(i);
        if n <= 1 || i >= n {
            return;
        }
        self.places.copy_within(i + 1..n, i);
        self.places[n - 1] = Place::NONE;
        self.n_places -= 1;
        let fix = |v: u8| {
            let v = usize::from(v);
            (if v >= i && v > 0 { v - 1 } else { v }).min(n - 2) as u8
        };
        // The one removed or after it moves back one; before it stays.
        let keep = |v: u8| if usize::from(v) < i { v } else { fix(v) };
        self.current = keep(self.current);
        self.home = keep(self.home);
    }

    /// Back to sense after reading from the store: indexes in range, the
    /// places real (else the defaults, the key kept).
    pub(crate) fn tidy(&mut self) {
        let n = usize::from(self.n_places).clamp(1, PLACES_MAX);
        if !self.places[..n].iter().all(Place::valid) {
            *self = Self {
                tempest_key: self.tempest_key,
                ..Self::DEFAULT
            };
            return;
        }
        self.n_places = n as u8;
        self.current = self.current.min(n as u8 - 1);
        self.home = self.home.min(n as u8 - 1);
        self.region %= REGIONS.len() as u8;
        self.refresh %= REFRESH.len() as u8;
        // Only the base pack exists yet.
        self.pack = 0;
    }
}
