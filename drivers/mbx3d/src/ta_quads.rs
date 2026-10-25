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

/// How the quads' corners go in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primitive {
    /// A strip of four a quad, each ended in the end window (`rp-glesfeat`
    /// G2 drew one this way): the primitive word and an end a quad.
    Strips,
    /// One triangle list (primitive word `0x7...`, the GLES driver's
    /// `0x02274a1c`) for the batch: six corners a quad (two triangles), one
    /// end for all, so the words run unbroken (a DMA's pieces). Whether the
    /// TA draws it is P4's run's question.
    List,
}

impl Primitive {
    /// Corners a quad.
    #[must_use]
    pub const fn corners(self) -> usize {
        match self {
            Self::Strips => 4,
            Self::List => 6,
        }
    }

    /// Its primitive word.
    #[must_use]
    pub const fn word(self) -> u32 {
        match self {
            Self::Strips => STRIP,
            Self::List => LIST,
        }
    }
}

/// The triangle list's primitive word (bits 31-28 `0x7`, the polygon
/// offset 0).
pub const LIST: u32 = 0x7000_0000;

/// The corners of a quad in a triangle list, as strip corner numbers (bit
/// 0 right, bit 1 bottom): top left, top right, bottom left; top right,
/// bottom right, bottom left (the renderer's triangles, same winding).
const LIST_CORNERS: [usize; 6] = [0, 1, 2, 1, 3, 2];

/// How a stream is shaped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    /// The vertex's words.
    pub form: Form,
    /// How often the state block goes in.
    pub batching: Batching,
    /// How the corners go in.
    pub primitive: Primitive,
}

impl Shape {
    /// Dell's: 7-word vertices, the state block a quad, a strip a quad.
    pub const DELL: Self = Self {
        form: Form::Coloured,
        batching: Batching::PerQuad,
        primitive: Primitive::Strips,
    };

    /// Whether quad `k` starts with the state block.
    const fn has_state(self, k: usize) -> bool {
        k == 0 || matches!(self.batching, Batching::PerQuad)
    }

    /// Whether quad `k` starts a primitive.
    const fn has_primitive(self, k: usize) -> bool {
        matches!(self.primitive, Primitive::Strips) || self.has_state(k)
    }

    /// Whether quad `k` of `n` ends its primitive.
    const fn has_end(self, k: usize, n: usize) -> bool {
        matches!(self.primitive, Primitive::Strips)
            || matches!(self.batching, Batching::PerQuad)
            || k + 1 == n
    }

    /// Words of quad `k`'s state block and primitive word.
    const fn head(self, k: usize) -> usize {
        (if self.has_state(k) { STATE_WORDS } else { 0 })
            + (if self.has_primitive(k) { 1 } else { 0 })
    }

    /// Words of a quad's corners.
    const fn body(self) -> usize {
        self.primitive.corners() * self.form.vertex_words()
    }

    /// Items of quads `0..k` of `n`.
    const fn prefix(self, k: usize, n: usize) -> usize {
        let mut total = 0;
        let mut j = 0;
        while j < k {
            total += self.head(j) + self.body() + if self.has_end(j, n) { 1 } else { 0 };
            j += 1;
        }
        total
    }
}

/// Items (words and primitive ends) the stream of `n` quads holds.
#[must_use]
pub const fn items(shape: Shape, n: usize) -> usize {
    shape.prefix(n, n)
}

/// The stream for `quads` in `shape`: the state block ([`TEXTURED_MASK`],
/// then `state`), the primitive word, each quad's corners (a strip: top
/// left, top right, bottom left, bottom right, the order the TA drew in
/// `rp-glesfeat` G2; a list: [`LIST_CORNERS`]), and the primitive's end.
#[must_use]
pub fn quads(state: State, shape: Shape, quads: &[Quad]) -> Quads<'_> {
    Quads {
        state,
        shape,
        quads,
        quad: 0,
        at: 0,
    }
}

/// The iterator [`quads`] returns.
#[derive(Clone, Debug)]
pub struct Quads<'a> {
    state: State,
    shape: Shape,
    quads: &'a [Quad],
    /// The quad being sent.
    quad: usize,
    /// The item within it, its state block (when it has one) first.
    at: usize,
}

impl Quads<'_> {
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

    /// Word `f` of corner `c` (strip numbering: bit 0 right, bit 1
    /// bottom) of `q`.
    fn vertex_word(&self, q: &Quad, c: usize, f: usize) -> u32 {
        let (right, bottom) = (c & 1 != 0, c & 2 != 0);
        let pick = |a: f32, b: f32, second: bool| if second { b } else { a };
        // The colour word is the fifth; the white form leaves it out.
        let f = if self.shape.form == Form::White && f >= 4 {
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
        let sh = self.shape;
        let k = self.quad;
        let state = if sh.has_state(k) { STATE_WORDS } else { 0 };
        let head = sh.head(k);
        let vw = sh.form.vertex_words();
        let at = self.at;
        self.at += 1;
        let item = if at < state {
            Item::Word(self.state_word(at))
        } else if at < head {
            Item::Word(sh.primitive.word())
        } else if at < head + sh.body() {
            let i = at - head;
            let corner = match sh.primitive {
                Primitive::Strips => i / vw,
                Primitive::List => LIST_CORNERS[i / vw],
            };
            let w = Item::Word(self.vertex_word(q, corner, i % vw));
            if at + 1 == head + sh.body() && !sh.has_end(k, self.quads.len()) {
                self.quad += 1;
                self.at = 0;
            }
            w
        } else {
            self.quad += 1;
            self.at = 0;
            Item::End
        };
        Some(item)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.quads.len();
        let all = items(self.shape, n);
        let done = self.shape.prefix(self.quad, n) + self.at;
        let left = all.saturating_sub(done);
        (left, Some(left))
    }
}

impl ExactSizeIterator for Quads<'_> {}
