//! Half-Life's two screens the owner asked to see before they are built
//! (docs/research/half-life-licences.md, docs/design/half-life-readiness.md
//! line 12): the pop-up that says where the game's files go, and the
//! default controls in landscape. Drawn for `cargo xtask ui-preview` by the
//! shell and the 2D renderer the X51v runs, with the app kit's own glass
//! panels, keys and fonts: nothing here is a mark the device cannot draw.
//! Concepts for his pick; nothing ships from this file.

use rp_app::kit::{self, DIM, FOCUS, KeyState, TEXT, tint};
use rp_app::{Argb, Ctx, Event, Frame, Program, Rect, Reply};

/// The pop-up shown when Half-Life's icon is opened and the card has no
/// game files yet: where they go, how to put them there, and that the same
/// icon starts the game after.
pub struct HlFiles {
    /// The wordier of the two (numbered steps), or the short one.
    pub steps: bool,
}

/// A line of words at `(x, y)`.
fn line(f: &mut Frame, cx: &Ctx, x: u16, y: u16, c: Argb, s: &[u8]) {
    kit::text(f, x, y, cx.fonts.ui, c, s);
}

impl Program for HlFiles {
    fn title(&self) -> &'static str {
        "Half-Life"
    }
    fn softkeys(&self) -> (&'static str, &'static str) {
        ("Check again", "Close")
    }
    fn accent(&self) -> Option<Argb> {
        Some(tint::CALCULATOR)
    }
    fn draw(&mut self, f: &mut Frame, cx: &Ctx) {
        let t = tint::CALCULATOR;
        kit::text(f, 24, 84, cx.fonts.title, TEXT, b"Add your game files");
        line(f, cx, 24, 114, DIM, b"Half-Life plays from your own copy.");
        if self.steps {
            kit::card(f, cx, Rect::new(16, 136, 448, 250), b"HOW", t);
            let rows: [(&[u8], &[u8], &[u8]); 3] = [
                (
                    b"1",
                    b"Connect the card to your PC",
                    b"A card reader, or the Axim by USB",
                ),
                (
                    b"2",
                    b"Open the folder named valve",
                    b"It was just made on this card",
                ),
                (
                    b"3",
                    b"Drag your Half-Life valve files in",
                    b"Everything inside your own valve folder",
                ),
            ];
            for (i, (n, head, sub)) in rows.iter().enumerate() {
                let y = 176 + 68 * i as u16;
                kit::key(f, cx, Rect::new(32, y, 40, 40), KeyState::On);
                kit::centred(f, Rect::new(32, y, 40, 40), y + 27, cx.fonts.ui, TEXT, n);
                line(f, cx, 88, y + 16, TEXT, head);
                line(f, cx, 88, y + 40, DIM, sub);
            }
            kit::card(f, cx, Rect::new(16, 398, 448, 84), b"WHERE", t);
            line(f, cx, 32, 458, TEXT, b"SD card  >  valve");
            kit::right(f, 448, 458, cx.fonts.ui, FOCUS, b"empty");
        } else {
            kit::card(f, cx, Rect::new(16, 136, 448, 120), b"WHERE", t);
            kit::text(f, 32, 206, cx.fonts.title, TEXT, b"SD card  >  valve");
            line(f, cx, 32, 236, DIM, b"This folder was just made for you.");
            kit::card(f, cx, Rect::new(16, 268, 448, 150), b"HOW", t);
            line(f, cx, 32, 326, TEXT, b"From a PC, drag everything inside");
            line(
                f,
                cx,
                32,
                352,
                TEXT,
                b"your Half-Life valve folder into it.",
            );
            line(f, cx, 32, 390, DIM, b"A card reader, or the Axim by USB.");
        }
        line(f, cx, 24, 512, DIM, b"Then open Half-Life again to play.");
        kit::gel(f, cx, Rect::new(16, 530, 216, 44), b"CHECK AGAIN", true);
        kit::gel(f, cx, Rect::new(248, 530, 216, 44), b"CLOSE", false);
    }
    fn event(&mut self, _: Event, _: &Ctx) -> Reply {
        Reply::Nothing
    }
}

/// The default controls, the device drawn as it is held in landscape (its
/// top edge at the player's right, as DoomGLES's `Orientation 1` turns the
/// picture: the D-pad under the left thumb).
pub struct HlControls {
    /// The D-pad's left and right step sideways (the stylus alone turns),
    /// or turn as in DoomGLES.
    pub strafe: bool,
}

impl Program for HlControls {
    fn title(&self) -> &'static str {
        "Half-Life"
    }
    fn softkeys(&self) -> (&'static str, &'static str) {
        ("Change", "Done")
    }
    fn accent(&self) -> Option<Argb> {
        Some(tint::CALCULATOR)
    }
    fn draw(&mut self, f: &mut Frame, cx: &Ctx) {
        let t = tint::CALCULATOR;
        kit::text(f, 24, 84, cx.fonts.title, TEXT, b"Controls");
        line(
            f,
            cx,
            24,
            112,
            DIM,
            if self.strafe {
                b"Stylus looks and turns, the pad moves"
            } else {
                b"As DoomGLES: the pad moves and turns"
            },
        );
        // The device on its side: the pad and the four buttons at the left
        // (the device's bottom edge), the screen to their right.
        let body = Rect::new(16, 128, 448, 196);
        kit::panel(f, cx, body);
        let screen = Rect::new(176, 144, 272, 164);
        kit::round_rect(f, cx, screen, 0xFF10_1820);
        kit::centred(f, screen, 214, cx.fonts.ui, TEXT, b"Drag: look around");
        kit::centred(f, screen, 240, cx.fonts.ui, DIM, b"Touch: fire");
        // Two glass keys in the picture's corners: what has no button.
        kit::gel(f, cx, Rect::new(184, 268, 92, 32), b"LIGHT", false);
        kit::gel(f, cx, Rect::new(348, 268, 92, 32), b"FIRE 2", false);
        // The D-pad: four keys round the centre.
        let (px, py) = (72u16, 196u16);
        let pad: [(i16, i16, &[u8]); 5] = [
            (0, -44, b"^"),
            (0, 44, b"v"),
            (-44, 0, b"<"),
            (44, 0, b">"),
            (0, 0, b"F"),
        ];
        for (dx, dy, word) in pad {
            let r = Rect::new(
                (i32::from(px) + i32::from(dx)) as u16,
                (i32::from(py) + i32::from(dy)) as u16,
                40,
                40,
            );
            kit::key(
                f,
                cx,
                r,
                if dx == 0 && dy == 0 {
                    KeyState::On
                } else {
                    KeyState::Rest
                },
            );
            kit::centred(f, r, r.y + 27, cx.fonts.ui, TEXT, word);
        }
        // The four program buttons, two each side of the pad as the device
        // has them, numbered for the list below.
        for (i, (x, y)) in [(24u16, 140u16), (24, 268), (128, 140), (128, 268)]
            .iter()
            .enumerate()
        {
            let r = Rect::new(*x, *y, 36, 36);
            kit::key(f, cx, r, KeyState::Rest);
            kit::centred(f, r, y + 25, cx.fonts.ui, TEXT, &[b'1' + i as u8]);
        }
        // What each does.
        kit::card(f, cx, Rect::new(16, 336, 448, 236), b"BUTTONS", t);
        let side: &[u8] = if self.strafe {
            b"step left, right"
        } else {
            b"turn"
        };
        let rows: [(&[u8], &[u8]); 9] = [
            (b"Pad up, down", b"walk"),
            (b"Pad left, right", side),
            (b"Pad centre  F", b"fire"),
            (b"1  Calendar", b"jump"),
            (b"2  Contacts", b"use"),
            (b"3  Mail", b"crouch"),
            (b"4  Home", b"reload"),
            (b"Record (side)", b"next weapon"),
            (b"Wireless", b"menu"),
        ];
        for (i, (what, does)) in rows.iter().enumerate() {
            let y = 390 + 20 * i as u16;
            line(f, cx, 32, y, DIM, what);
            kit::right(f, 448, y, cx.fonts.ui, TEXT, does);
        }
    }
    fn event(&mut self, _: Event, _: &Ctx) -> Reply {
        Reply::Nothing
    }
}
