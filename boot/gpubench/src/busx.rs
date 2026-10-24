//! Run BX (`RP_GPUBENCH_BX`): the backlog's P12 and V17, unattended, at
//! Windows Mobile's clock.
//!
//! - **P12, DMA into 2700G memory by burst and width:** 960 KiB from SDRAM
//!   into local memory in 120 KiB pieces with bursts of 8, 16 and 32 bytes
//!   and the width field 0, byte, half and word; each timed, the result
//!   hashed against the source (the C11 sweep did SDRAM to SDRAM: 32
//!   bytes best, width no difference).
//! - **V17, status bit 10:** `0x12c` read through scan-out changes (the
//!   graphics plane repointed every frame, the video stream switched on and
//!   off, a DMA into local memory), each phase's bits ever set and always
//!   set (B10: not "commands queued"; FI2: not 2D, 3D or the FIFO's level).

use super::*;
use rp_hal_api::Width;

const PIECE: usize = 120 * 1024;
const PIECES: usize = 8;

/// Words hashed (FNV-1a).
fn fnv(words: &mut dyn Iterator<Item = u32>) -> u32 {
    words.fold(0x811C_9DC5u32, |h, w| (h ^ w).wrapping_mul(0x0100_0193))
}

#[inline(never)]
pub(super) fn run(g: &mut Rig) {
    log!(
        "BX: P12 (DMA into 2700G memory by burst and width) and V17 (status bit 10 through scan-out changes)"
    );
    wm_clock_first(g);
    clock_card(
        g,
        ["RUN BX", "DMA BURSTS AND", "STATUS BIT 10"],
        0xFF20_2040,
        2,
    );
    let Some(ram) = rp_hal_x51v::devices::take_ram_pool() else {
        log!("  BX: no RAM pool");
        return;
    };
    let Some(mut dma) = rt::take_dma(rp_hal_pxa27x::dma_map::SD) else {
        log!("  BX: no DMA channel");
        return;
    };
    let src = &mut ram[..PIECE * PIECES / 4];
    for (k, w) in src.iter_mut().enumerate() {
        *w = (k as u32).wrapping_mul(0x9E37_79B9) ^ 0x5A5A_0000;
    }
    let want = fnv(&mut src.iter().copied());
    let bytes = words_as_bytes(src);
    let Some(dst) = g.pool.alloc((PIECE * PIECES) as u32, 0x1000) else {
        log!("  BX: no 2700G memory");
        return;
    };

    // P12.
    for burst in [8u32, 16, 32] {
        for (wname, w) in [
            ("0", None),
            ("byte", Some(Width::Byte)),
            ("half", Some(Width::Half)),
            ("word", Some(Width::Word)),
        ] {
            // The destination cleared first, so a copy that moved nothing
            // can't pass.
            let zero = [0u32; 256];
            for k in 0..(PIECE * PIECES) / 1024 {
                g.mem.store(dst as usize + 1024 * k, &zero);
            }
            g.mem.sync();
            rt::check_in();
            let t0 = rt::ticks();
            let (mut ok, mut arm_us) = (true, 0u32);
            for (k, piece) in bytes.chunks(PIECE).enumerate() {
                let ta = rt::ticks();
                let armed = dma.arm_copy_to_device_memory_shaped(
                    piece,
                    CS3 + dst + (k * PIECE) as u32,
                    burst,
                    w,
                );
                arm_us += us_since(ta);
                let tw = rt::ticks();
                let mut done = None;
                while armed && us_since(tw) < 200_000 {
                    if let Some(r) = dma.done() {
                        done = Some(r);
                        break;
                    }
                }
                if armed && done.is_none() {
                    dma.abort();
                }
                ok &= armed && done == Some(true);
            }
            let us = us_since(t0);
            let got = fnv(&mut (0..(PIECE * PIECES) / 4)
                .map(|k| g.mem.load(dst as usize + 4 * k).unwrap_or(0)));
            log!(
                "  BX P12 burst {burst} width {wname}: 960 KiB in {us} us ({} KB/s), the CPU arming {arm_us} us; done {ok}; the copy right: {}",
                (PIECE * PIECES) as u64 * 1000 / u64::from(us.max(1)),
                got == want
            );
            rt::check_in();
        }
    }

    // V17: bit 10 (and the rest) through scan-out changes.
    let status = |g: &mut Rig| g.e.regs.read32(MBX + reg::STATUS);
    let phase = |g: &mut Rig, what: &str, f: &mut dyn FnMut(&mut Rig, u32)| {
        let (mut or, mut and) = (0u32, u32::MAX);
        for k in 0..30 {
            rt::check_in();
            g.vevent_clear();
            let _ = g.vevent_wait();
            f(g, k);
            for _ in 0..40 {
                let s = status(g);
                or |= s;
                and &= s;
                rt::delay_us(200);
            }
        }
        log!(
            "  BX V17 {what}: 0x12c ever set {or:08X}, always set {and:08X}; bit 10 ever {}",
            or >> 10 & 1
        );
    };
    let shown = g.e.regs.read32(dreg::GSCADR);
    phase(g, "idle (30 frames)", &mut |_, _| {});
    phase(
        g,
        "the graphics plane repointed every frame",
        &mut |g, k| {
            let v = if k % 2 == 0 { shown + 0x100 } else { shown };
            g.e.regs.write32(dreg::GSCADR, v);
        },
    );
    g.e.regs.write32(dreg::GSCADR, shown);
    let vregs = video_regs(
        dst,
        960,
        Layer {
            slot: Slot::Middle,
            blend: Blend::Global(0x40),
        },
    );
    phase(g, "the video stream on and off every frame", &mut |g, k| {
        if k % 2 == 0 {
            video_on(g, &vregs);
        } else {
            g.e.regs.write32(lreg::VSCADR, 0);
        }
    });
    g.e.regs.write32(lreg::VSCADR, 0);
    phase(g, "a DMA into local memory each frame", &mut |g, _| {
        let _ = dma.arm_copy_to_device_memory(&bytes[..PIECE], CS3 + dst);
        let t = rt::ticks();
        while us_since(t) < 100_000 && dma.done().is_none() {}
        let _ = g;
    });
    let _ = g.pool.free(dst, (PIECE * PIECES) as u32);
    log!("BX: done");
}
