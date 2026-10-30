//! The Weather app's settings (build-ui-foundation.md step 4; the owner's
//! direction in weather-flows.md): the region (which sets every default as
//! phones do), then the units system on its own, the place, the data
//! source, the refresh, the icon pack and the radar's map. Plain values the
//! shell shows and the tests check; kept in RAM until the settings store
//! exists.
//!
//! The international rule (owner, 2026-10-30): any country works, US
//! defaults stay; units are named for the system ("US customary",
//! "Metric"), never for a country.

use crate::{Region, Temp, Wind};

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

/// The places offered until search exists (WF3): the sample forecast is
/// shown for any of them, labelled sample data.
pub const PLACES: [&str; 8] = [
    "Seattle", "Portland", "Denver", "Chicago", "New York", "London", "Oslo", "Tokyo",
];

/// Where the forecast comes from (WF5 decides the ADR; R125 checks each).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// The built-in sample (no network yet).
    Sample,
    /// Open-Meteo: keyless forecast.
    OpenMeteo,
    /// The owner's Tempest station, with his own key.
    Tempest,
}

impl Source {
    /// As shown.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Sample => "Sample data",
            Self::OpenMeteo => "Open-Meteo",
            Self::Tempest => "Tempest",
        }
    }

    /// The next choice (a tap on its row).
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Sample => Self::OpenMeteo,
            Self::OpenMeteo => Self::Tempest,
            Self::Tempest => Self::Sample,
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
    /// Index into [`PLACES`].
    pub place: u8,
    /// The data source.
    pub source: Source,
    /// Index into [`REFRESH`].
    pub refresh: u8,
    /// The icon pack's index (0: VClouds, the base set; packs on the card
    /// later).
    pub pack: u8,
    /// The radar's map.
    pub radar: RadarMap,
    /// The Tempest station's key.
    pub tempest_key: Secret,
}

impl Default for Settings {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl Settings {
    /// The defaults: United States, US customary, Seattle, sample data,
    /// every 3 hours, VClouds, the Night map.
    pub const DEFAULT: Self = Self {
        region: 0,
        metric: false,
        place: 0,
        source: Source::Sample,
        refresh: 3,
        pack: 0,
        // The owner, 2026-10-30, after the X51v run: "the dark map looks
        // way better then the ink map".
        radar: RadarMap::Night,
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

    /// The refresh as shown.
    #[must_use]
    pub fn refresh_name(&self) -> &'static str {
        match REFRESH[usize::from(self.refresh) % REFRESH.len()] {
            15 => "Every 15 min",
            30 => "Every 30 min",
            60 => "Every hour",
            180 => "Every 3 hours",
            _ => "By hand",
        }
    }

    /// The place's name.
    #[must_use]
    pub fn place_name(&self) -> &'static str {
        PLACES[usize::from(self.place) % PLACES.len()]
    }
}
