//! The XScale's performance monitor: the cycle counter `CCNT` and the event
//! counters `PMN0`-`PMN3`, each counting one event chosen in `EVTSEL` (the
//! performance gate's C8).
//!
//! **Claims until the X51v shows them** ([Intel], the PXA27x / XScale core
//! manuals): `PMNC` CP14 c0, c1 (bit 0 enable, bit 1 reset the event
//! counters, bit 2 reset `CCNT`; the cycle counter's use since 2026-10-15
//! confirms bits 0 and 2), `EVTSEL` c8, c1 (one byte an event counter,
//! counter 0 in bits 7-0), `PMN0`-`PMN2` c0-c2, c2, `PMN3` c3, c2, and the
//! event numbers of [`Event`]. The probe's known-answer checks (`rp-probe`
//! `pmu_*`: an instruction and a branch count for a loop of known length,
//! data-cache misses for walks that do and don't fit the cache) are what
//! make them facts. The emulator fork defines the registers but every event
//! counter reads 0 (not modellable: cache and TLB behaviour), and puts
//! `PMN3` at c2, c3 where the manual says c3, c2: until a device run
//! settles it, [`read`] reads `PMN0`-`PMN2` only.

/// An event an event counter can count ([Intel] numbers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Event {
    /// An instruction fetch missed the I-cache.
    IcacheMiss = 0x00,
    /// Cycles the I-cache couldn't deliver an instruction.
    IcacheStall = 0x01,
    /// Cycles stalled on a data dependency.
    DataStall = 0x02,
    /// An I-TLB miss.
    ItlbMiss = 0x03,
    /// A D-TLB miss.
    DtlbMiss = 0x04,
    /// A branch executed.
    Branch = 0x05,
    /// A branch mispredicted.
    BranchMiss = 0x06,
    /// An instruction executed.
    Instruction = 0x07,
    /// Cycles stalled with the data cache's buffers full.
    BufferStallCycles = 0x08,
    /// Stalls with the data cache's buffers full (occurrences).
    BufferStall = 0x09,
    /// A data cache access.
    DcacheAccess = 0x0A,
    /// A data cache miss.
    DcacheMiss = 0x0B,
    /// A data cache line written back.
    DcacheWriteback = 0x0C,
    /// Software changed the PC.
    PcChanged = 0x0D,
}

/// `EVTSEL` for `events` (counter `k`'s event in byte `k`).
#[must_use]
pub const fn evtsel(events: [Event; 4]) -> u32 {
    (events[0] as u32)
        | (events[1] as u32) << 8
        | (events[2] as u32) << 16
        | (events[3] as u32) << 24
}

/// What was counted between [`start`] and [`read`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Core cycles.
    pub cycles: u32,
    /// Events counted by `PMN0`-`PMN2` (the order given to [`start`]).
    pub events: [u32; 3],
}

/// The event sets a profile rotates through, one a span (three counters
/// each; R81 item 1, the owner's "measure first"): the stall cycles by
/// cause, then what causes them. Linux `perf stat` multiplexes more events
/// than counters the same way; the spans of a set are a sample of the
/// whole, so a profile runs over a repeated workload (the touch replay).
pub const SETS: [[Event; 3]; 2] = [
    [
        Event::DataStall,
        Event::BufferStallCycles,
        Event::IcacheStall,
    ],
    [Event::DcacheMiss, Event::DtlbMiss, Event::Instruction],
];

/// One profiled zone's sums, each span added under the set it counted.
/// Spans must not nest: [`start`] resets the event counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    /// Spans counted under each of [`SETS`].
    pub spans: [u32; 2],
    /// Cycles of those spans.
    pub cycles: [u64; 2],
    /// Their events, in [`SETS`]' order.
    pub events: [[u64; 3]; 2],
}

/// A zone's split: where its cycles went (per mille of the stall set's
/// cycles) and what caused it (per thousand instructions of the miss
/// set's), all integer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Split {
    /// Spans in all.
    pub spans: u32,
    /// Mean cycles a span (both sets).
    pub cycles_per_span: u64,
    /// Cycles waiting on a data dependency (a load still in flight: the
    /// memory's share), per mille.
    pub data_stall: u32,
    /// Cycles with the data cache's write and fill buffers full (writes
    /// faster than the bus takes them), per mille.
    pub buffer_stall: u32,
    /// Cycles the instruction cache couldn't deliver, per mille.
    pub icache_stall: u32,
    /// What is left: cycles doing work, per mille (0 if the causes sum
    /// past the whole: then they overlap and `overlap` is set).
    pub work: u32,
    /// The three stall counts summed past the cycles (the events overlap
    /// on this core: the split is then an upper bound a cause).
    pub overlap: bool,
    /// Data-cache misses a thousand instructions.
    pub dmiss_per_k: u32,
    /// Data-TLB misses a thousand instructions.
    pub dtlb_per_k: u32,
    /// Instructions a cycle, x100.
    pub ipc_x100: u32,
}

impl Tally {
    /// `c`, counted under set `set` of [`SETS`], added.
    pub fn add(&mut self, set: usize, c: Counts) {
        let set = set % SETS.len();
        self.spans[set] = self.spans[set].saturating_add(1);
        self.cycles[set] += u64::from(c.cycles);
        for (sum, e) in self.events[set].iter_mut().zip(c.events) {
            *sum += u64::from(e);
        }
    }

    /// The split of what was added.
    #[must_use]
    pub fn split(&self) -> Split {
        let pm = |n: u64, of: u64| {
            n.saturating_mul(1000)
                .checked_div(of)
                .map_or(0, |v| u32::try_from(v).unwrap_or(u32::MAX))
        };
        let [data, buffer, icache] = self.events[0];
        let stalls = data + buffer + icache;
        let [dmiss, dtlb, inst] = self.events[1];
        let spans = self.spans[0].saturating_add(self.spans[1]);
        Split {
            spans,
            cycles_per_span: (self.cycles[0] + self.cycles[1]) / u64::from(spans.max(1)),
            data_stall: pm(data, self.cycles[0]),
            buffer_stall: pm(buffer, self.cycles[0]),
            icache_stall: pm(icache, self.cycles[0]),
            work: 1000u32.saturating_sub(pm(stalls, self.cycles[0])),
            overlap: stalls > self.cycles[0],
            dmiss_per_k: pm(dmiss, inst),
            dtlb_per_k: pm(dtlb, inst),
            ipc_x100: pm(inst, self.cycles[1]) / 10,
        }
    }
}

impl core::fmt::Display for Split {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let pct = |pm: u32| (pm / 10, pm % 10);
        let (w, w1) = pct(self.work);
        let (d, d1) = pct(self.data_stall);
        let (b, b1) = pct(self.buffer_stall);
        let (i, i1) = pct(self.icache_stall);
        write!(
            f,
            "{} spans, {} cycles a span; work {w}.{w1}%, waiting on data {d}.{d1}%, write buffers full {b}.{b1}%, instruction fetch {i}.{i1}%{}; D-cache misses {} a 1,000 instructions, D-TLB misses {}, IPC {}.{:02}",
            self.spans,
            self.cycles_per_span,
            if self.overlap {
                " (the causes overlap)"
            } else {
                ""
            },
            self.dmiss_per_k,
            self.dtlb_per_k,
            self.ipc_x100 / 100,
            self.ipc_x100 % 100
        )
    }
}

#[cfg(all(target_arch = "arm", target_os = "none"))]
mod target {
    use super::{Counts, Event, evtsel};

    /// A counting span: [`start`] to [`read`].
    #[derive(Clone, Copy, Debug)]
    pub struct Span {
        cycles: u32,
    }

    /// Selects the events (the fourth unused: see the module notes), resets
    /// the event counters and starts counting. `CCNT` isn't reset (other
    /// code times with it): the span keeps its start. Privileged modes.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
    pub fn start(events: [Event; 3]) -> Span {
        let sel = evtsel([events[0], events[1], events[2], Event::PcChanged]);
        // SAFETY: writes the performance monitor's event selection and
        // control (CP14); enable (bit 0) and reset the event counters (bit
        // 1), `CCNT` left running, no interrupts (`INTEN` stays 0).
        unsafe {
            core::arch::asm!(
                "mcr p14, 0, {sel}, c8, c1, 0",
                "mcr p14, 0, {pmnc}, c0, c1, 0",
                sel = in(reg) sel,
                pmnc = in(reg) 0b011u32,
                options(nostack),
            );
        }
        Span {
            cycles: crate::rt::cycles(),
        }
    }

    /// The counts since `span` started.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
    #[must_use]
    pub fn read(span: Span) -> Counts {
        let (a, b, c): (u32, u32, u32);
        // SAFETY: reads of the event counters; no side effects. Not `nomem`:
        // the work being counted mustn't move across it.
        unsafe {
            core::arch::asm!(
                "mrc p14, 0, {a}, c0, c2, 0",
                "mrc p14, 0, {b}, c1, c2, 0",
                "mrc p14, 0, {c}, c2, c2, 0",
                a = out(reg) a,
                b = out(reg) b,
                c = out(reg) c,
                options(nostack),
            );
        }
        Counts {
            cycles: crate::rt::cycles().wrapping_sub(span.cycles),
            events: [a, b, c],
        }
    }

    /// A loop of known shape for the known-answer checks: `subs` and `bne`,
    /// `n` times (2 `n` instructions, `n` branches, no data accesses).
    #[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
    pub fn known_loop(n: u32) {
        if n == 0 {
            return;
        }
        // SAFETY: counts a register down to 0; touches no memory.
        unsafe {
            core::arch::asm!(
                "2:",
                "subs {n}, {n}, #1",
                "bne 2b",
                n = inout(reg) n => _,
                options(nostack, nomem),
            );
        }
    }
}

#[cfg(all(target_arch = "arm", target_os = "none"))]
pub use target::{Span, known_loop, read, start};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tally_splits_cycles_by_cause_and_misses_by_instructions() {
        let mut t = Tally::default();
        // Set 0: 1,000 cycles, 300 waiting on data, 100 on full buffers,
        // 50 on fetch; twice.
        for _ in 0..2 {
            t.add(
                0,
                Counts {
                    cycles: 1000,
                    events: [300, 100, 50],
                },
            );
        }
        // Set 1: 2,000 cycles, 10 D-cache misses, 2 D-TLB, 1,000
        // instructions.
        t.add(
            1,
            Counts {
                cycles: 2000,
                events: [10, 2, 1000],
            },
        );
        let s = t.split();
        assert_eq!(s.spans, 3);
        assert_eq!(s.cycles_per_span, 4000 / 3);
        assert_eq!(
            (s.data_stall, s.buffer_stall, s.icache_stall),
            (300, 100, 50)
        );
        assert_eq!(s.work, 550);
        assert!(!s.overlap);
        assert_eq!((s.dmiss_per_k, s.dtlb_per_k, s.ipc_x100), (10, 2, 50));
        // A set index past the end wraps; an empty tally divides by nothing.
        t.add(
            2,
            Counts {
                cycles: 0,
                events: [0; 3],
            },
        );
        assert_eq!(t.spans[0], 3);
        assert_eq!(Tally::default().split(), Split::default().with_work(1000));
    }

    #[test]
    fn overlapping_causes_are_flagged_not_wrapped() {
        let mut t = Tally::default();
        t.add(
            0,
            Counts {
                cycles: 100,
                events: [80, 60, 0],
            },
        );
        let s = t.split();
        assert!(s.overlap);
        assert_eq!(s.work, 0);
    }

    impl Split {
        fn with_work(mut self, w: u32) -> Self {
            self.work = w;
            self
        }
    }

    #[test]
    fn evtsel_puts_counter_k_in_byte_k() {
        assert_eq!(
            evtsel([
                Event::Instruction,
                Event::Branch,
                Event::DcacheMiss,
                Event::DtlbMiss
            ]),
            0x040B_0507
        );
        assert_eq!(evtsel([Event::IcacheMiss; 4]), 0);
    }
}
