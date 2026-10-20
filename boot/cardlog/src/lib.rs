//! A test image's run log on the SD card, written as the run goes.
//!
//! The loader keeps a run's log in SDRAM and the PXA's internal SRAM, and a
//! watchdog reset into Dell's factory prompt clears both (2026-10-20, run Z:
//! `docs/hw/memory-map.md`, quality audit C23): the runs that most needed
//! their log lost it. The card keeps what is written to it.
//!
//! Before the test, [`start`] reserves the run's own file,
//! `rplog/<image>-NNNN.txt` (the next number: a re-run never overwrites a
//! log not yet fetched), zero-filled in one run of sectors
//! ([`rp_fat::Reserved`], written crash-safe like every card write). From
//! then on every line goes into it **in place**, one sector write a line
//! ([`CardLog`]): the directory and the FAT never change during the run, so
//! a reset at any moment leaves the file system whole and the file holding
//! every line finished before it (the text ends at the first zero byte).
//! [`stop`] writes a closing line: a log without one is a run that didn't
//! come back. `cargo xtask cardlog` fetches it through the loader.
//!
//! Only the run's own folder is written (the card is shared:
//! `docs/hw/sdcard.md`).

#![cfg_attr(not(test), no_std)]

use rp_fat::{Reserved, SECTOR};
use rp_hal_api::BlockDevice;

/// The folder every run log goes in.
pub const FOLDER: &str = "rplog";
/// Each run's file: 4x the SDRAM run log (64 KiB), for long soaks.
pub const FILE_BYTES: usize = 256 * 1024;

/// A reserved file written line by line, in place.
pub struct CardLog<D> {
    dev: D,
    at: Reserved,
    /// The sector being filled, from the file's start.
    sector: u32,
    buf: [u8; SECTOR],
    used: usize,
    /// `buf` holds text not yet on the card.
    dirty: bool,
    /// Sector writes, and those the card refused.
    pub writes: u32,
    /// Sector writes the card refused (each line's sector is written again
    /// with the next line, so a refused write is retried once more).
    pub failed: u32,
    /// Bytes that didn't fit (the file was full).
    pub dropped: u32,
}

impl<D: BlockDevice> CardLog<D> {
    /// Writes into `at` (zero-filled) on `dev`.
    pub fn new(dev: D, at: Reserved) -> Self {
        Self {
            dev,
            at,
            sector: 0,
            buf: [0; SECTOR],
            used: 0,
            dirty: false,
            writes: 0,
            failed: 0,
            dropped: 0,
        }
    }

    /// Whether every sector is used.
    #[must_use]
    pub fn full(&self) -> bool {
        self.sector >= self.at.sectors
    }

    /// Appends `text`; the sector is written whenever a line ends in it
    /// and when it fills.
    pub fn write(&mut self, mut text: &[u8]) {
        while !text.is_empty() {
            if self.full() {
                self.dropped = self.dropped.saturating_add(text.len() as u32);
                return;
            }
            let n = (SECTOR - self.used).min(text.len());
            self.buf[self.used..self.used + n].copy_from_slice(&text[..n]);
            let line_end = text[..n].contains(&b'\n');
            self.used += n;
            self.dirty = true;
            text = &text[n..];
            if self.used == SECTOR {
                self.flush();
                self.sector += 1;
                self.used = 0;
                self.buf = [0; SECTOR];
            } else if line_end {
                self.flush();
            }
        }
    }

    /// Writes the sector being filled if it holds text not yet written.
    pub fn flush(&mut self) {
        if !self.dirty || self.full() {
            return;
        }
        self.writes += 1;
        if self.dev.write_blocks(self.at.lba + self.sector, &self.buf).is_ok() {
            self.dirty = false;
        } else {
            self.failed += 1;
        }
    }

    /// The device back.
    pub fn into_inner(self) -> D {
        self.dev
    }
}

/// The number after the highest `<image>-NNNN.txt` among `names` (1 when
/// there is none).
#[must_use]
pub fn next_number<'a>(image: &str, names: impl Iterator<Item = &'a [u8]>) -> u32 {
    names
        .filter_map(|n| {
            let rest = n.strip_prefix(image.as_bytes())?.strip_prefix(b"-")?;
            let digits = rest.strip_suffix(b".txt")?;
            if digits.is_empty() || digits.len() > 9 || !digits.iter().all(u8::is_ascii_digit) {
                return None;
            }
            Some(
                digits
                    .iter()
                    .fold(0u32, |v, &d| v * 10 + u32::from(d - b'0')),
            )
        })
        .max()
        .map_or(1, |m| m.saturating_add(1))
}

/// `rplog/<image>-NNNN.txt` into `out`; its length.
pub fn path(image: &str, number: u32, out: &mut [u8; 64]) -> usize {
    let mut n = 0;
    let mut put = |b: &[u8]| {
        let k = b.len().min(out.len() - n);
        out[n..n + k].copy_from_slice(&b[..k]);
        n += k;
    };
    put(FOLDER.as_bytes());
    put(b"/");
    put(image.as_bytes());
    put(b"-");
    let mut digits = [b'0'; 10];
    let mut v = number;
    let mut len = 0;
    while v > 0 || len < 4 {
        digits[9 - len] = b'0' + (v % 10) as u8;
        v /= 10;
        len += 1;
    }
    put(&digits[10 - len..]);
    put(b".txt");
    n
}

#[cfg(target_os = "none")]
mod device;
#[cfg(target_os = "none")]
pub use device::{CardOnly, CardRunLog, start, stop, write};

#[cfg(test)]
mod tests;
