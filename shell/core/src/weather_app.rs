//! The Weather app (docs/design/weather-flows.md, the "mix" frame): the
//! title bar, a header strip with the place, the page, then the tabs and the
//! softkeys at the bottom where the thumb is. Three pages: Now, Hourly and
//! 10 Day, all drawn from the shell's one [`rp_weather::Forecast`], the same
//! one the Today screen's hero and forecast row read.
//!
//! Movement (the owner, 2026-10-30: "flashy ... animation where we can get
//! away with it"): a page arrives with its rows sliding in one after another
//! from the right, eased out, and its bars growing; the big temperature
//! counts up. Every frame is a redraw of the page's rectangle by the 2D
//! engine, which the Today screen's floors show it does in well under a
//! frame, so the movement costs no new hardware path.
//!
//! Iterate, don't imitate (rule 0): WM5 had no weather app, and SPB's and
//! HTC's sat as one fixed page. Ours is the WM frame (tabs and softkeys
//! where WM put them) around modern content: a temperature bar per hour,
//! each day's range on the week's scale, and swipes as well as the D-pad.

use rp_draw::{Argb, Cmd, Frame, Rect};
use rp_weather::{DAYS, HOURS, Icon, Line};

use super::{
    BAR_LEFT, BAR_RIGHT, DEG_BIG, DIM, FOCUS, GLASS_LEFT, GLASS_RIGHT, H, Shell, TEXT, Target, W,
};

/// The header strip: the place and the source.
pub(crate) const HEAD: Rect = Rect::new(0, 36, W, 58);
/// The page between the header and the tabs.
pub(crate) const PAGE: Rect = Rect::new(0, 94, W, 462);
/// The tab strip above the softkeys.
pub(crate) const TABS: Rect = Rect::new(0, H - 84, W, 44);
/// The pages.
pub(crate) const TAB_NAMES: [&[u8]; 3] = [b"NOW", b"HOURLY", b"10 DAY"];

/// Rows on the Hourly and 10 Day pages.
const ROWS: usize = 10;
const ROW_Y: u16 = 102;
const ROW_STEP: u16 = 45;
const ROW_H: u16 = 42;
/// The furthest the Hourly page scrolls.
pub(crate) const MAX_SCROLL: u8 = (HOURS - ROWS) as u8;

/// One item's entrance: how long it moves, how far, and the gap before the
/// next item starts.
const ITEM_MS: u32 = 260;
const STAGGER_MS: u32 = 28;
const SLIDE: u32 = 56;
/// The whole entrance (the last row starts after nine gaps).
pub(crate) const ANIM_MS: u32 = ITEM_MS + STAGGER_MS * ROWS as u32;

const RAIN: Argb = rp_draw::look::theme(0xFF7F_C8FF);
const TRACK: Argb = rp_draw::look::theme(0x30FF_FFFF);
/// The selected tab: a lighter piece of the bar itself (a translucent white
/// fill over the bar would be blended against the wallpaper by the 2D
/// engine, not against the bar, and wash the label out).
const SELECTED_LEFT: Argb = rp_draw::look::theme(0xD830_3840);
const SELECTED_RIGHT: Argb = rp_draw::look::theme(0xC84A_5460);
/// The temperature scale's ends: cold to warm.
const COLD: Argb = 0xFF4F_A8FF;
const WARM: Argb = 0xFFFF_A040;

/// Where tab `i` is.
pub(crate) const fn tab_rect(i: u8) -> Rect {
    Rect::new(i as u16 * 160, TABS.y, 160, TABS.h)
}

/// A row on the Hourly and 10 Day pages.
const fn row(k: usize) -> Rect {
    Rect::new(16, ROW_Y + k as u16 * ROW_STEP, 448, ROW_H)
}

/// `a` to `b` by `t` of 1024, channel by channel.
fn mix(a: Argb, b: Argb, t: u32) -> Argb {
    let t = t.min(1024);
    let ch = |s: u32| {
        let (x, y) = ((a >> s) & 0xFF, (b >> s) & 0xFF);
        ((x * (1024 - t) + y * t) / 1024) << s
    };
    rp_draw::look::theme(ch(24) | ch(16) | ch(8) | ch(0))
}

/// Where `v` falls between `lo` and `hi`, in 1024ths.
fn frac(v: i16, lo: i16, hi: i16) -> u32 {
    let span = (i32::from(hi) - i32::from(lo)).max(1);
    ((i32::from(v) - i32::from(lo)).clamp(0, span) * 1024 / span) as u32
}

impl Shell {
    /// Item `i`'s entrance, eased out: 0 just started, 1024 in place.
    pub(crate) fn ease(&self, i: u32) -> u32 {
        let Some(start) = self.anim else {
            return 1024;
        };
        let t = self.now.wrapping_sub(start);
        let t = t.saturating_sub(i * STAGGER_MS).min(ITEM_MS);
        let r = 1024 - t * 1024 / ITEM_MS;
        1024 - r * r / 1024 * r / 1024
    }

    /// How far right item `i` still is.
    fn slide(&self, i: u32) -> u16 {
        ((1024 - self.ease(i)) * SLIDE / 1024) as u16
    }

    /// Opens the Weather app on its Now page.
    pub(crate) fn open_weather(&mut self) {
        self.screen = super::Screen::Weather;
        self.tab = 0;
        self.scroll = 0;
        self.menu_open = false;
        self.pressed = None;
        if self.focus.is_some() {
            self.focus = Some(Target::Tab(0));
        }
        self.anim = Some(self.now);
        self.dirty(Rect::new(0, 0, W, H));
    }

    /// Shows page `t` (0 Now, 1 Hourly, 2 10 Day), arriving.
    pub(crate) fn set_tab(&mut self, t: u8) {
        let t = t.min(TAB_NAMES.len() as u8 - 1);
        if t == self.tab {
            return;
        }
        self.tab = t;
        self.scroll = 0;
        self.anim = Some(self.now);
        self.dirty(PAGE);
        self.dirty(TABS);
    }

    /// Scrolls the Hourly page by `rows` (clamped).
    pub(crate) fn scroll_by(&mut self, rows: i16) {
        let s = (i16::from(self.scroll) + rows).clamp(0, i16::from(MAX_SCROLL)) as u8;
        if s != self.scroll {
            self.scroll = s;
            self.dirty(PAGE);
        }
    }

    /// The Weather app's header, page and tabs (the title bar and softkeys
    /// are the shell's).
    pub(crate) fn weather_scene(&self, f: &mut Frame) {
        let (ui, label) = (self.fonts.ui, self.fonts.label);
        let wx = &self.wx;

        // Header: the place, the source and when it was read, today now.
        f.push(Cmd::HGradient {
            r: HEAD,
            left: GLASS_LEFT,
            right: GLASS_RIGHT,
        });
        f.push(Cmd::Fill {
            r: Rect::new(0, HEAD.y + HEAD.h - 1, W, 1),
            color: super::GLASS_EDGE,
        });
        let place = super::upper(wx.place);
        f.text(16, HEAD.y + 25, label, TEXT, place.as_bytes());
        let read = Line::new().push(wx.source.as_bytes()).push(b", ").push(
            self.region
                .time(self.time.hour, self.time.minute, true)
                .as_bytes(),
        );
        f.text(16, HEAD.y + 49, ui, DIM, read.as_bytes());
        let now = self.region.temp(wx.now.temp);
        let w = ui.width(now.as_bytes());
        f.text(W - 16 - w, HEAD.y + 37, ui, TEXT, now.as_bytes());

        match self.tab {
            0 => self.now_page(f),
            1 => self.hourly_page(f),
            _ => self.days_page(f),
        }

        // Tabs.
        f.push(Cmd::HGradient {
            r: TABS,
            left: BAR_LEFT,
            right: BAR_RIGHT,
        });
        for (i, name) in TAB_NAMES.iter().enumerate() {
            let r = tab_rect(i as u8);
            let on = usize::from(self.tab) == i;
            if on {
                f.push(Cmd::HGradient {
                    r,
                    left: SELECTED_LEFT,
                    right: SELECTED_RIGHT,
                });
                f.push(Cmd::Fill {
                    r: Rect::new(r.x + 40, r.y + r.h - 4, r.w - 80, 3),
                    color: FOCUS,
                });
            }
            self.centred(f, r, r.y + 28, label, if on { TEXT } else { DIM }, name);
            self.decorate(f, Target::Tab(i as u8));
        }
    }

    fn now_page(&self, f: &mut Frame) {
        let (ui, label, big) = (self.fonts.ui, self.fonts.label, self.fonts.big);
        let (wx, rg) = (&self.wx, &self.region);
        let icon = Icon::of(wx.now.sky);

        // The big panel: the temperature counting up, the sky, the day.
        let dx = self.slide(0);
        let p = Rect::new(16 + dx, 104, 448, 206);
        Self::glass(f, p, GLASS_LEFT, GLASS_RIGHT);
        let shown = rg.degrees(wx.now.temp) * self.ease(0) as i32 / 1024;
        let t = Line::new().num(shown).push(&[DEG_BIG]);
        f.text(p.x + 20, p.y + 104, big, TEXT, t.as_bytes());
        let pic = match (icon, self.icons.hero) {
            (Icon::PartlyDay, Some(h)) => Some(h),
            _ => self.icons.card[usize::from(icon.id() - 1)],
        };
        if let Some(img) = pic {
            let drop = ((1024 - self.ease(1)) * 24 / 1024) as u16;
            f.push(Cmd::Image {
                x: p.x + p.w - img.w - 6,
                y: (p.y + p.h.saturating_sub(img.h) / 2).saturating_sub(18 + drop),
                img,
            });
        }
        f.text(p.x + 20, p.y + 140, ui, TEXT, icon.words().as_bytes());
        let feels = Line::new()
            .push(b"Feels like ")
            .push(rg.temp(wx.now.feels).as_bytes());
        f.text(p.x + 20, p.y + 166, ui, DIM, feels.as_bytes());
        let d = &wx.days[0];
        let hl = Line::new()
            .push(b"High ")
            .push(rg.temp(d.high).as_bytes())
            .push(b"   Low ")
            .push(rg.temp(d.low).as_bytes());
        f.text(p.x + 20, p.y + 192, ui, DIM, hl.as_bytes());

        // The summary strip.
        let s = Rect::new(16 + self.slide(1), 318, 448, 40);
        Self::glass(f, s, GLASS_LEFT, GLASS_RIGHT);
        f.text(s.x + 16, s.y + 27, ui, TEXT, wx.summary(rg).as_bytes());

        // Four cards, each with a bar.
        let minutes = u32::from(self.time.hour) * 60 + u32::from(self.time.minute);
        let (rise, set) = (u32::from(wx.sun.0), u32::from(wx.sun.1));
        let day_gone = (minutes.clamp(rise, set) - rise) * 1024 / (set - rise).max(1);
        let uv = Line::new().num(i32::from(wx.now.uv)).push(match wx.now.uv {
            0..=2 => b" Low",
            3..=5 => b" Moderate",
            6..=7 => b" High",
            _ => b" Very high",
        });
        let cards: [(&[u8], Line, u32, bool); 4] = [
            (
                b"WIND",
                rg.wind(wx.now.sky.wind, wx.now.wind_from),
                u32::from(wx.now.sky.wind).min(600) * 1024 / 600,
                false,
            ),
            (
                b"HUMIDITY",
                Line::new().num(i32::from(wx.now.humidity)).push(b"%"),
                u32::from(wx.now.humidity) * 1024 / 100,
                false,
            ),
            (
                b"UV INDEX",
                uv,
                u32::from(wx.now.uv).min(11) * 1024 / 11,
                true,
            ),
            (
                b"SUNSET",
                rg.time((set / 60) as u8, (set % 60) as u8, true),
                day_gone,
                true,
            ),
        ];
        for (i, (name, value, amount, scale)) in cards.iter().enumerate() {
            let e = self.ease(2 + i as u32);
            let c = Rect::new(
                16 + (i as u16 % 2) * 232 + self.slide(2 + i as u32),
                366 + (i as u16 / 2) * 92,
                216,
                84,
            );
            Self::glass(f, c, GLASS_LEFT, GLASS_RIGHT);
            f.text(c.x + 14, c.y + 26, label, DIM, name);
            f.text(c.x + 14, c.y + 54, ui, TEXT, value.as_bytes());
            let track = Rect::new(c.x + 14, c.y + 66, 188, 6);
            f.push(Cmd::Fill {
                r: track,
                color: TRACK,
            });
            let w = (u32::from(track.w) * amount / 1024 * e / 1024) as u16;
            if w > 0 {
                let right = if *scale {
                    mix(COLD, WARM, *amount)
                } else {
                    FOCUS
                };
                f.push(Cmd::HGradient {
                    r: Rect::new(track.x, track.y, w, track.h),
                    left: if *scale { mix(COLD, WARM, 0) } else { RAIN },
                    right,
                });
            }
        }
    }

    fn hourly_page(&self, f: &mut Frame) {
        let (ui, label) = (self.fonts.ui, self.fonts.label);
        let (wx, rg) = (&self.wx, &self.region);
        let (lo, hi) = wx.hours.iter().fold((i16::MAX, i16::MIN), |(a, b), h| {
            (a.min(h.temp), b.max(h.temp))
        });
        for k in 0..ROWS {
            let i = usize::from(self.scroll) + k;
            let Some(h) = wx.hours.get(i) else { break };
            let e = self.ease(k as u32);
            let mut r = row(k);
            r.x += self.slide(k as u32);
            Self::glass(f, r, GLASS_LEFT, GLASS_RIGHT);
            let when = if i == 0 {
                Line::new().push(b"NOW")
            } else {
                rg.time(h.hour, 0, false)
            };
            f.text(r.x + 14, r.y + 28, label, TEXT, when.as_bytes());
            let icon = Icon::of(h.sky);
            if let Some(img) = self.icons.row[usize::from(icon.id() - 1)] {
                f.push(Cmd::Image {
                    x: r.x + 96 + (40 - img.w.min(40)) / 2,
                    y: r.y + (r.h.saturating_sub(img.h)) / 2,
                    img,
                });
            }
            if h.precip >= 10 {
                let p = Line::new().num(i32::from(h.precip)).push(b"%");
                f.text(r.x + 146, r.y + 28, ui, RAIN, p.as_bytes());
            }
            // The temperature on the day's scale: a bar from the coldest
            // hour to this one, coloured cold to warm.
            let track = Rect::new(r.x + 200, r.y + 18, 168, 6);
            f.push(Cmd::Fill {
                r: track,
                color: TRACK,
            });
            let at = frac(h.temp, lo, hi);
            let w = (12 + 156 * at / 1024) * e / 1024;
            if w > 0 {
                f.push(Cmd::HGradient {
                    r: Rect::new(track.x, track.y, w as u16, track.h),
                    left: mix(COLD, WARM, 0),
                    right: mix(COLD, WARM, at),
                });
            }
            let t = rg.temp(h.temp);
            let tw = ui.width(t.as_bytes());
            f.text(r.x + r.w - 14 - tw, r.y + 28, ui, TEXT, t.as_bytes());
        }
        // Where the list is: a thin bar at the right edge.
        let track = Rect::new(W - 9, ROW_Y, 3, ROW_STEP * ROWS as u16 - 3);
        f.push(Cmd::Fill {
            r: track,
            color: TRACK,
        });
        let thumb_h = track.h * ROWS as u16 / HOURS as u16;
        let y = track.y + (track.h - thumb_h) * u16::from(self.scroll) / u16::from(MAX_SCROLL);
        f.push(Cmd::Fill {
            r: Rect::new(track.x, y, 3, thumb_h),
            color: FOCUS,
        });
    }

    fn days_page(&self, f: &mut Frame) {
        const NAMES: [&[u8]; 7] = [b"MON", b"TUE", b"WED", b"THU", b"FRI", b"SAT", b"SUN"];
        let (ui, label) = (self.fonts.ui, self.fonts.label);
        let (wx, rg) = (&self.wx, &self.region);
        let (lo, hi) = wx.range();
        for (k, d) in wx.days.iter().enumerate().take(DAYS.min(ROWS)) {
            let e = self.ease(k as u32);
            let mut r = row(k);
            r.x += self.slide(k as u32);
            Self::glass(f, r, GLASS_LEFT, GLASS_RIGHT);
            let name: &[u8] = if k == 0 {
                b"TODAY"
            } else {
                NAMES[usize::from(d.weekday.min(6))]
            };
            f.text(r.x + 14, r.y + 28, label, TEXT, name);
            let icon = Icon::of(d.sky);
            if let Some(img) = self.icons.row[usize::from(icon.id() - 1)] {
                f.push(Cmd::Image {
                    x: r.x + 90 + (40 - img.w.min(40)) / 2,
                    y: r.y + (r.h.saturating_sub(img.h)) / 2,
                    img,
                });
            }
            if d.precip >= 20 {
                let p = Line::new().num(i32::from(d.precip)).push(b"%");
                f.text(r.x + 138, r.y + 28, ui, RAIN, p.as_bytes());
            }
            let low = rg.temp(d.low);
            let lw = ui.width(low.as_bytes());
            f.text(r.x + 230 - lw, r.y + 28, ui, DIM, low.as_bytes());
            // The day's range on the week's scale, coloured by its own
            // temperatures; it grows out from its low.
            let track = Rect::new(r.x + 240, r.y + 18, 140, 6);
            f.push(Cmd::Fill {
                r: track,
                color: TRACK,
            });
            let (a, b) = (frac(d.low, lo, hi), frac(d.high, lo, hi));
            let x0 = u32::from(track.w) * a / 1024;
            let w = (u32::from(track.w) * (b - a) / 1024).max(6) * e / 1024;
            if w > 0 {
                f.push(Cmd::HGradient {
                    r: Rect::new(track.x + x0 as u16, track.y, w as u16, track.h),
                    left: mix(COLD, WARM, a),
                    right: mix(COLD, WARM, b),
                });
            }
            let high = rg.temp(d.high);
            let hw = ui.width(high.as_bytes());
            f.text(r.x + r.w - 14 - hw, r.y + 28, ui, TEXT, high.as_bytes());
        }
    }
}
