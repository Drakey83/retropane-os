//! Your station's charts, to try (N3 and N4: docs/design/station-everywhere.md,
//! the owner's ask of 2026-11-01 to see them on the X51v). A tap on a
//! reading (the Now page's big temperature and cards with a station live,
//! the Station screen's tiles) slides a sheet up over the lower part of the
//! screen, with one line: the station's last hours (solid, its readings)
//! meeting the forecast's next hours (dashed) at now. A finger along the
//! line, or the D-pad's left and right, moves a marker and reads that
//! half-hour large; up and down change the reading. Swipe down, a tap above
//! the sheet, or Close shut it.
//!
//! The line is painted once into Hourly's graph picture (its one store slot:
//! Hourly is never shown under the sheet, and is painted again when it
//! closes); only the marker moves, a few fills.
//!
//! Iterate (rule 0): Tempest's app charts the station's past alone, full
//! screen; phone weather apps chart the forecast alone. Joined at now, one
//! line shows whether the forecast has been right today, and the sheet keeps
//! what was tapped in view.

use rp_draw::{Argb, Cmd, Frame, Rect};
use rp_weather::Line;
use rp_weather::tempest::HIST;

use super::weather_app::{CURVE, FROST, isqrt, over};
use super::{
    DEG_BIG, DIM, FOCUS, GLASS_EDGE, H, MENU_LEFT, MENU_RIGHT, PRESSED_UNDER, Screen, Shell, TEXT,
    Target, W,
};

/// Off: no reading opens a chart (the owner may revert, 2026-11-01).
pub(crate) const CHARTS: bool = true;

/// A reading a chart shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reading {
    /// Air temperature.
    Temp,
    /// The wind's average.
    Wind,
    /// Relative humidity.
    Humidity,
    /// Pressure.
    Pressure,
}

impl Reading {
    /// Every reading, in [`Target::Reading`]'s order.
    pub const ALL: [Self; 4] = [Self::Temp, Self::Wind, Self::Humidity, Self::Pressure];

    const fn name(self) -> &'static [u8] {
        match self {
            Self::Temp => b"TEMPERATURE",
            Self::Wind => b"WIND",
            Self::Humidity => b"HUMIDITY",
            Self::Pressure => b"PRESSURE",
        }
    }

    /// The least the chart's height spans, in the reading's unit (a calm
    /// day's line isn't stretched into a storm).
    const fn least(self) -> i32 {
        match self {
            Self::Temp => 30,
            Self::Wind => 80,
            Self::Humidity => 10,
            Self::Pressure => 20,
        }
    }
}

/// The chart open: its reading, its span (0: twelve hours each side of now,
/// 1: a day) and the marker (half-hours from now).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Chart {
    pub(crate) reading: Reading,
    pub(crate) span: u8,
    pub(crate) at: i8,
}

impl Chart {
    /// Half-hours each side of now (a day back is the history's whole).
    pub(crate) const fn reach(self) -> i32 {
        if self.span == 0 { 24 } else { HIST as i32 - 1 }
    }
}

/// The chart's picture: painted into Hourly's graph slot (its size or
/// less), small enough for the 2D engine's 256 KiB picture cache (200 KB).
pub const CHART_PIC: (u16, u16) = (super::weather_app::GRAPH_PIC.0, 112);
const GRAPH_PIC: (u16, u16) = CHART_PIC;

/// The sheet: from under the big temperature and the sky's words (the
/// Station screen's first row of tiles starts here too) to the softkeys.
pub(crate) const SHEET: Rect = Rect::new(0, 220, W, H - 40 - 220);
const TITLE_Y: u16 = SHEET.y + 40;
const WHEN_Y: u16 = SHEET.y + 96;
const KIND_Y: u16 = SHEET.y + 124;
const VALUE_Y: u16 = SHEET.y + 140;
/// The line's picture (Hourly's graph slot).
const CHART_X: u16 = 17;
const CHART_Y: u16 = SHEET.y + 156;
/// Where a finger scrubs: the picture and a margin round it.
const SCRUB: Rect = Rect::new(0, CHART_Y - 20, W, GRAPH_PIC.1 + 40);
const AXIS_Y: u16 = CHART_Y + GRAPH_PIC.1 + 22;
const LEGEND_Y: u16 = AXIS_Y + 30;
/// The line's ends inside the picture (room for the marker's dot), and its
/// top and bottom rows.
const PAD: i64 = 10;
const TOP: i64 = 14;
const BOTTOM: i64 = GRAPH_PIC.1 as i64 - 14;
/// A swipe down this far from the sheet's top closes it.
const PULL_CLOSE: u16 = 40;

const LIVE: Argb = super::station::LIVE;
const NOW_LINE: Argb = rp_draw::look::theme(0x40FF_FFFF);

/// A span chip ("12 HOURS", "2 DAYS"), top right of the sheet.
pub(crate) const fn span_chip(i: u8) -> Rect {
    Rect::new(W - 16 - 2 * 108 + 8 + i as u16 * 108, SHEET.y + 16, 100, 34)
}

/// The picture's column for `o` half-hours from now.
fn col_of(c: Chart, o: i32) -> i64 {
    let w = i64::from(GRAPH_PIC.0) - 2 * PAD;
    PAD + i64::from(o + c.reach()) * w / i64::from(2 * c.reach())
}

/// A reading in its own unit as words: tenths of °C, tenths of km/h, %,
/// tenths of a mb.
fn value_line(s: &Shell, r: Reading, v: i32) -> Line {
    let rg = &s.region;
    match r {
        Reading::Temp => Line::new()
            .num(rg.degrees(v.clamp(-900, 700) as i16))
            .push(&[DEG_BIG]),
        Reading::Wind => {
            let unit: &[u8] = match rg.wind {
                rp_weather::Wind::Mph => b" mph",
                rp_weather::Wind::Kmh => b" km/h",
                rp_weather::Wind::Ms => b" m/s",
            };
            Line::new()
                .num(rg.wind_speed(v.clamp(0, 6000) as u16))
                .push(unit)
        }
        Reading::Humidity => Line::new().num(v).push(b"%"),
        Reading::Pressure => super::station::pressure_line(
            v.clamp(0, i32::from(u16::MAX)) as u16,
            rg.temp == rp_weather::Temp::F,
        ),
    }
}

impl Shell {
    /// The station's last day as Tempest's observations answer it: for
    /// the PC's previews, which have no relay.
    pub fn set_history_from_tempest(&mut self, json: &[u8]) {
        self.history = rp_weather::tempest::history(json);
        self.history_at = self.now;
        self.dirty(Rect::new(0, 0, W, H));
    }

    /// A reading `o` half-hours from now, in its own unit: the station's up
    /// to now (its reading now, else its history's half-hour), the
    /// forecast's hours after.
    pub(crate) fn chart_value(&self, r: Reading, o: i32) -> Option<i32> {
        if o > 0 {
            // The forecast's first hour is the one under way.
            let i = usize::try_from((i32::from(self.time.minute) + o * 30) / 60).ok()?;
            let h = self.wx.hours.get(i)?;
            return match r {
                Reading::Temp => Some(i32::from(h.temp)),
                Reading::Wind => Some(i32::from(h.sky.wind)),
                Reading::Humidity => Some(i32::from(h.humidity)),
                Reading::Pressure => None,
            };
        }
        let live = self.smart_station();
        if o == 0
            && let Some(n) = live
        {
            let v = match r {
                Reading::Temp => n.temp.map(i32::from),
                Reading::Wind => n.wind.map(i32::from),
                Reading::Humidity => n.humidity.map(i32::from),
                // The history's is the station's own pressure, the reading
                // now the sea level's: the history's, lifted below.
                Reading::Pressure => None,
            };
            if v.is_some() {
                return v;
            }
        }
        let h = self.history.as_ref()?;
        let s = h.s.get(usize::try_from(HIST as i32 - 1 + o).ok()?)?;
        match r {
            Reading::Temp => s.temp.map(i32::from),
            Reading::Wind => s.wind.map(i32::from),
            Reading::Humidity => s.humidity.map(i32::from),
            Reading::Pressure => {
                // Tempest's history is at the station's height, its reading
                // now at sea level: the day's line lifted by the difference
                // now, so the chart and the card say the same.
                let lift = live
                    .and_then(|n| n.pressure)
                    .zip(h.s[HIST - 1].pressure)
                    .map_or(0, |(sea, here)| i32::from(sea) - i32::from(here));
                s.pressure.map(|p| i32::from(p) + lift)
            }
        }
    }

    /// The chart's lowest and highest value with a margin, `None` with no
    /// values at all.
    fn chart_range(&self, c: Chart) -> Option<(i32, i32)> {
        let (lo, hi) = (-c.reach()..=c.reach())
            .filter_map(|o| self.chart_value(c.reading, o))
            .fold((i32::MAX, i32::MIN), |(a, b), v| (a.min(v), b.max(v)));
        if lo > hi {
            return None;
        }
        let pad = ((hi - lo).max(c.reading.least()) - (hi - lo)) / 2 + (hi - lo) / 10 + 1;
        Some((lo - pad, hi + pad))
    }

    /// What the chart shows (its reading, span and every value): a new key
    /// is a new picture to paint.
    pub(crate) fn chart_key(&self, c: Chart) -> u32 {
        let seed = 0x5EED_C4A7u32 ^ (u32::from(c.span) << 8) ^ c.reading as u32;
        (-c.reach()..=c.reach()).fold(seed, |h, o| {
            let v = self
                .chart_value(c.reading, o)
                .map_or(0x7FFF_FFFF, |v| v as u32);
            (h ^ v).wrapping_mul(0x0100_0193)
        })
    }

    /// The row (x 256) of `v` in the picture.
    fn chart_row(lo: i32, hi: i32, v: i32) -> i64 {
        let span = i64::from((hi - lo).max(1));
        BOTTOM * 256 - (BOTTOM - TOP) * 256 * i64::from(v - lo) / span
    }

    /// The line painted per pixel into `px` (a see-through picture of
    /// [`GRAPH_PIC`]): the station's half-hours solid with the frost under
    /// them, the forecast's dashed, both anti-aliased by their distance from
    /// the line as Hourly's is, and a faint line at now.
    pub(crate) fn paint_chart(&self, px: &mut [u32], c: Chart) {
        let stride = usize::from(GRAPH_PIC.0);
        let rows = i64::from(GRAPH_PIC.1);
        let Some((lo, hi)) = self.chart_range(c) else {
            return;
        };
        let reach = c.reach();
        let w = i64::from(GRAPH_PIC.0) - 2 * PAD;
        let row_at = |o256: i64| -> Option<i64> {
            // Between the half-hours either side, straight.
            let o0 = o256.div_euclid(256) as i32;
            let s = o256.rem_euclid(256);
            let a = self.chart_value(c.reading, o0)?;
            let b = if s == 0 {
                a
            } else {
                self.chart_value(c.reading, o0 + 1)?
            };
            let (ya, yb) = (Self::chart_row(lo, hi, a), Self::chart_row(lo, hi, b));
            Some(ya + (yb - ya) * s / 256)
        };
        let now_col = col_of(c, 0);
        for x in 0..i64::from(GRAPH_PIC.0) {
            if x == now_col {
                for r in 0..rows {
                    if let Some(p) = px.get_mut(r as usize * stride + x as usize) {
                        *p = over(*p, NOW_LINE);
                    }
                }
            }
            if x < PAD || x > PAD + w {
                continue;
            }
            let o256 = (x - PAD) * i64::from(2 * reach) * 256 / w - i64::from(reach) * 256;
            let Some(cy) = row_at(o256) else { continue };
            let past = o256 <= 0;
            // The forecast's dashes: 8 px on, 6 off.
            if !past && (x - now_col) % 14 >= 8 {
                continue;
            }
            let step = (i64::from(2 * reach) * 256 / w).max(1);
            let dc = row_at(o256 + step).map_or(0, |n| (n - cy).abs());
            let half = 320 * i64::from(isqrt(65_536 + (dc * dc / 256) as i32)) / 256;
            let color = if past { CURVE } else { FOCUS };
            let top = ((cy - half - 256) / 256).max(0);
            let bottom = if past {
                rows
            } else {
                ((cy + half + 512) / 256).min(rows)
            };
            for ly in top..bottom {
                let Some(p) = px.get_mut(ly as usize * stride + x as usize) else {
                    continue;
                };
                let d = ly * 256 + 128 - cy;
                if past && d > 0 {
                    *p = over(*p, FROST);
                }
                let cover = (half + 128 - d.abs()).clamp(0, 256) as u32;
                if cover > 0 {
                    *p = over(*p, ((cover * 255 / 256) << 24) | (color & 0x00FF_FFFF));
                }
            }
        }
    }

    /// Whether reading `i` can be tapped now (and where: [`Self::reading_rect`]).
    pub(crate) fn reading_shown(&self, i: u8) -> bool {
        if !CHARTS || self.chart.is_some() || self.menu_open || self.place_menu {
            return false;
        }
        match self.screen {
            Screen::Weather => self.tab == 0 && self.smart_station().is_some(),
            // The Station screen's tiles: the big temperature, wind and
            // pressure (its humidity is in the temperature's tile).
            Screen::Station => self.station.is_some() && i != 2,
            _ => false,
        }
    }

    /// Where reading `i`'s tile is.
    pub(crate) fn reading_rect(&self, i: u8) -> Rect {
        if self.screen == Screen::Station {
            super::station::reading_rect(i)
        } else {
            self.now_reading_rect(i)
        }
    }

    /// Opens reading `r`'s chart, the marker at now; the station's day
    /// asked for when it is missing or ten minutes old.
    pub(crate) fn open_chart(&mut self, r: Reading) {
        if !CHARTS {
            return;
        }
        self.chart = Some(Chart {
            reading: r,
            span: 0,
            at: 0,
        });
        if self.history.is_none() || self.now.wrapping_sub(self.history_at) > 600_000 {
            self.want(super::Ask::History);
        }
        self.pressed = None;
        self.focus = None;
        self.gesture = super::Gesture::None;
        self.dirty(Rect::new(0, 0, W, H));
    }

    /// Closes the chart (the page under it drawn whole again).
    pub(crate) fn close_chart(&mut self) {
        if self.chart.take().is_some() {
            self.gesture = super::Gesture::None;
            self.dirty(Rect::new(0, 0, W, H));
        }
    }

    /// The chart's span: 0 twelve hours, 1 a day (the marker kept inside).
    pub(crate) fn set_chart_span(&mut self, span: u8) {
        if let Some(c) = self.chart.as_mut() {
            c.span = span.min(1);
            let r = c.reach();
            c.at = i32::from(c.at).clamp(-r, r) as i8;
            self.dirty(SHEET);
        }
    }

    /// The marker to `o` half-hours from now (kept on the chart).
    fn chart_to(&mut self, o: i32) {
        if let Some(c) = self.chart.as_mut() {
            let r = c.reach();
            let at = o.clamp(-r, r) as i8;
            if at != c.at {
                c.at = at;
                // The readout and the line (the marker's old and new place).
                let top = VALUE_Y - 88;
                self.dirty(Rect::new(0, top, W, CHART_Y + GRAPH_PIC.1 + 8 - top));
            }
        }
    }

    /// The marker to the half-hour under the pen at `x`.
    fn scrub_to(&mut self, x: u16) {
        let Some(c) = self.chart else { return };
        let w = i64::from(GRAPH_PIC.0) - 2 * PAD;
        let col = i64::from(x) - i64::from(CHART_X) - PAD;
        let n = i64::from(2 * c.reach());
        let o = (col * n + w / 2).div_euclid(w) - i64::from(c.reach());
        self.chart_to(o as i32);
    }

    /// The softkeys while the chart is open.
    pub(crate) fn chart_softkeys(&self) -> Option<(&'static str, &'static str)> {
        let c = self.chart?;
        Some(("Close", if c.span == 0 { "2 days" } else { "12 hours" }))
    }

    /// An input while the chart is open: `Some` if the chart took it.
    pub(crate) fn chart_input(&mut self, e: super::Input) -> Option<super::Action> {
        use super::{Action, Gesture, Input, Key};
        let c = self.chart?;
        match e {
            Input::PenDown { x, y } => {
                if self.menu_open || super::START.contains_point(x, y) {
                    return None;
                }
                if y < SHEET.y {
                    // A tap above the sheet closes it (and does nothing else).
                    self.close_chart();
                    return Some(Action::Nothing);
                }
                if SCRUB.contains_point(x, y) {
                    self.gesture = Gesture::Scrub;
                    self.scrub_to(x);
                    return Some(Action::Nothing);
                }
                if y < SHEET.y + 60 && !(0..2).any(|i| span_chip(i).contains_point(x, y)) {
                    self.gesture = Gesture::Sheet { y0: y };
                    return Some(Action::Nothing);
                }
                None
            }
            Input::PenMove { x, y } => match self.gesture {
                Gesture::Scrub => {
                    self.scrub_to(x);
                    Some(Action::Nothing)
                }
                Gesture::Sheet { y0 } => {
                    if y >= y0 + PULL_CLOSE {
                        self.close_chart();
                    }
                    Some(Action::Nothing)
                }
                _ => None,
            },
            Input::PenUp { .. } => match self.gesture {
                Gesture::Scrub | Gesture::Sheet { .. } => {
                    self.gesture = Gesture::None;
                    Some(Action::Nothing)
                }
                _ => None,
            },
            // The D-pad: left and right move the marker a half-hour (an
            // hour on two days), up and down change the reading.
            Input::Key(k @ (Key::Left | Key::Right)) => {
                let step = if c.span == 0 { 1 } else { 2 };
                let d = if k == Key::Left { -step } else { step };
                self.chart_to(i32::from(c.at) + d);
                Some(Action::Nothing)
            }
            Input::Key(k @ (Key::Up | Key::Down)) => {
                let i = Reading::ALL
                    .iter()
                    .position(|&r| r == c.reading)
                    .unwrap_or(0);
                let n = Reading::ALL.len();
                let i = if k == Key::Up {
                    (i + n - 1) % n
                } else {
                    (i + 1) % n
                };
                if let Some(c) = self.chart.as_mut() {
                    c.reading = Reading::ALL[i];
                }
                if self.history.is_none() {
                    self.want(super::Ask::History);
                }
                self.dirty(SHEET);
                Some(Action::Nothing)
            }
            _ => None,
        }
    }

    /// The sheet over what was tapped: the reading's name and spans, the
    /// marker's time and value large, the line with its marker, the times
    /// along it and what solid and dashed mean.
    pub(crate) fn chart_scene(&self, f: &mut Frame, c: Chart) {
        let (ui, label, big) = (self.fonts.ui, self.fonts.label, self.fonts.big);
        // The sheet over the lower part (what is above it stays as it was:
        // the 2D engine's fills set the layer, so a shade would erase it).
        f.push(Cmd::HGradient {
            r: SHEET,
            left: MENU_LEFT,
            right: MENU_RIGHT,
        });
        f.push(Cmd::Fill {
            r: Rect::new(0, SHEET.y, W, 1),
            color: GLASS_EDGE,
        });
        // The grip: it pulls down.
        f.push(Cmd::Fill {
            r: Rect::new(W / 2 - 22, SHEET.y + 7, 44, 4),
            color: DIM,
        });
        f.text(20, TITLE_Y, label, TEXT, c.reading.name());
        for i in 0..2u8 {
            let r = span_chip(i);
            if c.span == i {
                f.push(Cmd::Fill {
                    r,
                    color: PRESSED_UNDER,
                });
            } else {
                self.press_under(f, Target::Span(i));
            }
            f.push(Cmd::Outline {
                r,
                color: GLASS_EDGE,
            });
            let t: &[u8] = if i == 0 { b"12 HOURS" } else { b"2 DAYS" };
            let tx = r.x + r.w.saturating_sub(label.width(t)) / 2;
            f.text(tx, r.y + 23, label, if c.span == i { TEXT } else { DIM }, t);
            self.decorate(f, Target::Span(i));
        }

        // The marker's half-hour: its time, where the value is from, the
        // value large (its digits in the big numerals, the unit after).
        let at = i32::from(c.at);
        let v = self.chart_value(c.reading, at);
        let live = self.smart_station().is_some();
        let (kind, kind_color): (&[u8], Argb) = match (at.signum(), v) {
            (0, Some(_)) if live => (b"NOW, YOUR STATION", LIVE),
            (-1 | 0, Some(_)) => (b"YOUR STATION MEASURED", LIVE),
            (1, Some(_)) => (b"THE FORECAST", FOCUS),
            (-1 | 0, None) if self.wants(super::Ask::History) => {
                (b"Loading your station's day", DIM)
            }
            (-1 | 0, None) => (b"No reading from your station", DIM),
            (_, None) if c.reading == Reading::Pressure => (b"No forecast for pressure", DIM),
            _ => (b"Past the forecast's hours", DIM),
        };
        let vx = 20;
        let vy = VALUE_Y;
        let mut right = vx;
        if let Some(v) = v {
            let text = value_line(self, c.reading, v);
            let b = text.as_bytes();
            let k = b
                .iter()
                .position(|&ch| !(ch.is_ascii_digit() || ch == DEG_BIG))
                .unwrap_or(b.len());
            f.text(vx, vy, big, TEXT, &b[..k]);
            right = vx + big.width(&b[..k]);
            if k < b.len() {
                f.text(right + 4, vy, ui, DIM, &b[k..]);
                right += 4 + ui.width(&b[k..]);
            }
        } else {
            f.text(vx, vy, big, DIM, b"--");
        }
        let tx = (right + 24).max(220);
        let when = self.chart_when(at);
        f.text(tx, WHEN_Y, ui, TEXT, when.as_bytes());
        f.text(tx, KIND_Y, label, kind_color, kind);

        // The line, then the marker over it: a thin rule and a dot.
        if let Some(pic) = self.graph {
            f.push(Cmd::Pic {
                x: CHART_X,
                y: CHART_Y as i16,
                pic,
            });
        }
        let mx = CHART_X + col_of(c, at) as u16;
        f.push(Cmd::Fill {
            r: Rect::new(mx - 1, CHART_Y, 2, GRAPH_PIC.1),
            color: TEXT,
        });
        if let (Some(v), Some((lo, hi))) = (v, self.chart_range(c)) {
            let my = CHART_Y + (Self::chart_row(lo, hi, v) / 256) as u16;
            f.push(Cmd::Fill {
                r: Rect::new(mx - 6, my - 6, 12, 12),
                color: if at > 0 { FOCUS } else { CURVE },
            });
        }

        // The times along it: the ends and now.
        let (left, end): (&[u8], &[u8]) = if c.span == 0 {
            (b"12 H AGO", b"IN 12 H")
        } else {
            (b"A DAY AGO", b"IN A DAY")
        };
        f.text(CHART_X + PAD as u16, AXIS_Y, label, DIM, left);
        let nx = CHART_X + col_of(c, 0) as u16;
        f.text(nx - label.width(b"NOW") / 2, AXIS_Y, label, DIM, b"NOW");
        let ex = CHART_X + GRAPH_PIC.0 - PAD as u16 - label.width(end);
        f.text(ex, AXIS_Y, label, DIM, end);

        // Solid: your station; dashed: the forecast.
        f.push(Cmd::Fill {
            r: Rect::new(20, LEGEND_Y - 7, 28, 3),
            color: CURVE,
        });
        f.text(56, LEGEND_Y, label, DIM, b"YOUR STATION");
        let fx = 56 + label.width(b"YOUR STATION") + 32;
        for d in 0..2u16 {
            f.push(Cmd::Fill {
                r: Rect::new(fx + d * 14, LEGEND_Y - 7, 8, 3),
                color: FOCUS,
            });
        }
        f.text(fx + 36, LEGEND_Y, label, DIM, b"FORECAST");
    }

    /// The time `o` half-hours from now, as the chart names it: "NOW",
    /// else the day and the half-hour ("YESTERDAY 9:30 PM").
    fn chart_when(&self, o: i32) -> Line {
        if o == 0 {
            return Line::new().push(b"Now");
        }
        let m = i32::from(self.time.hour) * 60 + i32::from(self.time.minute);
        let t = (m / 30 + o) * 30;
        let day: &[u8] = match t.div_euclid(1440) {
            -2 => b"2 days ago, ",
            -1 => b"Yesterday ",
            0 => b"Today ",
            1 => b"Tomorrow ",
            _ => b"",
        };
        let mm = t.rem_euclid(1440);
        Line::new().push(day).push(
            self.region
                .time((mm / 60) as u8, (mm % 60) as u8, true)
                .as_bytes(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_marker_reaches_both_ends_and_now_sits_in_the_middle() {
        for span in 0..2 {
            let c = Chart {
                reading: Reading::Temp,
                span,
                at: 0,
            };
            let w = i64::from(GRAPH_PIC.0);
            assert_eq!(col_of(c, -c.reach()), PAD);
            assert_eq!(col_of(c, c.reach()), w - PAD);
            assert!((col_of(c, 0) - w / 2).abs() <= 1);
        }
    }
}
