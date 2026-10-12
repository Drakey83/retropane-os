//! PXA27x keypad controller and the Dell Axim X51v button matrix.
//!
//! Written from `docs/hw/keypad.md`. Uses automatic scan-on-activity and reads
//! the scan result bitmaps, so it works by polling now and with interrupts
//! later (MIE is already enabled for that).

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod keys;

use rp_hal_api::{Field, RegisterBus};

/// Register offsets.
pub mod reg {
    /// Control.
    pub const KPC: usize = 0x00;
    /// Automatic scan status.
    pub const KPAS: usize = 0x20;
    /// Scan result bitmaps for columns 0-1, 2-3, 4-5, 6-7.
    pub const KPASMKP: [usize; 4] = [0x28, 0x30, 0x38, 0x40];
}

const KPC_ASACT: Field = Field::bit(29);
const KPC_MKRN: Field = Field::new(26, 3);
const KPC_MKCN: Field = Field::new(23, 3);
const KPC_MS: Field = Field::new(13, 8);
const KPC_ME: Field = Field::bit(12);
const KPC_MIE: Field = Field::bit(11);
/// Matrix interrupt: set on key activity, cleared by reading KPC.
const KPC_MI: Field = Field::bit(22);

/// The X51v's buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    /// D-pad up.
    Up,
    /// D-pad down.
    Down,
    /// D-pad left.
    Left,
    /// D-pad right.
    Right,
    /// D-pad center.
    Enter,
    /// Calendar.
    Calendar,
    /// Contacts.
    Contacts,
    /// Mail.
    Mail,
    /// Home.
    Home,
    /// Record (side button).
    Record,
    /// Wi-Fi.
    Wifi,
}

impl Button {
    /// All buttons.
    pub const ALL: [Button; 11] = [
        Self::Up,
        Self::Down,
        Self::Left,
        Self::Right,
        Self::Enter,
        Self::Calendar,
        Self::Contacts,
        Self::Mail,
        Self::Home,
        Self::Record,
        Self::Wifi,
    ];

    /// Matrix position `(row, column)` (docs/hw/keypad.md).
    #[must_use]
    pub const fn position(self) -> (u8, u8) {
        match self {
            Self::Wifi => (0, 0),
            Self::Up => (0, 2),
            Self::Record => (1, 0),
            Self::Down => (1, 2),
            Self::Calendar => (3, 0),
            Self::Right => (3, 2),
            Self::Contacts => (4, 0),
            Self::Left => (4, 2),
            Self::Mail => (5, 0),
            Self::Enter => (5, 2),
            Self::Home => (6, 0),
        }
    }

    /// A short name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Up => "Up",
            Self::Down => "Down",
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Enter => "Enter",
            Self::Calendar => "Calendar",
            Self::Contacts => "Contacts",
            Self::Mail => "Mail",
            Self::Home => "Home",
            Self::Record => "Record",
            Self::Wifi => "Wifi",
        }
    }
}

/// The set of currently pressed buttons, as a bit per [`Button::ALL`] index.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pressed(u16);

impl Pressed {
    /// Nothing pressed.
    pub const NONE: Self = Self(0);

    /// The same set with `b` pressed too.
    #[must_use]
    pub fn with(self, b: Button) -> Self {
        Self(self.0 | 1 << Self::index(b))
    }

    /// Whether `b` is pressed.
    #[must_use]
    pub fn contains(self, b: Button) -> bool {
        self.0 & (1 << Self::index(b)) != 0
    }

    fn index(b: Button) -> usize {
        Button::ALL.iter().position(|&x| x == b).expect("in ALL")
    }

    /// Buttons that changed between `self` (old) and `new`, with their new state.
    pub fn changes(self, new: Pressed) -> impl Iterator<Item = (Button, bool)> {
        Button::ALL
            .into_iter()
            .filter(move |&b| self.contains(b) != new.contains(b))
            .map(move |b| (b, new.contains(b)))
    }
}

/// Whether the scan bitmaps report `(row, col)` as pressed.
#[must_use]
pub fn bitmap_has(bitmaps: [u32; 4], row: u8, col: u8) -> bool {
    let shift = u32::from(row) + if col % 2 == 1 { 16 } else { 0 };
    bitmaps[usize::from(col / 2)] & (1 << shift) != 0
}

/// The keypad controller, configured for the X51v (7 rows × 3 columns).
#[derive(Debug)]
pub struct Keypad<B> {
    regs: B,
}

/// X51v matrix size.
pub const ROWS: u32 = 7;
/// X51v matrix size.
pub const COLS: u32 = 3;

impl<B: RegisterBus> Keypad<B> {
    /// Takes the controller's registers. Its clock must already be enabled.
    pub fn new(regs: B) -> Self {
        Self { regs }
    }

    /// Enables matrix scanning on activity, with interrupts enabled for later.
    pub fn enable(&mut self) {
        let kpc = KPC_MKRN.insert(0, ROWS - 1);
        let kpc = KPC_MKCN.insert(kpc, COLS - 1);
        let kpc = KPC_MS.insert(kpc, 0xFF);
        let kpc = KPC_ASACT.insert(kpc, 1);
        let kpc = KPC_ME.insert(kpc, 1);
        let kpc = KPC_MIE.insert(kpc, 1);
        self.regs.write32(reg::KPC, kpc);
    }

    /// Acknowledges the matrix interrupt (reading KPC clears it) and says
    /// whether there was key activity. Call before unmasking the keypad's
    /// interrupt again, or the still-asserted interrupt would fire at once.
    pub fn acknowledge(&mut self) -> bool {
        KPC_MI.extract(self.regs.read32(reg::KPC)) != 0
    }

    /// Reads which buttons are currently pressed.
    pub fn pressed(&mut self) -> Pressed {
        let mut bitmaps = [0u32; 4];
        for (b, &r) in bitmaps.iter_mut().zip(&reg::KPASMKP) {
            *b = self.regs.read32(r);
        }
        let mut p = Pressed::default();
        for (i, b) in Button::ALL.iter().enumerate() {
            let (row, col) = b.position();
            if bitmap_has(bitmaps, row, col) {
                p.0 |= 1 << i;
            }
        }
        p
    }

    /// Gives the registers back.
    pub fn into_inner(self) -> B {
        self.regs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp_hal_mock::MockBus;

    #[test]
    fn acknowledge_reads_kpc_once() {
        let mut bus = MockBus::new();
        bus.script_reads(reg::KPC, &[1 << 22, 0]);
        let mut k = Keypad::new(bus);
        assert!(k.acknowledge(), "activity reported");
        assert!(!k.acknowledge(), "cleared by the read");
        assert!(
            k.into_inner().writes().is_empty(),
            "acknowledging writes nothing"
        );
    }

    #[test]
    fn enable_programs_matrix_size_and_scanning() {
        let mut k = Keypad::new(MockBus::new());
        k.enable();
        let kpc = k.into_inner().value(reg::KPC);
        assert_eq!(KPC_MKRN.extract(kpc), 6);
        assert_eq!(KPC_MKCN.extract(kpc), 2);
        assert_eq!(KPC_MS.extract(kpc), 0xFF);
        assert_eq!(KPC_ASACT.extract(kpc), 1);
        assert_eq!(KPC_ME.extract(kpc), 1);
        assert_eq!(KPC_MIE.extract(kpc), 1);
    }

    #[test]
    fn bitmap_layout_matches_spec() {
        // Column 2 (even) of register 1 uses bits 7:0; column 1 uses 23:16.
        assert!(bitmap_has([0, 1 << 3, 0, 0], 3, 2));
        assert!(bitmap_has([1 << (16 + 5), 0, 0, 0], 5, 1));
        assert!(!bitmap_has([1 << 5, 0, 0, 0], 5, 1));
    }

    #[test]
    fn decodes_pressed_buttons() {
        let mut bus = MockBus::new();
        // Up = (0,2) -> reg1 bit 0; Home = (6,0) -> reg0 bit 6.
        bus.preset(reg::KPASMKP[0], 1 << 6)
            .preset(reg::KPASMKP[1], 1);
        let p = Keypad::new(bus).pressed();
        assert!(p.contains(Button::Up) && p.contains(Button::Home));
        assert!(!p.contains(Button::Down));
    }

    #[test]
    fn every_button_round_trips_through_the_bitmaps() {
        for b in Button::ALL {
            let (row, col) = b.position();
            let mut maps = [0u32; 4];
            maps[usize::from(col / 2)] |= 1 << (u32::from(row) + if col % 2 == 1 { 16 } else { 0 });
            let mut bus = MockBus::new();
            for (i, &r) in reg::KPASMKP.iter().enumerate() {
                bus.preset(r, maps[i]);
            }
            let p = Keypad::new(bus).pressed();
            assert!(p.contains(b), "{}", b.name());
            assert_eq!(Button::ALL.iter().filter(|&&x| p.contains(x)).count(), 1);
        }
    }

    #[test]
    fn reports_changes() {
        let old = Pressed(1 << 0); // Up held
        let new = Pressed(1 << 4); // Enter pressed, Up released
        let c: Vec<_> = old.changes(new).collect();
        assert_eq!(c, [(Button::Up, false), (Button::Enter, true)]);
    }
}
