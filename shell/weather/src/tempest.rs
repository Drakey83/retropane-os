//! Tempest's forecast (WeatherFlow's REST `better_forecast`, metric: °C,
//! km/h) read into the shell's [`Forecast`] (R132, the dock's dev relay:
//! docs/research/sheets/r132-dock-internet.md). The JSON comes over a USB
//! cable from a relay on the PC, so it is read as untrusted: a small
//! reader with no allocation, every number range-checked, anything missing
//! or malformed leaving that field as the base forecast had it.
//!
//! Iterate, don't imitate (rule 0): Windows Mobile weather programs (SPB,
//! HTC) parsed whole documents into a DOM; ours walks the bytes once and
//! keeps only the fields the screens show.

use super::json::{Json, local, tenths, whole};
use super::{DAYS, Day, Forecast, HOURS, Hour, Name, Sky};

/// Tempest's icon name as the WMO code the shell's icons are chosen by,
/// and whether it is the night's.
fn sky_of(icon: &[u8]) -> Option<(u8, bool)> {
    let night = icon.ends_with(b"-night");
    let base = icon
        .strip_suffix(b"-night")
        .or_else(|| icon.strip_suffix(b"-day"))
        .unwrap_or(icon);
    let wmo = match base {
        b"clear" => 0,
        b"partly-cloudy" => 2,
        b"cloudy" | b"windy" => 3,
        b"foggy" => 45,
        b"possibly-rainy" => 61,
        b"rainy" => 63,
        b"possibly-sleet" | b"sleet" => 66,
        b"possibly-snow" => 71,
        b"snow" => 73,
        b"possibly-thunderstorm" | b"thunderstorm" => 95,
        _ => return None,
    };
    Some((wmo, night))
}

/// Tempest's `better_forecast` JSON over `base`: the fields it carries
/// replace the base's, the place named `place` (the caller keeps the
/// station's name, [`place_name`]), the source "Tempest".
/// `None` if it isn't a forecast at all (no current conditions).
#[must_use]
pub fn from_tempest(json: &[u8], base: &Forecast, place: Name) -> Option<Forecast> {
    let mut f = *base;
    f.place = place;
    f.source = "Tempest";
    let mut offset = 0i64;
    let mut have_now = false;
    let mut root = Json::new(json);
    // The offset first: the local days and times depend on it.
    root.object(|k, v| {
        if k == b"timezone_offset_minutes" {
            offset = whole(v.number(), -900, 900).unwrap_or(0);
        }
    })?;
    let mut root = Json::new(json);
    root.object(|k, v| match k {
        b"current_conditions" => {
            let _ = v.object(|k, v| match k {
                b"air_temperature" => {
                    if let Some(t) = tenths(v.number(), -900, 700) {
                        f.now.temp = t;
                        have_now = true;
                    }
                }
                b"feels_like" => f.now.feels = tenths(v.number(), -900, 700).unwrap_or(f.now.feels),
                b"relative_humidity" => {
                    f.now.humidity = whole(v.number(), 0, 100).map_or(f.now.humidity, |h| h as u8);
                }
                b"wind_avg" => {
                    f.now.sky.wind =
                        tenths(v.number(), 0, 5000).map_or(f.now.sky.wind, |w| w as u16)
                }
                b"wind_direction" => {
                    f.now.wind_from =
                        whole(v.number(), 0, 360).map_or(f.now.wind_from, |d| d as u16);
                }
                b"uv" => f.now.uv = whole(v.number(), 0, 20).map_or(f.now.uv, |u| u as u8),
                b"icon" => {
                    if let Some((wmo, night)) = v.string().and_then(sky_of) {
                        f.now.sky.wmo = wmo;
                        f.now.sky.night = night;
                    }
                }
                _ => {}
            });
        }
        b"forecast" => {
            let _ = v.object(|k, v| match k {
                b"daily" => {
                    let _ = v.array(|n, v| {
                        if n >= DAYS {
                            return;
                        }
                        let mut d: Day = f.days[n];
                        let _ = v.object(|k, v| match k {
                            b"air_temp_high" => {
                                d.high = tenths(v.number(), -900, 700).unwrap_or(d.high)
                            }
                            b"air_temp_low" => {
                                d.low = tenths(v.number(), -900, 700).unwrap_or(d.low)
                            }
                            b"precip_probability" => {
                                d.precip = whole(v.number(), 0, 100).map_or(d.precip, |p| p as u8);
                            }
                            b"day_start_local" => {
                                if let Some(e) = whole(v.number(), 0, 1 << 40) {
                                    let (day, _) = local(e, offset);
                                    d.weekday = (day + 3).rem_euclid(7) as u8;
                                }
                            }
                            b"sunrise" if n == 0 => {
                                if let Some(e) = whole(v.number(), 0, 1 << 40) {
                                    f.sun.0 = local(e, offset).1;
                                }
                            }
                            b"sunset" if n == 0 => {
                                if let Some(e) = whole(v.number(), 0, 1 << 40) {
                                    f.sun.1 = local(e, offset).1;
                                }
                            }
                            b"icon" => {
                                if let Some((wmo, _)) = v.string().and_then(sky_of) {
                                    d.sky = Sky {
                                        wmo,
                                        night: false,
                                        ..d.sky
                                    };
                                }
                            }
                            _ => {}
                        });
                        f.days[n] = d;
                    });
                }
                b"hourly" => {
                    let _ = v.array(|n, v| {
                        if n >= HOURS {
                            return;
                        }
                        let mut h: Hour = f.hours[n];
                        let _ = v.object(|k, v| match k {
                            b"air_temperature" => {
                                h.temp = tenths(v.number(), -900, 700).unwrap_or(h.temp)
                            }
                            b"precip_probability" => {
                                h.precip = whole(v.number(), 0, 100).map_or(h.precip, |p| p as u8);
                            }
                            b"local_hour" => {
                                h.hour = whole(v.number(), 0, 23).map_or(h.hour, |x| x as u8)
                            }
                            b"wind_avg" => {
                                h.sky.wind =
                                    tenths(v.number(), 0, 5000).map_or(h.sky.wind, |w| w as u16)
                            }
                            b"icon" => {
                                if let Some((wmo, night)) = v.string().and_then(sky_of) {
                                    h.sky.wmo = wmo;
                                    h.sky.night = night;
                                }
                            }
                            _ => {}
                        });
                        f.hours[n] = h;
                    });
                }
                _ => {}
            });
        }
        _ => {}
    })?;
    have_now.then_some(f)
}

/// The station's name in Tempest's `better_forecast` (`location_name`).
#[must_use]
pub fn place_name(json: &[u8]) -> Name {
    let mut n = Name::EMPTY;
    let mut root = Json::new(json);
    let _ = root.object(|k, v| {
        if k == b"location_name"
            && let Some(s) = v.string()
        {
            n = Name::from_text(s);
        }
    });
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trimmed `better_forecast` in Tempest's shape (metric).
    const JSON: &[u8] = br#"{
      "latitude": 37.7, "timezone": "America/Chicago", "timezone_offset_minutes": -300,
      "location_name": "Perryville Loop Station",
      "current_conditions": {"time": 1761900000, "conditions": "Clear", "icon": "clear-night",
        "air_temperature": 17.4, "feels_like": 17.1, "relative_humidity": 93,
        "wind_avg": 5.0, "wind_direction": 360, "uv": 0, "station_pressure": 813.2},
      "forecast": {
        "daily": [
          {"day_start_local": 1761886800, "day_num": 31, "month_num": 10, "conditions": "Rain Likely",
           "icon": "rainy", "sunrise": 1761911400, "sunset": 1761950400,
           "air_temp_high": 21.0, "air_temp_low": 10.5, "precip_probability": 70},
          {"day_start_local": 1761973200, "icon": "partly-cloudy-day",
           "air_temp_high": 18.2, "air_temp_low": 7.9, "precip_probability": 10}
        ],
        "hourly": [
          {"time": 1761901200, "icon": "possibly-rainy-night", "air_temperature": 17.0,
           "precip_probability": 30, "local_hour": 4, "wind_avg": 6.0},
          {"time": 1761904800, "icon": "cloudy", "air_temperature": -2.5,
           "precip_probability": 0, "local_hour": 5, "wind_avg": 2.0}
        ]
      }
    }"#;

    #[test]
    fn tempest_fills_the_forecast_it_carries() {
        let base = crate::sample();
        let f =
            from_tempest(JSON, &base, Name::new("Perryville Loop Station")).expect("a forecast");
        assert_eq!(
            (f.place.as_str(), f.source),
            ("Perryville Loop Station", "Tempest")
        );
        assert_eq!(
            (f.now.temp, f.now.feels, f.now.humidity, f.now.uv),
            (174, 171, 93, 0)
        );
        assert_eq!(
            (
                f.now.sky.wmo,
                f.now.sky.night,
                f.now.sky.wind,
                f.now.wind_from
            ),
            (0, true, 50, 360)
        );
        // 2026-10-31 local (UTC-5): a Friday; sunrise 06:50, sunset 17:40.
        assert_eq!(
            (f.days[0].high, f.days[0].low, f.days[0].precip),
            (210, 105, 70)
        );
        assert_eq!((f.days[0].weekday, f.days[0].sky.wmo), (4, 63));
        assert_eq!(f.days[1].weekday, 5);
        assert_eq!(f.sun, (6 * 60 + 50, 17 * 60 + 40));
        assert_eq!(
            (f.hours[0].hour, f.hours[0].temp, f.hours[0].precip),
            (4, 170, 30)
        );
        assert_eq!((f.hours[1].temp, f.hours[1].sky.wmo), (-25, 3));
        // What it doesn't carry stays the base's.
        assert_eq!(f.days[2], base.days[2]);
        assert_eq!(f.hours[2], base.hours[2]);
        assert_eq!(place_name(JSON).as_str(), "Perryville Loop Station");
    }

    #[test]
    fn hostile_or_broken_input_is_refused_or_ignored_never_trusted() {
        let base = crate::sample();
        // Not a forecast, truncated, garbage, deep nesting, wild numbers.
        for bad in [
            &b"{}"[..],
            b"{\"current_conditions\": {\"air_temperature\": 17.4",
            b"\xff\x00garbage",
            b"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[",
            b"{\"current_conditions\": {\"air_temperature\": 1e9}}",
            b"",
        ] {
            assert!(
                from_tempest(bad, &base, Name::EMPTY).is_none(),
                "{:?}",
                core::str::from_utf8(bad)
            );
        }
        // Out of range: ignored, the base's kept.
        let f = from_tempest(
            b"{\"current_conditions\": {\"air_temperature\": 10.0, \"relative_humidity\": 400, \"uv\": -3}}",
            &base,
            Name::EMPTY,
        )
        .unwrap();
        assert_eq!(
            (f.now.temp, f.now.humidity, f.now.uv),
            (100, base.now.humidity, base.now.uv)
        );
    }
}
