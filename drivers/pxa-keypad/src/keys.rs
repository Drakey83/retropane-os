//! Keys as Windows Mobile programs know them: each button's CE virtual-key
//! code (the public SDK values), D-pad auto-repeat, and WM6's "press and
//! hold" on the program buttons. No button gets a job Windows Mobile didn't
//! give it (the owner's rule).

use crate::{Button, Pressed, Scan};

/// CE virtual-key codes (public SDK `winuser.h` values).
pub mod vk {
    /// `VK_RETURN` (D-pad centre, the action key).
    pub const RETURN: u8 = 0x0D;
    /// `VK_LEFT`.
    pub const LEFT: u8 = 0x25;
    /// `VK_UP`.
    pub const UP: u8 = 0x26;
    /// `VK_RIGHT`.
    pub const RIGHT: u8 = 0x27;
    /// `VK_DOWN`.
    pub const DOWN: u8 = 0x28;
    /// `VK_APP1`: the first program button (`VK_APP2`.. follow).
    pub const APP1: u8 = 0xC1;
}

impl Button {
    /// The virtual key Windows Mobile programs see for this button. The
    /// program buttons are numbered in the order Windows Mobile devices
    /// conventionally use (Calendar 1, Contacts 2, Mail 3, Home 4, Record
    /// 5, the wireless button 6); docs/hw/keypad.md marks this "likely",
    /// to confirm against the owner's Buttons settings.
    #[must_use]
    pub const fn vk(self) -> u8 {
        match self {
            Self::Up => vk::UP,
            Self::Down => vk::DOWN,
            Self::Left => vk::LEFT,
            Self::Right => vk::RIGHT,
            Self::Enter => vk::RETURN,
            Self::Calendar => vk::APP1,
            Self::Contacts => vk::APP1 + 1,
            Self::Mail => vk::APP1 + 2,
            Self::Home => vk::APP1 + 3,
            Self::Record => vk::APP1 + 4,
            Self::Wifi => vk::APP1 + 5,
        }
    }

    /// The D-pad's directions repeat while held, as on Windows Mobile.
    #[must_use]
    pub const fn repeats(self) -> bool {
        matches!(self, Self::Up | Self::Down | Self::Left | Self::Right)
    }

    /// The program buttons can be held for their second action (WM6's
    /// "press and hold" in Settings, Buttons).
    #[must_use]
    pub const fn holds(self) -> bool {
        matches!(
            self,
            Self::Calendar | Self::Contacts | Self::Mail | Self::Home | Self::Record
        )
    }
}

/// What happened to a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Pressed.
    Down,
    /// Still held: an auto-repeat (D-pad).
    Repeat,
    /// Held long enough for its "press and hold" action (once per press).
    Hold,
    /// Released; `after_hold` when the hold action already ran (so the
    /// press action shouldn't also).
    Up {
        /// The hold action ran for this press.
        after_hold: bool,
    },
}

/// A key event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    /// The button.
    pub button: Button,
    /// Its virtual-key code.
    pub vk: u8,
    /// What happened.
    pub kind: Kind,
}

/// Timings, in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    /// Before the first repeat.
    pub repeat_delay: u32,
    /// Between repeats.
    pub repeat_every: u32,
    /// Before "press and hold".
    pub hold: u32,
}

impl Timing {
    /// Windows Mobile-like defaults: repeat after half a second, then 20
    /// a second; hold after one second.
    pub const DEFAULT: Self = Self {
        repeat_delay: 500,
        repeat_every: 50,
        hold: 1000,
    };
}

/// Turns pressed-button snapshots into key events.
pub struct Keys {
    timing: Timing,
    last: Pressed,
    /// When each held button went down, and its next timed event.
    since: [u32; Button::ALL.len()],
    next: [u32; Button::ALL.len()],
    held: [bool; Button::ALL.len()],
    missed: u32,
}

impl Keys {
    /// Nothing pressed yet.
    #[must_use]
    pub const fn new(timing: Timing) -> Self {
        Self {
            timing,
            last: Pressed::NONE,
            since: [0; Button::ALL.len()],
            next: [0; Button::ALL.len()],
            held: [false; Button::ALL.len()],
            missed: 0,
        }
    }

    /// Key moves that happened entirely between two scans, so their events
    /// were never delivered: the controller latched activity but the held
    /// set came back unchanged (a tap, or a release and press again). A
    /// lower bound: a tap hidden behind another key's change isn't seen.
    /// Which button it was can't be known afterwards (see [`Scan`]); only
    /// a scan while the key is down can say, so this must stay 0 wherever
    /// input is serviced fast enough, and run logs report it.
    #[must_use]
    pub const fn missed(&self) -> u32 {
        self.missed
    }

    /// Feeds a keypad scan taken at `now_ms` (any rate; call at least every
    /// repeat interval while a key is held), calling `out` for each event.
    pub fn update(&mut self, now_ms: u32, scan: Scan, out: &mut dyn FnMut(Event)) {
        let pressed = scan.held;
        if scan.activity && pressed == self.last {
            self.missed = self.missed.saturating_add(1);
        }
        for (i, b) in Button::ALL.into_iter().enumerate() {
            let was = self.last.contains(b);
            let is = pressed.contains(b);
            let ev = |kind| Event {
                button: b,
                vk: b.vk(),
                kind,
            };
            match (was, is) {
                (false, true) => {
                    self.since[i] = now_ms;
                    self.held[i] = false;
                    self.next[i] = now_ms.wrapping_add(if b.repeats() {
                        self.timing.repeat_delay
                    } else {
                        self.timing.hold
                    });
                    out(ev(Kind::Down));
                }
                (true, false) => out(ev(Kind::Up {
                    after_hold: self.held[i],
                })),
                (true, true) => {
                    // Timed events, catching up if the caller was late.
                    while now_ms.wrapping_sub(self.next[i]) < 1 << 31 {
                        if b.repeats() {
                            out(ev(Kind::Repeat));
                            self.next[i] =
                                self.next[i].wrapping_add(self.timing.repeat_every.max(1));
                        } else if b.holds() && !self.held[i] {
                            self.held[i] = true;
                            out(ev(Kind::Hold));
                            break;
                        } else {
                            break;
                        }
                    }
                }
                (false, false) => {}
            }
        }
        self.last = pressed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(keys: &mut Keys, now: u32, pressed: &[Button]) -> Vec<(Button, Kind)> {
        let mut p = Pressed::NONE;
        for &b in pressed {
            p = p.with(b);
        }
        let mut out = Vec::new();
        let scan = Scan {
            held: p,
            activity: p != keys.last,
        };
        keys.update(now, scan, &mut |e| out.push((e.button, e.kind)));
        out
    }

    #[test]
    fn a_tap_between_two_scans_is_counted_never_silent() {
        let mut k = Keys::new(Timing::DEFAULT);
        let tap = |k: &mut Keys, now, held| {
            let mut n = 0;
            k.update(
                now,
                Scan {
                    held,
                    activity: true,
                },
                &mut |_| n += 1,
            );
            n
        };
        // Enter pressed and released between two scans (a redraw stall):
        // nothing held either time, but the latch saw it.
        assert_eq!(tap(&mut k, 10, Pressed::NONE), 0, "no button to report");
        assert_eq!(k.missed(), 1);
        // Up held throughout while Enter is tapped: still counted.
        run(&mut k, 20, &[Button::Up]);
        assert_eq!(tap(&mut k, 90, Pressed::NONE.with(Button::Up)), 0);
        assert_eq!(k.missed(), 2);
        // Ordinary presses and releases, latched or not, aren't misses.
        run(&mut k, 100, &[]);
        run(&mut k, 200, &[Button::Enter]);
        run(&mut k, 300, &[]);
        k.update(400, Scan::default(), &mut |_| {});
        assert_eq!(k.missed(), 2);
    }

    #[test]
    fn codes_are_the_public_ce_values() {
        assert_eq!(Button::Up.vk(), 0x26);
        assert_eq!(Button::Enter.vk(), 0x0D);
        assert_eq!(Button::Calendar.vk(), 0xC1);
        assert_eq!(Button::Home.vk(), 0xC4);
        let mut all: Vec<u8> = Button::ALL.iter().map(|b| b.vk()).collect();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), Button::ALL.len(), "one code per button");
    }

    #[test]
    fn the_d_pad_repeats_while_held() {
        let mut k = Keys::new(Timing::DEFAULT);
        assert_eq!(
            run(&mut k, 0, &[Button::Down]),
            [(Button::Down, Kind::Down)]
        );
        assert!(run(&mut k, 499, &[Button::Down]).is_empty());
        assert_eq!(
            run(&mut k, 500, &[Button::Down]),
            [(Button::Down, Kind::Repeat)]
        );
        // Late caller: every missed repeat is delivered.
        assert_eq!(run(&mut k, 650, &[Button::Down]).len(), 3);
        assert_eq!(
            run(&mut k, 700, &[]),
            [(Button::Down, Kind::Up { after_hold: false })]
        );
        // Enter doesn't repeat, and has no hold.
        run(&mut k, 1000, &[Button::Enter]);
        assert!(run(&mut k, 5000, &[Button::Enter]).is_empty());
    }

    #[test]
    fn program_buttons_have_press_and_hold_once() {
        let mut k = Keys::new(Timing::DEFAULT);
        run(&mut k, 0, &[Button::Calendar]);
        assert_eq!(
            run(&mut k, 1000, &[Button::Calendar]),
            [(Button::Calendar, Kind::Hold)]
        );
        assert!(run(&mut k, 3000, &[Button::Calendar]).is_empty(), "once");
        assert_eq!(
            run(&mut k, 3100, &[]),
            [(Button::Calendar, Kind::Up { after_hold: true })]
        );
        // A short press: no hold.
        run(&mut k, 4000, &[Button::Home]);
        assert_eq!(
            run(&mut k, 4300, &[]),
            [(Button::Home, Kind::Up { after_hold: false })]
        );
    }

    #[test]
    fn keys_are_independent_and_survive_the_clock_wrapping() {
        let mut k = Keys::new(Timing::DEFAULT);
        let t0 = u32::MAX - 100;
        run(&mut k, t0, &[Button::Up, Button::Mail]);
        let ev = run(&mut k, t0.wrapping_add(1000), &[Button::Up, Button::Mail]);
        assert!(ev.contains(&(Button::Mail, Kind::Hold)));
        assert!(ev.iter().filter(|e| e.0 == Button::Up).count() >= 10);
    }
}
