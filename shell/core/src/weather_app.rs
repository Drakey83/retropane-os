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

use super::{DEG_BIG, DIM, FOCUS, H, Shell, TEXT, Target, W};

/// The header: one thin row (the mix mock-up's): the place and its menu,
/// the places' dots, an alert, the reading's time. Public with [`PAGE`] and
/// [`TABS`] for the picture checks (emu-today), which read the layout from
/// here, never as numbers of their own (theirs went stale twice).
pub const HEAD: Rect = Rect::new(0, 36, W, 34);
/// The header's Station button, left of the reading's time (shown once a
/// station answers).
pub(crate) const STATION_BTN: Rect = Rect::new(W - 196, 36 + 5, 84, 24);
/// The header's place and its menu.
pub(crate) const PLACE_BTN: Rect = Rect::new(0, 36, 200, 34);
/// The alert chip's colours (amber, the mock-up's).
/// The card's word shadows: dark in the dark look, a light halo in the
/// light look (kept by the light look's mapping).
pub(crate) const SHADOW_DARK: Argb = 0x8C00_0000;
pub(crate) const SHADOW_LIGHT: Argb = 0xB4FF_FFFF;
const ALERT: Argb = rp_draw::look::theme(0xFFF2_B45A);
/// The alert screen's name.
pub(crate) const ALERT_TEXT: Argb = ALERT;
const ALERT_BACK: Argb = rp_draw::look::theme(0xFF4A_3A24);
/// The page between the header and the tabs.
pub const PAGE: Rect = Rect::new(0, 70, W, 486);
/// The tab strip above the softkeys.
pub const TABS: Rect = Rect::new(0, H - 84, W, 44);
/// The pages.
pub(crate) const TAB_NAMES: [&[u8]; 4] = [b"Now", b"Hourly", b"10 Day", b"Radar"];
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
const ROW_Y: u16 = 78;
const ROW_STEP: u16 = 45;
const ROW_H: u16 = 42;
/// How far page `tab` scrolls (px): the Hourly list's last row up to the
/// page's bottom; the pages that fit, 0 (they still give way and spring
/// back).
pub(crate) const fn max_scroll(tab: u8) -> u16 {
    // Hourly: the graph and card, then the 24-hour grid to scroll to; the
    // other pages fit (they still give way and spring back).
    let bottom = PAGE.y + PAGE.h;
    let end = match tab {
        1 => GRID_Y + GRID_ROWS as u16 * GRID_STEP + 4,
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
/// Hourly's graph and the picked hour's card (R140, the mix mock-up X2).
pub(crate) const HOURLY_GRAPH: Rect = Rect::new(16, 78, 448, 240);
const HOURLY_CARD: Rect = Rect::new(16, 326, 448, 196);
/// The hour card's lines, from its left: the picture's column (up to
/// [`HOUR_PIC_W`] wide) is left of them.
const HOUR_TEXT_X: u16 = 176;
/// The widest picture the hour card shows (the size the icon cache decodes
/// it at).
pub const HOUR_PIC_W: u16 = 160;
/// The 24-hour grid under the card: four rows of six cells.
const GRID_Y: u16 = 550;
const GRID_ROWS: usize = 4;
const GRID_H: u16 = 112;
const GRID_STEP: u16 = 118;
const CELL_W: u16 = 448 / 6;
/// The graph's hours: 24 across, each a column.
const GRAPH_X0: u16 = 24;
const HOUR_W: u16 = 36;
/// Hours on the graph (the mock-up's: now and the next 11).
const GRAPH_HOURS: usize = 12;
/// Hourly's graph's picture (its line and the band under it), from the
/// panel's first inner column and [`GRAPH_CUT`] rows below its top: width,
/// height (to the band's foot). Painted once a forecast by
/// [`Shell::paint_graph`] into a store picture and drawn as one, on the
/// planes and the 2D path alike. It must fit the 2D engine's picture cache
/// (256 KiB): at 446 x 160 (285 KB) the 2D path drew no line at all (seen
/// in ui-preview 2026-11-01; on the X51v, Hourly with a menu over it).
pub const GRAPH_PIC: (u16, u16) = (HOURLY_GRAPH.w - 2, 120);
/// The graph's rows above its picture: the curve's top is row 58 and a
/// Catmull-Rom overshoot with the line's width stays below row 44.
const GRAPH_CUT: u16 = 40;
/// The radar's play mark: a right-pointing triangle, anti-aliased by 4 x 4
/// samples a pixel, made when the shell is built (the 2D engine has
/// rectangles, not triangles; the stepped one of 2 px rectangles looked
/// broken on the panel: the owner's run, 2026-10-31, R161).
const PLAY_W: usize = 20;
const PLAY_H: usize = 28;
const PLAY_PX: [Argb; PLAY_W * PLAY_H] = {
    let mut px = [0; PLAY_W * PLAY_H];
    let (w, h) = (PLAY_W as i32, PLAY_H as i32);
    let mut y = 0;
    while y < h {
        let mut x = 0;
        while x < w {
            // Inside: |y - h/2| <= h/2 (1 - x / w), in eighths of a pixel.
            let mut hits = 0u32;
            let mut j = 0;
            while j < 4 {
                let mut i = 0;
                while i < 4 {
                    let sx = 8 * x + 2 * i + 1;
                    let sy = 8 * y + 2 * j + 1;
                    let dy = sy - 4 * h;
                    let dy = if dy < 0 { -dy } else { dy };
                    if dy * w <= 4 * h * w - (h / 2) * sx {
                        hits += 1;
                    }
                    i += 1;
                }
                j += 1;
            }
            px[(y * w + x) as usize] = (hits * 255 / 16) << 24 | (TEXT & 0x00FF_FFFF);
            x += 1;
        }
        y += 1;
    }
    px
};
static PLAY_ICON: rp_draw::Image = rp_draw::Image {
    w: PLAY_W as u16,
    h: PLAY_H as u16,
    px: &PLAY_PX,
};

#[cfg(test)]
pub(crate) fn play_icon() -> rp_draw::Image {
    rp_draw::Image {
        w: PLAY_W as u16,
        h: PLAY_H as u16,
        px: &PLAY_PX,
    }
}

/// Integer square root.
pub(crate) const fn isqrt(v: i32) -> i32 {
    let mut r = 0;
    while (r + 1) * (r + 1) <= v {
        r += 1;
    }
    r
}
const fn hour_x(i: u8) -> u16 {
    GRAPH_X0 + i as u16 * HOUR_W
}
pub(crate) const CURVE: Argb = rp_draw::look::theme(0xFFF2_B45A);
/// Under the curve: the mock-up's band, a light frost over the panel's
/// glass.
pub(crate) const FROST: Argb = rp_draw::look::theme(0x50F4_F2EC);

/// `top` over `under` (straight alpha) as the one layer that looks the
/// same: the renderer lays every see-through fill straight over the
/// wallpaper, so the band (frost over the panel's glass) is drawn as this.
pub(crate) const fn over(under: Argb, top: Argb) -> Argb {
    let (ua, ta) = (under >> 24, top >> 24);
    let a = ta + ua * (255 - ta) / 255;
    if a == 0 {
        return 0;
    }
    let mut out = a << 24;
    let mut s = 0;
    while s < 24 {
        let (u, t) = ((under >> s) & 0xFF, (top >> s) & 0xFF);
        let c = (t * ta + u * ua * (255 - ta) / 255) / a;
        out |= (if c > 255 { 255 } else { c }) << s;
        s += 8;
    }
    out
}

/// The rain bars: the mock-up's softer blue.
const RAIN_BAR: Argb = rp_draw::look::theme(0xFF5C_94DC);
/// The gauges' and timelines' track: solid (the owner, 2026-10-31: no
/// see-through bars).
const TRACK: Argb = rp_draw::look::theme(0xFF2C_3238);
/// The tab bar, its selected cell and the cells' dividers, opaque (the
/// agreed mix mock-up's colours, read from its pixels).
const TABS_LEFT: Argb = rp_draw::look::theme(0xFF0A_0E11);
const TABS_RIGHT: Argb = rp_draw::look::theme(0xFF25_2A30);
const SELECTED: Argb = rp_draw::look::theme(0xFF30_3539);
const DIVIDER: Argb = rp_draw::look::theme(0xFF4B_4E53);
/// A separator, " · " (the fonts draw 0x80 as the middle dot; the
/// sources' text is folded to ASCII, so it never holds that byte).
pub(crate) const SEP: &[u8] = b" \x80 ";
/// The header: slate, nearly opaque (the mock-up's).
const HEAD_LEFT: Argb = rp_draw::look::theme(0xF021_2B35);
const HEAD_RIGHT: Argb = rp_draw::look::theme(0xEC38_424E);
/// The temperature scale's ends: cold to warm.
const COLD: Argb = 0xFF4F_A8FF;
const WARM: Argb = 0xFFFF_A040;

/// The page layer's clear rows above and below a page's picture: the
/// rebound shows the still wallpaper through them (`motion.rs`'s
/// `BAND_MAX_PX`).
pub const LAYER_MARGIN: u16 = 120;

/// Rows in page `tab`'s picture on the page layer (480 wide): the margins,
/// the page, and how far it scrolls (Hourly: 1,332).
#[must_use]
pub const fn picture_rows(tab: u8) -> u16 {
    2 * LAYER_MARGIN + PAGE.h + max_scroll(tab)
}

/// A page as the plane path shows it (sheet weather-gpu-movement.md, ADR-0034
/// C4 as changed, L2): its picture, the video window on screen, and the
/// picture's row at the window's top.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PageView {
    /// The page (0 Now, 1 Hourly, 2 10 Day).
    pub tab: u8,
    /// The window (even sizes).
    pub win: Rect,
    /// The picture's row at its top.
    pub row: u16,
}

/// The Hourly list's scroll bar: on screen 0, through the layer's clear
/// right edge.
const THUMB_TRACK: Rect = Rect::new(W - 9, ROW_Y, 3, ROW_STEP * ROWS as u16 - 3);

/// Where tab `i` is.
pub(crate) const fn tab_rect(i: u8) -> Rect {
    let w = W / TAB_NAMES.len() as u16;
    Rect::new(i as u16 * w, TABS.y, w, TABS.h)
}

/// The radar frame nearest screen x on the timeline (of `n` frames).
pub(crate) fn frame_at(x: u16, n: u8) -> u8 {
    let last = u32::from(n.max(2) - 1);
    let at = u32::from(x.clamp(TRACK_X, TRACK_X + TRACK_W) - TRACK_X);
    ((at * last + u32::from(TRACK_W) / 2) / u32::from(TRACK_W)) as u8
}

/// Where frame `k` of `n` sits on the timeline.
fn frame_x(k: u8, n: u8) -> u16 {
    let last = u16::from(n.max(2) - 1);
    TRACK_X + TRACK_W * u16::from(k) / last
}

/// A row on the Hourly and 10 Day pages.
const fn row(k: usize) -> Rect {
    Rect::new(16, ROW_Y + k as u16 * ROW_STEP, 448, ROW_H)
}

/// How a page arrives (step 2 of build-ui-foundation.md: the owner picks
/// one from moving mock-ups; until then the first build's stays).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Motion {
    /// Rows slide in from the right one after another (the first build).
    Stagger,
    /// The whole page rises a little into place, as one.
    Rise,
    /// From the Today screen, the hero card grows into the app's big
    /// panel (a shared element), the rest following it; pages rise.
    Expand,
    /// No entrance: pages appear (the owner's rule for a movement under
    /// 30 fps that can't be optimized to run faster).
    None,
    /// The page shown from the top down, its window growing, while it
    /// rises as one (sheet weather-gpu-movement.md: the video plane's
    /// window and address moved in the blanking, no engine a frame).
    Cascade,
    /// From the Today screen the app opens out of the hero card: a window
    /// growing from the card to the screen, the app in place behind it,
    /// revealed, not scaled; pages then cascade (the owner's pick of the
    /// sheet's recommendation, 2026-10-30).
    Reveal,
}

/// The Now page's big panel's top (the hero card becomes it).
const PANEL_Y: u16 = 80;
/// The Now page's four cards: the first row's top, the step to the next.
const CARD_Y: u16 = 342;
const CARD_STEP: u16 = 92;
/// A card's bar, from the card's corner.
const BAR_DX: u16 = 14;
/// Your station on the Now page (N1, N2: docs/design/station-everywhere.md,
/// the owner's choice to try, 2026-11-01): off reverts to the forecast's
/// cards alone.
pub(crate) const STATION_ON_NOW: bool = true;
/// N2's strip, and N1's cards under it (shorter, to make its room).
const STRIP: Rect = Rect::new(16, CARD_Y, 448, 52);
const SMART_CARD_Y: u16 = CARD_Y + 60;
const SMART_CARD_STEP: u16 = 76;
const SMART_CARD_H: u16 = 70;
const BAR_DY: u16 = 66;
/// The Now page's sunset bar, unscrolled: its length follows the clock, so
/// a picture check masks it (emu-today; a mask of its own went stale when
/// the header shrank to 34 px and the cards moved up, 216 pixels "far").
pub const SUNSET_BAR: Rect = Rect::new(16 + 232 + BAR_DX, CARD_Y + CARD_STEP + BAR_DY, 188, 6);

/// How far a rising page starts below its place.
const RISE: u32 = 40;

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
        // A page drawn into its picture is drawn in place.
        if self.band.is_some() {
            return 1024;
        }
        // Cascade and Reveal move the page's window and place only: its
        // content is drawn once, final (the plane path draws nothing a
        // frame), so nothing inside it counts up or slides.
        if matches!(self.motion, Motion::Cascade | Motion::Reveal) {
            return 1024;
        }
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
        if self.motion != Motion::Stagger || self.band.is_some() {
            return 0;
        }
        ((1024 - self.ease(i)) * SLIDE / 1024) as u16
    }

    /// How much of the whole entrance is still to come, eased out: 1024
    /// just started, 0 in place (item 0's curve, stretched).
    fn whole_left(&self) -> u32 {
        let Some(start) = self.anim else { return 0 };
        let t = self.now.wrapping_sub(start).min(ANIM_MS);
        let r = 1024 - t * 1024 / ANIM_MS;
        r * r / 1024 * r / 1024
    }

    /// Whether this page arrives by cascading (Cascade, and Reveal's pages
    /// once the app is open).
    fn cascading(&self) -> bool {
        self.anim.is_some()
            && (self.motion == Motion::Cascade || self.motion == Motion::Reveal && !self.expanding)
    }

    /// How far below its place the page still is (Rise, Expand, Cascade).
    fn rise(&self) -> i32 {
        match self.motion {
            Motion::Rise | Motion::Expand => (self.whole_left() * RISE / 1024) as i32,
            _ if self.cascading() => (self.whole_left() * RISE / 1024) as i32,
            _ => 0,
        }
    }

    /// While the app opens by Reveal: the window it shows through, from
    /// the Today screen's hero card to everything under the title bar
    /// (the bar is the same on both screens), and how many rows lower the
    /// app is read inside it, so its big panel starts where the card was
    /// and eases into place (on the plane path, the window and an offset
    /// in the picture's start address).
    pub(crate) fn reveal(&self) -> Option<(Rect, u16)> {
        if self.motion != Motion::Reveal || !self.expanding || self.anim.is_none() {
            return None;
        }
        // Smoothstep, not the pages' ease-out: it starts from rest. The
        // ease-out's start, three times its mean speed, dropped the window's
        // foot ~38 px in the first refresh (the owner's 60 fps video,
        // 2026-11-01: "jerk the screen violently down at the very
        // beginning"); this one moves ~1 px then, fastest half-way.
        let t = i64::from(self.now.wrapping_sub(self.anim.unwrap_or(0)).min(ANIM_MS)) * 1024
            / i64::from(ANIM_MS);
        let left = (1024 - t * t * (3 * 1024 - 2 * t) / (1024 * 1024)) as i32;
        let lerp =
            |a: u16, b: u16| (i32::from(b) + (i32::from(a) - i32::from(b)) * left / 1024) as u16;
        let (card, top) = (super::HERO, super::TITLE.h);
        // The app read lower so its panel starts where the card was, or
        // held still (`reveal_glide` off: the device run telling apart the
        // glide and the hand-over, 2026-10-30).
        let shift = if self.reveal_glide {
            lerp(PANEL_Y - card.y, 0)
        } else {
            0
        };
        Some((
            Rect::new(
                lerp(card.x, 0),
                lerp(card.y, top),
                lerp(card.w, W),
                lerp(card.h, H - top),
            ),
            shift,
        ))
    }

    /// Expand's share of the way from the hero card to the page, 0-1024
    /// (1024 when not expanding).
    fn grow(&self) -> u32 {
        if self.motion != Motion::Expand || !self.expanding || self.band.is_some() {
            return 1024;
        }
        self.ease(0)
    }

    /// What an entrance redraws: the page, or from the title bar's foot
    /// down while the hero card grows (it starts above the page).
    pub(crate) fn page_area(&self) -> Rect {
        if self.reveal().is_some() {
            return Rect::new(0, 0, W, H);
        }
        if self.cascading() {
            // The page's window grows down from its top.
            let h = u32::from(PAGE.h) * (1024 - self.whole_left()) / 1024;
            return Rect::new(PAGE.x, PAGE.y, PAGE.w, (h as u16).max(1));
        }
        if self.grow() < 1024 {
            Rect::new(0, HEAD.y, W, PAGE.y + PAGE.h - HEAD.y)
        } else {
            PAGE
        }
    }

    /// Sets how pages arrive (the preview's mock-ups; the device keeps the
    /// build's until the owner picks).
    pub fn set_motion(&mut self, m: Motion) {
        self.motion = m;
    }

    /// The Weather app as Reveal opens it (its Now page at rest, drawn
    /// whole in 2D) into `f`, whatever is on screen now: the device
    /// composes it ahead while the Today screen is still, so a tap shows
    /// Reveal's window at the next refresh. Nothing on screen changes.
    pub fn weather_preview(&mut self, f: &mut Frame) {
        self.weather_preview_as(f, false);
    }

    /// [`Self::weather_preview`] on the plane path: the app's screen as
    /// Reveal leaves it, its chrome only (the page on the layer), composed
    /// ahead so Reveal ends with the screen plane pointed at it.
    pub fn weather_chrome_preview(&mut self, f: &mut Frame) {
        self.weather_preview_as(f, self.planes);
    }

    fn weather_preview_as(&mut self, f: &mut Frame, planes: bool) {
        let saved = (
            self.screen,
            self.tab,
            self.scroll,
            self.anim,
            self.expanding,
            self.planes,
            self.menu_open,
            self.pressed,
            self.focus,
        );
        self.screen = super::Screen::Weather;
        self.tab = 0;
        self.scroll = super::motion::Scroll::new(max_scroll(0));
        self.anim = None;
        self.expanding = false;
        self.planes = planes;
        self.menu_open = false;
        self.pressed = None;
        self.focus = None;
        f.clear();
        f.damage(Rect::new(0, 0, W, H));
        self.scene(f);
        self.look(f);
        (
            self.screen,
            self.tab,
            self.scroll,
            self.anim,
            self.expanding,
            self.planes,
            self.menu_open,
            self.pressed,
            self.focus,
        ) = saved;
    }

    /// The Weather app's screen as it will be once the entrance under way
    /// ends, into `f` (drawn whole): on the plane path its chrome, the page
    /// left to the layer. Drawn and held while Reveal runs, so Reveal ends
    /// in one switch with nothing left to draw. Nothing on screen changes.
    pub fn weather_after_entrance(&mut self, f: &mut Frame) {
        let saved = (self.anim, self.expanding);
        self.anim = None;
        self.expanding = false;
        f.clear();
        f.damage(Rect::new(0, 0, W, H));
        self.scene(f);
        self.look(f);
        (self.anim, self.expanding) = saved;
    }

    /// Whether Reveal's content glides up into place (the mock-up's) or is
    /// held still while the window grows.
    pub fn set_reveal_glide(&mut self, on: bool) {
        self.reveal_glide = on;
    }

    /// The plane path on (the device): the Weather pages on the page layer,
    /// arriving by Reveal from the Today card and Cascade between pages
    /// (the owner's picks, 2026-10-30).
    pub fn use_planes(&mut self, on: bool) {
        self.planes = on;
        if on {
            self.motion = Motion::Reveal;
        }
        self.dirty(Rect::new(0, 0, W, H));
    }

    /// Restarts the entrance under way from `now` (ms): the plane path's
    /// Reveal starts when its window first shows, not at the tap, so it
    /// grows from the card whatever composing the app took.
    pub fn restart_entrance(&mut self, now: u32) {
        self.now = now;
        if self.anim.is_some() {
            self.anim = Some(now);
        }
    }

    /// The page the layer shows now, if any: on the plane path, the Weather
    /// app's Now, Hourly or 10 Day page with nothing over it and not
    /// opening by Reveal (the radar, menus and settings stay 2D).
    pub(crate) fn layer_tab(&self) -> Option<u8> {
        (self.planes
            && self.screen == super::Screen::Weather
            && self.tab < RADAR
            && !self.menu_open
            && !self.place_menu
            && self.chart.is_none()
            && self.align.is_none()
            && !self.bare
            && self.reveal().is_none())
        .then_some(self.tab)
    }

    /// The page layer's view now (`None`: the page is drawn on screen 0).
    #[must_use]
    pub fn page_layer(&self) -> Option<PageView> {
        let tab = self.layer_tab()?;
        let a = self.page_area();
        let win = Rect::new(a.x, a.y, a.w & !1, (a.h & !1).max(2));
        let row = i32::from(LAYER_MARGIN) + self.scroll.offset() - self.rise();
        let last = i32::from(picture_rows(tab)) - i32::from(win.h);
        Some(PageView {
            tab,
            win,
            row: row.clamp(0, last) as u16,
        })
    }

    /// Reveal on the page layer (the owner's run, 2026-10-31: the end's
    /// hand-off from the 2D-composed app to the layer's mix changed the
    /// picture twice in 80 ms, DSIG): the opening's window over the page's
    /// own picture, so the opening ends already as the app rests. The
    /// window is the card's rectangle inside the page area, the picture
    /// read as at rest, `shift` rows lower for the glide.
    #[must_use]
    pub fn reveal_page(&self) -> Option<PageView> {
        let (r, shift) = self.reveal_plane()?;
        let a = PAGE;
        let top = r.y.max(a.y);
        let bottom = (r.y + r.h).min(a.y + a.h);
        if bottom <= top + 2 {
            return None;
        }
        let x = r.x & !7;
        let w = ((r.w + (r.x - x)).min(W - x)) & !1;
        let h = (bottom - top) & !1;
        let tab = self.tab;
        let row = LAYER_MARGIN + (top - a.y) + shift;
        let last = picture_rows(tab).saturating_sub(h);
        Some(PageView {
            tab,
            win: Rect::new(x, top, w.max(2), h.max(2)),
            row: row.min(last),
        })
    }

    /// While the app opens by Reveal on the plane path: the window and how
    /// many rows lower the app is read inside it.
    #[must_use]
    pub fn reveal_plane(&self) -> Option<(Rect, u16)> {
        self.reveal_window().filter(|_| self.planes)
    }

    /// Page `tab`'s picture rows `top..top + rows` into `f` (its damage
    /// those rows), in picture coordinates: the page in place, scrolled to
    /// its top, below [`LAYER_MARGIN`] clear rows. The CPU draws it once
    /// into the page layer's picture.
    pub fn page_band(&mut self, tab: u8, f: &mut Frame, top: u16, rows: u16) {
        f.clear();
        f.damage(Rect::new(0, top, W, rows));
        let saved = (self.tab, self.band);
        self.tab = tab;
        self.band = Some((i32::from(top), i32::from(top) + i32::from(rows)));
        match tab {
            0 => self.now_page(f),
            1 => self.hourly_page(f),
            _ => self.days_page(f),
        }
        self.look(f);
        (self.tab, self.band) = saved;
    }

    /// The page moved (a drag, a fling, the D-pad): redrawn on screen 0, or
    /// on the planes only the scroll bar there (the layer moves).
    pub(crate) fn page_moved(&mut self) {
        match self.layer_tab() {
            Some(1) if max_scroll(1) > 0 => self.dirty(THUMB_TRACK),
            Some(_) => {}
            None => self.dirty(PAGE),
        }
    }

    /// How far content laid out at `y` is drawn above it: the scroll less
    /// the rise, or in a picture the page's place below its margin.
    fn view_off(&self) -> i32 {
        if self.band.is_some() {
            i32::from(PAGE.y) - i32::from(LAYER_MARGIN)
        } else {
            self.scroll.offset() - self.rise()
        }
    }

    /// The rows drawn: the picture's band, or the page down to its bottom.
    fn view_rows(&self) -> (i32, i32) {
        self.band.unwrap_or((0, i32::from(PAGE.y + PAGE.h)))
    }

    /// Opens the Weather app on its Now page.
    pub(crate) fn open_weather(&mut self) {
        self.screen = super::Screen::Weather;
        self.tab = 0;
        self.scroll = super::motion::Scroll::new(max_scroll(0));
        self.menu_open = false;
        self.place_menu = false;
        self.pressed = None;
        if self.focus.is_some() {
            self.focus = Some(Target::Tab(0));
        }
        self.anim = (self.motion != Motion::None).then_some(self.now);
        self.expanding = true;
        self.dirty(Rect::new(0, 0, W, H));
    }

    /// Shows page `t` (0 Now, 1 Hourly, 2 10 Day), arriving.
    pub(crate) fn set_tab(&mut self, t: u8) {
        let t = t.min(TAB_NAMES.len() as u8 - 1);
        if t == self.tab {
            return;
        }
        let was = self.tab;
        self.tab = t;
        self.place_menu = false;
        self.scroll = super::motion::Scroll::new(max_scroll(t));
        self.anim = (self.motion != Motion::None).then_some(self.now);
        self.expanding = false;
        // On the planes the page's picture changes, not screen 0 (only the
        // scroll bar, Hourly's alone).
        if self.layer_tab().is_some() {
            if (was == 1 || t == 1) && max_scroll(1) > 0 {
                self.dirty(THUMB_TRACK);
            }
        } else {
            self.dirty(PAGE);
        }
        self.dirty(TABS);
    }

    /// Scrolls the page by `rows` rows (the D-pad; clamped).
    pub(crate) fn scroll_by(&mut self, rows: i16) {
        let before = self.scroll.offset();
        self.scroll
            .set(before + i32::from(rows) * i32::from(ROW_STEP));
        if self.scroll.offset() != before {
            self.page_moved();
        }
    }

    /// Where content laid out at `y` is drawn, scrolled (may be negative
    /// or past the page: the clip keeps it in).
    fn sy(&self, y: u16) -> i32 {
        i32::from(y) - self.view_off()
    }

    /// A glass panel at a scrolled `y`, cut at the screen's top (the clip
    /// does the rest; a cut panel loses its top corners, as it should).
    fn glass_at(&self, f: &mut Frame, x: u16, y: i32, w: u16, h: u16) {
        if self.panels_off {
            return;
        }
        let top = y.max(0);
        let bottom = y + i32::from(h);
        if bottom <= top {
            return;
        }
        let r = Rect::new(x, top as u16, w, (bottom - top) as u16);
        super::panel::draw(f, r, self.light());
    }

    /// Text whose baseline is at a scrolled `y` (left out when its line is
    /// above the screen: the page's clip hides it anyway).
    fn text_at(f: &mut Frame, x: u16, y: i32, font: &'static rp_draw::Font, c: Argb, t: &[u8]) {
        if y - i32::from(font.ascent) >= 0 && y < i32::from(H) + 64 {
            f.text(x, y as u16, font, c, t);
        }
    }

    /// The Weather app's header, page and tabs (the title bar and softkeys
    /// are the shell's), inside `outer` when Reveal shows it through a
    /// window.
    pub(crate) fn weather_scene(&self, f: &mut Frame, outer: Option<Rect>) {
        let (ui, label) = (self.fonts.ui, self.fonts.label);
        let wx = &self.wx;

        // Header (the mix mock-up's): the place and its menu, a dot for
        // each place, the alert, the station, when the forecast was read.
        f.push(Cmd::HGradient {
            r: HEAD,
            left: HEAD_LEFT,
            right: HEAD_RIGHT,
        });
        f.push(Cmd::Fill {
            r: Rect::new(0, HEAD.y + HEAD.h - 1, W, 1),
            color: super::GLASS_EDGE,
        });
        let base = HEAD.y + 24;
        let place = wx.place.as_bytes();
        self.press_under(f, Target::PlaceMenu);
        f.text(PLACE_BTN.x + 8, base, ui, TEXT, place);
        // The menu's mark: a small triangle pointing down.
        let mx = PLACE_BTN.x + 14 + ui.width(place);
        for k in 0..4u16 {
            f.push(Cmd::Fill {
                r: Rect::new(mx + k, HEAD.y + 14 + k, 8 - 2 * k, 1),
                color: DIM,
            });
        }
        self.decorate(f, Target::PlaceMenu);
        // The places: the one shown filled.
        let mut x = mx + 22;
        for i in 0..self.wset.n_places.min(10) {
            let r = Rect::new(x, HEAD.y + 13, 8, 8);
            if i == self.wset.current {
                f.push(Cmd::Fill { r, color: TEXT });
            } else {
                f.push(Cmd::Outline { r, color: DIM });
            }
            x += 14;
        }
        // The alert in force, if any.
        if let Some((r, a)) = self.alert_chip() {
            f.push(Cmd::Fill {
                r,
                color: ALERT_BACK,
            });
            self.press_under(f, Target::Alert);
            f.push(Cmd::Outline { r, color: ALERT });
            f.text(r.x + 9, r.y + 18, ui, ALERT, b"!");
            f.text(r.x + 24, r.y + 17, label, ALERT, a.as_bytes());
            self.decorate(f, Target::Alert);
        }
        // Your station, once it answers: its instruments a tap away.
        if self.station.is_some() {
            let r = STATION_BTN;
            f.push(Cmd::Fill {
                r,
                color: super::keyboard::KEY_FILL,
            });
            self.press_under(f, Target::Station);
            f.push(Cmd::Outline { r, color: FOCUS });
            super::keyboard::centre_in(f, r, label, TEXT, b"STATION");
            self.decorate(f, Target::Station);
        }
        // When the forecast was read (a clock time), or "sample".
        let read = self.read_at();
        let w = label.width(read.as_bytes());
        f.text(W - 12 - w, base - 2, label, DIM, read.as_bytes());
        // The page scrolls inside its own rectangle (an expanding hero card
        // may still reach above it).
        // On the planes Cascade is the video window's: screen 0's part of
        // the page (the scroll bar) is drawn whole from the first frame.
        let page = if self.reveal().is_some() || self.layer_tab().is_some() {
            PAGE
        } else {
            self.page_area()
        };
        f.clip(Some(outer.map_or(page, |o| page.intersect(&o))));
        match (self.layer_tab(), self.tab) {
            // On the layer: only the scroll bar is screen 0's.
            (Some(1), _) => self.hourly_thumb(f),
            (Some(_), _) => {}
            (None, 0) => self.now_page(f),
            (None, 1) => self.hourly_page(f),
            (None, 2) => self.days_page(f),
            (None, _) => self.radar_page(f),
        }
        f.clip(outer);

        // Tabs.
        f.push(Cmd::HGradient {
            r: TABS,
            left: TABS_LEFT,
            right: TABS_RIGHT,
        });
        for (i, name) in TAB_NAMES.iter().enumerate() {
            let r = tab_rect(i as u8);
            let on = usize::from(self.tab) == i;
            if i > 0 {
                f.push(Cmd::Fill {
                    r: Rect::new(r.x, r.y, 1, r.h),
                    color: DIVIDER,
                });
            }
            if on {
                f.push(Cmd::Fill {
                    r: Rect::new(r.x + 1, r.y, r.w - 1, r.h),
                    color: SELECTED,
                });
                // The underline follows a swipe live: a full tab's width
                // for a swipe of the turning distance and more, the way the
                // page will go (a swipe left moves on).
                // At most one tab either way, and never past the first or
                // last: a swipe turns one page (the owner, 2026-10-30: a
                // long swipe slid the line two or three tabs).
                let last = TAB_NAMES.len() as i32 - 1;
                let (lo, hi) = (
                    if i == 0 { 0 } else { -i32::from(r.w) },
                    if i as i32 == last { 0 } else { i32::from(r.w) },
                );
                let lead = (-i32::from(self.swipe_dx()) * i32::from(r.w) / 180).clamp(lo, hi);
                let x = (i32::from(r.x) + 1 + lead).clamp(0, i32::from(W) - i32::from(r.w - 1));
                f.push(Cmd::Fill {
                    r: Rect::new(x as u16, r.y + r.h - 3, r.w - 1, 3),
                    color: FOCUS,
                });
            }
            self.press_under(f, Target::Tab(i as u8));
            self.centred(f, r, r.y + 30, ui, if on { TEXT } else { DIM }, name);
            self.decorate(f, Target::Tab(i as u8));
        }
    }

    /// Text with a soft shadow under it (1 px down and right): the card's
    /// words stay readable over the living sky's clouds (the owner, 2026-
    /// 11-01: "the clouds should not appear behind lettering, it makes
    /// words hard to read"; his pick of the shadow over keeping the clouds
    /// away).
    fn shadowed(
        f: &mut Frame,
        sh: Argb,
        x: u16,
        y: i32,
        font: &'static rp_draw::Font,
        c: Argb,
        t: &[u8],
    ) {
        Self::text_at(f, x + 1, y + 1, font, sh, t);
        Self::text_at(f, x, y, font, c, t);
    }

    /// The weather card: Today's hero and the Now page's panel, the same
    /// lines in the same places from the card's top `y` (so opening the app
    /// by Reveal changes nothing in its first refresh): the place, marked
    /// LIVE with a green dot when your station is connected; the
    /// temperature, the station's then (S3, the owner's yes, 2026-11-01),
    /// else the forecast's; the sky in words; the high, low and feel; the
    /// date; the picture in its box (lowered `drop` px while arriving).
    pub(crate) fn weather_card(&self, f: &mut Frame, x: u16, y: i32, drop: i32) {
        let (ui, label, big) = (self.fonts.ui, self.fonts.label, self.fonts.big);
        let (wx, rg) = (&self.wx, &self.region);
        let live = self.smart_station();
        let sh = if self.light() {
            SHADOW_LIGHT
        } else {
            SHADOW_DARK
        };
        let mut lx = x + 20;
        if live.is_some() && y + 25 >= 0 {
            f.push(Cmd::Fill {
                r: Rect::new(x + 20, (y + 25) as u16, 7, 7),
                color: super::station::LIVE,
            });
            lx += 16;
        }
        // The place, cut before the picture's box (and LIVE after it).
        let live_w = if live.is_some() {
            label.width(b" LIVE")
        } else {
            0
        };
        let head = super::upper(wx.place.as_str());
        let mut b = head.as_bytes();
        while label.width(b) + live_w > super::HERO_LABEL_W - (lx - x - 20) {
            b = &b[..b.len() - 1];
        }
        Self::shadowed(f, sh, lx, y + 34, label, DIM, b);
        if live.is_some() {
            Self::shadowed(
                f,
                sh,
                lx + label.width(b),
                y + 34,
                label,
                super::station::LIVE,
                b" LIVE",
            );
        }
        let temp = live.and_then(|n| n.temp).unwrap_or(wx.now.temp);
        let feels = live.and_then(|n| n.feels).unwrap_or(wx.now.feels);
        let t = Line::new().num(rg.degrees(temp)).push(&[DEG_BIG]);
        Self::shadowed(f, sh, x + 20, y + 116, big, TEXT, t.as_bytes());
        let icon = Icon::of(wx.now.sky);
        Self::shadowed(f, sh, x + 20, y + 146, ui, TEXT, icon.words().as_bytes());
        let d = &wx.days[0];
        let hl = Line::new()
            .push(b"High ")
            .push(rg.temp(d.high).as_bytes())
            .push(b"  Low ")
            .push(rg.temp(d.low).as_bytes())
            .push(b"  Feels ")
            .push(rg.temp(feels).as_bytes());
        Self::shadowed(f, sh, x + 20, y + 174, ui, DIM, hl.as_bytes());
        // The picture in its box: 8 px past the card's right edge, from 14
        // px above its top (Today's HERO_ICON, from the card).
        if let Some(pic) = self.icons.big(icon) {
            let bx = i32::from(x) + 448 + 8 - 250;
            let by = y - 14 + i32::from(176u16.saturating_sub(pic.h) / 2) - drop;
            if by >= 0 {
                f.push(Cmd::Pic {
                    x: (bx + 250 - i32::from(pic.w)) as u16,
                    y: (by as u16).cast_signed(),
                    pic,
                });
            }
        }
        let mut buf = [0u8; 32];
        let date = super::date_line(&mut buf, self.time);
        Self::shadowed(
            f,
            sh,
            x + 432 - label.width(date),
            y + 176,
            label,
            TEXT,
            date,
        );
    }

    pub(crate) fn now_page(&self, f: &mut Frame) {
        let (ui, label, big) = (self.fonts.ui, self.fonts.label, self.fonts.big);
        let (wx, rg) = (&self.wx, &self.region);

        // The big panel: the temperature counting up, the sky, the day.
        let dx = self.slide(0);
        let (px, py) = (16 + dx, self.sy(PANEL_Y));
        let g = self.grow();
        if g < 1024 {
            // The hero card (16, 52, 448 x 196) growing into the panel
            // (16, 104, 448 x 206): the card alone until it is in place.
            let lerp = |a: i32, b: i32| a + (b - a) * g as i32 / 1024;
            let (y, h) = (lerp(52, i32::from(PANEL_Y)), lerp(196, 206));
            self.glass_at(f, 16, y, 448, h as u16);
            let t = Line::new().num(rg.degrees(wx.now.temp)).push(&[DEG_BIG]);
            Self::text_at(f, 36, y + lerp(116, 104), big, TEXT, t.as_bytes());
            if g < 700 {
                return;
            }
        }
        self.glass_at(f, px, py, 448, 206);
        // The same card as Today's, line for line and pixel for pixel, so
        // Reveal's first refresh shows exactly what was tapped (the owner's
        // 60 fps video, 2026-11-01: the app's lines sat 12-18 px from
        // Today's and its number differed, "just as jarring"); the panel's
        // last 10 rows come into view as the window grows.
        let drop = ((1024 - self.ease(1)) * 24 / 1024) as i32;
        self.weather_card(f, px, py, drop);
        let live = self.smart_station();

        // The summary strip.
        let (sx, sy) = (16 + self.slide(1), self.sy(294));
        self.glass_at(f, sx, sy, 448, 40);
        Self::text_at(f, sx + 16, sy + 27, ui, TEXT, wx.summary(rg).as_bytes());

        if let Some(n) = live {
            self.now_station_cards(f, &n);
            return;
        }
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
            let cy = self.sy(CARD_Y + (i as u16 / 2) * CARD_STEP);
            self.glass_at(f, cx, cy, 216, 84);
            Self::text_at(f, cx + 14, cy + 26, label, DIM, name);
            Self::text_at(f, cx + 14, cy + 54, ui, TEXT, value.as_bytes());
            let bar_y = cy + i32::from(BAR_DY);
            if bar_y < 0 {
                continue;
            }
            let track = Rect::new(cx + BAR_DX, bar_y as u16, SUNSET_BAR.w, SUNSET_BAR.h);
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

    /// Your station's readings when it is connected and recent (N1, N2:
    /// docs/design/station-everywhere.md), else `None` (the forecast's).
    pub(crate) fn smart_station(&self) -> Option<rp_weather::tempest::StationNow> {
        if !STATION_ON_NOW {
            return None;
        }
        let n = self.station?;
        // An hour old is not "live".
        (self.now.wrapping_sub(self.station_at) < 60 * 60_000).then_some(n)
    }

    /// N2, "what matters now": up to three of the station's readings,
    /// picked by what is happening (a storm's lightning, rain, strong
    /// gusts), then the steady ones (the pressure's trend, UV, dew point).
    pub(crate) fn strip_readings(
        &self,
        n: &rp_weather::tempest::StationNow,
    ) -> [Option<(&'static [u8], Line, Argb)>; 3] {
        let rg = &self.region;
        let us = rg.temp == rp_weather::Temp::F;
        let wet = matches!(self.wx.now.sky.wmo, 51..=67 | 80..=82 | 95..=99);
        let unit: &[u8] = match rg.wind {
            rp_weather::Wind::Mph => b" mph",
            rp_weather::Wind::Kmh => b" km/h",
            rp_weather::Wind::Ms => b" m/s",
        };
        let mut out = [None, None, None];
        let mut k = 0;
        let mut add = |r: (&'static [u8], Line, Argb)| {
            if k < 3 {
                out[k] = Some(r);
                k += 1;
            }
        };
        if let Some(c) = n.strikes_3h.filter(|&c| c > 0) {
            let mut l = Line::new().num(i32::from(c)).push(b" strikes");
            if let Some(km) = n.strike_km {
                l = l
                    .push(b", ")
                    .push(super::station::km_line(km, us).as_bytes());
            }
            add((b"LIGHTNING", l, super::station::BOLT));
        }
        if wet && let Some(r) = n.rain_today {
            add((b"RAIN TODAY", super::station::rain_line(r, us), RAIN));
        }
        // 20 mph and over.
        if let Some(g) = n.gust.filter(|&g| g >= 322) {
            add((b"GUSTS", Line::new().num(rg.wind_speed(g)).push(unit), TEXT));
        }
        if n.trend.is_some() {
            add((
                b"PRESSURE",
                Line::new().push(super::station::trend_word(n.trend)),
                DIM,
            ));
        }
        if let Some(u) = n.uv.filter(|&u| u >= 3) {
            add((b"UV", Line::new().num(i32::from(u)), DIM));
        }
        if let Some(d) = n.dew {
            add((b"DEW POINT", rg.temp(d), DIM));
        }
        if !wet && let Some(r) = n.rain_today.filter(|&r| r > 0) {
            add((b"RAIN TODAY", super::station::rain_line(r, us), RAIN));
        }
        out
    }

    /// Where slot `i`'s field is on the Now page with a station live: the
    /// big panel, the wind, pressure, humidity and sunset cards, the
    /// strip's three.
    pub(crate) fn now_reading_rect(&self, i: u8) -> Rect {
        let (x, y, w, h) = match i {
            0 => (16, PANEL_Y, 448, 206),
            1 => (16, SMART_CARD_Y, 216, SMART_CARD_H),
            2 => (248, SMART_CARD_Y, 216, SMART_CARD_H),
            3 => (16, SMART_CARD_Y + SMART_CARD_STEP, 216, SMART_CARD_H),
            4 => (248, SMART_CARD_Y + SMART_CARD_STEP, 216, SMART_CARD_H),
            // The strip's items.
            k => (16 + (k.min(7) - 5) as u16 * 152, STRIP.y, 144, STRIP.h),
        };
        let top = (i32::from(y) - self.view_off()).clamp(0, i32::from(H)) as u16;
        Rect::new(x, top, w, h)
    }

    /// N1 and N2 on the Now page: the strip, then four cards with your
    /// station's readings where it has them (a green dot) and the
    /// forecast's sunset.
    fn now_station_cards(&self, f: &mut Frame, n: &rp_weather::tempest::StationNow) {
        let (ui, label) = (self.fonts.ui, self.fonts.label);
        let (wx, rg) = (&self.wx, &self.region);
        let us = rg.temp == rp_weather::Temp::F;
        let dash = Line::new().push(b"--");
        let unit: &[u8] = match rg.wind {
            rp_weather::Wind::Mph => b" mph",
            rp_weather::Wind::Kmh => b" km/h",
            rp_weather::Wind::Ms => b" m/s",
        };
        for (i, r) in self.strip_readings(n).iter().enumerate() {
            let Some((name, value, color)) = r else {
                continue;
            };
            let x = 16 + i as u16 * 152 + self.slide(2);
            let y = self.sy(STRIP.y);
            self.glass_at(f, x, y, 144, STRIP.h);
            Self::text_at(f, x + 10, y + 20, label, *color, name);
            Self::text_at(f, x + 10, y + 44, ui, TEXT, value.as_bytes());
        }
        let minutes = u32::from(self.time.hour) * 60 + u32::from(self.time.minute);
        let (rise, set) = (u32::from(wx.sun.0), u32::from(wx.sun.1));
        let day_gone = (minutes.clamp(rise, set) - rise) * 1024 / (set - rise).max(1);
        let wind = n.wind.map_or(dash, |w| {
            let mut l = Line::new().num(rg.wind_speed(w)).push(unit);
            if let Some(d) = n.wind_from {
                l = l.push(b" from ").push(rp_weather::compass(d));
            }
            if let Some(g) = n.gust {
                l = l.push(b", gusts ").num(rg.wind_speed(g));
            }
            l
        });
        let pressure = n.pressure.map_or(dash, |p| {
            super::station::pressure_line(p, us)
                .push(b", ")
                .push(super::station::trend_word(n.trend))
        });
        let humidity = n
            .humidity
            .map_or(dash, |h| Line::new().num(i32::from(h)).push(b"%"));
        let pressure_amount =
            u32::from(n.pressure.unwrap_or(9800).clamp(9800, 10_400) - 9800) * 1024 / 600;
        let cards: [(&[u8], Line, u32, bool, bool); 4] = [
            (
                b"WIND",
                wind,
                u32::from(n.wind.unwrap_or(0)).min(600) * 1024 / 600,
                false,
                n.wind.is_some(),
            ),
            (
                b"PRESSURE",
                pressure,
                pressure_amount,
                false,
                n.pressure.is_some(),
            ),
            (
                b"HUMIDITY",
                humidity,
                u32::from(n.humidity.unwrap_or(0)) * 1024 / 100,
                false,
                n.humidity.is_some(),
            ),
            (
                b"SUNSET",
                rg.time((set / 60) as u8, (set % 60) as u8, true),
                day_gone,
                true,
                false,
            ),
        ];
        for (i, (name, value, amount, scale, live)) in cards.iter().enumerate() {
            let e = self.ease(2 + i as u32);
            let cx = 16 + (i as u16 % 2) * 232 + self.slide(2 + i as u32);
            let cy = self.sy(SMART_CARD_Y + (i as u16 / 2) * SMART_CARD_STEP);
            self.glass_at(f, cx, cy, 216, SMART_CARD_H);
            Self::text_at(f, cx + 14, cy + 22, label, DIM, name);
            if *live && cy + 13 >= 0 {
                f.push(Cmd::Fill {
                    r: Rect::new(cx + 20 + label.width(name), (cy + 13) as u16, 7, 7),
                    color: super::station::LIVE,
                });
            }
            Self::text_at(f, cx + 14, cy + 48, ui, TEXT, value.as_bytes());
            let bar_y = cy + 58;
            if bar_y < 0 {
                continue;
            }
            let track = Rect::new(cx + BAR_DX, bar_y as u16, 188, 4);
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

    pub(crate) fn hourly_page(&self, f: &mut Frame) {
        let dy = -self.view_off();
        let (top, bottom) = self.view_rows();
        let seen = |r: Rect| {
            let y = i32::from(r.y) + dy;
            y + i32::from(r.h) > top && y < bottom
        };
        if seen(HOURLY_GRAPH) {
            self.hourly_graph(f);
        }
        if seen(HOURLY_CARD) {
            self.hourly_card(f);
        }
        self.hourly_grid(f, dy, (top, bottom));
        if self.band.is_none() {
            self.hourly_thumb(f);
        }
    }

    /// Hourly's graph (the mix mock-up X2).
    fn hourly_graph(&self, f: &mut Frame) {
        let (ui, label) = (self.fonts.ui, self.fonts.label);
        let (wx, rg) = (&self.wx, &self.region);
        let shown = &wx.hours[..GRAPH_HOURS];
        let (lo, hi) = shown.iter().fold((i16::MAX, i16::MIN), |(a, b), h| {
            (a.min(h.temp), b.max(h.temp))
        });
        let dy = -self.view_off();
        let grow = self.ease(0);
        // The graph (the mix mock-up X2, the owner's pick): the next 12
        // hours across, times every two, the temperature as a smooth line
        // over a lighter band with its turns labelled, the chance of rain
        // as bars under it.
        let g = HOURLY_GRAPH;
        let gy = i32::from(g.y) + dy;
        self.glass_at(f, g.x, gy, g.w, g.h);
        let y0 = |v: i32| (v + gy).max(0) as u16;
        // A rectangle `h` tall from local row `y`, cut at the screen's top
        // (scrolled up, it must shrink, not slide down).
        let clip = |x: u16, y: i32, w: u16, h: i32| {
            let t = (y + gy).max(0);
            let b = (y + gy + h).max(t);
            Rect::new(x, t as u16, w, (b - t) as u16)
        };
        let (curve_top, span) = (58, 84);
        let base = curve_top + span + 18;
        let level = |t: i16| {
            let at = frac(t, lo, hi) as i32;
            let top = curve_top + span - span * at / 1024;
            base - (base - top) * grow as i32 / 1024
        };
        // The line and the band under it: one picture painted once a
        // forecast ([`Shell::paint_graph`]), drawn the same way on the
        // planes and the 2D path (the 2D path's 49 line pieces, ~95
        // commands a frame, are gone).
        if let Some(pic) = self.graph {
            f.push(Cmd::Pic {
                x: g.x + 1,
                y: i16::try_from(gy + i32::from(GRAPH_CUT)).unwrap_or(i16::MIN),
                pic,
            });
        }
        for (i, h) in shown.iter().enumerate() {
            let x = hour_x(i as u8) + HOUR_W / 2;
            if h.precip >= 10 {
                let bh = (i32::from(h.precip) * 56 / 100).max(2) * grow as i32 / 1024;
                f.push(Cmd::Fill {
                    r: clip(x - 10, i32::from(g.h) - 12 - bh, 20, bh),
                    color: RAIN_BAR,
                });
            }
        }
        // Times every two hours.
        for i in (0..GRAPH_HOURS).step_by(2) {
            let t = if i == 0 {
                Line::new().push(b"Now")
            } else {
                rg.time(shown[i].hour, 0, false)
            };
            let x = (hour_x(i as u8) + HOUR_W / 2)
                .saturating_sub(label.width(t.as_bytes()) / 2)
                .max(g.x + 8);
            f.text(x, y0(26), label, DIM, t.as_bytes());
        }
        // The line's turns labelled: the first hour, each high and low, the
        // last (as the mock-up's 68, 65, 71, 62).
        for (i, h) in shown.iter().enumerate() {
            let prev = if i == 0 {
                None
            } else {
                Some(shown[i - 1].temp)
            };
            let next = shown.get(i + 1).map(|n| n.temp);
            let high = prev.is_none_or(|p| h.temp >= p) && next.is_none_or(|n| h.temp > n);
            let low = prev.is_none_or(|p| h.temp <= p) && next.is_none_or(|n| h.temp < n);
            let edge = i == 0 || i + 1 == GRAPH_HOURS;
            if !(edge || high || low) || (i > 0 && i + 2 == GRAPH_HOURS) {
                continue;
            }
            let t = rg.temp(h.temp);
            let w = ui.width(t.as_bytes());
            let cx = hour_x(i as u8) + HOUR_W / 2;
            let x = cx.saturating_sub(w / 2).clamp(g.x + 6, g.x + g.w - 6 - w);
            let y = level(h.temp);
            let y = if low && !high { y + 28 } else { y - 10 };
            f.text(x, y0(y), ui, TEXT, t.as_bytes());
        }
        // The mock-up's lower-case label, after the bars.
        let rc = b"rain chance";
        f.text(
            g.x + g.w - 10 - label.width(rc),
            y0(i32::from(g.h) - 12),
            label,
            RAIN,
            rc,
        );
        // The hour picked: a line through the graph.
        if usize::from(self.hour_pick) < GRAPH_HOURS {
            let pick = usize::from(self.hour_pick);
            f.push(Cmd::Fill {
                r: clip(
                    hour_x(pick as u8) + HOUR_W / 2 - 1,
                    40,
                    2,
                    i32::from(g.h) - 52,
                ),
                color: FOCUS,
            });
        }
    }

    /// The open chart's line painted into `px`, a picture of
    /// [`super::chart::CHART_PIC`] (nothing with no chart open).
    pub fn paint_chart_pic(&self, px: &mut [u32]) {
        if let Some(c) = self.chart {
            self.paint_chart(px, c);
        }
    }

    /// Hourly's line and band painted per pixel into `px`, a see-through
    /// picture of [`GRAPH_PIC`]: for every column the smooth curve through
    /// the hours (Catmull-Rom, continued on its own slope to the panel's
    /// sides), the band's frost from it down, and over it a 3 px line
    /// anti-aliased by its distance from the curve (the owner, 2026-10-31:
    /// "give me a smooth line"). Painted once a forecast (the key:
    /// [`Shell::graph_key`]); drawn over the panel's glass it is exactly
    /// what painting over the glass gives ([`over`] composes).
    pub fn paint_graph(&self, px: &mut [u32]) {
        let stride = usize::from(GRAPH_PIC.0);
        let rows = (0, i32::from(GRAPH_PIC.1));
        let shown = &self.wx.hours[..GRAPH_HOURS];
        let (lo, hi) = shown.iter().fold((i32::MAX, i32::MIN), |(a, b), h| {
            (a.min(i32::from(h.temp)), b.max(i32::from(h.temp)))
        });
        let g = HOURLY_GRAPH;
        // The picture's first column is the panel's first inner one.
        let col0 = i64::from(g.x) + 1;
        let row0 = -i64::from(GRAPH_CUT);
        let (curve_top, span) = (58i64, 84i64);
        let base = curve_top + span + 18;
        let t = |i: i64| i64::from(shown[i.clamp(0, GRAPH_HOURS as i64 - 1) as usize].temp);
        // The temperature at hour position `u` (x 1024), on the curve, in
        // sixteenths of a tenth: whole tenths flattened the line's ends (a
        // slope taken over 1/16 hour rounded to nothing, emu-today
        // 2026-10-31) and stepped its thickness.
        let last = (GRAPH_HOURS as i64 - 1) * 1024;
        let curve = |u: i64| {
            let i = (u / 1024).min(GRAPH_HOURS as i64 - 2);
            let s = u - i * 1024;
            let (p0, p1, p2, p3) = (t(i - 1), t(i), t(i + 1), t(i + 2));
            let (s2, s3) = (s * s / 1024, s * s / 1024 * s / 1024);
            (2048 * p1
                + (p2 - p0) * s
                + (2 * p0 - 5 * p1 + 4 * p2 - p3) * s2
                + (3 * p1 - p0 - 3 * p2 + p3) * s3)
                / 128
        };
        let temp_at = |u: i64| -> i64 {
            // Past the ends, on the curve's own tangent there: the end
            // hours repeat, so it is half the step to the next hour.
            if u < 0 {
                curve(0) + (t(1) - t(0)) * u / 128
            } else if u > last {
                curve(last) + (t(last / 1024) - t(last / 1024 - 1)) * (u - last) / 128
            } else {
                curve(u)
            }
        };
        // The line's centre at temperature `v` (tenths x 16), local rows
        // x 256.
        let range = (i64::from(hi) - i64::from(lo)).max(1) * 16;
        let y_of =
            |v: i64| (curve_top + span) * 256 - span * 256 * (v - 16 * i64::from(lo)) / range;
        let x_mid = i64::from(hour_x(0) + HOUR_W / 2);
        for x in i64::from(g.x) + 1..i64::from(g.x + g.w) - 1 {
            let u = (x - x_mid) * 1024 / i64::from(HOUR_W);
            let c = y_of(temp_at(u));
            // The slope, for the line's thickness down a steep stretch.
            let dc = (y_of(temp_at(u + 1024 / i64::from(HOUR_W))) - c).abs();
            let half = 384 * i64::from(isqrt(65_536 + (dc * dc / 256) as i32)) / 256;
            let top = (c - half - 256) / 256;
            for ly in top.max(0)..base {
                let r = row0 + ly;
                if r < i64::from(rows.0) || r >= i64::from(rows.1) {
                    continue;
                }
                let at = r as usize * stride + (x - col0) as usize;
                let Some(p) = px.get_mut(at) else { continue };
                let d = ly * 256 + 128 - c;
                if d > 0 {
                    *p = over(*p, FROST);
                }
                let cover = (half + 128 - d.abs()).clamp(0, 256) as u32;
                if cover > 0 {
                    let a = cover * 255 / 256;
                    *p = over(*p, (a << 24) | (CURVE & 0x00FF_FFFF));
                }
            }
        }
    }

    /// Hourly's card: the hour picked, its picture and what it will be like.
    fn hourly_card(&self, f: &mut Frame) {
        let (ui, label) = (self.fonts.ui, self.fonts.label);
        let (wx, rg) = (&self.wx, &self.region);
        let dy = -self.view_off();
        let pick = usize::from(self.hour_pick).min(HOURS - 1);
        let c = HOURLY_CARD;
        let cy = i32::from(c.y) + dy;
        self.glass_at(f, c.x, cy, c.w, c.h);
        let h = wx.hours[pick];
        let when = if pick == 0 {
            Line::new().push(b"Now")
        } else {
            rg.time(h.hour, 0, false)
        };
        let cy0 = |v: i32| (v + cy).max(0) as u16;
        f.text(c.x + 16, cy0(30), label, TEXT, when.as_bytes());
        // The hour's picture, up to 160 px wide (the owner's pick of three
        // drawn sizes, 2026-10-31: docs/design/mockups/hourly-card-picture-
        // sizes.png), centred in the column left of the lines and below the
        // time: its own size, else the card's until that is decoded.
        let icon = Icon::of(h.sky);
        let k = usize::from(icon.id() - 1);
        if let Some(pic) = self.icons.mid[k].or(self.icons.card[k]) {
            f.push(Cmd::Pic {
                x: c.x + (HOUR_TEXT_X / 2).saturating_sub(pic.w / 2),
                y: (cy0(110 - i32::from(pic.h / 2))).cast_signed(),
                pic,
            });
        }
        let us = rg.temp == rp_weather::Temp::F;
        let unit: &[u8] = match rg.wind {
            rp_weather::Wind::Mph => b" mph ",
            rp_weather::Wind::Kmh => b" km/h ",
            rp_weather::Wind::Ms => b" m/s ",
        };
        // Rain: hundredths of an inch, or tenths of a mm.
        let rain = if us {
            let h100 = (u32::from(h.rain) * 100 + 127) / 254;
            Line::new()
                .num((h100 / 100) as i32)
                .push(b".")
                .push(&[b'0' + (h100 % 100 / 10) as u8, b'0' + (h100 % 10) as u8])
                .push(b" in")
        } else {
            Line::new()
                .num(i32::from(h.rain / 10))
                .push(b".")
                .num(i32::from(h.rain % 10))
                .push(b" mm")
        };
        let vis = if us {
            Line::new()
                .num((u32::from(h.vis) * 6214 / 100_000) as i32)
                .push(b" mi")
        } else {
            Line::new().num(i32::from(h.vis / 10)).push(b" km")
        };
        // Each line's two parts, the mock-up's " · " between them.
        let lines: [(Line, Line); 5] = [
            (
                rg.temp(h.temp)
                    .push(b", feels ")
                    .push(rg.temp(h.feels).as_bytes()),
                Line::new().push(icon.words().as_bytes()),
            ),
            (
                Line::new()
                    .push(b"Rain ")
                    .num(i32::from(h.precip))
                    .push(b"%"),
                rain,
            ),
            (
                Line::new()
                    .push(b"Wind ")
                    .num(rg.wind_speed(h.sky.wind))
                    .push(unit)
                    .push(rp_weather::compass(h.wind_from))
                    .push(b", gusts ")
                    .num(rg.wind_speed(h.gust)),
                Line::new(),
            ),
            (
                Line::new()
                    .push(b"Humidity ")
                    .num(i32::from(h.humidity))
                    .push(b"%"),
                Line::new()
                    .push(b"dew point ")
                    .push(rg.temp(h.dew).as_bytes()),
            ),
            (
                Line::new().push(b"UV ").num(i32::from(h.uv)),
                Line::new().push(b"visibility ").push(vis.as_bytes()),
            ),
        ];
        for (k, (a, b)) in lines.iter().enumerate() {
            let color = if k == 4 { DIM } else { TEXT };
            let mut l = *a;
            if !b.as_bytes().is_empty() {
                l = l.push(SEP).push(b.as_bytes());
            }
            f.text(
                c.x + HOUR_TEXT_X,
                cy0(58 + 29 * k as i32),
                ui,
                color,
                l.as_bytes(),
            );
        }
    }

    /// The next 24 hours under the card: four rows of six, each hour's
    /// time, picture, temperature and chance of rain (the owner,
    /// 2026-10-31: hour by hour below the graph, not 10 Day's bars).
    fn hourly_grid(&self, f: &mut Frame, dy: i32, (top, bottom): (i32, i32)) {
        let (ui, label) = (self.fonts.ui, self.fonts.label);
        let (wx, rg) = (&self.wx, &self.region);
        let ly = i32::from(GRID_Y) - 10 + dy;
        if ly > top && ly - 14 < bottom {
            f.text(22, ly.max(0) as u16, label, TEXT, b"NEXT 24 HOURS");
        }
        for r in 0..GRID_ROWS {
            let y = i32::from(GRID_Y) + r as i32 * i32::from(GRID_STEP) + dy;
            if y + i32::from(GRID_H) <= top || y >= bottom {
                continue;
            }
            self.glass_at(f, 16, y, 448, GRID_H);
            for c in 0..6usize {
                let i = r * 6 + c;
                let Some(h) = wx.hours.get(i) else { continue };
                let x = 16 + c as u16 * CELL_W;
                let cy = |v: i32| (y + v).max(0) as u16;
                if usize::from(self.hour_pick) == i {
                    f.push(Cmd::Outline {
                        r: Rect::new(x + 2, cy(2), CELL_W - 4, GRID_H - 4),
                        color: FOCUS,
                    });
                }
                let when = if i == 0 {
                    Line::new().push(b"NOW")
                } else {
                    rg.time(h.hour, 0, false)
                };
                let cx = x + CELL_W / 2;
                f.text(
                    cx.saturating_sub(label.width(when.as_bytes()) / 2),
                    cy(18),
                    label,
                    DIM,
                    when.as_bytes(),
                );
                if let Some(pic) = self.icons.row[usize::from(Icon::of(h.sky).id() - 1)] {
                    f.push(Cmd::Pic {
                        x: cx.saturating_sub(pic.w / 2),
                        y: (cy(24 + i32::from(40u16.saturating_sub(pic.h) / 2))).cast_signed(),
                        pic,
                    });
                }
                let t = rg.temp(h.temp);
                f.text(
                    cx.saturating_sub(ui.width(t.as_bytes()) / 2),
                    cy(84),
                    ui,
                    TEXT,
                    t.as_bytes(),
                );
                // Every hour's chance, 0% too: a gap read as broken (the
                // owner, 2026-10-31, said three times).
                {
                    let p = Line::new().num(i32::from(h.precip)).push(b"%");
                    f.text(
                        cx.saturating_sub(label.width(p.as_bytes()) / 2),
                        cy(102),
                        label,
                        RAIN,
                        p.as_bytes(),
                    );
                }
            }
        }
    }

    /// The hour at screen (`x`, `y`) on Hourly's graph or grid, if any.
    pub(crate) fn hour_at(&self, x: u16, y: u16) -> Option<u8> {
        if self.tab != 1 || !self.on_page(x, y) {
            return None;
        }
        // In the page's own rows (it scrolls).
        let py = i32::from(y) + self.view_off();
        let g = HOURLY_GRAPH;
        if (i32::from(g.y)..i32::from(g.y + g.h)).contains(&py) && (g.x..g.x + g.w).contains(&x) {
            let i = (x.saturating_sub(GRAPH_X0) / HOUR_W).min(GRAPH_HOURS as u16 - 1);
            return Some(i as u8);
        }
        let gy = py - i32::from(GRID_Y);
        if gy >= 0 && (16..464).contains(&x) {
            let (r, inside) = (gy / i32::from(GRID_STEP), gy % i32::from(GRID_STEP));
            let c = (x - 16) / CELL_W;
            if (r as usize) < GRID_ROWS && inside < i32::from(GRID_H) && c < 6 {
                return Some((r as usize * 6 + usize::from(c)) as u8);
            }
        }
        None
    }

    /// Where the Hourly list is: a thin bar at the right edge (none since
    /// the page fits).
    fn hourly_thumb(&self, f: &mut Frame) {
        if max_scroll(1) == 0 {
            return;
        }
        let track = THUMB_TRACK;
        f.push(Cmd::Fill {
            r: track,
            color: TRACK,
        });
        let max = i32::from(max_scroll(1)).max(1);
        let thumb_h = track.h * ROWS as u16 / HOURS as u16;
        let at = self.scroll.offset().clamp(0, max);
        let y = track.y + ((i32::from(track.h - thumb_h) * at) / max) as u16;
        f.push(Cmd::Fill {
            r: Rect::new(track.x, y, 3, thumb_h),
            color: FOCUS,
        });
    }

    /// Moves the radar loop on to `now`: true if its frame changed.
    pub(crate) fn radar_step(&mut self, now: u32) -> bool {
        // The real radar holds its newest frame until every frame is in
        // (R177: the loop played what had come, the bare map between).
        let held = self.radar_real().is_some() && !self.radar_complete();
        if self.screen != super::Screen::Weather || self.tab != RADAR || !self.radar_play || held {
            self.radar_at = now;
            return false;
        }
        let last = self.radar_frames() - 1;
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
        // The real rain is the device's to redraw (its wallpaper bands, only
        // where the rain changed: R184 step 2); here only the timeline. The
        // sample's cells are page commands: the page.
        self.dirty(if self.radar_real().is_some() {
            TIME_PANEL
        } else {
            PAGE
        });
        true
    }

    /// Whether the radar's map is under the page, and in which look (Auto:
    /// Paper between sunrise and sunset, Night after). The caller puts the
    /// map, coloured in that look, into the wallpaper under the page while
    /// it is.
    #[must_use]
    pub fn backdrop(&self) -> Option<rp_weather::maplayers::Look> {
        if self.screen != super::Screen::Weather || self.tab != RADAR {
            return None;
        }
        let m = u16::from(self.time.hour) * 60 + u16::from(self.time.minute);
        Some(
            self.wset
                .radar
                .look(m >= self.wx.sun.0 && m < self.wx.sun.1),
        )
    }

    /// The radar: the rain cells over the map (the backdrop), a chip naming
    /// the layer, and the timeline with play and pause.
    fn radar_page(&self, f: &mut Frame) {
        use rp_weather::radar::{NOW_FRAME, STEP_MIN, STEPS, TRACKS};
        let (ui, label) = (self.fonts.ui, self.fonts.label);
        let real = self.radar_real().is_some();
        let n = self.radar_frames();
        let now_frame = if real { n - 1 } else { NOW_FRAME };
        // The sample's cells only while there is no real radar.
        for t in TRACKS.iter().filter(|_| !real) {
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
        // The real radar's sources, small over the map's corner: the rain's
        // (RainViewer) and the map's (ours from Natural Earth, public
        // domain, and the open elevation tiles, whose terms ask for
        // "Mapzen"); while loading or on the sample, a note says so.
        if real && !self.radar_loading() {
            let credit = b"Natural Earth, Mapzen, RainViewer";
            let w = label.width(credit);
            f.text(W - 12 - w, TIME_PANEL.y - 8, label, DIM, credit);
        } else {
            let chip = Rect::new(16, 80, 300, 40);
            // The menus' denser glass: readable over the map.
            self.glass(f, chip, super::MENU_LEFT, super::MENU_RIGHT);
            let what: &[u8] = if self.radar_loading() {
                b"RADAR LOADING"
            } else {
                b"RAIN  SAMPLE"
            };
            f.text(chip.x + 14, chip.y + 27, label, TEXT, what);
        }
        // You are here: a round dot, white-edged, at the view's centre
        // (the view is centred on the place shown).
        if real {
            let (cx, cy) = (W / 2, PAGE.y + rp_weather::tiles::VIEW_H as u16 / 2);
            for (r, color) in [
                (Rect::new(cx - 4, cy - 7, 8, 14), TEXT),
                (Rect::new(cx - 7, cy - 4, 14, 8), TEXT),
                (Rect::new(cx - 2, cy - 5, 4, 10), FOCUS),
                (Rect::new(cx - 5, cy - 2, 10, 4), FOCUS),
            ] {
                f.push(Cmd::Fill { r, color });
            }
        }
        self.glass(f, TIME_PANEL, super::MENU_LEFT, super::MENU_RIGHT);
        // Play or pause: a smooth triangle (one picture made at build
        // time), or two bars.
        self.press_under(f, Target::Play);
        let (px, py) = (PLAY.x + 20, PLAY.y + 20);
        if self.radar_play {
            f.push(Cmd::Image {
                x: px,
                y: py,
                img: &PLAY_ICON,
            });
        } else {
            for dx in [0u16, 12] {
                f.push(Cmd::Fill {
                    r: Rect::new(px + dx, py, 6, 28),
                    color: TEXT,
                });
            }
        }
        self.decorate(f, Target::Play);
        // Play waits for the frames: dimmed by the panel's own glass until
        // they are all in.
        let waiting = real && !self.radar_complete();
        if waiting {
            f.push(Cmd::Fill {
                r: Rect::new(px - 4, py - 4, 34, 36),
                color: 0xB000_0000 | (super::MENU_LEFT & 0xFF_FFFF),
            });
        }
        // The frame's time and whether it is past or forecast.
        let k = i16::from(self.radar_frame) - i16::from(now_frame);
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
        // Real frames: how long ago each was taken.
        let when = if real {
            let ago = self.radar_times[usize::from(n - 1)]
                .saturating_sub(self.radar_times[usize::from(self.radar_frame.min(n - 1))])
                / 60;
            if ago == 0 {
                Line::new().push(b"latest")
            } else {
                Line::new().num(ago as i32).push(b" min ago")
            }
        } else {
            when
        };
        f.text(TRACK_X, TIME_PANEL.y + 28, ui, TEXT, when.as_bytes());
        // The span the frames cover (two hours from RainViewer: R184 step
        // 2), or how many are in while they come.
        let span = if real {
            let have = self.radar_have().count_ones();
            if waiting {
                Line::new()
                    .num(have as i32)
                    .push(b" of ")
                    .num(i32::from(n))
                    .push(b" in")
            } else {
                let m = self.radar_times[usize::from(n - 1)]
                    .saturating_sub(self.radar_times[0])
                    .div_ceil(60);
                let past = Line::new().push(b"past ");
                if m >= 60 && m.is_multiple_of(60) {
                    past.num((m / 60) as i32)
                        .push(if m == 60 { b" hour" } else { b" hours" })
                } else {
                    past.num(m as i32).push(b" min")
                }
            }
        } else {
            Line::new().push(b"-1 h       +1 h")
        };
        let ends = span.as_bytes();
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
        // A tick per frame; on the real radar a frame not yet in is a
        // short stub, so the ticks fill in as they land.
        for i in 0..n {
            let missing = real && self.radar_have() & 1 << i == 0;
            let h = match (i == now_frame, missing) {
                (_, true) => 2,
                (true, false) => 12,
                (false, false) => 6,
            };
            f.push(Cmd::Fill {
                r: Rect::new(frame_x(i, n), ty + 2 - h / 2, 2, h),
                color: if i == now_frame { WARM } else { DIM },
            });
        }
        f.push(Cmd::Fill {
            r: Rect::new(
                frame_x(self.radar_frame, n).saturating_sub(5),
                ty - 7,
                12,
                18,
            ),
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

    pub(crate) fn days_page(&self, f: &mut Frame) {
        const NAMES: [&[u8]; 7] = [b"MON", b"TUE", b"WED", b"THU", b"FRI", b"SAT", b"SUN"];
        let (ui, label) = (self.fonts.ui, self.fonts.label);
        let (wx, rg) = (&self.wx, &self.region);
        let (lo, hi) = wx.range();
        for (k, d) in wx.days.iter().enumerate().take(DAYS.min(ROWS)) {
            let e = self.ease(k as u32);
            let y = self.sy(row(k).y);
            let (top, bottom) = self.view_rows();
            if y + i32::from(ROW_H) <= top || y >= bottom {
                continue;
            }
            let r = Rect::new(16 + self.slide(k as u32), y.max(0) as u16, 448, ROW_H);
            self.glass_at(f, r.x, y, r.w, r.h);
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
            if let Some(pic) = self.icons.row[usize::from(icon.id() - 1)] {
                f.push(Cmd::Pic {
                    x: r.x + 90 + (40 - pic.w.min(40)) / 2,
                    y: (r.y + (r.h.saturating_sub(pic.h)) / 2).cast_signed(),
                    pic,
                });
            }
            // Every day's chance of rain, 0% too: rows without one looked
            // broken (the owner, 2026-10-31, said three times).
            let p = Line::new().num(i32::from(d.precip)).push(b"%");
            f.text(r.x + 138, r.y + 28, ui, RAIN, p.as_bytes());
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
