//! The card log on the X51v (the plan is [`crate::cardlog`]): its state
//! and the stream in SDRAM, and the sector writes a program makes through
//! the SD controller and the CF socket the loader set up
//! ([`crate::cardwrite`]).
//!
//! Every program gets it with no code of its own: [`after_write`] runs in
//! [`crate::devices::log_write`]. The loader, which owns both cards, syncs
//! with its own drivers instead ([`sync_with`]).

use rp_hal_pxa27x::rt;

use crate::cardlog::{CF, Io, SD, SECTOR, State, due, sync};
use crate::cardwrite::{Sd, cf_write, sd_write};
use crate::devices::{self, LOADER_BASE, LOADER_SIZE};

/// The card log's state and its sector buffer: one page right after the
/// trace ring (boot/link.x), left by the loader for the program.
pub const CARDLOG_BASE: usize = 0xA851_1000;
/// Where the sector being written is built (a whole cache line apart from
/// the state).
const SECTOR_AT: usize = CARDLOG_BASE + 0x200;
/// The stream past the run log: what a long run logs after the 64 KiB run
/// log is full still reaches the card (the file holds 256 KiB).
pub const CARDLOG_MORE: usize = 0xA852_0000;
/// Size of [`CARDLOG_MORE`]'s reservation.
pub const MORE_SIZE: usize = 192 * 1024;

/// The state in SDRAM.
#[allow(clippy::mut_from_ref)]
fn state() -> &'static mut State {
    // SAFETY: CARDLOG_BASE's page is SDRAM reserved for the card log
    // (boot/link.x, docs/hw/memory-map.md), identity-mapped and word
    // aligned; single core, used from the main thread only (an interrupt
    // that logs finds `busy` set and leaves its text in the run log).
    unsafe { &mut *(CARDLOG_BASE as *mut State) }
}

#[allow(clippy::mut_from_ref)]
fn sector() -> &'static mut [u8; SECTOR] {
    // SAFETY: the page's second half, as in `state`; only `sync` uses it.
    unsafe { &mut *(SECTOR_AT as *mut [u8; SECTOR]) }
}

/// Whether the code running is the resident loader's (it owns the cards
/// and syncs with its own drivers; a program never does this).
fn in_loader() -> bool {
    let here = after_write as *const () as usize;
    (LOADER_BASE..LOADER_BASE + LOADER_SIZE).contains(&here)
}

/// The state, when a card log is on for this run.
#[must_use]
pub fn current() -> Option<&'static mut State> {
    let s = state();
    s.valid().then_some(s)
}

/// Starts a card log for the run about to start (the loader, after
/// reserving the files and before `devices::log_start`).
pub fn begin(s: State) {
    *state() = s;
    // SAFETY: as in `more_len`.
    unsafe {
        extra(0).write_volatile(0);
        extra(1).write_volatile(0);
    }
}

/// No card log for the run about to start (the loader, when no file could
/// be reserved), or the previous one's ended.
pub fn end() {
    state().magic = 0;
}

/// The stream past the run log: bytes kept, bytes dropped. Apart from
/// [`State`], so that appending (a note, mid-sync) never touches what a
/// sync holds.
const EXTRA: usize = CARDLOG_BASE + 0x100;

fn extra(i: usize) -> *mut u32 {
    (EXTRA + 4 * i) as *mut u32
}

fn more_len() -> usize {
    // SAFETY: a word of the card log's page (see `state`).
    unsafe { extra(0).read_volatile() as usize }
}

/// Bytes that fitted in neither the run log nor the stream past it.
#[must_use]
pub fn dropped() -> u32 {
    // SAFETY: as in `more_len`.
    unsafe { extra(1).read_volatile() }
}

/// Stream bytes so far: the run log's text, then [`CARDLOG_MORE`]'s.
fn len() -> usize {
    devices::log_len() + more_len()
}

/// What didn't fit in the run log: kept for the card past it, up to
/// [`MORE_SIZE`]; the rest is counted as dropped.
pub(crate) fn more(data: &[u8]) {
    if current().is_none() {
        return;
    }
    let at = more_len();
    let n = data.len().min(MORE_SIZE - at);
    // SAFETY: CARDLOG_MORE..+MORE_SIZE is SDRAM reserved for the card log
    // (boot/link.x); `at + n <= MORE_SIZE`. The two words are the page's
    // (see `state`).
    unsafe {
        core::ptr::copy_nonoverlapping(data.as_ptr(), (CARDLOG_MORE + at) as *mut u8, n);
        extra(0).write_volatile((at + n) as u32);
        extra(1).write_volatile(dropped().saturating_add((data.len() - n) as u32));
    }
}

/// Stream bytes logged so far (the loader's checks).
#[must_use]
pub fn stream_len() -> usize {
    len()
}

/// Copies stream bytes `at..` into `out` (all below [`stream_len`]).
pub fn copy(at: usize, out: &mut [u8]) {
    let text = devices::log_text();
    let mut i = 0;
    if at < text.len() {
        let n = (text.len() - at).min(out.len());
        out[..n].copy_from_slice(&text[at..at + n]);
        i = n;
    }
    if i < out.len() {
        let from = at + i - text.len();
        // SAFETY: stream bytes past the run log are in CARDLOG_MORE, below
        // `more` (callers copy only below the stream's length).
        let src = unsafe {
            core::slice::from_raw_parts((CARDLOG_MORE + from) as *const u8, out.len() - i)
        };
        out[i..].copy_from_slice(src);
    }
}

/// After [`crate::devices::log_write`] appended `text`: in a program, the
/// line goes to a card once it ends.
pub(crate) fn after_write(text: &[u8]) {
    if in_loader() {
        return;
    }
    let Some(s) = current() else { return };
    if due(s, text, len()) {
        let mut io = Program {
            sd: Sd {
                rca: s.sd.rca as u16,
                high_capacity: s.sd.flags & 1 != 0,
                wide: s.sd.flags & 2 != 0,
                blocks: s.sd.blocks,
            },
            cf_sectors: s.cf_sectors,
        };
        sync(s, &mut io, sector());
    }
}

/// The loader's sync: `io`'s stream is the same, its cards its own.
pub fn sync_with(io: &mut dyn LoaderCards) {
    let Some(s) = current() else { return };
    sync(s, &mut Loader(io), sector());
}

/// The loader, at its next start: drops the files that aren't the run's
/// any more ([`crate::cardlog::keep_after_reset`]); the units dropped.
pub fn keep_after_reset(read: &mut dyn FnMut(usize, u32, &mut [u8; SECTOR]) -> bool) -> u32 {
    let Some(s) = current() else { return 0 };
    crate::cardlog::keep_after_reset(s, &Stream, read)
}

/// The stream alone (no cards), for the checks above.
struct Stream;

impl Io for Stream {
    fn logged(&self) -> usize {
        len()
    }
    fn copy(&self, at: usize, out: &mut [u8]) {
        copy(at, out);
    }
    fn note(&mut self, _text: &[u8]) {}
    fn free(&mut self, _unit: usize) -> bool {
        false
    }
    fn write(&mut self, _unit: usize, _lba: u32, _sector: &[u8; SECTOR]) -> bool {
        false
    }
    fn ticks(&self) -> u32 {
        0
    }
}

/// The loader's cards, for [`sync_with`].
pub trait LoaderCards {
    /// Writes one sector to `unit` ([`SD`], [`CF`]); whether it took it.
    fn write(&mut self, unit: usize, lba: u32, sector: &[u8; SECTOR]) -> bool;
}

struct Loader<'a>(&'a mut dyn LoaderCards);

impl Io for Loader<'_> {
    fn logged(&self) -> usize {
        len()
    }
    fn copy(&self, at: usize, out: &mut [u8]) {
        copy(at, out);
    }
    fn note(&mut self, text: &[u8]) {
        devices::log_append(text);
    }
    fn free(&mut self, _unit: usize) -> bool {
        true
    }
    fn write(&mut self, unit: usize, lba: u32, sector: &[u8; SECTOR]) -> bool {
        self.0.write(unit, lba, sector)
    }
    fn ticks(&self) -> u32 {
        rt::ticks()
    }
}

/// A program's sector writes, through the controllers the loader set up:
/// the cards as the loader identified them.
struct Program {
    sd: Sd,
    cf_sectors: u32,
}

impl Io for Program {
    fn logged(&self) -> usize {
        len()
    }
    fn copy(&self, at: usize, out: &mut [u8]) {
        copy(at, out);
    }
    fn note(&mut self, text: &[u8]) {
        devices::log_append(text);
    }
    fn free(&mut self, unit: usize) -> bool {
        match unit {
            SD => rt::mmc_if_untaken().is_some(),
            _ => devices::cf_if_untaken().is_some(),
        }
    }
    fn write(&mut self, unit: usize, lba: u32, sector: &[u8; SECTOR]) -> bool {
        match unit {
            SD => {
                let Some(mut regs) = rt::mmc_if_untaken() else {
                    return false;
                };
                sd_write(&mut regs, self.sd, lba, sector)
            }
            CF => {
                let Some(mut common) = devices::cf_if_untaken() else {
                    return false;
                };
                cf_write(&mut common, self.cf_sectors, lba, sector)
            }
            _ => false,
        }
    }
    fn ticks(&self) -> u32 {
        rt::ticks()
    }
}
