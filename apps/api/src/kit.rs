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
pub const DONE: Argb = theme(0xFF5C_E07A);
/// Late, delete.
pub const LATE: Argb = theme(0xFFFF_7A6B);
/// Warning.
pub const AMBER: Argb = theme(0xFFFF_C44D);

/// The app tints (design-language.md "App tints").
pub mod tint {
    use super::{Argb, theme};
    /// Calendar.
    pub const CALENDAR: Argb = theme(0xFFFF_7A5C);
    /// Contacts.
    pub const CONTACTS: Argb = theme(0xFF35_C2B0);
    /// Tasks.
    pub const TASKS: Argb = theme(0xFF4C_9BFF);
    /// Notes.
    pub const NOTES: Argb = theme(0xFFFF_C44D);
    /// The calculator's operators.
    pub const CALCULATOR: Argb = theme(0xFFFF_9F2E);
}

/// The eight identity colours (calendars, lists, notes, avatars): coral,
/// amber, lime, teal, sky, indigo, violet, rose.
pub const IDENTITY: [Argb; 8] = [
    theme(0xFFFF_7A5C),
    theme(0xFFFF_C44D),
    theme(0xFFA6_E05A),
    theme(0xFF35_C2B0),
    theme(0xFF4C_B8FF),
    theme(0xFF6C_7BFF),
    theme(0xFFB0_7CFF),
    theme(0xFFFF_6FA8),
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

/// The sizes of picture the kit paints (the picture store's classes
/// for apps): small marks, row avatars and the card's big avatar.
pub const PIC_CLASSES: [(u16, u16, u8); 3] = [(24, 24, 24), (40, 40, 24), (96, 96, 2)];

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
    let c = c | 0xFF00_0000;
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
    let n = labels.len().max(1) as u16;
    let gap = 6;
    let cw = (r.w - gap * (n - 1)) / n;
    for (i, l) in labels.iter().enumerate() {
        let c = Rect::new(r.x + i as u16 * (cw + gap), r.y, cw, r.h);
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

/// The 4-bit-free antialiased disc: `px` is `size` x `size` ARGB, filled
/// with `fill` inside the circle (and a `ring` of 2 px if given),
/// transparent outside, edges blended by coverage.
pub fn paint_disc(px: &mut [Argb], size: u16, fill: Argb, ring: Option<Argb>) {
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
                let lit = if ring.is_none() {
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
                let shade = if ring.is_none() && y * 4 > s * 3 {
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
