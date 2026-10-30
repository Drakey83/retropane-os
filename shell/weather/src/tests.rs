use super::*;

#[test]
fn every_wmo_code_of_the_pack_table_has_its_icon_and_the_rest_are_unknown() {
    let s = |wmo| Sky {
        wmo,
        night: false,
        cover: 50,
        wind: 0,
    };
    for (codes, icon) in [
        (&[0u8, 1][..], Icon::ClearDay),
        (&[2], Icon::PartlyDay),
        (&[3], Icon::Cloudy),
        (&[45, 48], Icon::Fog),
        (&[51, 53, 55], Icon::Drizzle),
        (&[61], Icon::LightRain),
        (&[63, 65], Icon::HeavyRain),
        (&[56, 57, 66, 67], Icon::FreezingRain),
        (&[80, 81, 82], Icon::ShowersDay),
        (&[71, 77], Icon::LightSnow),
        (&[73, 75], Icon::HeavySnow),
        (&[85, 86], Icon::SnowShowersDay),
        (&[95], Icon::Thunder),
        (&[96, 99], Icon::ThunderHail),
    ] {
        for &c in codes {
            assert_eq!(Icon::of(s(c)), icon, "WMO {c}");
        }
    }
    let known = [
        0u8, 1, 2, 3, 45, 48, 51, 53, 55, 56, 57, 61, 63, 65, 66, 67, 71, 73, 75, 77, 80, 81, 82,
        85, 86, 95, 96, 99,
    ];
    for c in 0..=255u8 {
        if !known.contains(&c) {
            assert_eq!(Icon::of(s(c)), Icon::Unknown, "WMO {c}");
        }
    }
}

#[test]
fn night_overcast_and_wind_pick_their_variants_both_sides_of_each_edge() {
    let base = Sky {
        wmo: 0,
        night: true,
        cover: 0,
        wind: 0,
    };
    assert_eq!(Icon::of(base), Icon::ClearNight);
    assert_eq!(Icon::of(Sky { wmo: 80, ..base }), Icon::ShowersNight);
    assert_eq!(
        Icon::of(Sky {
            wmo: 3,
            cover: 89,
            ..base
        }),
        Icon::Cloudy
    );
    assert_eq!(
        Icon::of(Sky {
            wmo: 3,
            cover: 90,
            ..base
        }),
        Icon::Overcast
    );
    assert_eq!(Icon::of(Sky { wind: 399, ..base }), Icon::ClearNight);
    assert_eq!(Icon::of(Sky { wind: 400, ..base }), Icon::Windy);
    // Rain in a gale is still rain.
    assert_eq!(
        Icon::of(Sky {
            wmo: 61,
            wind: 900,
            ..base
        }),
        Icon::LightRain
    );
}

#[test]
fn pack_ids_go_both_ways_for_all_22_and_nothing_else_is_an_icon() {
    for id in 1..=22u8 {
        assert_eq!(Icon::from_id(id).id(), id);
    }
    assert_eq!(Icon::from_id(0), Icon::Unknown);
    assert_eq!(Icon::from_id(23), Icon::Unknown);
}

#[test]
fn temperatures_round_half_away_from_zero_in_both_units() {
    let us = Region::US;
    let m = Region::METRIC;
    for (c10, f, c) in [
        (0i16, 32, 0),
        (-400, -40, -40),
        (1000, 212, 100),
        (222, 72, 22),
        (-5, 31, -1),
        (5, 33, 1),
        (-4, 31, 0),
        (4, 33, 0),
        (i16::MIN, -5866, -3277),
        (i16::MAX, 5930, 3277),
    ] {
        assert_eq!(us.degrees(c10), f, "{c10} in F");
        assert_eq!(m.degrees(c10), c, "{c10} in C");
    }
}

#[test]
fn whole_fahrenheit_survives_the_trip_through_the_stored_unit() {
    // A source in °F stored as tenths of °C must show the same number.
    for f in -60..=130 {
        let a = (f - 32) * 50;
        let c10 = (if a >= 0 { (a + 4) / 9 } else { -((-a + 4) / 9) }) as i16;
        assert_eq!(Region::US.degrees(c10), f);
    }
}

#[test]
fn wind_and_compass_in_each_unit() {
    assert_eq!(Region::US.wind(129, 315).as_bytes(), b"8 mph NW");
    assert_eq!(Region::METRIC.wind(129, 0).as_bytes(), b"13 km/h N");
    let ms = Region {
        wind: Wind::Ms,
        ..Region::METRIC
    };
    assert_eq!(ms.wind(360, 180).as_bytes(), b"10 m/s S");
    assert_eq!(compass(11), b"N");
    assert_eq!(compass(12), b"NNE");
    assert_eq!(compass(349), b"N");
    assert_eq!(compass(348), b"NNW");
    assert_eq!(compass(720 + 90), b"E");
}

#[test]
fn times_on_both_clocks_including_midnight_and_noon() {
    let us = Region::US;
    assert_eq!(us.time(0, 0, false).as_bytes(), b"12 AM");
    assert_eq!(us.time(12, 0, false).as_bytes(), b"12 PM");
    assert_eq!(us.time(23, 5, true).as_bytes(), b"11:05 PM");
    assert_eq!(us.time(9, 41, true).as_bytes(), b"9:41 AM");
    assert_eq!(Region::METRIC.time(7, 3, false).as_bytes(), b"07:03");
    assert_eq!(Region::METRIC.time(24, 60, true).as_bytes(), b"00:00");
}

#[test]
fn a_line_drops_what_does_not_fit_and_numbers_keep_their_sign() {
    let l = Line::new().push(&[b'x'; 50]);
    assert_eq!(l.as_bytes().len(), 40);
    assert_eq!(Line::new().num(-7).as_bytes(), b"-7");
    assert_eq!(Line::new().num(i32::MIN).as_bytes(), b"-2147483648");
    assert_eq!(Line::new().num(0).as_bytes(), b"0");
}

#[test]
fn the_sample_keeps_the_today_screens_earlier_reading() {
    let s = sample();
    let us = Region::US;
    assert_eq!(us.degrees(s.now.temp), 72);
    assert_eq!(
        (us.degrees(s.days[0].high), us.degrees(s.days[0].low)),
        (75, 52)
    );
    let earlier = [(73, 52), (63, 48), (61, 48), (59, 46), (70, 50)];
    for (d, (hi, lo)) in s.days[1..6].iter().zip(earlier) {
        assert_eq!((us.degrees(d.high), us.degrees(d.low)), (hi, lo));
    }
    let icons: Vec<u8> = s.days[1..6].iter().map(|d| Icon::of(d.sky).id()).collect();
    assert_eq!(icons, [9, 5, 12, 1, 3]);
    assert_eq!(Icon::of(s.now.sky), Icon::PartlyDay);
}

#[test]
fn the_sample_is_whole_and_sensible() {
    let s = sample();
    for (i, h) in s.hours.iter().enumerate() {
        assert_eq!(usize::from(h.hour), (12 + i) % 24);
        assert!(h.precip <= 100);
    }
    for d in &s.days {
        assert!(d.low <= d.high && d.weekday < 7);
    }
    let (lo, hi) = s.range();
    assert_eq!((Region::US.degrees(lo), Region::US.degrees(hi)), (45, 75));
    assert_eq!(
        s.summary(&Region::US).as_bytes(),
        b"Partly cloudy, rain at 3 PM"
    );
}

#[test]
fn the_icons_the_sample_uses_are_the_ones_baked_into_the_test_image() {
    // tools/shell-assets/icons.py bakes these; it reads this list from here.
    let s = sample();
    let mut used: Vec<u8> = core::iter::once(Icon::of(s.now.sky).id())
        .chain(s.hours.iter().map(|h| Icon::of(h.sky).id()))
        .chain(s.days.iter().map(|d| Icon::of(d.sky).id()))
        .collect();
    used.sort_unstable();
    used.dedup();
    assert_eq!(used, crate::SAMPLE_ICONS);
    let mut cards: Vec<u8> = core::iter::once(Icon::of(s.now.sky).id())
        .chain(s.days[1..6].iter().map(|d| Icon::of(d.sky).id()))
        .collect();
    cards.sort_unstable();
    cards.dedup();
    assert_eq!(cards, crate::SAMPLE_CARD_ICONS);
}

#[test]
fn a_region_sets_its_units_and_clock_and_the_system_switch_overrides() {
    use crate::settings::{REGIONS, Settings};
    let mut s = Settings::DEFAULT;
    assert_eq!(s.units(), Region::US, "US customary by default");
    let names: Vec<&str> = (0..REGIONS.len())
        .map(|_| {
            s.next_region();
            s.region_info().name
        })
        .collect();
    assert_eq!(names.last(), Some(&"United States"), "it cycles round");
    s.region = 2; // the United Kingdom: Celsius with mph, 24-hour
    s.metric = true;
    assert_eq!(
        s.units(),
        Region {
            temp: Temp::C,
            wind: Wind::Mph,
            clock12: false
        }
    );
    s.metric = false;
    assert_eq!(s.units().temp, Temp::F);
    assert!(!s.units().clock12, "the clock stays the region's");
}

#[test]
fn a_secret_is_never_shown_or_debugged_in_full() {
    use crate::settings::{KEY_MAX, Secret};
    let key = b"0a1b2c3d-4e5f-6789-abcd-ef0123456789";
    let mut s = Secret::new(key).expect("printable");
    assert_eq!(s.masked().as_bytes(), b"**** 6789");
    let dbg = format!("{s:?}");
    assert!(!dbg.contains("0a1b") && dbg.contains("36"), "{dbg}");
    assert_eq!(s.expose(), key);
    s.clear();
    assert!(!s.is_set());
    assert_eq!(s.masked().as_bytes(), b"Not set");
    assert!(Secret::new(b"").is_none());
    assert!(Secret::new(b"has space").is_none());
    assert!(Secret::new(&[b'a'; KEY_MAX + 1]).is_none());
    assert!(Secret::new(&[b'a'; KEY_MAX]).is_some());
}
