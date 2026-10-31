//! The sample forecast the screens show until there is a network (the device
//! has none yet; live data comes later through USB to the PC, DESIGN.md 10b).
//! Written in °F, as the owner reads it, and stored like any source's data.
//! The first six days keep the Today screen's earlier fixed reading, so its
//! picture didn't change when it began reading the model.

use crate::{Day, Forecast, Hour, Now, Sky};

/// °F to tenths of °C, rounded half away from zero.
const fn c10(f: i32) -> i16 {
    let a = (f - 32) * 50;
    (if a >= 0 { (a + 4) / 9 } else { -((-a + 4) / 9) }) as i16
}

const fn sky(wmo: u8, night: bool, cover: u8) -> Sky {
    Sky {
        wmo,
        night,
        cover,
        wind: 130,
    }
}

const fn hour(hour: u8, f: i32, wmo: u8, cover: u8, precip: u8) -> Hour {
    // Sun up 7:52 AM to 6:02 PM (Seattle, end of October).
    let night = hour < 8 || hour >= 18;
    let s = sky(wmo, night, cover);
    Hour {
        hour,
        temp: c10(f),
        sky: s,
        precip,
        feels: c10(f - 2),
        humidity: 60 + precip / 4,
        dew: c10(f - 12),
        gust: s.wind * 16 / 10,
        wind_from: 315,
        rain: precip as u16 / 4,
        uv: if night { 0 } else { 2 },
        vis: 160,
    }
}

const fn day(weekday: u8, high: i32, low: i32, wmo: u8, cover: u8, precip: u8) -> Day {
    Day {
        weekday,
        high: c10(high),
        low: c10(low),
        sky: sky(wmo, false, cover),
        precip,
    }
}

/// The sample: a place in the Pacific Northwest at noon on a Thursday,
/// partly cloudy, a shower in the afternoon, cooler days ahead.
#[must_use]
pub const fn sample() -> Forecast {
    Forecast {
        place: crate::Name::new("Seattle"),
        source: "Sample data",
        sun: (7 * 60 + 52, 18 * 60 + 2),
        now: Now {
            temp: c10(72),
            feels: c10(70),
            sky: sky(2, false, 45),
            wind_from: 315,
            humidity: 64,
            uv: 3,
        },
        hours: [
            hour(12, 72, 2, 45, 5),
            hour(13, 74, 2, 50, 10),
            hour(14, 75, 3, 70, 20),
            hour(15, 73, 80, 80, 60),
            hour(16, 70, 61, 85, 70),
            hour(17, 67, 3, 80, 40),
            hour(18, 64, 2, 60, 20),
            hour(19, 62, 2, 40, 10),
            hour(20, 60, 0, 20, 5),
            hour(21, 59, 0, 10, 0),
            hour(22, 57, 0, 10, 0),
            hour(23, 56, 0, 15, 0),
            hour(0, 55, 2, 30, 0),
            hour(1, 54, 2, 35, 5),
            hour(2, 54, 3, 70, 5),
            hour(3, 53, 3, 75, 10),
            hour(4, 53, 3, 80, 10),
            hour(5, 52, 3, 75, 10),
            hour(6, 52, 2, 50, 5),
            hour(7, 53, 2, 40, 5),
            hour(8, 56, 2, 35, 5),
            hour(9, 60, 1, 20, 0),
            hour(10, 64, 1, 15, 0),
            hour(11, 68, 2, 30, 0),
        ],
        days: [
            day(3, 75, 52, 2, 45, 60),
            day(4, 73, 52, 61, 85, 70),
            day(5, 63, 48, 3, 70, 30),
            day(6, 61, 48, 80, 80, 60),
            day(0, 59, 46, 0, 10, 0),
            day(1, 70, 50, 2, 40, 10),
            day(2, 64, 49, 3, 75, 20),
            day(3, 58, 45, 61, 90, 80),
            day(4, 62, 47, 2, 50, 20),
            day(5, 66, 50, 0, 15, 0),
        ],
    }
}
