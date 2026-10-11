//! Per-process protection with ARM domains (ADR-0003, ADR-0008).
//!
//! CE5 let any process read every other process's slot. RetroPane gives each
//! active process its own ARM domain and enables only that one (plus the
//! kernel's and the shared area's) in the Domain Access Control Register, so
//! another process's pages fault even for the kernel. Switching is a DACR
//! write: no page-table, TLB or cache work.
//!
//! ARMv5 has 16 domains and CE5 allows 31 processes, so domains are
//! recycled: when every process domain is taken, the least recently run
//! process is *parked*: its slot's first-level entries are retagged to
//! [`PARKED`] (never enabled) and it gets a domain again when it next runs.
//! [`Domains::activate`] says what to retag; the caller does it (32 L1
//! entries per slot, plus a TLB flush for that slot).

use crate::layout::{FIRST_PROCESS_SLOT, LAST_PROCESS_SLOT};

/// The kernel's domain.
pub const KERNEL: u32 = 0;
/// The shared memory-mapped file area's domain.
pub const SHARED: u32 = 1;
/// Domain of parked processes' pages: never enabled.
pub const PARKED: u32 = 15;
/// Domains handed to processes.
pub const FIRST: u32 = 2;
/// See [`FIRST`].
pub const LAST: u32 = 14;

const SLOTS: usize = LAST_PROCESS_SLOT as usize + 1;

/// What switching to a process requires.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub struct Activation {
    /// The process's domain.
    pub domain: u32,
    /// The DACR to load: kernel, shared area and this process's domain as
    /// clients.
    pub dacr: u32,
    /// The process just got this domain: retag its slot's L1 entries to it
    /// (it was new or parked).
    pub retag: bool,
    /// Another process lost the domain: retag its slot to [`PARKED`] and
    /// flush its TLB entries first.
    pub evicted: Option<u32>,
}

/// Domain ownership.
pub struct Domains {
    /// Per slot: its domain, or 0 for none (0 is the kernel's, never a
    /// process's).
    of_slot: [u8; SLOTS],
    /// Per domain: the slot that owns it, or 0.
    owner: [u8; 16],
    /// Per domain: when it was last activated.
    used: [u32; 16],
    clock: u32,
}

impl Default for Domains {
    fn default() -> Self {
        Self::new()
    }
}

impl Domains {
    /// No process has a domain.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            of_slot: [0; SLOTS],
            owner: [0; 16],
            used: [0; 16],
            clock: 0,
        }
    }

    /// The domain `slot` currently has.
    #[must_use]
    pub fn domain_of(&self, slot: u32) -> Option<u32> {
        match self.of_slot.get(slot as usize) {
            Some(&d) if d != 0 => Some(u32::from(d)),
            _ => None,
        }
    }

    /// Prepares to run the process in `slot`.
    ///
    /// # Panics
    /// If `slot` isn't a process slot.
    pub fn activate(&mut self, slot: u32) -> Activation {
        assert!(
            (FIRST_PROCESS_SLOT..=LAST_PROCESS_SLOT).contains(&slot),
            "not a process slot"
        );
        self.clock = self.clock.wrapping_add(1);
        if let Some(d) = self.domain_of(slot) {
            self.used[d as usize] = self.clock;
            return Self::activation(d, false, None);
        }
        // A free domain, or the least recently used one.
        let d = (FIRST..=LAST)
            .find(|&d| self.owner[d as usize] == 0)
            .unwrap_or_else(|| {
                (FIRST..=LAST)
                    .min_by_key(|&d| {
                        self.clock
                            .wrapping_sub(self.used[d as usize])
                            .wrapping_neg()
                    })
                    .unwrap_or(FIRST)
            });
        let evicted = match self.owner[d as usize] {
            0 => None,
            victim => {
                self.of_slot[usize::from(victim)] = 0;
                Some(u32::from(victim))
            }
        };
        self.owner[d as usize] = slot as u8;
        self.of_slot[slot as usize] = d as u8;
        self.used[d as usize] = self.clock;
        Self::activation(d, true, evicted)
    }

    /// Forgets `slot` (its process exited); its domain becomes free.
    pub fn release(&mut self, slot: u32) {
        if let Some(d) = self.domain_of(slot) {
            self.owner[d as usize] = 0;
            self.of_slot[slot as usize] = 0;
        }
    }

    fn activation(domain: u32, retag: bool, evicted: Option<u32>) -> Activation {
        Activation {
            domain,
            dacr: crate::arm::dacr((1 << KERNEL) | (1 << SHARED) | (1 << domain)),
            retag,
            evicted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_process_gets_its_own_domain() {
        let mut d = Domains::new();
        let a = d.activate(5);
        let b = d.activate(6);
        assert_ne!(a.domain, b.domain);
        assert!(a.retag && b.retag && a.evicted.is_none());
        assert!((FIRST..=LAST).contains(&a.domain));
        // The DACR enables kernel, shared and its own domain, nothing else.
        assert_eq!(a.dacr, crate::arm::dacr(0b11 | (1 << a.domain)));
        assert_eq!(b.dacr & (0b11 << (2 * a.domain)), 0, "b can't touch a");
        // Running a again: same domain, nothing to retag.
        let again = d.activate(5);
        assert_eq!(
            (again.domain, again.retag, again.evicted),
            (a.domain, false, None)
        );
    }

    #[test]
    fn the_least_recently_run_process_is_parked() {
        let mut d = Domains::new();
        let n = (LAST - FIRST + 1) as usize; // 13 process domains
        for slot in 2..2 + n as u32 {
            assert!(d.activate(slot).evicted.is_none());
        }
        // Run slot 2 again, so slot 3 is now the least recent.
        d.activate(2);
        let x = d.activate(2 + n as u32);
        assert_eq!(x.evicted, Some(3));
        assert!(x.retag);
        assert_eq!(d.domain_of(3), None, "parked");
        assert_eq!(d.domain_of(2 + n as u32), Some(x.domain));
        // Slot 3 runs again: it gets a domain back (evicting slot 4 now).
        let y = d.activate(3);
        assert_eq!(y.evicted, Some(4));
        assert!(y.retag);
    }

    #[test]
    fn released_domains_are_reused_without_evicting() {
        let mut d = Domains::new();
        for slot in 2..=14 {
            d.activate(slot);
        }
        let freed = d.domain_of(9).unwrap();
        d.release(9);
        let z = d.activate(20);
        assert_eq!((z.domain, z.evicted), (freed, None));
        d.release(31); // never had one: no effect
    }

    #[test]
    fn reserved_domains_never_go_to_processes() {
        let mut d = Domains::new();
        for round in 0..3 {
            for slot in FIRST_PROCESS_SLOT..=LAST_PROCESS_SLOT {
                let a = d.activate(slot);
                assert!(
                    ![KERNEL, SHARED, PARKED].contains(&a.domain),
                    "round {round} slot {slot}"
                );
            }
        }
    }

    #[test]
    #[should_panic(expected = "not a process slot")]
    fn slot_zero_isnt_a_process() {
        Domains::new().activate(0);
    }
}
