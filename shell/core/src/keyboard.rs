//! K1 compact, the on-screen keyboard (the owner's pick 2026-10-31,
//! `docs/design/mockups/keyboard-compact-sheet.jpg`): K1's layout (a number
//! row, QWERTY, shift and delete, ?123, @, space, ., Done, a strip with
//! Paste and From PC, the pressed key shown large above the stylus) at the
//! size a resistive screen needs: a stylus, not a fingertip. 30 px keys,
//! the panel 208 px, docked 6 px above the softkey bar so the field and the
//! list above stay in view.
//!
//! Iterate, don't imitate (rule 0): Windows Mobile's SIP was a 15 px panel
//! with no paste and no feedback; a modern phone keyboard is 364 px of
//! fingertip keys here. Ours keeps the phone keyboard's layout and feedback
//! at stylus size, and adds From PC: docked, the PC's clipboard comes
//! across, so a 36-character token is pasted, never typed.

use rp_draw::{Argb, Cmd, Font, Frame, Rect};

use super::{GLASS_EDGE, TEXT, W};

/// The panel: from just above the softkey bar's 6 px gap, 208 px tall.
pub(crate) const PANEL: Rect = Rect::new(0, 392, W, 208);
const KH: u16 = 30;
const G: u16 = 4;
/// The strip's top and the first key row's.
const STRIP_Y: u16 = PANEL.y + 4;
const ROWS_Y: u16 = PANEL.y + 32;

/// What a key does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Cap {
    /// Types this byte.
    Char(u8),
    /// The next letter upper case.
    Shift,
    /// Deletes the last character.
    Delete,
    /// Letters and symbols switch.
    Symbols,
    /// The field is done.
    Done,
    /// The shell's clipboard into the field.
    Paste,
    /// The dock PC's clipboard into the field.
    FromPc,
}

/// Keys in all (the strip's two chips last).
pub(crate) const CAPS: u8 = 10 + 10 + 9 + 9 + 5 + 2;

const LETTERS: [&[u8]; 3] = [b"1234567890", b"qwertyuiop", b"asdfghjkl"];
const SYMBOLS: [&[u8]; 3] = [b"1234567890", b"-/:;()$&@\"", b".,?!'#%+="];
const LOW_LETTERS: &[u8] = b"zxcvbnm";
const LOW_SYMBOLS: &[u8] = b"_*~<>[]";

/// The keyboard's own state.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Keyboard {
    /// The next letter upper case.
    pub shift: bool,
    /// The symbols layer is up.
    pub symbols: bool,
}

/// `n` keys over `x0..x0+w` with gaps: key `j`'s left and width.
const fn spread(x0: u16, w: u16, n: u16, j: u16) -> (u16, u16) {
    let l = x0 + j * (w + G) / n;
    let r = x0 + (j + 1) * (w + G) / n - G;
    (l, r - l)
}

impl Keyboard {
    /// Key `i`: what it does and where it is.
    pub(crate) fn cap(&self, i: u8) -> Option<(Cap, Rect)> {
        let row_y = |r: u16| ROWS_Y + r * (KH + G);
        let rows = if self.symbols { SYMBOLS } else { LETTERS };
        let mut k = u16::from(i);
        // The three full rows (the third indented by 22).
        for (r, keys) in rows.iter().enumerate() {
            let n = keys.len() as u16;
            if k < n {
                let (x0, w) = if r == 2 { (28, 424) } else { (6, 468) };
                let (x, kw) = spread(x0, w, n, k);
                let mut c = keys[usize::from(k)];
                if self.shift && !self.symbols {
                    c = c.to_ascii_uppercase();
                }
                return Some((Cap::Char(c), Rect::new(x, row_y(r as u16), kw, KH)));
            }
            k -= n;
        }
        // Shift, seven, delete.
        if k < 9 {
            let y = row_y(3);
            return Some(match k {
                0 => (Cap::Shift, Rect::new(6, y, 60, KH)),
                8 => (Cap::Delete, Rect::new(W - 66, y, 60, KH)),
                _ => {
                    let (x, kw) = spread(70, 340, 7, k - 1);
                    let low = if self.symbols {
                        LOW_SYMBOLS
                    } else {
                        LOW_LETTERS
                    };
                    let mut c = low[usize::from(k - 1)];
                    if self.shift && !self.symbols {
                        c = c.to_ascii_uppercase();
                    }
                    (Cap::Char(c), Rect::new(x, y, kw, KH))
                }
            });
        }
        k -= 9;
        // ?123, @, space, ., Done.
        if k < 5 {
            const BOTTOM: [(Cap, u16); 5] = [
                (Cap::Symbols, 62),
                (Cap::Char(b'@'), 40),
                (Cap::Char(b' '), 214),
                (Cap::Char(b'.'), 40),
                (Cap::Done, 94),
            ];
            let mut x = 6;
            for (j, (c, w)) in BOTTOM.iter().enumerate() {
                if j == usize::from(k) {
                    return Some((*c, Rect::new(x, row_y(4), *w, KH)));
                }
                x += w + G;
            }
        }
        k -= 5;
        match k {
            0 => Some((Cap::Paste, Rect::new(8, STRIP_Y, 92, 24))),
            1 => Some((Cap::FromPc, Rect::new(106, STRIP_Y, 92, 24))),
            _ => None,
        }
    }

    /// The key that types `c` (the tests type through it; upper case is
    /// Shift then the letter).
    #[cfg(test)]
    pub(crate) fn cap_of(&self, c: u8) -> Option<u8> {
        (0..CAPS).find(|&i| matches!(self.cap(i), Some((Cap::Char(b), _)) if b == c))
    }

    /// A key pressed: what it means for the field. Shift and the layers
    /// change here; the caller handles the rest.
    pub(crate) fn press(&mut self, cap: Cap) -> Cap {
        match cap {
            Cap::Shift => self.shift = !self.shift,
            Cap::Symbols => {
                self.symbols = !self.symbols;
                self.shift = false;
            }
            Cap::Char(_) => self.shift = false,
            _ => {}
        }
        cap
    }

    /// The panel, its keys, and the pressed key shown large above it.
    pub(crate) fn draw(
        &self,
        f: &mut Frame,
        (ui, label): (&'static Font, &'static Font),
        pressed: Option<u8>,
        focus: Option<u8>,
        accent: Argb,
    ) {
        // Dense: keys read over any wallpaper (the sunset's first preview,
        // 2026-10-31, lost them in the colour).
        f.push(Cmd::HGradient {
            r: PANEL,
            left: PANEL_LEFT,
            right: PANEL_RIGHT,
        });
        f.push(Cmd::Fill {
            r: Rect::new(0, PANEL.y, W, 1),
            color: GLASS_EDGE,
        });
        for i in 0..CAPS {
            let Some((cap, r)) = self.cap(i) else {
                continue;
            };
            let lit = (cap == Cap::Shift && self.shift) || (cap == Cap::Symbols && self.symbols);
            let fill = if cap == Cap::Done {
                accent
            } else if pressed == Some(i) || lit {
                KEY_DOWN
            } else {
                KEY
            };
            // The 4 px gaps between keys show the panel: no outline needed,
            // which keeps the keyboard to two commands a key (the frame
            // holds 160).
            f.push(Cmd::Fill { r, color: fill });
            let ch;
            let (font, text): (&'static Font, &[u8]) = match cap {
                Cap::Char(b' ') => (label, b"space"),
                Cap::Char(c) => {
                    ch = [c];
                    (ui, &ch)
                }
                Cap::Shift => (label, b"SHIFT"),
                Cap::Delete => (label, b"DEL"),
                Cap::Symbols if self.symbols => (label, b"ABC"),
                Cap::Symbols => (label, b"?123"),
                Cap::Done => (label, b"DONE"),
                Cap::Paste => (label, b"PASTE"),
                Cap::FromPc => (label, b"FROM PC"),
            };
            let color = if cap == Cap::Done { DONE_TEXT } else { TEXT };
            centre_in(f, r, font, color, text);
            if focus == Some(i) {
                f.push(Cmd::Outline { r, color: accent });
            }
        }
        // The pressed letter large above the stylus (it hides the key).
        if let Some(i) = pressed
            && let Some((Cap::Char(c), r)) = self.cap(i)
            && c != b' '
        {
            let pop = Rect::new(
                (r.x + r.w / 2).saturating_sub(22),
                r.y.saturating_sub(48),
                44,
                54,
            );
            f.push(Cmd::Fill { r: pop, color: POP });
            f.push(Cmd::Outline {
                r: pop,
                color: GLASS_EDGE,
            });
            centre_in(f, Rect::new(pop.x, pop.y, pop.w, 40), ui, TEXT, &[c]);
        }
    }

    /// Where the pressed key's enlargement goes (to redraw it and what it
    /// covered).
    pub(crate) fn pop_rect(&self, i: u8) -> Option<Rect> {
        let (_, r) = self.cap(i)?;
        Some(Rect::new(
            (r.x + r.w / 2).saturating_sub(22),
            r.y.saturating_sub(48),
            44,
            r.y + r.h - r.y.saturating_sub(48),
        ))
    }
}

/// `text` centred in `r`, by its capitals' height (the owner, 2026-10-31:
/// letters sat low in their keys).
pub(crate) fn centre_in(f: &mut Frame, r: Rect, font: &'static Font, color: Argb, text: &[u8]) {
    let w = font.width(text);
    let cap_h = u16::from(font.ascent) * 7 / 10;
    let base = r.y + (r.h + cap_h) / 2;
    f.text(r.x + r.w.saturating_sub(w) / 2, base, font, color, text);
}

const PANEL_LEFT: Argb = rp_draw::look::theme(0xF40C_1014);
const PANEL_RIGHT: Argb = rp_draw::look::theme(0xF020_262C);
const KEY: Argb = rp_draw::look::theme(0xFF3A_424C);
/// A key's fill, for buttons that look like keys.
pub(crate) const KEY_FILL: Argb = KEY;
const KEY_DOWN: Argb = rp_draw::look::theme(0xFF6A_7480);
const POP: Argb = rp_draw::look::theme(0xF040_4850);
pub(crate) const DONE_TEXT: Argb = rp_draw::look::theme(0xFF06_121C);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_is_inside_the_panel_and_none_overlap() {
        for kb in [
            Keyboard::default(),
            Keyboard {
                shift: true,
                symbols: false,
            },
            Keyboard {
                shift: false,
                symbols: true,
            },
        ] {
            let caps: Vec<Rect> = (0..CAPS).map(|i| kb.cap(i).expect("every key").1).collect();
            assert!(kb.cap(CAPS).is_none());
            for (i, a) in caps.iter().enumerate() {
                assert!(
                    a.x + a.w <= W && a.y >= PANEL.y && a.y + a.h <= PANEL.y + PANEL.h,
                    "{i}: {a:?}"
                );
                // 10 px clear of the softkey bar (the owner's mark, 2026-10-31).
                assert!(a.y + a.h + 10 <= super::super::H - 40, "{i}");
                for b in &caps[i + 1..] {
                    assert!(
                        a.intersect(b).w == 0 || a.intersect(b).h == 0,
                        "{a:?} {b:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn every_printable_key_character_can_be_typed() {
        let kb = Keyboard::default();
        let sym = Keyboard {
            shift: false,
            symbols: true,
        };
        for c in b"abcdefghijklmnopqrstuvwxyz0123456789@. -_" {
            assert!(
                kb.cap_of(*c).is_some() || sym.cap_of(*c).is_some(),
                "{}",
                *c as char
            );
        }
    }
}
