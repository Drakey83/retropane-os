//! The open program's sentence capitals ([`rp_app::Program::caps`]) given
//! to the keyboard.

use super::{Shell, keyboard};

impl Shell {
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
}
