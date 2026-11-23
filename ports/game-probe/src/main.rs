//! Game Mode's test game (G7a, docs/design/game-mode-build.md G7): the
//! kernel starts it as Game Mode's one program, in user mode, while Full
//! Mode's processes are frozen, and it checks each thing the kernel gives a
//! game in place of what the bare test image did privileged:
//!
//! 1. **The large region:** 4 MiB at `LARGE_BASE` in sections, each
//!    megabyte written and read back.
//! 2. **The clock:** the OS timer's counter through its read-only window.
//! 3. **Cache calls:** 256 write-buffer drains timed (the system call's
//!    cost on its own).
//! 4. **DMA:** 64 KiB copied by the game's channel from one large-region
//!    buffer to another, the source across a megabyte edge (two sections,
//!    rarely one physical run, so the kernel splits a descriptor), the chain
//!    named in the game's own addresses; the channel's status read through
//!    the controller's read-only window; every word checked.
//! 5. **Refusals:** a chain naming the kernel's memory, a channel the game
//!    doesn't hold, a cache call on memory not its own and an unknown
//!    operation are each refused.
//!
//! It reports (`SYS_REPORT`): the failed checks (a bit each), the drains'
//! time, the copy's time (OS timer ticks, 3.25 MHz) and the descriptors the
//! channel ran with the refusals seen; the kernel then ends it.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// The large region the kernel maps for this game.
pub const LARGE_LEN: usize = 4 << 20;
/// The DMA channel the kernel grants it (`dma_map::SD`, the runner's card
/// channel: in Game Mode the game holds the card).
pub const CHANNEL: u32 = 0;
/// The copy's length: 64 KiB (16 descriptors of 4 KiB).
pub const COPY_LEN: usize = 64 * 1024;
/// Its source: 32 KiB short of the first megabyte edge.
pub const COPY_SRC: usize = (1 << 20) - 32 * 1024;
/// Its target, in the third megabyte.
pub const COPY_DST: usize = 0x28_0000;
// The copy crosses one megabyte edge, its target after it, both inside
// the region, in 16 descriptors of 4 KiB.
const _: () = assert!(
    COPY_SRC < 1 << 20
        && COPY_SRC + COPY_LEN > 1 << 20
        && COPY_DST >= COPY_SRC + COPY_LEN
        && COPY_DST + COPY_LEN <= LARGE_LEN
        && COPY_LEN / 4096 == 16
);
/// Drains timed.
pub const DRAINS: u32 = 256;

/// Check bits in the report's first word: a large-region word read back
/// wrong.
pub const FAIL_LARGE: u32 = 1 << 0;
/// The OS timer didn't move.
pub const FAIL_CLOCK: u32 = 1 << 1;
/// A drain call failed.
pub const FAIL_DRAIN: u32 = 1 << 2;
/// The kernel refused the copy's chain.
pub const FAIL_DMA_START: u32 = 1 << 3;
/// The channel stopped on a bus error or not within 0.1 s.
pub const FAIL_DMA_DONE: u32 = 1 << 4;
/// A copied word differs.
pub const FAIL_DMA_DATA: u32 = 1 << 5;
/// A refusal didn't come.
pub const FAIL_REFUSALS: u32 = 1 << 6;
/// A cache call on the game's own memory failed.
pub const FAIL_CACHE: u32 = 1 << 7;
/// Refusals expected.
pub const REFUSALS: u32 = 4;

/// The word the copy's source holds at byte `off`.
#[must_use]
pub const fn pattern(off: usize) -> u32 {
    (off as u32).wrapping_mul(0x9E37_79B1) ^ 0x6A4D_0001
}

/// `DCMD` for a memory-to-memory piece of `len` bytes: both addresses
/// increment, 32-byte bursts, word width.
#[must_use]
pub const fn copy_cmd(len: u32) -> u32 {
    1 << 31 | 1 << 30 | 3 << 16 | 3 << 14 | len
}

/// `DCSR`'s stop state: the channel ran its chain to the end.
pub const DCSR_STOPSTATE: u32 = 1 << 3;
/// `DCSR`'s bus error.
pub const DCSR_BUSERR: u32 = 1 << 0;

#[cfg(target_os = "none")]
mod game {
    use super::*;
    use rp_hal_api::RegisterBus;
    use rp_hal_user::{Device, DmaDesc, Large, Window, cache, dma, is_err, report};

    rp_hal_user::entry!(main);

    const OSCR: usize = 0x10;

    fn main(_: u32, _: u32, _: u32, _: u32) -> ! {
        let mut fail = 0;
        let mut ost = Window::of(Device::Ost);
        let mut status = Window::of(Device::DmaStatus);
        let mut big = Large::new(LARGE_LEN);

        // 1. The large region.
        for mb in 0..LARGE_LEN >> 20 {
            for off in [0, 0x4_0004, 0xF_FFFC] {
                let at = (mb << 20) + off;
                big.write(at, 0x1A26_0000 + at as u32);
                if big.read(at) != 0x1A26_0000 + at as u32 {
                    fail |= FAIL_LARGE;
                }
            }
        }

        // 2. The clock moves.
        let t0 = ost.read32(OSCR);
        let mut spins = 0u32;
        while ost.read32(OSCR) == t0 && spins < 100_000 {
            spins += 1;
        }
        if spins == 100_000 {
            fail |= FAIL_CLOCK;
        }

        // 3. Drains.
        let t0 = ost.read32(OSCR);
        for _ in 0..DRAINS {
            if cache(3, 0, 0) != 0 {
                fail |= FAIL_DRAIN;
            }
        }
        let drains = ost.read32(OSCR).wrapping_sub(t0);

        // 4. DMA: the source filled, the target cleared, both made memory's.
        for off in (0..COPY_LEN).step_by(4) {
            big.write(COPY_SRC + off, pattern(off));
            big.write(COPY_DST + off, 0);
        }
        let len = COPY_LEN as u32;
        if cache(0, big.addr(COPY_SRC), len) != 0 || cache(2, big.addr(COPY_DST), len) != 0 {
            fail |= FAIL_CACHE;
        }
        let mut chain = [DmaDesc::default(); COPY_LEN / 4096];
        for (k, d) in chain.iter_mut().enumerate() {
            *d = DmaDesc {
                src: big.addr(COPY_SRC + k * 4096),
                dst: big.addr(COPY_DST + k * 4096),
                cmd: copy_cmd(4096),
            };
        }
        let t0 = ost.read32(OSCR);
        let ran = dma(CHANNEL, &chain);
        let mut copy = 0;
        if is_err(ran) {
            fail |= FAIL_DMA_START;
        } else {
            // At most ~0.1 s (325,000 ticks).
            loop {
                let s = status.read32(4 * CHANNEL as usize);
                copy = ost.read32(OSCR).wrapping_sub(t0);
                if s & DCSR_BUSERR != 0 || copy > 325_000 {
                    fail |= FAIL_DMA_DONE;
                    break;
                }
                if s & DCSR_STOPSTATE != 0 {
                    break;
                }
            }
            if cache(1, big.addr(COPY_DST), len) != 0 {
                fail |= FAIL_CACHE;
            }
            for off in (0..COPY_LEN).step_by(4) {
                if big.read(COPY_DST + off) != pattern(off) {
                    fail |= FAIL_DMA_DATA;
                }
            }
        }

        // 5. Refusals: nothing started, an error each.
        let mut refused = 0;
        let kernel = [DmaDesc {
            src: big.addr(COPY_SRC),
            dst: 0xA800_8000,
            cmd: copy_cmd(64),
        }];
        refused += u32::from(is_err(dma(CHANNEL, &kernel)));
        refused += u32::from(is_err(dma(CHANNEL + 5, &chain[..1])));
        refused += u32::from(is_err(cache(0, 0xA800_8000, 64)));
        refused += u32::from(is_err(cache(7, big.addr(0), 64)));
        if refused != REFUSALS {
            fail |= FAIL_REFUSALS;
        }
        report(fail, drains, copy, ran & 0xFF | refused << 8)
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pattern_never_repeats_inside_the_copy_and_is_never_zero() {
        let mut seen: Vec<u32> = (0..COPY_LEN).step_by(4).map(pattern).collect();
        assert!(seen.iter().all(|&w| w != 0));
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), COPY_LEN / 4);
    }

    #[test]
    fn a_copy_piece_is_both_sides_moving_with_no_refused_flag() {
        let c = copy_cmd(4096);
        assert_eq!(c & 0x1FFF, 4096);
        assert_eq!(c >> 30, 0b11);
        // Interrupts, fly-by, compare: bits 19-25 clear.
        assert_eq!(c & (0x7F << 19), 0);
    }
}
