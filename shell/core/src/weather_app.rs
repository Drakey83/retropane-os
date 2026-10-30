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
pub(crate) const TAB_NAMES: [&[u8]; 4] = [b"NOW", b"HOURLY", b"10 DAY", b"RADAR"];
/// The radar's page.
pub(crate) const RADAR: u8 = 3;
/// The radar's timeline panel, its play control and its track.
const TIME_PANEL: Rect = Rect::new(16, 468, 448, 80);
pub(crate) const PLAY: Rect = Rect::new(20, 474, 56, 68);
pub(crate) const TIMELINE: Rect = Rect::new(80, 468, 384, 80);
const TRACK_X: u16 = 96;
const TRACK_W: u16 = 352;
/// How long a radar frame shows (ms), and the last one before the loop
/// starts again.
const RADAR_FRAME_MS: u32 = 400;
const RADAR_HOLD_MS: u32 = 1200;

/// Rows on the Hourly and 10 Day pages.
const ROWS: usize = 10;
const ROW_Y: u16 = 102;
const ROW_STEP: u16 = 45;
const ROW_H: u16 = 42;
/// How far page `tab` scrolls (px): the Hourly list's last row up to the
/// page's bottom; the pages that fit, 0 (they still give way and spring
/// back).
pub(crate) const fn max_scroll(tab: u8) -> u16 {
    let bottom = PAGE.y + PAGE.h;
    let end = match tab {
        1 => ROW_Y + HOURS as u16 * ROW_STEP + 4,
        _ => 0,
    };
    end.saturating_sub(bottom)
}

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
    let w = W / TAB_NAMES.len() as u16;
    Rect::new(i as u16 * w, TABS.y, w, TABS.h)
}

/// The radar frame nearest screen x on the timeline.
pub(crate) fn frame_at(x: u16) -> u8 {
    let last = u32::from(rp_weather::radar::FRAMES - 1);
    let at = u32::from(x.clamp(TRACK_X, TRACK_X + TRACK_W) - TRACK_X);
    ((at * last + u32::from(TRACK_W) / 2) / u32::from(TRACK_W)) as u8
}

/// Where frame `k` sits on the timeline.
fn frame_x(k: u8) -> u16 {
    let last = u16::from(rp_weather::radar::FRAMES - 1);
    TRACK_X + TRACK_W * u16::from(k) / last
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
        self.scroll = super::motion::Scroll::new(max_scroll(0));
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
        self.scroll = super::motion::Scroll::new(max_scroll(t));
        self.anim = Some(self.now);
        self.dirty(PAGE);
        self.dirty(TABS);
    }

    /// Scrolls the page by `rows` rows (the D-pad; clamped).
    pub(crate) fn scroll_by(&mut self, rows: i16) {
        let before = self.scroll.offset();
        self.scroll
            .set(before + i32::from(rows) * i32::from(ROW_STEP));
        if self.scroll.offset() != before {
            self.dirty(PAGE);
        }
    }

    /// Where content laid out at `y` is drawn, scrolled (may be negative
    /// or past the page: the clip keeps it in).
    fn sy(&self, y: u16) -> i32 {
        i32::from(y) - self.scroll.offset()
    }

    /// A glass panel at a scrolled `y`, cut at the screen's top (the clip
    /// does the rest; the tint runs across, so a cut keeps it).
    fn glass_at(f: &mut Frame, x: u16, y: i32, w: u16, h: u16) {
        let top = y.max(0);
        let bottom = y + i32::from(h);
        if bottom <= top {
            return;
        }
        let r = Rect::new(x, top as u16, w, (bottom - top) as u16);
        f.push(Cmd::HGradient {
            r,
            left: GLASS_LEFT,
            right: GLASS_RIGHT,
        });
        // Cut too: its top edge then sits at the screen's top, outside
        // every page's clip.
        f.push(Cmd::Outline {
            r,
            color: super::GLASS_EDGE,
        });
    }

    /// Text whose baseline is at a scrolled `y` (left out when its line is
    /// above the screen: the page's clip hides it anyway).
    fn text_at(f: &mut Frame, x: u16, y: i32, font: &'static rp_draw::Font, c: Argb, t: &[u8]) {
        if y - i32::from(font.ascent) >= 0 && y < i32::from(H) + 64 {
            f.text(x, y as u16, font, c, t);
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
        self.back_arrow(f);
        let place = super::upper(wx.place);
        f.text(72, HEAD.y + 25, label, TEXT, place.as_bytes());
        let read = Line::new().push(wx.source.as_bytes()).push(b", ").push(
            self.region
                .time(self.time.hour, self.time.minute, true)
                .as_bytes(),
        );
        f.text(72, HEAD.y + 49, ui, DIM, read.as_bytes());
        let now = self.region.temp(wx.now.temp);
        let w = ui.width(now.as_bytes());
        f.text(W - 16 - w, HEAD.y + 37, ui, TEXT, now.as_bytes());

        // The page scrolls inside its own rectangle.
        f.clip(Some(PAGE));
        match self.tab {
            0 => self.now_page(f),
            1 => self.hourly_page(f),
            2 => self.days_page(f),
            _ => self.radar_page(f),
        }
        f.clip(None);

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
                // The underline follows a swipe live: a full tab's width
                // for a swipe of the turning distance and more, the way the
                // page will go (a swipe left moves on).
                let lead = -i32::from(self.swipe_dx()) * i32::from(r.w) / 180;
                let x = (i32::from(r.x) + 40 + lead).clamp(0, i32::from(W) - i32::from(r.w - 80));
                f.push(Cmd::Fill {
                    r: Rect::new(x as u16, r.y + r.h - 4, r.w - 80, 3),
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
        let (px, py) = (16 + dx, self.sy(104));
        Self::glass_at(f, px, py, 448, 206);
        let shown = rg.degrees(wx.now.temp) * self.ease(0) as i32 / 1024;
        let t = Line::new().num(shown).push(&[DEG_BIG]);
        Self::text_at(f, px + 20, py + 104, big, TEXT, t.as_bytes());
        let pic = match (icon, self.icons.hero) {
            (Icon::PartlyDay, Some(h)) => Some(h),
            _ => self.icons.card[usize::from(icon.id() - 1)],
        };
        if let Some(img) = pic {
            let drop = ((1024 - self.ease(1)) * 24 / 1024) as i32;
            let y = py + i32::from(206u16.saturating_sub(img.h) / 2) - 18 - drop;
            if y >= 0 {
                f.push(Cmd::Image {
                    x: px + 448 - img.w - 6,
                    y: y as u16,
                    img,
                });
            }
        }
        Self::text_at(f, px + 20, py + 140, ui, TEXT, icon.words().as_bytes());
        let feels = Line::new()
            .push(b"Feels like ")
            .push(rg.temp(wx.now.feels).as_bytes());
        Self::text_at(f, px + 20, py + 166, ui, DIM, feels.as_bytes());
        let d = &wx.days[0];
        let hl = Line::new()
            .push(b"High ")
            .push(rg.temp(d.high).as_bytes())
            .push(b"   Low ")
            .push(rg.temp(d.low).as_bytes());
        Self::text_at(f, px + 20, py + 192, ui, DIM, hl.as_bytes());

        // The summary strip.
        let (sx, sy) = (16 + self.slide(1), self.sy(318));
        Self::glass_at(f, sx, sy, 448, 40);
        Self::text_at(f, sx + 16, sy + 27, ui, TEXT, wx.summary(rg).as_bytes());

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
            let cx = 16 + (i as u16 % 2) * 232 + self.slide(2 + i as u32);
            let cy = self.sy(366 + (i as u16 / 2) * 92);
            Self::glass_at(f, cx, cy, 216, 84);
            Self::text_at(f, cx + 14, cy + 26, label, DIM, name);
            Self::text_at(f, cx + 14, cy + 54, ui, TEXT, value.as_bytes());
            if cy + 66 < 0 {
                continue;
            }
            let track = Rect::new(cx + 14, (cy + 66) as u16, 188, 6);
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
        // Only the rows on the page (and the one or two cut at its edges).
        let off = self.scroll.offset();
        let first = ((off - i32::from(ROW_H)).max(0) / i32::from(ROW_STEP)) as usize;
        for i in first..HOURS {
            let Some(h) = wx.hours.get(i) else { break };
            let y = i32::from(ROW_Y) + i as i32 * i32::from(ROW_STEP) - off;
            if y >= i32::from(PAGE.y + PAGE.h) {
                break;
            }
            // The entrance staggers the rows on screen, not the list's.
            let k = (i - first) as u32;
            let e = self.ease(k);
            let r = Rect::new(16 + self.slide(k), y.max(0) as u16, 448, ROW_H);
            Self::glass_at(f, r.x, y, r.w, r.h);
            if y < 0 {
                continue;
            }
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
        let max = i32::from(max_scroll(1)).max(1);
        let thumb_h = track.h * ROWS as u16 / HOURS as u16;
        let at = off.clamp(0, max);
        let y = track.y + ((i32::from(track.h - thumb_h) * at) / max) as u16;
        f.push(Cmd::Fill {
            r: Rect::new(track.x, y, 3, thumb_h),
            color: FOCUS,
        });
    }

    /// Moves the radar loop on to `now`: true if its frame changed.
    pub(crate) fn radar_step(&mut self, now: u32) -> bool {
        if self.screen != super::Screen::Weather || self.tab != RADAR || !self.radar_play {
            self.radar_at = now;
            return false;
        }
        let last = rp_weather::radar::FRAMES - 1;
        let wait = if self.radar_frame == last {
            RADAR_HOLD_MS
        } else {
            RADAR_FRAME_MS
        };
        if now.wrapping_sub(self.radar_at) < wait {
            return false;
        }
        self.radar_at = now;
        self.radar_frame = if self.radar_frame >= last {
            0
        } else {
            self.radar_frame + 1
        };
        self.dirty(PAGE);
        true
    }

    /// Whether the radar's map is under the page, and which look: `Some(true)`
    /// Ink, `Some(false)` Night (Auto: Ink between sunrise and sunset). The
    /// caller puts the map into the wallpaper under the page while it is.
    #[must_use]
    pub fn backdrop(&self) -> Option<bool> {
        if self.screen != super::Screen::Weather || self.tab != RADAR {
            return None;
        }
        let m = u16::from(self.time.hour) * 60 + u16::from(self.time.minute);
        Some(match self.wset.radar {
            rp_weather::settings::RadarMap::Ink => true,
            rp_weather::settings::RadarMap::Night => false,
            rp_weather::settings::RadarMap::Auto => m >= self.wx.sun.0 && m < self.wx.sun.1,
        })
    }

    /// The radar: the rain cells over the map (the backdrop), a chip naming
    /// the layer, and the timeline with play and pause.
    fn radar_page(&self, f: &mut Frame) {
        use rp_weather::radar::{FRAMES, NOW_FRAME, STEP_MIN, STEPS, TRACKS};
        let (ui, label) = (self.fonts.ui, self.fonts.label);
        for t in TRACKS {
            let Some(img) = self.icons.radar[usize::from(t.cell)] else {
                continue;
            };
            let (x, y) = t.at(self.radar_frame);
            if x >= 0 && y >= 0 && x as u16 + img.w <= W {
                f.push(Cmd::Image {
                    x: x as u16,
                    y: y as u16,
                    img,
                });
            }
        }
        let chip = Rect::new(16, 104, 220, 40);
        // The menus' denser glass: readable over the light Ink map too.
        Self::glass(f, chip, super::MENU_LEFT, super::MENU_RIGHT);
        f.text(chip.x + 14, chip.y + 27, label, TEXT, b"RAIN  SAMPLE");
        Self::glass(f, TIME_PANEL, super::MENU_LEFT, super::MENU_RIGHT);
        // Play or pause: a triangle of steps, or two bars.
        let (px, py) = (PLAY.x + 20, PLAY.y + 20);
        if self.radar_play {
            for k in 0..8u16 {
                f.push(Cmd::Fill {
                    r: Rect::new(px + 2 * k, py + k, 2, 28 - 2 * k),
                    color: TEXT,
                });
            }
        } else {
            for dx in [0u16, 12] {
                f.push(Cmd::Fill {
                    r: Rect::new(px + dx, py, 6, 28),
                    color: TEXT,
                });
            }
        }
        self.decorate(f, Target::Play);
        // The frame's time and whether it is past or forecast.
        let k = i16::from(self.radar_frame) - i16::from(NOW_FRAME);
        let m = (i16::from(self.time.hour) * 60 + i16::from(self.time.minute) + k * STEP_MIN)
            .rem_euclid(24 * 60);
        let when = self
            .region
            .time((m / 60) as u8, (m % 60) as u8, true)
            .push(match k {
                0 => b"  now",
                k if k < 0 => b"  past",
                _ => b"  forecast",
            });
        f.text(TRACK_X, TIME_PANEL.y + 28, ui, TEXT, when.as_bytes());
        let ends = b"-1 h       +1 h";
        f.text(
            TRACK_X + TRACK_W - ui.width(ends),
            TIME_PANEL.y + 28,
            ui,
            DIM,
            ends,
        );
        let ty = TIME_PANEL.y + 44;
        f.push(Cmd::Fill {
            r: Rect::new(TRACK_X, ty, TRACK_W, 4),
            color: TRACK,
        });
        for i in 0..FRAMES {
            let h = if i == NOW_FRAME { 12 } else { 6 };
            f.push(Cmd::Fill {
                r: Rect::new(frame_x(i), ty + 2 - h / 2, 2, h),
                color: if i == NOW_FRAME { WARM } else { DIM },
            });
        }
        f.push(Cmd::Fill {
            r: Rect::new(frame_x(self.radar_frame).saturating_sub(5), ty - 7, 12, 18),
            color: FOCUS,
        });
        // The scale: light to heavy.
        for (i, (c, _)) in STEPS.iter().enumerate() {
            f.push(Cmd::Fill {
                r: Rect::new(TRACK_X + 64 + i as u16 * 48, TIME_PANEL.y + 62, 44, 6),
                color: rp_draw::look::theme(0xFF00_0000 | c),
            });
        }
        f.text(TRACK_X, TIME_PANEL.y + 72, label, DIM, b"LIGHT");
        f.text(
            TRACK_X + 64 + 5 * 48 + 6,
            TIME_PANEL.y + 72,
            label,
            DIM,
            b"HEAVY",
        );
        if self.focus == Some(Target::Timeline) {
            f.push(Cmd::Outline {
                r: TIMELINE,
                color: FOCUS,
            });
        }
    }

    fn days_page(&self, f: &mut Frame) {
        const NAMES: [&[u8]; 7] = [b"MON", b"TUE", b"WED", b"THU", b"FRI", b"SAT", b"SUN"];
        let (ui, label) = (self.fonts.ui, self.fonts.label);
        let (wx, rg) = (&self.wx, &self.region);
        let (lo, hi) = wx.range();
        for (k, d) in wx.days.iter().enumerate().take(DAYS.min(ROWS)) {
            let e = self.ease(k as u32);
            let y = self.sy(row(k).y);
            if y + i32::from(ROW_H) <= 0 || y >= i32::from(PAGE.y + PAGE.h) {
                continue;
            }
            let r = Rect::new(16 + self.slide(k as u32), y.max(0) as u16, 448, ROW_H);
            Self::glass_at(f, r.x, y, r.w, r.h);
            if y < 0 {
                continue;
            }
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
