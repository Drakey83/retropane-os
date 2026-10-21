extern crate std;

use std::cell::RefCell;
use std::rc::Rc;
use std::vec::Vec;

use rp_drv_cf::CfCard;
use rp_drv_pxa_mmc::PxaMmc;
use rp_drv_sdcard::{Identity, SdCard};
use rp_hal_api::BlockDevice;

use super::*;

/// A register bank that answers as a working card does and records every
/// write: (offset, value, width).
#[derive(Clone)]
struct Bus {
    writes: Rc<RefCell<Vec<(usize, u32, u8)>>>,
    /// The card status in every R1 response.
    status: u32,
    /// The MMC STAT register.
    stat: u32,
    /// The ATA status register.
    ata: u8,
}

/// Ready for data, in the transfer state.
const TRAN: u32 = (1 << 8) | (4 << 9);

impl Bus {
    fn new() -> Self {
        Self {
            writes: Rc::new(RefCell::new(Vec::new())),
            status: TRAN,
            stat: mmc::STAT_END_CMD_RES | mmc::STAT_DATA_DONE | mmc::STAT_PRG_DONE,
            ata: 0x48,
        }
    }
    fn trace(&self) -> Vec<(usize, u32, u8)> {
        self.writes.borrow().clone()
    }
}

impl RegisterBus for Bus {
    fn read32(&mut self, offset: usize) -> u32 {
        match offset {
            mmc::STAT => self.stat,
            mmc::I_REG => mmc::I_TXFIFO_REQ,
            mmc::RES => {
                // Nine words in turn: the response the driver decodes.
                let n = self
                    .writes
                    .borrow()
                    .iter()
                    .filter(|w| w.0 == usize::MAX)
                    .count();
                self.writes.borrow_mut().push((usize::MAX, 0, 0));
                let s = self.status;
                match n % 9 {
                    0 => s >> 24,
                    1 => (s >> 8) & 0xFFFF,
                    2 => (s & 0xFF) << 8,
                    _ => 0,
                }
            }
            _ => 0,
        }
    }
    fn write32(&mut self, offset: usize, value: u32) {
        self.writes.borrow_mut().push((offset, value, 32));
    }
}

impl RegisterBus8 for Bus {
    fn read8(&mut self, offset: usize) -> u8 {
        if offset == 0x7 { self.ata } else { 0 }
    }
    fn write8(&mut self, offset: usize, value: u8) {
        self.writes.borrow_mut().push((offset, u32::from(value), 8));
    }
}

impl RegisterBus16 for Bus {
    fn read16(&mut self, _offset: usize) -> u16 {
        0
    }
    fn write16(&mut self, offset: usize, value: u16) {
        self.writes
            .borrow_mut()
            .push((offset, u32::from(value), 16));
    }
}

/// The writes that reach the device (the response reads' marks left out).
fn writes(t: &[(usize, u32, u8)]) -> Vec<(usize, u32, u8)> {
    t.iter().copied().filter(|w| w.0 != usize::MAX).collect()
}

fn sector() -> [u8; SECTOR] {
    core::array::from_fn(|i| (i * 7 + 3) as u8)
}

#[test]
fn an_sd_sector_goes_through_the_same_registers_as_the_drivers() {
    for (high_capacity, wide, lba) in [(true, true, 12_345), (false, true, 77), (true, false, 9)] {
        let theirs = Bus::new();
        let id = Identity {
            rca: 0xB368,
            high_capacity,
            wide,
            blocks: 1 << 20,
        };
        SdCard::adopt(PxaMmc::new(theirs.clone()), id)
            .write(lba, &sector())
            .unwrap();
        // The driver sets the timeouts when it takes the controller (the
        // loader's are left as they are).
        let theirs: Vec<_> = writes(&theirs.trace())
            .into_iter()
            .filter(|w| w.0 != 0x14 && w.0 != 0x18)
            .collect();
        let mut ours = Bus::new();
        let card = Sd {
            rca: 0xB368,
            high_capacity,
            wide,
            blocks: lba + 1,
        };
        assert!(sd_write(&mut ours, card, lba, &sector()));
        assert_eq!(
            writes(&ours.trace()),
            theirs,
            "hc {high_capacity} wide {wide}"
        );
    }
}

#[test]
fn a_cf_sector_goes_through_the_same_registers_as_the_driver() {
    let theirs = Bus::new();
    let mut card = CfCard::adopt(Bus::new(), theirs.clone(), 1 << 28);
    card.write_blocks(0x0123_4567, &sector()).unwrap();
    let mut ours = Bus::new();
    assert!(cf_write(&mut ours, 1 << 28, 0x0123_4567, &sector()));
    assert_eq!(writes(&ours.trace()), writes(&theirs.trace()));
}

#[test]
fn sd_refusals_and_timeouts_are_false_never_a_hang() {
    let card = Sd {
        rca: 1,
        high_capacity: true,
        wide: true,
        blocks: 8,
    };
    // Past the card's end: refused before any command (an SDSC card's
    // address would wrap: 2^23 blocks of 512 bytes is block 0 again).
    for high_capacity in [false, true] {
        let card = Sd {
            high_capacity,
            ..card
        };
        for lba in [8, 1 << 23, u32::MAX] {
            let mut bus = Bus::new();
            assert!(!sd_write(&mut bus, card, lba, &sector()));
            assert!(writes(&bus.trace()).is_empty(), "no register written");
        }
    }
    // No response.
    let mut bus = Bus::new();
    bus.stat = mmc::STAT_TOUT_RES;
    assert!(!sd_write(&mut bus, card, 0, &sector()));
    assert!(!writes(&bus.trace()).iter().any(|w| w.0 == mmc::TXFIFO));
    // An error in the card's status: no data sent.
    let mut bus = Bus::new();
    bus.status = TRAN | (1 << 26);
    assert!(!sd_write(&mut bus, card, 0, &sector()));
    assert!(!writes(&bus.trace()).iter().any(|w| w.0 == mmc::TXFIFO));
}

#[test]
fn cf_refusals_and_the_end_of_the_card_are_false() {
    let mut bus = Bus::new();
    assert!(!cf_write(&mut bus, 100, 100, &sector()));
    assert!(bus.trace().is_empty(), "past the end: nothing sent");
    let mut bus = Bus::new();
    bus.ata = 0x41; // ready, error
    assert!(!cf_write(&mut bus, 100, 5, &sector()));
    assert!(
        !bus.trace().iter().any(|w| w.2 == 16),
        "no data after an error"
    );
}
