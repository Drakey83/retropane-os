//! Test image: the 2700G's bus timing (item 68, docs/research/memory-timing.md
//! §7, the owner's go of 2026-10-15). Windows Mobile sets the 2700G's chip
//! select (CS3, the upper half of `MSC1`) to 17 memory clocks per 32-bit
//! beat; the PXA27x allows 8. This image steps CS3's strobe timing from
//! Windows Mobile's value toward that minimum ([`STEPS`]), and at each step:
//!
//! - times a full-screen flush (1.2 MB, SDRAM to 2700G memory, the OS's
//!   `memcpy`) and checks every word of it;
//! - times reading 256 KiB back;
//! - writes and reads back six patterns over 512 KiB ([`Pattern`]);
//! - writes and reads back a 2700G register (`BGCOLOR`);
//! - has the 2D engine fill 480 x 64 pixels (its command words cross the
//!   same bus) and checks every word of the fill.
//!
//! After the steps, a 60 s soak at the step **two looser** than the
//! tightest that passed (the owner's rule, 2026-10-19: any setting must
//! hold on every X51v, not this one only), repeating the flush, the fill
//! and a pattern; any miss ends it.
//!
//! It stops at the first mismatch, then re-reads the failing word at
//! Windows Mobile's timing to tell a write fault from a read fault. Windows
//! Mobile's `MSC1` goes back at the end, and from the fault handler (the
//! soft watchdog's FIQ lands there too). Only CS3's two strobe fields
//! change (`diag::set_cs3_timing` refuses anything else); the SDRAM timings
//! and CS3's recovery time are never touched. It refuses to start unless
//! `MSC1` is exactly Windows Mobile's. 2700G memory used: the run-time pool
//! only (drivers/vram-map), away from the scanned planes and the 3D blocks.
//! Writes nothing to flash or the cards.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// The last run-log line of a complete run.
pub const MARKER: &str = "RETROPANE-CS3BUS-DONE";

/// `MSC1` as Windows Mobile leaves it (probes 2026-10-11 and 2026-10-15).
pub const WM_MSC1: u32 = 0x2484_2481;

/// The steps, in order: CS3's `RDF` (strobe asserted for `RDF` + 1 memory
/// clocks) then `RDN` (released for 2 x `RDN`). Windows Mobile's first, as
/// the baseline; one field and one clock at a time; the last is the PXA's
/// VLIO minimum.
pub const STEPS: [(u32, u32); 8] = [
    (8, 4),
    (7, 4),
    (6, 4),
    (5, 4),
    (4, 4),
    (3, 4),
    (3, 3),
    (3, 2),
];

/// Memory clocks per 32-bit beat at a step, without RDY waits.
#[must_use]
pub const fn clocks_per_beat(rdf: u32, rdn: u32) -> u32 {
    rdf + 1 + 2 * rdn
}

/// A test pattern: the word each index gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    /// The word's own byte address (catches address aliasing).
    Address,
    /// Its complement.
    Inverse,
    /// Alternating `0xAAAAAAAA` / `0x55555555` (neighbouring data lines).
    Checker,
    /// One bit set, walking.
    WalkOne,
    /// One bit clear, walking.
    WalkZero,
    /// Hashed noise (every bit pattern, data lines switching hard).
    Random,
}

impl Pattern {
    /// Every pattern, in the order run.
    pub const ALL: [Self; 6] = [
        Self::Address,
        Self::Inverse,
        Self::Checker,
        Self::WalkOne,
        Self::WalkZero,
        Self::Random,
    ];

    /// Its name in the log.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Address => "address",
            Self::Inverse => "inverse",
            Self::Checker => "checker",
            Self::WalkOne => "walk1",
            Self::WalkZero => "walk0",
            Self::Random => "random",
        }
    }

    /// The word for index `i` of a region at byte address `base`.
    #[must_use]
    pub const fn word(self, base: u32, i: u32) -> u32 {
        match self {
            Self::Address => base.wrapping_add(i.wrapping_mul(4)),
            Self::Inverse => !base.wrapping_add(i.wrapping_mul(4)),
            Self::Checker => {
                if i & 1 == 0 {
                    0xAAAA_AAAA
                } else {
                    0x5555_5555
                }
            }
            Self::WalkOne => 1 << (i % 32),
            Self::WalkZero => !(1 << (i % 32)),
            Self::Random => mix(i),
        }
    }
}

/// A stateless 32-bit hash (finaliser of MurmurHash3), so any word can be
/// recomputed without replaying a generator.
#[must_use]
pub const fn mix(i: u32) -> u32 {
    let mut x = i.wrapping_mul(0x9E37_79B9) ^ 0x5EED_C0DE;
    x ^= x >> 16;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    x = x.wrapping_mul(0xC2B2_AE35);
    x ^ (x >> 16)
}

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use super::{MARKER, Pattern, STEPS, WM_MSC1, clocks_per_beat, mix};
    use rp_drv_blit2d::{self as cmd, Engine, Format};
    use rp_hal_api::RegisterBus;
    use rp_hal_mem::{load32_volatile, store32_volatile};
    use rp_hal_pxa27x::{cpu, diag, rt};
    use rp_hal_x51v::devices::RunLog;
    use rp_vram_map::bench32::{POOL, SCREEN_BYTES};

    use rp_image::crash;

    rp_image::entry!(main);

    /// `BGCOLOR`: a plain read/write 2700G register, hidden behind the
    /// full-screen graphics plane.
    const BGCOLOR: usize = 0x2174;
    /// The flush region: one screen picture at the pool's start.
    const FLUSH_AT: usize = POOL.at as usize;
    const FLUSH_BYTES: usize = SCREEN_BYTES as usize;
    /// The pattern region, past the flush region, inside the pool.
    const PATTERN_AT: usize = POOL.at as usize + 0x14_0000;
    const PATTERN_BYTES: usize = 512 * 1024;
    const READ_BYTES: usize = 256 * 1024;
    /// The 2D fill's region, past the patterns: 480 x 64 at 32 bits.
    const FILL_AT: usize = PATTERN_AT + PATTERN_BYTES;
    const FILL_STRIDE: usize = 480 * 4;
    const FILL_ROWS: usize = 64;
    /// The 2D engine's fence word (the memory map's).
    const FENCE_AT: usize = rp_vram_map::bench32::FENCE.at as usize;
    /// How long the soak runs.
    const SOAK_US: u32 = 60_000_000;
    const _: () = assert!(FILL_AT + FILL_STRIDE * FILL_ROWS <= POOL.end() as usize);
    const _: () = assert!(PATTERN_AT + PATTERN_BYTES <= POOL.end() as usize);
    const _: () = assert!(FLUSH_AT + FLUSH_BYTES <= PATTERN_AT);

    /// Puts Windows Mobile's CS3 timing back if it isn't in effect. The
    /// image only runs when `MSC1` started as Windows Mobile's, so that is
    /// always the value to restore (no state to keep, so the fault handler
    /// can call it whatever failed).
    fn restore() {
        if diag::msc1() != WM_MSC1 {
            let _ = diag::set_cs3_timing(WM_MSC1 >> 20 & 0xF, WM_MSC1 >> 24 & 0xF);
        }
    }

    fn us(ticks: u32) -> u32 {
        ticks / 13 * 4
    }

    /// The first mismatch: word index, expected, read.
    type Miss = Option<(usize, u32, u32)>;

    fn note(miss: &mut Miss, count: &mut u32, k: usize, want: u32, got: Option<u32>) {
        let got = got.unwrap_or(!want);
        if got != want {
            *count += 1;
            if miss.is_none() {
                *miss = Some((k, want, got));
            }
        }
    }

    /// The 2D engine fills the fill region with `colour` (and waits); the
    /// words that came out wrong, read back at the current timing.
    fn fill_check(e: &mut Engine<rp_hal_pxa27x::Mmio>, vram: &mut [u8], colour: u32) -> u32 {
        let words = rp_image::need(
            cmd::fill(
                Format::BPP32,
                FILL_AT,
                FILL_STRIDE,
                (0, 0, 480, FILL_ROWS as u16),
                colour,
            ),
            "a 2D rectangle",
        );
        if e.submit(&words)
            .and_then(|()| e.finish(vram, 200_000))
            .is_err()
        {
            return u32::MAX;
        }
        let mut bad = 0;
        for k in 0..FILL_STRIDE * FILL_ROWS / 4 {
            if load32_volatile(vram, FILL_AT + 4 * k) != Some(colour) {
                bad += 1;
            }
        }
        bad
    }

    fn main() -> ! {
        // A crash puts MSC1 back first (rp-image's crash order).
        rp_image::on_crash(restore);
        rt::init_mmu();
        let Some((regs, vram)) = rp_image::take_mbx() else {
            crash(format_args!("no 2700G"))
        };
        let _ = store32_volatile(vram, FENCE_AT, 0);
        cpu::drain_write_buffer();
        let mut e = Engine::new(regs, FENCE_AT, rt::ticks, 3_250);
        let Some(ram) = rp_hal_x51v::devices::take_scratch() else {
            crash(format_args!("no scratch RAM"))
        };
        let found = diag::msc1();
        let _ = writeln!(
            RunLog,
            "cs3bus: MSC1 {found:08X} (Windows Mobile's {WM_MSC1:08X})"
        );
        if found != WM_MSC1 {
            let _ = writeln!(RunLog, "MSC1 not as expected: nothing changed");
            let _ = writeln!(RunLog, "{MARKER}");
            finish();
        }

        // The flush source: noise in SDRAM.
        let (src, rest) = ram.split_at_mut(FLUSH_BYTES);
        let back = &mut rest[..READ_BYTES];
        for (k, w) in src.chunks_exact_mut(4).enumerate() {
            w.copy_from_slice(&mix(k as u32 ^ 0xF1A5_0000).to_le_bytes());
        }
        // Which BGCOLOR bits hold a value, at Windows Mobile's timing.
        let bg_found = e.regs.read32(BGCOLOR);
        e.regs.write32(BGCOLOR, 0xFFFF_FFFF);
        let bg_mask = e.regs.read32(BGCOLOR);
        e.regs.write32(BGCOLOR, bg_found);
        let _ = writeln!(RunLog, "BGCOLOR {bg_found:08X}, bits held {bg_mask:08X}");
        let _ = writeln!(
            RunLog,
            "step RDF RDN clk/beat MSC1 | flush us MB/s miss | read256K us MB/s | patterns miss | reg miss | 2D fill miss"
        );

        let mut verdict = "all steps passed";
        // The tightest step that passed everything.
        let mut passed: Option<usize> = None;
        for (step, &(rdf, rdn)) in STEPS.iter().enumerate() {
            rt::check_in();
            let Some(now) = diag::set_cs3_timing(rdf, rdn) else {
                verdict = "refused a step (outside the legal range)";
                break;
            };

            // Full-screen flush, best of three, then every word checked.
            let mut best = u32::MAX;
            for _ in 0..3 {
                let t = rt::ticks();
                vram[FLUSH_AT..FLUSH_AT + FLUSH_BYTES].copy_from_slice(src);
                cpu::drain_write_buffer();
                best = best.min(us(rt::ticks().wrapping_sub(t)));
                rt::check_in();
            }
            let (mut fmiss, mut fcount) = (None, 0);
            for (k, w) in src.chunks_exact(4).enumerate() {
                let want = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
                note(
                    &mut fmiss,
                    &mut fcount,
                    k,
                    want,
                    load32_volatile(vram, FLUSH_AT + 4 * k),
                );
            }
            rt::check_in();

            // Reading back: 256 KiB, 2700G memory to SDRAM.
            let t = rt::ticks();
            back.copy_from_slice(&vram[FLUSH_AT..FLUSH_AT + READ_BYTES]);
            let read_us = us(rt::ticks().wrapping_sub(t));
            for (k, w) in back.chunks_exact(4).enumerate() {
                let want = u32::from_le_bytes([
                    src[4 * k],
                    src[4 * k + 1],
                    src[4 * k + 2],
                    src[4 * k + 3],
                ]);
                let got = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
                note(&mut fmiss, &mut fcount, k, want, Some(got));
            }

            // Patterns over 512 KiB: written, then read back.
            let (mut pmiss, mut pcount, mut pname) = (None, 0, "");
            let base = 0x0C00_0000 + PATTERN_AT as u32;
            for p in Pattern::ALL {
                for k in 0..PATTERN_BYTES / 4 {
                    let _ = store32_volatile(vram, PATTERN_AT + 4 * k, p.word(base, k as u32));
                }
                cpu::drain_write_buffer();
                rt::check_in();
                let before = pcount;
                for k in 0..PATTERN_BYTES / 4 {
                    let got = load32_volatile(vram, PATTERN_AT + 4 * k);
                    note(&mut pmiss, &mut pcount, k, p.word(base, k as u32), got);
                }
                rt::check_in();
                if pcount > before {
                    // Stop here, so the region still holds this pattern
                    // for the re-read below.
                    pname = p.name();
                    break;
                }
            }

            // A register: written and read back.
            let mut rcount = 0;
            for w in [0x00A5_A5A5, 0x005A_5A5A, 0x00FF_FFFF, 0x0012_3456, 0] {
                e.regs.write32(BGCOLOR, w);
                if e.regs.read32(BGCOLOR) != w & bg_mask {
                    rcount += 1;
                }
            }
            e.regs.write32(BGCOLOR, bg_found);

            // The 2D engine: its commands cross this bus too.
            let dcount = fill_check(&mut e, vram, mix(step as u32 ^ 0x2D00));
            rt::check_in();

            let mbs = |bytes: usize, t: u32| bytes as u32 / t.max(1);
            let _ = writeln!(
                RunLog,
                "{step} {rdf} {rdn} {} {now:08X} | {best} {} {fcount} | {read_us} {} | {pcount} | {rcount} | {dcount}",
                clocks_per_beat(rdf, rdn),
                mbs(FLUSH_BYTES, best),
                mbs(READ_BYTES, read_us),
            );

            if fcount + pcount + rcount > 0 || dcount != 0 {
                // Write fault or read fault? Re-read the first bad word at
                // Windows Mobile's timing.
                restore();
                if let Some((k, want, got)) = fmiss {
                    let again = load32_volatile(vram, FLUSH_AT + 4 * k).unwrap_or(0);
                    let _ = writeln!(
                        RunLog,
                        "flush miss: word {k} wanted {want:08X} read {got:08X}, at WM timing {again:08X}"
                    );
                }
                if let Some((k, want, got)) = pmiss {
                    let again = load32_volatile(vram, PATTERN_AT + 4 * k).unwrap_or(0);
                    let _ = writeln!(
                        RunLog,
                        "pattern {pname} miss: word {k} wanted {want:08X} read {got:08X}, at WM timing {again:08X}"
                    );
                }
                if dcount != 0 {
                    let _ = writeln!(
                        RunLog,
                        "2D fill: {dcount} words wrong (u32::MAX: the engine never finished)"
                    );
                }
                verdict = "stopped at the first mismatch";
                break;
            }
            passed = Some(step);
        }
        restore();

        // The soak: two steps looser than the tightest that passed (step 0
        // is Windows Mobile's own, so a candidate needs step 3 or tighter
        // to have passed).
        match passed.and_then(|t| t.checked_sub(2)).filter(|&c| c >= 1) {
            None => {
                let _ = writeln!(
                    RunLog,
                    "soak: no step two looser than the tightest pass is faster than WM's: none run"
                );
            }
            Some(c) => {
                let (rdf, rdn) = STEPS[c];
                let _ = diag::set_cs3_timing(rdf, rdn);
                let t0 = rt::ticks();
                let (mut rounds, mut misses) = (0u32, 0u32);
                while us(rt::ticks().wrapping_sub(t0)) < SOAK_US && misses == 0 {
                    rt::check_in();
                    vram[FLUSH_AT..FLUSH_AT + FLUSH_BYTES].copy_from_slice(src);
                    cpu::drain_write_buffer();
                    for (k, w) in src.chunks_exact(4).enumerate() {
                        let want = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
                        if load32_volatile(vram, FLUSH_AT + 4 * k) != Some(want) {
                            misses += 1;
                        }
                    }
                    let d = fill_check(&mut e, vram, mix(rounds ^ 0x50AC));
                    misses = misses.saturating_add(d);
                    let base = 0x0C00_0000 + PATTERN_AT as u32 + rounds;
                    for k in 0..PATTERN_BYTES / 4 {
                        let _ = store32_volatile(
                            vram,
                            PATTERN_AT + 4 * k,
                            Pattern::Random.word(base, k as u32),
                        );
                    }
                    cpu::drain_write_buffer();
                    for k in 0..PATTERN_BYTES / 4 {
                        if load32_volatile(vram, PATTERN_AT + 4 * k)
                            != Some(Pattern::Random.word(base, k as u32))
                        {
                            misses += 1;
                        }
                    }
                    rounds += 1;
                }
                restore();
                let _ = writeln!(
                    RunLog,
                    "soak at step {c} (RDF {rdf} RDN {rdn}, {} clk/beat): {rounds} rounds in {} s, {misses} misses",
                    clocks_per_beat(rdf, rdn),
                    us(rt::ticks().wrapping_sub(t0)) / 1_000_000
                );
            }
        }

        // Back at Windows Mobile's timing: one pattern pass, as a health check.
        let after = diag::msc1();
        let base = 0x0C00_0000 + PATTERN_AT as u32;
        for k in 0..PATTERN_BYTES / 4 {
            let _ = store32_volatile(
                vram,
                PATTERN_AT + 4 * k,
                Pattern::Random.word(base, k as u32),
            );
        }
        cpu::drain_write_buffer();
        let (mut miss, mut count) = (None, 0);
        for k in 0..PATTERN_BYTES / 4 {
            let got = load32_volatile(vram, PATTERN_AT + 4 * k);
            note(
                &mut miss,
                &mut count,
                k,
                Pattern::Random.word(base, k as u32),
                got,
            );
        }
        let _ = writeln!(
            RunLog,
            "{verdict}; MSC1 restored {after:08X}, check at WM timing: {count} misses"
        );
        let _ = writeln!(RunLog, "{MARKER}");
        finish();
    }

    fn finish() -> ! {
        rp_image::finish()
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_start_at_windows_mobile_and_tighten_one_field_at_a_time() {
        assert_eq!(STEPS[0], (WM_MSC1 >> 20 & 0xF, WM_MSC1 >> 24 & 0xF));
        for w in STEPS.windows(2) {
            let ((f0, n0), (f1, n1)) = (w[0], w[1]);
            let one_down = (f1 + 1 == f0 && n1 == n0) || (f1 == f0 && n1 + 1 == n0);
            assert!(one_down, "{w:?}");
        }
        // Inside the PXA27x's VLIO range: RDF >= 3, RDN >= 2.
        assert!(STEPS.iter().all(|&(f, n)| f >= 3 && n >= 2));
        assert_eq!(clocks_per_beat(8, 4), 17);
        assert_eq!(clocks_per_beat(3, 2), 8);
    }

    #[test]
    fn patterns_differ_between_neighbours() {
        for p in Pattern::ALL {
            let a = p.word(0x0CA8_0000, 0);
            let b = p.word(0x0CA8_0000, 1);
            assert_ne!(a, b, "{}", p.name());
        }
        // Random hits every bit both ways over a few words.
        let (mut or, mut and) = (0u32, !0u32);
        for i in 0..64 {
            or |= mix(i);
            and &= mix(i);
        }
        assert_eq!((or, and), (!0, 0));
    }
}
