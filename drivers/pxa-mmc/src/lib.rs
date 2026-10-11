//! PXA27x MMC/SD host controller, polled (docs/hw/sd.md).

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use rp_drv_sdcard::{BLOCK, Command, HostError, Response, SdHost};
use rp_hal_api::{RegisterBus, RegisterBus8};

/// Register offsets.
pub mod reg {
    /// Clock start/stop.
    pub const STRPCL: usize = 0x00;
    /// Status.
    pub const STAT: usize = 0x04;
    /// Clock rate.
    pub const CLKRT: usize = 0x08;
    /// Command/data control.
    pub const CMDAT: usize = 0x10;
    /// Response timeout.
    pub const RESTO: usize = 0x14;
    /// Read timeout.
    pub const RDTO: usize = 0x18;
    /// Block length.
    pub const BLKLEN: usize = 0x1C;
    /// Block count.
    pub const NUMBLK: usize = 0x20;
    /// Interrupt request (status).
    pub const I_REG: usize = 0x2C;
    /// Command index.
    pub const CMD: usize = 0x30;
    /// Argument high.
    pub const ARGH: usize = 0x34;
    /// Argument low.
    pub const ARGL: usize = 0x38;
    /// Response FIFO.
    pub const RES: usize = 0x3C;
    /// Receive FIFO.
    pub const RXFIFO: usize = 0x40;
}

const STRPCL_START: u32 = 1 << 1;
const STAT_TOUT_RES: u32 = 1 << 1;
const STAT_DATA_DONE: u32 = 1 << 11;
const STAT_END_CMD_RES: u32 = 1 << 13;
const CMDAT_DATA_EN: u32 = 1 << 2;
const CMDAT_BUSY: u32 = 1 << 5;
const CMDAT_INIT: u32 = 1 << 6;
const I_RXFIFO_REQ: u32 = 1 << 5;
const FIFO_BYTES: usize = 32;

/// Base card clock.
pub const BASE_HZ: u32 = 19_500_000;
/// Poll iterations before declaring a timeout.
const SPIN_LIMIT: u32 = 2_000_000;

/// `CLKRT` value for at most `hz` (19.5 MHz >> n, n = 0..6).
#[must_use]
pub fn clkrt_for(hz: u32) -> u32 {
    (0..=6).find(|&n| BASE_HZ >> n <= hz).unwrap_or(6)
}

/// `CMDAT` response-type bits.
fn response_bits(r: Response) -> u32 {
    match r {
        Response::None => 0,
        Response::R1 => 1,
        Response::R1b => 1 | CMDAT_BUSY,
        Response::R2 => 2,
        Response::R3 => 3,
    }
}

/// Decodes the nine response FIFO words (docs/hw/sd.md).
#[must_use]
pub fn decode_response(words: [u16; 9]) -> [u32; 4] {
    let mut out = [0u32; 4];
    let mut v = u32::from(words[0]);
    for (i, slot) in out.iter_mut().enumerate() {
        let w1 = u32::from(words[1 + 2 * i]);
        let w2 = u32::from(words[2 + 2 * i]);
        *slot = (v << 24) | (w1 << 8) | (w2 >> 8);
        v = w2;
    }
    out
}

/// The controller.
#[derive(Debug)]
pub struct PxaMmc<B> {
    regs: B,
}

impl<B: RegisterBus + RegisterBus8> PxaMmc<B> {
    /// Takes the controller's registers. Its clock must already be enabled.
    pub fn new(mut regs: B) -> Self {
        regs.write32(reg::RESTO, 0x7F);
        regs.write32(reg::RDTO, 0xFFFF);
        Self { regs }
    }

    fn wait_stat(&mut self, mask: u32) -> Result<u32, HostError> {
        for _ in 0..SPIN_LIMIT {
            let s = self.regs.read32(reg::STAT);
            if s & mask != 0 {
                return Ok(s);
            }
        }
        Err(HostError::Timeout)
    }

    fn issue(&mut self, cmd: Command, data_blocks: u32) -> Result<[u32; 4], HostError> {
        self.regs.write32(reg::CMD, u32::from(cmd.index & 0x3F));
        self.regs.write32(reg::ARGH, cmd.arg >> 16);
        self.regs.write32(reg::ARGL, cmd.arg & 0xFFFF);
        let mut cmdat = response_bits(cmd.response);
        if cmd.init_clocks {
            cmdat |= CMDAT_INIT;
        }
        if data_blocks > 0 {
            self.regs.write32(reg::BLKLEN, BLOCK as u32);
            self.regs.write32(reg::NUMBLK, data_blocks);
            cmdat |= CMDAT_DATA_EN;
        }
        self.regs.write32(reg::CMDAT, cmdat);
        self.regs.write32(reg::STRPCL, STRPCL_START);

        let stat = self.wait_stat(STAT_END_CMD_RES | STAT_TOUT_RES)?;
        if stat & STAT_TOUT_RES != 0 {
            return Err(HostError::Timeout);
        }
        let mut words = [0u16; 9];
        if cmd.response != Response::None {
            for w in &mut words {
                *w = self.regs.read32(reg::RES) as u16;
            }
        }
        Ok(decode_response(words))
    }
}

impl<B: RegisterBus + RegisterBus8> SdHost for PxaMmc<B> {
    fn set_clock(&mut self, hz: u32) {
        self.regs.write32(reg::CLKRT, clkrt_for(hz));
    }

    fn command(&mut self, cmd: Command) -> Result<[u32; 4], HostError> {
        self.issue(cmd, 0)
    }

    fn read_block(&mut self, cmd: Command, buf: &mut [u8; BLOCK]) -> Result<u32, HostError> {
        let status = self.issue(cmd, 1)?[0];
        for chunk in buf.chunks_mut(FIFO_BYTES) {
            let mut ready = false;
            for _ in 0..SPIN_LIMIT {
                if self.regs.read32(reg::I_REG) & I_RXFIFO_REQ != 0 {
                    ready = true;
                    break;
                }
            }
            if !ready {
                return Err(HostError::Data);
            }
            for b in chunk {
                *b = self.regs.read8(reg::RXFIFO);
            }
        }
        self.wait_stat(STAT_DATA_DONE)
            .map_err(|_| HostError::Data)?;
        Ok(status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp_hal_mock::MockBus;

    #[test]
    fn clock_rates() {
        assert_eq!(clkrt_for(25_000_000), 0);
        assert_eq!(clkrt_for(19_500_000), 0);
        assert_eq!(clkrt_for(400_000), 6); // 304 kHz
        assert_eq!(clkrt_for(10_000_000), 1);
        assert_eq!(clkrt_for(1), 6);
    }

    #[test]
    fn decodes_r1_like_linux_pxamci_layout() {
        // Status 0x12345678 as packed by the controller.
        let words = [0x0012, 0x3456, 0x7800, 0, 0, 0, 0, 0, 0];
        assert_eq!(decode_response(words)[0], 0x1234_5678);
    }

    #[test]
    fn command_programs_registers_then_starts_clock() {
        let mut bus = MockBus::new();
        bus.preset(reg::STAT, STAT_END_CMD_RES);
        bus.script_reads(reg::RES, &[0x00BE, 0xEF05, 0x0000]);
        let mut mmc = PxaMmc::new(bus);
        let r = mmc.command(Command::new(3, 0, Response::R1)).unwrap();
        assert_eq!(r[0] >> 16, 0xBEEF);
        let w = mmc.into_inner_for_test().writes();
        let tail: Vec<_> = w[2..].to_vec(); // after RESTO/RDTO
        assert_eq!(
            tail,
            [
                (reg::CMD, 3),
                (reg::ARGH, 0),
                (reg::ARGL, 0),
                (reg::CMDAT, 1),
                (reg::STRPCL, 2)
            ]
        );
    }

    #[test]
    fn init_clocks_and_busy_flags() {
        let mut bus = MockBus::new();
        bus.preset(reg::STAT, STAT_END_CMD_RES);
        let mut mmc = PxaMmc::new(bus);
        let mut c0 = Command::new(0, 0, Response::None);
        c0.init_clocks = true;
        mmc.command(c0).unwrap();
        mmc.command(Command::new(7, 0xBEEF_0000, Response::R1b))
            .unwrap();
        let w = mmc.into_inner_for_test().writes();
        let cmdats: Vec<u32> = w
            .iter()
            .filter(|(o, _)| *o == reg::CMDAT)
            .map(|&(_, v)| v)
            .collect();
        assert_eq!(cmdats, [CMDAT_INIT, 1 | CMDAT_BUSY]);
        assert!(w.contains(&(reg::ARGH, 0xBEEF)));
    }

    #[test]
    fn timeout_is_reported() {
        let mut bus = MockBus::new();
        bus.preset(reg::STAT, STAT_TOUT_RES);
        let mut mmc = PxaMmc::new(bus);
        assert_eq!(
            mmc.command(Command::new(8, 0x1AA, Response::R1)),
            Err(HostError::Timeout)
        );
    }

    #[test]
    fn reads_a_block_through_the_fifo() {
        let mut bus = MockBus::new();
        bus.preset(reg::STAT, STAT_END_CMD_RES | STAT_DATA_DONE);
        bus.preset(reg::I_REG, I_RXFIFO_REQ);
        let bytes: Vec<u32> = (0..512u32).map(|i| i & 0xFF).collect();
        bus.script_reads(reg::RXFIFO, &bytes);
        let mut mmc = PxaMmc::new(bus);
        let mut buf = [0u8; BLOCK];
        mmc.read_block(Command::new(17, 0, Response::R1), &mut buf)
            .unwrap();
        assert_eq!(buf[0], 0);
        assert_eq!(buf[255], 255);
        assert_eq!(buf[511], 255);
        let w = mmc.into_inner_for_test().writes();
        assert!(w.contains(&(reg::BLKLEN, 512)) && w.contains(&(reg::NUMBLK, 1)));
        assert!(w.contains(&(reg::CMDAT, 1 | CMDAT_DATA_EN)));
    }

    impl<B> PxaMmc<B> {
        fn into_inner_for_test(self) -> B {
            self.regs
        }
    }
}
