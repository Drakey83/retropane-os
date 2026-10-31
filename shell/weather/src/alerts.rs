//! Weather alerts in force for the place shown: the US National Weather
//! Service's active alerts for a point (`api.weather.gov/alerts/active`,
//! GeoJSON), fetched by the dock's relay and read here as untrusted JSON
//! (see [`crate::json`]). The header shows the first alert's name; the
//! alert screen its headline, how severe and until when, and its text.
//! Other countries' services (MeteoAlarm and the rest) come with their
//! readers (R125, weather-flows.md "International").

use super::json::Json;
use super::{Name, fold_into};

/// Longer text kept by value (a headline, an alert's description).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Long {
    buf: [u8; 600],
    n: u16,
}

impl Long {
    /// Empty.
    pub const EMPTY: Self = Self {
        buf: [0; 600],
        n: 0,
    };

    /// A source's text folded to printable ASCII ([`Name::from_text`]'s
    /// rules: escapes and Latin letters folded, control bytes dropped,
    /// line breaks a space), cut at 600 bytes.
    #[must_use]
    pub fn from_text(raw: &[u8]) -> Self {
        let mut out = Self::EMPTY;
        out.n = fold_into(raw, &mut out.buf) as u16;
        out
    }

    /// The text.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..usize::from(self.n)]
    }
}

impl core::fmt::Debug for Long {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{:?}",
            core::str::from_utf8(self.as_bytes()).unwrap_or("")
        )
    }
}

/// How severe (the CAP scale the services use).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Severity {
    /// Unknown.
    Unknown,
    /// Minor.
    Minor,
    /// Moderate.
    Moderate,
    /// Severe.
    Severe,
    /// Extreme.
    Extreme,
}

/// One alert.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Alert {
    /// Its name ("Winter Weather Advisory").
    pub event: Name,
    /// The service's one-line headline.
    pub headline: Long,
    /// How severe.
    pub severity: Severity,
    /// Until when (the service's local ISO time, `ends` else `expires`).
    pub until: Name,
    /// What it says (the description, cut).
    pub text: Long,
}

impl Alert {
    /// None.
    pub const NONE: Self = Self {
        event: Name::EMPTY,
        headline: Long::EMPTY,
        severity: Severity::Unknown,
        until: Name::EMPTY,
        text: Long::EMPTY,
    };
}

/// The most alerts kept.
pub const ALERTS_MAX: usize = 3;

/// The NWS answer's alerts into `out`: how many (an alert without a name is
/// skipped; a broken answer gives none).
#[must_use]
pub fn from_nws(json: &[u8], out: &mut [Alert; ALERTS_MAX]) -> usize {
    let mut n = 0;
    let _ = Json::new(json).object(|k, v| {
        if k != b"features" {
            return;
        }
        let _ = v.array(|_, v| {
            if n >= ALERTS_MAX {
                return;
            }
            let mut a = Alert::NONE;
            let (mut ends, mut expires) = (Name::EMPTY, Name::EMPTY);
            let _ = v.object(|k, v| {
                if k != b"properties" {
                    return;
                }
                let _ = v.object(|k, v| match k {
                    b"event" => a.event = v.string().map_or(Name::EMPTY, Name::from_text),
                    b"headline" => a.headline = v.string().map_or(Long::EMPTY, Long::from_text),
                    b"description" => a.text = v.string().map_or(Long::EMPTY, Long::from_text),
                    b"severity" => {
                        a.severity = match v.string() {
                            Some(b"Minor") => Severity::Minor,
                            Some(b"Moderate") => Severity::Moderate,
                            Some(b"Severe") => Severity::Severe,
                            Some(b"Extreme") => Severity::Extreme,
                            _ => Severity::Unknown,
                        };
                    }
                    b"ends" => ends = v.string().map_or(Name::EMPTY, Name::from_text),
                    b"expires" => expires = v.string().map_or(Name::EMPTY, Name::from_text),
                    _ => {}
                });
            });
            a.until = if ends.is_empty() { expires } else { ends };
            if !a.event.is_empty() {
                out[n] = a;
                n += 1;
            }
        });
    });
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO: &[u8] = include_bytes!("../tests/data/nws-alerts-two.json");

    #[test]
    fn real_alerts_are_read_and_hostile_ones_refused() {
        let mut out = [Alert::NONE; ALERTS_MAX];
        let n = from_nws(TWO, &mut out);
        assert_eq!(n, 2);
        assert_eq!(out[0].event.as_str(), "Winter Weather Advisory");
        assert_eq!(out[0].severity, Severity::Moderate);
        assert_eq!(out[0].until.as_str(), "2026-11-02T11:00:00-07:00");
        assert!(
            out[0]
                .headline
                .as_bytes()
                .starts_with(b"Winter Weather Advisory issued October 31")
        );
        assert!(out[0].text.as_bytes().len() > 100);
        assert!(
            out[0]
                .text
                .as_bytes()
                .iter()
                .all(|b| (0x20..0x7F).contains(b))
        );
        // None in force, not an answer, broken: none.
        assert_eq!(
            from_nws(br#"{"type":"FeatureCollection","features":[]}"#, &mut out),
            0
        );
        assert_eq!(from_nws(b"\xff{", &mut out), 0);
        assert_eq!(
            from_nws(
                br#"{"features":[{"properties":{"headline":"no name"}}]}"#,
                &mut out
            ),
            0
        );
    }
}
