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
