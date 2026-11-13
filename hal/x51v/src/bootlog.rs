//! The loader's own start-up log on the cards (reminder R193; the sheet is
//! `docs/research/sheets/r193-loader-startup-log.md`).
//!
//! A run's card log ([`crate::cardlog`]) covers what an image prints; the
//! loader's own start-up (the board as it found it, then what the PC does
//! on USB) stayed on screen and in RAM. On a device whose USB never works
//! (the X50v, 2026-11-13) that was nothing to read. At its first start
//! after HaRET the loader now reserves a run slot under the name [`NAME`],
//! exactly as for a run (so `cargo xtask cardlog loader` reads it back),
//! and appends its lines here.
//!
//! Lines are **queued in RAM** and written a sector at a time when the
//! loader calls [`BootLog::flush`] (when USB has been quiet, or the queue
//! is half full): a card's programming between two USB control requests
//! could change the very enumeration being logged. Sector `k` of the file
//! holds bytes `k * 512..` of the log, the same on both cards; the sector
//! still filling is rewritten with zeros after its text, so a reader stops
//! at the first zero ([`crate::cardlog::join`]). Nothing here touches the
//! file system: the reservation is the loader's, and after it only sectors
//! inside the reserved file are written (ADR-0013 rule 2, R22).

use crate::cardlog::{File, SECTOR};
use core::fmt::{self, Write};

/// The image name the log's slot carries (`rplog/index.txt`).
pub const NAME: &[u8] = b"loader";
/// Bytes queued between flushes; past it a line is dropped and counted.
pub const QUEUE: usize = 4 * 1024;
/// How long the log stays open at most (milliseconds): the first few
/// minutes (the X50v's 30 bus resets came within them).
pub const LIFE_MS: u32 = 5 * 60 * 1000;
/// USB quiet for this long (milliseconds) lets the queue go to the cards.
pub const QUIET_MS: u32 = 100;
/// The heartbeat's period (milliseconds): the port registers and counters
/// even when nothing changes.
pub const HEARTBEAT_MS: u32 = 10_000;
/// Events closer together than this (milliseconds) share one line.
pub const EVENT_MS: u32 = 50;
/// The memory a log needs from its owner ([`BootLog::new`]): the sector
/// being filled, then the queue. Borrowed, not owned, so the loader keeps
/// it in its SDRAM scratch and not in its zeroed RAM (ADR-0005's budget).
pub const BUF: usize = SECTOR + QUEUE;

/// The log: the files it writes, the queue, and what happened so far.
pub struct BootLog<'a> {
    files: [File; 2],
    /// Bit per unit given up on (a card refused a write).
    off: u32,
    /// Bytes on the cards' stream so far.
    at: usize,
    /// The sector holding byte `at` (its bytes before `at % SECTOR`).
    sector: &'a mut [u8; SECTOR],
    queue: &'a mut [u8],
    queued: usize,
    dropped: u32,
    writes: u32,
    failed: u32,
    closed: bool,
}

impl<'a> BootLog<'a> {
    /// A log writing `files` (a unit with no sectors is skipped), in `buf`'s
    /// first [`BUF`] bytes; `None` if `buf` is shorter.
    #[must_use]
    pub fn new(files: [File; 2], buf: &'a mut [u8]) -> Option<Self> {
        let (sector, rest) = buf.split_first_chunk_mut::<SECTOR>()?;
        let queue = rest.get_mut(..QUEUE)?;
        sector.fill(0);
        Some(Self {
            files,
            off: 0,
            at: 0,
            sector,
            queue,
            queued: 0,
            dropped: 0,
            writes: 0,
            failed: 0,
            closed: false,
        })
    }

    /// Whether lines still go anywhere: not closed, and a unit has a file
    /// it hasn't given up on.
    #[must_use]
    pub fn open(&self) -> bool {
        !self.closed && (0..2).any(|u| self.files[u].sectors > 0 && self.off & (1 << u) == 0)
    }

    /// Queues `text`. What doesn't fit is dropped and counted (said in the
    /// closing line: never silently). Zero bytes become `.` (a reader stops
    /// at a zero). Whether it all fitted.
    pub fn push(&mut self, text: &[u8]) -> bool {
        if self.closed {
            return false;
        }
        let n = text.len().min(QUEUE - self.queued);
        for (d, &s) in self.queue[self.queued..self.queued + n]
            .iter_mut()
            .zip(text)
        {
            *d = if s == 0 { b'.' } else { s };
        }
        self.queued += n;
        let lost = text.len() - n;
        self.dropped = self.dropped.saturating_add(lost as u32);
        lost == 0
    }

    /// Bytes waiting for [`BootLog::flush`].
    #[must_use]
    pub const fn queued(&self) -> usize {
        self.queued
    }

    /// The queue is half full: flush now, quiet or not.
    #[must_use]
    pub const fn wants_flush(&self) -> bool {
        self.queued >= QUEUE / 2
    }

    /// The stream's room: the smallest file still written.
    fn capacity(&self) -> usize {
        (0..2)
            .filter(|&u| self.files[u].sectors > 0 && self.off & (1 << u) == 0)
            .map(|u| self.files[u].sectors as usize * SECTOR)
            .min()
            .unwrap_or(0)
    }

    /// The queue onto the cards through `write` (unit, sector, data):
    /// every sector it completes, then the one still filling.
    pub fn flush(&mut self, write: &mut dyn FnMut(usize, u32, &[u8; SECTOR]) -> bool) {
        let mut i = 0;
        while i < self.queued {
            let room = self.capacity().saturating_sub(self.at);
            if room == 0 {
                self.dropped = self.dropped.saturating_add((self.queued - i) as u32);
                break;
            }
            let off = self.at % SECTOR;
            let n = (SECTOR - off).min(self.queued - i).min(room);
            self.sector[off..off + n].copy_from_slice(&self.queue[i..i + n]);
            self.at += n;
            i += n;
            let full = self.at.is_multiple_of(SECTOR);
            if full || i == self.queued {
                self.write_sector((self.at - 1) / SECTOR, write);
            }
            if full {
                self.sector.fill(0);
            }
        }
        self.queued = 0;
    }

    fn write_sector(&mut self, k: usize, write: &mut dyn FnMut(usize, u32, &[u8; SECTOR]) -> bool) {
        for u in 0..2 {
            let f = self.files[u];
            if (f.sectors as usize) <= k || self.off & (1 << u) != 0 {
                continue;
            }
            self.writes = self.writes.wrapping_add(1);
            if !write(u, f.lba + k as u32, self.sector) {
                self.failed = self.failed.wrapping_add(1);
                self.off |= 1 << u;
            }
        }
    }

    /// The closing line (why, and the write counts), flushed; nothing is
    /// written after it.
    pub fn close(&mut self, why: &str, write: &mut dyn FnMut(usize, u32, &[u8; SECTOR]) -> bool) {
        if self.closed {
            return;
        }
        // The closing line must fit: what's queued goes first.
        self.flush(write);
        let mut line = Text::<160>::new();
        let _ = writeln!(
            line,
            "[start-up log closed: {why}; {} sector writes, {} refused, {} bytes dropped]",
            self.writes, self.failed, self.dropped
        );
        self.push(line.bytes());
        self.flush(write);
        self.closed = true;
    }

    /// Sector writes so far, those refused, bytes dropped.
    #[must_use]
    pub const fn counts(&self) -> (u32, u32, u32) {
        (self.writes, self.failed, self.dropped)
    }
}

/// A line built on the stack (cut short, never failing, past `N`).
pub struct Text<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> Text<N> {
    /// Empty.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
        }
    }

    /// The text so far.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

impl<const N: usize> Default for Text<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Write for Text<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let k = s.len().min(N - self.len);
        self.buf[self.len..self.len + k].copy_from_slice(&s.as_bytes()[..k]);
        self.len += k;
        Ok(())
    }
}

/// `UP2OCR`'s fields by name (PXA27x Developer's Manual 12.6.3: bit 0
/// CPVEN, 1 CPVPE, 2 DPPDE, 3 DMPDE, 4 DPPUE, 5 DMPUE, 6 DPPUBE, 7 DMPUBE,
/// 8 EXSP, 9 EXSUS, 16 HXS, 17 HXOE, 26-24 SEOS).
pub fn up2ocr(out: &mut dyn Write, v: u32) -> fmt::Result {
    const BITS: [(u32, &str); 12] = [
        (17, "HXOE"),
        (16, "HXS"),
        (9, "EXSUS"),
        (8, "EXSP"),
        (7, "DMPUBE"),
        (6, "DPPUBE"),
        (5, "DMPUE"),
        (4, "DPPUE"),
        (3, "DMPDE"),
        (2, "DPPDE"),
        (1, "CPVPE"),
        (0, "CPVEN"),
    ];
    write!(out, "SEOS {}", (v >> 24) & 7)?;
    for (bit, name) in BITS {
        if v & (1 << bit) != 0 {
            write!(out, " {name}")?;
        }
    }
    if v & (1 << 17) == 0 {
        out.write_str(" (HXOE off: the transceiver drives nothing)")?;
    }
    if v & (1 << 16) != 0 {
        out.write_str(" (HXS: port 2 is the host controller's)")?;
    }
    Ok(())
}

/// GPIO `n`'s level, direction (true: output) and alternate function from
/// the level, direction and `GAFR` words (`GPLR0-3`, `GPDR0-3`,
/// `GAFR0_L` .. `GAFR3_U`); `None` past GPIO 120.
#[must_use]
pub fn pin(level: &[u32; 4], dir: &[u32; 4], gafr: &[u32; 8], n: u32) -> Option<(bool, bool, u32)> {
    if n > 120 {
        return None;
    }
    let (w, b) = ((n / 32) as usize, n % 32);
    let (a, s) = ((n / 16) as usize, 2 * (n % 16));
    Some((
        level[w] >> b & 1 != 0,
        dir[w] >> b & 1 != 0,
        gafr[a] >> s & 3,
    ))
}

/// One pin's line: `GPIO14 level 0 in AF0`.
pub fn pin_line(
    out: &mut dyn Write,
    level: &[u32; 4],
    dir: &[u32; 4],
    gafr: &[u32; 8],
    n: u32,
) -> fmt::Result {
    match pin(level, dir, gafr, n) {
        Some((l, o, af)) => write!(
            out,
            "GPIO{n} level {} {} AF{af}",
            u8::from(l),
            if o { "out" } else { "in" }
        ),
        None => write!(out, "GPIO{n} none"),
    }
}

/// CPLD `0x1C`'s bit 3, the lead for the X50v's USB (Dell's X50v
/// `usbfndrv` sets it when its cable check says attached): `on`/`off`.
#[must_use]
pub const fn bit3(v: u16) -> &'static str {
    if v & 8 != 0 { "on" } else { "off" }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    /// The log's memory, dirty (it must not leak into the cards).
    fn buf() -> Vec<u8> {
        std::vec![0xEE; BUF]
    }

    fn files(sd: u32, cf: u32) -> [File; 2] {
        [
            File {
                lba: 100,
                sectors: sd,
            },
            File {
                lba: 9000,
                sectors: cf,
            },
        ]
    }

    /// A card per unit: what each sector holds, by LBA.
    struct Cards {
        sectors: [std::collections::BTreeMap<u32, [u8; SECTOR]>; 2],
        refuse: [bool; 2],
        calls: Vec<(usize, u32)>,
    }

    impl Cards {
        fn new() -> Self {
            Self {
                sectors: Default::default(),
                refuse: [false; 2],
                calls: Vec::new(),
            }
        }
        fn write(&mut self, u: usize, lba: u32, s: &[u8; SECTOR]) -> bool {
            self.calls.push((u, lba));
            if self.refuse[u] {
                return false;
            }
            self.sectors[u].insert(lba, *s);
            true
        }
        /// The unit's file text, sector by sector from `lba`, to the first
        /// zero.
        fn text(&self, u: usize, lba: u32) -> Vec<u8> {
            let mut out = Vec::new();
            let mut k = lba;
            while let Some(s) = self.sectors[u].get(&k) {
                out.extend_from_slice(s);
                k += 1;
            }
            out.into_iter().take_while(|&b| b != 0).collect()
        }
    }

    fn flush(log: &mut BootLog, c: &mut Cards) {
        log.flush(&mut |u, lba, s| c.write(u, lba, s));
    }

    #[test]
    fn lines_reach_both_cards_alike_across_sector_ends() {
        let mut m = buf();
        let mut log = BootLog::new(files(512, 512), &mut m).unwrap();
        let mut c = Cards::new();
        let mut want = Vec::new();
        // Lines of awkward lengths, flushed at odd moments: every sector
        // end crossed, and one that ends exactly on a sector's end.
        for i in 0..200u32 {
            let line = std::format!("line {i} {}\n", "x".repeat((i % 37) as usize));
            want.extend_from_slice(line.as_bytes());
            assert!(log.push(line.as_bytes()));
            if i % 7 == 0 {
                flush(&mut log, &mut c);
            }
        }
        let pad = SECTOR - want.len() % SECTOR;
        let exact = "y".repeat(pad);
        want.extend_from_slice(exact.as_bytes());
        log.push(exact.as_bytes());
        flush(&mut log, &mut c);
        assert_eq!(want.len() % SECTOR, 0);
        assert_eq!(c.text(0, 100), want);
        assert_eq!(c.text(1, 9000), want);
        // And the next line starts the next sector cleanly.
        log.push(b"after\n");
        flush(&mut log, &mut c);
        want.extend_from_slice(b"after\n");
        assert_eq!(c.text(0, 100), want);
        assert_eq!(
            crate::cardlog::join(Some(&c.text(0, 100)), Some(&c.text(1, 9000))).collect::<Vec<_>>(),
            want
        );
    }

    #[test]
    fn nothing_is_written_before_a_flush_and_a_flush_with_nothing_writes_nothing() {
        let mut m = buf();
        let mut log = BootLog::new(files(4, 4), &mut m).unwrap();
        let mut c = Cards::new();
        log.push(b"held\n");
        assert!(c.calls.is_empty());
        flush(&mut log, &mut c);
        assert_eq!(c.calls.len(), 2);
        flush(&mut log, &mut c);
        assert_eq!(c.calls.len(), 2);
    }

    #[test]
    fn the_file_full_drops_and_counts_never_past_its_end() {
        let mut m = buf();
        let mut log = BootLog::new(files(2, 2), &mut m).unwrap();
        let mut c = Cards::new();
        let big = [b'a'; 3 * SECTOR];
        assert!(log.push(&big));
        flush(&mut log, &mut c);
        assert!(
            c.calls
                .iter()
                .all(|&(u, lba)| if u == 0 { lba < 102 } else { lba < 9002 })
        );
        assert_eq!(log.counts().2, SECTOR as u32);
        assert_eq!(c.text(0, 100).len(), 2 * SECTOR);
    }

    #[test]
    fn the_queue_full_drops_and_counts_and_the_close_says_so() {
        let mut m = buf();
        let mut log = BootLog::new(files(512, 0), &mut m).unwrap();
        let mut c = Cards::new();
        assert!(log.push(&[b'q'; QUEUE - 3]));
        assert!(log.wants_flush());
        assert!(!log.push(b"12345\n"));
        assert_eq!(log.counts().2, 3);
        log.close("test", &mut |u, lba, s| c.write(u, lba, s));
        let t = std::string::String::from_utf8(c.text(0, 100)).unwrap();
        assert!(t.ends_with("3 bytes dropped]\n"), "{t}");
        assert!(t.contains("start-up log closed: test"));
        // Nothing after closing.
        let n = c.calls.len();
        assert!(!log.push(b"late\n"));
        flush(&mut log, &mut c);
        assert_eq!(c.calls.len(), n);
        assert!(!log.open());
    }

    #[test]
    fn a_card_that_refuses_is_given_up_the_other_keeps_going() {
        let mut m = buf();
        let mut log = BootLog::new(files(8, 8), &mut m).unwrap();
        let mut c = Cards::new();
        c.refuse[0] = true;
        log.push(b"one\n");
        flush(&mut log, &mut c);
        log.push(b"two\n");
        flush(&mut log, &mut c);
        assert_eq!(c.calls.iter().filter(|&&(u, _)| u == 0).count(), 1);
        assert_eq!(c.text(1, 9000), b"one\ntwo\n");
        assert_eq!(log.counts().1, 1);
        assert!(log.open());
        c.refuse[1] = true;
        log.push(b"three\n");
        flush(&mut log, &mut c);
        assert!(!log.open());
    }

    #[test]
    fn no_file_means_closed_from_the_start_and_zeros_never_reach_a_card() {
        assert!(!BootLog::new(files(0, 0), &mut buf()).unwrap().open());
        // Too little memory: no log.
        assert!(BootLog::new(files(4, 4), &mut [0u8; BUF - 1]).is_none());
        let mut m = buf();
        let mut log = BootLog::new(files(0, 4), &mut m).unwrap();
        let mut c = Cards::new();
        log.push(b"a\0b\n");
        flush(&mut log, &mut c);
        assert_eq!(c.text(1, 9000), b"a.b\n");
        assert!(c.sectors[0].is_empty());
    }

    #[test]
    fn up2ocr_names_its_fields_and_flags_the_two_suspects() {
        let mut t = Text::<200>::new();
        up2ocr(&mut t, 0x0002_0000).unwrap();
        assert_eq!(t.bytes(), b"SEOS 0 HXOE");
        let mut t = Text::<200>::new();
        up2ocr(&mut t, 0x0001_0020).unwrap();
        assert_eq!(
            t.bytes(),
            &b"SEOS 0 HXS DMPUE (HXOE off: the transceiver drives nothing) (HXS: port 2 is the host controller's)"[..]
        );
        let mut t = Text::<200>::new();
        up2ocr(&mut t, 0x0700_03FF).unwrap();
        assert!(
            std::str::from_utf8(t.bytes())
                .unwrap()
                .starts_with("SEOS 7 EXSUS EXSP DMPUBE")
        );
    }

    #[test]
    fn pins_come_from_their_words_at_every_bank_edge() {
        let mut level = [0u32; 4];
        let mut dir = [0u32; 4];
        let mut gafr = [0u32; 8];
        level[0] = 1 << 14;
        dir[0] = 1 << 2;
        gafr[0] = 2 << 28; // GPIO14: AF2
        gafr[1] = 3 << 30; // GPIO31
        level[3] = 1 << 24; // GPIO120
        gafr[7] = 1 << 16; // GPIO120: word 7, bits 17-16
        assert_eq!(pin(&level, &dir, &gafr, 14), Some((true, false, 2)));
        assert_eq!(pin(&level, &dir, &gafr, 2), Some((false, true, 0)));
        assert_eq!(pin(&level, &dir, &gafr, 31), Some((false, false, 3)));
        assert_eq!(pin(&level, &dir, &gafr, 32), Some((false, false, 0)));
        assert_eq!(pin(&level, &dir, &gafr, 120), Some((true, false, 1)));
        assert_eq!(pin(&level, &dir, &gafr, 121), None);
        let mut t = Text::<64>::new();
        pin_line(&mut t, &level, &dir, &gafr, 14).unwrap();
        assert_eq!(t.bytes(), b"GPIO14 level 1 in AF2");
    }

    #[test]
    fn bit3_and_a_text_cut_short_never_fails() {
        assert_eq!(bit3(0x000E), "on");
        assert_eq!(bit3(0x0006), "off");
        let mut t = Text::<4>::new();
        assert!(write!(t, "abcdef").is_ok());
        assert_eq!(t.bytes(), b"abcd");
    }
}
