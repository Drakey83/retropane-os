//! The screen through the 2700G (`rp_gfx_open`, `rp_gfx_frame`,
//! `rp_gfx_wait`, `rp_gfx_close` in `ports/include/retropane.h`;
//! docs/design/ref-rp.md step 3): the program hands a frame's words over
//! in one buffer and the port layer checks every one before any reaches
//! the tile accelerator.
//!
//! Iterate, don't imitate (rule 0). GL hands a driver hundreds of calls a
//! frame, a call a vertex, and trusts what it is given; a Windows Mobile
//! game on Dell's GLES writes the TA's window itself. Ours is one call a
//! frame whose words are the TA's own (no state machine in between), in a
//! form with one difference: a strip's word carries its corner count, so
//! the check knows where every primitive ends without a marker that a
//! vertex word could equal. Only the state block, the strip and the fan the
//! port layer knows pass, and only textures inside the program's own texture
//! memory: a wrong word stops the frame at the check instead of hanging
//! the chip (the TA hangs on words it does not take: audit C80, R288).

use rp_hal_cabi::{Errno, Gfx};

/// A state block's first word: the mask of the words after it (TSP, ISP,
/// texture unit 0's three words, the clip word, the bit 6 word), as P4's
/// textured draw and Dell's GLES send it (`ta_quads::TEXTURED_MASK`).
pub const STATE: u32 = 0x1000_0057;
/// Words after [`STATE`].
pub const STATE_WORDS: usize = 7;
/// A two-texture state block's first word (Half-Life kit 112, the world in
/// one pass: the texture times its lightmap on the 2700G's second texture
/// unit; ADR-0040, under ADR-0039): mask bit 3 adds texture unit 1's three
/// words after unit 0's, as Dell's GLES driver sends them (`0x2267868`).
/// The TSP word's bit 16 tells the TA the corners carry a second
/// coordinate pair (`rp-glesfeat` M). Corners of strips and fans after it
/// carry [`UNIT1_WORDS`] more words (u1, v1).
pub const STATE2: u32 = 0x1000_005F;
/// Words after [`STATE2`]: TSP, ISP, unit 0's TEX0-2, unit 1's TEX0-2, the
/// clip word, the bit 6 word.
pub const STATE2_WORDS: usize = 10;
/// The words a corner gains after a [`STATE2`] block.
pub const UNIT1_WORDS: usize = 2;
/// A strip of `n` corners: this, `| n`. The TA is sent the strip word
/// without the count.
pub const STRIP: u32 = 0x4000_0000;
/// A fan of `n` corners (the first corner shared by every triangle: the
/// TA's own fan, primitive word `0x5`, as Dell's GLES driver sends it;
/// docs/design/ref-rp.md step 4): this, `| n`. The TA is sent the fan word
/// without the count. Its corner counts are the strip's.
pub const FAN: u32 = 0x5000_0000;
/// The fewest and the most corners a strip may have (a face of the world
/// clipped by the near plane has at most its corners plus one: the emitter's
/// `RP_MAX_CORNERS` + 1).
pub const STRIP_CORNERS: (u32, u32) = (3, 255);
/// Words in a corner: x, y, z, 1/w, colour, u, v (the GLES driver's
/// textured vertex).
pub const CORNER_WORDS: usize = 7;
/// A strip's or fan's header bit 21 (ADR-0040 step 7g7, shown on the
/// X51v, run 144): its corners have no z word, [`CORNER_WORDS_NO_Z`]
/// words each (x, y, 1/w, colour, u, v), the depth taken from 1/w. Sent to
/// the TA with the header.
pub const NO_Z: u32 = 1 << 21;
/// Words in a corner of a [`NO_Z`] strip or fan.
pub const CORNER_WORDS_NO_Z: usize = 6;

/// Whether `w` is a strip's or a fan's header (either corner layout).
#[must_use]
pub const fn is_prim(w: u32) -> bool {
    let k = w & !NO_Z & 0xFFFF_FF00;
    k == STRIP || k == FAN
}

/// Words in each corner of the strip or fan whose header is `w`.
#[must_use]
pub const fn corner_words(w: u32) -> usize {
    if w & NO_Z != 0 { CORNER_WORDS_NO_Z } else { CORNER_WORDS }
}

/// Words in each corner of the strip or fan whose header is `w`, after a
/// [`STATE2`] block when `two`.
#[must_use]
pub const fn corner_words_in(w: u32, two: bool) -> usize {
    corner_words(w) + if two { UNIT1_WORDS } else { 0 }
}

/// TEX0's texel formats a texture may have here, all 16 bits a texel
/// (`rp_drv_mbx3d::tex`: ARGB1555, RGB565, ARGB4444, RGB332).
const FORMATS_16: [u32; 4] = [0, 2, 4, 6];

/// The 2700G as the runner gives it to a program.
pub trait Screen {
    /// Takes the screen; what [`Gfx`] the program gets.
    ///
    /// # Errors
    /// Open already (`EBUSY`), or the chip would not set up (`EIO`).
    fn open(&mut self) -> Result<Gfx, Errno>;
    /// One checked frame's TA words ([`Piece`]s) drawn into the screen,
    /// in parts drawn in their order (each checked as a frame of its own:
    /// each starts with a state block).
    ///
    /// # Errors
    /// Not open (`EBADF`), the check refused a word (`EINVAL`: nothing of
    /// the frame drawn), or the chip did not finish (`EIO`).
    fn frame(&mut self, parts: &[&[u32]]) -> Result<(), Errno>;
    /// Waits until the last frame is drawn.
    ///
    /// # Errors
    /// Not open (`EBADF`), or the render did not finish (`EIO`).
    fn wait(&mut self) -> Result<(), Errno>;
    /// Gives the screen back.
    ///
    /// # Errors
    /// Not open (`EBADF`).
    fn close(&mut self) -> Result<(), Errno>;
    /// A look at work the chip does after [`Self::frame`] returned (a
    /// frame fed by DMA: its render started once the chip has taken it),
    /// never a wait: called from the program's clock reads, which a game
    /// makes many times a frame. Nothing by default.
    fn pump(&mut self) {}
}

/// One thing for the tile accelerator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Piece {
    /// A word into the TA's window.
    Word(u32),
    /// The primitive ends (a `1` into the end window).
    End,
}

/// What the check counted in a frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// State blocks.
    pub states: u32,
    /// Strips and fans.
    pub strips: u32,
    /// Corners.
    pub corners: u32,
}

/// Why a frame was refused: the word's place and what was wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refused {
    /// The word's index in the frame.
    pub at: usize,
    /// What was wrong.
    pub why: Why,
}

/// See [`Refused`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// Neither a state block, a strip nor a fan.
    Unknown,
    /// A strip or fan before any state block.
    NoState,
    /// A strip's or fan's corner count outside [`STRIP_CORNERS`].
    Corners,
    /// The frame ends inside a state block or a strip.
    Short,
    /// A texture not wholly inside the program's texture memory, or of a
    /// format not taken.
    Texture,
}

/// The bytes of the texture a state block names, and where it starts, from
/// its TEX0 and TEX1 words: `None` for a format not taken.
#[must_use]
pub fn texture(tex0: u32, tex1: u32) -> Option<(u32, u32)> {
    let format = (tex0 >> 13) & 0x1F;
    if !FORMATS_16.contains(&format) {
        return None;
    }
    let (cu, cv) = ((tex0 >> 24) & 15, (tex0 >> 20) & 15);
    if cu > 8 || cv > 8 {
        return None;
    }
    let bytes = (8u32 << cu) * (8u32 << cv) * 2;
    Some(((tex1 & 0x01FF_FFFF) << 7, bytes))
}

/// Checks a frame's `words` against `g`'s texture memory: what is in it,
/// or the first word refused and why. Before [`send`]: a refused frame
/// gives the chip nothing.
///
/// # Errors
/// The first word refused, and why.
pub fn check(words: &[u32], g: &Gfx) -> Result<Counts, Refused> {
    walk(words, Some(g), |_| true)
}

/// A frame [`check`] took, as runs of words to `run` (a state block, a
/// strip's header, its corners) and each strip's end to `end`, in the
/// order [`send`] gives them word by word; either returning false stops it
/// (the chip stuck). What was sent. The feed writes a run in one tight
/// loop (run 39: the frame's words were costing ~100 ns each of software
/// on top of the bus's ~133).
pub fn send_runs<T>(
    words: &[u32],
    ctx: &mut T,
    mut run: impl FnMut(&mut T, &[u32]) -> bool,
    mut end: impl FnMut(&mut T) -> bool,
) -> Counts {
    let mut c = Counts::default();
    let mut at = 0;
    let mut two = false;
    while at < words.len() {
        let w = words[at];
        if w == STATE || w == STATE2 {
            let sw = if w == STATE2 { STATE2_WORDS } else { STATE_WORDS };
            let Some(block) = words.get(at..=at + sw) else {
                break;
            };
            two = w == STATE2;
            c.states += 1;
            if !run(ctx, block) {
                break;
            }
            at += 1 + sw;
        } else if is_prim(w) {
            let len = (w & 0xFF) as usize * corner_words_in(w, two);
            let Some(corners) = words.get(at + 1..=at + len) else {
                break;
            };
            c.strips += 1;
            c.corners += w & 0xFF;
            if !(run(ctx, &[w & 0xFFFF_FF00]) && run(ctx, corners) && end(ctx)) {
                break;
            }
            at += 1 + len;
        } else {
            break;
        }
    }
    c
}

/// A frame [`check`] took, as pieces to `put` in order; `put` returning
/// false stops it (the chip stuck). What was sent.
pub fn send(words: &[u32], put: impl FnMut(Piece) -> bool) -> Counts {
    walk(words, None, put).unwrap_or_default()
}

/// The frame's pieces to `put`; with `g`, every texture checked too.
fn walk(
    words: &[u32],
    g: Option<&Gfx>,
    mut put: impl FnMut(Piece) -> bool,
) -> Result<Counts, Refused> {
    let mut c = Counts::default();
    let mut at = 0;
    let mut stated = false;
    let mut two = false;
    let refuse = |at, why| Err(Refused { at, why });
    let inside = |start: u32, bytes: u32| {
        g.is_none_or(|g| {
            start >= g.tex_at
                && u64::from(start) + u64::from(bytes)
                    <= u64::from(g.tex_at) + u64::from(g.tex_bytes)
        })
    };
    while at < words.len() {
        let w = words[at];
        if w == STATE || w == STATE2 {
            let sw = if w == STATE2 { STATE2_WORDS } else { STATE_WORDS };
            let Some(block) = words.get(at + 1..=at + sw) else {
                return refuse(at, Why::Short);
            };
            match texture(block[2], block[3]) {
                Some((start, bytes)) if inside(start, bytes) => {}
                _ => return refuse(at, Why::Texture),
            }
            // unit 1's texture too, inside the program's memory as unit 0's
            if w == STATE2 {
                match texture(block[5], block[6]) {
                    Some((start, bytes)) if inside(start, bytes) => {}
                    _ => return refuse(at, Why::Texture),
                }
            }
            two = w == STATE2;
            stated = true;
            c.states += 1;
            if !(put(Piece::Word(w)) && block.iter().all(|&b| put(Piece::Word(b)))) {
                return Ok(c);
            }
            at += 1 + sw;
        } else if is_prim(w) {
            if !stated {
                return refuse(at, Why::NoState);
            }
            let n = w & 0xFF;
            if n < STRIP_CORNERS.0 || n > STRIP_CORNERS.1 {
                return refuse(at, Why::Corners);
            }
            let len = n as usize * corner_words_in(w, two);
            let Some(corners) = words.get(at + 1..=at + len) else {
                return refuse(at, Why::Short);
            };
            c.strips += 1;
            c.corners += n;
            if !(put(Piece::Word(w & 0xFFFF_FF00))
                && corners.iter().all(|&v| put(Piece::Word(v)))
                && put(Piece::End))
            {
                return Ok(c);
            }
            at += 1 + len;
        } else {
            return refuse(at, Why::Unknown);
        }
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    const G: Gfx = Gfx {
        width: 480,
        height: 640,
        tex_at: 0x80_0000,
        tex_bytes: 0x70_0000,
        tex: core::ptr::null_mut(),
        screen: core::ptr::null(),
        flags: 0,
    };

    /// A state block naming a 512 x 512 RGB565 page at `addr`.
    fn state(addr: u32) -> [u32; 8] {
        let tex0 = 0xA000_0000 | 6 << 24 | 6 << 20 | 2 << 13;
        [
            STATE,
            0x2222_0C00,
            0,
            tex0,
            0x0E00_0000 | addr >> 7,
            0x8608_6610,
            0,
            0,
        ]
    }

    #[test]
    fn a_strip_without_z_words_has_six_word_corners_and_keeps_its_bit() {
        let mut words = state(G.tex_at).to_vec();
        words.push(STRIP | NO_Z | 3);
        words.extend(0..3 * CORNER_WORDS_NO_Z as u32);
        words.extend(strip(3, 100));
        let (r, out) = run(&words);
        let c = r.expect("accepted");
        assert_eq!((c.strips, c.corners), (2, 6));
        assert_eq!(out[8], Piece::Word(STRIP | NO_Z));
        assert_eq!(out[9 + 3 * CORNER_WORDS_NO_Z], Piece::End);
        assert_eq!(out[10 + 3 * CORNER_WORDS_NO_Z], Piece::Word(STRIP));
        let mut runs = Vec::new();
        send_runs(&words, &mut runs, |r, ws| { r.push(ws.to_vec()); true }, |_| true);
        assert_eq!(runs[1], vec![STRIP | NO_Z]);
        assert_eq!(runs[2].len(), 3 * CORNER_WORDS_NO_Z);
    }

    fn strip(n: u32, first: u32) -> Vec<u32> {
        let mut v = vec![STRIP | n];
        v.extend((0..n * CORNER_WORDS as u32).map(|k| first + k));
        v
    }

    fn run(words: &[u32]) -> (Result<Counts, Refused>, Vec<Piece>) {
        let mut out = Vec::new();
        let r = check(words, &G);
        if r.is_ok() {
            let sent = send(words, |p| {
                out.push(p);
                true
            });
            assert_eq!(Ok(sent), r);
            // The runs give the same words and ends in the same order.
            let mut runs: Vec<Piece> = Vec::new();
            let counted = send_runs(
                words,
                &mut runs,
                |o, ws| {
                    o.extend(ws.iter().map(|&w| Piece::Word(w)));
                    true
                },
                |o| {
                    o.push(Piece::End);
                    true
                },
            );
            assert_eq!(Ok(counted), r);
            assert_eq!(runs, out);
        }
        (r, out)
    }

    #[test]
    fn a_two_texture_block_takes_nine_word_corners_and_checks_both_textures() {
        let mut b = vec![STATE2, 0x2225_0C00, 0xE000_0000];
        b.extend_from_slice(&state(G.tex_at)[3..6]);
        b.extend_from_slice(&state(G.tex_at + 0x1000)[3..6]);
        b.extend_from_slice(&[0, 0]);
        let mut f = b.clone();
        f.push(STRIP | 3);
        f.extend((0..27).map(|k| k as u32));
        let (r, _) = run(&f);
        assert_eq!(r.map(|c| (c.states, c.strips, c.corners)), Ok((1, 1, 3)));
        // 7-word corners after it are short by six words: refused
        let mut short = b.clone();
        short.push(STRIP | 3);
        short.extend((0..21).map(|k| k as u32));
        assert!(run(&short).0.is_err());
        // unit 1's texture outside the program's memory: refused
        let mut out = b;
        out[6] = state(G.tex_at + G.tex_bytes)[3];
        out[7] = state(G.tex_at + G.tex_bytes)[4];
        out.push(STRIP | 3);
        out.extend((0..27).map(|k| k as u32));
        assert!(run(&out).0.is_err());
    }

    #[test]
    fn a_frame_goes_through_word_for_word_each_strip_ended() {
        let mut f = state(0x80_0000).to_vec();
        f.extend(strip(4, 100));
        f.extend(strip(3, 0xF000_0000));
        let (r, out) = run(&f);
        assert_eq!(
            r,
            Ok(Counts {
                states: 1,
                strips: 2,
                corners: 7
            })
        );
        let mut want: Vec<Piece> = state(0x80_0000).iter().map(|&w| Piece::Word(w)).collect();
        want.push(Piece::Word(STRIP));
        want.extend((0..28).map(|k| Piece::Word(100 + k)));
        want.push(Piece::End);
        want.push(Piece::Word(STRIP));
        // A corner word equal to the TA's own end word is a word like any
        // other: the count, not a marker, ends the strip.
        want.extend((0..21).map(|k| Piece::Word(0xF000_0000 + k)));
        want.push(Piece::End);
        assert_eq!(out, want);
    }

    #[test]
    fn a_fan_goes_through_as_the_ta_s_fan_word() {
        let mut f = state(0x80_0000).to_vec();
        f.push(FAN | 5);
        f.extend((0..35).map(|k| 200 + k));
        f.extend(strip(3, 0));
        let (r, out) = run(&f);
        assert_eq!(
            r,
            Ok(Counts {
                states: 1,
                strips: 2,
                corners: 8
            })
        );
        assert_eq!(out[8], Piece::Word(FAN));
        assert_eq!(out[9], Piece::Word(200));
        assert_eq!(out[43], Piece::Word(234));
        assert_eq!(out[44], Piece::End);
        assert_eq!(out[45], Piece::Word(STRIP));
        // A fan's corner counts are the strip's.
        let mut g = state(0x80_0000).to_vec();
        g.push(FAN | 2);
        g.extend(0..14);
        assert_eq!(
            run(&g).0,
            Err(Refused {
                at: 8,
                why: Why::Corners
            })
        );
    }

    #[test]
    fn an_empty_frame_is_taken() {
        assert_eq!(run(&[]), (Ok(Counts::default()), vec![]));
    }

    #[test]
    fn a_refused_frame_is_named_by_its_first_wrong_word() {
        let cases: [(Vec<u32>, usize, Why); 7] = [
            (strip(3, 0), 0, Why::NoState),
            (
                [state(0x80_0000).to_vec(), vec![0x7000_0003]].concat(),
                8,
                Why::Unknown,
            ),
            (
                [state(0x80_0000).to_vec(), strip(2, 0)].concat(),
                8,
                Why::Corners,
            ),
            (
                [state(0x80_0000).to_vec(), strip(4, 0)[..20].to_vec()].concat(),
                8,
                Why::Short,
            ),
            (state(0x80_0000)[..5].to_vec(), 0, Why::Short),
            // Below the texture memory, and running past its end.
            (state(0x7F_FF80).to_vec(), 0, Why::Texture),
            (state(0xE8_0000 + 0x80).to_vec(), 0, Why::Texture),
        ];
        for (f, at, why) in cases {
            let (r, out) = run(&f);
            assert_eq!(r, Err(Refused { at, why }), "{f:x?}");
            assert!(out.is_empty(), "{f:x?}");
        }
        // The last page that fits is taken.
        assert!(run(&state(0xE8_0000)).0.is_ok());
    }

    #[test]
    fn a_texture_s_bytes_follow_its_size_codes_and_format() {
        let t0 = |cu: u32, cv: u32, f: u32| 0xA000_0000 | cu << 24 | cv << 20 | f << 13;
        assert_eq!(texture(t0(0, 0, 2), 0x0E00_0001), Some((0x80, 128)));
        assert_eq!(texture(t0(6, 6, 2), 0), Some((0, 512 * 512 * 2)));
        assert_eq!(texture(t0(7, 3, 4), 0), Some((0, 1024 * 64 * 2)));
        // 32-bit and compressed texels are not taken yet, nor a side past
        // 2,048.
        assert_eq!(texture(t0(3, 3, 12), 0), None);
        assert_eq!(texture(t0(3, 3, 28), 0), None);
        assert_eq!(texture(t0(9, 3, 2), 0), None);
    }

    #[test]
    fn a_stuck_chip_stops_the_feed() {
        let mut f = state(0x80_0000).to_vec();
        f.extend(strip(3, 0));
        let mut n = 0;
        assert!(check(&f, &G).is_ok());
        let sent = send(&f, |_| {
            n += 1;
            n < 5
        });
        assert_eq!(n, 5);
        assert_eq!(sent.states, 1);
    }
}
