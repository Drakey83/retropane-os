//! Run G21's half on C71's scenes (ADR-0034 amendment P, the owner's P2,
//! 2026-10-24), shared by the image and its host test: C71's five kinds
//! of four cards, each rendered 20 times four ways with the screen inside
//! the safe region, every render held word for word to the kind's
//! reference. The host test renders the scenes through the model of the
//! 3D core into a target on the screen and one off it, and holds the two
//! to each other (a picture doesn't depend on where it is drawn).

use rp_mbx3d_render::{Draw, Paint, Texture, UV_ONE};

/// The pictures' size: RGB565, two pixels a word.
pub const W: u32 = 480;
pub const H: u32 = 640;
pub const WORDS: usize = (W * H / 2) as usize;

/// The safe region's end: while the 3D core renders, the display may read
/// local memory below it (`rp-glesfeat` G19 on the owner's X51v: harmless
/// with a screen ending at `0xAE000`, wrong from one ending at `0xB6000`;
/// ADR-0034 amendment P rule 1). This run's own; `rp-vram-map` takes the
/// measured one with option A (ROADMAP step 3 (16)(b)).
pub const SAFE_END: u32 = 0xA_E000;

/// The screen: local memory offset 0 (Windows Mobile's place; rule 1).
pub const SCREEN_AT: u32 = 0;
pub const SCREEN_BYTES: u32 = W * H * 2;
const _: () = assert!(SCREEN_AT + SCREEN_BYTES <= SAFE_END);

/// What the CPU fills a picture with before each render or copy into it,
/// so one that silently doesn't happen can't pass on an earlier picture.
pub const SENTINEL: u32 = 0x5A5A_5A5A;

/// How a way renders. The screen is at [`SCREEN_AT`] in every way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Way {
    /// Off screen with the display's fetch paused around each render (C71:
    /// 0 of 100 renders differ that way): the kind's reference.
    Reference,
    /// Option A: off screen, the display reading the screen, then the 2D
    /// engine copies the picture onto it.
    CopyIn,
    /// Option C: the screen itself the target, the display reading the
    /// picture the core is drawing.
    Inside,
    /// C with the display's fetch paused around each render: the target's
    /// place alone.
    InsidePaused,
}

/// The ways in order: the reference first, every other held to it.
pub const WAYS: [Way; 4] = [Way::Reference, Way::CopyIn, Way::Inside, Way::InsidePaused];

impl Way {
    /// The target is the screen.
    #[must_use]
    pub const fn on_screen(self) -> bool {
        matches!(self, Self::Inside | Self::InsidePaused)
    }

    /// The display's fetch paused around each render.
    #[must_use]
    pub const fn paused(self) -> bool {
        matches!(self, Self::Reference | Self::InsidePaused)
    }

    /// The 2D engine copies the picture onto the screen after each render.
    #[must_use]
    pub const fn copies(self) -> bool {
        matches!(self, Self::CopyIn)
    }

    /// For the log.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Reference => "the reference: off screen, the display paused",
            Self::CopyIn => "A: off screen, a 2D copy onto the screen",
            Self::Inside => "C: drawn straight into the screen",
            Self::InsidePaused => "C with the display paused",
        }
    }
}

/// C71's kinds (run C71 v4), in its order.
pub const KINDS: [&str; 5] = [
    "flat cards 200 x 260",
    "textured cards 1:1 (128 x 128)",
    "textured cards 1.56x, point sampling",
    "textured cards 1.56x, bilinear",
    "textured cards shrunk 2:1 (64 x 64), point sampling",
];

/// The cards' places (W5's last frame, C71's) and the flat kind's colours.
pub const PLACES: [(i32, i32); 4] = [(20, 40), (260, 40), (20, 340), (272, 320)];
pub const COLOURS: [u32; 4] = [0xFFC0_4040, 0xFF40_C040, 0xFF40_40C0, 0xFFC0_C040];
/// The background (W5's).
pub const BACKGROUND: u32 = 0xFF20_2830;

/// Card `j` of kind `kind`, textured with `tex` (a 128 x 128 card; C71's
/// `cards[j]`) where the kind is.
#[must_use]
pub fn card(kind: usize, j: usize, tex: Texture) -> Draw {
    let (x, y) = PLACES[j % PLACES.len()];
    let textured = |w: i32, h: i32, t: Texture| {
        Draw::rect(
            x,
            y,
            w,
            h,
            Paint::Texture {
                tex: t,
                uv: [(0, 0), (UV_ONE, 0), (UV_ONE, UV_ONE), (0, UV_ONE)],
                tint: 0xFFFF_FFFF,
            },
        )
    };
    match kind {
        0 => Draw::rect(x, y, 200, 260, Paint::Flat(COLOURS[j % COLOURS.len()])),
        1 => textured(128, 128, tex),
        2 => textured(200, 260, tex),
        3 => textured(200, 260, tex.with_linear(true)),
        _ => textured(64, 64, tex),
    }
}

/// Up to four words where two pictures differ.
pub type First = [(u32, u32, u32, u32); 4];

/// The words of `b` that aren't `a`'s, over [`WORDS`]: how many, and the
/// first four as (x, y, a's, b's), x and y the word's first pixel.
pub fn diff(a: impl Fn(usize) -> u32, b: impl Fn(usize) -> u32) -> (u32, First, usize) {
    let mut first = [(0, 0, 0, 0); 4];
    let (mut n, mut nf) = (0u32, 0usize);
    for k in 0..WORDS {
        let (x, y) = (a(k), b(k));
        if x != y {
            n += 1;
            if nf < first.len() {
                first[nf] = ((2 * k as u32) % W, (2 * k as u32) / W, x, y);
                nf += 1;
            }
        }
    }
    (n, first, nf)
}

/// One way's 20 renders of a kind: renders and words off the reference,
/// the first words off, renders that didn't finish, the render times
/// (fastest, sum) and the copies' (fastest, sum, inexact, words off).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    pub renders: u32,
    pub unfinished: u32,
    pub off: u32,
    pub words: u32,
    pub first: First,
    pub nfirst: usize,
    pub render_min: u32,
    pub render_sum: u32,
    pub copies: u32,
    pub copy_min: u32,
    pub copy_sum: u32,
    pub copies_off: u32,
    pub copy_words: u32,
}

impl Tally {
    /// A render: its time (`None`: it didn't finish) and its words off.
    pub fn render(&mut self, us: Option<u32>, d: (u32, First, usize)) {
        self.renders += 1;
        let Some(us) = us else {
            self.unfinished += 1;
            return;
        };
        let finished = self.renders - self.unfinished;
        self.render_min = if finished == 1 {
            us
        } else {
            self.render_min.min(us)
        };
        self.render_sum = self.render_sum.saturating_add(us);
        self.off += u32::from(d.0 > 0);
        self.words += d.0;
        for &f in &d.1[..d.2] {
            if self.nfirst < self.first.len() {
                self.first[self.nfirst] = f;
                self.nfirst += 1;
            }
        }
    }

    /// A copy onto the screen: whether it finished, its time, the screen's
    /// words not the picture's.
    pub fn copy(&mut self, done: bool, us: u32, words: u32) {
        self.copy_min = if self.copies == 0 { us } else { self.copy_min.min(us) };
        self.copies += 1;
        self.copy_sum = self.copy_sum.saturating_add(us);
        self.copies_off += u32::from(!done || words > 0);
        self.copy_words += words;
    }

    /// The finished renders' mean time.
    #[must_use]
    pub const fn render_mean(&self) -> u32 {
        let n = self.renders - self.unfinished;
        if n == 0 { 0 } else { self.render_sum / n }
    }

    /// The copies' mean time.
    #[must_use]
    pub const fn copy_mean(&self) -> u32 {
        if self.copies == 0 { 0 } else { self.copy_sum / self.copies }
    }
}

#[cfg(test)]
#[path = "g21_tests.rs"]
mod tests;
