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
}

/// A clip that clips nothing (`Cmd::Clip`'s end).
pub const NO_CLIP: Rect = Rect::new(0, 0, u16::MAX, u16::MAX);

impl Cmd {
    /// The pixels the command may touch: a renderer skips it for a dirty
    /// rectangle it doesn't meet. Text is bounded by its font's line.
    #[must_use]
    pub fn bounds(&self, frame: &Frame) -> Rect {
        match *self {
            Cmd::Fill { r, .. } | Cmd::HGradient { r, .. } | Cmd::Outline { r, .. } => r,
            Cmd::Clip { .. } => Rect::new(0, 0, 0, 0),
            Cmd::Image { x, y, img } => Rect::new(x, y, img.w, img.h),
            // The line box with a margin: glyphs may overhang their advance
            // (italics, `left` < 0) by a few pixels.
            Cmd::Text {
                x,
                baseline,
                font,
                text,
                ..
            } => Rect::new(
                x.saturating_sub(TEXT_MARGIN),
                baseline.saturating_sub(u16::from(font.ascent) + TEXT_MARGIN),
                font.width(frame.text_of(text))
                    .saturating_add(2 * TEXT_MARGIN),
                u16::from(font.line) + 2 * TEXT_MARGIN,
            ),
        }
    }
}

/// How far a text run's bounds reach past its line box.
const TEXT_MARGIN: u16 = 4;

/// Commands a frame holds.
pub const MAX_CMDS: usize = 160;
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

#[cfg(test)]
mod tests;
