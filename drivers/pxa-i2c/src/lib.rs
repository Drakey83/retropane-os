//! PXA27x I2C controller (the standard one, GPIO117/118 on the X51v) as a
//! polled bus master (PXA27x Developer's Manual, I2C chapter): writes, and
//! presence checks by address acknowledge. Every wait is bounded, and a
//! refused transaction always ends with a stop, so the bus is left free.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use rp_hal_api::{I2cBus, I2cError, RegisterBus};

/// Register offsets from IBMR.
mod reg {
    pub const IDBR: usize = 0x08;
    pub const ICR: usize = 0x10;
    pub const ISR: usize = 0x18;
    pub const ISAR: usize = 0x20;
}

const ICR_START: u32 = 1 << 0;
const ICR_STOP: u32 = 1 << 1;
const ICR_TB: u32 = 1 << 3;
/// Master abort: sends a stop without a byte.
const ICR_MA: u32 = 1 << 4;
const ICR_SCLE: u32 = 1 << 5;
const ICR_IUE: u32 = 1 << 6;
const ICR_UR: u32 = 1 << 14;

const ISR_ACKNAK: u32 = 1 << 1;
const ISR_UB: u32 = 1 << 2;
const ISR_ALD: u32 = 1 << 5;
const ISR_ITE: u32 = 1 << 6;
const ISR_BED: u32 = 1 << 10;
/// The write-one-to-clear status bits.
const ISR_CLEAR: u32 = 0x07F0;

/// Status polls before a byte is declared lost (~100 kHz bus: a byte is
/// ~90 us).
const SPIN_LIMIT: u32 = 200_000;

/// The controller.
pub struct PxaI2c<B> {
    regs: B,
}

impl<B: RegisterBus> PxaI2c<B> {
    /// Takes the controller's registers (its clock enabled): reset, own
    /// slave address 0 (never answered), enabled as a master at the
    /// standard 100 kHz.
    pub fn new(mut regs: B) -> Self {
        regs.write32(reg::ICR, ICR_UR);
        regs.write32(reg::ISR, ISR_CLEAR);
        regs.write32(reg::ICR, 0);
        regs.write32(reg::ISAR, 0);
        regs.write32(reg::ICR, ICR_IUE | ICR_SCLE);
        Self { regs }
    }

    /// Sends the byte in IDBR with `flags` and waits for it to go.
    fn send(&mut self, byte: u8, flags: u32) -> Result<(), I2cError> {
        self.regs.write32(reg::IDBR, u32::from(byte));
        self.regs
            .write32(reg::ICR, ICR_IUE | ICR_SCLE | ICR_TB | flags);
        for _ in 0..SPIN_LIMIT {
            let s = self.regs.read32(reg::ISR);
            if s & ISR_ALD != 0 {
                self.regs.write32(reg::ISR, ISR_CLEAR);
                return Err(I2cError::Bus);
            }
            if s & ISR_ITE != 0 {
                self.regs.write32(reg::ISR, ISR_CLEAR);
                return if s & ISR_ACKNAK != 0 {
                    Err(I2cError::Nak)
                } else {
                    Ok(())
                };
            }
        }
        Err(I2cError::Bus)
    }

    /// Ends a refused transaction with a stop, and waits for the bus.
    fn abort(&mut self) {
        self.regs.write32(reg::ICR, ICR_IUE | ICR_SCLE | ICR_MA);
        self.idle();
        self.regs.write32(reg::ICR, ICR_IUE | ICR_SCLE);
    }

    fn idle(&mut self) {
        for _ in 0..SPIN_LIMIT {
            if self.regs.read32(reg::ISR) & ISR_UB == 0 {
                return;
            }
        }
    }
}

impl<B: RegisterBus> I2cBus for PxaI2c<B> {
    fn write(&mut self, addr: u8, data: &[u8]) -> Result<(), I2cError> {
        let last = data.len();
        let first_flags = ICR_START | if last == 0 { ICR_STOP } else { 0 };
        if let Err(e) = self.send(addr << 1, first_flags) {
            self.abort();
            return Err(if e == I2cError::Nak {
                I2cError::NoDevice
            } else {
                e
            });
        }
        for (i, &b) in data.iter().enumerate() {
            let flags = if i + 1 == last { ICR_STOP } else { 0 };
            if let Err(e) = self.send(b, flags) {
                self.abort();
                return Err(e);
            }
        }
        self.idle();
        // A bus error flag left over (it accompanies a refused byte) is
        // cleared for the next transaction.
        if self.regs.read32(reg::ISR) & ISR_BED != 0 {
            self.regs.write32(reg::ISR, ISR_CLEAR);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp_hal_mock::MockBus;

    /// A controller whose status register keeps reading `status` (the
    /// mock stores writes; the hardware's status bits clear on a 1).
    fn with_status(status: u32) -> PxaI2c<MockBus> {
        let mut i2c = PxaI2c::new(MockBus::new());
        i2c.regs.script_reads(reg::ISR, &[status; 64]);
        i2c
    }

    #[test]
    fn writes_address_then_data_with_stop_on_the_last_byte() {
        let mut i2c = with_status(ISR_ITE);
        i2c.write(0x1A, &[0x1E, 0x00]).unwrap();
        let w = i2c.regs.writes();
        let sends: Vec<(u32, u32)> = w
            .windows(2)
            .filter(|p| p[0].0 == reg::IDBR && p[1].0 == reg::ICR)
            .map(|p| (p[0].1, p[1].1))
            .collect();
        let base = ICR_IUE | ICR_SCLE | ICR_TB;
        assert_eq!(
            sends,
            [
                (0x34, base | ICR_START),
                (0x1E, base),
                (0x00, base | ICR_STOP)
            ]
        );
    }

    #[test]
    fn an_address_nobody_answers_is_no_device_and_the_bus_is_released() {
        let mut i2c = with_status(ISR_ITE | ISR_ACKNAK | ISR_BED);
        assert_eq!(i2c.write(0x1B, &[1, 2]), Err(I2cError::NoDevice));
        let w = i2c.regs.writes();
        assert!(
            w.contains(&(reg::ICR, ICR_IUE | ICR_SCLE | ICR_MA)),
            "stop sent"
        );
        // Nothing after the address.
        assert_eq!(w.iter().filter(|x| x.0 == reg::IDBR).count(), 1);
    }

    #[test]
    fn a_presence_check_is_address_and_stop() {
        let mut i2c = with_status(ISR_ITE);
        i2c.write(0x1A, &[]).unwrap();
        let w = i2c.regs.writes();
        assert!(w.contains(&(reg::ICR, ICR_IUE | ICR_SCLE | ICR_TB | ICR_START | ICR_STOP)));
    }

    #[test]
    fn a_stuck_bus_or_lost_arbitration_is_a_bus_error() {
        let mut i2c = PxaI2c::new(MockBus::new());
        i2c.regs.preset(reg::ISR, 0); // ITE never comes
        assert_eq!(i2c.write(0x1A, &[0]), Err(I2cError::Bus));
        let mut i2c = with_status(ISR_ALD);
        assert_eq!(i2c.write(0x1A, &[0]), Err(I2cError::Bus));
    }
}
