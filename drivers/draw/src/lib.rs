//! The drawing contract (ADR-0021): what the shell hands to whatever draws.
//!
//! The shell describes a frame; a [`Renderer`] puts it on the screen. The
//! shell depends on this crate only, never on pixel code, so it can't
//! assume the CPU draws: the CPU renderer (`rp-gfx`) and the MBX renderer
//! (item 69) are interchangeable behind [`Renderer`].
//!
//! - **A frame at a time, not a call at a time.** A [`Frame`] is the dirty
//!   rectangles plus the scene's commands. A GPU renderer submits it once
//!   and waits once (under Windows Mobile, waiting per operation cost
//!   ~1.7 ms each, docs/perf.md); the CPU renderer redraws only inside the
//!   dirty rectangles.
//! - **GPU-shaped primitives.** Fills, horizontal gradients, outlines,
//!   images and glyph runs: rectangles, textured quads and alpha masks, all
//!   things the 2700G's MBX core draws. Nothing here needs the CPU.
//! - **Assets are static data.** Fonts and images are `&'static`, made on
//!   the PC at build time; a GPU renderer can upload each once.
//! - **Bounded, no allocation.** A frame has fixed capacity; one that fills
//!   up stops taking commands and says so ([`Frame::overflowed`]), and too
//!   many dirty rectangles merge into their bounding box.
//!
//! Coordinates are screen pixels in `u16` (the screen is 480 x 640).

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

/// An ARGB8888 colour, `0xAARRGGBB`, straight alpha (0 clear, 255 solid).
pub type Argb = u32;

pub mod look;

/// A rectangle: `x..x + w` by `y..y + h`. Empty when `w` or `h` is 0.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Rect {
    /// Left edge.
    pub x: u16,
    /// Top edge.
    pub y: u16,
    /// Width.
    pub w: u16,
    /// Height.
    pub h: u16,
}

impl Rect {
    /// A rectangle.
    #[must_use]
    pub const fn new(x: u16, y: u16, w: u16, h: u16) -> Self {
        Self { x, y, w, h }
    }

    /// No pixels.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    /// One past the right edge (saturating).
    #[must_use]
    pub const fn right(&self) -> u16 {
        self.x.saturating_add(self.w)
    }

    /// One past the bottom edge (saturating).
    #[must_use]
    pub const fn bottom(&self) -> u16 {
        self.y.saturating_add(self.h)
    }

    /// The pixels in both (empty if none).
    #[must_use]
    pub fn intersect(&self, o: &Rect) -> Rect {
        let (x, y) = (self.x.max(o.x), self.y.max(o.y));
        let (r, b) = (self.right().min(o.right()), self.bottom().min(o.bottom()));
        if r <= x || b <= y {
            Rect::default()
        } else {
            Rect::new(x, y, r - x, b - y)
        }
    }

    /// The smallest rectangle holding both (an empty one adds nothing).
    #[must_use]
    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        let (x, y) = (self.x.min(o.x), self.y.min(o.y));
        Rect::new(
            x,
            y,
            self.right().max(o.right()) - x,
            self.bottom().max(o.bottom()) - y,
        )
    }

    /// Whether the point is inside.
    #[must_use]
    pub const fn contains_point(&self, px: u16, py: u16) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }
}

/// An image with its own alpha (icons), ARGB8888 straight, row after row.
#[derive(Clone, Copy, Debug)]
pub struct Image {
    /// Width.
    pub w: u16,
    /// Height.
    pub h: u16,
    /// Pixels, `w * h` (a shorter slice draws nothing).
    pub px: &'static [Argb],
}

/// One glyph: a coverage mask (0..=255 per pixel) and its placement.
#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    /// Mask width.
    pub w: u8,
    /// Mask height.
    pub h: u8,
    /// Mask's left edge from the pen position.
    pub left: i8,
    /// Mask's top edge from the baseline (positive: above it).
    pub top: i8,
    /// How far the pen moves after this glyph.
    pub advance: u8,
    /// Coverage, `w * h`.
    pub cover: &'static [u8],
}

/// A font at one size: printable ASCII from `first`, made on the PC.
#[derive(Debug)]
pub struct Font {
    /// Line height.
    pub line: u8,
    /// Baseline from the top of a line.
    pub ascent: u8,
    /// The first character covered (usually `' '`).
    pub first: u8,
    /// Glyphs from `first`, in order.
    pub glyphs: &'static [Glyph],
}

impl Font {
    /// The glyph for `c`, if the font has it.
    #[must_use]
    pub fn glyph(&self, c: u8) -> Option<&Glyph> {
        self.glyphs.get(usize::from(c.checked_sub(self.first)?))
    }

    /// The width `text` takes (characters it lacks take nothing).
    #[must_use]
    pub fn width(&self, text: &[u8]) -> u16 {
        text.iter()
            .filter_map(|&c| self.glyph(c))
            .fold(0u16, |w, g| w.saturating_add(u16::from(g.advance)))
    }

    /// Where `text`'s glyphs' masks fall, from the pen and the baseline:
    /// (left, right, top, bottom), x right and y down, `None` with no
    /// mask. A glyph may lie wholly left of its pen (the living sky's
    /// clouds: `left` = -their width).
    #[must_use]
    pub fn ink(&self, text: &[u8]) -> Option<(i32, i32, i32, i32)> {
        let mut pen = 0i32;
        let mut ink: Option<(i32, i32, i32, i32)> = None;
        for g in text.iter().filter_map(|&c| self.glyph(c)) {
            if g.w > 0 && g.h > 0 {
                let x0 = pen + i32::from(g.left);
                let y0 = -i32::from(g.top);
                let b = (x0, x0 + i32::from(g.w), y0, y0 + i32::from(g.h));
                ink = Some(ink.map_or(b, |a| {
                    (a.0.min(b.0), a.1.max(b.1), a.2.min(b.2), a.3.max(b.3))
                }));
            }
            pen += i32::from(g.advance);
        }
        ink
    }
}

/// Where a text run's bytes are in its frame's text store.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TextRef {
    start: u16,
    len: u16,
}

/// One drawing command.
#[derive(Clone, Copy, Debug)]
pub enum Cmd {
    /// A rectangle set to one colour, alpha included (glass is a
    /// translucent fill).
    Fill {
        /// Where.
        r: Rect,
        /// The colour.
        color: Argb,
    },
    /// A rectangle whose colour goes from `left` (first column) to `right`
    /// (last), alpha included: the glass panels' tint.
    HGradient {
        /// Where.
        r: Rect,
        /// First column's colour.
        left: Argb,
        /// Last column's colour.
        right: Argb,
    },
    /// A one-pixel frame just inside a rectangle, drawn over what's there:
    /// panel edges, the focus ring.
    Outline {
        /// The rectangle.
        r: Rect,
        /// The colour.
        color: Argb,
    },
    /// An image drawn over what's there.
    Image {
        /// Left edge.
        x: u16,
        /// Top edge.
        y: u16,
        /// The image.
        img: &'static Image,
    },
    /// A picture from the RAM store ([`pics::Pics`]) drawn over what's
    /// there: one decoded at run time (an icon at its size), named by its
    /// handle. A renderer without the store, or a stale handle, draws
    /// nothing.
    Pic {
        /// Left edge.
        x: u16,
        /// Top edge: above the top (negative), the picture's rows there
        /// are cut (a picture scrolled partly off).
        y: i16,
        /// The picture.
        pic: pics::Pic,
    },
    /// A run of text.
    Text {
        /// The pen's starting position.
        x: u16,
        /// The baseline.
        baseline: u16,
        /// The font.
        font: &'static Font,
        /// The colour (its alpha scales the glyphs' coverage).
        color: Argb,
        /// The bytes, in the frame's text store.
        text: TextRef,
    },
    /// Every later command is drawn only inside `r` (until the next
    /// `Clip`; [`NO_CLIP`] ends it): a list moved by the pixel inside its
    /// viewport. Draws nothing itself; renderers read it through
    /// [`Frame::cmds_clipped`].
    Clip {
        /// Where later commands may draw.
        r: Rect,
    },
    /// A panel drawn from pre-rendered art ([`PanelArt`]): one command, the
    /// renderer draws it as the fills and images [`panel_cmds`] gives (a
    /// glass box of any size with rounded corners and a shaded body, no
    /// pixel computed per frame).
    Panel {
        /// Where.
        r: Rect,
        /// Its art.
        art: &'static PanelArt,
    },
}

/// A panel's pre-rendered art (made offline, e.g.
/// `tools/shell-assets/panel_art.py`): the body's colour from its top row
/// to its bottom, the straight rim's rows and columns, the four corners as
/// images (the rim and the body's rounded ends in them).
#[derive(Debug)]
pub struct PanelArt {
    /// The corners' size (px), each `radius` square.
    pub radius: u16,
    /// The body's colour from top to bottom (stretched to the panel's
    /// height, a row each).
    pub rows: &'static [Argb; 64],
    /// The top and bottom rim rows (over the body, between the corners).
    pub top: [Argb; 2],
    /// See [`Self::top`].
    pub bottom: [Argb; 2],
    /// The side rim's colour (one column each side, between the corners).
    pub side: Argb,
    /// The corners: top left, top right, bottom left, bottom right.
    pub corners: [&'static Image; 4],
}

/// A panel's clear (fully see-through): see [`panel_cmds`].
pub const CLEAR: Argb = 0;

/// Panel `r` in `art` as fills and images, each passed to `emit`: first
/// the body's area (all but the corners) cleared, as fills of
/// [`CLEAR`]; then the body row by row (rows of one colour joined), the
/// rim's rows and columns, the corners. Nothing for a panel smaller than
/// its corners.
///
/// A renderer may take the clears as "the wallpaper here" and the fills
/// after them as blends over it: the 2D engine copies the wallpaper once
/// for the panel, not under every row (R176: a row's fill was a copy and a
/// blend, ~120 engine operations a panel). The pixels are the same.
pub fn panel_cmds(r: Rect, art: &PanelArt, mut emit: impl FnMut(Cmd)) {
    let k = art.radius;
    if r.w < 2 * k + 2 || r.h < 2 * k + 2 {
        return;
    }
    for c in [
        Rect::new(r.x + k, r.y, r.w - 2 * k, k),
        Rect::new(r.x, r.y + k, r.w, r.h - 2 * k),
        Rect::new(r.x + k, r.y + r.h - k, r.w - 2 * k, k),
    ] {
        emit(Cmd::Fill { r: c, color: CLEAR });
    }
    let row = |y: u16| {
        let i = u32::from(y) * 63 * 2 / u32::from(r.h - 1);
        art.rows[(i.div_ceil(2) as usize).min(63)]
    };
    // The body: between the corners across the top and bottom bands, the
    // whole width (less the side rims) between them; a fill for each run
    // of rows of one colour.
    let mut y = 0;
    while y < r.h {
        let band = y < k || y >= r.h - k;
        let rim = match y {
            0 | 1 => Some(art.top[usize::from(y)]),
            _ if y >= r.h - 2 => Some(art.bottom[usize::from(y - (r.h - 2))]),
            _ => None,
        };
        let color = rim.unwrap_or_else(|| row(y));
        let mut n = 1;
        while rim.is_none()
            && y + n < r.h
            && (y + n < k || y + n >= r.h - k) == band
            && y + n < r.h - 2
            && row(y + n) == color
        {
            n += 1;
        }
        let (x, w) = if band {
            (r.x + k, r.w - 2 * k)
        } else {
            (r.x + 1, r.w - 2)
        };
        emit(Cmd::Fill {
            r: Rect::new(x, r.y + y, w, n),
            color,
        });
        y += n;
    }
    // The side rims, between the corners.
    for x in [r.x, r.x + r.w - 1] {
        emit(Cmd::Fill {
            r: Rect::new(x, r.y + k, 1, r.h - 2 * k),
            color: art.side,
        });
    }
    for (i, &(x, y)) in [
        (r.x, r.y),
        (r.x + r.w - k, r.y),
        (r.x, r.y + r.h - k),
        (r.x + r.w - k, r.y + r.h - k),
    ]
    .iter()
    .enumerate()
    {
        emit(Cmd::Image {
            x,
            y,
            img: art.corners[i],
        });
    }
}

/// A clip that clips nothing (`Cmd::Clip`'s end).
pub const NO_CLIP: Rect = Rect::new(0, 0, u16::MAX, u16::MAX);

impl Cmd {
    /// The pixels the command may touch: a renderer skips it for a dirty
    /// rectangle it doesn't meet. Text is bounded by its font's line.
    #[must_use]
    pub fn bounds(&self, frame: &Frame) -> Rect {
        match *self {
            Cmd::Fill { r, .. }
            | Cmd::HGradient { r, .. }
            | Cmd::Outline { r, .. }
            | Cmd::Panel { r, .. } => r,
            Cmd::Clip { .. } => Rect::new(0, 0, 0, 0),
            Cmd::Image { x, y, img } => Rect::new(x, y, img.w, img.h),
            Cmd::Pic { x, y, pic } => {
                let cut = y.min(0).unsigned_abs();
                Rect::new(
                    x,
                    y.max(0).cast_unsigned(),
                    pic.w,
                    pic.h.saturating_sub(cut),
                )
            }
            // The line box and where the glyphs' masks fall (a cloud of the
            // living sky lies wholly left of its pen: bounded by its line
            // box alone, it was skipped once its pen passed the sky's right
            // edge and vanished there, the owner's run of 2026-11-01), with
            // a margin.
            Cmd::Text {
                x,
                baseline,
                font,
                text,
                ..
            } => {
                let t = frame.text_of(text);
                let (x, b) = (i32::from(x), i32::from(baseline));
                let (mut x0, mut x1) = (x, x + i32::from(font.width(t)));
                let (mut y0, mut y1) = (
                    b - i32::from(font.ascent),
                    b - i32::from(font.ascent) + i32::from(font.line),
                );
                if let Some((l, r, top, bottom)) = font.ink(t) {
                    (x0, x1) = (x0.min(x + l), x1.max(x + r));
                    (y0, y1) = (y0.min(b + top), y1.max(b + bottom));
                }
                let m = i32::from(TEXT_MARGIN);
                let (x0, y0) = ((x0 - m).max(0), (y0 - m).max(0));
                let (x1, y1) = ((x1 + m).clamp(x0, 0xFFFF), (y1 + m).clamp(y0, 0xFFFF));
                Rect::new(x0 as u16, y0 as u16, (x1 - x0) as u16, (y1 - y0) as u16)
            }
        }
    }
}

/// How far a text run's bounds reach past its line box.
const TEXT_MARGIN: u16 = 4;

/// Commands a frame holds: the busiest frame the shell's tests draw (with a
/// picture in every slot, as on the X51v) is 168 commands (Hourly scrolled
/// with the menu open, every hour's chance of rain shown), a quarter spare
/// (rp-shell's test holds it). It was
/// 256 while Hourly's line was 49 pieces on the 2D path (209 commands); at
/// 160, Hourly drawn on screen 0 (185) lost its scroll bar, tabs and
/// softkeys without a word (emu-today, 2026-10-31). The line is one
/// picture since option C's step 6. Back to 256 on 2026-11-01: the light
/// look's glass fades at its edges (five pieces a panel, the owner's ask),
/// its busiest frame ~181, a quarter spare.
pub const MAX_CMDS: usize = 256;
/// Bytes of text a frame holds.
pub const MAX_TEXT: usize = 1024;
/// Dirty rectangles a frame keeps apart; more merge into their union.
pub const MAX_DIRTY: usize = 8;

/// One frame: what changed on screen, and the whole scene to draw there.
pub struct Frame {
    cmds: [Option<Cmd>; MAX_CMDS],
    n: usize,
    text: [u8; MAX_TEXT],
    tn: usize,
    dirty: [Rect; MAX_DIRTY],
    dn: usize,
    overflowed: bool,
}

impl Default for Frame {
    fn default() -> Self {
        Self::new()
    }
}

impl Frame {
    /// An empty frame.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            cmds: [None; MAX_CMDS],
            n: 0,
            text: [0; MAX_TEXT],
            tn: 0,
            dirty: [Rect::new(0, 0, 0, 0); MAX_DIRTY],
            dn: 0,
            overflowed: false,
        }
    }

    /// [`Frame::new`] made straight into `slot` (the image's, in `.bss`),
    /// a field at a time: at 256 commands a frame is 5 KB, and made on the
    /// stack and moved it took a 5,136-byte frame (`cargo xtask stacks`,
    /// P6).
    pub fn new_in(slot: &mut core::mem::MaybeUninit<Self>) -> &mut Self {
        rp_hal_mem::in_place!(slot, Frame {
            cmds: @each None,
            n: 0,
            text: @each 0,
            tn: 0,
            dirty: @each Rect::new(0, 0, 0, 0),
            dn: 0,
            overflowed: false,
        })
    }

    /// Every colour the frame's commands draw with passed through `f`
    /// (pictures and the text they hold keep theirs): the look's switch
    /// (light and dark) done once for every screen, after it is drawn.
    /// `f` is told whether the colour is text's (text may need more
    /// contrast than a fill).
    pub fn map_colors(&mut self, f: impl Fn(Argb, bool) -> Argb) {
        for c in self.cmds.iter_mut().take(self.n).flatten() {
            match c {
                Cmd::Fill { color, .. } | Cmd::Outline { color, .. } => {
                    *color = f(*color, false);
                }
                Cmd::Text { color, .. } => *color = f(*color, true),
                Cmd::HGradient { left, right, .. } => {
                    *left = f(*left, false);
                    *right = f(*right, false);
                }
                Cmd::Image { .. } | Cmd::Pic { .. } | Cmd::Clip { .. } | Cmd::Panel { .. } => {}
            }
        }
    }

    /// Empties it for the next frame.
    pub fn clear(&mut self) {
        self.n = 0;
        self.tn = 0;
        self.dn = 0;
        self.overflowed = false;
    }

    /// Adds a command (ignored once the frame is full: [`Self::overflowed`]).
    pub fn push(&mut self, c: Cmd) {
        match self.cmds.get_mut(self.n) {
            Some(slot) => {
                *slot = Some(c);
                self.n += 1;
            }
            None => self.overflowed = true,
        }
    }

    /// Stores `text` and adds a run of it (bytes outside printable ASCII
    /// are kept; the font draws only what it has).
    pub fn text(&mut self, x: u16, baseline: u16, font: &'static Font, color: Argb, text: &[u8]) {
        let end = self.tn + text.len();
        let (Some(store), Ok(start), Ok(len)) = (
            self.text.get_mut(self.tn..end),
            u16::try_from(self.tn),
            u16::try_from(text.len()),
        ) else {
            self.overflowed = true;
            return;
        };
        store.copy_from_slice(text);
        self.tn = end;
        self.push(Cmd::Text {
            x,
            baseline,
            font,
            color,
            text: TextRef { start, len },
        });
    }

    /// The bytes of a text run.
    #[must_use]
    pub fn text_of(&self, t: TextRef) -> &[u8] {
        let start = usize::from(t.start);
        self.text
            .get(start..start + usize::from(t.len))
            .unwrap_or(&[])
    }

    /// Marks `r` as changed; past [`MAX_DIRTY`] rectangles it is merged
    /// into the one it grows least.
    pub fn damage(&mut self, r: Rect) {
        if r.is_empty() {
            return;
        }
        if self.dirty[..self.dn].iter().any(|d| d.intersect(&r) == r) {
            return;
        }
        if self.dn < MAX_DIRTY {
            self.dirty[self.dn] = r;
            self.dn += 1;
            return;
        }
        let area = |r: &Rect| u32::from(r.w) * u32::from(r.h);
        let best = (0..self.dn)
            .min_by_key(|&i| area(&self.dirty[i].union(&r)) - area(&self.dirty[i]))
            .unwrap_or(0);
        self.dirty[best] = self.dirty[best].union(&r);
    }

    /// The commands, in drawing order (`Clip`s included).
    pub fn cmds(&self) -> impl Iterator<Item = &Cmd> {
        self.cmds[..self.n].iter().flatten()
    }

    /// Limits every later command to `r` (`None`: the whole screen again).
    pub fn clip(&mut self, r: Option<Rect>) {
        self.push(Cmd::Clip {
            r: r.unwrap_or(NO_CLIP),
        });
    }

    /// The drawing commands in order, each with the clip it is drawn in
    /// ([`NO_CLIP`] outside any): what a renderer draws. `Clip`s
    /// themselves are left out.
    pub fn cmds_clipped(&self) -> impl Iterator<Item = (&Cmd, Rect)> {
        self.cmds()
            .scan(NO_CLIP, |clip, c| {
                Some(match *c {
                    Cmd::Clip { r } => {
                        *clip = r;
                        None
                    }
                    _ => Some((c, *clip)),
                })
            })
            .flatten()
    }

    /// The changed rectangles.
    #[must_use]
    pub fn dirty(&self) -> &[Rect] {
        &self.dirty[..self.dn]
    }

    /// Whether commands or text were dropped because the frame was full.
    #[must_use]
    pub const fn overflowed(&self) -> bool {
        self.overflowed
    }
}

/// What a renderer reports for a frame (for the run log and benchmarks).
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Drawn {
    /// Commands drawn (counted once per dirty rectangle they met).
    pub cmds: u32,
    /// Pixels written to the screen.
    pub pixels: u64,
}

/// Puts frames on the screen: the CPU renderer in `rp-gfx`, the MBX one
/// later. Returns when the frame's pixels are in the plane (a GPU renderer
/// waits for its one fence).
pub trait Renderer {
    /// Draws `frame`: inside each dirty rectangle, clear, then every
    /// command that meets it in order, then out to the screen. A renderer
    /// may return before the frame is on the screen ([`Self::poll`]).
    fn draw(&mut self, frame: &Frame) -> Drawn;

    /// Puts a frame `draw` left drawing in the background on the screen
    /// once it is done. True when nothing is left pending (the last frame
    /// drawn is on the screen); call it until then, without blocking on it.
    /// The CPU renderer is done when `draw` returns.
    fn poll(&mut self) -> bool {
        true
    }
}

pub mod pics;
pub mod shrink;

#[cfg(test)]
mod tests;
