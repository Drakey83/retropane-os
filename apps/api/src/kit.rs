//! The shared app kit (design-language.md version 2): every look an app
//! needs is one call here, so five apps keep one look.
//!
//! Everything is drawn with the renderer's own commands: panels from their
//! pre-made art, colour as fills inside them, round things (avatars, the
//! tick, dots) as pictures painted once into the picture store and then
//! only placed. Nothing paints pixels per frame.

use rp_draw::look::theme;
use rp_draw::pics::{Pic, Pics};
use rp_draw::{Argb, Cmd, Font, Frame, Rect};

use crate::{Ctx, MARGIN, W, WIDE};

/// Text.
pub const TEXT: Argb = theme(0xFFF0_F2F4);
/// Secondary text: labels, hints, times.
pub const DIM: Argb = theme(0xFFCC_D3DA);
/// The system accent: focus, the caret.
pub const FOCUS: Argb = theme(0xFF7F_D0FF);
/// Done, a ticked box.
pub const DONE: Argb = theme(0xFF81_C08B);
/// Late, delete.
pub const LATE: Argb = theme(0xFFD9_9086);
/// Warning.
pub const AMBER: Argb = theme(0xFFD2_B37B);

/// The app tints (design-language.md "App tints").
pub mod tint {
    use super::{Argb, theme};
    /// Calendar.
    pub const CALENDAR: Argb = theme(0xFFD9_907E);
    /// Contacts.
    pub const CONTACTS: Argb = theme(0xFF74_B1A7);
    /// Tasks.
    pub const TASKS: Argb = theme(0xFF74_9DD3);
    /// Notes.
    pub const NOTES: Argb = theme(0xFFD2_B37B);
    /// The calculator's operators.
    pub const CALCULATOR: Argb = theme(0xFFD6_A270);
    /// Weather (the shell's own app): the sky.
    pub const WEATHER: Argb = theme(0xFF7A_ADD3);
}

/// Every accent's light-look twins, (dark look, fill, text): its hue and
/// chroma kept in OKLCH, darkened until it reads on the light glass at
/// its darkest, 3:1 for a fill (a bar, a dot) and 4.5:1 for text. Written
/// by `tools/shell-assets/accents.py`, never by hand.
pub const LIGHT_TWINS: [(Argb, Argb, Argb); 17] = [
    (theme(0xFF81_C08B), theme(0xFF10_5223), theme(0xFF00_3310)),
    (theme(0xFFD9_9086), theme(0xFF73_342D), theme(0xFF54_1713)),
    (theme(0xFFD2_B37B), theme(0xFF5C_4100), theme(0xFF3C_2902)),
    (theme(0xFFD9_907E), theme(0xFF73_3425), theme(0xFF53_1708)),
    (theme(0xFF74_B1A7), theme(0xFF0C_5048), theme(0xFF00_322D)),
    (theme(0xFF74_9DD3), theme(0xFF22_4777), theme(0xFF05_2C5A)),
    (theme(0xFFD6_A270), theme(0xFF68_3C03), theme(0xFF44_2501)),
    (theme(0xFFA0_C07E), theme(0xFF33_4E0A), theme(0xFF1E_3201)),
    (theme(0xFF7A_ADD3), theme(0xFF18_4B6C), theme(0xFF01_2F4A)),
    (theme(0xFF78_86CD), theme(0xFF38_4182), theme(0xFF1F_2462)),
    (theme(0xFFA7_8ED4), theme(0xFF52_3A78), theme(0xFF38_1E5A)),
    (theme(0xFFD9_8BA6), theme(0xFF73_2F4A), theme(0xFF53_112F)),
    (theme(0xFF79_A6D5), theme(0xFF1E_4972), theme(0xFF00_2C52)),
    (theme(0xFFD5_A275), theme(0xFF69_3D0C), theme(0xFF44_2300)),
    (theme(0xFF8F_B4D2), theme(0xFF28_4963), theme(0xFF0C_2F47)),
    (theme(0xFF76_94BB), theme(0xFF2D_486B), theme(0xFF13_2D4D)),
    (theme(0xFFCC_AA7E), theme(0xFF5E_4115), theme(0xFF40_2700)),
];

/// [`LIGHT_TWINS`] for the window's white glass (design-language.md
/// version 3): the same rule against a far lighter glass, so an accent
/// keeps its colour there. Written by `tools/shell-assets/accents.py`.
pub const WINDOW_TWINS: [(Argb, Argb, Argb); 17] = [
    (theme(0xFF81_C08B), theme(0xFF55_9260), theme(0xFF37_7444)),
    (theme(0xFFD9_9086), theme(0xFFB8_7268), theme(0xFF99_564E)),
    (theme(0xFFD2_B37B), theme(0xFF9E_8049), theme(0xFF81_642D)),
    (theme(0xFFD9_907E), theme(0xFFB9_7362), theme(0xFF99_5646)),
    (theme(0xFF74_B1A7), theme(0xFF53_8F85), theme(0xFF34_7168)),
    (theme(0xFF74_9DD3), theme(0xFF5F_87BC), theme(0xFF43_6A9C)),
    (theme(0xFFD6_A270), theme(0xFFAC_7A49), theme(0xFF8E_5E2D)),
    (theme(0xFFA0_C07E), theme(0xFF70_8E4E), theme(0xFF54_7031)),
    (theme(0xFF7A_ADD3), theme(0xFF58_89AE), theme(0xFF3C_6D90)),
    (theme(0xFF78_86CD), theme(0xFF72_80C6), theme(0xFF56_62A6)),
    (theme(0xFFA7_8ED4), theme(0xFF92_79BD), theme(0xFF74_5B9D)),
    (theme(0xFFD9_8BA6), theme(0xFFB9_6E89), theme(0xFF99_516C)),
    (theme(0xFF79_A6D5), theme(0xFF5B_87B5), theme(0xFF40_6B97)),
    (theme(0xFFD5_A275), theme(0xFFAB_7A4E), theme(0xFF8D_5E33)),
    (theme(0xFF8F_B4D2), theme(0xFF65_89A5), theme(0xFF49_6B87)),
    (theme(0xFF76_94BB), theme(0xFF69_86AD), theme(0xFF4E_6A8F)),
    (theme(0xFFCC_AA7E), theme(0xFFA0_7F54), theme(0xFF82_6339)),
];

/// An accent's twin on the light look's window glass ([`WINDOW_TWINS`]).
#[must_use]
pub fn window_twin(c: Argb, text: bool) -> Option<Argb> {
    let rgb = c & 0x00FF_FFFF;
    WINDOW_TWINS
        .iter()
        .find(|t| t.0 & 0x00FF_FFFF == rgb)
        .map(|t| (c & 0xFF00_0000) | (if text { t.2 } else { t.1 } & 0x00FF_FFFF))
}

/// An accent's light-look twin (`text` for lettering), its alpha kept;
/// `None` for a colour that isn't one of the accents.
#[must_use]
pub fn light_twin(c: Argb, text: bool) -> Option<Argb> {
    let rgb = c & 0x00FF_FFFF;
    LIGHT_TWINS
        .iter()
        .find(|t| t.0 & 0x00FF_FFFF == rgb)
        .map(|t| (c & 0xFF00_0000) | (if text { t.2 } else { t.1 } & 0x00FF_FFFF))
}

/// A colour that stays itself in every look (paper, ink on it): the
/// shell's light look maps a program's colours (white text to dark, glass
/// to frosted) but leaves a kept one alone. Marked in its alpha's low
/// nibble, 0xD (no other colour in the system has it: a change of at most
/// 13/255 in opacity, unseen), so translucent ones can be kept too.
#[must_use]
pub const fn keep(c: Argb) -> Argb {
    (c & 0xF0FF_FFFF) | 0x0D00_0000
}

/// Whether `c` is [`keep`]'s.
#[must_use]
pub const fn kept(c: Argb) -> bool {
    (c >> 24) & 0xF == 0xD
}

/// The eight identity colours (calendars, lists, notes, avatars): coral,
/// amber, lime, teal, sky, indigo, violet, rose.
pub const IDENTITY: [Argb; 8] = [
    theme(0xFFD9_907E),
    theme(0xFFD2_B37B),
    theme(0xFFA0_C07E),
    theme(0xFF74_B1A7),
    theme(0xFF7A_ADD3),
    theme(0xFF78_86CD),
    theme(0xFFA7_8ED4),
    theme(0xFFD9_8BA6),
];

/// A name's identity colour: a hash of its bytes, so it never changes.
#[must_use]
pub fn colour_of(name: &[u8]) -> Argb {
    // FNV-1a: well spread for short names, a few instructions a byte.
    let h = name.iter().fold(0x811C_9DC5_u32, |h, &b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    });
    IDENTITY[(h >> 29) as usize]
}

/// `c` at `alpha` (0-255) of its own.
#[must_use]
pub const fn alpha(c: Argb, alpha: u8) -> Argb {
    (c & 0x00FF_FFFF) | ((alpha as u32) << 24)
}

/// Every size of picture an app program may ask the picture store for:
/// small marks, row avatars, the card's big avatar, a note's ink panel and
/// a note card's miniature. One list, built into the device's store and
/// the PC model's alike (2026-11-14: Notes' ink sizes were the model's
/// only, so the X51v drew no ink once the pen lifted and every check
/// passed). An app that needs a new size adds it here.
pub const PIC_CLASSES: [(u16, u16, u8); 6] = [
    (24, 24, 24),
    (40, 40, 24),
    // The big avatars of a favourites row and small dials.
    (64, 64, 8),
    (96, 96, 2),
    (INK_PIC.0, INK_PIC.1, 1),
    (INK_MINI.0, INK_MINI.1, 4),
];

/// A note's ink panel's picture (Notes).
pub const INK_PIC: (u16, u16) = (448, 196);
/// A note card's ink miniature (Notes).
pub const INK_MINI: (u16, u16) = (212, 84);

/// A glass panel.
pub fn panel(f: &mut Frame, cx: &Ctx, r: Rect) {
    f.push(Cmd::Panel {
        r,
        art: cx.look.panel,
    });
}

/// A glass panel under the pen.
pub fn pressed(f: &mut Frame, cx: &Ctx, r: Rect) {
    f.push(Cmd::Panel {
        r,
        art: cx.look.pressed,
    });
}

/// `top` laid over `base` at `strength` (0-255), as one opaque colour.
#[must_use]
pub const fn mix(base: Argb, top: Argb, strength: u8) -> Argb {
    const fn ch(base: Argb, top: Argb, s: u32, shift: u32) -> u32 {
        let b = (base >> shift) & 0xFF;
        let t = (top >> shift) & 0xFF;
        ((b * (255 - s) + t * s) / 255) << shift
    }
    let s = strength as u32;
    0xFF00_0000 | ch(base, top, s, 16) | ch(base, top, s, 8) | ch(base, top, s, 0)
}

/// The rounded shapes' corner radius.
pub const CORNER: u16 = 12;

/// The dark glass of a shape with no tint of its own.
pub const GLASS: Argb = 0xFF2A_3038;

/// A rounded rectangle in one opaque colour: two fills crossing and a disc
/// picture at each corner (painted once per colour, `Shapes`), so the
/// shape is one colour edge to edge with anti-aliased corners.
///
/// Version 2's first chips and cards laid a tint inside the glass panel's
/// art: the chosen chip read as a box on a button and the art's corners
/// showed as light squares at chip height (the owner, 2026-11-14). The
/// colour is opaque on purpose: the 2D engine lays a translucent fill over
/// the wallpaper itself (`rp_mbx_render`'s `Fill`), so a translucent body
/// and its corner pictures would blend twice where they meet.
pub fn round_rect(f: &mut Frame, cx: &Ctx, r: Rect, c: Argb) {
    // Opaque; a kept colour keeps its mark (paper is paper in both looks:
    // without it the light look's pass turned a sheet into grey glass).
    let c = if kept(c) { c } else { c | 0xFF00_0000 };
    let k = CORNER;
    let disc = cx.pics.and_then(|p| cx.shapes.disc(p, c));
    if r.w < 2 * k || r.h < 2 * k || disc.is_none() {
        f.push(Cmd::Fill { r, color: c });
        return;
    }
    f.push(Cmd::Fill {
        r: Rect::new(r.x + k, r.y, r.w - 2 * k, r.h),
        color: c,
    });
    f.push(Cmd::Fill {
        r: Rect::new(r.x, r.y + k, r.w, r.h - 2 * k),
        color: c,
    });
    if let Some(pic) = disc {
        let (x1, y1) = (r.x + r.w - 2 * k, r.y + r.h - 2 * k);
        for (x, y) in [(r.x, r.y), (x1, r.y), (r.x, y1), (x1, y1)] {
            f.push(Cmd::Pic {
                x,
                y: y as i16,
                pic,
            });
        }
    }
}

/// A card that belongs to something with a colour (a calendar, a list, a
/// note): the glass panel with the colour as an accent bar on its left
/// edge. Aero, not Metro (the owner, 2026-11-14): colour marks a thing, it
/// never floods a tile. `strength` above half makes the bar a little
/// wider, for the one card a screen wants to stand out.
pub fn tinted(f: &mut Frame, cx: &Ctx, r: Rect, c: Argb, strength: u8) {
    panel(f, cx, r);
    let w = if strength > 0x80 { 6 } else { 4 };
    if r.h > 16 {
        f.push(Cmd::Fill {
            r: Rect::new(r.x + 6, r.y + 7, w, r.h - 14),
            color: c | 0xFF00_0000,
        });
    }
}

/// An Aero glass key: at rest, under the pen, or the chosen one (the
/// blue glow).
pub fn key(f: &mut Frame, cx: &Ctx, r: Rect, state: KeyState) {
    let art = match state {
        KeyState::Rest => cx.look.key,
        KeyState::Down => cx.look.key_down,
        KeyState::On => cx.look.on,
    };
    f.push(Cmd::Panel { r, art });
}

/// How a key is drawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyState {
    /// At rest.
    Rest,
    /// Under the pen.
    Down,
    /// Chosen: the blue glow.
    On,
}

/// A row's colour bar: 4 px on its left edge.
pub fn colour_bar(f: &mut Frame, r: Rect, c: Argb) {
    f.push(Cmd::Fill {
        r: Rect::new(r.x + 6, r.y + 7, 4, r.h.saturating_sub(14)),
        color: c,
    });
}

/// Text from the pen at `x`, baseline `y`.
pub fn text(f: &mut Frame, x: u16, y: u16, font: &'static Font, c: Argb, s: &[u8]) {
    f.text(x, y, font, c, s);
}

/// Text ending at `right`.
pub fn right(f: &mut Frame, right: u16, y: u16, font: &'static Font, c: Argb, s: &[u8]) {
    let w = font.width(s);
    f.text(right.saturating_sub(w), y, font, c, s);
}

/// Text centred in `r` across, baseline `y`.
pub fn centred(f: &mut Frame, r: Rect, y: u16, font: &'static Font, c: Argb, s: &[u8]) {
    let w = font.width(s);
    f.text(r.x + r.w.saturating_sub(w) / 2, y, font, c, s);
}

/// Text cut to `max` px wide at a character, with "..", never shrunk.
pub fn fitted(f: &mut Frame, x: u16, y: u16, font: &'static Font, c: Argb, s: &[u8], max: u16) {
    if font.width(s) <= max {
        f.text(x, y, font, c, s);
        return;
    }
    let dots = font.width(b"..");
    let mut n = s.len();
    while n > 0 && font.width(&s[..n]) + dots > max {
        n -= 1;
    }
    let mut buf = [0u8; 96];
    let n = n.min(buf.len() - 2);
    buf[..n].copy_from_slice(&s[..n]);
    buf[n..n + 2].copy_from_slice(b"..");
    f.text(x, y, font, c, &buf[..n + 2]);
}

/// The app's backdrop: the wallpaper dimmed under the app's area, darker
/// at the top, so the content reads as an app over the picture
/// (design-language.md version 2: a busy photo behind 90%-glass rows made
/// the text hard to read on the owner's sunset).
pub fn backdrop(f: &mut Frame) {
    f.push(Cmd::Fill {
        r: crate::CONTENT,
        color: alpha(0xFF0A_0D12, 0x98),
    });
}

/// The large title band's height (design-language.md "The large title").
pub const TITLE_BAND: u16 = 76;

/// An app home's large title: `title` in 30 px with a tint glow line under
/// it, and the one-line live `summary`. Returns the y under the band.
pub fn large_title(
    f: &mut Frame,
    cx: &Ctx,
    top: u16,
    title: &[u8],
    summary: &[u8],
    tint: Argb,
) -> u16 {
    f.push(Cmd::HGradient {
        r: Rect::new(0, top, W, TITLE_BAND),
        left: alpha(0xFF0A_0E14, 0xB0),
        right: alpha(0xFF0A_0E14, 0x20),
    });
    f.push(Cmd::Fill {
        r: Rect::new(MARGIN, top + 8, 4, 30),
        color: tint,
    });
    text(f, MARGIN + 14, top + 37, cx.fonts.title, TEXT, title);
    text(f, MARGIN + 14, top + 64, cx.fonts.ui, DIM, summary);
    top + TITLE_BAND
}

/// A section label in spaced capitals with a hairline to the right.
pub fn section(f: &mut Frame, cx: &Ctx, y: u16, s: &[u8], c: Argb) {
    let w = cx.fonts.label.width(s);
    text(f, MARGIN + 4, y + 15, cx.fonts.label, c, s);
    f.push(Cmd::Fill {
        r: Rect::new(MARGIN + 12 + w, y + 9, WIDE.saturating_sub(16 + w), 1),
        color: alpha(c, 0x60),
    });
}

/// A row of chips across `r`: the chosen one filled in `tint`.
pub fn chips(f: &mut Frame, cx: &Ctx, r: Rect, labels: &[&[u8]], chosen: usize, tint: Argb) {
    for (i, l) in labels.iter().enumerate() {
        let c = chip(r, labels.len(), i);
        // Aero glass keys; the chosen one glows blue (XP and Vista's
        // selection), whatever the app's tint.
        let _ = tint;
        key(
            f,
            cx,
            c,
            if i == chosen {
                KeyState::On
            } else {
                KeyState::Rest
            },
        );
        centred(f, c, c.y + c.h / 2 + 6, cx.fonts.label, TEXT, l);
    }
}

/// Chip `i` of `n` across `r` ([`chips`]' layout: what a tap on one hits).
#[must_use]
pub fn chip(r: Rect, n: usize, i: usize) -> Rect {
    let n = n.max(1) as u16;
    let gap = 6;
    let cw = (r.w - gap * (n - 1)) / n;
    Rect::new(r.x + i as u16 * (cw + gap), r.y, cw, r.h)
}

/// An accent as a picture painted for this look should use it (pictures
/// aren't recoloured by the look's pass): the accent in the dark, its
/// window twin in the light.
#[must_use]
pub fn painted(cx: &Ctx, c: Argb) -> Argb {
    if cx.look.light {
        window_twin(c, false).unwrap_or(c)
    } else {
        c
    }
}

/// A gauge's track for this look: faint light on the dark glass, faint
/// slate on the white.
#[must_use]
pub fn track(cx: &Ctx) -> Argb {
    if cx.look.light {
        0x3040_5068
    } else {
        alpha(TEXT, 0x38)
    }
}

/// A ring gauge painted into `px` (`size` x `size`): a `track` all the way
/// round and `fill` over `part` of 256 of it, clockwise from the top, both
/// `thick` px wide, edges blended by coverage; a soft gloss on its upper
/// left (the Aero light). Design-language.md version 3: an app's numbers
/// are drawn, as Weather draws its dials, not only written.
pub fn paint_ring(px: &mut [Argb], size: u16, thick: u16, part: u16, fill: Argb, track: Argb) {
    let s = i32::from(size);
    let (outer, inner) = (s * 8 - 8, s * 8 - 8 - i32::from(thick) * 16);
    for y in 0..s {
        for x in 0..s {
            // The pixel's centre from the ring's, in 1/16 px.
            let (dx, dy) = (x * 16 + 8 - s * 8, y * 16 + 8 - s * 8);
            let d2 = dx * dx + dy * dy;
            // An integer square root (Newton, three steps from a good guess).
            let mut d = (dx.abs().max(dy.abs()) + dx.abs().min(dy.abs()) / 2).max(1);
            for _ in 0..3 {
                d = (d + d2 / d) / 2;
            }
            // Coverage: a pixel's width of blend at each edge.
            let cover = ((outer - d).clamp(0, 16) * (d - inner).clamp(0, 16)) as u32;
            if cover == 0 {
                px[(y * s + x) as usize] = 0;
                continue;
            }
            // The angle clockwise from the top, 0-1023 a turn: an octant's
            // angle is near enough its tangent (within 4 degrees: a gauge,
            // not a protractor).
            let (ax, ay) = (dx.abs(), dy.abs());
            let t = if ax.max(ay) == 0 {
                0
            } else {
                ax.min(ay) * 128 / ax.max(ay)
            };
            let in_quadrant = if ax < ay { t } else { 256 - t };
            let turn = match (dx >= 0, dy < 0) {
                (true, true) => in_quadrant,
                (true, false) => 512 - in_quadrant,
                (false, false) => 512 + in_quadrant,
                (false, true) => 1024 - in_quadrant,
            };
            let on = turn * 256 < i32::from(part) * 1024;
            let c = if on { fill } else { track };
            // Gloss: lighter toward the upper left.
            let gloss = ((-(dx + dy)).clamp(0, s * 8) * 60 / (s * 8)) as u8;
            let c = if on { mix(c, 0xFFFF_FFFF, gloss) } else { c };
            let a = (c >> 24) * cover / 256;
            px[(y * s + x) as usize] = (c & 0x00FF_FFFF) | (a << 24);
        }
    }
}

/// [`paint_ring`] into a new picture of the store.
pub fn ring(pics: &Pics, size: u16, thick: u16, part: u16, fill: Argb, track: Argb) -> Option<Pic> {
    let p = pics.alloc(size, size)?;
    pics.write(p, |px| paint_ring(px, size, thick, part, fill, track));
    Some(p)
}

/// A glossy disc (an avatar's, a colour well's): `fill` with the Aero
/// light on its upper half and a darker foot, edges blended by coverage.
pub fn paint_gloss_disc(px: &mut [Argb], size: u16, fill: Argb) {
    let s = i32::from(size);
    let r = s * 8 - 4;
    for y in 0..s {
        for x in 0..s {
            let (dx, dy) = (x * 16 + 8 - s * 8, y * 16 + 8 - s * 8);
            let d2 = dx * dx + dy * dy;
            let mut d = (dx.abs().max(dy.abs()) + dx.abs().min(dy.abs()) / 2).max(1);
            for _ in 0..3 {
                d = (d + d2 / d) / 2;
            }
            let cover = (r - d).clamp(0, 16) as u32;
            if cover == 0 {
                px[(y * s + x) as usize] = 0;
                continue;
            }
            // Top to foot: lit, the colour, shaded; a bright cap above the
            // middle (the gel highlight).
            let v = y * 256 / s;
            let mut c = if v < 128 {
                mix(fill, 0xFFFF_FFFF, ((128 - v) * 90 / 128) as u8)
            } else {
                mix(fill, 0xFF00_0000, ((v - 128) * 70 / 128) as u8)
            };
            let cap = (dx * dx / 2 + (dy + s * 4) * (dy + s * 4)) < (s * 5) * (s * 5) && dy < -s;
            if cap {
                c = mix(c, 0xFFFF_FFFF, 46);
            }
            // A thin dark rim.
            if r - d < 20 {
                c = mix(c, 0xFF00_0000, 60);
            }
            px[(y * s + x) as usize] = (c & 0x00FF_FFFF) | ((0xFF * cover / 16) << 24);
        }
    }
}

/// [`paint_gloss_disc`] into a new picture of the store.
pub fn gloss_disc(pics: &Pics, size: u16, fill: Argb) -> Option<Pic> {
    let p = pics.alloc(size, size)?;
    pics.write(p, |px| paint_gloss_disc(px, size, fill));
    Some(p)
}

/// What a tap can hit, recorded as a screen is drawn (the owner,
/// 2026-11-14: "everything needs to be clickable via the stylus"): each
/// thing drawn that looks tappable adds its rectangle, so nothing drawn is
/// left without a target and the tap code never repeats the layout. The
/// last added wins (drawn on top).
pub struct Hits<T: Copy, const N: usize> {
    at: [(Rect, Option<T>); N],
    n: usize,
}

impl<T: Copy, const N: usize> Default for Hits<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy, const N: usize> Hits<T, N> {
    /// None yet.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            at: [(Rect::new(0, 0, 0, 0), None); N],
            n: 0,
        }
    }

    /// Forgets the last drawing's (at the start of a draw).
    pub fn clear(&mut self) {
        self.n = 0;
    }

    /// `r` taps as `t` (past `N` the rest are dropped; size `N` for the
    /// busiest screen, a test checks it).
    pub fn add(&mut self, r: Rect, t: T) {
        if self.n < N {
            self.at[self.n] = (r, Some(t));
            self.n += 1;
        }
    }

    /// What a tap at (`x`, `y`) hits: the last added there.
    #[must_use]
    pub fn at(&self, x: u16, y: u16) -> Option<T> {
        self.at[..self.n]
            .iter()
            .rev()
            .find(|(r, _)| r.contains_point(x, y))
            .and_then(|(_, t)| *t)
    }

    /// Where `t` was drawn (the last added), if it was.
    #[must_use]
    pub fn find(&self, t: T) -> Option<Rect>
    where
        T: PartialEq,
    {
        self.at[..self.n]
            .iter()
            .rev()
            .find(|(_, h)| *h == Some(t))
            .map(|(r, _)| *r)
    }

    /// How many were added (tests: under `N`).
    #[must_use]
    pub const fn len(&self) -> usize {
        self.n
    }

    /// Whether none were.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.n == 0
    }
}

/// The 4-bit-free antialiased disc: `px` is `size` x `size` ARGB, filled
/// with `fill` inside the circle (and a `ring` of 2 px if given),
/// transparent outside, edges blended by coverage.
pub fn paint_disc(px: &mut [Argb], size: u16, fill: Argb, ring: Option<Argb>) {
    paint_disc_as(px, size, fill, ring, true);
}

/// A flat disc, no highlight: the rounded corners of a flat shape (a
/// sheet of paper, a bar) are quarters of one (the orb's gloss showed as
/// an arc in every corner, 2026-11-14).
pub fn flat_disc(pics: &Pics, size: u16, fill: Argb) -> Option<Pic> {
    let p = pics.alloc(size, size)?;
    pics.write(p, |px| paint_disc_as(px, size, fill, None, false));
    Some(p)
}

fn paint_disc_as(px: &mut [Argb], size: u16, fill: Argb, ring: Option<Argb>, orb: bool) {
    let s = i32::from(size);
    // Radius and the ring's inner radius, in 1/16 px.
    let r = s * 8;
    let inner = r - 32;
    for y in 0..s {
        for x in 0..s {
            // Coverage from a 4 x 4 grid of samples in the pixel.
            let mut cover = 0u32;
            let mut ring_cover = 0u32;
            for sy in 0..4 {
                for sx in 0..4 {
                    let dx = (x * 16 + sx * 4 + 2) - r;
                    let dy = (y * 16 + sy * 4 + 2) - r;
                    let d2 = dx * dx + dy * dy;
                    if d2 <= r * r {
                        cover += 1;
                        if ring.is_some() && d2 > inner * inner {
                            ring_cover += 1;
                        }
                    }
                }
            }
            let i = (y * s + x) as usize;
            px[i] = if cover == 0 {
                0
            } else {
                let base = if ring_cover * 2 >= cover {
                    ring.unwrap_or(fill)
                } else {
                    fill
                };
                // The Aero orb: a soft highlight over the upper part, an
                // ellipse lit from above, and the lower edge a shade deeper.
                let lit = if orb && ring.is_none() {
                    let (hx, hy) = (x * 2 - s, y * 4 - s);
                    let d = (hx * hx + hy * hy) as u32;
                    let r2 = (s * s) as u32;
                    if y * 2 < s && d < r2 {
                        110 - 110 * d / r2
                    } else {
                        0
                    }
                } else {
                    0
                };
                let shade = if orb && ring.is_none() && y * 4 > s * 3 {
                    24
                } else {
                    0
                };
                let ch = |sh: u32| {
                    let v = (base >> sh) & 0xFF;
                    let v = v + (255 - v) * lit / 255;
                    (v * (255 - shade) / 255) << sh
                };
                let a = (base >> 24) * cover / 16;
                ch(16) | ch(8) | ch(0) | (a << 24)
            };
        }
    }
}

/// A disc picture of `size` (one of [`PIC_CLASSES`]) in `fill`, with an
/// optional ring: painted once, then only placed.
#[must_use]
pub fn disc(pics: &Pics, size: u16, fill: Argb, ring: Option<Argb>) -> Option<Pic> {
    let p = pics.alloc(size, size)?;
    pics.write(p, |px| paint_disc(px, size, fill, ring));
    Some(p)
}

/// The initials of a name: the first letter of its first two words, in
/// capitals ("Grace Hopper" is "GH").
#[must_use]
pub fn initials(name: &[u8]) -> ([u8; 2], usize) {
    let mut out = [0u8; 2];
    let mut n = 0;
    let mut start = true;
    for &b in name {
        if b == b' ' {
            start = true;
        } else if start {
            if n < 2 && b.is_ascii_alphanumeric() {
                out[n] = b.to_ascii_uppercase();
                n += 1;
            }
            start = false;
        }
    }
    (out, n)
}

/// An avatar: the disc picture `pic` (in the person's colour) at x, y with
/// the initials over it in `font`.
pub fn avatar(
    f: &mut Frame,
    pic: Option<Pic>,
    x: u16,
    y: u16,
    size: u16,
    font: &'static Font,
    name: &[u8],
) {
    if let Some(pic) = pic {
        f.push(Cmd::Pic {
            x,
            y: y as i16,
            pic,
        });
    }
    let (ini, n) = initials(name);
    let base = y + size / 2 + u16::from(font.ascent) * 2 / 5;
    centred(f, Rect::new(x, y, size, size), base, font, TEXT, &ini[..n]);
}

/// The round tick box at x, y (24 px): `ring` open, `full` ticked (both
/// pictures from [`disc`]), with the tick drawn over a full one.
pub fn round_check(
    f: &mut Frame,
    cx: &Ctx,
    x: u16,
    y: u16,
    ring: Option<Pic>,
    full: Option<Pic>,
    done: bool,
) {
    let pic = if done { full } else { ring };
    if let Some(pic) = pic {
        f.push(Cmd::Pic {
            x,
            y: y as i16,
            pic,
        });
    }
    if done {
        centred(
            f,
            Rect::new(x, y, 24, 24),
            y + 18,
            cx.fonts.ui,
            0xFF10_1418,
            b"v",
        );
    }
}

/// A small dot picture placed at x, y.
pub fn dot(f: &mut Frame, pic: Option<Pic>, x: u16, y: u16) {
    if let Some(pic) = pic {
        f.push(Cmd::Pic {
            x,
            y: y as i16,
            pic,
        });
    }
}

/// A thin progress ring drawn as a bar under a label (rings are pictures
/// the app paints; this is the bar form for rows).
pub fn progress_bar(f: &mut Frame, r: Rect, done: u16, of: u16, c: Argb) {
    // Opaque: a translucent track is laid over the wallpaper, not the
    // panel under it (see `tinted`).
    f.push(Cmd::Fill {
        r,
        color: mix(0xFF14_181E, TEXT, 0x40),
    });
    if of > 0 {
        let w = (u32::from(r.w) * u32::from(done.min(of)) / u32::from(of)) as u16;
        f.push(Cmd::Fill {
            r: Rect::new(r.x, r.y, w, r.h),
            color: c,
        });
    }
}
