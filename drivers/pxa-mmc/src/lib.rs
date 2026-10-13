//! PXA27x MMC/SD host controller, polled (docs/hw/sd.md).
//!
//! Multi-block data moves by DMA when the controller is given a
//! [`DmaEngine`] ([`PxaMmc::with_dma`]) that accepts the buffer; otherwise
//! (and always for single blocks, where setting DMA up costs more than it
//! saves) through the FIFOs by programmed I/O.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use rp_drv_sdcard::{BLOCK, Command, HostError, Response, SdHost};
use rp_hal_api::{DmaEngine, NoDma, Port, RegisterBus, RegisterBus8, Width};

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
    /// Transmit FIFO.
    pub const TXFIFO: usize = 0x44;
}

const STRPCL_STOP: u32 = 1 << 0;
const STRPCL_START: u32 = 1 << 1;
/// STAT: the card clock is running.
const STAT_CLK_EN: u32 = 1 << 8;
/// STAT: the card didn't send the next data block within RDTO.
const STAT_READ_TIMEOUT: u32 = 1 << 0;
const STAT_TOUT_RES: u32 = 1 << 1;
/// STAT: a received data block failed its CRC.
const STAT_CRC_READ: u32 = 1 << 3;
const STAT_DATA_DONE: u32 = 1 << 11;
/// STAT: the card finished programming the written data.
const STAT_PRG_DONE: u32 = 1 << 12;
const STAT_END_CMD_RES: u32 = 1 << 13;
const CMDAT_DATA_EN: u32 = 1 << 2;
/// CMDAT: the data goes to the card.
const CMDAT_WRITE: u32 = 1 << 3;
/// CMDAT: this command (CMD12) stops a multi-block write; without it the
/// controller, still in write mode, never issues the command (seen on the
/// X51v: a command timeout with no response flag).
const CMDAT_STOP_TRAN: u32 = 1 << 10;
const CMDAT_BUSY: u32 = 1 << 5;
const CMDAT_INIT: u32 = 1 << 6;
/// CMDAT: 4-bit SD data bus.
const CMDAT_SD_4DAT: u32 = 1 << 8;
/// FIFO requests go to the DMA controller instead of I_REG.
const CMDAT_DMA_EN: u32 = 1 << 7;
/// The controller's physical address (the DMA controller sees physical
/// addresses).
pub const PHYS_BASE: u32 = 0x4110_0000;
/// DMA request line for the receive FIFO (PXA27x manual, DRCMR table).
pub const DREQ_RX: u8 = 21;
/// DMA request line for the transmit FIFO.
pub const DREQ_TX: u8 = 22;
/// The receive FIFO as a DMA port (byte-wide).
const RX_PORT: Port = Port {
    request: DREQ_RX,
    addr: PHYS_BASE + reg::RXFIFO as u32,
    width: Width::Byte,
};
/// The transmit FIFO as a DMA port.
const TX_PORT: Port = Port {
    request: DREQ_TX,
    addr: PHYS_BASE + reg::TXFIFO as u32,
    width: Width::Byte,
};
const I_RXFIFO_REQ: u32 = 1 << 5;
const I_TXFIFO_REQ: u32 = 1 << 6;
const FIFO_BYTES: usize = 32;

/// Base card clock.
pub const BASE_HZ: u32 = 19_500_000;
/// Poll iterations before declaring a timeout.
const SPIN_LIMIT: u32 = 2_000_000;
/// Poll iterations for writes: a card may stay busy programming for up to
/// 250 ms per block (SD specification), and a multi-block write stalls the
/// transmit FIFO request for that long between blocks.
const WRITE_SPIN_LIMIT: u32 = 60_000_000;

/// Slowest card clock: 19.5 MHz >> 6 = 304.6875 kHz (still under the 400 kHz
/// the SD identification phase allows).
pub const MIN_HZ: u32 = BASE_HZ >> 6;

/// `CLKRT` value for at most `hz` (19.5 MHz >> n, n = 0..6).
///
/// # Panics
/// In debug builds, if `hz` is below [`MIN_HZ`]: the controller can't go that
/// slow, and silently running faster than asked could break a card.
#[must_use]
pub fn clkrt_for(hz: u32) -> u32 {
    debug_assert!(hz >= MIN_HZ, "card clock below the controller's minimum");
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
pub struct PxaMmc<B, D = NoDma> {
    regs: B,
    dma: D,
    /// The current data command's FIFO requests go to DMA.
    dma_on: bool,
    /// DMA may be used for reads, and for writes (see [`PxaMmc::set_dma`]).
    dma_reads: bool,
    dma_writes: bool,
    /// Transfers that went by DMA (for benchmarks and the run log).
    dma_transfers: u32,
    /// The card was switched to the 4-bit bus (ACMD6).
    wide: bool,
    /// A multi-block write is in progress: the next CMD12 stops it.
    writing: bool,
    /// The last command that failed and the status it left: (index, STAT).
    last_failure: Option<(u8, u32)>,
    /// The data command in progress (for `last_failure`).
    data_cmd: u8,
}

impl<B: RegisterBus + RegisterBus8> PxaMmc<B> {
    /// Takes the controller's registers. Its clock must already be enabled.
    pub fn new(regs: B) -> Self {
        Self::with_dma(regs, NoDma)
    }
}

impl<B: RegisterBus + RegisterBus8, D: DmaEngine> PxaMmc<B, D> {
    /// Takes the controller's registers (its clock enabled) and a DMA
    /// engine for multi-block transfers.
    pub fn with_dma(mut regs: B, dma: D) -> Self {
        regs.write32(reg::RESTO, 0x7F);
        regs.write32(reg::RDTO, 0xFFFF);
        Self {
            regs,
            dma,
            dma_on: false,
            dma_reads: true,
            dma_writes: true,
            dma_transfers: 0,
            wide: false,
            writing: false,
            last_failure: None,
            data_cmd: 0,
        }
    }

    /// Allows DMA for reads and for writes (both, by default, with an
    /// engine). A caller that hasn't yet seen DMA work on this hardware can
    /// keep a direction on programmed I/O.
    pub fn set_dma(&mut self, reads: bool, writes: bool) {
        self.dma_reads = reads;
        self.dma_writes = writes;
    }

    /// Whether DMA is allowed for (reads, writes).
    #[must_use]
    pub fn dma_allowed(&self) -> (bool, bool) {
        (self.dma_reads, self.dma_writes)
    }

    /// How many transfers went by DMA.
    #[must_use]
    pub fn dma_transfers(&self) -> u32 {
        self.dma_transfers
    }

    /// The last command that timed out and the status register it left.
    #[must_use]
    pub fn last_failure(&self) -> Option<(u8, u32)> {
        self.last_failure
    }

    fn wait_stat(&mut self, mask: u32) -> Result<u32, HostError> {
        self.wait_stat_for(mask, SPIN_LIMIT)
    }

    fn wait_stat_for(&mut self, mask: u32, limit: u32) -> Result<u32, HostError> {
        for _ in 0..limit {
            let s = self.regs.read32(reg::STAT);
            if s & mask != 0 {
                return Ok(s);
            }
        }
        Err(HostError::Timeout)
    }

    fn issue(&mut self, cmd: Command, data_blocks: u32) -> Result<[u32; 4], HostError> {
        self.issue_data(cmd, data_blocks, false)
    }

    fn issue_data(
        &mut self,
        cmd: Command,
        data_blocks: u32,
        write: bool,
    ) -> Result<[u32; 4], HostError> {
        self.regs.write32(reg::CMD, u32::from(cmd.index & 0x3F));
        self.regs.write32(reg::ARGH, cmd.arg >> 16);
        self.regs.write32(reg::ARGL, cmd.arg & 0xFFFF);
        let mut cmdat = response_bits(cmd.response);
        if cmd.init_clocks {
            cmdat |= CMDAT_INIT;
        }
        if cmd.index == 12 && self.writing {
            cmdat |= CMDAT_STOP_TRAN;
        }
        self.writing = write && data_blocks > 1;
        self.data_cmd = cmd.index;
        if data_blocks > 0 {
            self.regs.write32(reg::BLKLEN, BLOCK as u32);
            self.regs.write32(reg::NUMBLK, data_blocks);
            cmdat |= CMDAT_DATA_EN;
            if write {
                cmdat |= CMDAT_WRITE;
            }
            if self.wide {
                cmdat |= CMDAT_SD_4DAT;
            }
            if self.dma_on {
                cmdat |= CMDAT_DMA_EN;
            }
        }
        self.regs.write32(reg::CMDAT, cmdat);
        self.regs.write32(reg::STRPCL, STRPCL_START);

        // A busy-signalled command (R1b: CMD7, and the CMD12 that stops a
        // multi-block write) ends only when the card has finished
        // programming, which can take far longer than any read.
        let limit = if cmd.response == Response::R1b {
            WRITE_SPIN_LIMIT
        } else {
            SPIN_LIMIT
        };
        match self.wait_stat_for(STAT_END_CMD_RES | STAT_TOUT_RES, limit) {
            Ok(s) if s & STAT_TOUT_RES == 0 => {}
            r => {
                let s = r.unwrap_or_else(|_| self.regs.read32(reg::STAT));
                self.last_failure = Some((cmd.index, s));
                self.end_dma();
                return Err(HostError::Timeout);
            }
        };
        let mut words = [0u16; 9];
        if cmd.response != Response::None {
            for w in &mut words {
                *w = self.regs.read32(reg::RES) as u16;
            }
        }
        Ok(decode_response(words))
    }
}

impl<B: RegisterBus + RegisterBus8, D: DmaEngine> SdHost for PxaMmc<B, D> {
    fn set_clock(&mut self, hz: u32) {
        // The rate changes only while the clock is stopped (PXA27x manual);
        // the next command's start restarts it at the new rate.
        self.regs.write32(reg::STRPCL, STRPCL_STOP);
        for _ in 0..SPIN_LIMIT {
            if self.regs.read32(reg::STAT) & STAT_CLK_EN == 0 {
                break;
            }
        }
        self.regs.write32(reg::CLKRT, clkrt_for(hz));
    }

    fn command(&mut self, cmd: Command) -> Result<[u32; 4], HostError> {
        self.issue(cmd, 0)
    }

    fn read_block(&mut self, cmd: Command, buf: &mut [u8; BLOCK]) -> Result<u32, HostError> {
        let status = self.issue(cmd, 1)?[0];
        self.receive(buf)?;
        Ok(status)
    }

    fn can_wide_bus(&self) -> bool {
        true
    }

    fn set_wide_bus(&mut self) {
        self.wide = true;
    }

    fn can_read_multiple(&self) -> bool {
        true
    }

    fn read_blocks(&mut self, cmd: Command, buf: &mut [u8]) -> Result<u32, HostError> {
        self.read_blocks_while(cmd, buf, &mut || {})
    }

    /// While the DMA moves the blocks, `wait` runs in the waiting loop
    /// (useful work instead of spinning: the USB drive sends the previous
    /// chunk). Through the FIFO there is no waiting to use.
    fn read_blocks_while(
        &mut self,
        cmd: Command,
        buf: &mut [u8],
        wait: &mut dyn FnMut(),
    ) -> Result<u32, HostError> {
        let blocks = u32::try_from(buf.len() / BLOCK).map_err(|_| HostError::Data)?;
        self.dma_on = blocks > 1 && self.dma_reads && self.dma.arm_from_device(RX_PORT, buf);
        let status = self.issue(cmd, blocks)?[0];
        if self.dma_on {
            self.dma_wait(STAT_READ_TIMEOUT | STAT_CRC_READ, SPIN_LIMIT, wait)?;
            self.receive(&mut [])?;
        } else {
            self.receive(buf)?;
        }
        Ok(status)
    }

    fn write_block(&mut self, cmd: Command, buf: &[u8; BLOCK]) -> Result<u32, HostError> {
        let status = self.issue_data(cmd, 1, true)?[0];
        self.transmit(buf)?;
        // One block: the card programs it now; the controller reports when
        // the card is done (CMD25 runs leave that to the CMD12 that stops them).
        self.wait_stat_for(STAT_PRG_DONE, WRITE_SPIN_LIMIT)
            .map_err(|_| HostError::Data)?;
        Ok(status)
    }

    fn can_write_multiple(&self) -> bool {
        true
    }

    fn write_blocks(&mut self, cmd: Command, buf: &[u8]) -> Result<u32, HostError> {
        let blocks = u32::try_from(buf.len() / BLOCK).map_err(|_| HostError::Data)?;
        self.dma_on = blocks > 1 && self.dma_writes && self.dma.arm_to_device(TX_PORT, buf);
        let status = self.issue_data(cmd, blocks, true)?[0];
        if self.dma_on {
            self.dma_wait(0, WRITE_SPIN_LIMIT, &mut || {})?;
            self.transmit(&[])?;
        } else {
            self.transmit(buf)?;
        }
        Ok(status)
    }
}

impl<B: RegisterBus + RegisterBus8, D: DmaEngine> PxaMmc<B, D> {
    /// Waits for the armed DMA transfer, watching the controller's `errors`
    /// status bits so a failing card ends it at once (the DMA would wait
    /// for FIFO requests that never come). `wait` runs on every pass: it
    /// must be short (the limit counts passes, so a slow `wait` stretches
    /// the timeout).
    fn dma_wait(
        &mut self,
        errors: u32,
        limit: u32,
        wait: &mut dyn FnMut(),
    ) -> Result<(), HostError> {
        for _ in 0..limit {
            wait();
            match self.dma.done() {
                Some(true) => {
                    self.dma_on = false;
                    self.dma_transfers = self.dma_transfers.wrapping_add(1);
                    return Ok(());
                }
                Some(false) => {
                    self.dma_on = false;
                    let stat = self.regs.read32(reg::STAT);
                    self.last_failure = Some((self.data_cmd, stat));
                    return Err(HostError::Data);
                }
                None => {}
            }
            let stat = self.regs.read32(reg::STAT);
            if stat & errors != 0 {
                self.end_dma();
                return Err(self.data_failure(stat));
            }
        }
        self.end_dma();
        let stat = self.regs.read32(reg::STAT);
        Err(self.data_failure(stat))
    }

    /// Stops an armed DMA transfer (after a failed command or transfer).
    fn end_dma(&mut self) {
        if self.dma_on {
            self.dma.abort();
            self.dma_on = false;
        }
    }

    /// Drains the receive FIFO, 32 bytes per FIFO request, until `buf` is
    /// full and the controller reports the transfer done.
    fn receive(&mut self, buf: &mut [u8]) -> Result<(), HostError> {
        for chunk in buf.chunks_mut(FIFO_BYTES) {
            let mut ready = false;
            for _ in 0..SPIN_LIMIT {
                if self.regs.read32(reg::I_REG) & I_RXFIFO_REQ != 0 {
                    ready = true;
                    break;
                }
                // A read timeout or CRC error ends the transfer: stop
                // waiting at once (the card's driver retries).
                let stat = self.regs.read32(reg::STAT);
                if stat & (STAT_READ_TIMEOUT | STAT_CRC_READ) != 0 {
                    return Err(self.data_failure(stat));
                }
            }
            if !ready {
                let stat = self.regs.read32(reg::STAT);
                return Err(self.data_failure(stat));
            }
            for b in chunk {
                *b = self.regs.read8(reg::RXFIFO);
            }
        }
        match self.wait_stat(STAT_DATA_DONE) {
            Ok(stat) if stat & STAT_CRC_READ != 0 => Err(self.data_failure(stat)),
            Ok(_) => Ok(()),
            Err(_) => {
                let stat = self.regs.read32(reg::STAT);
                Err(self.data_failure(stat))
            }
        }
    }

    /// Records a failed data transfer and names it from the status bits.
    fn data_failure(&mut self, stat: u32) -> HostError {
        self.last_failure = Some((self.data_cmd, stat));
        if stat & STAT_CRC_READ != 0 {
            HostError::Crc
        } else if stat & STAT_READ_TIMEOUT != 0 {
            HostError::Timeout
        } else {
            HostError::Data
        }
    }

    /// Fills the transmit FIFO, 32 bytes per FIFO request, until `buf` has
    /// gone out and the controller reports the transfer done.
    fn transmit(&mut self, buf: &[u8]) -> Result<(), HostError> {
        for chunk in buf.chunks(FIFO_BYTES) {
            let mut ready = false;
            for _ in 0..WRITE_SPIN_LIMIT {
                if self.regs.read32(reg::I_REG) & I_TXFIFO_REQ != 0 {
                    ready = true;
                    break;
                }
            }
            if !ready {
                return Err(HostError::Data);
            }
            for &b in chunk {
                self.regs.write8(reg::TXFIFO, b);
            }
        }
        self.wait_stat_for(STAT_DATA_DONE, WRITE_SPIN_LIMIT)
            .map_err(|_| HostError::Data)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "below the controller's minimum")]
    fn refuses_impossibly_slow_clocks() {
        let _ = clkrt_for(MIN_HZ - 1);
    }
    use rp_hal_mock::MockBus;

    #[test]
    fn clock_rates() {
        assert_eq!(clkrt_for(25_000_000), 0);
        assert_eq!(clkrt_for(19_500_000), 0);
        assert_eq!(clkrt_for(400_000), 6); // 304 kHz
        assert_eq!(clkrt_for(10_000_000), 1);
        assert_eq!(clkrt_for(MIN_HZ), 6);
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

    #[test]
    fn multiple_blocks_on_the_wide_bus() {
        let mut bus = MockBus::new();
        bus.preset(reg::STAT, STAT_END_CMD_RES | STAT_DATA_DONE);
        bus.preset(reg::I_REG, I_RXFIFO_REQ);
        let bytes: Vec<u32> = (0..1536u32).map(|i| i / 512).collect();
        bus.script_reads(reg::RXFIFO, &bytes);
        let mut mmc = PxaMmc::new(bus);
        assert!(mmc.can_wide_bus() && mmc.can_read_multiple());
        mmc.set_wide_bus();
        let mut buf = [0u8; 3 * BLOCK];
        mmc.read_blocks(Command::new(18, 7, Response::R1), &mut buf)
            .unwrap();
        assert!(buf[..512].iter().all(|&b| b == 0) && buf[1024..].iter().all(|&b| b == 2));
        let w = mmc.into_inner_for_test().writes();
        assert!(w.contains(&(reg::NUMBLK, 3)));
        assert!(w.contains(&(reg::CMDAT, 1 | CMDAT_DATA_EN | CMDAT_SD_4DAT)));
    }

    #[test]
    fn writes_a_block_through_the_fifo_and_waits_for_programming() {
        let mut bus = MockBus::new();
        bus.preset(reg::STAT, STAT_END_CMD_RES | STAT_DATA_DONE | STAT_PRG_DONE);
        bus.preset(reg::I_REG, I_TXFIFO_REQ);
        let mut mmc = PxaMmc::new(bus);
        let block: [u8; BLOCK] = core::array::from_fn(|i| i as u8);
        mmc.write_block(Command::new(24, 9, Response::R1), &block)
            .unwrap();
        let w = mmc.into_inner_for_test().writes();
        assert!(w.contains(&(reg::CMDAT, 1 | CMDAT_DATA_EN | CMDAT_WRITE)));
        assert!(w.contains(&(reg::ARGL, 9)) && w.contains(&(reg::NUMBLK, 1)));
        let fifo: Vec<u32> = w
            .iter()
            .filter(|(o, _)| *o == reg::TXFIFO)
            .map(|&(_, v)| v)
            .collect();
        assert_eq!(fifo.len(), BLOCK);
        assert_eq!(fifo[0], 0);
        assert_eq!(fifo[511], 255);
    }

    #[test]
    fn a_card_that_never_finishes_programming_is_a_data_error() {
        let mut bus = MockBus::new();
        bus.preset(reg::STAT, STAT_END_CMD_RES | STAT_DATA_DONE);
        bus.preset(reg::I_REG, I_TXFIFO_REQ);
        let mut mmc = PxaMmc::new(bus);
        assert_eq!(
            mmc.write_block(Command::new(24, 0, Response::R1), &[0; BLOCK]),
            Err(HostError::Data)
        );
    }

    #[test]
    fn multiple_block_writes_leave_the_programming_wait_to_cmd12() {
        let mut bus = MockBus::new();
        bus.preset(reg::STAT, STAT_END_CMD_RES | STAT_DATA_DONE);
        bus.preset(reg::I_REG, I_TXFIFO_REQ);
        let mut mmc = PxaMmc::new(bus);
        mmc.set_wide_bus();
        assert!(mmc.can_write_multiple());
        mmc.write_blocks(Command::new(25, 4, Response::R1), &[1u8; 2 * BLOCK])
            .unwrap();
        let w = mmc.into_inner_for_test().writes();
        assert!(w.contains(&(reg::NUMBLK, 2)));
        assert!(w.contains(&(reg::CMDAT, 1 | CMDAT_DATA_EN | CMDAT_WRITE | CMDAT_SD_4DAT)));
        assert_eq!(
            w.iter().filter(|(o, _)| *o == reg::TXFIFO).count(),
            2 * BLOCK
        );
    }

    #[test]
    fn the_stop_after_a_multi_block_write_says_so() {
        let mut bus = MockBus::new();
        bus.preset(reg::STAT, STAT_END_CMD_RES | STAT_DATA_DONE);
        bus.preset(reg::I_REG, I_TXFIFO_REQ);
        let mut mmc = PxaMmc::new(bus);
        mmc.write_blocks(Command::new(25, 0, Response::R1), &[0u8; 2 * BLOCK])
            .unwrap();
        mmc.command(Command::new(12, 0, Response::R1b)).unwrap();
        // A later CMD12 (after a read) is a plain command again.
        mmc.command(Command::new(12, 0, Response::R1b)).unwrap();
        let w = mmc.into_inner_for_test().writes();
        let cmdats: Vec<u32> = w
            .iter()
            .filter(|(o, _)| *o == reg::CMDAT)
            .map(|&(_, v)| v)
            .collect();
        assert_eq!(cmdats[1], 1 | CMDAT_BUSY | CMDAT_STOP_TRAN);
        assert_eq!(cmdats[2], 1 | CMDAT_BUSY);
    }

    #[test]
    fn a_timed_out_command_is_remembered() {
        let mut bus = MockBus::new();
        bus.preset(reg::STAT, STAT_TOUT_RES);
        let mut mmc = PxaMmc::new(bus);
        assert_eq!(
            mmc.command(Command::new(8, 0x1AA, Response::R1)),
            Err(HostError::Timeout)
        );
        assert_eq!(mmc.last_failure(), Some((8, STAT_TOUT_RES)));
    }

    #[test]
    fn a_crc_error_mid_read_ends_the_transfer_and_is_named() {
        let mut bus = MockBus::new();
        bus.preset(reg::STAT, STAT_END_CMD_RES | STAT_CRC_READ);
        // The first FIFO request comes, then none.
        bus.script_reads(reg::I_REG, &[I_RXFIFO_REQ]);
        let bytes: Vec<u32> = vec![0; 32];
        bus.script_reads(reg::RXFIFO, &bytes);
        let mut mmc = PxaMmc::new(bus);
        let mut buf = [0u8; 2 * BLOCK];
        assert_eq!(
            mmc.read_blocks(Command::new(18, 0, Response::R1), &mut buf),
            Err(HostError::Crc)
        );
        assert_eq!(
            mmc.last_failure(),
            Some((18, STAT_END_CMD_RES | STAT_CRC_READ))
        );
    }

    #[test]
    fn the_clock_is_stopped_before_its_rate_changes() {
        let mut bus = MockBus::new();
        bus.script_reads(reg::STAT, &[STAT_CLK_EN, STAT_CLK_EN, 0]);
        let mut mmc = PxaMmc::new(bus);
        mmc.set_clock(MIN_HZ);
        let w = mmc.into_inner_for_test().writes();
        let tail: Vec<_> = w[2..].to_vec();
        assert_eq!(tail, [(reg::STRPCL, STRPCL_STOP), (reg::CLKRT, 6)]);
    }

    #[test]
    fn commands_without_data_never_set_the_bus_width() {
        let mut bus = MockBus::new();
        bus.preset(reg::STAT, STAT_END_CMD_RES);
        let mut mmc = PxaMmc::new(bus);
        mmc.set_wide_bus();
        mmc.command(Command::new(13, 0, Response::R1)).unwrap();
        let w = mmc.into_inner_for_test().writes();
        assert!(w.contains(&(reg::CMDAT, 1)));
    }

    impl<B, D> PxaMmc<B, D> {
        fn into_inner_for_test(self) -> B {
            self.regs
        }
    }

    /// A DMA engine that moves nothing (it fills read buffers with 0xD4)
    /// but records what it was asked, and finishes (or fails) after a set
    /// number of polls.
    #[derive(Default)]
    struct FakeDma {
        armed: bool,
        arms: Vec<(u8, u32, usize)>,
        polls: u32,
        polls_left: u32,
        fail: bool,
        refuse: bool,
        aborted: u32,
    }

    impl FakeDma {
        fn arm(&mut self, req: u8, fifo: u32, len: usize) -> bool {
            assert!(!self.armed, "armed twice");
            if self.refuse {
                return false;
            }
            self.armed = true;
            self.polls_left = self.polls;
            self.arms.push((req, fifo, len));
            true
        }
    }

    impl DmaEngine for FakeDma {
        fn arm_from_device(&mut self, port: Port, buf: &mut [u8]) -> bool {
            assert_eq!(port.width, Width::Byte);
            let ok = self.arm(port.request, port.addr, buf.len());
            if ok {
                buf.fill(0xD4);
            }
            ok
        }
        fn arm_to_device(&mut self, port: Port, buf: &[u8]) -> bool {
            assert_eq!(port.width, Width::Byte);
            self.arm(port.request, port.addr, buf.len())
        }
        fn done(&mut self) -> Option<bool> {
            assert!(self.armed, "polled while disarmed");
            if self.polls_left > 0 {
                self.polls_left -= 1;
                return None;
            }
            self.armed = false;
            Some(!self.fail)
        }
        fn abort(&mut self) {
            assert!(self.armed, "aborted while disarmed");
            self.armed = false;
            self.aborted += 1;
        }
    }

    fn fake() -> FakeDma {
        FakeDma {
            polls: 3,
            ..FakeDma::default()
        }
    }

    #[test]
    fn multi_block_transfers_go_by_dma_to_the_right_fifo_and_line() {
        let mut bus = MockBus::new();
        bus.preset(reg::STAT, STAT_END_CMD_RES | STAT_DATA_DONE);
        let mut mmc = PxaMmc::with_dma(bus, fake());
        let mut buf = vec![0u8; 4 * BLOCK];
        mmc.read_blocks(Command::new(18, 0, Response::R1), &mut buf)
            .unwrap();
        assert!(buf.iter().all(|&b| b == 0xD4));
        mmc.write_blocks(Command::new(25, 8, Response::R1), &buf[..2 * BLOCK])
            .unwrap();
        assert_eq!(mmc.dma_transfers(), 2);
        assert_eq!(
            mmc.dma.arms,
            [(21, 0x4110_0040, 4 * BLOCK), (22, 0x4110_0044, 2 * BLOCK)]
        );
        assert!(!mmc.dma.armed);
        let w = mmc.into_inner_for_test().writes();
        assert!(w.contains(&(reg::CMDAT, 1 | CMDAT_DATA_EN | CMDAT_DMA_EN)));
        assert!(w.contains(&(reg::CMDAT, 1 | CMDAT_DATA_EN | CMDAT_WRITE | CMDAT_DMA_EN)));
        assert!(w.contains(&(reg::NUMBLK, 4)));
    }

    #[test]
    fn single_blocks_and_refused_buffers_use_the_fifo() {
        // One block: no DMA even when offered.
        let mut bus = MockBus::new();
        bus.preset(reg::STAT, STAT_END_CMD_RES | STAT_DATA_DONE);
        bus.preset(reg::I_REG, I_RXFIFO_REQ);
        bus.script_reads(reg::RXFIFO, &[7; 512]);
        let mut mmc = PxaMmc::with_dma(bus, fake());
        let mut buf = vec![0u8; BLOCK];
        mmc.read_blocks(Command::new(18, 0, Response::R1), &mut buf)
            .unwrap();
        assert!(buf.iter().all(|&b| b == 7));
        assert!(mmc.dma.arms.is_empty());
        // Several blocks the engine refuses (misaligned, say): the FIFO.
        let mut bus = MockBus::new();
        bus.preset(reg::STAT, STAT_END_CMD_RES | STAT_DATA_DONE);
        bus.preset(reg::I_REG, I_RXFIFO_REQ);
        bus.script_reads(reg::RXFIFO, &[9; 1024]);
        let mut dma = fake();
        dma.refuse = true;
        let mut mmc = PxaMmc::with_dma(bus, dma);
        let mut buf = vec![0u8; 2 * BLOCK];
        mmc.read_blocks(Command::new(18, 0, Response::R1), &mut buf)
            .unwrap();
        assert!(buf.iter().all(|&b| b == 9));
        assert_eq!(mmc.dma_transfers(), 0);
        let w = mmc.into_inner_for_test().writes();
        assert!(w.contains(&(reg::CMDAT, 1 | CMDAT_DATA_EN)));
    }

    #[test]
    fn each_direction_can_be_kept_on_the_fifo() {
        let mut bus = MockBus::new();
        bus.preset(reg::STAT, STAT_END_CMD_RES | STAT_DATA_DONE);
        bus.preset(reg::I_REG, I_TXFIFO_REQ);
        let mut mmc = PxaMmc::with_dma(bus, fake());
        assert_eq!(mmc.dma_allowed(), (true, true));
        mmc.set_dma(true, false);
        let mut buf = vec![0u8; 2 * BLOCK];
        mmc.read_blocks(Command::new(18, 0, Response::R1), &mut buf)
            .unwrap();
        mmc.write_blocks(Command::new(25, 0, Response::R1), &buf)
            .unwrap();
        // Only the read went by DMA.
        assert_eq!(mmc.dma.arms, [(21, 0x4110_0040, 2 * BLOCK)]);
        let w = mmc.into_inner_for_test().writes();
        assert_eq!(
            w.iter().filter(|&&(r, _)| r == reg::TXFIFO).count(),
            2 * BLOCK
        );
    }

    #[test]
    fn a_card_error_mid_dma_aborts_it_and_is_named() {
        let mut bus = MockBus::new();
        bus.preset(reg::STAT, STAT_END_CMD_RES | STAT_CRC_READ);
        let mut dma = fake();
        dma.polls = u32::MAX;
        let mut mmc = PxaMmc::with_dma(bus, dma);
        let mut buf = vec![0u8; 2 * BLOCK];
        assert_eq!(
            mmc.read_blocks(Command::new(18, 0, Response::R1), &mut buf),
            Err(HostError::Crc)
        );
        assert_eq!(mmc.dma.aborted, 1);
        assert!(!mmc.dma.armed);
        assert_eq!(mmc.last_failure().map(|f| f.0), Some(18));
    }

    #[test]
    fn a_command_that_times_out_disarms_the_dma() {
        let mut bus = MockBus::new();
        bus.preset(reg::STAT, STAT_TOUT_RES);
        let mut mmc = PxaMmc::with_dma(bus, fake());
        let buf = vec![0u8; 2 * BLOCK];
        assert_eq!(
            mmc.write_blocks(Command::new(25, 0, Response::R1), &buf),
            Err(HostError::Timeout)
        );
        assert_eq!(mmc.dma.aborted, 1);
        assert!(!mmc.dma.armed && !mmc.dma_on);
    }

    #[test]
    fn a_dma_bus_error_is_a_data_error() {
        let mut bus = MockBus::new();
        bus.preset(reg::STAT, STAT_END_CMD_RES);
        let mut dma = fake();
        dma.fail = true;
        let mut mmc = PxaMmc::with_dma(bus, dma);
        let mut buf = vec![0u8; 2 * BLOCK];
        assert_eq!(
            mmc.read_blocks(Command::new(18, 0, Response::R1), &mut buf),
            Err(HostError::Data)
        );
        assert_eq!(mmc.dma_transfers(), 0);
        assert_eq!(mmc.dma.aborted, 0);
        assert!(!mmc.dma_on);
    }
}
