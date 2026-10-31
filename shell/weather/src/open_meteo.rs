//! Open-Meteo (open-meteo.com): the default forecast (free, worldwide, no
//! account, WMO codes natively) and its place search (names and postal
//! codes, worldwide). Both answers are read as untrusted JSON (see
//! [`crate::json`]): every number range-checked, anything missing left as
//! the base forecast had it.
//!
//! The forecast request the relay makes (docs/design/weather-settings-build.md):
//! `current` (temperature, feels like, humidity, code, day or night, cloud
//! cover, wind, its direction, UV), 24 `hourly` and 10 `daily` values,
//! `timezone=auto` (the place's own time) and `timeformat=unixtime`, metric.
//! Recorded answers in `tests/data/open-meteo-*.json`.

use super::json::{Json, local, tenths, whole};
use super::{DAYS, Forecast, HOURS, Name};

/// A place search's match.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Found {
    /// Its name ("Perryville").
    pub name: Name,
    /// Its state or region and country code ("Missouri, US").
    pub area: Name,
    /// Latitude and longitude, thousandths of a degree.
    pub lat: i32,
    /// Longitude, thousandths of a degree.
    pub lon: i32,
}

/// The most matches kept (the screen shows them all).
pub const FOUND_MAX: usize = 8;

/// Open-Meteo's `forecast` JSON over `base`, for `place`. `None` if it
/// carries no current reading.
#[must_use]
pub fn forecast(json: &[u8], base: &Forecast, place: Name) -> Option<Forecast> {
    let mut f = *base;
    f.place = place;
    f.source = "Open-Meteo";
    // The offset first: local hours and days depend on it.
    let mut offset = 0i64;
    Json::new(json).object(|k, v| {
        if k == b"utc_offset_seconds" {
            offset = whole(v.number(), -16 * 3600, 16 * 3600).unwrap_or(0) / 60;
        }
    })?;
    let mut have_now = false;
    Json::new(json).object(|k, v| match k {
        b"current" => {
            let _ = v.object(|k, v| {
                let n = &mut f.now;
                match k {
                    b"temperature_2m" => {
                        if let Some(t) = tenths(v.number(), -900, 700) {
                            n.temp = t;
                            have_now = true;
                        }
                    }
                    b"apparent_temperature" => {
                        n.feels = tenths(v.number(), -900, 700).unwrap_or(n.feels)
                    }
                    b"relative_humidity_2m" => {
                        n.humidity = whole(v.number(), 0, 100).map_or(n.humidity, |h| h as u8);
                    }
                    b"weather_code" => {
                        n.sky.wmo = whole(v.number(), 0, 99).map_or(n.sky.wmo, |c| c as u8)
                    }
                    b"is_day" => {
                        n.sky.night = whole(v.number(), 0, 1).map_or(n.sky.night, |d| d == 0)
                    }
                    b"cloud_cover" => {
                        n.sky.cover = whole(v.number(), 0, 100).map_or(n.sky.cover, |c| c as u8)
                    }
                    b"wind_speed_10m" => {
                        n.sky.wind = tenths(v.number(), 0, 5000).map_or(n.sky.wind, |w| w as u16);
                    }
                    b"wind_direction_10m" => {
                        n.wind_from = whole(v.number(), 0, 360).map_or(n.wind_from, |d| d as u16);
                    }
                    // UV is a decimal: rounded.
                    b"uv_index" => {
                        n.uv = v
                            .number()
                            .and_then(|u| whole(Some(u + 500), 0, 20))
                            .map_or(n.uv, |u| u as u8);
                    }
                    _ => {}
                }
            });
        }
        b"hourly" => {
            let _ = v.object(|k, v| {
                let _ = v.array(|i, v| {
                    if i >= HOURS {
                        return;
                    }
                    let h = &mut f.hours[i];
                    match k {
                        b"time" => {
                            if let Some(e) = whole(v.number(), 0, 1 << 40) {
                                h.hour = (local(e, offset).1 / 60) as u8;
                            }
                        }
                        b"temperature_2m" => {
                            h.temp = tenths(v.number(), -900, 700).unwrap_or(h.temp)
                        }
                        b"weather_code" => {
                            h.sky.wmo = whole(v.number(), 0, 99).map_or(h.sky.wmo, |c| c as u8)
                        }
                        b"precipitation_probability" => {
                            h.precip = whole(v.number(), 0, 100).map_or(h.precip, |p| p as u8);
                        }
                        b"is_day" => {
                            h.sky.night = whole(v.number(), 0, 1).map_or(h.sky.night, |d| d == 0)
                        }
                        b"cloud_cover" => {
                            h.sky.cover = whole(v.number(), 0, 100).map_or(h.sky.cover, |c| c as u8)
                        }
                        b"wind_speed_10m" => {
                            h.sky.wind =
                                tenths(v.number(), 0, 5000).map_or(h.sky.wind, |w| w as u16);
                        }
                        b"apparent_temperature" => {
                            h.feels = tenths(v.number(), -900, 700).unwrap_or(h.feels);
                        }
                        b"relative_humidity_2m" => {
                            h.humidity = whole(v.number(), 0, 100).map_or(h.humidity, |x| x as u8);
                        }
                        b"dew_point_2m" => h.dew = tenths(v.number(), -900, 700).unwrap_or(h.dew),
                        b"wind_gusts_10m" => {
                            h.gust = tenths(v.number(), 0, 5000).map_or(h.gust, |w| w as u16);
                        }
                        b"wind_direction_10m" => {
                            h.wind_from =
                                whole(v.number(), 0, 360).map_or(h.wind_from, |d| d as u16);
                        }
                        b"precipitation" => {
                            h.rain = tenths(v.number(), 0, 5000).map_or(h.rain, |r| r as u16);
                        }
                        b"uv_index" => {
                            h.uv = v
                                .number()
                                .and_then(|u| whole(Some(u + 500), 0, 20))
                                .map_or(h.uv, |u| u as u8);
                        }
                        // Metres, kept in tenths of a km.
                        b"visibility" => {
                            h.vis =
                                whole(v.number(), 0, 200_000).map_or(h.vis, |m| (m / 100) as u16);
                        }
                        _ => {}
                    }
                });
            });
        }
        b"daily" => {
            let _ = v.object(|k, v| {
                let _ = v.array(|i, v| {
                    if i >= DAYS {
                        return;
                    }
                    let d = &mut f.days[i];
                    match k {
                        // Midnight local, as unixtime.
                        b"time" => {
                            if let Some(e) = whole(v.number(), 0, 1 << 40) {
                                let (day, _) = local(e, offset);
                                d.weekday = (day + 3).rem_euclid(7) as u8;
                            }
                        }
                        b"weather_code" => {
                            if let Some(c) = whole(v.number(), 0, 99) {
                                d.sky.wmo = c as u8;
                                d.sky.night = false;
                            }
                        }
                        b"temperature_2m_max" => {
                            d.high = tenths(v.number(), -900, 700).unwrap_or(d.high)
                        }
                        b"temperature_2m_min" => {
                            d.low = tenths(v.number(), -900, 700).unwrap_or(d.low)
                        }
                        b"precipitation_probability_max" => {
                            d.precip = whole(v.number(), 0, 100).map_or(d.precip, |p| p as u8);
                        }
                        b"sunrise" if i == 0 => {
                            if let Some(e) = whole(v.number(), 0, 1 << 40) {
                                f.sun.0 = local(e, offset).1;
                            }
                        }
                        b"sunset" if i == 0 => {
                            if let Some(e) = whole(v.number(), 0, 1 << 40) {
                                f.sun.1 = local(e, offset).1;
                            }
                        }
                        _ => {}
                    }
                });
            });
        }
        _ => {}
    })?;
    have_now.then_some(f)
}

/// Open-Meteo's place search answer: the matches written to `out`, how
/// many. A match without a name or a position in range is skipped.
#[must_use]
pub fn found(json: &[u8], out: &mut [Found; FOUND_MAX]) -> usize {
    let mut n = 0;
    let _ = Json::new(json).object(|k, v| {
        if k != b"results" {
            return;
        }
        let _ = v.array(|_, v| {
            if n >= FOUND_MAX {
                return;
            }
            let (mut name, mut admin, mut cc) = (Name::EMPTY, Name::EMPTY, Name::EMPTY);
            let (mut lat, mut lon) = (None, None);
            let _ = v.object(|k, v| match k {
                b"name" => name = v.string().map_or(Name::EMPTY, Name::from_text),
                b"admin1" => admin = v.string().map_or(Name::EMPTY, Name::from_text),
                b"country_code" => cc = v.string().map_or(Name::EMPTY, Name::from_text),
                b"latitude" => lat = v.number().filter(|l| (-90_000..=90_000).contains(l)),
                b"longitude" => lon = v.number().filter(|l| (-180_000..=180_000).contains(l)),
                _ => {}
            });
            if let (false, Some(lat), Some(lon)) = (name.is_empty(), lat, lon) {
                let area = crate::Line::new()
                    .push(admin.as_bytes())
                    .push(if admin.is_empty() || cc.is_empty() {
                        b""
                    } else {
                        b", "
                    })
                    .push(cc.as_bytes());
                out[n] = Found {
                    name,
                    area: Name::from_text(area.as_bytes()),
                    lat: lat as i32,
                    lon: lon as i32,
                };
                n += 1;
            }
        });
    });
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEATTLE: &[u8] = include_bytes!("../tests/data/open-meteo-seattle.json");
    const PERRYVILLE: &[u8] = include_bytes!("../tests/data/open-meteo-find-perryville.json");
    const ZIP: &[u8] = include_bytes!("../tests/data/open-meteo-find-98101.json");

    #[test]
    fn the_recorded_forecast_fills_every_field() {
        let base = crate::sample();
        let f = forecast(SEATTLE, &base, Name::new("Seattle")).expect("a forecast");
        assert_eq!((f.place.as_str(), f.source), ("Seattle", "Open-Meteo"));
        // Recorded 2026-10-31 (UTC-7), the hour card's fields included:
        // 6.1 C, feels 3.8, 86 %, code 0, day, 16 % cloud, 6.2 km/h from
        // 85, UV 1.0.
        assert_eq!((f.now.temp, f.now.feels, f.now.humidity), (61, 38, 86));
        assert_eq!(
            (f.now.sky.wmo, f.now.sky.night, f.now.sky.cover),
            (0, false, 16)
        );
        assert_eq!((f.now.sky.wind, f.now.wind_from, f.now.uv), (62, 85, 1));
        // The first hour is 09:00 local, a Saturday: feels 2.7, 95 %, dew
        // 4.2, gusts 9.4 from 64, no rain, UV 0.25 (0), 12.2 km.
        assert_eq!((f.hours[0].hour, f.hours[1].hour), (9, 10));
        let h = f.hours[0];
        assert_eq!(
            (h.feels, h.humidity, h.dew, h.gust, h.wind_from),
            (27, 95, 42, 94, 64)
        );
        assert_eq!((h.rain, h.uv, h.vis), (0, 0, 122));
        assert_eq!(f.days[0].weekday, 5);
        assert_eq!(f.days[1].weekday, 6);
        // Every hour and day came from the answer, none left the sample's.
        for (h, b) in f.hours.iter().zip(base.hours.iter()) {
            assert_ne!(h, b);
        }
        assert!(f.days.iter().all(|d| d.high >= d.low));
        assert!(f.sun.0 > 6 * 60 && f.sun.0 < 8 * 60 && f.sun.1 > 17 * 60 && f.sun.1 < 19 * 60);
    }

    #[test]
    fn place_search_by_name_and_by_postal_code() {
        let mut out = [Found {
            name: Name::EMPTY,
            area: Name::EMPTY,
            lat: 0,
            lon: 0,
        }; FOUND_MAX];
        let n = found(PERRYVILLE, &mut out);
        assert!(n >= 3, "{n}");
        assert_eq!(out[0].name.as_str(), "Perryville");
        assert_eq!(out[0].area.as_str(), "Missouri, US");
        assert_eq!((out[0].lat, out[0].lon), (37_724, -89_861));
        let n = found(ZIP, &mut out);
        assert_eq!((n, out[0].name.as_str()), (1, "Seattle"));
        assert_eq!(out[0].area.as_str(), "Washington, US");
        // Nothing found, or not a search answer.
        assert_eq!(found(b"{\"generationtime_ms\":0.5}", &mut out), 0);
        assert_eq!(found(b"\xff", &mut out), 0);
    }

    #[test]
    fn hostile_answers_are_refused_or_ignored() {
        let base = crate::sample();
        for bad in [
            &b""[..],
            b"{}",
            b"[1,2]",
            b"{\"current\":{\"temperature_2m\":1e9}}",
            b"{\"current\":",
        ] {
            assert!(forecast(bad, &base, Name::EMPTY).is_none());
        }
        // Out of range: ignored, the base's kept; a code past 99 too.
        let f = forecast(
            b"{\"current\":{\"temperature_2m\":5,\"relative_humidity_2m\":101,\"weather_code\":300}}",
            &base,
            Name::EMPTY,
        )
        .unwrap();
        assert_eq!(
            (f.now.temp, f.now.humidity, f.now.sky.wmo),
            (50, base.now.humidity, base.now.sky.wmo)
        );
        // A name with control bytes and escapes is folded, positions checked.
        let mut out = [Found {
            name: Name::EMPTY,
            area: Name::EMPTY,
            lat: 0,
            lon: 0,
        }; FOUND_MAX];
        let n = found(
            br#"{"results":[{"name":"Z\u00fcrich\n\u0007","latitude":47.37,"longitude":8.54,"country_code":"CH"},{"name":"Nowhere","latitude":91,"longitude":0}]}"#,
            &mut out,
        );
        assert_eq!(
            (n, out[0].name.as_str(), out[0].area.as_str()),
            (1, "Zurich", "CH")
        );
    }
}
