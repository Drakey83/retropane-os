//! Calculator, version 2 ([calculator-rule0.md](../../../docs/design/calculator-rule0.md)):
//! an app program ([`rp_app::Program`]).
//!
//! Iterate, don't imitate (rule 0): Windows Mobile 5 worked each key as it
//! came (2 + 3 x 4 = 20) in binary floats, with memory keys and nothing
//! else; the iPhone keeps a history and converts units and money, Soulver
//! does tips and dates in words. Ours keeps version 1's exact decimals and
//! precedence, shows the whole sum with its answer live, keeps a tape to
//! reuse, brings back WM's memory keys, and adds scientific keys, units,
//! money at the European Central Bank's daily rates, tip and split, and
//! dates, each a mode one chip away.

#![no_std]
#![forbid(unsafe_code)]

use rp_app::kit::{self, DIM, TEXT, tint};
use rp_app::{Ctx, Event, Field, Frame, Key, MARGIN, Program, Rect, Reply, W, WIDE};

/// The modes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// The keypad.
    Basic,
    /// The scientific keys.
    Scientific,
    /// Units.
    Units,
    /// Money.
    Money,
    /// Tip and split.
    Tip,
    /// Dates.
    Dates,
}

const MODES: [&[u8]; 6] = [b"BASIC", b"SCI", b"UNITS", b"MONEY", b"TIP", b"DATES"];

/// The Calculator program.
pub struct Calculator {
    mode: Mode,
    /// The sum typed, and its answer so far.
    expr: &'static [u8],
    answer: &'static [u8],
    /// The tape: sums and answers worked before.
    tape: &'static [(&'static [u8], &'static [u8])],
    /// Memory holds a value.
    memory: bool,
}

const GRAPHITE: u32 = 0xF02A_2F36;
const GRAPHITE_HI: u32 = 0xF03A_414A;

impl Calculator {
    /// A calculator in `mode` showing `expr` with `answer` previewed, and
    /// `tape` above.
    #[must_use]
    pub fn new(
        mode: Mode,
        expr: &'static [u8],
        answer: &'static [u8],
        tape: &'static [(&'static [u8], &'static [u8])],
        memory: bool,
    ) -> Self {
        Self {
            mode,
            expr,
            answer,
            tape,
            memory,
        }
    }

    fn key(f: &mut Frame, cx: &Ctx, r: Rect, word: &[u8], kind: u8) {
        let (fill, col) = match kind {
            // Digits: graphite.
            0 => (GRAPHITE_HI, TEXT),
            // Operators: orange.
            1 => (tint::CALCULATOR, 0xFF10_1418),
            // The = key: the tint, brighter.
            2 => (0xFFFF_B35C, 0xFF10_1418),
            // Functions: darker graphite.
            _ => (GRAPHITE, DIM),
        };
        f.push(rp_app::Cmd::Fill { r, color: fill });
        let font = if word.len() <= 2 && word.iter().all(|c| b"0123456789.,-+x/=%".contains(c)) {
            cx.fonts.num
        } else {
            cx.fonts.ui
        };
        let base = r.y + r.h / 2 + u16::from(font.ascent) * 2 / 5;
        kit::centred(f, r, base, font, col, word);
    }

    fn display(&self, f: &mut Frame, cx: &Ctx, top: u16) -> u16 {
        // The tape, newest last.
        let mut y = top;
        for (sum, ans) in self.tape {
            kit::right(f, W - MARGIN - 12, y + 18, cx.fonts.ui, DIM, sum);
            kit::right(f, W - MARGIN - 12, y + 40, cx.fonts.ui, TEXT, ans);
            y += 46;
            f.push(rp_app::Cmd::Fill {
                r: Rect::new(MARGIN + 160, y - 3, WIDE - 172, 1),
                color: kit::alpha(TEXT, 0x30),
            });
        }
        let d = Rect::new(MARGIN, y + 4, WIDE, 92);
        f.push(rp_app::Cmd::Fill {
            r: d,
            color: 0xE010_1418,
        });
        if self.memory {
            kit::text(
                f,
                d.x + 12,
                d.y + 24,
                cx.fonts.label,
                tint::CALCULATOR,
                b"M",
            );
        }
        kit::right(f, d.right() - 12, d.y + 30, cx.fonts.ui, TEXT, self.expr);
        kit::right(
            f,
            d.right() - 12,
            d.y + 80,
            cx.fonts.num,
            kit::alpha(TEXT, 0xB0),
            self.answer,
        );
        d.bottom() + 6
    }

    fn keypad(&self, f: &mut Frame, cx: &Ctx, top: u16) {
        let rows: [[(&[u8], u8); 4]; 6] = match self.mode {
            Mode::Scientific => [
                [(b"sin", 3), (b"cos", 3), (b"tan", 3), (b"deg", 3)],
                [(b"ln", 3), (b"log", 3), (b"x^2", 3), (b"x^y", 3)],
                [(b"sqrt", 3), (b"pi", 3), (b"e", 3), (b"n!", 3)],
                [(b"(", 3), (b")", 3), (b"1/x", 3), (b"/", 1)],
                [(b"EE", 3), (b"abs", 3), (b"rand", 3), (b"x", 1)],
                [(b"inv", 3), (b"hyp", 3), (b"%", 3), (b"=", 2)],
            ],
            _ => [
                [(b"MC", 3), (b"MR", 3), (b"M+", 3), (b"M-", 3)],
                [(b"C", 3), (b"( )", 3), (b"%", 3), (b"/", 1)],
                [(b"7", 0), (b"8", 0), (b"9", 0), (b"x", 1)],
                [(b"4", 0), (b"5", 0), (b"6", 0), (b"-", 1)],
                [(b"1", 0), (b"2", 0), (b"3", 0), (b"+", 1)],
                [(b"+/-", 0), (b"0", 0), (b".", 0), (b"=", 2)],
            ],
        };
        let kh = (596 - top) / 6 - 4;
        for (r, row) in rows.iter().enumerate() {
            for (c, (word, kind)) in row.iter().enumerate() {
                let k = Rect::new(MARGIN + c as u16 * 113, top + r as u16 * (kh + 4), 108, kh);
                Self::key(f, cx, k, word, *kind);
            }
        }
    }

    fn convert(&self, f: &mut Frame, cx: &Ctx, top: u16, money: bool) {
        let rows: [(&[u8], &[u8], &[u8]); 2] = if money {
            [
                (b"US DOLLAR", b"USD", b"120.00"),
                (b"EURO", b"EUR", b"110.52"),
            ]
        } else {
            [
                (b"MILES", b"mi", b"26.2"),
                (b"KILOMETRES", b"km", b"42.165"),
            ]
        };
        let mut y = top;
        for (k, (name, unit, value)) in rows.iter().enumerate() {
            let r = Rect::new(MARGIN, y, WIDE, 86);
            kit::panel(f, cx, r);
            if k == 0 {
                f.push(rp_app::Cmd::Outline {
                    r,
                    color: kit::FOCUS,
                });
            }
            kit::text(f, r.x + 14, r.y + 24, cx.fonts.label, DIM, name);
            kit::chips(
                f,
                cx,
                Rect::new(r.x + 12, r.y + 40, 92, 32),
                &[unit],
                0,
                tint::CALCULATOR,
            );
            kit::right(f, r.right() - 14, r.y + 74, cx.fonts.num, TEXT, value);
            y += 92;
        }
        let note: &[u8] = if money {
            b"ECB rates of 14 Nov \x80 kept for offline"
        } else {
            b"Length \x80 tap a unit to change it"
        };
        kit::text(f, MARGIN + 8, y + 16, cx.fonts.ui, DIM, note);
        // The number pad under it.
        let pad: [[(&[u8], u8); 4]; 3] = [
            [(b"7", 0), (b"8", 0), (b"9", 0), (b"C", 3)],
            [(b"4", 0), (b"5", 0), (b"6", 0), (b"swap", 3)],
            [(b"1", 0), (b"2", 0), (b"3", 0), (b"0", 0)],
        ];
        let top = y + 28;
        let kh = (596 - top) / 3 - 4;
        for (r, row) in pad.iter().enumerate() {
            for (c, (w, k)) in row.iter().enumerate() {
                Self::key(
                    f,
                    cx,
                    Rect::new(MARGIN + c as u16 * 113, top + r as u16 * (kh + 4), 108, kh),
                    w,
                    *k,
                );
            }
        }
    }

    fn tip(&self, f: &mut Frame, cx: &Ctx, top: u16) {
        let bill = Rect::new(MARGIN, top, WIDE, 70);
        kit::panel(f, cx, bill);
        kit::text(f, bill.x + 14, bill.y + 24, cx.fonts.label, DIM, b"BILL");
        kit::right(
            f,
            bill.right() - 14,
            bill.y + 60,
            cx.fonts.num,
            TEXT,
            b"86.40",
        );
        kit::chips(
            f,
            cx,
            Rect::new(MARGIN, top + 80, WIDE, 40),
            &[b"15%", b"18%", b"20%", b"OTHER"],
            2,
            tint::CALCULATOR,
        );
        let people = Rect::new(MARGIN, top + 130, WIDE, 50);
        kit::panel(f, cx, people);
        kit::text(
            f,
            people.x + 14,
            people.y + 32,
            cx.fonts.ui,
            TEXT,
            b"People",
        );
        kit::right(
            f,
            people.right() - 14,
            people.y + 32,
            cx.fonts.ui,
            TEXT,
            b"<   3   >",
        );
        let mut y = top + 192;
        for (name, v) in [
            (&b"TIP"[..], &b"17.28"[..]),
            (b"TOTAL", b"103.68"),
            (b"EACH", b"34.56"),
        ] {
            let r = Rect::new(MARGIN, y, WIDE, 64);
            kit::tinted(
                f,
                cx,
                r,
                tint::CALCULATOR,
                if name == b"EACH" { 0x60 } else { 0x20 },
            );
            kit::text(f, r.x + 14, r.y + 38, cx.fonts.label, TEXT, name);
            kit::right(f, r.right() - 14, r.y + 54, cx.fonts.num, TEXT, v);
            y += 70;
        }
    }

    fn dates(&self, f: &mut Frame, cx: &Ctx, top: u16) {
        let mut y = top;
        for (label, v) in [
            (&b"FROM"[..], &b"Mon, Jan 1 2024"[..]),
            (b"TO", b"Wed, Dec 25 2024"),
        ] {
            let r = Rect::new(MARGIN, y, WIDE, 56);
            kit::panel(f, cx, r);
            kit::text(f, r.x + 14, r.y + 22, cx.fonts.label, DIM, label);
            kit::text(f, r.x + 14, r.y + 46, cx.fonts.ui, TEXT, v);
            kit::right(f, r.right() - 14, r.y + 36, cx.fonts.ui, DIM, b"<  >");
            y += 62;
        }
        let r = Rect::new(MARGIN, y + 6, WIDE, 120);
        kit::tinted(f, cx, r, tint::CALCULATOR, 0x50);
        kit::right(f, r.right() - 14, r.y + 56, cx.fonts.num, TEXT, b"359");
        kit::text(f, r.x + 14, r.y + 50, cx.fonts.title, TEXT, b"days");
        kit::text(
            f,
            r.x + 14,
            r.y + 92,
            cx.fonts.ui,
            TEXT,
            b"51 weeks and 2 days \x80 257 weekdays",
        );
        kit::text(
            f,
            r.x + 14,
            r.y + 112,
            cx.fonts.ui,
            DIM,
            b"or: a date plus a number of days",
        );
    }
}

/// The calculator as it first opens (the program's entry and the PC's
/// model).
#[must_use]
pub fn opened(_today: i32) -> Calculator {
    Calculator::new(Mode::Basic, b"", b"0", &[], false)
}

impl Program for Calculator {
    fn title(&self) -> &'static str {
        "Calculator"
    }

    fn softkeys(&self) -> (&'static str, &'static str) {
        ("Back", "Copy")
    }

    fn draw(&mut self, f: &mut Frame, cx: &Ctx) {
        kit::backdrop(f);
        let chosen = match self.mode {
            Mode::Basic => 0,
            Mode::Scientific => 1,
            Mode::Units => 2,
            Mode::Money => 3,
            Mode::Tip => 4,
            Mode::Dates => 5,
        };
        kit::chips(
            f,
            cx,
            Rect::new(8, 42, W - 16, 30),
            &MODES,
            chosen,
            tint::CALCULATOR,
        );
        let top = 80;
        match self.mode {
            Mode::Basic | Mode::Scientific => {
                let y = self.display(f, cx, top);
                self.keypad(f, cx, y);
            }
            Mode::Units => self.convert(f, cx, top, false),
            Mode::Money => self.convert(f, cx, top, true),
            Mode::Tip => self.tip(f, cx, top),
            Mode::Dates => self.dates(f, cx, top),
        }
    }

    fn event(&mut self, e: Event, _cx: &Ctx) -> Reply {
        match e {
            Event::Key(Key::SoftLeft) => Reply::Close,
            Event::Key(Key::Right) => {
                self.mode = match self.mode {
                    Mode::Basic => Mode::Scientific,
                    Mode::Scientific => Mode::Units,
                    Mode::Units => Mode::Money,
                    Mode::Money => Mode::Tip,
                    Mode::Tip => Mode::Dates,
                    Mode::Dates => Mode::Basic,
                };
                Reply::Redraw
            }
            _ => Reply::Nothing,
        }
    }

    fn keyboard(&self) -> Option<Field> {
        None
    }
}
