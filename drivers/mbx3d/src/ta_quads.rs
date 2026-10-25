//! The TA streams P4 measures (step 2, docs/design/p4-ta-speed.md):
//! textured quads as strips of four, the state block before every quad
//! (Dell's GLES driver, a draw call a sprite) or once for the batch, and
//! the vertex with or without its colour word.
//!
//! The stream is an iterator that works out each word when [`super::ta::feed`]
//! asks for it: a 5,000-quad frame goes from the quads' 40 bytes each
//! straight into the TA window, with no 150-190 KiB copy of the stream
//! built in memory first (the GLES driver's transform stage builds its
//! vertices in a buffer, then copies them).

use super::ta::{Item, STRIP, TEX_VERTEX_WORDS};

/// The state block's mask with TSP, ISP, texture unit 0's three words, the
/// clip word and the bit 6 word (the GLES driver's textured draw,
/// `0x0226eae4`).
pub const TEXTURED_MASK: u32 = 0x1000_0057;
/// Words in that state block, its mask included.
pub const STATE_WORDS: usize = 8;

/// A textured quad: its screen rectangle, depth and texture corners.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    /// Top left corner, in pixels.
    pub x0: f32,
    /// See [`Self::x0`].
    pub y0: f32,
    /// Bottom right corner, in pixels.
    pub x1: f32,
    /// See [`Self::x1`].
    pub y1: f32,
    /// Depth, every corner.
    pub z: f32,
    /// Texture coordinates at the top left corner.
    pub u0: f32,
    /// See [`Self::u0`].
    pub v0: f32,
    /// Texture coordinates at the bottom right corner.
    pub u1: f32,
    /// See [`Self::u1`].
    pub v1: f32,
    /// The colour every corner carries in [`Form::Coloured`].
    pub argb: u32,
}

/// The state block's words after its mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    /// The TSP word.
    pub tsp: u32,
    /// The ISP word (depth and alpha test).
    pub isp: u32,
    /// Texture unit 0's TEX0-TEX2.
    pub tex: [u32; 3],
    /// The clip word ([`super::ta::clip`]).
    pub clip: u32,
}

/// The vertex's words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// Dell's: x, y, z, 1/w, colour, u, v ([`TEX_VERTEX_WORDS`]; drawn
    /// right through the TA on the X51v as triangles, `rp-glesfeat` L, M).
    Coloured,
    /// x, y, z, 1/w, u, v: no colour word, for quads tinted white (-14% of
    /// the bytes). Which state word tells the TA its vertices carry no
    /// colour is P4's run's question (the GLES driver's transform stage
    /// drops the word when the context's `+0x5E0` is 0, `0x02272064`; how
    /// the TA learns it isn't read yet): the [`State`]'s TSP carries the
    /// candidate.
    White,
}

impl Form {
    /// Words a vertex.
    #[must_use]
    pub const fn vertex_words(self) -> usize {
        match self {
            Self::Coloured => TEX_VERTEX_WORDS,
            Self::White => TEX_VERTEX_WORDS - 1,
        }
    }
}

/// How often the state block goes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Batching {
    /// Before every quad: Dell's GLES driver, one draw call a sprite.
    PerQuad,
    /// Once, the quads' strips after it (ADR-0026 rule 3). Whether the TA
    /// keeps the state across a primitive's end is P4's way 3.
    Once,
}

/// Items (words and primitive ends) the stream of `n` quads holds.
#[must_use]
pub const fn items(form: Form, batching: Batching, n: usize) -> usize {
    let quad = 1 + 4 * form.vertex_words() + 1;
    match batching {
        Batching::PerQuad => n * (STATE_WORDS + quad),
        Batching::Once if n == 0 => 0,
        Batching::Once => STATE_WORDS + n * quad,
    }
}

/// The stream for `quads`: the state block ([`TEXTURED_MASK`], then
/// `state`), then for each quad the strip word, its four corners (top
/// left, top right, bottom left, bottom right: the strip order the TA
/// drew in `rp-glesfeat` G2), and the primitive's end.
#[must_use]
pub fn quads(state: State, form: Form, batching: Batching, quads: &[Quad]) -> Quads<'_> {
    Quads {
        state,
        form,
        batching,
        quads,
        quad: 0,
        at: 0,
    }
}

/// The iterator [`quads`] returns.
#[derive(Clone, Debug)]
pub struct Quads<'a> {
    state: State,
    form: Form,
    batching: Batching,
    quads: &'a [Quad],
    /// The quad being sent.
    quad: usize,
    /// The item within it, its state block (when it has one) first.
    at: usize,
}

impl Quads<'_> {
    /// Whether quad `k` starts with the state block.
    fn has_state(&self, k: usize) -> bool {
        k == 0 || self.batching == Batching::PerQuad
    }

    fn state_word(&self, k: usize) -> u32 {
        let s = &self.state;
        match k {
            0 => TEXTURED_MASK,
            1 => s.tsp,
            2 => s.isp,
            3..=5 => s.tex[k - 3],
            6 => s.clip,
            // The bit 6 word, 0 as in the GLES driver's draws.
            _ => 0,
        }
    }

    /// Word `f` of corner `c` (strip order) of `q`.
    fn vertex_word(&self, q: &Quad, c: usize, f: usize) -> u32 {
        let (right, bottom) = (c & 1 != 0, c & 2 != 0);
        let pick = |a: f32, b: f32, second: bool| if second { b } else { a };
        // The colour word is the fifth; the white form leaves it out.
        let f = if self.form == Form::White && f >= 4 {
            f + 1
        } else {
            f
        };
        match f {
            0 => pick(q.x0, q.x1, right).to_bits(),
            1 => pick(q.y0, q.y1, bottom).to_bits(),
            2 => q.z.to_bits(),
            3 => 1.0f32.to_bits(),
            4 => q.argb,
            5 => pick(q.u0, q.u1, right).to_bits(),
            _ => pick(q.v0, q.v1, bottom).to_bits(),
        }
    }
}

impl Iterator for Quads<'_> {
    type Item = Item;

    fn next(&mut self) -> Option<Item> {
        let q = self.quads.get(self.quad)?;
        let state = if self.has_state(self.quad) {
            STATE_WORDS
        } else {
            0
        };
        let vw = self.form.vertex_words();
        let at = self.at;
        self.at += 1;
        let item = if at < state {
            Item::Word(self.state_word(at))
        } else if at == state {
            Item::Word(STRIP)
        } else if at <= state + 4 * vw {
            let k = at - state - 1;
            Item::Word(self.vertex_word(q, k / vw, k % vw))
        } else {
            self.quad += 1;
            self.at = 0;
            Item::End
        };
        Some(item)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let all = items(self.form, self.batching, self.quads.len());
        let done = items(self.form, self.batching, self.quad) + self.at;
        let left = all.saturating_sub(done);
        (left, Some(left))
    }
}

impl ExactSizeIterator for Quads<'_> {}
