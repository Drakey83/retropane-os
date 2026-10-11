//! SD card protocol over any host controller (docs/hw/sd.md).
//!
//! Follows the SD Physical Layer Simplified Specification: reset, interface
//! condition (CMD8), operating conditions (ACMD41, with SDHC support), card
//! identification, selection, then 512-byte block reads. Host controllers
//! implement [`SdHost`].

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

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
        host.set_clock(TRANSFER_HZ);
        Ok(Self {
            host,
            rca,
            high_capacity,
        })
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

    /// Reads block `lba` (512 bytes).
    pub fn read(&mut self, lba: u32, buf: &mut [u8; BLOCK]) -> Result<(), Error> {
        let addr = if self.high_capacity {
            lba
        } else {
            lba.wrapping_mul(BLOCK as u32)
        };
        check(
            self.host
                .read_block(Command::new(17, addr, Response::R1), buf)?,
        )
    }

    /// Gives the host back.
    pub fn into_inner(self) -> H {
        self.host
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
    }

    impl FakeCard {
        fn new(v2: bool, sdhc: bool) -> Self {
            let mut data = vec![0u8; 4 * BLOCK];
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
