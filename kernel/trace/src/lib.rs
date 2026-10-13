//! The trace ring (docs/design-brief.md): an always-on, fixed-size record of
//! what the kernel did lately (thread switches, faults, calls, timers), so a
//! hang, a stutter or a slow path can be read after the fact instead of
//! reproduced under a debugger.
//!
//! - **Fixed size, no allocation:** `N` records of 16 bytes; the oldest is
//!   overwritten. All zero when empty, so a ring in a static costs nothing
//!   in the image (`.bss`).
//! - **Cheap:** a record is four word stores and a counter update; no locks
//!   (single core).
//! - **Self-checking:** every record carries its sequence number, stored
//!   last. A reader keeps only records whose number is the one expected in
//!   that slot, so a record cut short (an interrupt that recorded in the
//!   middle of another, or a crash mid-write) or overwritten is dropped,
//!   never misread; how many were lost is known exactly.
//! - **Portable:** the ring is plain words, so it can be copied out as it is
//!   (to the PC over USB) and decoded there with the same code.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

/// Words per record: sequence number, time, kind and argument, data.
pub const RECORD_WORDS: usize = 4;

/// What happened.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
#[repr(u8)]
pub enum Kind {
    /// A thread switch: `a` the thread leaving, `b` the one running.
    Switch = 1,
    /// A page fault resolved: `a` how (an outcome code), `b` the address.
    Fault = 2,
    /// A service call: `a` the server's slot, `b` the call depth.
    Call = 3,
    /// A call returned: `a` the caller's slot, `b` the result.
    Return = 4,
    /// A server restarted: `a` its slot.
    Restart = 5,
    /// A process started: `a` its slot, `b` its entry.
    Spawn = 6,
    /// A process ended: `a` its slot, `b` its exit code.
    Exit = 7,
    /// A timer alarm: `b` the deadline.
    Alarm = 8,
    /// Anything else worth marking: `a` and `b` are the caller's.
    Mark = 9,
}

impl Kind {
    fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            1 => Kind::Switch,
            2 => Kind::Fault,
            3 => Kind::Call,
            4 => Kind::Return,
            5 => Kind::Restart,
            6 => Kind::Spawn,
            7 => Kind::Exit,
            8 => Kind::Alarm,
            9 => Kind::Mark,
            _ => return None,
        })
    }

    /// A short name for printing.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Kind::Switch => "switch",
            Kind::Fault => "fault",
            Kind::Call => "call",
            Kind::Return => "return",
            Kind::Restart => "restart",
            Kind::Spawn => "spawn",
            Kind::Exit => "exit",
            Kind::Alarm => "alarm",
            Kind::Mark => "mark",
        }
    }
}

/// One recorded event.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub struct Event {
    /// Its number: 1 for the first ever recorded.
    pub seq: u32,
    /// When (the OS timer, 3.25 MHz).
    pub time: u32,
    /// What.
    pub kind: Kind,
    /// First argument (24 bits).
    pub a: u32,
    /// Second argument.
    pub b: u32,
}

/// A ring of `N` records.
pub struct Ring<const N: usize> {
    /// `N` records of [`RECORD_WORDS`] words.
    words: [[u32; RECORD_WORDS]; N],
    /// Records written so far (the newest one's number).
    total: u32,
}

impl<const N: usize> Default for Ring<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Ring<N> {
    /// An empty ring (all zero).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            words: [[0; RECORD_WORDS]; N],
            total: 0,
        }
    }

    /// Records an event. The sequence number goes in last, so a record cut
    /// short is never taken for a whole one.
    pub fn record(&mut self, time: u32, kind: Kind, a: u32, b: u32) {
        if N == 0 {
            return;
        }
        let seq = self.total.wrapping_add(1).max(1);
        let slot = &mut self.words[(seq - 1) as usize % N];
        slot[0] = 0;
        slot[1] = time;
        slot[2] = u32::from(kind as u8) | (a & 0x00FF_FFFF) << 8;
        slot[3] = b;
        slot[0] = seq;
        self.total = seq;
    }

    /// Records written so far.
    #[must_use]
    pub fn total(&self) -> u32 {
        self.total
    }

    /// The ring's words, as they lie (to copy out whole).
    #[must_use]
    pub fn words(&self) -> &[[u32; RECORD_WORDS]; N] {
        &self.words
    }

    /// The events still in the ring, oldest first.
    pub fn events(&self) -> impl Iterator<Item = Event> + '_ {
        decode(&self.words, self.total)
    }
}

/// The intact events in a ring's `words` (copied out, or in place) whose
/// newest record is number `total`, oldest first. Records that are torn,
/// overwritten or garbled are skipped.
pub fn decode<const N: usize>(
    words: &[[u32; RECORD_WORDS]; N],
    total: u32,
) -> impl Iterator<Item = Event> + '_ {
    decode_slots(words, total)
}

/// [`decode`] for records of any count (a ring copied out, or one in a
/// reserved region).
pub fn decode_slots(words: &[[u32; RECORD_WORDS]], total: u32) -> impl Iterator<Item = Event> + '_ {
    let n = words.len();
    let first = if n == 0 {
        total.wrapping_add(1)
    } else {
        total.saturating_sub(n as u32 - 1).max(1)
    };
    (first..=total).filter_map(move |seq| {
        let w = words[(seq - 1) as usize % n];
        if w[0] != seq {
            return None;
        }
        Some(Event {
            seq,
            time: w[1],
            kind: Kind::from_u8(w[2] as u8)?,
            a: w[2] >> 8,
            b: w[3],
        })
    })
}

/// Words in a [`Persistent`] ring's header: a magic number, the records
/// written so far, the magic's complement, and the ring's record count.
pub const HEADER_WORDS: usize = 4;
const MAGIC: u32 = u32::from_le_bytes(*b"RPTR");

/// The same ring kept in memory the caller hands over: a reserved region
/// that outlives the program, so the resident loader can send it to the PC
/// after the program returned, crashed or hung (`cargo xtask trace`). The
/// loader starts it when it starts a program; the program carries it on.
pub struct Persistent<'a> {
    header: &'a mut [u32; HEADER_WORDS],
    records: &'a mut [[u32; RECORD_WORDS]],
}

impl<'a> Persistent<'a> {
    /// Starts an empty ring in `area`: the header, then as many records as
    /// fit. `None` if not even one does.
    pub fn start(area: &'a mut [u32]) -> Option<Self> {
        let (header, rest) = area.split_first_chunk_mut::<HEADER_WORDS>()?;
        let records = rest.as_chunks_mut::<RECORD_WORDS>().0;
        if records.is_empty() {
            return None;
        }
        records.fill([0; RECORD_WORDS]);
        *header = [MAGIC, 0, !MAGIC, records.len() as u32];
        Some(Self { header, records })
    }

    /// Carries on the ring already in `area` (the loader started it), if
    /// its header is intact and matches the area; else `None`.
    pub fn attach(area: &'a mut [u32]) -> Option<Self> {
        let (header, rest) = area.split_first_chunk_mut::<HEADER_WORDS>()?;
        let records = rest.as_chunks_mut::<RECORD_WORDS>().0;
        let n = records.len();
        (header[0] == MAGIC && header[2] == !MAGIC && header[3] as usize == n && n > 0)
            .then_some(Self { header, records })
    }

    /// Carries on the ring in `area` if there is one (the loader started
    /// it), else starts one: a program that runs with or without the loader.
    pub fn resume(area: &'a mut [u32]) -> Option<Self> {
        let n = area.len().saturating_sub(HEADER_WORDS) / RECORD_WORDS;
        let ring = area.len() >= HEADER_WORDS
            && area[0] == MAGIC
            && area[2] == !MAGIC
            && area[3] as usize == n;
        if ring {
            Self::attach(area)
        } else {
            Self::start(area)
        }
    }

    /// Records an event (as [`Ring::record`]).
    pub fn record(&mut self, time: u32, kind: Kind, a: u32, b: u32) {
        let seq = self.header[1].wrapping_add(1).max(1);
        let n = self.records.len();
        let slot = &mut self.records[(seq - 1) as usize % n];
        slot[0] = 0;
        slot[1] = time;
        slot[2] = u32::from(kind as u8) | (a & 0x00FF_FFFF) << 8;
        slot[3] = b;
        slot[0] = seq;
        self.header[1] = seq;
    }

    /// Records written so far.
    #[must_use]
    pub fn total(&self) -> u32 {
        self.header[1]
    }

    /// Records the ring holds.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.records.len()
    }

    /// The events still in the ring, oldest first.
    pub fn events(&self) -> impl Iterator<Item = Event> + '_ {
        decode_slots(self.records, self.header[1])
    }
}

/// A [`Persistent`] ring's words as copied out (header, then records):
/// its total and its intact events, oldest first; `None` if the header
/// isn't a ring's. Bounded by `words`: a garbled count can't read past it.
#[must_use]
pub fn decode_words(words: &[u32]) -> Option<(u32, impl Iterator<Item = Event> + '_)> {
    let (header, rest) = words.split_first_chunk::<HEADER_WORDS>()?;
    let records = rest.as_chunks::<RECORD_WORDS>().0;
    let n = (header[3] as usize).min(records.len());
    if header[0] != MAGIC || header[2] != !MAGIC || n == 0 {
        return None;
    }
    Some((header[1], decode_slots(&records[..n], header[1])))
}

/// Records no longer in a ring of `n` whose newest is number `total`.
#[must_use]
pub fn lost(n: usize, total: u32) -> u32 {
    total.saturating_sub(n as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_persistent_ring_survives_being_handed_over() {
        let mut area = [0xAAAA_AAAAu32; HEADER_WORDS + 3 * RECORD_WORDS + 2];
        // Not a ring yet: nothing to carry on.
        assert!(Persistent::attach(&mut area).is_none());
        {
            let mut r = Persistent::start(&mut area).unwrap();
            assert_eq!((r.capacity(), r.total()), (3, 0));
            r.record(10, Kind::Mark, 1, 2);
        }
        // The program carries on the ring the loader started.
        {
            let mut r = Persistent::resume(&mut area).unwrap();
            assert_eq!(r.total(), 1, "carried on, not restarted");
            for i in 0..4u32 {
                r.record(20 + i, Kind::Switch, i, 0);
            }
            assert_eq!(r.total(), 5);
        }
        // The PC decodes the words as copied out: the newest three.
        let (total, events) = decode_words(&area).unwrap();
        let e: Vec<Event> = events.collect();
        assert_eq!(total, 5);
        assert_eq!(e.iter().map(|e| e.seq).collect::<Vec<_>>(), [3, 4, 5]);
        assert_eq!((e[2].time, e[2].kind, e[2].a), (23, Kind::Switch, 3));
    }

    #[test]
    fn garbled_dumps_are_refused_or_bounded() {
        let mut area = [0u32; HEADER_WORDS + 4 * RECORD_WORDS];
        Persistent::start(&mut area)
            .unwrap()
            .record(1, Kind::Mark, 0, 0);
        // A count larger than the words: bounded to what's there.
        let mut big = area;
        big[3] = 1_000_000;
        assert_eq!(decode_words(&big).unwrap().1.count(), 1);
        // A broken magic, a short dump, a torn record.
        let mut bad = area;
        bad[2] = 0;
        assert!(decode_words(&bad).is_none());
        assert!(decode_words(&area[..3]).is_none());
        let mut torn = area;
        torn[HEADER_WORDS] = 0;
        assert_eq!(decode_words(&torn).unwrap().1.count(), 0);
        // Every truncation decodes without panicking.
        for n in 0..area.len() {
            if let Some((_, e)) = decode_words(&area[..n]) {
                let _ = e.count();
            }
        }
    }

    #[test]
    fn keeps_the_newest_in_order() {
        let mut r = Ring::<4>::new();
        assert_eq!(r.events().count(), 0);
        for i in 0..6u32 {
            r.record(100 + i, Kind::Switch, i, i * 10);
        }
        let e: Vec<Event> = r.events().collect();
        assert_eq!(e.len(), 4);
        assert_eq!(e[0].seq, 3);
        assert_eq!(e[3].seq, 6);
        assert_eq!((e[3].time, e[3].a, e[3].b), (105, 5, 50));
        assert!(e.windows(2).all(|p| p[0].seq + 1 == p[1].seq));
        assert_eq!(lost(4, r.total()), 2);
    }

    #[test]
    fn arguments_keep_24_bits_and_kinds_round_trip() {
        let mut r = Ring::<16>::new();
        let kinds = [
            Kind::Switch,
            Kind::Fault,
            Kind::Call,
            Kind::Return,
            Kind::Restart,
            Kind::Spawn,
            Kind::Exit,
            Kind::Alarm,
            Kind::Mark,
        ];
        for (i, k) in kinds.iter().enumerate() {
            r.record(i as u32, *k, 0xABCD_EF01, 0xFFFF_FFFF);
        }
        let e: Vec<Event> = r.events().collect();
        assert_eq!(e.iter().map(|e| e.kind).collect::<Vec<_>>(), kinds);
        assert!(e.iter().all(|e| e.a == 0x00CD_EF01 && e.b == u32::MAX));
        assert_eq!(Kind::Call.name(), "call");
    }

    #[test]
    fn torn_and_garbled_records_are_dropped_not_misread() {
        let mut r = Ring::<8>::new();
        for i in 0..8u32 {
            r.record(i, Kind::Mark, i, i);
        }
        // A copy taken while record 5 was half written (its number cleared
        // first), and record 7 overwritten by garbage.
        let mut words = *r.words();
        words[4][0] = 0;
        words[6] = [0xDEAD_BEEF; 4];
        let e: Vec<u32> = decode(&words, r.total()).map(|e| e.seq).collect();
        assert_eq!(e, [1, 2, 3, 4, 6, 8]);
        // An unknown kind is dropped too.
        words[0][2] = 0xFF;
        assert_eq!(decode(&words, r.total()).count(), 5);
        // A count larger than what was written finds nothing extra.
        assert_eq!(decode(&words, 1000).count(), 0);
    }

    #[test]
    fn empty_and_zero_size_rings_are_safe() {
        let mut z = Ring::<0>::new();
        z.record(1, Kind::Mark, 1, 1);
        assert_eq!(z.events().count(), 0);
        let r = Ring::<4>::new();
        assert_eq!(r.events().count(), 0);
        assert_eq!(lost(4, 0), 0);
    }
}
