//! The card log: a run's log on the SD and CompactFlash cards as the run
//! goes, for every program the loader starts (quality audit C24, owner's
//! vote B, 2026-10-20).
//!
//! The run log lives in SDRAM, and a watchdog reset into Dell's factory
//! prompt clears it (runs G2 and Z lost theirs). Before starting a program
//! the loader, which has the file systems, reserves the run's own file on
//! each card it has, `rplog/<image>-NNNN.txt` (zero-filled, one run of
//! sectors, the same number on both), and leaves a [`State`] in SDRAM
//! saying where they are. From then on every line the program logs is
//! copied **from the run log itself** into the file, one sector write at a
//! line's end: sector `k` of the file always holds bytes `k * 512..` of the
//! run log (the *stream*: the run log's text, then what didn't fit in it),
//! so the card can't say anything the run log doesn't, and a reset at any
//! moment leaves the file system whole and every finished line on a card.
//!
//! Two programs can't drive one card at once. A program that takes the SD
//! controller (an SD test) moves the log to the CF card from the next line
//! on, a program that takes the CF socket keeps it on SD, and a program
//! that takes both pauses it; each with a line saying so. Each card's file
//! then holds its own part at the same offsets, zeros elsewhere: joining
//! them byte for byte gives the whole log ([`join`]). When the program
//! comes back the loader catches up whatever a pause held back and writes
//! a closing line.
//!
//! This module is the plan, host-tested; the device side (the drivers,
//! [`crate::devices::log_write`]'s hook) is `crate::cardsink`.

/// The folder every card log goes in (the card is shared: nothing else is
/// written, docs/hw/sd.md).
pub const FOLDER: &str = "rplog";
/// Each run's file: 4x the SDRAM run log, for long soaks.
pub const FILE_BYTES: usize = 256 * 1024;
/// Sector size on both cards.
pub const SECTOR: usize = 512;
/// The longest image name a file may carry (`name` command).
pub const NAME_MAX: usize = 24;
/// Files kept per image on each card: older ones are deleted when a run
/// reserves its own, so the folder can't fill the card (32 x 256 KiB =
/// 8 MiB per image per card).
pub const KEEP: u32 = 32;
/// The state's first word.
pub const MAGIC: u32 = u32::from_le_bytes(*b"RPCL");

/// The units, in order of preference.
pub const SD: usize = 0;
/// The CompactFlash card.
pub const CF: usize = 1;
/// [`State::noted`]'s bit for a unit whose last sector was sent and not
/// yet waited for (quality audit C59, the owner's choice (a), 2026-10-22):
/// a line costs the sector's transfer, not the card's programming, which
/// is waited for before the card's next command ([`settle`]). In a word
/// both the installed loader and a newer program agree on (the loader
/// starts it at 0).
pub const PENDING: u32 = 0x100;

/// Each unit's name in the notes.
const UNIT_NAMES: [&[u8]; 2] = [b"SD card", b"CF card"];

/// A reserved file: its first sector and length (no file: 0 sectors).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct File {
    /// First sector on the card.
    pub lba: u32,
    /// Sectors reserved.
    pub sectors: u32,
}

/// The SD card as the loader left it identified and selected, for a
/// program to write sectors without identifying it again.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SdCard {
    /// Relative card address.
    pub rca: u32,
    /// Bit 0: block addressing (SDHC/SDXC); bit 1: the 4-bit bus.
    pub flags: u32,
    /// Capacity in sectors.
    pub blocks: u32,
}

/// The card log's state, in SDRAM between the loader and the program
/// (`devices::CARDLOG_BASE`). Plain words: both sides are built from this
/// file, and the loader checks [`MAGIC`] and its complement.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct State {
    /// [`MAGIC`] while a card log is on.
    pub magic: u32,
    /// `!MAGIC`.
    pub check: u32,
    /// The run's number (both files carry it).
    pub number: u32,
    /// Each unit's file ([`SD`], [`CF`]).
    pub files: [File; 2],
    /// The SD card's identity.
    pub sd: SdCard,
    /// The CF card's capacity in sectors.
    pub cf_sectors: u32,
    /// The unit last written + 1 (0: none yet).
    pub on: u32,
    /// Units given up on: bit `u` taken by the program, bit `u + 8` refused
    /// a write.
    pub off: u32,
    /// Stream bytes on the cards.
    pub synced: u32,
    /// Sector writes, and those refused.
    pub writes: u32,
    /// Sector writes a card refused.
    pub failed: u32,
    /// The slowest sector write, in OS timer ticks (3.25 MHz).
    pub worst: u32,
    /// A sync is under way (an interrupt that logs meanwhile leaves its
    /// text in the run log; the sync in progress takes it).
    pub busy: u32,
    /// Bit 0: a pause was noted; bit 1: the file was found full; bit
    /// `8 + u`: unit `u`'s last sector sent, its programming not yet
    /// waited for ([`PENDING`]).
    pub noted: u32,
    /// Bit per unit: a sector was written to its file.
    pub used: u32,
    /// Each unit's first sector written (valid with its `used` bit).
    pub first: [u32; 2],
}

impl State {
    /// A fresh state for run `number` with these files (0 sectors: none).
    #[must_use]
    pub const fn new(number: u32, files: [File; 2], sd: SdCard, cf_sectors: u32) -> Self {
        Self {
            magic: MAGIC,
            check: !MAGIC,
            number,
            files,
            sd,
            cf_sectors,
            on: 0,
            off: 0,
            synced: 0,
            writes: 0,
            failed: 0,
            worst: 0,
            busy: 0,
            noted: 0,
            used: 0,
            first: [0; 2],
        }
    }

    /// Whether this is a card log (magic intact, at least one file).
    #[must_use]
    pub fn valid(&self) -> bool {
        self.magic == MAGIC
            && self.check == !MAGIC
            && self.files.iter().any(|f| f.sectors > 0)
            && self.on <= 2
    }

    /// The program is back (or gone): the loader has both cards again, each
    /// identified afresh, so none stays given up on, and a pause may be
    /// noted again.
    pub fn reclaim(&mut self) {
        self.off = 0;
        self.busy = 0;
        self.noted &= !1;
    }

    /// The stream bytes the files can hold (the smaller file's size; both
    /// are reserved alike).
    #[must_use]
    pub fn capacity(&self) -> u32 {
        self.files
            .iter()
            .filter(|f| f.sectors > 0)
            .map(|f| f.sectors.saturating_mul(SECTOR as u32))
            .min()
            .unwrap_or(0)
    }
}

/// What a sync works with: the stream and the cards.
pub trait Io {
    /// Stream bytes logged so far.
    fn logged(&self) -> usize;
    /// Copies stream bytes `at..at + out.len()` (all below [`Io::logged`]).
    fn copy(&self, at: usize, out: &mut [u8]);
    /// Appends a note to the stream (the run log too).
    fn note(&mut self, text: &[u8]);
    /// Whether the program has left `unit` alone (the loader: always).
    fn free(&mut self, unit: usize) -> bool;
    /// Writes one sector to `unit`; whether the card took it. It may
    /// return before the card has programmed it: [`Io::settle`] waits.
    fn write(&mut self, unit: usize, lba: u32, sector: &[u8; SECTOR]) -> bool;
    /// Waits for `unit` to finish programming the last sector written;
    /// whether it did without an error. Writers that wait in
    /// [`Io::write`] have nothing to wait for.
    fn settle(&mut self, _unit: usize) -> bool {
        true
    }
    /// Whether [`Io::write`] returns before the card has programmed the
    /// sector (a program's writes); a writer that waits (the loader's
    /// drivers) leaves nothing pending, and nothing for the next writer to
    /// wait for that it couldn't see.
    fn defers(&self) -> bool {
        false
    }
    /// The OS timer (3.25 MHz ticks).
    fn ticks(&self) -> u32;
}

/// The unit to write next, giving up on units the program has taken;
/// `None` when neither can be used.
fn pick(state: &mut State, io: &mut dyn Io) -> Option<usize> {
    for unit in [SD, CF] {
        if state.files[unit].sectors > 0 && state.off & (0x101 << unit) == 0 && !io.free(unit) {
            state.off |= 1 << unit;
        }
    }
    let usable = |u: usize| state.files[u].sectors > 0 && state.off & (0x101 << u) == 0;
    // Stay where the log is, else SD first.
    let current = (state.on as usize).checked_sub(1);
    current
        .filter(|&u| usable(u))
        .or_else(|| [SD, CF].into_iter().find(|&u| usable(u)))
}

/// Why `unit` was given up on, for the notes.
fn why(state: &State, unit: usize) -> &'static [u8] {
    if state.off & (0x100 << unit) != 0 {
        b" refused a write"
    } else {
        b" was taken by the program"
    }
}

/// Waits for every unit with a sector still programming. A unit that
/// fails it is given up on (as a refused write) and the stream rewinds to
/// that sector's start, so the next sync puts it on the other card. A
/// unit's last write is always the sector holding the last byte synced.
// One copy for every caller (ADR-0005 rule 8: size).
#[inline(never)]
pub fn settle(state: &mut State, io: &mut dyn Io) {
    for unit in [SD, CF] {
        if state.noted & (PENDING << unit) == 0 {
            continue;
        }
        state.noted &= !(PENDING << unit);
        let t0 = io.ticks();
        let ok = io.settle(unit);
        state.worst = state.worst.max(io.ticks().wrapping_sub(t0));
        if !ok {
            state.failed = state.failed.wrapping_add(1);
            state.off |= 0x100 << unit;
            let k = state.synced.saturating_sub(1) / SECTOR as u32;
            state.synced = k * SECTOR as u32;
        }
    }
}

/// Puts the stream not yet on a card onto one: the sector holding the
/// first byte not written, then each after it, until the cards hold the
/// whole stream (or neither can take more). Notes a move to the other
/// card, a pause and a full file in the stream itself, once each.
pub fn sync(state: &mut State, io: &mut dyn Io, sector: &mut [u8; SECTOR]) {
    if !state.valid() || state.busy != 0 {
        return;
    }
    state.busy = 1;
    loop {
        let len = io.logged();
        if state.synced as usize >= len {
            break;
        }
        if state.synced >= state.capacity() {
            if state.noted & 2 == 0 {
                state.noted |= 2;
                io.note(b"[card log full: the rest is in the run log only]\n");
            }
            break;
        }
        let was = (state.on as usize).checked_sub(1);
        let Some(unit) = pick(state, io) else {
            if state.noted & 1 == 0 {
                state.noted |= 1;
                io.note(b"[card log paused: ");
                for u in [SD, CF] {
                    if state.files[u].sectors > 0 {
                        io.note(b"the ");
                        io.note(UNIT_NAMES[u]);
                        io.note(why(state, u));
                        io.note(b"; ");
                    }
                }
                io.note(b"the loader adds the rest when the program returns]\n");
            }
            break;
        };
        state.on = unit as u32 + 1;
        if let Some(old) = was.filter(|&o| o != unit) {
            // Said in the stream, so the move is in both the run log and
            // the new card's part; the loop picks the note up.
            io.note(b"[card log: the ");
            io.note(UNIT_NAMES[old]);
            io.note(why(state, old));
            io.note(b"; continuing on the ");
            io.note(UNIT_NAMES[unit]);
            io.note(b"]\n");
            continue;
        }
        // The card's next command waits for its last sector.
        settle(state, io);
        if state.off & (0x100 << unit) != 0 {
            continue;
        }
        let k = state.synced as usize / SECTOR;
        let from = k * SECTOR;
        let n = (len - from).min(SECTOR);
        io.copy(from, &mut sector[..n]);
        sector[n..].fill(0);
        let t0 = io.ticks();
        let ok = io.write(unit, state.files[unit].lba + k as u32, sector);
        state.worst = state.worst.max(io.ticks().wrapping_sub(t0));
        state.writes = state.writes.wrapping_add(1);
        if ok {
            if io.defers() {
                state.noted |= PENDING << unit;
            }
            state.synced = (from + n) as u32;
            if state.used & 1 << unit == 0 {
                state.used |= 1 << unit;
                state.first[unit] = k as u32;
            }
        } else {
            state.failed = state.failed.wrapping_add(1);
            state.off |= 0x100 << unit;
        }
    }
    state.busy = 0;
}

/// After a reset (the loader, at its next start, before writing a card
/// again): the slot may hold another card now, whose own files could sit
/// where this run's was. Each unit's file is kept only if the first sector
/// the run wrote there still holds what it wrote (its text up to the first
/// zero, at least one byte, equal to the stream's bytes there); a file the
/// run never wrote has nothing to tell it by and isn't needed (nothing is
/// missing from it). The others are dropped from `state`, never written.
/// `read` reads one sector of a unit. Returns the units dropped as no
/// longer the run's (not counting unused ones).
pub fn keep_after_reset(
    state: &mut State,
    io: &dyn Io,
    read: &mut dyn FnMut(usize, u32, &mut [u8; SECTOR]) -> bool,
) -> u32 {
    let mut gone = 0;
    for unit in [SD, CF] {
        let f = state.files[unit];
        if f.sectors == 0 {
            continue;
        }
        let ours = state.used & 1 << unit != 0 && {
            let k = state.first[unit];
            let mut on_card = [0u8; SECTOR];
            let mut stream = [0u8; SECTOR];
            let at = k as usize * SECTOR;
            let n = io.logged().saturating_sub(at).min(SECTOR);
            io.copy(at, &mut stream[..n]);
            k < f.sectors && read(unit, f.lba + k, &mut on_card) && {
                let held = on_card.iter().position(|&b| b == 0).unwrap_or(SECTOR);
                held > 0 && held <= n && on_card[..held] == stream[..held]
            }
        };
        if !ours {
            if state.used & 1 << unit != 0 {
                gone |= 1 << unit;
            }
            state.files[unit].sectors = 0;
            if state.on == unit as u32 + 1 {
                state.on = 0;
            }
        }
    }
    gone
}

/// Whether a line just logged should go to the card now: it ended a line,
/// or a sector's worth is waiting.
#[must_use]
pub fn due(state: &State, text: &[u8], len: usize) -> bool {
    text.contains(&b'\n') || len >= state.synced as usize + SECTOR
}

/// The two halves of a run's card log (SD's file, CF's; either may be
/// missing) joined into the log: each byte from whichever file holds it,
/// up to the first byte neither does.
pub fn join<'a>(sd: Option<&'a [u8]>, cf: Option<&'a [u8]>) -> impl Iterator<Item = u8> + 'a {
    let at = |f: Option<&[u8]>, i: usize| f.and_then(|f| f.get(i)).copied().unwrap_or(0);
    (0..)
        .map(move |i| at(sd, i) | at(cf, i))
        .take_while(|&b| b != 0)
}

/// Whether `name` may name a card log: 1 to [`NAME_MAX`] of `a-z`, `0-9`,
/// `-` and `_` (it becomes part of a path on the card: nothing else).
#[must_use]
pub fn name_ok(name: &[u8]) -> bool {
    (1..=NAME_MAX).contains(&name.len())
        && name
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-' || *b == b'_')
}

/// The number in `<image>-NNNN.txt`, if `file` is one of `image`'s.
#[must_use]
pub fn number_of(image: &[u8], file: &[u8]) -> Option<u32> {
    let rest = file.strip_prefix(image)?.strip_prefix(b"-")?;
    let digits = rest
        .strip_suffix(b".txt")
        .or_else(|| rest.strip_suffix(b".TXT"))?;
    if digits.is_empty() || digits.len() > 9 || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some(
        digits
            .iter()
            .fold(0u32, |v, &d| v * 10 + u32::from(d - b'0')),
    )
}

/// `rplog/<image>-NNNN.txt` into `out`; its length.
pub fn path(image: &[u8], number: u32, out: &mut [u8; 64]) -> usize {
    let mut n = 0;
    let mut put = |b: &[u8]| {
        let k = b.len().min(out.len() - n);
        out[n..n + k].copy_from_slice(&b[..k]);
        n += k;
    };
    put(FOLDER.as_bytes());
    put(b"/");
    put(image);
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

#[cfg(test)]
mod tests;
