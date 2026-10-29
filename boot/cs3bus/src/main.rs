//! Test image: the 2700G's bus timing (item 68, docs/research/memory-timing.md
//! §7, the owner's go of 2026-10-15). Windows Mobile sets the 2700G's chip
//! select (CS3, the upper half of `MSC1`) to 17 memory clocks per 32-bit
//! beat and a 5-clock gap after it; the PXA27x allows 8 and 1. Run 1
//! (2026-10-19) stepped one strobe field at a time and stopped at the first
//! miss; this image (run 2, the give-up audit's item 5, reminder R3) tries
//! **every combination** of CS3's three timing fields from Windows Mobile's
//! down to the VLIO minimum ([`GRID`]: `RDF`, `RDN` and the recovery `RRR`,
//! which run 1 never touched), each point set from Windows Mobile's timing
//! and followed by a control there, a miss recorded and the next point run.
//! At each point:
//!
//! - times a full-screen flush (1.2 MB, SDRAM to 2700G memory, the OS's
//!   `memcpy`) and checks every word of it;
//! - times reading 256 KiB back;
//! - writes and reads back six patterns over 512 KiB ([`Pattern`]);
//! - writes and reads back a 2700G register (`BGCOLOR`);
//! - has the 2D engine fill 480 x 64 pixels (its command words cross the
//!   same bus) and checks every word of the fill.
//!
//! A point that misses has its first bad word re-read at Windows Mobile's
//! timing (a write fault from a read fault). After the grid, a 60 s soak
//! at the fastest point **with margin** ([`with_margin`]: every point up
//! to two codes tighter in each field passed too; the owner's rule,
//! 2026-10-19: any setting must hold on every X51v, not this one only),
//! repeating the flush, the fill and a pattern; any miss ends it.
//!
//! Windows Mobile's `MSC1` goes back after every point, at the end, and
//! from the fault handler (the soft watchdog's FIQ lands there too). Only
//! CS3's three timing fields change (`diag::set_cs3_vlio` refuses anything
//! else); the SDRAM timings are never touched. It refuses to start unless
//! `MSC1` is exactly Windows Mobile's. 2700G memory used: the run-time pool
//! only (drivers/vram-map), away from the scanned planes and the 3D blocks.
//! Writes nothing to flash or the cards. The screen says what runs and
//! which point it is on (written at Windows Mobile's timing, before each
//! point), from the first moment.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// The last run-log line of a complete run.
pub const MARKER: &str = "RETROPANE-CS3BUS-DONE";

/// `MSC1` as Windows Mobile leaves it (probes 2026-10-11 and 2026-10-15).
pub const WM_MSC1: u32 = 0x2484_2481;

/// CS3's timing fields as Windows Mobile leaves them: `RDF` (strobe
/// asserted for `RDF` + 1 memory clocks), `RDN` (released for 2 x `RDN`),
/// `RRR` (the chip select's gap after, 2 x `RRR` + 1).
pub const WM_CS3: (u32, u32, u32) = (
    WM_MSC1 >> 20 & 0xF,
    WM_MSC1 >> 24 & 0xF,
    WM_MSC1 >> 28 & 0x7,
);

/// The PXA27x's VLIO minimum for each field (`diag::cs3_vlio`'s range).
pub const VLIO_MIN: (u32, u32, u32) = (3, 2, 0);

/// Every point from Windows Mobile's timing down to the VLIO minimum, in
/// every field: 6 x 3 x 3. Windows Mobile's first (the baseline), then
/// `RRR` fastest-varying, `RDF` slowest, so the run's halves read apart.
pub const GRID: [(u32, u32, u32); 54] = grid();

const fn grid() -> [(u32, u32, u32); 54] {
    let mut g = [(0, 0, 0); 54];
    let mut k = 0;
    let mut rdf = WM_CS3.0;
    while rdf >= VLIO_MIN.0 {
        let mut rdn = WM_CS3.1;
        while rdn >= VLIO_MIN.1 {
            let mut rrr = WM_CS3.2 + 1;
            while rrr > VLIO_MIN.2 {
                rrr -= 1;
                g[k] = (rdf, rdn, rrr);
                k += 1;
            }
            rdn -= 1;
        }
        rdf -= 1;
    }
    assert!(k == 54);
    g
}

/// Memory clocks per 32-bit beat at a point, without RDY waits.
#[must_use]
pub const fn clocks_per_beat(rdf: u32, rdn: u32) -> u32 {
    rdf + 1 + 2 * rdn
}

/// Whether `p` has the any-Axim margin in `passed` (one flag per
/// [`GRID`] point): it passed, and so did every point up to two codes
/// tighter in each field (clamped at the VLIO minimum, below which the
/// controller can't go).
#[must_use]
pub fn with_margin(p: usize, passed: &[bool; 54]) -> bool {
    let (f, n, r) = GRID[p];
    passed[p]
        && GRID.iter().zip(passed).all(|(&(f2, n2, r2), &ok)| {
            let inside = f2 <= f && f2 + 2 >= f && n2 <= n && n2 + 2 >= n && r2 <= r && r2 + 2 >= r;
            !inside || ok
        })
}

/// The patterns and their hash: shared with the per-device self-test's
/// bus check (`rp_hal_pxa27x::buscheck`, R71).
pub use rp_hal_pxa27x::buscheck::{Pattern, mix};

/// Run R71's bus check instead of the grid (`RP_CS3BUS_SELFTEST`): the
/// per-device self-test's CS3 part, cold then warm.
pub const SELFTEST: bool = option_env!("RP_CS3BUS_SELFTEST").is_some();

/// The bus check's warm traffic, seconds (`RP_CS3BUS_WARM_SECS`; five
/// minutes by default).
pub const WARM_SECS: u32 = match option_env!("RP_CS3BUS_WARM_SECS") {
    Some(v) => parse_secs(v),
    None => 300,
};

/// The timing steps 2-4 try (`RP_CS3BUS_CANDIDATE`, three digits RDF RDN
/// RRR, e.g. `420`; `buscheck::FAST` by default). A test-only negative
/// control: run 2 read every word wrong at RDF 4 (memory-timing.md), so a
/// check run there must report misses and keep 7/3/1, or it is blind.
pub const CANDIDATE: (u32, u32, u32) = match option_env!("RP_CS3BUS_CANDIDATE") {
    Some(v) => {
        let b = v.as_bytes();
        assert!(b.len() == 3, "RP_CS3BUS_CANDIDATE: three digits");
        (
            (b[0] - b'0') as u32,
            (b[1] - b'0') as u32,
            (b[2] - b'0') as u32,
        )
    }
    None => rp_hal_pxa27x::buscheck::FAST,
};

const fn parse_secs(v: &str) -> u32 {
    let b = v.as_bytes();
    let (mut n, mut k) = (0u32, 0);
    while k < b.len() {
        n = n * 10 + (b[k] - b'0') as u32;
        k += 1;
    }
    n
}

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use super::{
        CANDIDATE, GRID, MARKER, Pattern, SELFTEST, WARM_SECS, WM_CS3, WM_MSC1, clocks_per_beat,
        mix, with_margin,
    };
    use rp_drv_blit2d::{self as cmd, Engine, Format};
    use rp_drv_mbx_display::{Display, Plane, console::Console};
    use rp_hal_api::RegisterBus;
    use rp_hal_mem::{load32_volatile, store32_volatile};
    use rp_hal_pxa27x::buscheck;
    use rp_hal_pxa27x::{cpu, diag, rt};
    use rp_hal_x51v::MBX_VRAM_SIZE;
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
            let _ = diag::set_cs3_vlio(WM_CS3.0, WM_CS3.1, WM_CS3.2);
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

    /// The owner's label at the top of the plane on screen, or nothing if
    /// the plane overlaps the test's pool (never, by the memory map).
    fn label(vram: &mut [u8], plane: &Plane, args: core::fmt::Arguments) {
        let end = plane.offset + plane.stride * usize::from(plane.height);
        if end > vram.len() || (plane.offset < POOL.end() as usize && end > POOL.at as usize) {
            return;
        }
        let mut c = Console::new(
            &mut vram[plane.offset..end],
            plane.stride,
            (0, 0),
            (480, 96),
            2,
            (0xFFFF, 0x0000),
        );
        let _ = c.write_fmt(args);
    }

    fn main() -> ! {
        // A crash puts MSC1 back first (rp-image's crash order).
        rp_image::on_crash(restore);
        rt::init_mmu();
        let Some((regs, vram)) = rp_image::take_mbx() else {
            crash(format_args!("no 2700G"))
        };
        // The plane on screen, kept as found (as every image takes it).
        let mut d = Display::new(regs);
        let (plane, _) = d.adopt(MBX_VRAM_SIZE as usize);
        let regs = d.into_inner();
        label(
            vram,
            &plane,
            format_args!(
                "CS3 BUS TEST (R3)\nSTARTING: 54 POINTS, THEN A\n60 S SOAK. NOTHING TO DO."
            ),
        );
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
        if SELFTEST {
            selftest(vram, &plane, &mut e, src);
            restore();
            let _ = writeln!(RunLog, "MSC1 restored {:08X}", diag::msc1());
            label(
                vram,
                &plane,
                format_args!("CS3 BUS CHECK (R71)\nDONE: BACK TO THE LOADER"),
            );
            let _ = writeln!(RunLog, "{MARKER}");
            finish();
        }
        // Which BGCOLOR bits hold a value, at Windows Mobile's timing.
        let bg_found = e.regs.read32(BGCOLOR);
        e.regs.write32(BGCOLOR, 0xFFFF_FFFF);
        let bg_mask = e.regs.read32(BGCOLOR);
        e.regs.write32(BGCOLOR, bg_found);
        let _ = writeln!(RunLog, "BGCOLOR {bg_found:08X}, bits held {bg_mask:08X}");
        let _ = writeln!(
            RunLog,
            "point RDF RDN RRR clk/beat MSC1 | flush us MB/s miss | read256K us MB/s | patterns miss | reg miss | 2D fill miss | control at WM miss"
        );

        let mut verdict = "every point ran";
        let mut passed = [false; GRID.len()];
        let mut flush_us = [u32::MAX; GRID.len()];
        let base = 0x0C00_0000 + PATTERN_AT as u32;
        for (point, &(rdf, rdn, rrr)) in GRID.iter().enumerate() {
            rt::check_in();
            // Each point from Windows Mobile's timing, so no point inherits
            // another's state.
            restore();
            label(
                vram,
                &plane,
                format_args!(
                    "CS3 BUS TEST (R3)\nPOINT {} OF {}\nRDF {rdf} RDN {rdn} RRR {rrr}",
                    point + 1,
                    GRID.len()
                ),
            );
            let Some(now) = diag::set_cs3_vlio(rdf, rdn, rrr) else {
                let _ = writeln!(
                    RunLog,
                    "{point} {rdf} {rdn} {rrr}: refused (outside the legal range)"
                );
                continue;
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
            let dcount = fill_check(&mut e, vram, mix(point as u32 ^ 0x2D00));
            rt::check_in();

            // Windows Mobile's timing back, then the first bad word re-read
            // there (a write fault or a read fault?) and a control pass.
            restore();
            let mut ccount = 0;
            for k in 0..PATTERN_BYTES / 4 {
                let _ = store32_volatile(vram, PATTERN_AT + 4 * k, mix(k as u32 ^ point as u32));
            }
            cpu::drain_write_buffer();
            for k in 0..PATTERN_BYTES / 4 {
                if load32_volatile(vram, PATTERN_AT + 4 * k) != Some(mix(k as u32 ^ point as u32)) {
                    ccount += 1;
                }
            }
            rt::check_in();

            let mbs = |bytes: usize, t: u32| bytes as u32 / t.max(1);
            let _ = writeln!(
                RunLog,
                "{point} {rdf} {rdn} {rrr} {} {now:08X} | {best} {} {fcount} | {read_us} {} | {pcount} | {rcount} | {dcount} | {ccount}",
                clocks_per_beat(rdf, rdn),
                mbs(FLUSH_BYTES, best),
                mbs(READ_BYTES, read_us),
            );
            if let Some((k, want, got)) = fmiss {
                let again = load32_volatile(vram, FLUSH_AT + 4 * k).unwrap_or(0);
                let _ = writeln!(
                    RunLog,
                    "  flush miss: word {k} wanted {want:08X} read {got:08X}, at WM timing {again:08X}"
                );
            }
            if pmiss.is_some() {
                // The region now holds the control's words: the pattern's
                // first bad word is named, not re-read.
                let (k, want, got) = pmiss.unwrap_or((0, 0, 0));
                let _ = writeln!(
                    RunLog,
                    "  pattern {pname} miss: word {k} wanted {want:08X} read {got:08X}"
                );
            }
            if dcount != 0 {
                let _ = writeln!(
                    RunLog,
                    "  2D fill: {dcount} words wrong (u32::MAX: the engine never finished)"
                );
            }
            if ccount != 0 {
                // Windows Mobile's own timing no longer reads right: stop
                // before anything else is tried.
                verdict = "stopped: the control at Windows Mobile's timing missed";
                break;
            }
            passed[point] = fcount + pcount + rcount == 0 && dcount == 0;
            flush_us[point] = best;
        }
        restore();
        let good = passed.iter().filter(|&&p| p).count();
        let margin = (0..GRID.len()).filter(|&p| with_margin(p, &passed)).count();
        let _ = writeln!(
            RunLog,
            "grid: {good} of {} points passed, {margin} with the margin",
            GRID.len()
        );

        // The soak: the fastest flush among the points with the margin,
        // Windows Mobile's own (point 0) aside.
        let pick = (1..GRID.len())
            .filter(|&p| with_margin(p, &passed))
            .min_by_key(|&p| flush_us[p]);
        match pick {
            None => {
                let _ = writeln!(
                    RunLog,
                    "soak: no point faster than WM's has the margin: none run"
                );
            }
            Some(c) => {
                let (rdf, rdn, rrr) = GRID[c];
                label(
                    vram,
                    &plane,
                    format_args!(
                        "CS3 BUS TEST (R3)\nSOAK, 60 S, AT POINT {}\nRDF {rdf} RDN {rdn} RRR {rrr}",
                        c + 1
                    ),
                );
                let _ = diag::set_cs3_vlio(rdf, rdn, rrr);
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
                    let base = base + rounds;
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
                    "soak at point {c} (RDF {rdf} RDN {rdn} RRR {rrr}, {} clk/beat, flush {} us against WM's {} us): {rounds} rounds in {} s, {misses} misses",
                    clocks_per_beat(rdf, rdn),
                    flush_us[c],
                    flush_us[0],
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
        label(
            vram,
            &plane,
            format_args!("CS3 BUS TEST (R3)\nDONE: BACK TO THE LOADER"),
        );
        let _ = writeln!(RunLog, "{MARKER}");
        finish();
    }

    /// R71's bus check over the pattern region at the timing set now.
    fn bus_check(vram: &mut [u8]) -> buscheck::Report {
        let base = 0x0C00_0000 + PATTERN_AT as u32;
        let vram = core::cell::RefCell::new(vram);
        buscheck::check(
            base,
            PATTERN_BYTES / 4,
            |k, w| {
                let _ = store32_volatile(&mut vram.borrow_mut(), PATTERN_AT + 4 * k, w);
            },
            |k| load32_volatile(&vram.borrow(), PATTERN_AT + 4 * k),
            || {
                cpu::drain_write_buffer();
                rt::check_in();
            },
        )
    }

    /// The per-device self-test's CS3 part (R71): the default 7 / 3 / 1
    /// checked, 7 / 2 / 0 checked cold, [`WARM_SECS`] of traffic at 7 / 2 / 0
    /// (the flush, the 2D fill, a pattern pass, every word checked), then
    /// 7 / 2 / 0 checked warm; the pick logged.
    fn selftest(vram: &mut [u8], plane: &Plane, e: &mut Engine<rp_hal_pxa27x::Mmio>, src: &[u8]) {
        let _ = writeln!(
            RunLog,
            "cs3bus self-test (R71): six patterns over {} KiB written and read back word for word ({} transfers a check), the default {:?}, then {:?} cold, {WARM_SECS} s of traffic at it, and warm",
            PATTERN_BYTES / 1024,
            6 * 2 * PATTERN_BYTES / 4,
            buscheck::DEFAULT,
            CANDIDATE
        );
        let (f, n, r) = CANDIDATE;
        let set = |t: buscheck::Timing| diag::set_cs3_vlio(t.0, t.1, t.2);
        let show = |r: &buscheck::Report| (r.transfers, r.misses, r.first, r.passed());
        label(
            vram,
            plane,
            format_args!("CS3 BUS CHECK (R71)\nSTEP 1 OF 4: 7/3/1"),
        );
        let m = set(buscheck::DEFAULT);
        let d = bus_check(vram);
        let _ = writeln!(
            RunLog,
            "default 7/3/1 (MSC1 {m:08X?}): transfers, misses, first, passed {:?}",
            show(&d)
        );
        label(
            vram,
            plane,
            format_args!("CS3 BUS CHECK (R71)\nSTEP 2 OF 4: {f}/{n}/{r} COLD"),
        );
        let m = set(CANDIDATE);
        let cold = bus_check(vram);
        let _ = writeln!(
            RunLog,
            "{f}/{n}/{r} cold (MSC1 {m:08X?}): {:?}",
            show(&cold)
        );
        label(
            vram,
            plane,
            format_args!("CS3 BUS CHECK (R71)\nSTEP 3 OF 4: WARM, {WARM_SECS} S\nAT {f}/{n}/{r}"),
        );
        let t0 = rt::ticks();
        let (mut rounds, mut misses) = (0u32, 0u32);
        while us(rt::ticks().wrapping_sub(t0)) < WARM_SECS * 1_000_000 && misses == 0 {
            rt::check_in();
            vram[FLUSH_AT..FLUSH_AT + FLUSH_BYTES].copy_from_slice(src);
            cpu::drain_write_buffer();
            for (k, w) in src.chunks_exact(4).enumerate() {
                let want = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
                if load32_volatile(vram, FLUSH_AT + 4 * k) != Some(want) {
                    misses += 1;
                }
            }
            misses = misses.saturating_add(fill_check(e, vram, mix(rounds ^ 0x50AC)));
            rounds += 1;
        }
        let _ = writeln!(
            RunLog,
            "warm traffic at {f}/{n}/{r}: {rounds} rounds (flush {} KiB and 2D fill each, every word checked) in {} s, {misses} misses",
            FLUSH_BYTES / 1024,
            us(rt::ticks().wrapping_sub(t0)) / 1_000_000
        );
        label(
            vram,
            plane,
            format_args!("CS3 BUS CHECK (R71)\nSTEP 4 OF 4: {f}/{n}/{r} WARM"),
        );
        let warm = (misses == 0).then(|| bus_check(vram));
        let _ = writeln!(RunLog, "{f}/{n}/{r} warm: {:?}", warm.as_ref().map(show));
        let (pick, default_ok) = buscheck::pick(&d, &cold, warm.as_ref());
        // `pick` names FAST when the candidate passed (it is FAST unless
        // `RP_CS3BUS_CANDIDATE` set a negative control).
        let earned = pick != buscheck::DEFAULT;
        let pick = if earned { CANDIDATE } else { pick };
        let _ = writeln!(
            RunLog,
            "self-test pick: {pick:?} ({f}/{n}/{r} {}); the default {}",
            if earned {
                "passed cold and warm"
            } else {
                "not earned: 7/3/1 kept"
            },
            if default_ok {
                "passed"
            } else {
                "FAILED (a fault for the log)"
            }
        );
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
    fn the_grid_is_every_point_from_windows_mobile_to_the_vlio_minimum() {
        assert_eq!(WM_CS3, (8, 4, 2));
        assert_eq!(GRID[0], WM_CS3);
        assert_eq!(GRID[53], VLIO_MIN);
        for (i, a) in GRID.iter().enumerate() {
            assert!(GRID[i + 1..].iter().all(|b| b != a), "{a:?} twice");
            // Inside diag::cs3_vlio's range (host-tested in rp-hal-pxa27x).
            assert!(
                (3..=8).contains(&a.0) && (2..=4).contains(&a.1) && a.2 <= 2,
                "{a:?}"
            );
        }
        assert_eq!(clocks_per_beat(8, 4), 17);
        assert_eq!(clocks_per_beat(3, 2), 8);
    }

    fn at(f: u32, n: u32, r: u32) -> usize {
        GRID.iter()
            .position(|&p| p == (f, n, r))
            .expect("on the grid")
    }

    #[test]
    fn the_margin_needs_every_point_two_codes_tighter() {
        // Run 1's unit: reads fail at RDF 4 and below, whatever else.
        let mut passed = [false; 54];
        for (k, p) in GRID.iter().enumerate() {
            passed[k] = p.0 >= 5;
        }
        // RDF 7 has 5 and 6 under it; RDF 6 has 4, which failed.
        assert!(with_margin(at(7, 2, 0), &passed));
        assert!(with_margin(at(8, 4, 2), &passed));
        assert!(!with_margin(at(6, 4, 2), &passed));
        assert!(!with_margin(at(5, 2, 0), &passed));
        // One failing point two codes under in RRR takes the margin away.
        passed[at(7, 3, 0)] = false;
        assert!(!with_margin(at(7, 3, 2), &passed));
        assert!(!with_margin(at(7, 4, 1), &passed));
        assert!(with_margin(at(7, 2, 0), &passed));
        // A point that failed never has it.
        assert!(!with_margin(at(7, 3, 0), &passed));
        // Every point passing: the VLIO minimum has it (clamped).
        assert!(with_margin(at(3, 2, 0), &[true; 54]));
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
