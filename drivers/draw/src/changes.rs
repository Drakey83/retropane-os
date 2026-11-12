//! Where a frame differs from the one drawn before it, found from the two
//! draw lists (R191, the owner's list of 2026-11-11).
//!
//! The shell marks damage by hand: a state change says which rectangle it
//! touches. A change of data that may show anywhere (a picture decoded, a
//! forecast answered) can't say, so it marked the whole screen: the X51v
//! drew Today whole for every icon at start-up and Settings whole for every
//! forecast and radar answer, 36-52 ms each, though nothing on Settings
//! changed (run today-0115). Windows Mobile's GWES has the same split (the
//! caller's `InvalidateRect`, else the whole window). Today's retained
//! renderers find it from the lists instead (Chromium's raster
//! invalidation, Flutter's layer diff, Android's damage from render
//! nodes): this is their idea at this list's size. Each command gets a
//! key, a 64-bit hash of everything it draws with (its fields, its text's
//! bytes, the clip it is drawn in) and the pixels it may touch; the lists
//! are compared from both ends, and only the commands between the first
//! and last difference mark their areas. 256 commands at most, about
//! 0.1 ms on the X51v; a frame whose list didn't change draws nothing.
//!
//! The keys are words (four a command), so an image keeps its lists in
//! its RAM pool, outside its zeroed RAM budget (two lists, 8 KB: the
//! budget had 1.1 KB left).
//!
//! What it can't see: pixels a command reads from outside the list (the
//! wallpaper or radar map under it, a picture's pixels behind an unchanged
//! handle). Those changes keep marking their own damage; a picture's
//! handle carries a generation ([`crate::pics::Pic::key`]), so new pixels
//! are a new handle.

use crate::{Cmd, Frame, Rect};

/// Words a command's key takes: the hash of what it draws (two), and the
/// pixels it may touch inside its clip (two: x and y, w and h).
pub const KEY_WORDS: usize = 4;

/// The pixels a key says its command may touch.
fn area(k: &[u32]) -> Rect {
    Rect::new(
        k[2] as u16,
        (k[2] >> 16) as u16,
        k[3] as u16,
        (k[3] >> 16) as u16,
    )
}

/// FNV-1a, 64 bits.
struct Hash(u64);

impl Hash {
    const fn new() -> Self {
        Self(0xCBF2_9CE4_8422_2325)
    }
    fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 = (self.0 ^ u64::from(x)).wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    fn rect(&mut self, r: Rect) {
        self.u32(u32::from(r.x) | u32::from(r.y) << 16);
        self.u32(u32::from(r.w) | u32::from(r.h) << 16);
    }
    /// A `'static` table's address: the same table, the same drawing.
    fn addr<T>(&mut self, p: &T) {
        self.bytes(&(core::ptr::from_ref(p) as usize).to_le_bytes());
    }
}

/// `f`'s commands' keys into `out` ([`KEY_WORDS`] each), in drawing order
/// (`Clip`s fold into the commands after them): how many commands. `None`
/// when `out` is too small or the frame dropped commands: its list doesn't
/// say what it shows.
#[must_use]
pub fn cmd_keys(f: &Frame, out: &mut [u32]) -> Option<usize> {
    if f.overflowed() {
        return None;
    }
    let mut n = 0;
    for (c, clip) in f.cmds_clipped() {
        let slot = out.get_mut(n * KEY_WORDS..(n + 1) * KEY_WORDS)?;
        let mut h = Hash::new();
        h.rect(clip);
        match *c {
            Cmd::Fill { r, color } => {
                h.u32(1);
                h.rect(r);
                h.u32(color);
            }
            Cmd::Outline { r, color } => {
                h.u32(2);
                h.rect(r);
                h.u32(color);
            }
            Cmd::HGradient { r, left, right } => {
                h.u32(3);
                h.rect(r);
                h.u32(left);
                h.u32(right);
            }
            Cmd::Image { x, y, img } => {
                h.u32(4);
                h.u32(u32::from(x) | u32::from(y) << 16);
                h.addr(img);
            }
            Cmd::Pic { x, y, pic } => {
                h.u32(5);
                h.u32(u32::from(x) | u32::from(y.cast_unsigned()) << 16);
                h.u32(pic.key());
                h.u32(u32::from(pic.w) | u32::from(pic.h) << 16);
            }
            Cmd::Text {
                x,
                baseline,
                font,
                color,
                text,
            } => {
                h.u32(6);
                h.u32(u32::from(x) | u32::from(baseline) << 16);
                h.addr(font);
                h.u32(color);
                let t = f.text_of(text);
                h.u32(t.len() as u32);
                h.bytes(t);
            }
            Cmd::Panel { r, art } => {
                h.u32(7);
                h.rect(r);
                h.addr(art);
            }
            // Folded into the commands after it by `cmds_clipped`.
            Cmd::Clip { .. } => continue,
        }
        let b = c.bounds(f).intersect(&clip);
        slot.copy_from_slice(&[
            h.0 as u32,
            (h.0 >> 32) as u32,
            u32::from(b.x) | u32::from(b.y) << 16,
            u32::from(b.w) | u32::from(b.h) << 16,
        ]);
        n += 1;
    }
    Some(n)
}

/// Each area where a frame drawn from `new` differs from one drawn from
/// `old` (keys from [`cmd_keys`]), passed to `mark`: the commands both lists share at the start and
/// at the end are matched, and every command between (of either list)
/// marks the pixels it may touch. Lists of one length are matched command
/// by command, so a change in the middle marks only what changed.
///
/// Sound because a pixel no marked command touches is drawn by matched
/// commands alone, the same ones in the same order in both frames.
pub fn changed_areas(old: &[u32], new: &[u32], mut mark: impl FnMut(Rect)) {
    let (old, new) = (
        old.as_chunks::<KEY_WORDS>().0,
        new.as_chunks::<KEY_WORDS>().0,
    );
    if old.len() == new.len() {
        for (a, b) in old.iter().zip(new) {
            if a != b {
                mark(area(a));
                mark(area(b));
            }
        }
        return;
    }
    let head = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let tail = old[head..]
        .iter()
        .rev()
        .zip(new[head..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    for k in &old[head..old.len() - tail] {
        mark(area(k));
    }
    for k in &new[head..new.len() - tail] {
        mark(area(k));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Font, Frame, Glyph, NO_CLIP};

    static A: Glyph = Glyph {
        w: 2,
        h: 3,
        left: 0,
        top: 3,
        advance: 4,
        cover: &[255; 6],
    };
    static FONT: Font = Font {
        line: 10,
        ascent: 8,
        first: b'A',
        glyphs: &[A, A, A],
    };

    fn keys(f: &Frame) -> std::vec::Vec<u32> {
        let mut out = [0; crate::MAX_CMDS * KEY_WORDS];
        let n = cmd_keys(f, &mut out).expect("room");
        out[..n * KEY_WORDS].to_vec()
    }
    fn marked(old: &Frame, new: &Frame) -> std::vec::Vec<Rect> {
        let mut v = std::vec::Vec::new();
        changed_areas(&keys(old), &keys(new), |r| v.push(r));
        v.retain(|r| !r.is_empty());
        v
    }
    fn fill(f: &mut Frame, x: u16, color: u32) {
        f.push(Cmd::Fill {
            r: Rect::new(x, 0, 10, 10),
            color,
        });
    }

    #[test]
    fn an_unchanged_list_marks_nothing_and_a_changed_command_only_itself() {
        let (mut a, mut b) = (Frame::new(), Frame::new());
        for f in [&mut a, &mut b] {
            fill(f, 0, 1);
            fill(f, 20, 2);
            fill(f, 40, 3);
        }
        assert!(marked(&a, &b).is_empty());
        b.clear();
        fill(&mut b, 0, 1);
        fill(&mut b, 20, 9);
        fill(&mut b, 40, 3);
        assert_eq!(marked(&a, &b), [Rect::new(20, 0, 10, 10); 2]);
    }

    #[test]
    fn a_command_added_or_taken_away_marks_only_its_area() {
        let (mut a, mut b) = (Frame::new(), Frame::new());
        fill(&mut a, 0, 1);
        fill(&mut a, 40, 3);
        fill(&mut b, 0, 1);
        fill(&mut b, 20, 2);
        fill(&mut b, 40, 3);
        assert_eq!(marked(&a, &b), [Rect::new(20, 0, 10, 10)]);
        assert_eq!(marked(&b, &a), [Rect::new(20, 0, 10, 10)]);
    }

    #[test]
    fn the_same_text_place_with_other_words_or_another_clip_is_a_change() {
        let font = &FONT;
        let (mut a, mut b) = (Frame::new(), Frame::new());
        a.text(0, 20, font, 1, b"AB");
        b.text(0, 20, font, 1, b"AC");
        assert_eq!(
            marked(&a, &b).len(),
            2,
            "same text store place, other bytes"
        );
        let (mut a, mut b) = (Frame::new(), Frame::new());
        a.clip(Some(Rect::new(0, 0, 5, 5)));
        fill(&mut a, 0, 1);
        b.clip(Some(NO_CLIP));
        fill(&mut b, 0, 1);
        assert_eq!(
            marked(&a, &b),
            [Rect::new(0, 0, 5, 5), Rect::new(0, 0, 10, 10)]
        );
    }
}
