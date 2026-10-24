//! Run FI2 (`RP_GPUBENCH_STATUS2`): the gate's P18, what run FI left
//! (docs/research/2700g-full-potential.md): the system block's interrupt
//! status (`MINTSTAT` `0x1C`, `SINTSTAT` `0x24`, both read safely before:
//! run S's snapshot, `rp-glesfeat` G9) and the display's `DINTRS`, beside
//! the MBX status word `0x12c`, polled while (1) idle, (2) a 2D batch that
//! fills the FIFO drains, (3) a 3D render runs. Reads only: no enable is
//! changed. Every bit that ever set (OR) and ever cleared (AND) logged for
//! each phase, and for (2) the FIFO level (`0x12c` bits 16-23, bit 30 the
//! ninth) against the time, so a low-water mark would show as a bit
//! following the level. Not here: the TA's done bit (P4's TA code) and
//! MVED1 (phase 7).

use super::*;

const MINTSTAT: usize = 0x1C;
const SINTSTAT: usize = 0x24;

/// OR and AND of each word over a phase.
#[derive(Clone, Copy)]
struct Seen {
    or: [u32; 4],
    and: [u32; 4],
    n: u32,
}

impl Seen {
    const fn new() -> Self {
        Self {
            or: [0; 4],
            and: [u32::MAX; 4],
            n: 0,
        }
    }

    fn take(&mut self, g: &mut Rig) -> [u32; 4] {
        let w = [
            g.e.regs.read32(MBX + reg::STATUS),
            g.e.regs.read32(MINTSTAT),
            g.e.regs.read32(SINTSTAT),
            g.e.regs.read32(dreg::DINTRS),
        ];
        for k in 0..4 {
            self.or[k] |= w[k];
            self.and[k] &= w[k];
        }
        self.n += 1;
        w
    }

    fn log(&self, phase: &str) {
        log!(
            "  FI2 {phase}: {} reads; ever set / always set: 0x12c {:08X}/{:08X}, MINTSTAT {:08X}/{:08X}, SINTSTAT {:08X}/{:08X}, DINTRS {:08X}/{:08X}",
            self.n,
            self.or[0],
            self.and[0],
            self.or[1],
            self.and[1],
            self.or[2],
            self.and[2],
            self.or[3],
            self.and[3]
        );
    }
}

fn level(status: u32) -> u32 {
    (status >> 16 & 0xFF) | (status >> 30 & 1) << 8
}

#[inline(never)]
pub(super) fn run(g: &mut Rig) {
    const PIC: u32 = 480 * 640 * 2;
    log!(
        "FI2: MINTSTAT, SINTSTAT, DINTRS and 0x12c while idle, while a full 2D FIFO drains, while a 3D render runs (reads only); enables as found: MINTENABLE {:08X} SINTENABLE {:08X} DINTRE {:08X} 0x130 {:08X}",
        g.e.regs.read32(0x18),
        g.e.regs.read32(0x20),
        g.e.regs.read32(dreg::DINTRE),
        g.e.regs.read32(MBX + reg::ENABLE)
    );
    clock_card(g, ["RUN FI2", "STATUS BITS", "(P18)"], 0xFF20_2040, 2);

    // (1) Idle, 20 ms.
    let mut idle = Seen::new();
    let t0 = rt::ticks();
    while us_since(t0) < 20_000 {
        idle.take(g);
        rt::delay_us(50);
    }
    idle.log("idle 20 ms");

    // (2) The FIFO filled with fills and drained, the level logged.
    let Some(pic) = g.pool.alloc(PIC, 0x1000) else {
        log!("  FI2: no memory for the picture");
        return;
    };
    let mut batch = [0u32; 7 * 60];
    for k in 0..60usize {
        let (x, y) = ((k % 10) as u16 * 48, (k / 10) as u16 * 100);
        let w = rp_image::need(
            cmd::fill(
                Format::RGB565,
                pic as usize,
                960,
                (x, y, x + 48, y + 100),
                0x0841 * k as u32,
            ),
            "a 2D rectangle",
        );
        batch[7 * k..7 * k + 7].copy_from_slice(&w);
    }
    let mut drain = Seen::new();
    let sent = g.e.submit(&batch).is_ok();
    let t0 = rt::ticks();
    let mut marks = [(0u32, 0u32, 0u32); 12];
    let (mut m, mut last) = (0usize, u32::MAX);
    while us_since(t0) < 50_000 {
        let w = drain.take(g);
        let lv = level(w[0]);
        if lv / 32 != last / 32 && m < marks.len() {
            marks[m] = (us_since(t0), lv, w[1] ^ idle.and[1] | w[2] ^ idle.and[2]);
            m += 1;
            last = lv;
        }
        if lv == 0 && us_since(t0) > 1_000 {
            break;
        }
    }
    let done = g.e.finish(g.mem.0, 200_000).is_ok();
    drain.log("a full 2D FIFO draining");
    for &(us, lv, diff) in &marks[..m] {
        log!(
            "    FI2 at {us} us the FIFO level {lv} words; the system status bits differing from idle {diff:08X}"
        );
    }
    log!("  FI2 2D: {} words sent {sent}, done {done}", batch.len());
    let _ = g.pool.free(pic, PIC);

    // (3) A 3D render (one full-screen textured quad), polled.
    let Some(mut r) = Renderer::new(&mut g.pool, 480, 640) else {
        log!("  FI2: no renderer");
        return;
    };
    let _ = r.set_background(Some(GREY));
    if let Some(t) = tex16(g, 6, format::RGB565, 1, &pattern565) {
        g.set(&mut r, 1, Some(cover(t, 16, false)));
        let mut render = Seen::new();
        r.invalidate_all();
        let res = r.render(
            &mut g.e.regs,
            &mut g.mem,
            &mut g.clock,
            Target {
                picture: PIC16,
                x: 0,
                y: 0,
            },
        );
        g.sync_resets(&r);
        let t0 = rt::ticks();
        let mut done_at = None;
        if let Ok(Some(_)) = res {
            while us_since(t0) < 200_000 {
                let w = render.take(g);
                if w[0] & RENDER_DONE != 0 {
                    done_at = Some(us_since(t0));
                    break;
                }
            }
            loop {
                match r.poll(&mut g.e.regs, &mut g.mem, &mut g.clock) {
                    Poll::Wait(_) => {}
                    Poll::Done(_) => break,
                    p => {
                        g.sync_resets(&r);
                        log!("    FI2 3D: {p:?}");
                        break;
                    }
                }
            }
        }
        render.log("a 3D render");
        let mut after = Seen::new();
        after.take(g);
        after.log("after the render was polled done");
        log!(
            "  FI2 3D: started {:?}, the status's render bit after {done_at:?} us",
            res.is_ok()
        );
        g.set(&mut r, 1, None);
        let _ = rp_mbx3d_render::release(&mut g.pool, t);
    }
    let _ = r.release(&mut g.pool);
    log!("FI2: done");
}
