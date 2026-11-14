//! The calculator's screen (docs/design/calculator-rule0.md): the sum so
//! far above the number, and twenty keys a thumb can hit. The arithmetic
//! and the keys' behaviour are `rp_pim::calc`.
//!
//! Iterate (rule 0): Windows Mobile's showed one line and worked each key
//! at once (2 + 3 x 4 = 20) in binary floating point; ours shows the sum,
//! follows precedence, and adds in exact decimals.

use rp_draw::{Cmd, Frame, Rect};
use rp_pim::calc::{Key, Op};

use super::{Act, DIM, FOCUS, MARGIN, TEXT, TOP, WIDE};
use crate::{PRESSED_UNDER, Shell, keyboard, panel};

/// The display: the sum's line and the number.
pub(crate) const DISPLAY: Rect = Rect::new(MARGIN, TOP + 4, WIDE, 96);
/// The keys: four columns of 106 with 8 between, five rows of 70.
const KEYS_Y: u16 = DISPLAY.y + DISPLAY.h + 8;
const KEY_W: u16 = 106;
const KEY_H: u16 = 70;
const GAP: u16 = 8;

/// The keys, row by row: what each does and its word.
const KEYS: [(Key, &[u8]); 20] = [
    (Key::Clear, b"C"),
    (Key::Delete, b"DEL"),
    (Key::Percent, b"%"),
    (Key::Op(Op::Div), b"/"),
    (Key::Digit(7), b"7"),
    (Key::Digit(8), b"8"),
    (Key::Digit(9), b"9"),
    (Key::Op(Op::Mul), b"x"),
    (Key::Digit(4), b"4"),
    (Key::Digit(5), b"5"),
    (Key::Digit(6), b"6"),
    (Key::Op(Op::Sub), b"-"),
    (Key::Digit(1), b"1"),
    (Key::Digit(2), b"2"),
    (Key::Digit(3), b"3"),
    (Key::Op(Op::Add), b"+"),
    (Key::Negate, b"+/-"),
    (Key::Digit(0), b"0"),
    (Key::Point, b"."),
    (Key::Equals, b"="),
];

#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
fn key_rect(k: u16) -> Rect {
    Rect::new(
        MARGIN + k % 4 * (KEY_W + GAP),
        KEYS_Y + k / 4 * (KEY_H + GAP),
        KEY_W,
        KEY_H,
    )
}

/// Where `key` is.
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
pub(crate) fn key_place(key: Key) -> Option<Rect> {
    KEYS.iter()
        .position(|(k, _)| *k == key)
        .map(|k| key_rect(k as u16))
}

impl Shell {
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn calculator_targets(&mut self) {
        for (k, (key, _)) in KEYS.iter().enumerate() {
            self.app_target(key_rect(k as u16), Act::Key(*key));
        }
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub(crate) fn calculator_scene(&self, f: &mut Frame) {
        let (ui, label, numerals) = (self.fonts.ui, self.fonts.label, self.fonts.num);
        panel::draw(f, DISPLAY, self.light());
        // The sum so far: its tail when it is longer than the line.
        let mut line = [0u8; 96];
        let n = self.apps.calc.line(&mut line);
        let room = DISPLAY.w - 28;
        let from = (0..n).find(|&k| ui.width(&line[k..n]) <= room).unwrap_or(n);
        self.text_right(
            f,
            DISPLAY.x + DISPLAY.w - 14,
            DISPLAY.y + 30,
            ui,
            DIM,
            &line[from..n],
        );
        let mut number = [0u8; 32];
        match self.apps.calc.number(&mut number) {
            Ok(n) => self.text_right(
                f,
                DISPLAY.x + DISPLAY.w - 14,
                DISPLAY.y + 80,
                numerals,
                TEXT,
                &number[..n],
            ),
            Err(e) => self.text_right(
                f,
                DISPLAY.x + DISPLAY.w - 14,
                DISPLAY.y + 74,
                ui,
                TEXT,
                e.says().as_bytes(),
            ),
        }
        for (k, (key, word)) in KEYS.iter().enumerate() {
            let r = key_rect(k as u16);
            let act = Act::Key(*key);
            let equals = *key == Key::Equals;
            if equals {
                f.push(Cmd::Fill {
                    r,
                    color: if self.app_pressed(act) {
                        PRESSED_UNDER
                    } else {
                        FOCUS
                    },
                });
            } else {
                self.row_glass(f, r, act);
            }
            // Digits and signs in the numerals; C and DEL as words.
            let (font, color) = match key {
                Key::Clear | Key::Delete => (label, TEXT),
                Key::Equals if !self.app_pressed(act) => (numerals, keyboard::DONE_TEXT),
                Key::Op(_) | Key::Percent => (numerals, FOCUS),
                _ => (numerals, TEXT),
            };
            keyboard::centre_in(f, r, font, color, word);
            self.app_marks(f, act);
        }
    }
}
