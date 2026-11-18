//! C2, the CPU past 624 MHz while a port plays (the owner's go,
//! 2026-11-16: "Yes, start C2"; `docs/research/sheets/c2-cpu-overclock-test1.md`,
//! reminder R123).
//!
//! Iterate, don't imitate: Intel rates the PXA270 at 624 MHz and Dell runs
//! it there; `rp-clocks`' C2b (2026-10-30) was to test past it alone and
//! never reached the overclock (its runs hung on the card log, R123). Here
//! the test rides the port runner, proven on this device: route A (the turbo
//! multiplier 2N 7 or 8 alone, 728 or 832 MHz; L 16, run, bus, memory and
//! the SDRAM's timing exactly Dell's 624 point, so the RAM rule holds), the
//! voltage pins never touched (the step is refused unless Windows Mobile left
//! Dell's 624 point, which carries his top voltage pattern). The core's
//! health is checked as it runs, not trusted: a known-answer integer
//! workload (`rp-clocks`' C2b mix: shifts, multiplies, a 64-bit product),
//! its answer worked out at 624 before the step, run again every frame and
//! before every card write. One wrong answer puts 624 back at once and keeps
//! it, so a core that has started to miscount never writes to the shared card.
//! Test images only: nothing ships past 624 MHz (ADR-0031) until the
//! per-device self-test (R71) decides.

use core::fmt::Arguments;
use rp_hal_pxa27x::clocks::{self, POINTS, Point};
use rp_hal_pxa27x::rt;

/// The step's state (the target has no atomics: the runtime's global).
#[derive(Clone, Copy)]
struct State {
    /// The core past 624 MHz now.
    stepped: bool,
    /// The workload's answer at 624 MHz.
    answer: u32,
    /// Checks run, and wrong answers seen.
    checks: u32,
    wrong: u32,
}

static STATE: rt::Global<State> = rt::Global::new(State {
    stepped: false,
    answer: 0,
    checks: 0,
    wrong: 0,
});

fn get() -> State {
    STATE.with(|s| *s)
}

/// Rounds of the workload a check: ~2,000 (tens of microseconds).
const ROUNDS: u32 = 2_000;

/// `rp-clocks`' C2b integer mix (xorshift, a multiply-add, a rotate and a
/// 64-bit product), shorter: every multiplier and shifter path the game
/// uses, with an answer that any one wrong bit changes.
#[inline(never)]
fn work(seed: u32) -> u32 {
    let (mut x, mut acc) = (seed | 1, 0u32);
    for i in 0..ROUNDS {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        acc = acc.wrapping_mul(0x0100_0193).wrapping_add(x ^ i);
        acc = acc.rotate_left(7) ^ ((u64::from(x) * u64::from(acc)) >> 32) as u32;
    }
    core::hint::black_box(acc)
}

/// Dell's 624 MHz point (L 16, 2N 6, the memory clock the bus, fast bus,
/// turbo).
fn at624() -> Point {
    POINTS[6].1
}

/// Steps the core to 2N `n2` (7: 728 MHz, 8: 832) if it runs Dell's 624
/// point; logs what happened. A refused or unlatched step leaves 624.
pub fn step(n2: u8, log: fn(Arguments<'_>)) {
    if !(7..=8).contains(&n2) {
        log(format_args!(
            "rp-cpu: 2N {n2} not a C2 step (7 or 8): 624 MHz kept"
        ));
        return;
    }
    let found = clocks::read();
    if found.point() != at624() {
        log(format_args!(
            "rp-cpu: not stepped: the core is not at Dell's 624 MHz point (CCCR 0x{:x}, CLKCFG 0x{:x}), so its voltage pattern is not known to be the top",
            found.cccr, found.clkcfg
        ));
        return;
    }
    let answer = work(0x5EED_0001);
    STATE.with(|s| s.answer = answer);
    let to = Point { n2, ..at624() };
    if !clocks::change_past_rating(to) {
        log(format_args!(
            "rp-cpu: 2N {n2} refused by the plan: 624 MHz kept"
        ));
        return;
    }
    let now = clocks::read();
    if now.point() != to {
        clocks::change(at624());
        log(format_args!(
            "rp-cpu: the PLL didn't take 2N {n2} (CCSR 0x{:x}): 624 MHz put back",
            now.ccsr
        ));
        return;
    }
    let khz = clocks::measure_core_khz(20);
    STATE.with(|s| s.stepped = true);
    let ok = check(log);
    log(format_args!(
        "rp-cpu: the core stepped to 2N {n2} ({} kHz planned, {khz} counted; bus, memory and SDRAM as at 624), the first check {}",
        to.core_khz(),
        if ok { "right" } else { "WRONG: 624 put back" }
    ));
}

/// The known answer again, if stepped: false (and 624 back, for good) on a
/// wrong one. Every frame and before every card write.
pub fn check(log: fn(Arguments<'_>)) -> bool {
    let st = get();
    if !st.stepped {
        return true;
    }
    STATE.with(|s| s.checks += 1);
    if work(0x5EED_0001) == st.answer {
        return true;
    }
    STATE.with(|s| {
        s.wrong += 1;
        s.stepped = false;
    });
    clocks::change(at624());
    log(format_args!(
        "rp-cpu: a WRONG answer at check {} past 624 MHz: Dell's 624 put back for the rest of the run",
        get().checks
    ));
    false
}

/// Whether the core may write to the card now: true at 624, or past it with
/// the known answer right this moment.
pub fn may_write() -> bool {
    let st = get();
    !st.stepped || work(0x5EED_0001) == st.answer
}

/// Dell's 624 back if stepped, with the run's counts.
pub fn put_back(log: fn(Arguments<'_>)) {
    let st = get();
    if st.stepped {
        STATE.with(|s| s.stepped = false);
        clocks::change(at624());
        log(format_args!(
            "rp-cpu: Dell's 624 MHz put back; {} checks past it, {} wrong",
            st.checks, st.wrong
        ));
    } else if st.checks > 0 {
        log(format_args!(
            "rp-cpu: {} checks past 624 MHz, {} wrong (624 already back)",
            st.checks, st.wrong
        ));
    }
}
