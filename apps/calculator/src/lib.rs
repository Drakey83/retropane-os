//! Calculator, version 2 ([calculator-rule0.md](../../../docs/design/calculator-rule0.md)):
//! an app program ([`rp_app::Program`]).
//!
//! Iterate, don't imitate (rule 0): Windows Mobile 5 worked each key as it
//! came (2 + 3 x 4 = 20) in binary floats, with memory keys and nothing
//! else; the iPhone keeps a history and converts units and money, Soulver
//! does tips and dates in words. Ours keeps the whole sum as typed and
//! works it with school precedence on exact decimals (`rp_pim::expr`),
//! shows the answer live, keeps a tape to reuse, brings back WM's memory
//! keys, and adds scientific keys (our own series: the X51v has no FPU),
//! units, money at the European Central Bank's daily rates, tip and split,
//! and dates, each a mode one chip away.

#![no_std]
#![forbid(unsafe_code)]

use rp_app::kit::{self, DIM, TEXT, tint};
use rp_app::{Ctx, Event, Field, Frame, Key, MARGIN, Program, Rect, Reply, Text, W, WIDE};
use rp_pim::calc::Dec;
use rp_pim::store::{Kind, tag};
use rp_pim::{date, expr};

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
const MODE_LIST: [Mode; 6] = [
    Mode::Basic,
    Mode::Scientific,
    Mode::Units,
    Mode::Money,
    Mode::Tip,
    Mode::Dates,
];
const CHIPS: Rect = Rect::new(8, 42, W - 16, 30);
const TOP: u16 = 80;
const BOTTOM: u16 = 596;

/// A key: its face, what it types (or does), and its kind (0 digit, 1
/// operator, 2 equals, 3 function).
type KeyDef = (&'static [u8], &'static [u8], u8);

const BASIC: [[KeyDef; 4]; 6] = [
    [
        (b"MC", b"MC", 3),
        (b"MR", b"MR", 3),
        (b"M+", b"M+", 3),
        (b"M-", b"M-", 3),
    ],
    [
        (b"C", b"C", 3),
        (b"( )", b"()", 3),
        (b"%", b"%", 3),
        (b"/", b"/", 1),
    ],
    [
        (b"7", b"7", 0),
        (b"8", b"8", 0),
        (b"9", b"9", 0),
        (b"x", b"x", 1),
    ],
    [
        (b"4", b"4", 0),
        (b"5", b"5", 0),
        (b"6", b"6", 0),
        (b"-", b"-", 1),
    ],
    [
        (b"1", b"1", 0),
        (b"2", b"2", 0),
        (b"3", b"3", 0),
        (b"+", b"+", 1),
    ],
    [
        (b"+/-", b"NEG", 0),
        (b"0", b"0", 0),
        (b".", b".", 0),
        (b"=", b"=", 2),
    ],
];

const SCIENTIFIC: [[KeyDef; 4]; 6] = [
    [
        (b"sin", b"s(", 3),
        (b"cos", b"c(", 3),
        (b"tan", b"t(", 3),
        (b"DEG", b"ANG", 3),
    ],
    [
        (b"ln", b"l(", 3),
        (b"log", b"L(", 3),
        (b"x^2", b"^2", 3),
        (b"x^y", b"^", 3),
    ],
    [
        (b"sqrt", b"q(", 3),
        (b"pi", b"p", 3),
        (b"e", b"e", 3),
        (b"n!", b"!", 3),
    ],
    [
        (b"(", b"(", 3),
        (b")", b")", 3),
        (b"1/x", b"^~1", 3),
        (b"/", b"/", 1),
    ],
    [
        (b"EE", b"x10^", 3),
        (b"INV", b"INV", 3),
        (b"DEL", b"DEL", 3),
        (b"x", b"x", 1),
    ],
    [
        (b"C", b"C", 3),
        (b"-", b"-", 1),
        (b"+", b"+", 1),
        (b"=", b"=", 2),
    ],
];

/// A unit: its name and its size in its category's base unit.
type Unit = (&'static [u8], &'static [u8]);

/// Units: the categories, and in each, units with their size in the base
/// unit (temperatures are worked apart).
const UNITS: [(&[u8], [Unit; 4]); 6] = [
    (
        b"LENGTH",
        [
            (b"mi", b"1609.344"),
            (b"km", b"1000"),
            (b"ft", b"0.3048"),
            (b"m", b"1"),
        ],
    ),
    (
        b"WEIGHT",
        [
            (b"lb", b"0.45359237"),
            (b"kg", b"1"),
            (b"oz", b"0.028349523125"),
            (b"g", b"0.001"),
        ],
    ),
    (
        b"VOLUME",
        [
            (b"gal", b"3.785411784"),
            (b"l", b"1"),
            (b"cup", b"0.2365882365"),
            (b"ml", b"0.001"),
        ],
    ),
    (
        b"TEMP",
        [(b"F", b""), (b"C", b""), (b"K", b""), (b"F", b"")],
    ),
    (
        b"SPEED",
        [
            (b"mph", b"0.44704"),
            (b"km/h", b"0.277777777778"),
            (b"knot", b"0.514444444444"),
            (b"m/s", b"1"),
        ],
    ),
    (
        b"DATA",
        [
            (b"MB", b"1000000"),
            (b"MiB", b"1048576"),
            (b"GB", b"1000000000"),
            (b"KB", b"1000"),
        ],
    ),
];

/// Tape lines kept (each about 70 bytes; three show, a drag scrolls).
const TAPE_MAX: usize = 24;
/// Tape lines shown at once.
const TAPE_SHOWN: usize = 3;
/// A tape line's height.
const TAPE_LINE: u16 = 46;
/// A hold this many ticks (about 30 ms each: half a second) copies.
const HOLD_TICKS: u8 = 16;
/// The COPIED mark shows this many ticks (about a second).
const COPIED_TICKS: u8 = 32;
/// A stylus that moves more than this (pixels) scrolls, not taps.
const SLOP: u16 = 10;

/// The stylus on the tape.
#[derive(Clone, Copy)]
struct TapePen {
    y0: u16,
    back0: usize,
    line: Option<usize>,
    moved: bool,
    held: u8,
}

/// The Calculator program.
pub struct Calculator {
    mode: Mode,
    /// The sum as typed, in `rp_pim::expr`'s letters.
    expr: [u8; expr::MAX],
    n: usize,
    /// The tape: sums worked and their answers, newest last.
    tape: [(Text<40>, Text<24>); TAPE_MAX],
    n_tape: usize,
    /// Lines scrolled back from the newest (a drag on the tape).
    back: usize,
    /// The stylus on the tape: where it went down, the scroll then, the
    /// line under it, whether it moved and the ticks it has been held.
    pen: Option<TapePen>,
    /// A line copied by a hold, and the ticks its COPIED mark shows.
    copied: Option<(usize, u8)>,
    memory: Option<Dec>,
    radians: bool,
    inverse: bool,
    /// Units: the category and the from and to units; the value typed.
    cat: usize,
    from: usize,
    to: usize,
    /// Tip: the bill typed, the percent chosen, the people.
    tip_pct: u8,
    people: u8,
    /// Dates: the two days, and which one the keys move.
    d0: i32,
    d1: i32,
    /// A number typed in the units, money and tip modes.
    entry: [u8; 16],
    entry_n: usize,
    /// Money: the rates (units per euro, millionths), their day, whether
    /// the store was read for them and whether to ask the network; the
    /// currencies converted between (indices in the rates, the euro last).
    rates: [([u8; 3], u64); 40],
    n_rates: usize,
    rates_day: Option<i32>,
    rates_read: bool,
    want_rates: bool,
    cur_from: usize,
    cur_to: usize,
}

/// The currencies offered first, the US default first.
const CURRENCIES: [&[u8; 3]; 8] = [
    b"USD", b"EUR", b"GBP", b"CAD", b"MXN", b"JPY", b"AUD", b"CHF",
];

impl Calculator {
    /// A calculator in `mode` on `today`.
    #[must_use]
    pub fn new(mode: Mode, today: i32) -> Self {
        Self {
            mode,
            expr: [0; expr::MAX],
            n: 0,
            tape: [(Text::EMPTY, Text::EMPTY); TAPE_MAX],
            n_tape: 0,
            back: 0,
            pen: None,
            copied: None,
            memory: None,
            radians: false,
            inverse: false,
            cat: 0,
            from: 0,
            to: 1,
            tip_pct: 20,
            people: 2,
            d0: today,
            d1: today,
            entry: [0; 16],
            entry_n: 0,
            rates: [([0; 3], 0); 40],
            n_rates: 0,
            rates_day: None,
            rates_read: false,
            want_rates: false,
            cur_from: 0,
            cur_to: 1,
        }
    }

    /// The rates read from the ECB's daily file (the network's answer,
    /// and the previews' canned one); the euro added as 1.
    pub fn set_rates_from(&mut self, answer: &[u8]) -> bool {
        let mut found = [rp_pim::net::Rate {
            code: [0; 3],
            micro: 0,
        }; 39];
        let (n, day) = rp_pim::net::ecb(answer, &mut found);
        if n == 0 {
            return false;
        }
        for (slot, r) in self.rates.iter_mut().zip(&found[..n]) {
            *slot = (r.code, r.micro);
        }
        self.rates[n] = (*b"EUR", 1_000_000);
        self.n_rates = n + 1;
        self.rates_day = day;
        true
    }

    /// The rates kept in the store as one record (`Data` "rates": the day,
    /// then `CODE=millionths;` each), read once.
    fn rates_kept(&mut self, cx: &Ctx) {
        if self.rates_read {
            return;
        }
        self.rates_read = true;
        let Some(cell) = cx.pim else { return };
        let Ok(guard) = cell.try_borrow() else { return };
        let Some(store) = guard.as_ref() else { return };
        let Some(r) = store
            .all(Kind::Data)
            .find(|r| r.text(tag::NAME) == b"rates")
        else {
            return;
        };
        let t = r.text(tag::TEXT);
        let mut n = 0;
        for part in t.split(|&c| c == b';') {
            if let Some(day) = part.strip_prefix(b"day=") {
                self.rates_day = day.iter().try_fold(0i32, |v, &c| {
                    c.is_ascii_digit().then(|| v * 10 + i32::from(c - b'0'))
                });
            } else if part.len() > 4 && part[3] == b'=' && n < self.rates.len() {
                let v = part[4..].iter().try_fold(0u64, |v, &c| {
                    c.is_ascii_digit().then(|| v * 10 + u64::from(c - b'0'))
                });
                if let Some(v) = v {
                    self.rates[n] = ([part[0], part[1], part[2]], v);
                    n += 1;
                }
            }
        }
        self.n_rates = n;
    }

    /// The rates kept in the store for offline use.
    fn keep_rates(&self, cx: &Ctx) {
        let mut t = [0u8; 1200];
        let mut k = 0;
        let mut put = |b: &[u8], k: &mut usize| {
            for &c in b {
                if *k < t.len() {
                    t[*k] = c;
                    *k += 1;
                }
            }
        };
        if let Some(d) = self.rates_day {
            put(b"day=", &mut k);
            let mut b = [0u8; 12];
            let n = num(&mut b, d.max(0) as u32);
            put(&b[..n], &mut k);
            put(b";", &mut k);
        }
        for (code, v) in &self.rates[..self.n_rates] {
            put(code, &mut k);
            put(b"=", &mut k);
            let mut b = [0u8; 20];
            let mut i = b.len();
            let mut v = *v;
            loop {
                i -= 1;
                b[i] = b'0' + (v % 10) as u8;
                v /= 10;
                if v == 0 {
                    break;
                }
            }
            put(&b[i..], &mut k);
            put(b";", &mut k);
        }
        let Some(cell) = cx.pim else { return };
        let Ok(mut guard) = cell.try_borrow_mut() else {
            return;
        };
        let Some(store) = guard.as_mut() else { return };
        let id = store
            .all(Kind::Data)
            .find(|r| r.text(tag::NAME) == b"rates")
            .map(|r| r.id);
        let _ = store.put(
            Kind::Data,
            id,
            &[(tag::NAME, b"rates"), (tag::TEXT, &t[..k])],
        );
    }

    /// Currency `code`'s rate (units per euro, millionths).
    fn rate(&self, code: &[u8; 3]) -> Option<u64> {
        self.rates[..self.n_rates]
            .iter()
            .find(|(c, _)| c == code)
            .map(|(_, v)| *v)
    }

    /// `v` in currency `from` as currency `to`, through the euro.
    fn exchange(&self, v: Dec, from: &[u8; 3], to: &[u8; 3]) -> Option<Dec> {
        let (a, b) = (self.rate(from)?, self.rate(to)?);
        let micro = |m: u64| Dec::whole(m as i64).over(Dec::whole(1_000_000)).ok();
        v.over(micro(a)?).ok()?.times(micro(b)?).ok()
    }

    /// Types `keys` (faces' actions, as a tap would): the previews.
    pub fn type_keys(&mut self, keys: &[&[u8]]) {
        for k in keys {
            self.press(k);
        }
    }

    /// Sets the number typed in units, money and tip (the previews).
    pub fn set_entry(&mut self, s: &[u8]) {
        let n = s.len().min(self.entry.len());
        self.entry[..n].copy_from_slice(&s[..n]);
        self.entry_n = n;
    }

    /// Moves the second date `days` on (the previews).
    pub fn set_dates(&mut self, d0: i32, d1: i32) {
        self.d0 = d0;
        self.d1 = d1;
    }

    fn push(&mut self, s: &[u8]) {
        for &c in s {
            if self.n < self.expr.len() {
                self.expr[self.n] = c;
                self.n += 1;
            }
        }
    }

    fn last_is_value(&self) -> bool {
        self.n > 0
            && matches!(
                self.expr[self.n - 1],
                b'0'..=b'9' | b'.' | b')' | b'p' | b'e' | b'%' | b'!'
            )
    }

    fn opens(&self) -> i32 {
        self.expr[..self.n].iter().fold(0, |k, &c| match c {
            b'(' => k + 1,
            b')' => k - 1,
            _ => k,
        })
    }

    /// The value of the sum so far (the live answer), if whole.
    fn value(&self) -> Option<Dec> {
        expr::eval(&self.expr[..self.n], self.radians).ok()
    }

    /// Writes `d` as typed letters (a minus as `~`).
    fn put_dec(&mut self, d: Dec) {
        let mut b = [0u8; 32];
        let n = d.show(0, &mut b);
        for &c in &b[..n] {
            self.push(&[if c == b'-' { b'~' } else { c }]);
        }
    }

    fn press(&mut self, k: &[u8]) {
        match k {
            b"C" => self.n = 0,
            b"DEL" => {
                self.n = self.n.saturating_sub(1);
                // A function's letter goes with its bracket.
                if self.n > 0 && b"sctSCTlLq".contains(&self.expr[self.n - 1]) {
                    self.n -= 1;
                }
            }
            b"=" => {
                if let Some(v) = self.value() {
                    let mut shown = [0u8; 64];
                    let sn = self.shown(&mut shown);
                    let mut ans = [0u8; 32];
                    let an = v.show(b',', &mut ans);
                    if self.n_tape == self.tape.len() {
                        self.tape.copy_within(1.., 0);
                        self.n_tape -= 1;
                    }
                    self.tape[self.n_tape] = (Text::of(&shown[..sn]), Text::of(&ans[..an]));
                    self.n_tape += 1;
                    // A new answer brings the tape back to its newest.
                    self.back = 0;
                    self.n = 0;
                    self.put_dec(v);
                }
            }
            b"()" => {
                if self.opens() > 0 && self.last_is_value() {
                    self.push(b")");
                } else {
                    if self.last_is_value() {
                        self.push(b"x");
                    }
                    self.push(b"(");
                }
            }
            b"NEG" => {
                // The minus goes before the last number typed.
                let mut at = self.n;
                while at > 0 && matches!(self.expr[at - 1], b'0'..=b'9' | b'.') {
                    at -= 1;
                }
                if at > 0 && self.expr[at - 1] == b'~' {
                    self.expr.copy_within(at..self.n, at - 1);
                    self.n -= 1;
                } else if self.n < self.expr.len() {
                    self.expr.copy_within(at..self.n, at + 1);
                    self.expr[at] = b'~';
                    self.n += 1;
                }
            }
            b"MC" => self.memory = None,
            b"MR" => {
                if let Some(m) = self.memory {
                    if self.last_is_value() {
                        self.push(b"x");
                    }
                    self.put_dec(m);
                }
            }
            b"M+" | b"M-" => {
                if let Some(v) = self.value() {
                    let m = self.memory.unwrap_or(Dec::ZERO);
                    let r = if k == b"M+" { m.plus(v) } else { m.minus(v) };
                    self.memory = r.ok();
                }
            }
            b"ANG" => self.radians = !self.radians,
            b"INV" => self.inverse = !self.inverse,
            _ => {
                // Inverse turns sin, cos and tan into their arcs.
                let k: &[u8] = match (self.inverse, k) {
                    (true, b"s(") => b"S(",
                    (true, b"c(") => b"C(",
                    (true, b"t(") => b"T(",
                    _ => k,
                };
                if k.ends_with(b"(") && self.last_is_value() {
                    self.push(b"x");
                }
                self.push(k);
                self.inverse = false;
            }
        }
    }

    /// The sum as people read it ("sin(30) + 2^10"): its length.
    fn shown(&self, out: &mut [u8; 64]) -> usize {
        let mut n = 0;
        let mut put = |s: &[u8], n: &mut usize| {
            for &c in s {
                if *n < out.len() {
                    out[*n] = c;
                    *n += 1;
                }
            }
        };
        for &c in &self.expr[..self.n] {
            match c {
                b's' => put(b"sin", &mut n),
                b'c' => put(b"cos", &mut n),
                b't' => put(b"tan", &mut n),
                b'S' => put(b"asin", &mut n),
                b'C' => put(b"acos", &mut n),
                b'T' => put(b"atan", &mut n),
                b'l' => put(b"ln", &mut n),
                b'L' => put(b"log", &mut n),
                b'q' => put(b"sqrt", &mut n),
                b'p' => put(b"pi", &mut n),
                b'~' => put(b"-", &mut n),
                b'+' | b'-' | b'x' | b'/' => put(&[b' ', c, b' '], &mut n),
                _ => put(&[c], &mut n),
            }
        }
        n
    }

    fn keys(&self) -> &'static [[KeyDef; 4]; 6] {
        if self.mode == Mode::Scientific {
            &SCIENTIFIC
        } else {
            &BASIC
        }
    }

    /// Where key (row, column) is on the keypad that starts at `top`.
    fn key_rect(top: u16, r: usize, c: usize) -> Rect {
        let kh = (BOTTOM - top) / 6 - 4;
        Rect::new(MARGIN + c as u16 * 113, top + r as u16 * (kh + 4), 108, kh)
    }

    fn keypad_top(&self) -> u16 {
        TOP + self.n_tape.min(3) as u16 * 46 + 102
    }

    fn key(f: &mut Frame, cx: &Ctx, r: Rect, word: &[u8], kind: u8) {
        // Aero glass keys (the owner, 2026-11-14: XP, not Metro): the
        // digits bright, the operators' signs in the tint, the functions
        // quieter, and = the blue glow of the main action.
        let (state, col) = match kind {
            0 => (kit::KeyState::Rest, TEXT),
            1 => (kit::KeyState::Rest, tint::CALCULATOR),
            2 => (kit::KeyState::On, TEXT),
            _ => (kit::KeyState::Rest, DIM),
        };
        kit::key(f, cx, r, state);
        let font = if word.len() <= 2 && word.iter().all(|c| b"0123456789.,-+x/=%".contains(c)) {
            cx.fonts.num
        } else {
            cx.fonts.ui
        };
        let base = r.y + r.h / 2 + u16::from(font.ascent) * 2 / 5;
        kit::centred(f, r, base, font, col, word);
    }

    /// Where tape line `k` (of those shown) lies, and the display.
    fn tape_rect(&self, k: usize) -> Rect {
        Rect::new(MARGIN + 140, TOP + k as u16 * TAPE_LINE, WIDE - 140, 44)
    }

    /// The tape lines shown: the first one's index and how many.
    fn tape_window(&self) -> (usize, usize) {
        let shown = self.n_tape.min(TAPE_SHOWN);
        let back = self.back.min(self.n_tape - shown);
        (self.n_tape - shown - back, shown)
    }

    /// The tape's area (the stylus scrolls and holds there).
    fn tape_area(&self) -> Rect {
        Rect::new(
            MARGIN,
            TOP,
            WIDE,
            self.n_tape.min(TAPE_SHOWN) as u16 * TAPE_LINE,
        )
    }

    /// The tape line under x, y, as an index into the tape.
    fn tape_line_at(&self, x: u16, y: u16) -> Option<usize> {
        let (first, shown) = self.tape_window();
        (0..shown)
            .find(|&k| self.tape_rect(k).contains_point(x, y))
            .map(|k| first + k)
    }

    fn display_rect(&self) -> Rect {
        Rect::new(MARGIN, TOP + self.n_tape.min(3) as u16 * 46 + 4, WIDE, 92)
    }

    /// `b` (Dec::show's: `,` groups, `.` the decimal mark) in the region's
    /// marks: the US's as they are; elsewhere `.` groups and `,` marks
    /// (the international rule; a region setting later).
    fn regional(b: &mut [u8], us: bool) {
        if !us {
            for c in b.iter_mut() {
                *c = match *c {
                    b',' => b'.',
                    b'.' => b',',
                    x => x,
                };
            }
        }
    }

    fn display(&self, f: &mut Frame, cx: &Ctx) {
        let mut y = TOP;
        let (first, shown) = self.tape_window();
        for (k, (sum, ans)) in self.tape[first..first + shown].iter().enumerate() {
            kit::right(f, W - MARGIN - 12, y + 18, cx.fonts.ui, DIM, sum.get());
            kit::right(f, W - MARGIN - 12, y + 40, cx.fonts.ui, TEXT, ans.get());
            if self.copied.is_some_and(|(line, _)| line == first + k) {
                kit::text(
                    f,
                    MARGIN + 150,
                    y + 40,
                    cx.fonts.label,
                    tint::CALCULATOR,
                    b"COPIED",
                );
            }
            y += TAPE_LINE;
        }
        // Where the lines shown sit in the whole tape, when it scrolls.
        if self.n_tape > TAPE_SHOWN {
            let track = Rect::new(MARGIN + 132, TOP + 4, 4, shown as u16 * TAPE_LINE - 8);
            let thumb_h = (u32::from(track.h) * shown as u32 / self.n_tape as u32).max(12) as u16;
            let room = track.h - thumb_h;
            let span = (self.n_tape - shown) as u32;
            let thumb_y = track.y + (u32::from(room) * first as u32 / span) as u16;
            f.push(rp_app::Cmd::Fill {
                r: track,
                color: kit::mix(DIM, 0xFF00_0000, 160),
            });
            f.push(rp_app::Cmd::Fill {
                r: Rect::new(track.x, thumb_y, 4, thumb_h),
                color: tint::CALCULATOR,
            });
        }
        let d = Rect::new(MARGIN, y + 4, WIDE, 92);
        kit::panel(f, cx, d);
        let mut marks = [0u8; 16];
        let mut mn = 0;
        for (on, word) in [
            (self.memory.is_some(), &b"M  "[..]),
            (self.radians, b"RAD  "),
            (self.inverse, b"INV"),
        ] {
            if on {
                marks[mn..mn + word.len()].copy_from_slice(word);
                mn += word.len();
            }
        }
        kit::text(
            f,
            d.x + 12,
            d.y + 24,
            cx.fonts.label,
            tint::CALCULATOR,
            &marks[..mn],
        );
        let mut shown = [0u8; 64];
        let sn = self.shown(&mut shown);
        kit::right(f, d.right() - 12, d.y + 30, cx.fonts.ui, TEXT, &shown[..sn]);
        let mut ans = [0u8; 32];
        let an = match self.value() {
            Some(v) => {
                let n = v.show(b',', &mut ans);
                Self::regional(&mut ans[..n], cx.us);
                n
            }
            None if self.n == 0 => {
                ans[0] = b'0';
                1
            }
            None => 0,
        };
        kit::right(f, d.right() - 12, d.y + 80, cx.fonts.num, TEXT, &ans[..an]);
    }

    fn keypad(&self, f: &mut Frame, cx: &Ctx) {
        let top = self.keypad_top();
        for (r, row) in self.keys().iter().enumerate() {
            for (c, (word, act, kind)) in row.iter().enumerate() {
                let word: &[u8] = match *act {
                    b"ANG" if self.radians => b"RAD",
                    _ => word,
                };
                let kind = if (*act == b"INV" && self.inverse) || (*act == b"ANG" && self.radians) {
                    0
                } else {
                    *kind
                };
                Self::key(f, cx, Self::key_rect(top, r, c), word, kind);
            }
        }
    }

    fn entry_dec(&self) -> Option<Dec> {
        Dec::typed(&self.entry[..self.entry_n])
    }

    /// A small number pad under the modes' rows: the key at (row, column).
    const PAD: [[&'static [u8]; 4]; 3] = [
        [b"7", b"8", b"9", b"DEL"],
        [b"4", b"5", b"6", b"."],
        [b"1", b"2", b"3", b"0"],
    ];

    fn pad_rect(r: usize, c: usize) -> Rect {
        let top = 420;
        let kh = (BOTTOM - top) / 3 - 4;
        Rect::new(MARGIN + c as u16 * 113, top + r as u16 * (kh + 4), 108, kh)
    }

    fn pad(&self, f: &mut Frame, cx: &Ctx) {
        for (r, row) in Self::PAD.iter().enumerate() {
            for (c, w) in row.iter().enumerate() {
                Self::key(
                    f,
                    cx,
                    Self::pad_rect(r, c),
                    w,
                    if *w == b"DEL" { 3 } else { 0 },
                );
            }
        }
    }

    /// `v` in unit `from` of category `cat` as unit `to`.
    fn convert(cat: usize, from: usize, to: usize, v: Dec) -> Option<Dec> {
        let units = &UNITS[cat].1;
        if UNITS[cat].0 == b"TEMP" {
            // To Celsius, then out.
            let five_ninths = Dec::whole(5).over(Dec::whole(9)).ok()?;
            let c = match units[from].0 {
                b"F" => v.minus(Dec::whole(32)).ok()?.times(five_ninths).ok()?,
                b"K" => v.minus(Dec::typed(b"273.15")?).ok()?,
                _ => v,
            };
            return match units[to].0 {
                b"F" => c.over(five_ninths).ok()?.plus(Dec::whole(32)).ok(),
                b"K" => c.plus(Dec::typed(b"273.15")?).ok(),
                _ => Some(c),
            };
        }
        let a = Dec::typed(units[from].1)?;
        let b = Dec::typed(units[to].1)?;
        v.times(a).ok()?.over(b).ok()
    }

    fn units(&self, f: &mut Frame, cx: &Ctx) {
        let names: [&[u8]; 6] = [b"LENGTH", b"WEIGHT", b"VOLUME", b"TEMP", b"SPEED", b"DATA"];
        kit::chips(
            f,
            cx,
            Rect::new(8, TOP, W - 16, 28),
            &names,
            self.cat,
            kit::IDENTITY[3],
        );
        let v = self.entry_dec().unwrap_or(Dec::ZERO);
        let out = Self::convert(self.cat, self.from, self.to, v);
        let mut y = TOP + 38;
        for (k, (unit, value)) in [(self.from, Some(v)), (self.to, out)].iter().enumerate() {
            let r = Rect::new(MARGIN, y, WIDE, 86);
            kit::panel(f, cx, r);
            let u = Rect::new(r.x + 12, r.y + 12, 96, 32);
            kit::key(f, cx, u, kit::KeyState::On);
            kit::centred(
                f,
                u,
                u.y + 22,
                cx.fonts.ui,
                TEXT,
                UNITS[self.cat].1[*unit].0,
            );
            let mut b = [0u8; 32];
            let n = match value {
                Some(d) if k == 0 && self.entry_n > 0 => {
                    b[..self.entry_n].copy_from_slice(&self.entry[..self.entry_n]);
                    self.entry_n
                }
                Some(d) => d.show(b',', &mut b),
                None => 0,
            };
            kit::right(f, r.right() - 14, r.y + 74, cx.fonts.num, TEXT, &b[..n]);
            y += 92;
        }
        kit::text(
            f,
            MARGIN + 8,
            y + 20,
            cx.fonts.ui,
            DIM,
            b"Tap a unit to change it",
        );
        self.pad(f, cx);
    }

    fn money(&self, f: &mut Frame, cx: &Ctx) {
        if self.n_rates == 0 {
            let r = Rect::new(MARGIN, TOP + 8, WIDE, 150);
            kit::panel(f, cx, r);
            kit::text(f, r.x + 16, r.y + 36, cx.fonts.title, TEXT, b"No rates yet");
            kit::text(
                f,
                r.x + 16,
                r.y + 70,
                cx.fonts.ui,
                DIM,
                b"The European Central Bank's daily",
            );
            kit::text(
                f,
                r.x + 16,
                r.y + 94,
                cx.fonts.ui,
                DIM,
                b"rates arrive with a connection",
            );
            kit::text(
                f,
                r.x + 16,
                r.y + 118,
                cx.fonts.ui,
                DIM,
                b"(Wi-Fi or the dock), kept offline.",
            );
            self.pad(f, cx);
            return;
        }
        let (from, to) = (CURRENCIES[self.cur_from], CURRENCIES[self.cur_to]);
        let v = self.entry_dec().unwrap_or(Dec::ZERO);
        let out = self.exchange(v, from, to);
        let mut y = TOP + 8;
        for (k, (code, value)) in [(from, Some(v)), (to, out)].iter().enumerate() {
            let r = Rect::new(MARGIN, y, WIDE, 86);
            kit::panel(f, cx, r);
            let u = Rect::new(r.x + 12, r.y + 12, 96, 32);
            kit::key(f, cx, u, kit::KeyState::On);
            kit::centred(f, u, u.y + 22, cx.fonts.ui, TEXT, *code);
            let mut b = [0u8; 32];
            let n = match value {
                Some(_) if k == 0 && self.entry_n > 0 => {
                    b[..self.entry_n].copy_from_slice(&self.entry[..self.entry_n]);
                    self.entry_n
                }
                Some(d) => cents(*d, &mut b),
                None => 0,
            };
            kit::right(f, r.right() - 14, r.y + 74, cx.fonts.num, TEXT, &b[..n]);
            y += 92;
        }
        // Whose rates and of when.
        let mut t = [0u8; 48];
        let mut n = 0;
        for &c in b"ECB rates of " {
            t[n] = c;
            n += 1;
        }
        if let Some(d) = self.rates_day {
            let dd = date::date_of(d);
            let months: [&[u8]; 12] = [
                b"Jan", b"Feb", b"Mar", b"Apr", b"May", b"Jun", b"Jul", b"Aug", b"Sep", b"Oct",
                b"Nov", b"Dec",
            ];
            for &c in months[usize::from(dd.month - 1)] {
                t[n] = c;
                n += 1;
            }
            t[n] = b' ';
            n += 1;
            n += num(&mut t[n..], u32::from(dd.day));
        }
        for &c in b" \x80 kept offline" {
            t[n] = c;
            n += 1;
        }
        kit::text(f, MARGIN + 8, y + 20, cx.fonts.ui, DIM, &t[..n]);
        self.pad(f, cx);
    }

    fn tip(&self, f: &mut Frame, cx: &Ctx) {
        let bill = Rect::new(MARGIN, TOP, WIDE, 64);
        kit::panel(f, cx, bill);
        kit::text(f, bill.x + 14, bill.y + 22, cx.fonts.label, DIM, b"BILL");
        let shown: &[u8] = if self.entry_n == 0 {
            b"0"
        } else {
            &self.entry[..self.entry_n]
        };
        kit::right(f, bill.right() - 14, bill.y + 56, cx.fonts.num, TEXT, shown);
        let pcts: [&[u8]; 4] = [b"15%", b"18%", b"20%", b"25%"];
        let chosen = [15, 18, 20, 25]
            .iter()
            .position(|&p| p == self.tip_pct)
            .unwrap_or(2);
        kit::chips(
            f,
            cx,
            Rect::new(MARGIN, TOP + 70, WIDE, 34),
            &pcts,
            chosen,
            tint::CALCULATOR,
        );
        let people = Rect::new(MARGIN, TOP + 110, WIDE, 44);
        kit::panel(f, cx, people);
        kit::text(
            f,
            people.x + 14,
            people.y + 29,
            cx.fonts.ui,
            TEXT,
            b"People",
        );
        let mut pb = *b"<  0  >";
        pb[3] = b'0' + self.people.min(9);
        kit::right(
            f,
            people.right() - 14,
            people.y + 29,
            cx.fonts.ui,
            TEXT,
            &pb,
        );
        let b = self.entry_dec().unwrap_or(Dec::ZERO);
        let tip = b
            .times(Dec::whole(i64::from(self.tip_pct)))
            .and_then(|t| t.over(Dec::whole(100)));
        let total = tip.and_then(|t| t.plus(b));
        let each = total.and_then(|t| t.over(Dec::whole(i64::from(self.people.max(1)))));
        let mut y = TOP + 160;
        for (name, v) in [(&b"TIP"[..], tip), (b"TOTAL", total), (b"EACH", each)] {
            let r = Rect::new(MARGIN, y, WIDE, 52);
            kit::tinted(
                f,
                cx,
                r,
                tint::CALCULATOR,
                if name == b"EACH" { 0x60 } else { 0x20 },
            );
            kit::text(f, r.x + 14, r.y + 32, cx.fonts.label, TEXT, name);
            let mut out = [0u8; 32];
            // Money to the cent.
            let n = v.ok().map_or(0, |d| cents(d, &mut out));
            kit::right(f, r.right() - 14, r.y + 44, cx.fonts.num, TEXT, &out[..n]);
            y += 56;
        }
        self.pad(f, cx);
    }

    fn dates(&self, f: &mut Frame, cx: &Ctx) {
        let mut y = TOP;
        for (label, d) in [(&b"FROM"[..], self.d0), (b"TO", self.d1)] {
            let r = Rect::new(MARGIN, y, WIDE, 56);
            kit::panel(f, cx, r);
            kit::text(f, r.x + 14, r.y + 22, cx.fonts.label, DIM, label);
            let dd = date::date_of(d);
            let wd = [&b"Mon"[..], b"Tue", b"Wed", b"Thu", b"Fri", b"Sat", b"Sun"]
                [usize::from(date::weekday(d))];
            let months: [&[u8]; 12] = [
                b"Jan", b"Feb", b"Mar", b"Apr", b"May", b"Jun", b"Jul", b"Aug", b"Sep", b"Oct",
                b"Nov", b"Dec",
            ];
            let mut b = [0u8; 32];
            let mut n = 0;
            for s in [wd, b", ", months[usize::from(dd.month - 1)], b" "] {
                b[n..n + s.len()].copy_from_slice(s);
                n += s.len();
            }
            n += num(&mut b[n..], dd.day as u32);
            b[n] = b' ';
            n += 1;
            n += num(&mut b[n..], dd.year as u32);
            kit::text(f, r.x + 14, r.y + 46, cx.fonts.ui, TEXT, &b[..n]);
            kit::right(f, r.right() - 14, r.y + 36, cx.fonts.title, DIM, b"-   +");
            y += 62;
        }
        let days = self.d1 - self.d0;
        let r = Rect::new(MARGIN, y + 6, WIDE, 120);
        kit::tinted(f, cx, r, tint::CALCULATOR, 0x50);
        let mut b = [0u8; 16];
        let mut n = 0;
        if days < 0 {
            b[0] = b'-';
            n = 1;
        }
        n += num(&mut b[n..], days.unsigned_abs());
        kit::right(f, r.right() - 14, r.y + 56, cx.fonts.num, TEXT, &b[..n]);
        kit::text(
            f,
            r.x + 14,
            r.y + 50,
            cx.fonts.title,
            TEXT,
            if days.abs() == 1 { b"day" } else { b"days" },
        );
        // Weeks and days, and the weekdays between (Monday to Friday).
        let a = days.unsigned_abs();
        let (lo, hi) = if days < 0 {
            (self.d1, self.d0)
        } else {
            (self.d0, self.d1)
        };
        let weekdays = (lo..hi).filter(|&d| date::weekday(d) < 5).count() as u32;
        let mut s = [0u8; 64];
        let mut k = num(&mut s, a / 7);
        for &c in b" weeks and " {
            s[k] = c;
            k += 1;
        }
        k += num(&mut s[k..], a % 7);
        for &c in b" days \x80 " {
            s[k] = c;
            k += 1;
        }
        k += num(&mut s[k..], weekdays);
        for &c in b" weekdays" {
            s[k] = c;
            k += 1;
        }
        kit::text(f, r.x + 14, r.y + 92, cx.fonts.ui, TEXT, &s[..k]);
        kit::text(
            f,
            r.x + 14,
            r.y + 112,
            cx.fonts.ui,
            DIM,
            b"Tap - or + on a date to move it a day",
        );
    }
}

/// `d` to two decimals ("17.28"): its length in `out`.
fn cents(d: Dec, out: &mut [u8; 32]) -> usize {
    let r = d
        .times(Dec::whole(100))
        .ok()
        .and_then(|x| x.plus(Dec::typed(b"0.5")?).ok())
        .unwrap_or(d);
    let mut b = [0u8; 32];
    let n = r.show(0, &mut b);
    let whole = b[..n].iter().position(|&c| c == b'.').unwrap_or(n);
    let digits = &b[..whole];
    let k = digits.len();
    let (int, frac) = if k > 2 {
        digits.split_at(k - 2)
    } else {
        (&b"0"[..], digits)
    };
    let mut m = 0;
    for &c in int {
        out[m] = c;
        m += 1;
    }
    out[m] = b'.';
    m += 1;
    for _ in frac.len()..2 {
        out[m] = b'0';
        m += 1;
    }
    for &c in frac {
        out[m] = c;
        m += 1;
    }
    m
}

fn num(out: &mut [u8], v: u32) -> usize {
    let mut d = [0u8; 10];
    let mut i = d.len();
    let mut v = v;
    loop {
        i -= 1;
        d[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    let n = d.len() - i;
    out[..n].copy_from_slice(&d[i..]);
    n
}

/// The calculator as it first opens (the program's entry and the PC's
/// model).
#[must_use]
pub fn opened(today: i32) -> Calculator {
    Calculator::new(Mode::Basic, today)
}

impl Program for Calculator {
    fn accent(&self) -> Option<rp_app::Argb> {
        Some(rp_app::kit::tint::CALCULATOR)
    }

    fn title(&self) -> &'static str {
        "Calculator"
    }

    fn softkeys(&self) -> (&'static str, &'static str) {
        ("Back", "Copy")
    }

    fn draw(&mut self, f: &mut Frame, cx: &Ctx) {
        let chosen = MODE_LIST.iter().position(|&m| m == self.mode).unwrap_or(0);
        kit::chips(f, cx, CHIPS, &MODES, chosen, tint::CALCULATOR);
        match self.mode {
            Mode::Basic | Mode::Scientific => {
                self.display(f, cx);
                self.keypad(f, cx);
            }
            Mode::Units => self.units(f, cx),
            Mode::Money => {
                self.rates_kept(cx);
                if self.n_rates == 0 {
                    self.want_rates = true;
                }
                self.money(f, cx);
            }
            Mode::Tip => self.tip(f, cx),
            Mode::Dates => self.dates(f, cx),
        }
    }

    fn event(&mut self, e: Event, cx: &Ctx) -> Reply {
        match e {
            Event::Key(Key::SoftLeft) => Reply::Close,
            // Copy: the answer (or the money, tip or dates result typed).
            Event::Key(Key::SoftRight) => {
                let mut b = [0u8; 32];
                let n = match self.value() {
                    Some(v) => v.show(0, &mut b),
                    None => 0,
                };
                if n > 0 {
                    *cx.clip.borrow_mut() = Text::of(&b[..n]);
                }
                Reply::Redraw
            }
            Event::Key(Key::Right) | Event::Key(Key::Left) => {
                let k = MODE_LIST.iter().position(|&m| m == self.mode).unwrap_or(0);
                let k = if e == Event::Key(Key::Right) {
                    (k + 1) % 6
                } else {
                    (k + 5) % 6
                };
                self.mode = MODE_LIST[k];
                Reply::Redraw
            }
            Event::PenDown { x, y }
                if matches!(self.mode, Mode::Basic | Mode::Scientific)
                    && self.tape_area().contains_point(x, y) =>
            {
                self.pen = Some(TapePen {
                    y0: y,
                    back0: self.back,
                    line: self.tape_line_at(x, y),
                    moved: false,
                    held: 0,
                });
                Reply::Animate
            }
            Event::PenMove { y, .. } => {
                let Some(p) = self.pen.as_mut() else {
                    return Reply::Nothing;
                };
                if y.abs_diff(p.y0) > SLOP {
                    p.moved = true;
                }
                if !p.moved {
                    return Reply::Nothing;
                }
                // Dragging down brings older lines into view.
                let lines = (i32::from(y) - i32::from(p.y0)) / i32::from(TAPE_LINE);
                let most = self.n_tape.saturating_sub(TAPE_SHOWN) as i32;
                self.back = (p.back0 as i32 + lines).clamp(0, most) as usize;
                Reply::Redraw
            }
            Event::Tick if self.pen.is_some() || self.copied.is_some() => {
                if let Some(p) = self.pen.as_mut()
                    && !p.moved
                    && p.held < HOLD_TICKS
                {
                    p.held += 1;
                    if p.held == HOLD_TICKS
                        && let Some(line) = p.line
                    {
                        // Held: the line's answer onto the clipboard.
                        let ans = self.tape[line].1;
                        let mut b = [0u8; 24];
                        let mut n = 0;
                        for &c in ans.get().iter().filter(|c| **c != b',') {
                            b[n] = c;
                            n += 1;
                        }
                        *cx.clip.borrow_mut() = Text::of(&b[..n]);
                        self.copied = Some((line, COPIED_TICKS));
                        return Reply::Animate;
                    }
                }
                if let Some((line, left)) = self.copied {
                    self.copied = (left > 1).then(|| (line, left - 1));
                    if self.copied.is_none() {
                        return Reply::Redraw;
                    }
                }
                Reply::Animate
            }
            Event::PenUp { x, y } => {
                // A drag or a hold on the tape is done by now.
                if let Some(p) = self.pen.take()
                    && (p.moved || p.held >= HOLD_TICKS)
                {
                    return Reply::Redraw;
                }
                // The mode chips.
                if CHIPS.contains_point(x, y) {
                    let k = usize::from((x - CHIPS.x) * 6 / CHIPS.w).min(5);
                    self.mode = MODE_LIST[k];
                    return Reply::Redraw;
                }
                match self.mode {
                    Mode::Basic | Mode::Scientific => {
                        // A tape line's answer reused; the display pastes.
                        if let Some(line) = self.tape_line_at(x, y) {
                            {
                                let ans = self.tape[line].1;
                                if self.last_is_value() {
                                    self.push(b"x");
                                }
                                for &c in ans.get().iter().filter(|c| **c != b',') {
                                    self.push(&[if c == b'-' { b'~' } else { c }]);
                                }
                                return Reply::Redraw;
                            }
                        }
                        if self.display_rect().contains_point(x, y) {
                            let clip = *cx.clip.borrow();
                            let t = clip.get();
                            if !t.is_empty()
                                && t.iter().all(|c| c.is_ascii_digit() || b".-,".contains(c))
                            {
                                if self.last_is_value() {
                                    self.push(b"x");
                                }
                                for &c in t.iter().filter(|c| **c != b',') {
                                    self.push(&[if c == b'-' { b'~' } else { c }]);
                                }
                                return Reply::Redraw;
                            }
                        }
                        let top = self.keypad_top();
                        for (r, row) in self.keys().iter().enumerate() {
                            for (c, (_, act, _)) in row.iter().enumerate() {
                                if Self::key_rect(top, r, c).contains_point(x, y) {
                                    self.press(act);
                                    return Reply::Redraw;
                                }
                            }
                        }
                        Reply::Nothing
                    }
                    Mode::Units | Mode::Money | Mode::Tip => {
                        for (r, row) in Self::PAD.iter().enumerate() {
                            for (c, w) in row.iter().enumerate() {
                                if Self::pad_rect(r, c).contains_point(x, y) {
                                    if *w == b"DEL" {
                                        self.entry_n = self.entry_n.saturating_sub(1);
                                    } else if self.entry_n < self.entry.len() {
                                        self.entry[self.entry_n] = w[0];
                                        self.entry_n += 1;
                                    }
                                    return Reply::Redraw;
                                }
                            }
                        }
                        if self.mode == Mode::Units {
                            if Rect::new(8, TOP, W - 16, 28).contains_point(x, y) {
                                self.cat = usize::from((x - 8) * 6 / (W - 16)).min(5);
                                return Reply::Redraw;
                            }
                            // A unit chip: the next unit of the category.
                            if Rect::new(MARGIN + 12, TOP + 50, 96, 32).contains_point(x, y) {
                                self.from = (self.from + 1) % 4;
                                return Reply::Redraw;
                            }
                            if Rect::new(MARGIN + 12, TOP + 142, 96, 32).contains_point(x, y) {
                                self.to = (self.to + 1) % 4;
                                return Reply::Redraw;
                            }
                        }
                        if self.mode == Mode::Money && self.n_rates > 0 {
                            if Rect::new(MARGIN + 12, TOP + 20, 96, 32).contains_point(x, y) {
                                self.cur_from = (self.cur_from + 1) % CURRENCIES.len();
                                return Reply::Redraw;
                            }
                            if Rect::new(MARGIN + 12, TOP + 112, 96, 32).contains_point(x, y) {
                                self.cur_to = (self.cur_to + 1) % CURRENCIES.len();
                                return Reply::Redraw;
                            }
                        }
                        if self.mode == Mode::Tip {
                            let chips = Rect::new(MARGIN, TOP + 70, WIDE, 34);
                            if chips.contains_point(x, y) {
                                self.tip_pct =
                                    [15, 18, 20, 25][usize::from((x - MARGIN) * 4 / WIDE).min(3)];
                                return Reply::Redraw;
                            }
                            if Rect::new(MARGIN, TOP + 110, WIDE, 44).contains_point(x, y) {
                                self.people = if x > W / 2 {
                                    (self.people + 1).min(9)
                                } else {
                                    self.people.saturating_sub(1).max(1)
                                };
                                return Reply::Redraw;
                            }
                        }
                        Reply::Nothing
                    }
                    Mode::Dates => {
                        for (k, y0) in [(0, TOP), (1, TOP + 62)] {
                            if Rect::new(MARGIN, y0, WIDE, 56).contains_point(x, y) && x > W - 120 {
                                let step = if x > W - 60 { 1 } else { -1 };
                                if k == 0 {
                                    self.d0 += step;
                                } else {
                                    self.d1 += step;
                                }
                                return Reply::Redraw;
                            }
                        }
                        Reply::Nothing
                    }
                }
            }
            _ => Reply::Nothing,
        }
    }

    fn keyboard(&self) -> Option<Field> {
        None
    }

    /// Money with no rates: the ECB's file, over whatever connection.
    fn net_ask(&mut self) -> Option<rp_app::NetAsk> {
        if self.mode != Mode::Money || !self.want_rates {
            return None;
        }
        self.want_rates = false;
        Some(rp_app::NetAsk {
            what: b"RATES",
            arg: Text::EMPTY,
        })
    }

    fn net_answer(&mut self, ok: bool, body: &[u8], cx: &Ctx) -> Reply {
        if ok && self.set_rates_from(body) {
            self.keep_rates(cx);
            return Reply::Redraw;
        }
        Reply::Nothing
    }
}

#[cfg(test)]
mod tests;
