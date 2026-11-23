//! The open program's file steps ([`rp_app::FileAsk`]) handed to the
//! image, which does them on the card (`rp_today::appfiles`), and its
//! sentence capitals ([`rp_app::Program::caps`]) given to the keyboard.

use super::{H, Rect, Screen, Shell, W, keyboard};

impl Shell {
    /// The open program's file step, if it has one (taken: asked once).
    pub fn file_ask(&mut self) -> Option<rp_app::FileAsk> {
        if self.screen != Screen::Program {
            return None;
        }
        self.program
            .borrow_mut()
            .as_mut()
            .and_then(|p| p.file_ask())
    }

    /// Bytes of the file the open program is writing, from `off`.
    pub fn file_piece(&mut self, off: u32, out: &mut [u8]) -> usize {
        self.with_program(|p, cx| p.file_piece(off, out, cx))
            .unwrap_or(0)
    }

    /// The answer to the open program's file step.
    pub fn file_answer(&mut self, a: rp_app::FileAnswer<'_>) {
        let r = self.with_program(|p, cx| p.file_answer(a, cx));
        if r.is_some_and(|r| r != rp_app::Reply::Nothing) {
            self.dirty(Rect::new(0, 0, W, H));
        }
    }

    /// The keyboard's shift as the open program says its caret wants it
    /// (a sentence's start), while it types a sentence.
    pub(crate) fn program_caps(&mut self) {
        if self.prog_field != Some(rp_app::Field::Sentence) {
            return;
        }
        let caps = self.program.borrow().as_ref().and_then(|p| p.caps());
        if let Some(c) = caps
            && self.kb.shift != c
        {
            self.kb.shift = c;
            self.dirty(keyboard::PANEL);
        }
    }

    /// `f` run on the open program with its context.
    fn with_program<R>(
        &mut self,
        f: impl FnOnce(&mut dyn rp_app::Program, &rp_app::Ctx) -> R,
    ) -> Option<R> {
        let (fonts, look) = (self.program_fonts(), self.program_look());
        let cx = rp_app::Ctx {
            fonts: &fonts,
            look: &look,
            now: self.program_now(),
            pics: self.app_pics,
            forecast: &self.forecast[..usize::from(self.forecast_n)],
            us: true,
            utc_offset: self.utc_offset,
            shapes: &self.shapes,
            clip: &self.clip,
            pim: Some(&self.apps.store),
        };
        let mut p = self.program.borrow_mut();
        let p = p.as_mut()?;
        Some(self.owning(|| f(&mut **p, &cx)))
    }
}
