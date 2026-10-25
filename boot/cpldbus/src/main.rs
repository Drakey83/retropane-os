//! Test image: the board CPLD's bus timing (reminder R8, docs/research/
//! memory-timing.md section 9; the owner, 2026-10-24: "test and verify it",
//! every gain counts). Windows Mobile sets the CPLD's chip select (CS4, the
//! low half of `MSC2`) to `0x7FFC`, the slowest setting the PXA27x has. This
//! image shortens CS4's VLIO timing one field and one code at a time
//! ([`shorter`]: the strobe `RDF`, then the gap `RDN`, then the recovery
//! `RRR`), and at each step:
//!
//! - **reads, unattended:** the CPLD's status registers ([`OFFSETS`]) 1,000
//!   times each, every value against what Windows Mobile's timing read
//!   (reading changes nothing on the board); a field stops at its first
//!   wrong read, Windows Mobile's timing goes back, the CPLD is read again
//!   there, and the next field starts from the last clean step;
//! - times 10,000 reads (the speed the step gives).
//!
//! **Writes, only in the watched build** (`RP_CPLDBUS_WRITES=1`, the owner
//! at the screen; the no-blind-board-bits rule): at Windows Mobile's timing
//! first (the control), then at each step the reads proved clean (so the
//! CPLD decodes its addresses right at that timing), the backlight switched
//! off and on again (CPLD `0x18` bit 2, the whole shadowed value as every
//! write is), Windows Mobile's timing put back and the backlight switched
//! on again there. The owner says which steps blinked; a step that
//! doesn't is a write that didn't latch.
//!
//! Every step is labelled on screen ("STEP n OF m"). It refuses to start
//! unless `MSC2` is exactly Windows Mobile's; only CS4's three timing fields
//! change (`diag::set_cs4_vlio` refuses anything else); `MSC2` goes back at
//! the end and from the crash path. Writes nothing to flash or the cards.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

use rp_hal_pxa27x::diag::{cs4_vlio, vlio_clocks};

/// The last run-log line of a complete run.
pub const MARKER: &str = "RETROPANE-CPLDBUS-DONE";

/// `MSC2` as Windows Mobile leaves it (probe 2026-10-11): CS4 `0x7FFC`.
// inherited: R8, Windows Mobile's MSC2 read by the probe (probe-2026-10-11.md)
pub const WM_MSC2: u32 = 0x7FF4_7FFC;

/// The CPLD offsets read at every step: the status inputs that read non-zero
/// at hand-over (`0x00`, `0x04`, `0x06`, `0x14`) and two neighbours that
/// read zero (`0x02`, `0x16`), so a read landing on the wrong register shows
/// (docs/hw/cpld.md: every offset was read at hand-over, none had an
/// effect).
pub const OFFSETS: [usize; 6] = [0x00, 0x02, 0x04, 0x06, 0x14, 0x16];

/// CS4's VLIO timing codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    /// Strobe (`RDF4`).
    pub rdf: u32,
    /// Gap between accesses (`RDN4`).
    pub rdn: u32,
    /// Recovery after the chip select (`RRR4`).
    pub rrr: u32,
}

/// Windows Mobile's CS4 timing (`0x7FFC`).
pub const WM: Step = Step {
    rdf: 15,
    rdn: 15,
    rrr: 7,
};

/// The fields, in the order they are shortened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    /// The strobe.
    Rdf,
    /// The gap.
    Rdn,
    /// The recovery.
    Rrr,
}

/// The fields in order.
pub const FIELDS: [Field; 3] = [Field::Rdf, Field::Rdn, Field::Rrr];

/// Every step a run can take when every read is clean: 12 + 13 + 7.
pub const MAX_STEPS: usize = 32;

/// The steps the screen counts at most: the start, every read step, and
/// (the watched build) a blink at Windows Mobile's timing and at each read
/// step (the device's first watched run showed "66 of 65").
#[must_use]
pub const fn total_steps(writes: bool) -> usize {
    1 + MAX_STEPS + if writes { MAX_STEPS + 1 } else { 0 }
}

/// `s` one code shorter in `field`, or `None` at the PXA27x's VLIO minimum
/// (`RDF` 3, `RDN` 2, `RRR` 0).
#[must_use]
pub const fn shorter(s: Step, field: Field) -> Option<Step> {
    match field {
        Field::Rdf if s.rdf > 3 => Some(Step {
            rdf: s.rdf - 1,
            ..s
        }),
        Field::Rdn if s.rdn > 2 => Some(Step {
            rdn: s.rdn - 1,
            ..s
        }),
        Field::Rrr if s.rrr > 0 => Some(Step {
            rrr: s.rrr - 1,
            ..s
        }),
        _ => None,
    }
}

/// Memory clocks one access holds CS4 for, RDY waits aside: the strobe
/// (`RDF` + 1), the gap (2 x `RDN`) and the recovery (2 x `RRR` + 1), codes
/// 12-15 table-encoded (PXA27x manual table 6-28).
#[must_use]
pub const fn clocks(s: Step) -> u32 {
    vlio_clocks(s.rdf) + 1 + 2 * vlio_clocks(s.rdn) + 2 * s.rrr + 1
}

/// `MSC2` at step `s`, from Windows Mobile's.
#[must_use]
pub const fn msc2(s: Step) -> Option<u32> {
    cs4_vlio(WM_MSC2, s.rdf, s.rdn, s.rrr)
}

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use super::{
        FIELDS, MARKER, MAX_STEPS, OFFSETS, Step, WM, WM_MSC2, clocks, msc2, shorter, total_steps,
    };
    use rp_drv_cpld::{Cpld, handover};
    use rp_drv_mbx_display::Display;
    use rp_drv_mbx_display::console::Console;
    use rp_hal_pxa27x::{Mmio, clocks as pxa_clocks, diag, rt};
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices::{self, RunLog};

    rp_image::entry!(main);

    type BoardCpld = Cpld<Mmio, handover::Table<'static>>;

    /// The watched build: the backlight blinks at each clean step.
    const WRITES: bool = option_env!("RP_CPLDBUS_WRITES").is_some();
    /// Reads of each offset a step.
    const READS: u32 = 1_000;
    /// Reads timed a step.
    const TIMED: u32 = 10_000;

    macro_rules! log {
        ($($t:tt)*) => {{ let _ = writeln!(RunLog, $($t)*); }};
    }

    /// Windows Mobile's CS4 timing back, whatever failed (the image only
    /// runs when `MSC2` started as Windows Mobile's).
    fn restore() {
        if diag::msc2() != WM_MSC2 {
            restore_wm();
        }
    }

    fn restore_wm() {
        let _ = diag::set_cs4_vlio(WM.rdf, WM.rdn, WM.rrr);
    }

    /// Text for the owner, on screen when there is one, and in the log.
    struct Screen<'a>(Option<Console<'a>>);

    impl Screen<'_> {
        fn say(&mut self, args: core::fmt::Arguments) {
            log!("{args}");
            if let Some(c) = self.0.as_mut() {
                let _ = c.write_fmt(args);
                let _ = c.write_str("\n");
            }
        }
    }

    /// Wrong reads at the current timing against `base` (offsets marked
    /// unstable skipped), and the first: offset and value.
    fn check(
        cpld: &mut BoardCpld,
        base: &[u16; OFFSETS.len()],
        unstable: &[bool; OFFSETS.len()],
    ) -> (u32, Option<(usize, u16)>) {
        let mut bad = 0;
        let mut first = None;
        for _ in 0..READS {
            for (k, &off) in OFFSETS.iter().enumerate() {
                if unstable[k] {
                    continue;
                }
                let v = cpld.read(off);
                if v != base[k] {
                    bad += 1;
                    first.get_or_insert((off, v));
                }
            }
        }
        rt::check_in();
        (bad, first)
    }

    /// Nanoseconds a read at the current timing (10,000 reads of `0x00`).
    fn read_ns(cpld: &mut BoardCpld) -> u32 {
        let t0 = rt::ticks();
        for _ in 0..TIMED {
            let _ = cpld.read(OFFSETS[0]);
        }
        let t = rt::ticks().wrapping_sub(t0);
        rt::check_in();
        (u64::from(t) * 1_000_000_000 / u64::from(rt::TICK_HZ) / u64::from(TIMED)) as u32
    }

    fn set(s: Step) -> Option<u32> {
        diag::set_cs4_vlio(s.rdf, s.rdn, s.rrr)
    }

    fn main() -> ! {
        // A crash puts MSC2 back first (rp-image's crash order).
        rp_image::on_crash(restore);
        rt::init_mmu();
        let console = rp_image::take_mbx().map(|(regs, vram)| {
            let (plane, _) = Display::new(regs).adopt(MBX_VRAM_SIZE as usize);
            let end = plane.offset + plane.stride * usize::from(plane.height);
            Console::new(
                &mut vram[plane.offset..end],
                plane.stride,
                (0, 0),
                (480, 640),
                2,
                (0xFFFF, 0x0000),
            )
        });
        let mut screen = Screen(console);
        let steps = total_steps(WRITES);
        screen.say(format_args!(
            "CPLD BUS TEST (R8): STEP 1 OF UP TO {steps}{}",
            if WRITES {
                ": WATCH THE SCREEN LATER"
            } else {
                ": nothing to do"
            }
        ));
        let Some(mut cpld) = devices::take_cpld().map(|(b, t)| Cpld::adopt(b, t).0) else {
            rp_image::crash(format_args!("no CPLD"))
        };
        // RetroPane runs the CPLD at RDF 6 (`devices::CPLD_TIMING`, set by
        // `take_cpld`): this test starts from Windows Mobile's timing, as it
        // was measured against.
        if Some(diag::msc2()) == msc2(Step { rdf: 6, ..WM }) {
            restore_wm();
        }
        let found = diag::msc2();
        let mem_khz = pxa_clocks::read().point().mem_khz();
        log!(
            "cpldbus: MSC2 {found:08X} (Windows Mobile's {WM_MSC2:08X}); memory clock {mem_khz} kHz; writes {WRITES}"
        );
        if found != WM_MSC2 {
            log!("MSC2 not as expected: nothing changed");
            log!("{MARKER}");
            rp_image::finish();
        }

        // Windows Mobile's timing: what each offset reads, and whether it
        // holds still.
        let mut base = [0u16; OFFSETS.len()];
        let mut unstable = [false; OFFSETS.len()];
        for (k, &off) in OFFSETS.iter().enumerate() {
            base[k] = cpld.read(off);
            unstable[k] = (0..READS).any(|_| cpld.read(off) != base[k]);
            log!(
                "base {off:#04x} = {:#06x}{}",
                base[k],
                if unstable[k] {
                    " (changes by itself: not compared)"
                } else {
                    ""
                }
            );
        }
        let wm_ns = read_ns(&mut cpld);
        log!(
            "A0 Windows Mobile's timing: {} clocks, {wm_ns} ns a read",
            clocks(WM)
        );

        // A: each field shortened to its first wrong read.
        let mut clean = [WM; MAX_STEPS + 1];
        let mut n_clean = 1;
        let mut cur = WM;
        let mut n = 0;
        for field in FIELDS {
            while let Some(next) = shorter(cur, field) {
                n += 1;
                screen.say(format_args!(
                    "STEP {} OF UP TO {steps}: reads at RDF {} RDN {} RRR {}",
                    n + 1,
                    next.rdf,
                    next.rdn,
                    next.rrr
                ));
                let Some(now) = set(next) else {
                    log!("A{n} refused by set_cs4_vlio: stopping");
                    break;
                };
                let (bad, first) = check(&mut cpld, &base, &unstable);
                let ns = read_ns(&mut cpld);
                log!(
                    "A{n} RDF {} RDN {} RRR {}: MSC2 {now:08X}, {} clocks, {ns} ns a read ({wm_ns} at Windows Mobile's), {bad} wrong of {}",
                    next.rdf,
                    next.rdn,
                    next.rrr,
                    clocks(next),
                    READS * OFFSETS.len() as u32
                );
                if let Some((off, v)) = first {
                    restore();
                    let (again, _) = check(&mut cpld, &base, &unstable);
                    log!(
                        "A{n} first wrong: {off:#04x} read {v:#06x}; at Windows Mobile's timing again {again} wrong; {field:?} stays at the last clean step"
                    );
                    break;
                }
                cur = next;
                clean[n_clean] = next;
                n_clean += 1;
            }
        }
        restore();
        let best = clean[n_clean - 1];
        log!(
            "A done: the shortest clean RDF {} RDN {} RRR {} ({} clocks against {}); MSC2 restored {:08X}",
            best.rdf,
            best.rdn,
            best.rrr,
            clocks(best),
            clocks(WM),
            diag::msc2()
        );

        if WRITES {
            blinks(&mut screen, &mut cpld, &clean[..n_clean], n + 1, steps);
        }
        restore();
        cpld.set_backlight_enable(true);
        log!("cpldbus done; MSC2 restored {:08X}", diag::msc2());
        log!("{MARKER}");
        rp_image::finish()
    }

    /// B: the backlight off and on at each clean step (the first is Windows
    /// Mobile's own timing, the control), the owner watching.
    fn blinks(
        screen: &mut Screen<'_>,
        cpld: &mut BoardCpld,
        clean: &[Step],
        first: usize,
        steps: usize,
    ) {
        for (k, s) in clean.iter().enumerate() {
            let at = first + k + 1;
            screen.say(format_args!(
                "STEP {at} OF UP TO {steps}: BLINK {} OF {}: WATCH: the screen goes dark 1 s, then lights (RDF {} RDN {} RRR {})",
                k + 1,
                clean.len(),
                s.rdf,
                s.rdn,
                s.rrr
            ));
            rt::delay_us(3_000_000);
            rt::check_in();
            let ok = msc2(*s).is_some() && set(*s).is_some();
            cpld.set_backlight_enable(false);
            rt::delay_us(1_000_000);
            rt::check_in();
            cpld.set_backlight_enable(true);
            restore();
            // Lit again at Windows Mobile's timing, whatever latched.
            cpld.set_backlight_enable(true);
            log!(
                "B{k} step {at}: RDF {} RDN {} RRR {} ({} clocks){}: blinked? the owner says",
                s.rdf,
                s.rdn,
                s.rrr,
                clocks(*s),
                if ok { "" } else { " NOT SET" }
            );
            rt::delay_us(2_000_000);
            rt::check_in();
        }
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every step a clean run takes: one field and one code at a time, from
    /// Windows Mobile's value to the PXA27x's VLIO minimum, each one
    /// `MSC2` accepts, each shorter than the last.
    #[test]
    fn the_ladder_walks_each_field_to_the_vlio_minimum() {
        assert_eq!(msc2(WM), Some(WM_MSC2));
        let mut cur = WM;
        let mut n = 0;
        for f in FIELDS {
            while let Some(next) = shorter(cur, f) {
                let changed = [
                    next.rdf != cur.rdf,
                    next.rdn != cur.rdn,
                    next.rrr != cur.rrr,
                ];
                assert_eq!(changed.iter().filter(|&&c| c).count(), 1, "{next:?}");
                assert!(clocks(next) < clocks(cur), "{next:?}");
                assert!(msc2(next).is_some(), "{next:?}");
                cur = next;
                n += 1;
            }
        }
        assert_eq!(n, MAX_STEPS);
        assert_eq!(
            cur,
            Step {
                rdf: 3,
                rdn: 2,
                rrr: 0
            }
        );
        // Windows Mobile's 0x7FFC: 31 + 60 + 15 = 106 clocks; the minimum 9.
        assert_eq!(clocks(WM), 106);
        assert_eq!(clocks(cur), 9);
    }

    /// The screen's last step number is the total it shows: the start (1),
    /// the read steps (2-33), the blinks (34-66), as the image numbers them.
    #[test]
    fn the_screen_counts_every_step() {
        assert_eq!(total_steps(false), 1 + MAX_STEPS);
        let first_blink = 1 + MAX_STEPS + 1;
        let last_blink = first_blink + MAX_STEPS;
        assert_eq!(total_steps(true), last_blink);
        assert_eq!(total_steps(true), 66);
    }

    /// The minimum is the end in every field, whatever the others hold.
    #[test]
    fn nothing_shorter_than_the_minimum() {
        let s = Step {
            rdf: 3,
            rdn: 2,
            rrr: 0,
        };
        for f in FIELDS {
            assert_eq!(shorter(s, f), None);
        }
        assert_eq!(shorter(WM, Field::Rrr), Some(Step { rrr: 6, ..WM }));
    }
}
