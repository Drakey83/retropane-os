//! Hardware diagnostics for bring-up on real devices: read-only snapshots of
//! SoC state, and the few experiments the probe image runs.
//!
//! [`REGISTERS`] is a whitelist: only registers whose reads have no side
//! effects (configuration and level registers, never FIFOs or
//! clear-on-read status). Addresses from the PXA27x Developer's Manual.

/// One whitelisted register.
#[derive(Clone, Copy)]
pub struct Reg {
    /// Manual name.
    pub name: &'static str,
    /// Physical (and, under the bring-up map, virtual) address.
    pub addr: u32,
}

const fn r(name: &'static str, addr: u32) -> Reg {
    Reg { name, addr }
}

/// Register groups, each shown together in the probe report.
pub const GROUPS: &[(&str, &[Reg])] = &[
    (
        "CLOCKS",
        &[
            r("CCCR", 0x4130_0000),
            r("CKEN", 0x4130_0004),
            r("OSCC", 0x4130_0008),
            r("CCSR", 0x4130_000C),
        ],
    ),
    (
        "MEMORY",
        &[
            r("MDCNFG", 0x4800_0000),
            r("MDREFR", 0x4800_0004),
            r("MSC0", 0x4800_0008),
            r("MSC1", 0x4800_000C),
            r("MSC2", 0x4800_0010),
            r("MECR", 0x4800_0014),
            r("SXCNFG", 0x4800_001C),
            r("MDMRS", 0x4800_0040),
            r("BOOTDEF", 0x4800_0044),
            r("ARBCNTL", 0x4800_0048),
            r("SA1110", 0x4800_0064),
        ],
    ),
    (
        "INTC",
        &[
            r("ICMR", 0x40D0_0004),
            r("ICLR", 0x40D0_0008),
            r("ICCR", 0x40D0_0014),
            r("ICPR", 0x40D0_0010),
            r("ICMR2", 0x40D0_00A0),
            r("ICLR2", 0x40D0_00A4),
            r("ICPR2", 0x40D0_00AC),
        ],
    ),
    (
        "POWER",
        &[
            r("RCSR", 0x40F0_0030),
            r("PSSR", 0x40F0_0004),
            r("PCFR", 0x40F0_001C),
            r("PWER", 0x40F0_000C),
        ],
    ),
    (
        "PWM+LCD",
        &[
            r("PWMCR0", 0x40B0_0000),
            r("PWMDCR0", 0x40B0_0004),
            r("PWMPCR0", 0x40B0_0008),
            r("PWMCR1", 0x40C0_0000),
            r("PWMDCR1", 0x40C0_0004),
            r("PWMPCR1", 0x40C0_0008),
            r("LCCR0", 0x4400_0000),
            r("LCCR3", 0x4400_000C),
            r("IMPMCR", 0x5800_0000),
        ],
    ),
    (
        "GPIO LEVEL/DIR",
        &[
            r("GPLR0", 0x40E0_0000),
            r("GPLR1", 0x40E0_0004),
            r("GPLR2", 0x40E0_0008),
            r("GPLR3", 0x40E0_0100),
            r("GPDR0", 0x40E0_000C),
            r("GPDR1", 0x40E0_0010),
            r("GPDR2", 0x40E0_0014),
            r("GPDR3", 0x40E0_010C),
        ],
    ),
    (
        "GPIO ALT FN",
        &[
            r("GAFR0L", 0x40E0_0054),
            r("GAFR0U", 0x40E0_0058),
            r("GAFR1L", 0x40E0_005C),
            r("GAFR1U", 0x40E0_0060),
            r("GAFR2L", 0x40E0_0064),
            r("GAFR2U", 0x40E0_0068),
            r("GAFR3L", 0x40E0_006C),
            r("GAFR3U", 0x40E0_0070),
        ],
    ),
    (
        "GPIO EDGES",
        &[
            r("GRER0", 0x40E0_0030),
            r("GRER1", 0x40E0_0034),
            r("GRER2", 0x40E0_0038),
            r("GRER3", 0x40E0_0130),
            r("GFER0", 0x40E0_003C),
            r("GFER1", 0x40E0_0040),
            r("GFER2", 0x40E0_0044),
            r("GFER3", 0x40E0_013C),
        ],
    ),
];

/// Number of whitelisted registers.
pub const COUNT: usize = {
    let mut n = 0;
    let mut i = 0;
    while i < GROUPS.len() {
        n += GROUPS[i].1.len();
        i += 1;
    }
    n
};

/// Physical base of the PXA27x internal SRAM.
pub const SRAM_BASE: u32 = 0x5C00_0000;
/// Internal SRAM size.
pub const SRAM_SIZE: usize = 256 * 1024;
/// `CKEN` bit for the internal memory.
pub const CKEN_SRAM: u32 = 20;

/// Coprocessor state.
#[derive(Clone, Copy, Default)]
pub struct Cp {
    /// CP15 c0: main ID.
    pub id: u32,
    /// CP15 c1: control (MMU, caches, vectors).
    pub control: u32,
    /// CP15 c1 opcode2 1: XScale auxiliary control.
    pub aux: u32,
    /// CP15 c2: translation table base.
    pub ttb: u32,
    /// CP15 c3: domain access control.
    pub dacr: u32,
    /// CP15 c13: FCSE process ID.
    pub pid: u32,
    /// CP15 c15: coprocessor access.
    pub cpar: u32,
    /// CP14 c6: clock configuration (turbo, fast bus).
    pub clkcfg: u32,
}

#[cfg(all(target_arch = "arm", target_os = "none"))]
mod target {
    use super::{CKEN_SRAM, Cp, GROUPS, SRAM_BASE, SRAM_SIZE};

    /// Reads every whitelisted register, in [`GROUPS`] order.
    pub fn snapshot(out: &mut [u32; super::COUNT]) {
        let mut i = 0;
        for (_, regs) in GROUPS {
            for reg in *regs {
                // SAFETY: every address is a whitelisted PXA27x register with
                // no read side effects, mapped (identity, device memory) both
                // with the MMU off and under the bring-up map.
                out[i] =
                    unsafe { (crate::devmap::va(reg.addr as usize) as *const u32).read_volatile() };
                i += 1;
            }
        }
    }

    /// Reads the coprocessor registers.
    #[must_use]
    pub fn coprocessors() -> Cp {
        let mut c = Cp::default();
        // SAFETY: reads of ID, control, TTB, DACR, PID, CPAR and CLKCFG have
        // no side effects and are allowed in any privileged mode.
        unsafe {
            core::arch::asm!(
                "mrc p15, 0, {id}, c0, c0, 0",
                "mrc p15, 0, {ctl}, c1, c0, 0",
                "mrc p15, 0, {aux}, c1, c0, 1",
                "mrc p15, 0, {ttb}, c2, c0, 0",
                "mrc p15, 0, {dacr}, c3, c0, 0",
                "mrc p15, 0, {pid}, c13, c0, 0",
                "mrc p15, 0, {cpar}, c15, c1, 0",
                "mrc p14, 0, {clk}, c6, c0, 0",
                id = out(reg) c.id,
                ctl = out(reg) c.control,
                aux = out(reg) c.aux,
                ttb = out(reg) c.ttb,
                dacr = out(reg) c.dacr,
                pid = out(reg) c.pid,
                cpar = out(reg) c.cpar,
                clk = out(reg) c.clkcfg,
                options(nostack, nomem),
            );
        }
        c
    }

    static mut SRAM_TAKEN: bool = false;

    /// The 256 KiB internal SRAM, once, with its clock enabled. Needs the
    /// bring-up map (which maps it as normal memory).
    pub fn take_sram() -> Option<&'static mut [u8]> {
        // SAFETY: single core; callers run with IRQs masked or from the one
        // main thread, so nothing races this read-modify-write of the flag.
        if unsafe { core::ptr::addr_of_mut!(SRAM_TAKEN).replace(true) } {
            return None;
        }
        crate::rt::enable_clock(CKEN_SRAM);
        // SAFETY: the internal SRAM is 256 KiB of on-chip memory at
        // SRAM_BASE, mapped by the bring-up map; nothing else in RetroPane
        // uses it, and the flag above hands it out once.
        Some(unsafe {
            core::slice::from_raw_parts_mut(
                crate::devmap::va(SRAM_BASE as usize) as *mut u8,
                SRAM_SIZE,
            )
        })
    }

    /// Moves the live first-level page table to `to` (16 KiB aligned, 16 KiB
    /// of memory owned by the caller, e.g. from [`take_sram`]) and makes the
    /// MMU use it: the ADR-0006 rule 1 experiment. The table's contents are
    /// unchanged, so every mapping stays the same.
    ///
    /// # Panics
    /// If `to` isn't 16 KiB aligned and 16 KiB long.
    pub fn move_l1_table(to: &mut [u8]) {
        assert!(to.len() >= 16 * 1024 && (to.as_ptr() as usize).is_multiple_of(16 * 1024));
        // TTB takes a physical address: valid here only because the
        // bring-up map is the identity.
        assert!(!crate::devmap::kernel_map(), "bring-up map only");
        let ttb: u32;
        // SAFETY: reading TTB has no side effects.
        unsafe {
            core::arch::asm!("mrc p15, 0, {}, c2, c0, 0", out(reg) ttb, options(nostack, nomem))
        };
        let from = (ttb & !0x3FFF) as *const u8;
        // SAFETY: `from` is the live 16 KiB table (identity-mapped under the
        // bring-up map); `to` is caller-owned memory of at least 16 KiB. The
        // copy is complete and written back (clean D-cache line by line,
        // drain) before TTB switches, then the TLBs are invalidated, so no
        // stale walk can see a partial table. Interrupts don't matter: the
        // old table stays valid throughout.
        unsafe {
            core::ptr::copy_nonoverlapping(from, to.as_mut_ptr(), 16 * 1024);
            let mut line = to.as_ptr() as u32;
            let end = line + 16 * 1024;
            while line < end {
                core::arch::asm!("mcr p15, 0, {}, c7, c10, 1", in(reg) line, options(nostack));
                line += 32;
            }
            core::arch::asm!(
                "mov {t}, #0",
                "mcr p15, 0, {t}, c7, c10, 4",  // drain write buffer
                "mcr p15, 0, {ttb}, c2, c0, 0", // new table base
                "mcr p15, 0, {t}, c8, c7, 0",   // invalidate TLBs
                "mrc p15, 0, {t}, c2, c0, 0",   // CPWAIT
                "mov {t}, {t}",
                "sub pc, pc, #4",
                t = out(reg) _,
                ttb = in(reg) to.as_ptr() as u32,
                options(nostack),
            );
        }
    }
}

#[cfg(all(target_arch = "arm", target_os = "none"))]
pub use target::{coprocessors, move_l1_table, snapshot, take_sram};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitelist_is_aligned_and_unique() {
        let all = || GROUPS.iter().flat_map(|(_, regs)| regs.iter());
        for reg in all() {
            assert!(reg.addr.is_multiple_of(4), "{}", reg.name);
            assert_eq!(
                all().filter(|o| o.addr == reg.addr).count(),
                1,
                "duplicate {}",
                reg.name
            );
            assert!(reg.name.len() <= 8, "{} too long for the report", reg.name);
        }
        assert_eq!(all().count(), COUNT);
    }

    #[test]
    fn no_fifos_or_clear_on_read_registers() {
        // Spot-check the dangerous neighbours stay out: GEDR (edge status,
        // write-1-to-clear but read-safe, still excluded), ICIP/ICHP (live),
        // OSCR, UART data registers.
        for bad in [
            0x40E0_0048,
            0x40D0_0000,
            0x40D0_0018,
            0x40A0_0010,
            0x4010_0000,
        ] {
            assert!(
                GROUPS
                    .iter()
                    .all(|(_, regs)| regs.iter().all(|r| r.addr != bad)),
                "{bad:#x} must not be whitelisted"
            );
        }
    }

    #[test]
    fn sram_facts() {
        assert_eq!(SRAM_BASE, 0x5C00_0000);
        assert_eq!(SRAM_SIZE, 0x4_0000);
        assert!(SRAM_BASE.is_multiple_of(16 * 1024), "can hold an L1 table");
    }
}
