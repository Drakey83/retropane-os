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
    /// [`read_blocks`](Self::read_blocks), running `wait` while the host
    /// waits for the data (a host that moves it by DMA); by default it
    /// just reads.
    fn read_blocks_while(
        &mut self,
        cmd: Command,
        buf: &mut [u8],
        wait: &mut dyn FnMut(),
    ) -> Result<u32, HostError> {
        let _ = wait;
        self.read_blocks(cmd, buf)
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
    /// The blocks asked for aren't all on the card (past its capacity, or
    /// the capacity is unknown): refused before any command, since an
    /// SDSC card's byte address would wrap onto another block.
    OutOfRange,
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
            Self::OutOfRange => "block out of range",
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
/// Card status bits that mean *this* command failed (out of range, address,
/// ECC, ...). Not COM_CRC_ERROR (bit 23) or ILLEGAL_COMMAND (bit 22): those
/// report the *previous* command (a command that is itself illegal or
/// garbled gets no response at all), so a command that answered with them
/// succeeded. Counting them made one unanswered CMD12 fail every later
/// read on the X51v (status 0x00400900 on good CMD17s).
const STATUS_ERRORS: u32 = 0xFD39_0008;
/// Card status bit 31: an argument or a transfer past the card's end.
const OUT_OF_RANGE: u32 = 1 << 31;
/// CMD13s [`SdCard::after_reading_to`] sends at most waiting for the
/// `OUT_OF_RANGE` a read of the last block raises: ~1 ms at the X51v's
/// ~13 us a CMD13, 13 times the 76 us the owner's card took every time
/// (guard run version 4, 2026-10-21, 20 of 20). Counted, as the driver's
/// other waits are (it has no clock).
const END_POLLS: u32 = 80;
/// Card status: ready for data (not busy programming).
const STATUS_READY_FOR_DATA: u32 = 1 << 8;
/// Card status: the current state, `prg` = programming.
const STATE_PRG: u32 = 7;
/// Ranges remembered as weak (multi-block reads failed there).
const WEAK_RANGES: usize = 8;
/// Card state: transfer (selected, idle, ready for commands).
const STATE_TRAN: u32 = 4;
/// CMD13 polls while recovering from a failed transfer.
const RECOVER_POLLS: u32 = 1000;
/// Clock for re-reading a block the card is slow on (a quarter of the
/// transfer clock: the controller's read timeout counts card clocks).
pub const SLOW_READ_HZ: u32 = TRANSFER_HZ / 4;
/// Extra attempts at a read that failed in transfer (a CRC error or a
/// stalled block): seen on the X51v as an occasional 64-block read failing
/// under load (ADR-0014's drive), as mature SD stacks retry.
const READ_RETRIES: u32 = 2;
/// CMD13 polls while a write is being programmed (each is a card command,
/// about a millisecond; the specification allows 250 ms per block).
const READY_TRIES: u32 = 200_000;
const ACMD41_TRIES: u32 = 1000;

/// Clock during identification.
pub const INIT_HZ: u32 = 400_000;
/// Clock for data transfer (default speed).
pub const TRANSFER_HZ: u32 = 25_000_000;

/// An identified, selected card as [`SdCard::adopt`] takes it over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity {
    /// Relative card address.
    pub rca: u16,
    /// Block addressing (SDHC/SDXC).
    pub high_capacity: bool,
    /// Switched to the 4-bit bus.
    pub wide: bool,
    /// Capacity in 512-byte blocks.
    pub blocks: u32,
}

/// An initialised, selected SD card.
#[derive(Debug)]
pub struct SdCard<H> {
    host: H,
    rca: u16,
    /// The card takes ACMD23 (pre-erase before a multi-block write); off
    /// for good the first time it refuses.
    pre_erase: bool,
    high_capacity: bool,
    wide: bool,
    /// Capacity in 512-byte blocks, from the CSD register.
    blocks: u32,
    /// The last failure behind a [`BlockDevice`] error (which carries none).
    last_error: Option<Error>,
    /// Read commands issued (a multiple-block read counts once): each costs
    /// a few milliseconds of start-up on the X51v whatever its length.
    reads: u32,
    /// Reads that succeeded on a retry.
    read_retries: u32,
    /// Multi-block reads that succeeded block by block.
    read_fallbacks: u32,
    /// Single blocks read at the slower clock.
    slow_reads: u32,
    /// Reads ending on the last block whose status was collected after.
    end_settles: u32,
    /// Of those, the ones whose OUT_OF_RANGE never came (see `end_unseen`).
    end_unseen: u32,
    /// Ranges (first block, count) read block by block from now on.
    weak: [(u32, u32); WEAK_RANGES],
    weak_len: u8,
    weak_next: u8,
}

impl<H: SdHost> SdCard<H> {
    /// Identifies and selects the card.
    pub fn init(host: H) -> Result<Self, Error> {
        Self::try_init(host).map_err(|(e, _)| e)
    }

    /// Like [`SdCard::init`], but a failure gives the host back, so a card
    /// inserted later can be initialised on it (hot swap).
    pub fn try_init(mut host: H) -> Result<Self, (Error, H)> {
        match Self::identify(&mut host) {
            Ok((rca, high_capacity, wide, blocks)) => Ok(Self {
                host,
                rca,
                pre_erase: true,
                high_capacity,
                wide,
                blocks,
                last_error: None,
                reads: 0,
                read_retries: 0,
                read_fallbacks: 0,
                slow_reads: 0,
                end_settles: 0,
                end_unseen: 0,
                weak: [(0, 0); WEAK_RANGES],
                weak_len: 0,
                weak_next: 0,
            }),
            Err(e) => Err((e, host)),
        }
    }

    /// Takes over a card someone else identified and selected on this
    /// controller (the loader, for a program's card log: `rp_hal_x51v::
    /// cardlog`), as [`SdCard::identity`] described it: no command is sent,
    /// so the card stays as it was (transfer state, its bus width and
    /// clock), and the host is told of the 4-bit bus.
    pub fn adopt(mut host: H, id: Identity) -> Self {
        if id.wide {
            host.set_wide_bus();
        }
        Self {
            host,
            rca: id.rca,
            pre_erase: false,
            high_capacity: id.high_capacity,
            wide: id.wide,
            blocks: id.blocks,
            last_error: None,
            reads: 0,
            read_retries: 0,
            read_fallbacks: 0,
            slow_reads: 0,
            end_settles: 0,
            end_unseen: 0,
            weak: [(0, 0); WEAK_RANGES],
            weak_len: 0,
            weak_next: 0,
        }
    }

    /// What [`SdCard::adopt`] needs to take this card over.
    #[must_use]
    pub fn identity(&self) -> Identity {
        Identity {
            rca: self.rca,
            high_capacity: self.high_capacity,
            wide: self.wide,
            blocks: self.blocks,
        }
    }

    fn identify(host: &mut H) -> Result<(u16, bool, bool, u32), Error> {
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
        // CSD (standby state, before selection): the capacity.
        let blocks =
            csd_blocks(host.command(Command::new(9, u32::from(rca) << 16, Response::R2))?);
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
        Ok((rca, high_capacity, wide, blocks))
    }

    /// Capacity in 512-byte blocks (0 if the CSD was unreadable).
    #[must_use]
    pub fn blocks(&self) -> u32 {
        self.blocks
    }

    /// The card error behind the last [`BlockDevice`] failure, if any.
    #[must_use]
    pub fn last_error(&self) -> Option<Error> {
        self.last_error
    }

    /// Read commands issued so far (a multiple-block read counts once).
    #[must_use]
    pub fn reads(&self) -> u32 {
        self.reads
    }

    /// Reads that needed a retry and then succeeded.
    #[must_use]
    pub fn read_retries(&self) -> u32 {
        self.read_retries
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

    /// Checks `count` blocks from `lba` lie on the card. Every public read
    /// and write starts here: below it, `address` may wrap.
    fn on_card(&self, lba: u32, count: usize) -> Result<(), Error> {
        let fits = u32::try_from(count)
            .ok()
            .and_then(|n| lba.checked_add(n))
            .is_some_and(|end| end <= self.blocks);
        if fits { Ok(()) } else { Err(Error::OutOfRange) }
    }

    fn address(&self, lba: u32) -> u32 {
        if self.high_capacity {
            lba
        } else {
            lba.wrapping_mul(BLOCK as u32)
        }
    }

    /// Reads block `lba` (512 bytes).
    // One copy, however many callers (ADR-0005 rule 8: size).
    #[inline(never)]
    pub fn read(&mut self, lba: u32, buf: &mut [u8; BLOCK]) -> Result<(), Error> {
        self.on_card(lba, 1)?;
        let first = self.read_once(lba, buf);
        if !matches!(first, Err(Error::Host(_))) {
            return first;
        }
        // A block the card is slow on: the controller's read timeout counts
        // card clocks, so a quarter of the clock gives it four times longer
        // (the owner's card has one 0.79 s block, past the full-speed
        // timeout). Back to full speed whatever happens.
        // A quarter of the clock first, then the slowest the host offers
        // (card-clock timeouts: up to ~64 times longer at 305 kHz).
        let mut result = first;
        for hz in [SLOW_READ_HZ, INIT_HZ] {
            self.recover();
            self.host.set_clock(hz);
            result = self.read_once(lba, buf);
            self.host.set_clock(TRANSFER_HZ);
            if result.is_ok() {
                self.slow_reads = self.slow_reads.wrapping_add(1);
                return result;
            }
        }
        self.recover();
        result
    }

    fn read_once(&mut self, lba: u32, buf: &mut [u8; BLOCK]) -> Result<(), Error> {
        self.reads = self.reads.wrapping_add(1);
        let addr = self.address(lba);
        check(
            self.host
                .read_block(Command::new(17, addr, Response::R1), buf)?,
        )?;
        self.after_reading_to(lba, 1)
    }

    /// After a read of blocks `lba..lba + count`: if it ended on the card's
    /// last block, the `OUT_OF_RANGE` the card raises after it is waited
    /// for and collected (quality audit C46, the owner's option a). The
    /// owner's 32 GB card on the X51v raises it under 76 us after the read
    /// (guard run version 4, 20 of 20; a CMD13 straight after, version 3's
    /// fix, came too early) and reports it in the next command's status,
    /// whatever that is: every read after one of the last block failed
    /// with `0x80000900`. CMD13 is sent until the bit shows (then it is
    /// gone), [`END_POLLS`] at most; that bit alone is expected there,
    /// anything else CMD13 reports is an error. Other reads send nothing
    /// more.
    #[inline(always)]
    fn after_reading_to(&mut self, lba: u32, count: u32) -> Result<(), Error> {
        if lba.saturating_add(count) != self.blocks {
            return Ok(());
        }
        self.settle_end()
    }

    /// [`Self::after_reading_to`]'s CMD13, out of line: every other read
    /// pays only the compare (ADR-0005 rule 8; a call per read cost
    /// `sd_read_4_blocks` 1.5% in the emulator).
    #[cold]
    #[inline(never)]
    fn settle_end(&mut self) -> Result<(), Error> {
        let arg = u32::from(self.rca) << 16;
        self.end_settles = self.end_settles.wrapping_add(1);
        for _ in 0..END_POLLS {
            let status = self.host.command(Command::new(13, arg, Response::R1))?[0];
            check(status & !OUT_OF_RANGE)?;
            if status & OUT_OF_RANGE != 0 {
                return Ok(());
            }
        }
        // A card that doesn't raise it (or not within the bound).
        self.end_unseen = self.end_unseen.wrapping_add(1);
        Ok(())
    }

    /// Reads ending on the last block after which no `OUT_OF_RANGE` came
    /// within [`END_POLLS`] CMD13s (a card that doesn't raise it; if one
    /// raises it later, the next command fails as before C46).
    #[must_use]
    pub fn end_unseen(&self) -> u32 {
        self.end_unseen
    }

    /// Reads that ended on the last block and had the card's status
    /// collected after (see [`Self::after_reading_to`]).
    #[must_use]
    pub fn end_settles(&self) -> u32 {
        self.end_settles
    }

    /// Single blocks that needed the slower clock to read.
    #[must_use]
    pub fn slow_reads(&self) -> u32 {
        self.slow_reads
    }

    /// Reads `buf.len() / 512` consecutive blocks from `lba` with one
    /// command when the host can (otherwise block by block).
    ///
    /// # Panics
    /// If `buf` isn't a whole number of blocks.
    // One copy, however many callers (ADR-0005 rule 8: size).
    #[inline(never)]
    pub fn read_many(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), Error> {
        self.read_many_while(lba, buf, &mut || {})
    }

    /// [`read_many`](Self::read_many), running `wait` while the host waits
    /// for the data of the first attempt (by DMA: useful work instead of
    /// spinning). Retries and the block-by-block fallback run without it,
    /// exactly as `read_many`'s do.
    pub fn read_many_while(
        &mut self,
        lba: u32,
        buf: &mut [u8],
        wait: &mut dyn FnMut(),
    ) -> Result<(), Error> {
        self.on_card(lba, buf.len() / BLOCK)?;
        let blocks = (buf.len() / BLOCK) as u32;
        if blocks > 1 && self.touches_weak(lba, blocks) {
            // A range a multi-block read failed on before: block by block,
            // so the weak block only ever sees gentle single reads.
            return self.read_singly(lba, buf);
        }
        let mut result = self.read_many_once_while(lba, buf, wait);
        // A transfer error (CRC, a stalled block) may pass on a retry; a read
        // timeout won't: the card is slow on some block in the range, and
        // the multi-block command's timeout is shorter than a single
        // block's (measured on the owner's card: one block takes 0.79 s,
        // which CMD18 never survives and CMD17 does). So timeouts go
        // straight to reading block by block.
        for _ in 0..READ_RETRIES {
            match result {
                Err(Error::Host(HostError::Crc | HostError::Data)) => {}
                _ => break,
            }
            self.recover();
            result = self.read_many_once(lba, buf);
            if result.is_ok() {
                self.read_retries = self.read_retries.wrapping_add(1);
            }
        }
        if matches!(result, Err(Error::Host(_))) && blocks > 1 {
            // Remember the range: on the owner's card, every further
            // multi-block read of a weak block leaves the card unable to
            // deliver it for a while, so retrying that way makes it worse.
            self.remember_weak(lba, blocks);
            self.recover();
            result = self.read_singly(lba, buf);
            if result.is_ok() {
                self.read_fallbacks = self.read_fallbacks.wrapping_add(1);
            }
        }
        result
    }

    fn read_singly(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), Error> {
        buf.as_chunks_mut::<BLOCK>()
            .0
            .iter_mut()
            .zip(lba..)
            .try_for_each(|(block, b)| self.read(b, block))
    }

    fn touches_weak(&self, lba: u32, blocks: u32) -> bool {
        let end = lba.saturating_add(blocks);
        self.weak[..usize::from(self.weak_len)]
            .iter()
            .any(|&(start, n)| lba < start.saturating_add(n) && start < end)
    }

    fn remember_weak(&mut self, lba: u32, blocks: u32) {
        if self.touches_weak(lba, blocks) {
            return;
        }
        // A small ring: the newest ranges win.
        let i = usize::from(self.weak_next) % WEAK_RANGES;
        self.weak[i] = (lba, blocks);
        self.weak_next = self.weak_next.wrapping_add(1);
        self.weak_len = (self.weak_len + 1).min(WEAK_RANGES as u8);
    }

    /// How many ranges are read block by block (multi-block reads failed
    /// there).
    #[must_use]
    pub fn weak_ranges(&self) -> usize {
        usize::from(self.weak_len)
    }

    /// After a failed transfer: reads (and so clears) the card's status.
    /// When a read times out the card has already left the data state, so
    /// the CMD12 that stops it is illegal and unanswered, and the card
    /// reports ILLEGAL_COMMAND in its *next* response (SD specification,
    /// "clear on read" bits; seen on the X51v as a good CMD17 failing right
    /// after a timed-out CMD18). CMD13 takes that report; its status is
    /// deliberately not checked.
    ///
    /// The card may instead still be sending (the controller gave up on a
    /// slow block, the card didn't): then the next command would be illegal
    /// and unanswered. So: CMD12 first (it stops a card that is still
    /// sending, and is harmlessly illegal for one that isn't), then CMD13
    /// until the card is back in the transfer state, ready for data.
    fn recover(&mut self) {
        let _ = self.host.command(Command::new(12, 0, Response::R1b));
        let arg = u32::from(self.rca) << 16;
        for _ in 0..RECOVER_POLLS {
            if let Ok(r) = self.host.command(Command::new(13, arg, Response::R1))
                && (r[0] >> 9) & 0xF == STATE_TRAN
                && r[0] & STATUS_READY_FOR_DATA != 0
            {
                return;
            }
        }
    }

    /// Multi-block reads that failed and then succeeded block by block.
    #[must_use]
    pub fn read_fallbacks(&self) -> u32 {
        self.read_fallbacks
    }

    fn read_many_once(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), Error> {
        self.read_many_once_while(lba, buf, &mut || {})
    }

    // Out of line: `write_many`'s read-back would otherwise carry a copy.
    #[inline(never)]
    fn read_many_once_while(
        &mut self,
        lba: u32,
        buf: &mut [u8],
        wait: &mut dyn FnMut(),
    ) -> Result<(), Error> {
        assert_eq!(buf.len() % BLOCK, 0, "not a whole number of blocks");
        if buf.len() > BLOCK && self.host.can_read_multiple() {
            self.reads = self.reads.wrapping_add(1);
            let addr = self.address(lba);
            let status =
                self.host
                    .read_blocks_while(Command::new(18, addr, Response::R1), buf, wait);
            // Stop the card whatever happened, so it accepts commands again.
            let stop = self.host.command(Command::new(12, 0, Response::R1b));
            check(status?)?;
            stop?;
            return self.after_reading_to(lba, (buf.len() / BLOCK) as u32);
        }
        for (i, block) in buf.as_chunks_mut::<BLOCK>().0.iter_mut().enumerate() {
            self.read(lba + i as u32, block)?;
        }
        Ok(())
    }

    /// Writes block `lba` (512 bytes).
    pub fn write(&mut self, lba: u32, buf: &[u8; BLOCK]) -> Result<(), Error> {
        self.on_card(lba, 1)?;
        let addr = self.address(lba);
        check(
            self.host
                .write_block(Command::new(24, addr, Response::R1), buf)?,
        )?;
        self.wait_ready()
    }

    /// Waits until the card has finished programming (CMD13: ready for
    /// data, not in the `prg` state). Host controllers' own busy handling
    /// doesn't always cover the programming after a stop (seen on the
    /// X51v: the command after a multi-block write got no response).
    fn wait_ready(&mut self) -> Result<(), Error> {
        let arg = u32::from(self.rca) << 16;
        for _ in 0..READY_TRIES {
            let status = self.host.command(Command::new(13, arg, Response::R1))?[0];
            check(status)?;
            if status & STATUS_READY_FOR_DATA != 0 && (status >> 9) & 0xF != STATE_PRG {
                return Ok(());
            }
        }
        Err(Error::Host(HostError::Timeout))
    }

    /// Writes `buf.len() / 512` consecutive blocks from `lba` with one
    /// command when the host can (otherwise block by block).
    ///
    /// # Panics
    /// If `buf` isn't a whole number of blocks.
    // One copy, however many callers (ADR-0005 rule 8: size).
    #[inline(never)]
    pub fn write_many(&mut self, lba: u32, buf: &[u8]) -> Result<(), Error> {
        assert_eq!(buf.len() % BLOCK, 0, "not a whole number of blocks");
        self.on_card(lba, buf.len() / BLOCK)?;
        if buf.len() > BLOCK && self.host.can_write_multiple() {
            self.announce_write((buf.len() / BLOCK) as u32);
            let addr = self.address(lba);
            let status = self
                .host
                .write_blocks(Command::new(25, addr, Response::R1), buf);
            let stop = self.host.command(Command::new(12, 0, Response::R1b));
            check(status?)?;
            stop?;
            return self.wait_ready();
        }
        for (i, block) in buf.as_chunks::<BLOCK>().0.iter().enumerate() {
            self.write(lba + i as u32, block)?;
        }
        Ok(())
    }

    /// Tells the card how many blocks the next multi-block write brings
    /// (ACMD23, SET_WR_BLK_ERASE_COUNT), so it can erase them ahead: SD
    /// cards write faster that way (item 58). Every SD memory card should
    /// take it; one that refuses (no answer, "illegal command" in its next
    /// status) has its status read, which clears that, and isn't asked
    /// again. If the write is then cut off, only blocks inside the range
    /// being written can be left erased.
    fn announce_write(&mut self, blocks: u32) {
        if !self.pre_erase {
            return;
        }
        let rca = u32::from(self.rca) << 16;
        let taken = self
            .host
            .command(Command::new(55, rca, Response::R1))
            .is_ok()
            && self
                .host
                .command(Command::new(23, blocks & 0x7F_FFFF, Response::R1))
                .is_ok_and(|r| check(r[0]).is_ok());
        if !taken {
            self.pre_erase = false;
            let _ = self.host.command(Command::new(13, rca, Response::R1));
        }
    }

    /// Whether the card takes ACMD23 (pre-erase), as far as known.
    #[must_use]
    pub fn pre_erases(&self) -> bool {
        self.pre_erase
    }

    /// The host controller, for its diagnostics.
    #[must_use]
    pub fn host(&self) -> &H {
        &self.host
    }

    /// The host controller, for its settings (DMA use, say). Not for
    /// commands: those would leave the card in a state this driver doesn't
    /// know about.
    pub fn host_mut(&mut self) -> &mut H {
        &mut self.host
    }

    /// Gives the host back.
    pub fn into_inner(self) -> H {
        self.host
    }

    /// A card error as the block-device interface names it (the card's
    /// own reason kept for [`last_error`](Self::last_error)).
    fn block_error(&mut self, e: Error) -> BlockError {
        if e == Error::OutOfRange {
            return BlockError::OutOfRange;
        }
        self.last_error = Some(e);
        BlockError::Device
    }
}

impl<H: SdHost> BlockDevice for SdCard<H> {
    fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
        self.read_many(lba, buf).map_err(|e| self.block_error(e))
    }

    fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
        self.write_many(lba, buf).map_err(|e| self.block_error(e))
    }
}

/// `size` bits of a 128-bit register starting at bit `start` (0 = least
/// significant), from the four response words, most significant first.
fn bits(r: [u32; 4], start: u32, size: u32) -> u32 {
    let word = |i: u32| -> u64 {
        if i < 4 {
            u64::from(r[3 - i as usize])
        } else {
            0
        }
    };
    let lo = start / 32;
    let v = word(lo) | word(lo + 1) << 32;
    ((v >> (start % 32)) & ((1u64 << size) - 1)) as u32
}

/// Card capacity in 512-byte blocks from its CSD (SD Physical Layer
/// specification 5.3): version 2 (SDHC/SDXC) counts 512 KiB units, version
/// 1 (SDSC) multiplies; anything else reads as 0 (unknown).
fn csd_blocks(csd: [u32; 4]) -> u32 {
    match bits(csd, 126, 2) {
        1 => (bits(csd, 48, 22) + 1).saturating_mul(1024),
        0 => {
            let c_size = bits(csd, 62, 12);
            let mult = bits(csd, 47, 3);
            let read_bl_len = bits(csd, 80, 4);
            let bytes = u64::from(c_size + 1) << (mult + 2) << read_bl_len;
            u32::try_from(bytes / BLOCK as u64).unwrap_or(u32::MAX)
        }
        _ => 0,
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
        /// Data commands answer with an error status.
        fail_status: bool,
        /// Multi-block reads that fail in transfer before one succeeds.
        flaky_reads: u32,
        /// A block the card is too slow on for a multi-block read (a read
        /// timeout), though it reads alone.
        slow_block: Option<usize>,
        /// A block that can't be read at all.
        bad_block: Option<usize>,
        /// A block too slow to read alone at full speed (fine slower).
        very_slow_block: Option<usize>,
        /// The last data transfer ended early (the card left the data state).
        transfer_aborted: bool,
        /// ILLEGAL_COMMAND is reported in the next response.
        illegal_pending: bool,
        /// The card is still sending a block the controller gave up on:
        /// any command but CMD12 is illegal (unanswered).
        still_sending: bool,
        /// CMD13 polls still reporting "programming" after a write.
        busy_polls: u32,
        /// Refuses ACMD23 (no answer, illegal command reported next).
        no_acmd23: bool,
        /// Raises OUT_OF_RANGE reading the last block, reported in the
        /// next command's status (the owner's card on the X51v, C46).
        end_out_of_range: bool,
        /// OUT_OF_RANGE to report in the next response.
        oor_pending: bool,
        /// Commands after a last-block read before the card raises it (the
        /// X51v's card: it came at the second CMD13, ~76 us).
        end_delay: u32,
        /// Commands still to go before it is raised.
        oor_in: Option<u32>,
        /// Status bits every CMD13 adds.
        cmd13_extra: u32,
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
                fail_status: false,
                flaky_reads: 0,
                slow_block: None,
                bad_block: None,
                very_slow_block: None,
                transfer_aborted: false,
                illegal_pending: false,
                still_sending: false,
                busy_polls: 0,
                no_acmd23: false,
                end_out_of_range: false,
                oor_pending: false,
                end_delay: 0,
                oor_in: None,
                cmd13_extra: 0,
            }
        }

        fn data_status(&self) -> u32 {
            if self.fail_status { 0x8000_0900 } else { 0x900 }
        }

        /// OUT_OF_RANGE raised after `n` more commands (at once for 0).
        fn raise_after(&mut self, n: u32) {
            if n == 0 {
                self.oor_pending = true;
            } else {
                self.oor_in = Some(n);
            }
        }

        /// A command arrives: a delayed OUT_OF_RANGE counts down, and is
        /// raised (reported from the next response on) when it reaches 0.
        fn tick(&mut self) {
            if let Some(n) = self.oor_in {
                if n <= 1 {
                    self.oor_in = None;
                    self.oor_pending = true;
                } else {
                    self.oor_in = Some(n - 1);
                }
            }
        }

        /// The card's capacity in blocks, as its CSD gives it.
        fn capacity(&self) -> usize {
            if self.sdhc { 1024 } else { 8 }
        }

        /// A data transfer of `range` done: the X51v card's OUT_OF_RANGE
        /// raised if it ended on the last block. Returns the bit pending
        /// from before (reported in this response).
        fn transfer_done(&mut self, range: core::ops::Range<usize>) -> u32 {
            let earlier = if core::mem::take(&mut self.oor_pending) {
                OUT_OF_RANGE
            } else {
                0
            };
            if self.end_out_of_range && range.end == self.capacity() {
                self.raise_after(self.end_delay);
            }
            earlier
        }
    }

    impl SdHost for FakeCard {
        fn set_clock(&mut self, hz: u32) {
            self.clocks.push(hz);
        }
        fn command(&mut self, c: Command) -> Result<[u32; 4], HostError> {
            self.log.push((c.index, c.arg));
            self.tick();
            if c.index == 9 {
                // CSD v2 with C_SIZE 0 (1024 blocks) for SDHC; v1 with
                // READ_BL_LEN 9, C_SIZE 1, C_SIZE_MULT 0 (8 blocks) for SDSC.
                return Ok(if self.sdhc {
                    [0x4000_0000, 0, 0, 0]
                } else {
                    [0, 0x0009_0000, 0x4000_0000, 0]
                });
            }
            if self.still_sending {
                if c.index == 12 {
                    self.still_sending = false;
                    return Ok([0x0B00, 0, 0, 0]);
                }
                self.illegal_pending = true;
                return Err(HostError::Timeout);
            }
            if c.index == 12 && core::mem::take(&mut self.transfer_aborted) {
                // Not in a data state: illegal, unanswered, reported later.
                self.illegal_pending = true;
                return Err(HostError::Timeout);
            }
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
                (13, _) => {
                    if self.busy_polls > 0 {
                        self.busy_polls -= 1;
                        STATE_PRG << 9 // programming, not ready
                    } else {
                        0x900 | self.cmd13_extra // tran state, ready for data
                    }
                }
                (6, true) if c.arg == 2 => 0x0920,
                (23, true) if !self.no_acmd23 => 0x0920,
                (23, true) => {
                    self.illegal_pending = true;
                    return Err(HostError::Timeout);
                }
                (12, _) => 0x0B00,
                _ => return Err(HostError::Timeout),
            };
            let illegal = if core::mem::take(&mut self.illegal_pending) {
                1 << 22
            } else {
                0
            };
            let oor = if core::mem::take(&mut self.oor_pending) {
                OUT_OF_RANGE
            } else {
                0
            };
            Ok([r | illegal | oor, 0, 0, 0])
        }
        fn read_block(&mut self, c: Command, buf: &mut [u8; BLOCK]) -> Result<u32, HostError> {
            self.log.push((c.index, c.arg));
            if self.still_sending {
                self.illegal_pending = true;
                return Err(HostError::Timeout);
            }
            let block = if self.sdhc {
                c.arg
            } else {
                c.arg / BLOCK as u32
            } as usize;
            if self.bad_block == Some(block) {
                return Err(HostError::Timeout);
            }
            if self.very_slow_block == Some(block) && self.clocks.last() == Some(&TRANSFER_HZ) {
                // The card goes on sending after the controller gave up.
                self.transfer_aborted = false;
                self.still_sending = true;
                return Err(HostError::Timeout);
            }
            if core::mem::take(&mut self.illegal_pending) {
                // The previous command's error, reported now.
                buf.copy_from_slice(&self.data[block * BLOCK..(block + 1) * BLOCK]);
                return Ok(0x900 | 1 << 22);
            }
            buf.copy_from_slice(&self.data[block * BLOCK..(block + 1) * BLOCK]);
            let earlier = self.transfer_done(block..block + 1);
            Ok(self.data_status() | earlier)
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
            if self.flaky_reads > 0 {
                self.flaky_reads -= 1;
                return Err(HostError::Crc);
            }
            let first = if self.sdhc {
                c.arg
            } else {
                c.arg / BLOCK as u32
            } as usize;
            let range = first..first + buf.len() / BLOCK;
            if self.slow_block.is_some_and(|b| range.contains(&b))
                || self.bad_block.is_some_and(|b| range.contains(&b))
            {
                self.transfer_aborted = true;
                return Err(HostError::Timeout);
            }
            buf.copy_from_slice(&self.data[first * BLOCK..first * BLOCK + buf.len()]);
            let earlier = self.transfer_done(range);
            Ok(0x900 | earlier)
        }
        fn write_block(&mut self, c: Command, buf: &[u8; BLOCK]) -> Result<u32, HostError> {
            self.log.push((c.index, c.arg));
            if self.busy_polls != u32::MAX {
                self.busy_polls = 2;
            }
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
            if self.busy_polls != u32::MAX {
                self.busy_polls = 3;
            }
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
        // Every write is followed by CMD13 polls until the card is ready
        // (the fake stays "programming" for 2 polls after CMD24, 3 after
        // CMD25 + CMD12); a multi-block write is announced first (CMD55 +
        // ACMD23 with its block count: the card pre-erases them).
        let cmds: Vec<(u8, u32)> = host.log[host.log.len() - 14..].to_vec();
        let idx: Vec<u8> = cmds.iter().map(|&(i, _)| i).collect();
        assert_eq!(
            idx,
            [24, 13, 13, 13, 55, 23, 25, 12, 13, 13, 13, 13, 18, 12]
        );
        assert_eq!(cmds[5].1, 3, "ACMD23 carries the block count");
    }

    /// A card taken over (the loader's, for a program's card log): no
    /// command until the first write, which goes to the same place as the
    /// identifying owner's would.
    #[test]
    fn an_adopted_card_writes_without_being_identified_again() {
        let mut fake = FakeCard::new(true, true);
        fake.capable = true;
        let owner = SdCard::init(fake).unwrap();
        let id = owner.identity();
        assert!(id.high_capacity && id.wide);
        let mut host = owner.into_inner();
        let before = host.log.len();
        host.wide = false;
        let mut card = SdCard::adopt(host, id);
        assert_eq!(card.host().log.len(), before, "no command at adoption");
        assert!(card.host().wide, "the host is told of the 4-bit bus");
        card.write(3, &[0x3Cu8; BLOCK]).unwrap();
        let host = card.into_inner();
        assert_eq!(host.log[before].0, 24);
        assert!(host.data[3 * BLOCK..4 * BLOCK].iter().all(|&b| b == 0x3C));
    }

    /// A card that refuses ACMD23 (item 58): its status is read (clearing
    /// the illegal-command report), the write goes ahead and succeeds, and
    /// the card isn't asked again.
    #[test]
    fn a_card_refusing_pre_erase_still_writes_and_isnt_asked_again() {
        let mut fake = FakeCard::new(true, true);
        fake.capable = true;
        fake.no_acmd23 = true;
        let mut card = SdCard::init(fake).unwrap();
        let three = vec![0x5Au8; 3 * BLOCK];
        card.write_many(2, &three).unwrap();
        assert!(!card.pre_erases());
        card.write_many(5, &three[..2 * BLOCK]).unwrap();
        let mut back = vec![0u8; 5 * BLOCK];
        card.read_many(2, &mut back).unwrap();
        assert!(back.iter().all(|&b| b == 0x5A));
        let host = card.into_inner();
        let acmd23s = host.log.iter().filter(|&&(i, _)| i == 23).count();
        assert_eq!(acmd23s, 1, "asked once, then never again");
    }

    #[test]
    fn a_card_that_never_finishes_programming_times_out() {
        let mut card = SdCard::init(FakeCard::new(true, true)).unwrap();
        card.host.busy_polls = u32::MAX; // stuck in programming
        let r = card.write(1, &[0u8; BLOCK]);
        assert_eq!(r, Err(Error::Host(HostError::Timeout)));
    }

    #[test]
    fn without_multiple_writes_write_many_goes_block_by_block() {
        let mut card = SdCard::init(FakeCard::new(false, false)).unwrap();
        let two = vec![7u8; 2 * BLOCK];
        card.write_many(1, &two).unwrap();
        let host = card.into_inner();
        let writes: Vec<(u8, u32)> = host.log.iter().copied().filter(|&(i, _)| i == 24).collect();
        assert_eq!(writes, [(24, 512), (24, 1024)]);
        assert!(host.data[BLOCK..3 * BLOCK].iter().all(|&b| b == 7));
    }

    #[test]
    fn a_block_device_failure_keeps_the_card_error() {
        let mut card = SdCard::init(FakeCard::new(true, true)).unwrap();
        assert_eq!(card.last_error(), None);
        let mut buf = [0u8; BLOCK];
        // CMD17 to the fake's unknown block range: the fake copies from its
        // data, so make the failure a card status error instead.
        card.host.fail_status = true;
        assert_eq!(
            BlockDevice::read_blocks(&mut card, 1, &mut buf),
            Err(BlockError::Device)
        );
        assert!(matches!(card.last_error(), Some(Error::CardStatus(_))));
    }

    #[test]
    fn a_read_that_fails_in_transfer_is_retried() {
        let mut fake = FakeCard::new(true, true);
        fake.capable = true;
        let mut card = SdCard::init(fake).unwrap();
        card.host.flaky_reads = 2;
        let mut buf = vec![0u8; 2 * BLOCK];
        card.read_many(2, &mut buf).unwrap();
        assert!(buf[..BLOCK].iter().all(|&b| b == 2));
        assert_eq!(card.read_retries(), 1);
        // Three CMD18 attempts, each followed by recovery before the next.
        let cmds: Vec<u8> = card.host.log.iter().map(|&(i, _)| i).collect();
        assert_eq!(cmds.iter().filter(|&&c| c == 18).count(), 3);
        let between = &cmds[cmds.iter().position(|&c| c == 18).unwrap()..];
        assert!(between.windows(2).any(|w| w == [12, 13]), "{between:?}");
        // Three failures in a row: read block by block instead.
        card.host.flaky_reads = 3;
        card.read_many(2, &mut buf).unwrap();
        assert!(buf[BLOCK..].iter().all(|&b| b == 3));
        assert_eq!(card.read_fallbacks(), 1);
    }

    #[test]
    fn a_slow_block_times_out_a_multi_block_read_and_is_read_alone() {
        let mut fake = FakeCard::new(true, true);
        fake.capable = true;
        fake.slow_block = Some(4);
        let mut card = SdCard::init(fake).unwrap();
        let mut buf = vec![0u8; 4 * BLOCK];
        card.read_many(2, &mut buf).unwrap();
        for (i, b) in buf.chunks(BLOCK).enumerate() {
            assert!(b.iter().all(|&x| x == 2 + i as u8), "block {i}");
        }
        // No retry of a command that will time out again: one CMD18, then
        // recovery (CMD12/CMD13), then the four blocks alone.
        let cmds: Vec<u8> = card.host.log.iter().map(|&(i, _)| i).collect();
        let from = cmds.iter().rposition(|&c| c == 18).unwrap();
        assert_eq!(cmds[from..].iter().filter(|&&c| c == 18).count(), 1);
        assert_eq!(cmds[from..].iter().filter(|&&c| c == 17).count(), 4);
        assert!(cmds[from..].ends_with(&[17, 17, 17, 17]));
        assert_eq!((card.read_retries(), card.read_fallbacks()), (0, 1));
    }

    /// C46, the X51v card's way: reading the last block raises
    /// OUT_OF_RANGE, reported in the next command's status. The read after
    /// it must not fail (on the device every later read did), whichever
    /// read comes next, single or multi-block, the last block again too.
    #[test]
    fn a_read_ending_on_the_last_block_leaves_no_error_for_the_next() {
        for capable in [false, true] {
            let mut fake = FakeCard::new(true, false);
            fake.capable = capable;
            fake.end_out_of_range = true;
            // As the X51v's card: raised by the second command after the
            // read (version 3's single CMD13 straight after missed it).
            fake.end_delay = 2;
            let mut card = SdCard::init(fake).unwrap();
            let n = card.blocks();
            assert_eq!(n, 8);
            let mut one = [0u8; BLOCK];
            let mut two = vec![0u8; 2 * BLOCK];
            for _ in 0..2 {
                card.read(n - 1, &mut one).unwrap();
                assert!(one.iter().all(|&b| b == 7));
                card.read(0, &mut one).unwrap();
                card.read_many(n - 2, &mut two).unwrap();
                assert!(two[BLOCK..].iter().all(|&b| b == 7));
                card.read(1, &mut one).unwrap();
            }
            assert_eq!(card.end_settles(), 4, "capable {capable}");
            assert_eq!(card.end_unseen(), 0, "capable {capable}");
            // Reads elsewhere send nothing more.
            let before = card.host.log.len();
            card.read(2, &mut one).unwrap();
            assert_eq!(card.host.log.len(), before + 1);
        }
    }

    /// A card that never raises it: the wait ends at its bound, counted,
    /// and the reads go on.
    #[test]
    fn a_card_without_the_late_out_of_range_waits_its_bound_only() {
        let mut card = SdCard::init(FakeCard::new(true, false)).unwrap();
        let before = card.host.log.len();
        let mut one = [0u8; BLOCK];
        card.read(7, &mut one).unwrap();
        card.read(0, &mut one).unwrap();
        let cmd13s = card.host.log[before..].iter().filter(|c| c.0 == 13).count();
        assert_eq!(cmd13s, END_POLLS as usize);
        assert_eq!((card.end_settles(), card.end_unseen()), (1, 1));
    }

    /// The status collected after the last block is still checked: an
    /// error beside OUT_OF_RANGE there (a card ECC failure, bit 21) fails
    /// the read.
    #[test]
    fn another_error_after_the_last_block_still_fails() {
        let mut fake = FakeCard::new(true, false);
        fake.end_out_of_range = true;
        let mut card = SdCard::init(fake).unwrap();
        card.host.cmd13_extra = 1 << 21;
        let mut one = [0u8; BLOCK];
        assert!(matches!(card.read(7, &mut one), Err(Error::CardStatus(_))));
    }

    #[test]
    fn a_previous_commands_error_bits_dont_fail_a_good_read() {
        let mut card = SdCard::init(FakeCard::new(true, true)).unwrap();
        // ILLEGAL_COMMAND from an earlier, unanswered command arrives in
        // this read's response; the read itself is fine.
        card.host.illegal_pending = true;
        let mut buf = [0u8; BLOCK];
        card.read(5, &mut buf).unwrap();
        assert!(buf.iter().all(|&b| b == 5));
        // An error about this command (OUT_OF_RANGE) still fails it.
        assert!(check(1 << 31).is_err());
        assert!(check(1 << 22 | 1 << 23).is_ok());
    }

    #[test]
    fn a_block_too_slow_even_alone_is_read_at_a_slower_clock() {
        let mut fake = FakeCard::new(true, true);
        fake.capable = true;
        fake.slow_block = Some(4);
        fake.very_slow_block = Some(4);
        let mut card = SdCard::init(fake).unwrap();
        let mut buf = vec![0u8; 4 * BLOCK];
        card.read_many(2, &mut buf).unwrap();
        for (i, b) in buf.chunks(BLOCK).enumerate() {
            assert!(b.iter().all(|&x| x == 2 + i as u8), "block {i}");
        }
        assert_eq!((card.read_fallbacks(), card.slow_reads()), (1, 1));
        // The clock went down for the one block and back up.
        let host = card.into_inner();
        assert_eq!(
            &host.clocks[host.clocks.len() - 2..],
            [SLOW_READ_HZ, TRANSFER_HZ]
        );
        // After the timed-out block the card was stopped (CMD12 answered,
        // it was still sending) before anything else.
        let i = host
            .log
            .iter()
            .rposition(|&(c, a)| c == 17 && a == 4)
            .unwrap();
        let before: Vec<u8> = host.log[..i]
            .iter()
            .rev()
            .take(3)
            .map(|&(c, _)| c)
            .collect();
        assert_eq!(
            before,
            [13, 12, 17],
            "CMD17 timed out, CMD12 stopped it, CMD13 checked"
        );
        // And the next read is clean.
        let mut card = SdCard::init(host).unwrap();
        let mut one = [0u8; BLOCK];
        card.read(0, &mut one).unwrap();
    }

    #[test]
    fn a_range_that_failed_is_read_block_by_block_from_then_on() {
        let mut fake = FakeCard::new(true, true);
        fake.capable = true;
        fake.slow_block = Some(4);
        let mut card = SdCard::init(fake).unwrap();
        let mut buf = vec![0u8; 4 * BLOCK];
        card.read_many(2, &mut buf).unwrap();
        assert_eq!(card.weak_ranges(), 1);
        // Again, and an overlapping range: no CMD18 at all now.
        let before = card.host.log.len();
        card.read_many(2, &mut buf).unwrap();
        card.read_many(4, &mut buf).unwrap();
        assert!(card.host.log[before..].iter().all(|&(c, _)| c != 18));
        assert!(buf[..BLOCK].iter().all(|&b| b == 4));
        // Elsewhere: multi-block as usual.
        let before = card.host.log.len();
        let mut far = vec![0u8; 2 * BLOCK];
        card.read_many(0, &mut far).unwrap();
        assert!(card.host.log[before..].iter().any(|&(c, _)| c == 18));
        assert_eq!(card.weak_ranges(), 1);
    }

    #[test]
    fn an_unreadable_block_is_still_an_error() {
        let mut fake = FakeCard::new(true, true);
        fake.capable = true;
        fake.bad_block = Some(3);
        let mut card = SdCard::init(fake).unwrap();
        let mut buf = vec![0u8; 4 * BLOCK];
        assert_eq!(
            card.read_many(2, &mut buf),
            Err(Error::Host(HostError::Timeout))
        );
        assert_eq!(card.read_fallbacks(), 0);
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
        assert_eq!(cmds, [0, 8, 55, 41, 55, 41, 55, 41, 2, 3, 9, 7, 17]); // no CMD16
        assert_eq!(host.log.last(), Some(&(17, 3)));
    }

    /// Blocks past the card's end are refused before any command, by all
    /// four entry points and the block-device interface; an SDSC card's
    /// byte address would otherwise wrap onto block 0 (2^23 blocks of 512
    /// bytes is 2^32).
    #[test]
    fn blocks_past_the_end_are_refused_without_a_command() {
        for sdhc in [false, true] {
            let mut card = SdCard::init(FakeCard::new(sdhc, sdhc)).unwrap();
            let end = card.blocks();
            let before = card.host().log.len();
            let mut one = [0u8; BLOCK];
            let mut two = [0u8; 2 * BLOCK];
            for lba in [end, end + 1, 1 << 23, u32::MAX] {
                assert_eq!(card.read(lba, &mut one), Err(Error::OutOfRange));
                assert_eq!(card.write(lba, &one), Err(Error::OutOfRange));
                assert_eq!(card.read_many(lba, &mut two), Err(Error::OutOfRange));
                assert_eq!(card.write_many(lba, &two), Err(Error::OutOfRange));
            }
            // A run that starts on the card and ends past it.
            assert_eq!(card.read_many(end - 1, &mut two), Err(Error::OutOfRange));
            assert_eq!(card.write_many(end - 1, &two), Err(Error::OutOfRange));
            assert_eq!(
                BlockDevice::read_blocks(&mut card, end, &mut two),
                Err(BlockError::OutOfRange)
            );
            assert_eq!(
                BlockDevice::write_blocks(&mut card, end - 1, &two),
                Err(BlockError::OutOfRange)
            );
            assert_eq!(card.last_error(), None, "not a card failure");
            assert_eq!(card.host().log.len(), before, "no command sent");
            // The last block itself is fine (the fake holds 8 blocks; the
            // SDSC fake's capacity is exactly that).
            if !sdhc {
                card.read(end - 1, &mut one).unwrap();
                assert_eq!(one[0], (end - 1) as u8);
            }
        }
    }

    /// A card whose capacity couldn't be read refuses every transfer.
    #[test]
    fn an_unknown_capacity_refuses_transfers() {
        let mut card = SdCard::init(FakeCard::new(true, true)).unwrap();
        card.blocks = 0;
        let mut one = [0u8; BLOCK];
        assert_eq!(card.read(0, &mut one), Err(Error::OutOfRange));
        assert_eq!(card.write(0, &one), Err(Error::OutOfRange));
    }

    #[test]
    fn capacity_comes_from_the_csd() {
        let card = SdCard::init(FakeCard::new(true, true)).unwrap();
        assert_eq!(card.blocks(), 1024);
        assert!(card.into_inner().log.contains(&(9, 0xBEEF << 16)));
        let card = SdCard::init(FakeCard::new(false, false)).unwrap();
        assert_eq!(card.blocks(), 8);
        // A CSD v2 with C_SIZE 7623: 7,806,976 blocks, which is where the
        // owner's card's partition ends (start 8192 + 7,798,784 blocks).
        let csd = [0x400E_0032, 0x5B59_0000, 0x1DC7_7F80, 0x0A40_0000];
        assert_eq!(csd_blocks(csd), 8192 + 7_798_784);
        assert_eq!(csd_blocks([0xC000_0000, 0, 0, 0]), 0, "unknown structure");
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
        assert_eq!(card.reads(), 1, "one read command for three blocks");
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
        assert_eq!(card.reads(), 2, "one read command per block");
        let host = card.into_inner();
        assert_eq!(&host.log[host.log.len() - 2..], [(17, 5), (17, 6)]);
    }

    #[test]
    fn a_failed_init_gives_the_host_back_for_a_later_card() {
        let mut fake = FakeCard::new(true, true);
        fake.busy_rounds = u32::MAX; // no card answering properly
        let (e, mut host) = SdCard::try_init(fake).err().unwrap();
        assert_eq!(e, Error::InitTimeout);
        // A card arrives: the same host initialises it.
        host.busy_rounds = 1;
        let card = SdCard::try_init(host).ok().unwrap();
        assert!(card.is_high_capacity());
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
