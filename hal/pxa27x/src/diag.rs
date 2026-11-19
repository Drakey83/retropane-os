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
            // The low-power mode value (PASR, TCSR, drive strength) last
            // written: reading it issues nothing (writing does).
            r("MDMRSLP", 0x4800_0058),
            r("BOOTDEF", 0x4800_0044),
            r("ARBCNTL", 0x4800_0048),
            r("SA1110", 0x4800_0064),
        ],
    ),
    (
        // PC Card socket timing (socket 0: Wi-Fi, socket 1: CompactFlash,
        // docs/hw/cf.md): how Windows Mobile set them up.
        "PC CARD",
        &[
            r("MCMEM0", 0x4800_0028),
            r("MCMEM1", 0x4800_002C),
            r("MCATT0", 0x4800_0030),
            r("MCATT1", 0x4800_0034),
            r("MCIO0", 0x4800_0038),
            r("MCIO1", 0x4800_003C),
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

/// Physical address of `MSC1` (static chip selects 2 and 3; CS3 is the
/// 2700G on the X51v).
pub const MSC1: u32 = 0x4800_000C;

/// `MSC1` with CS3's VLIO strobe timing replaced: `rdf` (strobe asserted
/// for `rdf` + 1 memory clocks per beat) and `rdn` (released for 2 x `rdn`:
/// address setup and hold), the only two fields the bus-timing experiment
/// steps (docs/research/memory-timing.md §7). Everything else is kept:
/// CS2's half, CS3's type, width, recovery (`RRR`, which also guards the
/// data bus against the SDRAM's next cycle) and buffering. `None` unless
/// CS3 is a 32-bit VLIO device and both values are inside the PXA27x's
/// legal VLIO range with plain (not table-encoded) values: `rdf` 3-11,
/// `rdn` 2-11 (Developer's Manual table 6-28).
#[must_use]
pub const fn cs3_timing(msc1: u32, rdf: u32, rdn: u32) -> Option<u32> {
    let upper = msc1 >> 16;
    let vlio_32 = upper & 0xF == 0b0100; // RT3 = VLIO, RBW3 = 32 bits
    if !vlio_32 || rdf < 3 || rdf > 11 || rdn < 2 || rdn > 11 {
        return None;
    }
    Some(msc1 & !(0xFF << 20) | rdf << 20 | rdn << 24)
}

/// `MSC1` with all of CS3's VLIO timing replaced: `rdf` and `rdn` as
/// codes 3-15 and 2-15 (12-15 table-encoded to 15, 20, 26 and 30 clocks:
/// longer, so also legal), `rrr` 0-7 (the chip-select gap, 2 x `rrr` + 1
/// clocks). For the 2700G clock experiments (rp-mbx3d version 29), which
/// start from the long timing `0x79D4` and step down. CS2's half, CS3's
/// type, width and buffering are kept; `None` unless CS3 is a 32-bit VLIO
/// device and the values are in range.
#[must_use]
pub const fn cs3_vlio(msc1: u32, rdf: u32, rdn: u32, rrr: u32) -> Option<u32> {
    let upper = msc1 >> 16;
    let vlio_32 = upper & 0xF == 0b0100;
    if !vlio_32 || rdf < 3 || rdf > 15 || rdn < 2 || rdn > 15 || rrr > 7 {
        return None;
    }
    Some(msc1 & !(0x7FF << 20) | rdf << 20 | rdn << 24 | rrr << 28)
}

/// `MSC1` with chip select 2's half (bits 15-0) replaced by `half`, CS3's
/// half kept: the 2700G's second interface on the X51v (Dell's
/// `SRAMChipSelect` 2), whose timing Dell's driver rewrites at every clock
/// change (`ddi_2700g` `0x0211FFC8`: `0x7DF1` at 74.75 MHz / 1, `0x2481` at
/// Windows Mobile's 99.67 / 2, docs/hw/2700g.md "Test G10"). `None` if
/// `half` has bits above 15 or would change CS2's type and width (its low
/// nibble: RT and RBW), so only the timing fields move.
#[must_use]
pub const fn cs2_half(msc1: u32, half: u32) -> Option<u32> {
    if half > 0xFFFF || half & 0xF != msc1 & 0xF {
        return None;
    }
    Some(msc1 & !0xFFFF | half)
}

/// Physical address of `MSC2` (static chip selects 4 and 5; CS4 is the
/// board's CPLD on the X51v).
pub const MSC2: u32 = 0x4800_0010;

/// `MSC2` with chip select 4's VLIO timing replaced (its low half: `rdf`
/// bits 7-4, `rdn` bits 11-8, `rrr` bits 14-12), for the CPLD bus test
/// (reminder R8, docs/research/memory-timing.md section 9). Codes as
/// [`cs3_vlio`]: `rdf` 3-15, `rdn` 2-15 (12-15 table-encoded, longer),
/// `rrr` 0-7. CS5's half, CS4's type, width and buffering are kept; `None`
/// unless CS4 is a 16-bit VLIO device (`RT4` VLIO, `RBW4` 16 bits, as
/// Windows Mobile's `0x7FFC`) and the values are in range.
#[must_use]
pub const fn cs4_vlio(msc2: u32, rdf: u32, rdn: u32, rrr: u32) -> Option<u32> {
    let vlio_16 = msc2 & 0xF == 0b1100;
    if !vlio_16 || rdf < 3 || rdf > 15 || rdn < 2 || rdn > 15 || rrr > 7 {
        return None;
    }
    Some(msc2 & !0x7FF0 | rdf << 4 | rdn << 8 | rrr << 12)
}

/// A VLIO `RDF` or `RDN` code's clocks (codes 12-15 are a table).
#[must_use]
pub const fn vlio_clocks(code: u32) -> u32 {
    match code {
        12 => 15,
        13 => 20,
        14 => 26,
        15 => 30,
        c => c,
    }
}

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

/// The 32-bit word at byte `at` of `buf`, read with one word load (a
/// pointer chase must cost one load a hop, as Windows Mobile's C does:
/// assembling it from four byte loads cost ~30 ns a hop and four
/// counted D-cache misses, CPU gate V7, 2026-10-24). Panics if the word
/// isn't inside `buf` or isn't word aligned.
#[inline(always)]
#[must_use]
pub fn read_word(buf: &[u8], at: usize) -> u32 {
    let word = &buf[at..at + 4];
    assert!(word.as_ptr().addr().is_multiple_of(4), "unaligned word");
    // SAFETY: the four bytes are in `buf` (sliced above, so in bounds and
    // borrowed), the address is word aligned (asserted), and every bit
    // pattern is a valid `u32`.
    unsafe { word.as_ptr().cast::<u32>().read() }
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
    #[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
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

    /// One `MRC` (its encoding `enc`, the result in r0) that resumes
    /// with `None` if the instruction is undefined (no such coprocessor,
    /// or no access to it): the undefined entry's fixup (rt.rs).
    macro_rules! try_mrc {
        ($enc:expr) => {{
            let (v, failed): (u32, u32);
            // SAFETY: one coprocessor read with no side effects in any
            // coprocessor RetroPane knows of; if it is undefined the
            // undefined entry resumes at label 2 (registered in
            // .rp_fixup), which only sets the flag.
            unsafe {
                core::arch::asm!(
                    "mov {f}, #0",
                    "1: .inst {e}",
                    "b 3f",
                    "2: mov {f}, #1",
                    "mov r0, #0",
                    "3:",
                    ".pushsection .rp_fixup, \"a\"",
                    ".word 1b, 2b",
                    ".popsection",
                    e = const $enc,
                    f = out(reg) failed,
                    out("r0") v,
                    options(nostack, nomem),
                );
            }
            (failed == 0).then_some(v)
        }};
    }

    /// What [`coprocessor_survey`] found.
    #[derive(Clone, Copy, Debug)]
    pub struct CoSurvey {
        /// `CPAR` as found.
        pub cpar_was: u32,
        /// The access bits that stuck when all of CP0-CP13 were asked for.
        pub cpar_settable: u32,
        /// Bit n: `MRC pn, 0, r0, c0, c0, 0` ran (CP0-CP13) without trapping.
        pub answered: u16,
        /// What each answering coprocessor's c0 read gave.
        pub c0: [u32; 14],
        /// The VFP's ID register (`MRC p10, 7, r0, c0, c0, 0`, FPSID), if
        /// it answered: an FPU.
        pub fpsid: Option<u32>,
        /// CP7's registers at every CRn x CRm (`MRC p7, 0, r0, cN, cM, 0`,
        /// index `16 * N + M`), each if it answered.
        pub cp7: [Option<u32>; 256],
    }

    /// Asks the CPU itself which coprocessors exist (ADR-0040 step 7g7, the
    /// owner: "Test for an FPU too"): every access bit of `CPAR` set, each
    /// of CP0-CP13 read once (a trap means absent), the VFP's FPSID read;
    /// `CPAR` put back as found. Interrupts masked by the caller.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
    pub fn coprocessor_survey() -> CoSurvey {
        let mut s = CoSurvey {
            cpar_was: 0,
            cpar_settable: 0,
            answered: 0,
            c0: [0; 14],
            fpsid: None,
            cp7: [None; 256],
        };
        let cpwait = || {
            // SAFETY: CPWAIT (Intel's sequence: read CP15, use it, branch),
            // no side effects.
            unsafe {
                core::arch::asm!("mrc p15, 0, {t}, c2, c0, 0", "mov {t}, {t}", "sub pc, pc, #4", t = out(reg) _, options(nostack, nomem));
            }
        };
        let set = |v: u32| {
            // SAFETY: CPAR only gates coprocessor access (put back below).
            unsafe {
                core::arch::asm!("mcr p15, 0, {v}, c15, c1, 0", v = in(reg) v, options(nostack, nomem))
            };
        };
        let get = || {
            let v: u32;
            // SAFETY: a read of CPAR.
            unsafe {
                core::arch::asm!("mrc p15, 0, {v}, c15, c1, 0", v = out(reg) v, options(nostack, nomem))
            };
            v
        };
        s.cpar_was = get();
        set(0x3FFF);
        cpwait();
        s.cpar_settable = get();
        macro_rules! cp {
            ($($n:literal)*) => {$(
                if let Some(v) = try_mrc!(0xEE10_0010u32 | ($n << 8)) {
                    s.answered |= 1 << $n;
                    s.c0[$n] = v;
                }
            )*};
        }
        cp!(0 1 2 3 4 5 6 7 8 9 10 11 12 13);
        s.fpsid = try_mrc!(0xEEF0_0A10u32);
        // CP7 at every CRn x CRm (opc1 0, opc2 0): 256 reads, each one
        // that answers kept (Windows Mobile's interrupt entry reads
        // c4, c1 and tests bit 5: nk.exe 0x8C2466A0)
        macro_rules! cp7 {
            ($n:literal; $($m:literal)*) => {$(
                s.cp7[$n * 16 + $m] = try_mrc!(0xEE10_0710u32 | ($n << 16) | $m);
            )*};
        }
        macro_rules! cp7_all {
            ($($n:literal)*) => {$( cp7!($n; 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15); )*};
        }
        cp7_all!(0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15);
        set(s.cpar_was);
        cpwait();
        s
    }

    /// The live `MSC1`.
    #[must_use]
    pub fn msc1() -> u32 {
        // SAFETY: MSC1 is a memory-controller configuration register with no
        // read side effects, mapped as device memory under either map.
        unsafe { (crate::devmap::va(super::MSC1 as usize) as *const u32).read_volatile() }
    }

    /// Sets CS3's strobe timing ([`super::cs3_timing`]) and returns `MSC1`
    /// as read back, or `None` (nothing written) for a value outside the
    /// legal range. The write buffer is drained first, so every 2700G
    /// write already issued completes under the old timing; the read-back
    /// makes the controller latch the new value before the next access
    /// (manual §6.5.2). The caller keeps the 2700G otherwise idle meanwhile.
    pub fn set_cs3_timing(rdf: u32, rdn: u32) -> Option<u32> {
        let new = super::cs3_timing(msc1(), rdf, rdn)?;
        crate::cpu::drain_write_buffer();
        // SAFETY: only CS3's RDF/RDN fields change, within the PXA27x's legal
        // VLIO range (checked above); the register is the memory
        // controller's, mapped as device memory.
        unsafe { (crate::devmap::va(super::MSC1 as usize) as *mut u32).write_volatile(new) };
        Some(msc1())
    }

    /// Sets CS3's whole VLIO timing ([`super::cs3_vlio`]) and returns
    /// `MSC1` as read back, or `None` (nothing written) out of range. As
    /// [`set_cs3_timing`]: the write buffer drained first, the read-back
    /// latching the new value; the caller keeps the 2700G idle meanwhile.
    pub fn set_cs3_vlio(rdf: u32, rdn: u32, rrr: u32) -> Option<u32> {
        let new = super::cs3_vlio(msc1(), rdf, rdn, rrr)?;
        crate::cpu::drain_write_buffer();
        // SAFETY: only CS3's RDF/RDN/RRR fields change, within the PXA27x's
        // legal VLIO range (checked above); the register is the memory
        // controller's, mapped as device memory.
        unsafe { (crate::devmap::va(super::MSC1 as usize) as *mut u32).write_volatile(new) };
        Some(msc1())
    }

    /// Maps every MiB of `virt..virt + mibs MiB` that the live table leaves
    /// unmapped as a device section at the same physical address (a bus
    /// survey's reads then reach the hardware, not the MMU), runs `f`, and
    /// unmaps them again. Under the bring-up map only (returns false and
    /// does nothing otherwise). Interrupts masked by the caller is not
    /// needed: nothing else uses those addresses.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
    pub fn with_device_window(virt: u32, mibs: u32, f: &mut dyn FnMut()) -> bool {
        if crate::devmap::kernel_map() || !virt.is_multiple_of(1 << 20) {
            return false;
        }
        let ttb: u32;
        // SAFETY: reading TTB has no side effects.
        unsafe {
            core::arch::asm!("mrc p15, 0, {}, c2, c0, 0", out(reg) ttb, options(nostack, nomem))
        };
        let l1 = (ttb & !0x3FFF) as *mut u32;
        let first = (virt >> 20) as usize;
        let mut added = [false; 512];
        let n = (mibs as usize).min(512);
        for i in 0..n {
            // SAFETY: the live first-level table, identity-mapped under the
            // bring-up map (checked); only entries that map nothing (0) are
            // filled, and emptied again below.
            unsafe {
                if l1.add(first + i).read_volatile() == 0 {
                    l1.add(first + i).write_volatile(crate::mmu::section(
                        virt + ((i as u32) << 20),
                        crate::mmu::Memory::Device,
                    ));
                    added[i] = true;
                }
            }
        }
        crate::cpu::clean_range(l1 as u32 + 4 * first as u32, 4 * n as u32);
        crate::cpu::invalidate_tlb_all();
        f();
        for i in 0..n {
            if added[i] {
                // SAFETY: the entries filled above, emptied.
                unsafe { l1.add(first + i).write_volatile(0) };
            }
        }
        crate::cpu::clean_range(l1 as u32 + 4 * first as u32, 4 * n as u32);
        crate::cpu::invalidate_tlb_all();
        true
    }

    /// CS3 as a 32-bit non-burst SRAM chip select at its longest timing
    /// (RT3 `000`, RBW3 32 bits, RDF and RDN code 15 = 30 clocks, RRR 7)
    /// instead of VLIO, for a survey of the 2700G's space (ADR-0040 step
    /// 7g8): SRAM cycles do not wait for the chip's ready line, so a word
    /// the chip does not decode reads back whatever the bus holds instead
    /// of holding the bus forever. Reads only while it lasts (an SRAM
    /// write strobes nWE, not VLIO's nPWE). Returns `MSC1` as it was, for
    /// [`put_msc1`]; the caller keeps the 2700G idle meanwhile.
    pub fn set_cs3_sram() -> u32 {
        let was = msc1();
        let new = was & 0x0000_FFFF | (15 << 4 | 15 << 8 | 7 << 12) << 16;
        crate::cpu::drain_write_buffer();
        // SAFETY: CS3's half of the memory controller's static-memory
        // register; CS2's half kept. Only the type and timing change.
        unsafe { (crate::devmap::va(super::MSC1 as usize) as *mut u32).write_volatile(new) };
        let _ = msc1();
        was
    }

    /// `MSC1` put back to `v` (as [`set_cs3_sram`] returned it).
    pub fn put_msc1(v: u32) {
        crate::cpu::drain_write_buffer();
        // SAFETY: the value this register held before the survey.
        unsafe { (crate::devmap::va(super::MSC1 as usize) as *mut u32).write_volatile(v) };
        let _ = msc1();
    }

    /// CS2's half of `MSC1` replaced by `half` ([`super::cs2_half`]): the
    /// new `MSC1` as read back, or `None` (nothing written). The write
    /// buffer drained first; the caller keeps the 2700G idle meanwhile.
    pub fn set_cs2_half(half: u32) -> Option<u32> {
        let new = super::cs2_half(msc1(), half)?;
        crate::cpu::drain_write_buffer();
        // SAFETY: only CS2's timing fields change (its type and width are
        // held by `cs2_half`); the register is the memory controller's,
        // mapped as device memory.
        unsafe { (crate::devmap::va(super::MSC1 as usize) as *mut u32).write_volatile(new) };
        Some(msc1())
    }

    /// The live `MSC2`.
    #[must_use]
    pub fn msc2() -> u32 {
        // SAFETY: MSC2 is a memory-controller configuration register with no
        // read side effects, mapped as device memory under either map.
        unsafe { (crate::devmap::va(super::MSC2 as usize) as *const u32).read_volatile() }
    }

    /// Sets CS4's VLIO timing ([`super::cs4_vlio`]) and returns `MSC2` as
    /// read back, or `None` (nothing written) out of range. The write
    /// buffer is drained first, so every CPLD write already issued
    /// completes under the old timing; the read-back latches the new value
    /// before the next access. The caller keeps the CPLD otherwise idle.
    pub fn set_cs4_vlio(rdf: u32, rdn: u32, rrr: u32) -> Option<u32> {
        let new = super::cs4_vlio(msc2(), rdf, rdn, rrr)?;
        crate::cpu::drain_write_buffer();
        // SAFETY: only CS4's RDF/RDN/RRR fields change, within the PXA27x's
        // legal VLIO range (checked above); the register is the memory
        // controller's, mapped as device memory.
        unsafe { (crate::devmap::va(super::MSC2 as usize) as *mut u32).write_volatile(new) };
        Some(msc2())
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

    /// The internal SRAM as words, once (shares [`take_sram`]'s flag: one or
    /// the other), with its clock enabled, at its current virtual address
    /// (the bring-up map's identity or the kernel map's window, whichever is
    /// live when taken).
    pub fn take_sram_words() -> Option<&'static mut [u32]> {
        let bytes = take_sram()?;
        let (ptr, len) = (bytes.as_mut_ptr(), bytes.len());
        // SAFETY: `bytes` is the whole SRAM, handed out once (the flag in
        // take_sram); its base is 256 KiB aligned, so word aligned, and the
        // byte slice is consumed here, so this is the only reference.
        Some(unsafe { core::slice::from_raw_parts_mut(ptr.cast::<u32>(), len / 4) })
    }

    /// Moves the live first-level page table to `to`, whose physical address
    /// is `to_phys` (16 KiB aligned), and makes the MMU use it: ADR-0006 rule
    /// 1 (page tables in on-chip SRAM). Works under either map, as long as
    /// the live table is readable at its physical address (RAM is identity
    /// mapped in both). The contents are unchanged, so every mapping stays.
    ///
    /// # Panics
    /// If `to_phys` isn't 16 KiB aligned.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
    pub fn move_l1_table_at(to: &mut [u32; 4096], to_phys: u32) {
        assert!(to_phys.is_multiple_of(16 * 1024));
        let ttb: u32;
        // SAFETY: reading TTB has no side effects.
        unsafe {
            core::arch::asm!("mrc p15, 0, {}, c2, c0, 0", out(reg) ttb, options(nostack, nomem))
        };
        let from = (ttb & !0x3FFF) as *const u32;
        // SAFETY: `from` is the live 16 KiB table, readable at its physical
        // address (identity-mapped RAM); `to` is 16 KiB the caller owns. The
        // copy is written back (clean D-cache line by line, drain) before
        // TTB switches, then the TLBs are invalidated, so no walk sees a
        // partial table. The old table stays valid throughout.
        unsafe {
            core::ptr::copy_nonoverlapping(from, to.as_mut_ptr(), 4096);
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
                ttb = in(reg) to_phys,
                options(nostack),
            );
        }
    }

    /// The MiBs whose first-level entries [`hot_code_on_sram`] may replace.
    const HOT_MIBS: usize = 4;
    /// Their second-level tables, 1 KiB each and 1 KiB aligned
    /// (identity-mapped SDRAM, as the MMU walks them).
    #[repr(C, align(1024))]
    struct HotTables([u32; 256 * HOT_MIBS]);
    static mut HOT_L2: HotTables = HotTables([0; 256 * HOT_MIBS]);
    /// What is mapped now: the SRAM's address (0 none yet), the ranges,
    /// and each replaced entry's index and the section it held.
    struct Hot {
        sram: u32,
        ranges: [(u32, u32); HOT_MIBS],
        n: usize,
        saved: [(usize, u32); HOT_MIBS],
        mibs: usize,
        mapped: bool,
    }
    static mut HOT: Hot = Hot {
        sram: 0,
        ranges: [(0, 0); HOT_MIBS],
        n: 0,
        saved: [(0, 0); HOT_MIBS],
        mibs: 0,
        mapped: false,
    };

    /// Puts code onto the 256 KiB internal SRAM without moving it (ADR-0040
    /// step 7b): each of `ranges` (identity-mapped SDRAM, address and
    /// length whole 64 KiB pages, at most 256 KiB in all, at most four
    /// MiBs touched) is copied to the SRAM, and every MiB holding one is
    /// mapped as 64 KiB pages, those ranges' pages onto the SRAM and the
    /// rest onto the same SDRAM as its section did. The same addresses then
    /// run the same bytes from the SRAM. Takes the SRAM the first time
    /// ([`take_sram`]). Call with interrupts masked, under the bring-up map,
    /// with no code of the ranges running.
    ///
    /// # Errors
    /// A range is not whole 64 KiB pages of SDRAM, the ranges come to more
    /// than the SRAM or touch more than four MiBs, the kernel map is live,
    /// the SRAM was taken by something else, or the code is mapped already.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
    pub fn hot_code_on_sram(ranges: &[(u32, u32)]) -> Result<(u32, usize), &'static str> {
        const MIB: u32 = 1 << 20;
        const PAGE: u32 = 0x1_0000;
        // SAFETY: single core, interrupts masked by the caller: nothing
        // else touches HOT (only here and in `hot_code_off_sram`).
        let hot = unsafe { &mut *core::ptr::addr_of_mut!(HOT) };
        if hot.mapped {
            return Err("mapped already");
        }
        if crate::devmap::kernel_map() {
            return Err("the kernel map is live");
        }
        if ranges.is_empty() || ranges.len() > HOT_MIBS {
            return Err("no ranges, or more than four");
        }
        let mut total = 0u32;
        for &(at, len) in ranges {
            if len == 0
                || !at.is_multiple_of(PAGE)
                || !len.is_multiple_of(PAGE)
                || !crate::mmu::SDRAM.contains(&at)
                || at
                    .checked_add(len)
                    .is_none_or(|e| e > crate::mmu::SDRAM.end)
            {
                return Err("a range is not whole 64 KiB pages of SDRAM");
            }
            total += len;
        }
        if total as usize > SRAM_SIZE {
            return Err("more than the SRAM holds");
        }
        let mut mibs = [0u32; HOT_MIBS];
        let mut m = 0;
        for &(at, len) in ranges {
            let mut mib = at & !(MIB - 1);
            while mib < at + len {
                if !mibs[..m].contains(&mib) {
                    if m == HOT_MIBS {
                        return Err("more than four MiBs touched");
                    }
                    mibs[m] = mib;
                    m += 1;
                }
                mib += MIB;
            }
        }
        if hot.sram == 0 {
            let Some(sram) = take_sram() else {
                return Err("the SRAM is taken");
            };
            hot.sram = sram.as_mut_ptr() as u32;
        }
        let base = hot.sram;
        let mut off = 0;
        for &(at, len) in ranges {
            // SAFETY: `at..at + len` is identity-mapped SDRAM (checked);
            // `base + off..` is inside the SRAM this module took, and the
            // ranges fit it (checked).
            unsafe {
                core::ptr::copy_nonoverlapping(
                    at as *const u32,
                    (base + off) as *mut u32,
                    (len / 4) as usize,
                );
            }
            off += len;
        }
        crate::cpu::clean_range(base, total);
        let on_sram = |va: u32| -> Option<u32> {
            let mut off = 0;
            for &(at, len) in ranges {
                if (at..at + len).contains(&va) {
                    return Some(base + off + (va - at));
                }
                off += len;
            }
            None
        };
        let ttb: u32;
        // SAFETY: reading TTB has no side effects.
        unsafe {
            core::arch::asm!("mrc p15, 0, {}, c2, c0, 0", out(reg) ttb, options(nostack, nomem))
        };
        let l1 = (ttb & !0x3FFF) as *mut u32;
        // SAFETY: as HOT above.
        let tables = unsafe { &mut *core::ptr::addr_of_mut!(HOT_L2) };
        for (i, &mib) in mibs[..m].iter().enumerate() {
            let t = &mut tables.0[256 * i..256 * (i + 1)];
            for page in 0..16u32 {
                let va = mib + page * PAGE;
                // read/write for kernel and user, as the bring-up sections
                let d = crate::mmu::large_page(on_sram(va).unwrap_or(va), 0b11);
                t[16 * page as usize..16 * (page as usize + 1)].fill(d);
            }
            let t_at = t.as_ptr() as u32;
            crate::cpu::clean_range(t_at, 1024);
            let index = (mib >> 20) as usize;
            // SAFETY: the live first-level table, identity-mapped (bring-up
            // map, checked); this MiB's entry, a section of SDRAM, replaced
            // by a table mapping the same addresses to the same bytes.
            unsafe {
                hot.saved[i] = (index, l1.add(index).read_volatile());
                l1.add(index).write_volatile(crate::mmu::coarse(t_at));
            }
            crate::cpu::clean_range(l1 as u32 + 4 * index as u32, 4);
        }
        crate::cpu::invalidate_tlb_all();
        for &(at, len) in ranges {
            crate::cpu::clean_invalidate_range(at, len);
        }
        hot.ranges[..ranges.len()].copy_from_slice(ranges);
        hot.n = ranges.len();
        hot.mibs = m;
        hot.mapped = true;
        Ok((total, m))
    }

    /// Takes the code [`hot_code_on_sram`] put on the SRAM off it again: what
    /// was written to its addresses meanwhile (a module loaded over it) goes
    /// to the SRAM first, the sections come back, and the SRAM's bytes are
    /// copied back to the SDRAM, so the SDRAM holds the code as it is now.
    /// False if nothing was mapped. Same conditions as mapping.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
    pub fn hot_code_off_sram() -> bool {
        // SAFETY: as in `hot_code_on_sram`.
        let hot = unsafe { &mut *core::ptr::addr_of_mut!(HOT) };
        if !hot.mapped {
            return false;
        }
        for &(at, len) in &hot.ranges[..hot.n] {
            crate::cpu::clean_invalidate_range(at, len);
        }
        let ttb: u32;
        // SAFETY: reading TTB has no side effects.
        unsafe {
            core::arch::asm!("mrc p15, 0, {}, c2, c0, 0", out(reg) ttb, options(nostack, nomem))
        };
        let l1 = (ttb & !0x3FFF) as *mut u32;
        for &(index, section) in &hot.saved[..hot.mibs] {
            // SAFETY: the entries `hot_code_on_sram` replaced, given back.
            unsafe { l1.add(index).write_volatile(section) };
            crate::cpu::clean_range(l1 as u32 + 4 * index as u32, 4);
        }
        crate::cpu::invalidate_tlb_all();
        let mut off = 0;
        for &(at, len) in &hot.ranges[..hot.n] {
            // SAFETY: as in `hot_code_on_sram`, the other way.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    (hot.sram + off) as *const u32,
                    at as *mut u32,
                    (len / 4) as usize,
                );
            }
            crate::cpu::clean_invalidate_range(at, len);
            off += len;
        }
        hot.mapped = false;
        true
    }

    /// Maps `backing` (identity-mapped SDRAM, 1 MiB aligned, a whole number
    /// of MiB, at most 16) a second time at `virt` (1 MiB aligned, in the
    /// native slots `0xC0000000`-`0xFDFFFFFF`, mapping nothing yet) as
    /// `pages`, its second-level tables in `l2` (1 KiB aligned,
    /// identity-mapped: SDRAM or the internal SRAM; unused for sections),
    /// and runs `f` on the window; then the window is written back and
    /// unmapped. The CPU gate's C4 and V7 (the random hop by page size and
    /// table placement). `backing`'s own address isn't used meanwhile (it
    /// is borrowed), so the VIVT cache never holds the two aliases at once.
    /// False if refused (nothing mapped).
    #[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
    pub fn with_window(
        virt: u32,
        backing: &mut [u8],
        pages: crate::mmu::Pages,
        l2: &mut [u8],
        f: &mut dyn FnMut(&mut [u8]),
    ) -> bool {
        const MIB: u32 = 1 << 20;
        let (phys, len) = (backing.as_ptr() as u32, backing.len() as u32);
        let mib = len / MIB;
        let fits = len.is_multiple_of(MIB)
            && (1..=16).contains(&mib)
            && phys.is_multiple_of(MIB)
            && crate::mmu::SDRAM.contains(&phys)
            && phys + len <= crate::mmu::SDRAM.end
            && virt.is_multiple_of(MIB)
            && virt >= 0xC000_0000
            && virt + len <= 0xFE00_0000
            && (pages == crate::mmu::Pages::Sections
                || pages == crate::mmu::Pages::MiniSections
                || (l2.len() >= 1024 * mib as usize
                    && (l2.as_ptr() as usize).is_multiple_of(1024)));
        // Physical addresses are virtual ones only under the bring-up map.
        if !fits || crate::devmap::kernel_map() {
            return false;
        }
        let ttb: u32;
        // SAFETY: reading TTB has no side effects.
        unsafe {
            core::arch::asm!("mrc p15, 0, {}, c2, c0, 0", out(reg) ttb, options(nostack, nomem))
        };
        let l1 = (ttb & !0x3FFF) as *mut u32;
        let first = (virt >> 20) as usize;
        // SAFETY: the live first-level table, identity-mapped (bring-up
        // map, checked); reads of the window's entries.
        if (0..mib as usize).any(|i| unsafe { l1.add(first + i).read_volatile() } != 0) {
            return false;
        }
        // Nothing of `backing` left in the cache under its own address.
        crate::cpu::clean_invalidate_range(phys, len);
        let l2_phys = l2.as_ptr() as u32;
        // SAFETY: `l2` is caller-owned, 1 KiB aligned (so word aligned) and
        // at least 1 KiB a MiB long (checked); reborrowed as words for this
        // call only.
        let words = unsafe {
            core::slice::from_raw_parts_mut(l2.as_mut_ptr().cast::<u32>(), 256 * mib as usize)
        };
        crate::mmu::window_tables(words, phys, mib, pages);
        crate::cpu::clean_range(l2_phys, 1024 * mib);
        for i in 0..mib {
            let d = match pages {
                crate::mmu::Pages::Sections => {
                    crate::mmu::section(phys + i * MIB, crate::mmu::Memory::Normal)
                }
                crate::mmu::Pages::MiniSections => crate::mmu::mini_section(phys + i * MIB),
                _ => crate::mmu::coarse(l2_phys + i * 1024),
            };
            // SAFETY: entries for the window's unmapped MiBs (checked
            // above), in the live table; nothing running uses them.
            unsafe { l1.add(first + i as usize).write_volatile(d) };
        }
        crate::cpu::clean_range(l1 as u32 + 4 * first as u32, 4 * mib);
        crate::cpu::invalidate_tlb_all();
        // SAFETY: `virt..virt + len` now maps `backing`'s frames, which are
        // borrowed mutably for this call and not touched through their own
        // address until it returns; the slice lives only inside `f`.
        let window = unsafe { core::slice::from_raw_parts_mut(virt as *mut u8, len as usize) };
        f(window);
        crate::cpu::clean_invalidate_range(virt, len);
        for i in 0..mib as usize {
            // SAFETY: the entries written above, back to faulting.
            unsafe { l1.add(first + i).write_volatile(0) };
        }
        crate::cpu::clean_range(l1 as u32 + 4 * first as u32, 4 * mib);
        crate::cpu::invalidate_tlb_all();
        true
    }

    /// Changes the memory type of `len` bytes from `virt` (both 1 MiB
    /// aligned), already mapped by the bring-up map as identity sections,
    /// to `memory`: their first-level entries rewritten in place, the
    /// range written back and dropped from the D-cache first, the TLBs
    /// invalidated after. Returns the type the first section had (to put
    /// back), or `None`, changing nothing, if a section isn't an identity
    /// section or the kernel's map is in use. Test G16: the 2700G's local
    /// memory as Windows Mobile maps it for its probe (uncached and
    /// unbuffered) against the bring-up map's write-combining.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
    pub fn set_section_memory(
        virt: u32,
        len: u32,
        memory: crate::mmu::Memory,
    ) -> Option<crate::mmu::Memory> {
        const MIB: u32 = 1 << 20;
        if !virt.is_multiple_of(MIB)
            || !len.is_multiple_of(MIB)
            || len == 0
            || crate::devmap::kernel_map()
        {
            return None;
        }
        let ttb: u32;
        // SAFETY: reading TTB has no side effects.
        unsafe {
            core::arch::asm!("mrc p15, 0, {}, c2, c0, 0", out(reg) ttb, options(nostack, nomem))
        };
        let l1 = (ttb & !0x3FFF) as *mut u32;
        let (first, count) = ((virt >> 20) as usize, (len / MIB) as usize);
        // SAFETY: the live first-level table, identity-mapped under the
        // bring-up map (checked); reads of its entries.
        let entry = |i: usize| unsafe { l1.add(first + i).read_volatile() };
        let identity = (0..count).all(|i| {
            let d = entry(i);
            d & 3 == 0b10 && d & 0xFFF0_0000 == virt + i as u32 * MIB
        });
        if !identity {
            return None;
        }
        let was = match (entry(0) >> 2) & 3 {
            0b11 => crate::mmu::Memory::Normal,
            0b01 => crate::mmu::Memory::WriteCombine,
            _ => crate::mmu::Memory::Device,
        };
        crate::cpu::clean_invalidate_range(virt, len);
        crate::cpu::drain_write_buffer();
        for i in 0..count {
            let d = crate::mmu::section(virt + i as u32 * MIB, memory);
            // SAFETY: an identity section of the live table (checked)
            // rewritten to the same frames with another cache type; nothing
            // of the range is in the cache (written back above).
            unsafe { l1.add(first + i).write_volatile(d) };
        }
        crate::cpu::clean_range(l1 as u32 + 4 * first as u32, 4 * count as u32);
        crate::cpu::invalidate_tlb_all();
        Some(was)
    }

    /// Moves the live first-level page table to `to` (16 KiB aligned, 16 KiB
    /// of memory owned by the caller, e.g. from [`take_sram`]) and makes the
    /// MMU use it: the ADR-0006 rule 1 experiment. The table's contents are
    /// unchanged, so every mapping stays the same.
    ///
    /// # Panics
    /// If `to` isn't 16 KiB aligned and 16 KiB long.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
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
pub use target::{
    CoSurvey, coprocessor_survey, coprocessors, hot_code_off_sram, hot_code_on_sram, move_l1_table,
    move_l1_table_at, msc1, msc2, put_msc1, set_cs2_half, set_cs3_sram, set_cs3_timing,
    set_cs3_vlio, set_cs4_vlio, set_section_memory, snapshot, take_sram, take_sram_words,
    with_device_window, with_window,
};

#[cfg(test)]
mod tests {
    /// 16 word-aligned bytes: three words and a spare.
    #[repr(align(4))]
    struct Aligned([u8; 16]);

    fn aligned() -> Aligned {
        let mut a = Aligned([0; 16]);
        for (i, w) in [0x1122_3344u32, 0xA5A5_0001, 0xDEAD_BEEF]
            .iter()
            .enumerate()
        {
            a.0[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        a
    }

    #[test]
    fn read_word_reads_aligned_words() {
        let a = aligned();
        assert_eq!(read_word(&a.0, 0), 0x1122_3344);
        assert_eq!(read_word(&a.0, 4), 0xA5A5_0001);
        assert_eq!(read_word(&a.0, 8), 0xDEAD_BEEF);
        assert_eq!(read_word(&a.0, 12), 0);
    }

    #[test]
    #[should_panic(expected = "unaligned")]
    fn read_word_refuses_unaligned() {
        let _ = read_word(&aligned().0, 2);
    }

    #[test]
    #[should_panic]
    fn read_word_refuses_past_the_end() {
        let _ = read_word(&aligned().0, 16);
    }

    #[test]
    #[should_panic]
    fn read_word_refuses_straddling_the_end() {
        let _ = read_word(&aligned().0, 13);
    }

    use super::*;

    #[test]
    fn cs2s_half_moves_only_its_timing() {
        // Dell's two values on the X51v (rp-wmprobe v5): Windows Mobile's
        // clock and 74.75 / 1; CS3's half kept either way.
        assert_eq!(cs2_half(0x2484_2481, 0x7DF1), Some(0x2484_7DF1));
        assert_eq!(cs2_half(0x79D4_7DF1, 0x2481), Some(0x79D4_2481));
        assert_eq!(cs2_half(0x79D4_2481, 0x2481), Some(0x79D4_2481));
        // Its type or width changed, or a bit of CS3's: refused.
        assert_eq!(cs2_half(0x2484_2481, 0x7DF0), None);
        assert_eq!(cs2_half(0x2484_2481, 0x7DF9), None);
        assert_eq!(cs2_half(0x2484_2481, 0x1_7DF1), None);
    }

    #[test]
    fn cs3_timing_changes_only_the_strobes() {
        // Windows Mobile's MSC1 (probe 2026-10-11 and 2026-10-15).
        let wm = 0x2484_2481;
        assert_eq!(cs3_timing(wm, 8, 4), Some(wm));
        assert_eq!(cs3_timing(wm, 3, 2), Some(0x2234_2481));
        assert_eq!(cs3_timing(wm, 5, 3), Some(0x2354_2481));
        // Below the PXA's VLIO minimums, or table-encoded values: refused.
        assert_eq!(cs3_timing(wm, 2, 4), None);
        assert_eq!(cs3_timing(wm, 8, 1), None);
        assert_eq!(cs3_timing(wm, 12, 4), None);
        assert_eq!(cs3_timing(wm, 8, 12), None);
        // CS3 not a 32-bit VLIO device (SRAM type, or 16 bits): refused.
        assert_eq!(cs3_timing(0x2481_2481, 8, 4), None);
        assert_eq!(cs3_timing(0x248C_2481, 8, 4), None);
    }

    #[test]
    fn cs4_vlio_changes_only_chip_select_4s_timing() {
        // Windows Mobile's MSC2 (probe 2026-10-11): CS4 0x7FFC, the slowest.
        let wm = 0x7FF4_7FFC;
        assert_eq!(cs4_vlio(wm, 15, 15, 7), Some(wm));
        assert_eq!(cs4_vlio(wm, 3, 2, 0), Some(0x7FF4_023C));
        assert_eq!(cs4_vlio(wm, 11, 15, 7), Some(0x7FF4_7FBC));
        // Range edges, both sides.
        assert_eq!(cs4_vlio(wm, 2, 15, 7), None);
        assert_eq!(cs4_vlio(wm, 16, 15, 7), None);
        assert_eq!(cs4_vlio(wm, 15, 1, 7), None);
        assert_eq!(cs4_vlio(wm, 15, 16, 7), None);
        assert_eq!(cs4_vlio(wm, 15, 15, 8), None);
        // Not a 16-bit VLIO chip select: nothing changes.
        assert_eq!(cs4_vlio(0x7FF4_7FF4, 15, 15, 7), None);
        assert_eq!(cs4_vlio(0x7FF4_7FF8, 15, 15, 7), None);
        // CS5's half and RBUFF4 kept.
        assert_eq!(cs4_vlio(0x1234_FFFC, 3, 2, 0), Some(0x1234_823C));
    }

    #[test]
    fn cs3_vlio_sets_all_three_fields() {
        let wm = 0x2484_2481;
        // Dell's long timing (0x79D4: RDF code 13, RDN 9, RRR 7) and back.
        assert_eq!(cs3_vlio(wm, 13, 9, 7), Some(0x79D4_2481));
        assert_eq!(cs3_vlio(0x79D4_2481, 8, 4, 2), Some(wm));
        assert_eq!(cs3_vlio(wm, 3, 2, 0), Some(0x0234_2481));
        assert_eq!(cs3_vlio(wm, 15, 15, 7), Some(0x7FF4_2481));
        assert_eq!(cs3_vlio(wm, 2, 4, 2), None);
        assert_eq!(cs3_vlio(wm, 8, 1, 2), None);
        assert_eq!(cs3_vlio(wm, 16, 4, 2), None);
        assert_eq!(cs3_vlio(wm, 8, 4, 8), None);
        assert_eq!(cs3_vlio(0x2481_2481, 8, 4, 2), None);
        assert_eq!(
            vlio_clocks(13) + 1 + 2 * vlio_clocks(9),
            39,
            "0x79D4: 39 clocks a beat (memory-timing.md)"
        );
    }

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
