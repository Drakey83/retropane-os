//! The card log on the X51v: the card opened as the loader opens it, the
//! run's file reserved, one log for the whole image (single core: written
//! only from the main thread, like the SDRAM run log).

use core::fmt::Write;

use rp_drv_cpld::Cpld;
use rp_drv_pxa_mmc::PxaMmc;
use rp_drv_sdcard::{SdCard, mbr_partitions};
use rp_fat::{AnyVolume, Error};
use rp_hal_api::{BlockDevice, BlockError};
use rp_hal_pxa27x::{Mmio, rt};
use rp_hal_x51v::devices::{self, KeptRunLog};

use super::{CardLog, FILE_BYTES, FOLDER, next_number, path};

/// The card by the CPU's own transfers (no DMA): one sector a line needs
/// none, and the SD DMA channel stays free for the image (rp-gpubench's
/// upload tests borrow it).
type Sd = SdCard<PxaMmc<Mmio>>;

/// The card while the file is found and reserved.
struct Fed<'a>(&'a mut Sd);

impl BlockDevice for Fed<'_> {
    fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
        rt::check_in();
        self.0.read_blocks(lba, buf)
    }
    fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
        rt::check_in();
        self.0.write_blocks(lba, buf)
    }
}

/// The card while the log runs; each sector write timed.
struct Card {
    sd: Sd,
    /// The slowest sector write, in OS timer ticks (3.25 MHz).
    worst: u32,
}

impl BlockDevice for Card {
    fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
        rt::check_in();
        self.sd.read_blocks(lba, buf)
    }
    fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
        rt::check_in();
        let t0 = rt::ticks();
        let r = self.sd.write_blocks(lba, buf);
        self.worst = self.worst.max(rt::ticks().wrapping_sub(t0));
        r
    }
}

/// The log, once [`start`] has reserved its file.
static mut LOG: Option<CardLog<Card>> = None;
/// The file system's scratch while the file is reserved: 4 KiB zero-fills
/// the 256 KiB file in 64 commands and reads the FAT 8 sectors a command.
static mut SCRATCH: [u8; 4 * 1024] = [0; 4 * 1024];

fn with_log<R>(f: impl FnOnce(&mut CardLog<Card>) -> R) -> Option<R> {
    // SAFETY: single core; the log is written only from the main thread
    // (no interrupt handler logs), so this is the only reference.
    let log = unsafe { &mut *core::ptr::addr_of_mut!(LOG) };
    log.as_mut().map(f)
}

/// Opens the card (powered through the CPLD, as the loader does) and
/// reserves this run's file, `rplog/<image>-NNNN.txt`; from here
/// [`CardRunLog`] puts every line on the card. Returns the number, or why
/// the card log is off (the run goes on with the SDRAM log alone).
///
/// Takes the CPLD and the MMC controller (not its DMA channel): an image
/// that uses the card itself can't also keep a card log.
#[inline(never)]
pub fn start(image: &str) -> Result<u32, &'static str> {
    if with_log(|_| ()).is_some() {
        return Err("already on");
    }
    let (bus, table) = devices::take_cpld().ok_or("the CPLD is taken")?;
    let (mut cpld, _) = Cpld::adopt(bus, table);
    let mmc = rt::take_mmc().ok_or("the MMC controller is taken")?;
    cpld.set_backlight_enable(true);
    cpld.set_sd_power(true);
    rt::delay_us(10_000);
    let mut sd = SdCard::try_init(PxaMmc::new(mmc)).map_err(|_| "no card")?;
    // SAFETY: single core, main thread only; `start` runs once (checked
    // above), before any card log write.
    let scratch = unsafe { &mut *core::ptr::addr_of_mut!(SCRATCH) };
    let (number, at) = {
        let mut block = [0u8; rp_drv_sdcard::BLOCK];
        sd.read_blocks(0, &mut block).map_err(|_| "card read")?;
        let base = mbr_partitions(&block)
            .and_then(|p| p[0])
            .map_or(0, |p| p.start);
        let mut vol = AnyVolume::mount(Fed(&mut sd), base).map_err(|_| "not FAT32 or exFAT")?;
        let mut highest = 0u32;
        match vol.list(FOLDER, &mut |name, dir, _| {
            if !dir {
                highest = highest.max(next_number(image, core::iter::once(name)) - 1);
            }
        }) {
            Ok(()) | Err(Error::NotFound) => {}
            Err(e) => return Err(e.name()),
        }
        let number = highest + 1;
        let mut name = [0u8; 64];
        let n = path(image, number, &mut name);
        let name = core::str::from_utf8(&name[..n]).map_err(|_| "bad name")?;
        let at = vol
            .reserve(name, FILE_BYTES, scratch)
            .map_err(Error::name)?;
        (number, at)
    };
    let log = CardLog::new(Card { sd, worst: 0 }, at);
    // SAFETY: as in `with_log`; nothing else holds a reference now.
    unsafe { *core::ptr::addr_of_mut!(LOG) = Some(log) };
    Ok(number)
}

/// Appends to the card log, if it's on.
#[inline(never)]
pub fn write(text: &[u8]) {
    let _ = with_log(|l| l.write(text));
}

/// Ends the card log with its closing line (writes, refusals, the slowest
/// write, bytes dropped): a file without it is a run that didn't come back.
#[inline(never)]
pub fn stop() {
    let _ = with_log(|l| {
        // The OS timer counts 3.25 MHz.
        let worst_us = l.dev.worst * 4 / 13;
        let mut line = Line([0; 160], 0);
        let _ = write!(
            line,
            "[card log closed: {} sector writes before this line, {} refused, slowest {} us, {} bytes dropped]\n",
            l.writes,
            l.failed,
            worst_us,
            l.dropped
        );
        l.write(&line.0[..line.1]);
        l.flush();
    });
    // SAFETY: as in `with_log`; the reference above has ended.
    unsafe { *core::ptr::addr_of_mut!(LOG) = None };
}

/// A short line built on the stack.
struct Line([u8; 160], usize);

impl Write for Line {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let n = s.len().min(self.0.len() - self.1);
        self.0[self.1..self.1 + n].copy_from_slice(&s.as_bytes()[..n]);
        self.1 += n;
        Ok(())
    }
}

/// The run log ([`KeptRunLog`]: SDRAM and the SRAM copy) and the card log.
pub struct CardRunLog;

impl Write for CardRunLog {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let _ = KeptRunLog.write_str(s);
        write(s.as_bytes());
        Ok(())
    }
}

/// The card log alone (a crash message the loader's crash path already
/// puts in the SDRAM log).
pub struct CardOnly;

impl Write for CardOnly {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        write(s.as_bytes());
        Ok(())
    }
}
