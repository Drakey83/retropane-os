//! PXA27x I2C controller (the standard one, GPIO117/118 on the X51v) as a
//! polled bus master (PXA27x Developer's Manual, I2C chapter): writes, and
//! presence checks by address acknowledge. Every wait is bounded, and a
//! refused transaction always ends with a stop, so the bus is left free.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use rp_hal_api::{I2cBus, I2cError, RegisterBus};

/// Register offsets from IBMR.
mod reg {
    pub const IBMR: usize = 0x00;
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
/// Fast mode, 400 kHz (Windows Mobile drives the X51v's codec bus so:
/// `ICR` `0x8060`, measured 2026-10-18, docs/hw/audio.md).
const ICR_FM: u32 = 1 << 15;

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
    /// [`ICR_FM`] or 0, in every control write.
    speed: u32,
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
        Self { regs, speed: 0 }
    }

    /// The bus lines and the controller's state, read only (for logs):
    /// `IBMR` (bit 1 SCL, bit 0 SDA as the controller sees them), `ICR`,
    /// `ISR`.
    pub fn snapshot(&mut self) -> [u32; 3] {
        [
            self.regs.read32(reg::IBMR),
            self.regs.read32(reg::ICR),
            self.regs.read32(reg::ISR),
        ]
    }

    /// A diagnostic start: the address byte `addr` sent with a start, the
    /// first distinct (`ICR`, `ISR`) pairs read back into `seen` (at most
    /// `polls` reads), then the transaction aborted with a stop. Returns
    /// how many pairs it kept.
    pub fn trace_start(&mut self, addr: u8, seen: &mut [(u32, u32)], polls: u32) -> usize {
        self.regs.write32(reg::IDBR, u32::from(addr << 1));
        self.regs.write32(
            reg::ICR,
            ICR_IUE | ICR_SCLE | self.speed | ICR_TB | ICR_START,
        );
        let mut n = 0;
        let mut last = None;
        for _ in 0..polls {
            let now = (self.regs.read32(reg::ICR), self.regs.read32(reg::ISR));
            if last != Some(now) && n < seen.len() {
                seen[n] = now;
                n += 1;
                last = Some(now);
            }
        }
        self.abort();
        self.regs.write32(reg::ISR, ISR_CLEAR);
        n
    }

    /// Fast mode (400 kHz) on or off; standard (100 kHz) from [`Self::new`].
    pub fn set_fast(&mut self, fast: bool) {
        self.speed = if fast { ICR_FM } else { 0 };
        self.regs.write32(reg::ICR, ICR_IUE | ICR_SCLE | self.speed);
    }

    /// Sends the byte in IDBR with `flags` and waits for it to go.
    fn send(&mut self, byte: u8, flags: u32) -> Result<(), I2cError> {
        self.regs.write32(reg::IDBR, u32::from(byte));
        self.regs
            .write32(reg::ICR, ICR_IUE | ICR_SCLE | self.speed | ICR_TB | flags);
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

    /// Ends a refused transaction with a stop, and waits for the bus. A
    /// controller still busy after it (seen on the X51v: SCL held low after
    /// an acknowledged byte) is reset, so the bus is always left free.
    fn abort(&mut self) {
        self.regs
            .write32(reg::ICR, ICR_IUE | ICR_SCLE | self.speed | ICR_MA);
        self.idle();
        if self.regs.read32(reg::ISR) & ISR_UB != 0 {
            self.reset();
        }
        self.regs.write32(reg::ICR, ICR_IUE | ICR_SCLE | self.speed);
    }

    /// The unit reset (its own address 0); the caller enables it again.
    fn reset(&mut self) {
        self.regs.write32(reg::ICR, ICR_UR);
        self.regs.write32(reg::ISR, ISR_CLEAR);
        self.regs.write32(reg::ICR, 0);
        self.regs.write32(reg::ISAR, 0);
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
        // No address-only transfers (measured on the X51v, 2026-10-18,
        // docs/hw/audio.md): with START and STOP on the address byte the
        // controller never finishes it, and after an acknowledged address a
        // master abort leaves SCL held low. Callers write a register instead
        // (as Windows Mobile does). STOP goes with the last data byte, never
        // with START.
        if last == 0 {
            return Err(I2cError::Unsupported);
        }
        if let Err(e) = self.send(addr << 1, ICR_START) {
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

    #[test]
    fn a_traced_start_keeps_each_change_once_and_ends_with_a_stop() {
        let mut i2c = PxaI2c::new(MockBus::new());
        i2c.regs
            .script_reads(reg::ISR, &[0, 0, ISR_UB, ISR_UB, ISR_UB | ISR_ITE]);
        let mut seen = [(0, 0); 8];
        let n = i2c.trace_start(0x1A, &mut seen, 5);
        assert!(n >= 2, "{seen:x?}");
        assert!(i2c.regs.writes().contains(&(reg::IDBR, 0x34)));
        assert!(
            i2c.regs
                .writes()
                .contains(&(reg::ICR, ICR_IUE | ICR_SCLE | ICR_MA))
        );
    }

    #[test]
    fn fast_mode_is_in_every_control_write() {
        let mut i2c = PxaI2c::new(MockBus::new());
        i2c.regs.script_reads(reg::ISR, &[ISR_ITE; 64]);
        i2c.set_fast(true);
        i2c.write(0x1A, &[0x1E, 0x00]).unwrap();
        let icr: Vec<u32> = i2c
            .regs
            .writes()
            .iter()
            .filter(|w| w.0 == reg::ICR)
            .map(|w| w.1)
            .skip_while(|&v| v & ICR_FM == 0)
            .collect();
        assert!(icr.len() > 3);
        assert!(icr.iter().all(|&v| v & ICR_FM != 0), "{icr:x?}");
    }

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
    fn an_address_only_write_is_refused_without_touching_the_bus() {
        let mut i2c = with_status(ISR_ITE);
        let before = i2c.regs.writes().len();
        assert_eq!(i2c.write(0x1A, &[]), Err(I2cError::Unsupported));
        assert_eq!(i2c.regs.writes().len(), before);
    }

    #[test]
    fn a_controller_still_busy_after_the_stop_is_reset() {
        // The X51v after an acknowledged address and a master abort: SCL
        // held low, the unit busy.
        let mut i2c = PxaI2c::new(MockBus::new());
        // Busy for every read the driver can make (the wait for the bus
        // gives up after SPIN_LIMIT reads).
        let busy = vec![ISR_ITE | ISR_ACKNAK | ISR_UB; SPIN_LIMIT as usize + 16];
        i2c.regs.script_reads(reg::ISR, &busy);
        let set_up = i2c.regs.writes().len();
        assert_eq!(i2c.write(0x1B, &[1]), Err(I2cError::NoDevice));
        let w = i2c.regs.writes();
        assert!(w[set_up..].contains(&(reg::ICR, ICR_UR)), "unit reset");
        assert_eq!(
            w.last(),
            Some(&(reg::ICR, ICR_IUE | ICR_SCLE)),
            "enabled again"
        );
    }

    #[test]
    fn no_write_ever_puts_start_and_stop_on_one_byte() {
        // The class of 2026-10-18's silent codec: the controller never
        // finishes such a byte.
        for status in [ISR_ITE, ISR_ITE | ISR_ACKNAK | ISR_BED] {
            for data in [&[1][..], &[1, 2][..], &[1, 2, 3][..]] {
                for fast in [false, true] {
                    let mut i2c = with_status(status);
                    i2c.set_fast(fast);
                    let _ = i2c.write(0x1A, data);
                    for (off, v) in i2c.regs.writes() {
                        if off == reg::ICR {
                            assert!(
                                v & (ICR_START | ICR_STOP) != ICR_START | ICR_STOP,
                                "ICR {v:x} for {data:?}"
                            );
                        }
                    }
                }
            }
        }
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
