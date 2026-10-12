//! SD card protocol over any host controller (docs/hw/sd.md).
//!
//! Follows the SD Physical Layer Simplified Specification: reset, interface
//! condition (CMD8), operating conditions (ACMD41, with SDHC support), card
//! identification, selection, the 4-bit data bus (ACMD6) when the host has
//! one, then 512-byte block reads, several at a time (CMD18 + CMD12) when the
//! host can: an SD card takes milliseconds to start each read, so one
//! command per block wastes most of the time (measured on the real X51v:
//! 3.85 ms per single-block read). Writes likewise: CMD24 for one block,
//! CMD25 + CMD12 for a run. Host controllers implement [`SdHost`]; the
//! card implements [`BlockDevice`] for file systems.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use rp_hal_api::{BlockDevice, BlockError};

/// Block size used throughout.
pub const BLOCK: usize = 512;

/// The response a command expects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Response {
    /// No response.
    None,
    /// 48-bit card status (also R6, R7).
    R1,
    /// R1 with busy signalling.
    R1b,
    /// 136-bit CID/CSD.
    R2,
    /// 48-bit OCR, no CRC.
    R3,
}

/// One command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Command {
    /// Command index (CMDn / ACMDn).
    pub index: u8,
    /// Argument.
    pub arg: u32,
    /// Expected response.
    pub response: Response,
    /// Send the 80 initialisation clocks first (only before CMD0).
    pub init_clocks: bool,
}

impl Command {
    /// A command with no init clocks.
    #[must_use]
    pub const fn new(index: u8, arg: u32, response: Response) -> Self {
        Self {
            index,
            arg,
            response,
            init_clocks: false,
        }
    }
}

/// Host controller failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostError {
    /// No response in time.
    Timeout,
    /// CRC error.
    Crc,
    /// Data transfer failed.
    Data,
}

/// An SD host controller.
pub trait SdHost {
    /// Sets the card clock to at most `hz`.
    fn set_clock(&mut self, hz: u32);
    /// Sends a command; returns the response words (R2 uses all four, most
    /// significant first; others only `[0]`).
    fn command(&mut self, cmd: Command) -> Result<[u32; 4], HostError>;
    /// Sends a single-block read command and receives the block.
    fn read_block(&mut self, cmd: Command, buf: &mut [u8; BLOCK]) -> Result<u32, HostError>;
    /// Whether the host can use the 4-bit data bus.
    fn can_wide_bus(&self) -> bool {
        false
    }
    /// Switches the host to the 4-bit data bus (the card already switched).
    fn set_wide_bus(&mut self) {}
    /// Whether [`read_blocks`](Self::read_blocks) works.
    fn can_read_multiple(&self) -> bool {
        false
    }
    /// Sends a multiple-block read command (CMD18) and receives
    /// `buf.len() / BLOCK` blocks; the caller then stops the card (CMD12).
    fn read_blocks(&mut self, cmd: Command, buf: &mut [u8]) -> Result<u32, HostError> {
        let _ = (cmd, buf);
        Err(HostError::Data)
    }
    /// Sends a single-block write command (CMD24), the block, and waits for
    /// the card to finish programming it.
    fn write_block(&mut self, cmd: Command, buf: &[u8; BLOCK]) -> Result<u32, HostError> {
        let _ = (cmd, buf);
        Err(HostError::Data)
    }
    /// Whether [`write_blocks`](Self::write_blocks) works.
    fn can_write_multiple(&self) -> bool {
        false
    }
    /// Sends a multiple-block write command (CMD25) and `buf.len() / BLOCK`
    /// blocks; the caller then stops the card (CMD12), which also waits for
    /// the programming to finish.
    fn write_blocks(&mut self, cmd: Command, buf: &[u8]) -> Result<u32, HostError> {
        let _ = (cmd, buf);
        Err(HostError::Data)
    }
}

/// Card errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The host reported an error.
    Host(HostError),
    /// The card never left its busy/initialising state.
    InitTimeout,
    /// CMD8 echoed the wrong check pattern.
    BadInterfaceCondition,
    /// The card reported an error in its status.
    CardStatus(u32),
}

impl Error {
    /// A short fixed name, for logs that shouldn't pull in `Debug` formatting.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Host(HostError::Timeout) => "timeout",
            Self::Host(HostError::Crc) => "CRC error",
            Self::Host(HostError::Data) => "data error",
            Self::InitTimeout => "init timeout",
            Self::BadInterfaceCondition => "bad CMD8 echo",
            Self::CardStatus(_) => "card status error",
        }
    }
}

impl From<HostError> for Error {
    fn from(e: HostError) -> Self {
        Self::Host(e)
    }
}

const OCR_BUSY: u32 = 1 << 31;
const OCR_CCS: u32 = 1 << 30;
const OCR_VOLTAGES: u32 = 0x00FF_8000; // 2.7-3.6 V
const CMD8_ARG: u32 = 0x1AA; // 2.7-3.6 V, check pattern 0xAA
/// Card status bits that indicate an error (out of range .. card ECC etc.).
const STATUS_ERRORS: u32 = 0xFDF9_0008;
const ACMD41_TRIES: u32 = 1000;

/// Clock during identification.
pub const INIT_HZ: u32 = 400_000;
/// Clock for data transfer (default speed).
pub const TRANSFER_HZ: u32 = 25_000_000;

/// An initialised, selected SD card.
#[derive(Debug)]
pub struct SdCard<H> {
    host: H,
    rca: u16,
    high_capacity: bool,
    wide: bool,
}

impl<H: SdHost> SdCard<H> {
    /// Identifies and selects the card.
    pub fn init(mut host: H) -> Result<Self, Error> {
        host.set_clock(INIT_HZ);
        host.command(Command {
            init_clocks: true,
            ..Command::new(0, 0, Response::None)
        })?;

        // CMD8: only v2.00+ cards answer; v1 cards time out.
        let v2 = match host.command(Command::new(8, CMD8_ARG, Response::R1)) {
            Ok(r) if r[0] & 0xFFF == CMD8_ARG => true,
            Ok(_) => return Err(Error::BadInterfaceCondition),
            Err(HostError::Timeout) => false,
            Err(e) => return Err(e.into()),
        };

        let hcs = if v2 { OCR_CCS } else { 0 };
        let mut ocr = 0;
        for _ in 0..ACMD41_TRIES {
            host.command(Command::new(55, 0, Response::R1))?;
            ocr = host.command(Command::new(41, hcs | OCR_VOLTAGES, Response::R3))?[0];
            if ocr & OCR_BUSY != 0 {
                break;
            }
        }
        if ocr & OCR_BUSY == 0 {
            return Err(Error::InitTimeout);
        }
        let high_capacity = v2 && ocr & OCR_CCS != 0;

        host.command(Command::new(2, 0, Response::R2))?; // CID
        let rca = (host.command(Command::new(3, 0, Response::R1))?[0] >> 16) as u16;
        check(host.command(Command::new(7, u32::from(rca) << 16, Response::R1b))?[0])?;
        if !high_capacity {
            check(host.command(Command::new(16, BLOCK as u32, Response::R1))?[0])?;
        }
        // Every SD card supports the 4-bit bus (ACMD6, argument 2).
        let wide = host.can_wide_bus();
        if wide {
            host.command(Command::new(55, u32::from(rca) << 16, Response::R1))?;
            check(host.command(Command::new(6, 2, Response::R1))?[0])?;
            host.set_wide_bus();
        }
        host.set_clock(TRANSFER_HZ);
        Ok(Self {
            host,
            rca,
            high_capacity,
            wide,
        })
    }

    /// Whether the 4-bit data bus is in use.
    #[must_use]
    pub fn is_wide(&self) -> bool {
        self.wide
    }

    /// Whether the card is SDHC/SDXC (block-addressed).
    #[must_use]
    pub fn is_high_capacity(&self) -> bool {
        self.high_capacity
    }

    /// The card's relative address.
    #[must_use]
    pub fn rca(&self) -> u16 {
        self.rca
    }

    fn address(&self, lba: u32) -> u32 {
        if self.high_capacity {
            lba
        } else {
            lba.wrapping_mul(BLOCK as u32)
        }
    }

    /// Reads block `lba` (512 bytes).
    pub fn read(&mut self, lba: u32, buf: &mut [u8; BLOCK]) -> Result<(), Error> {
        let addr = self.address(lba);
        check(
            self.host
                .read_block(Command::new(17, addr, Response::R1), buf)?,
        )
    }

    /// Reads `buf.len() / 512` consecutive blocks from `lba` with one
    /// command when the host can (otherwise block by block).
    ///
    /// # Panics
    /// If `buf` isn't a whole number of blocks.
    pub fn read_many(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), Error> {
        assert_eq!(buf.len() % BLOCK, 0, "not a whole number of blocks");
        if buf.len() > BLOCK && self.host.can_read_multiple() {
            let addr = self.address(lba);
            let status = self
                .host
                .read_blocks(Command::new(18, addr, Response::R1), buf);
            // Stop the card whatever happened, so it accepts commands again.
            let stop = self.host.command(Command::new(12, 0, Response::R1b));
            check(status?)?;
            stop?;
            return Ok(());
        }
        for (i, block) in buf.as_chunks_mut::<BLOCK>().0.iter_mut().enumerate() {
            self.read(lba + i as u32, block)?;
        }
        Ok(())
    }

    /// Writes block `lba` (512 bytes).
    pub fn write(&mut self, lba: u32, buf: &[u8; BLOCK]) -> Result<(), Error> {
        let addr = self.address(lba);
        check(
            self.host
                .write_block(Command::new(24, addr, Response::R1), buf)?,
        )
    }

    /// Writes `buf.len() / 512` consecutive blocks from `lba` with one
    /// command when the host can (otherwise block by block).
    ///
    /// # Panics
    /// If `buf` isn't a whole number of blocks.
    pub fn write_many(&mut self, lba: u32, buf: &[u8]) -> Result<(), Error> {
        assert_eq!(buf.len() % BLOCK, 0, "not a whole number of blocks");
        if buf.len() > BLOCK && self.host.can_write_multiple() {
            let addr = self.address(lba);
            let status = self
                .host
                .write_blocks(Command::new(25, addr, Response::R1), buf);
            let stop = self.host.command(Command::new(12, 0, Response::R1b));
            check(status?)?;
            stop?;
            return Ok(());
        }
        for (i, block) in buf.as_chunks::<BLOCK>().0.iter().enumerate() {
            self.write(lba + i as u32, block)?;
        }
        Ok(())
    }

    /// Gives the host back.
    pub fn into_inner(self) -> H {
        self.host
    }
}

impl<H: SdHost> BlockDevice for SdCard<H> {
    fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
        self.read_many(lba, buf).map_err(|_| BlockError::Device)
    }

    fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
        self.write_many(lba, buf).map_err(|_| BlockError::Device)
    }
}

fn check(status: u32) -> Result<(), Error> {
    if status & STATUS_ERRORS == 0 {
        Ok(())
    } else {
        Err(Error::CardStatus(status))
    }
}

/// One MBR partition table entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Partition {
    /// Partition type byte (0x0B/0x0C = FAT32, 0x07 = exFAT/NTFS).
    pub kind: u8,
    /// First block.
    pub start: u32,
    /// Number of blocks.
    pub blocks: u32,
}

/// Parses the four MBR partition entries of `block0`, or `None` without the
/// 0x55AA signature. Empty entries are skipped.
#[must_use]
pub fn mbr_partitions(block0: &[u8; BLOCK]) -> Option<[Option<Partition>; 4]> {
    if block0[510..512] != [0x55, 0xAA] {
        return None;
    }
    let mut out = [None; 4];
    for (i, slot) in out.iter_mut().enumerate() {
        let e = &block0[446 + i * 16..462 + i * 16];
        let le = |o: usize| u32::from_le_bytes([e[o], e[o + 1], e[o + 2], e[o + 3]]);
        let p = Partition {
            kind: e[4],
            start: le(8),
            blocks: le(12),
        };
        if p.kind != 0 && p.blocks != 0 {
            *slot = Some(p);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake card that follows the spec's state machine.
    struct FakeCard {
        v2: bool,
        sdhc: bool,
        busy_rounds: u32,
        log: Vec<(u8, u32)>,
        clocks: Vec<u32>,
        expect_app: bool,
        data: Vec<u8>,
        /// Offers the 4-bit bus and multiple-block reads.
        capable: bool,
        wide: bool,
    }

    impl FakeCard {
        fn new(v2: bool, sdhc: bool) -> Self {
            let mut data = vec![0u8; 8 * BLOCK];
            for (i, b) in data.iter_mut().enumerate() {
                *b = (i / BLOCK) as u8;
            }
            Self {
                v2,
                sdhc,
                busy_rounds: 2,
                log: vec![],
                clocks: vec![],
                expect_app: false,
                data,
                capable: false,
                wide: false,
            }
        }
    }

    impl SdHost for FakeCard {
        fn set_clock(&mut self, hz: u32) {
            self.clocks.push(hz);
        }
        fn command(&mut self, c: Command) -> Result<[u32; 4], HostError> {
            self.log.push((c.index, c.arg));
            let app = core::mem::take(&mut self.expect_app);
            let r = match (c.index, app) {
                (0, _) => 0,
                (8, _) if self.v2 => c.arg & 0xFFF,
                (8, _) => return Err(HostError::Timeout),
                (55, _) => {
                    self.expect_app = true;
                    0x120
                }
                (41, true) => {
                    if self.busy_rounds > 0 {
                        self.busy_rounds -= 1;
                        OCR_VOLTAGES
                    } else {
                        let ccs = if self.sdhc && c.arg & OCR_CCS != 0 {
                            OCR_CCS
                        } else {
                            0
                        };
                        OCR_BUSY | ccs | OCR_VOLTAGES
                    }
                }
                (2, _) => 0x0353_4453, // CID word
                (3, _) => 0xBEEF_0500, // RCA 0xBEEF
                (7, _) | (16, _) => 0x0700,
                (6, true) if c.arg == 2 => 0x0920,
                (12, _) => 0x0B00,
                _ => return Err(HostError::Timeout),
            };
            Ok([r, 0, 0, 0])
        }
        fn read_block(&mut self, c: Command, buf: &mut [u8; BLOCK]) -> Result<u32, HostError> {
            self.log.push((c.index, c.arg));
            let block = if self.sdhc {
                c.arg
            } else {
                c.arg / BLOCK as u32
            } as usize;
            buf.copy_from_slice(&self.data[block * BLOCK..(block + 1) * BLOCK]);
            Ok(0x900)
        }
        fn can_wide_bus(&self) -> bool {
            self.capable
        }
        fn set_wide_bus(&mut self) {
            self.wide = true;
        }
        fn can_read_multiple(&self) -> bool {
            self.capable
        }
        fn read_blocks(&mut self, c: Command, buf: &mut [u8]) -> Result<u32, HostError> {
            self.log.push((c.index, c.arg));
            let first = if self.sdhc {
                c.arg
            } else {
                c.arg / BLOCK as u32
            } as usize;
            buf.copy_from_slice(&self.data[first * BLOCK..first * BLOCK + buf.len()]);
            Ok(0x900)
        }
        fn write_block(&mut self, c: Command, buf: &[u8; BLOCK]) -> Result<u32, HostError> {
            self.log.push((c.index, c.arg));
            let block = if self.sdhc {
                c.arg
            } else {
                c.arg / BLOCK as u32
            } as usize;
            self.data[block * BLOCK..(block + 1) * BLOCK].copy_from_slice(buf);
            Ok(0x900)
        }
        fn can_write_multiple(&self) -> bool {
            self.capable
        }
        fn write_blocks(&mut self, c: Command, buf: &[u8]) -> Result<u32, HostError> {
            self.log.push((c.index, c.arg));
            let first = if self.sdhc {
                c.arg
            } else {
                c.arg / BLOCK as u32
            } as usize;
            self.data[first * BLOCK..first * BLOCK + buf.len()].copy_from_slice(buf);
            Ok(0x900)
        }
    }

    #[test]
    fn writes_go_through_cmd24_or_cmd25_and_read_back() {
        let mut fake = FakeCard::new(true, true);
        fake.capable = true;
        let mut card = SdCard::init(fake).unwrap();
        let one = [0xA5u8; BLOCK];
        card.write(2, &one).unwrap();
        let three = vec![0x5Au8; 3 * BLOCK];
        card.write_many(4, &three).unwrap();
        let mut back = vec![0u8; 5 * BLOCK];
        card.read_many(2, &mut back).unwrap();
        assert!(back[..BLOCK].iter().all(|&b| b == 0xA5));
        assert!(
            back[BLOCK..2 * BLOCK].iter().all(|&b| b == 3),
            "block 3 untouched"
        );
        assert!(back[2 * BLOCK..].iter().all(|&b| b == 0x5A));
        let host = card.into_inner();
        let cmds: Vec<(u8, u32)> = host.log[host.log.len() - 5..].to_vec();
        assert_eq!(cmds, [(24, 2), (25, 4), (12, 0), (18, 2), (12, 0)]);
    }

    #[test]
    fn without_multiple_writes_write_many_goes_block_by_block() {
        let mut card = SdCard::init(FakeCard::new(false, false)).unwrap();
        let two = vec![7u8; 2 * BLOCK];
        card.write_many(1, &two).unwrap();
        let host = card.into_inner();
        assert_eq!(&host.log[host.log.len() - 2..], [(24, 512), (24, 1024)]);
        assert!(host.data[BLOCK..3 * BLOCK].iter().all(|&b| b == 7));
    }

    #[test]
    fn the_card_is_a_block_device() {
        let mut card = SdCard::init(FakeCard::new(true, true)).unwrap();
        let mut buf = vec![0u8; BLOCK];
        BlockDevice::read_blocks(&mut card, 6, &mut buf).unwrap();
        assert!(buf.iter().all(|&b| b == 6));
        buf.fill(9);
        BlockDevice::write_blocks(&mut card, 6, &buf).unwrap();
        assert!(
            card.into_inner().data[6 * BLOCK..7 * BLOCK]
                .iter()
                .all(|&b| b == 9)
        );
    }

    #[test]
    fn sdhc_card_initialises_and_reads_by_block() {
        let mut card = SdCard::init(FakeCard::new(true, true)).unwrap();
        assert!(card.is_high_capacity());
        assert_eq!(card.rca(), 0xBEEF);
        let mut buf = [0u8; BLOCK];
        card.read(3, &mut buf).unwrap();
        assert!(buf.iter().all(|&b| b == 3));
        let host = card.into_inner();
        assert_eq!(host.clocks, [INIT_HZ, TRANSFER_HZ]);
        let cmds: Vec<u8> = host.log.iter().map(|&(i, _)| i).collect();
        assert_eq!(cmds, [0, 8, 55, 41, 55, 41, 55, 41, 2, 3, 7, 17]); // no CMD16
        assert_eq!(host.log.last(), Some(&(17, 3)));
    }

    #[test]
    fn v1_card_uses_byte_addresses_and_sets_block_length() {
        let mut card = SdCard::init(FakeCard::new(false, false)).unwrap();
        assert!(!card.is_high_capacity());
        let mut buf = [0u8; BLOCK];
        card.read(2, &mut buf).unwrap();
        assert!(buf.iter().all(|&b| b == 2));
        let host = card.into_inner();
        assert!(host.log.contains(&(16, 512)));
        assert!(host.log.contains(&(41, OCR_VOLTAGES))); // no HCS for v1
        assert_eq!(host.log.last(), Some(&(17, 1024)));
    }

    #[test]
    fn a_capable_host_gets_the_wide_bus_and_multiple_block_reads() {
        let mut fake = FakeCard::new(true, true);
        fake.capable = true;
        let mut card = SdCard::init(fake).unwrap();
        assert!(card.is_wide());
        let mut buf = vec![0u8; 3 * BLOCK];
        card.read_many(4, &mut buf).unwrap();
        for (i, block) in buf.chunks(BLOCK).enumerate() {
            assert!(block.iter().all(|&b| b == 4 + i as u8), "block {i}");
        }
        let host = card.into_inner();
        assert!(host.wide);
        // ACMD6 (with the card's RCA) before the clock goes up; one CMD18 and
        // a CMD12 for all three blocks.
        let tail: Vec<(u8, u32)> = host.log[host.log.len() - 5..].to_vec();
        assert_eq!(
            tail,
            [
                (7, 0xBEEF << 16),
                (55, 0xBEEF << 16),
                (6, 2),
                (18, 4),
                (12, 0)
            ]
        );
    }

    #[test]
    fn without_multiple_reads_read_many_goes_block_by_block() {
        let mut card = SdCard::init(FakeCard::new(true, true)).unwrap();
        assert!(!card.is_wide());
        let mut buf = vec![0u8; 2 * BLOCK];
        card.read_many(5, &mut buf).unwrap();
        assert!(buf[..BLOCK].iter().all(|&b| b == 5));
        assert!(buf[BLOCK..].iter().all(|&b| b == 6));
        let host = card.into_inner();
        assert_eq!(&host.log[host.log.len() - 2..], [(17, 5), (17, 6)]);
    }

    #[test]
    fn card_that_stays_busy_times_out() {
        let mut fake = FakeCard::new(true, true);
        fake.busy_rounds = u32::MAX;
        assert!(matches!(SdCard::init(fake), Err(Error::InitTimeout)));
    }

    #[test]
    fn bad_cmd8_echo_is_rejected() {
        struct Bad(FakeCard);
        impl SdHost for Bad {
            fn set_clock(&mut self, hz: u32) {
                self.0.set_clock(hz)
            }
            fn command(&mut self, c: Command) -> Result<[u32; 4], HostError> {
                if c.index == 8 {
                    Ok([0x1A5, 0, 0, 0])
                } else {
                    self.0.command(c)
                }
            }
            fn read_block(&mut self, c: Command, b: &mut [u8; BLOCK]) -> Result<u32, HostError> {
                self.0.read_block(c, b)
            }
        }
        assert!(matches!(
            SdCard::init(Bad(FakeCard::new(true, true))),
            Err(Error::BadInterfaceCondition)
        ));
    }

    #[test]
    fn error_names() {
        assert_eq!(Error::Host(HostError::Timeout).name(), "timeout");
        assert_eq!(Error::CardStatus(1).name(), "card status error");
    }

    #[test]
    fn parses_mbr() {
        let mut b = [0u8; BLOCK];
        b[446 + 4] = 0x0C;
        b[446 + 8..446 + 12].copy_from_slice(&2048u32.to_le_bytes());
        b[446 + 12..446 + 16].copy_from_slice(&14336u32.to_le_bytes());
        b[510] = 0x55;
        b[511] = 0xAA;
        let p = mbr_partitions(&b).unwrap();
        assert_eq!(
            p[0],
            Some(Partition {
                kind: 0x0C,
                start: 2048,
                blocks: 14336
            })
        );
        assert_eq!(p[1], None);
        b[511] = 0;
        assert!(mbr_partitions(&b).is_none());
    }
}
