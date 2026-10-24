//! Run CH (`RP_GPUBENCH_HARNESS`): the compositor harness, a benchmark only
//! (docs/design/compositor-harness.md; the gate's CPU T1, T3 and T4: the
//! owner's vote 2026-10-21, option a). The frames of
//! docs/design/workload-brief.md, drawn the way compositor.md and ADR-0034
//! say the compositor will, at whatever clock the loader leaves (no clock
//! is changed: ADR-0035 rules 2 and 8), each timed:
//!
//! - **the planes** (W1, W2, W4, W9 and the three measured-but-not-critical
//!   frames): the page on the video plane (UYVY: the driver's video formats;
//!   compositor.md wants RGB565 there, the same two bytes a pixel), the
//!   glass on the graphics plane (ARGB8888 per-pixel alpha), each frame's
//!   repoint written at the frame event; W2's changing row and W4's lit key
//!   drawn by the CPU, by 2D commands the CPU writes, by the same commands
//!   moved by DMA (T3), and the row's pixels moved by DMA;
//! - **the 3D core** (W5, W6, W3) **in turns and as a pipeline** (T1): in
//!   turns the CPU prepares the next caption (drawn and put in texture
//!   order in SDRAM), then the core renders; as a pipeline the CPU prepares
//!   it while the core renders. 2D and 3D never overlap (ADR-0030 G2, P8);
//!   the 3D core composes only textures (ADR-0034 Gap 1). Both ways must end
//!   on the same picture (its hash), or the pipeline way is reported broken;
//! - **W7's rotation** by the 2D engine's rotated copy (B11) and by the 3D
//!   core (a textured screen, its corners turned);
//! - **the routing table (T4)**: each kind of work's time on each unit.
//!
//! Every sequence is labelled on screen (STEP n OF m). The frame budget is
//! [`FRAME_US`]; a frame is late when its work is still going at the next
//! frame event.

use super::*;

/// The page's and the glass's lines.
const VSTRIDE: u32 = 960;
const GSTRIDE: u32 = 480 * 4;
/// The page: two screens (C4's tall page).
const PAGE_ROWS: u32 = 1280;
/// The glass picture: two screens (a sheet slides over a taller picture).
const GLASS_ROWS: u32 = 1280;
/// Frames a sequence at most.
const MAXF: usize = 160;
/// Sequences, for the label.
const STEPS: u32 = 16;
/// W2's row: a list row, 480 x 48 UYVY, 46,080 bytes (whole cache lines).
const ROW_H: u32 = 48;
const ROW_BYTES: usize = (480 * 2 * ROW_H) as usize;
/// W4's keys: 10 x 4 of 44 x 52 pixels at 48 x 60 pitch.
const KEY: (u32, u32, u32, u32) = (44, 52, 48, 60);
/// W4's keyboard panel height, on the glass picture's second screen.
const KBD_H: u32 = 240;
/// The 3D sequences' frames.
const FRAMES_3D: u32 = 60;

/// Frame times of one sequence.
struct Seq {
    work: [u32; MAXF],
    busy: [u32; MAXF],
    n: usize,
    late: u32,
    lost: bool,
}

impl Seq {
    const fn new() -> Self {
        Self {
            work: [0; MAXF],
            busy: [0; MAXF],
            n: 0,
            late: 0,
            lost: false,
        }
    }

    fn add(&mut self, work: u32, busy: u32, late: bool) {
        if self.n < MAXF {
            self.work[self.n] = work;
            self.busy[self.n] = busy;
            self.n += 1;
        }
        self.late += u32::from(late);
    }

    fn stats(v: &[u32]) -> (u32, u32) {
        let mut s = [0u32; MAXF];
        s[..v.len()].copy_from_slice(v);
        s[..v.len()].sort_unstable();
        if v.is_empty() {
            (0, 0)
        } else {
            (s[v.len() / 2], s[v.len() - 1])
        }
    }

    /// Median and worst work, the share over `budget`, late frames, and the
    /// CPU's own share of the frame time (its busy time over the frames).
    fn report(&self, name: &str, budget: u32) {
        let (med, worst) = Self::stats(&self.work[..self.n]);
        let (bmed, bworst) = Self::stats(&self.busy[..self.n]);
        let over = self.work[..self.n].iter().filter(|&&w| w > budget).count();
        let cpu: u64 = self.busy[..self.n].iter().map(|&b| u64::from(b)).sum();
        let all = (self.n as u64 * u64::from(budget)).max(1);
        log!(
            "  CH {name}: {} frames, work median {med} us worst {worst} us, {over} over {budget} us, {} late, CPU busy median {bmed} us worst {bworst} us ({}% of the frames' time){}",
            self.n,
            self.late,
            cpu * 100 / all,
            if self.lost {
                ", THE FRAME EVENT STOPPED"
            } else {
                ""
            }
        );
    }
}

/// Runs `step` once a frame, right after the frame event, `n` times: what
/// it returns is the time it spent waiting on the 2700G (the CPU free).
fn frames(g: &mut Rig, n: u32, step: &mut dyn FnMut(&mut Rig, u32) -> u32) -> Seq {
    let mut s = Seq::new();
    for k in 0..n {
        rt::check_in();
        g.vevent_clear();
        if !g.vevent_wait() {
            s.lost = true;
            break;
        }
        g.vevent_clear();
        let t0 = rt::ticks();
        let waited = step(g, k);
        let work = us_since(t0);
        let late = g.e.regs.read32(dreg::DINTRS) & event::VEVENT0 != 0;
        s.add(work, work.saturating_sub(waited), late);
    }
    s
}

/// The 2D engine's work submitted and waited for: (done, the CPU's time
/// writing it, the time waited).
fn submit_wait(g: &mut Rig, words: &[u32]) -> (bool, u32, u32) {
    let t0 = rt::ticks();
    let sent = g.e.submit(words).is_ok();
    let write = us_since(t0);
    let t1 = rt::ticks();
    let done = sent && g.e.finish(g.mem.0, 200_000).is_ok();
    (done, write, us_since(t1))
}

/// A DMA source: whole 32-byte lines.
#[repr(align(32))]
struct Batch([u32; 64]);

/// `words` (at most 64) into the 2D command window by DMA, the window's
/// wrap kept clear (run FD's way), then waited for: (done, the CPU's time
/// arming it, the time waited).
fn dma_wait(g: &mut Rig, dma: &mut rp_hal_pxa27x::dma::Dma, words: &[u32]) -> (bool, u32, u32) {
    let mut b = Batch([cmd::FLUSH; 64]);
    let n = words.len().min(56);
    b.0[..n].copy_from_slice(&words[..n]);
    // Whole 32-byte lines: n rounded up, the rest FLUSH words.
    let len = n.div_ceil(8) * 8;
    let at = g.e.window_offset();
    if at + 4 * len > cmd::PORT_SIZE {
        let pad = [cmd::FLUSH; 64];
        let mut left = (cmd::PORT_SIZE - at) / 4;
        while left > 0 {
            let k = left.min(pad.len());
            let _ = g.e.submit(&pad[..k]);
            left -= k;
        }
    }
    let at = g.e.window_offset();
    let t0 = rt::ticks();
    let armed = dma.arm_copy_to_device_memory(
        words_as_bytes(&b.0[..len]),
        REGS_PHYS + (cmd::PORT + at) as u32,
    );
    let arm = us_since(t0);
    let t1 = rt::ticks();
    let mut moved = false;
    while armed && us_since(t1) < 200_000 {
        if let Some(ok) = dma.done() {
            moved = ok;
            break;
        }
    }
    if armed && !moved {
        dma.abort();
    }
    if moved {
        g.e.advanced(4 * len);
    }
    let done = moved && g.e.finish(g.mem.0, 200_000).is_ok();
    (done, arm, us_since(t1))
}

/// SDRAM bytes `src` (whole cache lines) to 2700G memory at `dst` by DMA,
/// waited for: (done, arming time, the time waited).
fn dma_pixels(dma: &mut rp_hal_pxa27x::dma::Dma, src: &[u8], dst: u32) -> (bool, u32, u32) {
    let t0 = rt::ticks();
    let armed = dma.arm_copy_to_device_memory(src, CS3 + dst);
    let arm = us_since(t0);
    let t1 = rt::ticks();
    let mut moved = false;
    while armed && us_since(t1) < 200_000 {
        if let Some(ok) = dma.done() {
            moved = ok;
            break;
        }
    }
    if armed && !moved {
        dma.abort();
    }
    (moved, arm, us_since(t1))
}

/// The label text for step `k`.
fn step_name(k: u32, what: &str) -> Name {
    let mut l = heapless_name();
    let _ = write!(l, "CH STEP {k} OF {STEPS}: {what}");
    l
}

/// The glass: `top` rows of dark glass holding the label (two lines) at
/// glass row `label_y`, a sheet (`sheet`: its rows, a blue-grey tint at
/// alpha 0xD0) and the rest clear. `kbd`: W4's keyboard on the second
/// screen.
fn glass_px(x: u32, y: u32, label: &str, label_y: u32, sheet: (u32, u32), kbd: bool) -> u32 {
    if (label_y..label_y + 64).contains(&y) {
        if in_text(x, y, label, 8, label_y + 10, 2) {
            return 0xFF_FF_FF_FF;
        }
        return 0xC0_00_00_00;
    }
    if kbd && (640..640 + KBD_H).contains(&y) {
        let (kw, kh, px, py) = KEY;
        let (cx, cy) = ((x.saturating_sub(4)) % px, (y - 640).saturating_sub(4) % py);
        if x >= 4 && y >= 644 && cx < kw && cy < kh && x < 4 + 10 * px {
            return 0xF0_E8_E8_F0;
        }
        return 0xE8_30_34_40;
    }
    if (sheet.0..sheet.1).contains(&y) {
        let row = (y - sheet.0) / 64;
        if in_text(x, y, "NOTIFICATION", 24, sheet.0 + row * 64 + 20, 3) {
            return 0xFF_FF_FF_FF;
        }
        return 0xD0_40_50_70;
    }
    0
}

fn draw_glass(g: &mut Rig, at: u32, label: &str, label_y: u32, sheet: (u32, u32), kbd: bool) {
    write_argb(g, at, 480, GLASS_ROWS, &|x, y| {
        glass_px(x, y, label, label_y, sheet, kbd)
    });
}

/// The glass's `GSCADR` with its first shown row `row`.
fn glass_at(at: u32, row: u32) -> u32 {
    gscadr_as(
        at + row * GSTRIDE,
        GSTRIDE,
        Layer {
            slot: Slot::Middle,
            blend: Blend::PixelAlpha,
        },
    )
}

const PAGE: Layer = Layer {
    slot: Slot::Bottom,
    blend: Blend::Opaque,
};

/// The video plane's `VSCADR` with the page's first shown row `row`.
fn page_at(at: u32, row: u32) -> u32 {
    vscadr_of(&video_regs(at + row * VSTRIDE, VSTRIDE, PAGE))
}

/// One of the routing table's rows (T4), logged as measured.
fn route(kind: &str, unit: &str, us: u32, cpu: u32, ok: bool) {
    log!("  CH T4 {kind} | {unit} | {us} us | CPU {cpu} us | done {ok}");
}

/// What the plane frames share.
struct Planes {
    page: u32,
    glass: u32,
    atlas: u32,
}

#[inline(never)]
pub(super) fn run(g: &mut Rig) {
    log!(
        "CH: the compositor harness (docs/design/compositor-harness.md): W1, W2, W4, W9, toast, volume, wake on the planes; W5, W6, W3 on the 3D core in turns and as a pipeline (T1); W7; the routing table (T4); clock as found: COREPLL {:08X} MBXCLK {:08X}",
        g.e.regs.read32(clock::COREPLL),
        g.e.regs.read32(MBXCLK)
    );
    let Some(scratch) = rp_hal_x51v::devices::take_scratch() else {
        log!("  H: no scratch SDRAM");
        return;
    };
    let Some(mut dma) = rt::take_dma(rp_hal_pxa27x::dma_map::SD) else {
        log!("  H: no DMA channel");
        return;
    };
    let (Some(page), Some(glass), Some(atlas)) = (
        g.pool.alloc(VSTRIDE * PAGE_ROWS, 0x1000),
        g.pool.alloc(GSTRIDE * GLASS_ROWS, 0x1000),
        g.pool.alloc(ROW_BYTES as u32, 0x1000),
    ) else {
        log!("  H: no 2700G memory for the planes");
        return;
    };
    let p = Planes { page, glass, atlas };
    plane_frames(g, &p, &mut dma, scratch);
    let _ = g.pool.free(page, VSTRIDE * PAGE_ROWS);
    let _ = g.pool.free(glass, GSTRIDE * GLASS_ROWS);
    let _ = g.pool.free(atlas, ROW_BYTES as u32);
    g.e.regs.write32(lreg::VSCADR, 0);
    three_d_frames(g, scratch);
    log!("CH: done");
}

/// The new row's pixels (`ROW n` in the list's colours, `NEW` in red) as
/// UYVY into `out` (480 x 48).
fn new_row(out: &mut [u8], n: u32) {
    let mut label = heapless_name();
    let _ = write!(label, "NEW ROW {n:02}");
    let f = |x: u32, y: u32| -> u32 {
        if in_text(x, y, label.as_str(), 16, 14, 3) {
            0x00_D0_10_10
        } else if y == ROW_H - 1 {
            0x00_20_A0_40
        } else {
            0x00_FF_F0_C0
        }
    };
    for y in 0..ROW_H {
        for k in 0..240u32 {
            let x = 2 * k;
            let yuv = |c: u32| scanout::rgb_to_yuv((c >> 16) as u8, (c >> 8) as u8, c as u8);
            let (y0, u0, v0) = yuv(f(x, y));
            let (y1, u1, v1) = yuv(f(x + 1, y));
            let u = (u32::from(u0) + u32::from(u1)).div_ceil(2);
            let v = (u32::from(v0) + u32::from(v1)).div_ceil(2);
            let w = u | u32::from(y0) << 8 | v << 16 | u32::from(y1) << 24;
            let o = ((y * 240 + k) * 4) as usize;
            out[o..o + 4].copy_from_slice(&w.to_le_bytes());
        }
    }
}

#[inline(never)]
fn plane_frames(g: &mut Rig, p: &Planes, dma: &mut rp_hal_pxa27x::dma::Dma, scratch: &mut [u8]) {
    let mut step = 1;
    // The page (the list, two screens) and the row atlas.
    let t0 = rt::ticks();
    write_uyvy(g, p.page, 480, PAGE_ROWS, &list_rgb);
    log!(
        "  CH page drawn by the CPU (480 x 1280 UYVY): {} ms",
        us_since(t0) / 1000
    );
    let row = &mut scratch[..ROW_BYTES];
    new_row(row, 0);
    for (k, c) in row.chunks_exact(4).enumerate() {
        let w = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
        g.mem.store(p.atlas as usize + 4 * k, &[w]);
    }
    g.mem.sync();

    // W1: the page panned 4 lines a frame under the glass header.
    let name = step_name(step, "W1 HOME PAN");
    draw_glass(g, p.glass, name.as_str(), 0, (0, 0), false);
    video_on(g, &video_regs(p.page, VSTRIDE, PAGE));
    g.e.regs.write32(dreg::GSCADR, glass_at(p.glass, 0));
    graphics_on(
        g,
        p.glass,
        GSTRIDE,
        Layer {
            slot: Slot::Middle,
            blend: Blend::PixelAlpha,
        },
    );
    let s = frames(g, MAXF as u32, &mut |g, k| {
        g.e.regs
            .write32(lreg::VSCADR, page_at(p.page, (k * 4) % 640));
        0
    });
    s.report("W1 pan (VSCADR a frame)", FRAME_US);
    step += 1;

    // W2: the same pan; every 20th frame a row changes off screen, drawn
    // four ways (T3, T4), each its own pass.
    let ways = ["CPU STORES", "2D BY CPU", "2D BY DMA", "DMA PIXELS"];
    for (w, way) in ways.iter().enumerate() {
        let mut label = heapless_name();
        let _ = write!(label, "W2 LIST, ROW: {way}");
        let name = step_name(step, label.as_str());
        draw_glass(g, p.glass, name.as_str(), 0, (0, 0), false);
        let (mut row_us, mut row_cpu, mut rows, mut rows_ok, mut rows_late) =
            (Times::default(), Times::default(), 0u32, 0u32, 0u32);
        let s = frames(g, MAXF as u32, &mut |g, k| {
            let pos = (k * 4) % 640;
            g.e.regs.write32(lreg::VSCADR, page_at(p.page, pos));
            if k % 20 != 10 {
                return 0;
            }
            // The row just below the screen, on a 48-line boundary.
            let y = (pos + 640).div_ceil(ROW_H) * ROW_H;
            if y + ROW_H > PAGE_ROWS {
                return 0;
            }
            let dst = p.page + y * VSTRIDE;
            let t0 = rt::ticks();
            let (ok, cpu, waited) = match w {
                0 => {
                    for (i, c) in scratch[..ROW_BYTES].chunks_exact(32).enumerate() {
                        let mut words = [0u32; 8];
                        for (j, d) in words.iter_mut().enumerate() {
                            *d = u32::from_le_bytes([
                                c[4 * j],
                                c[4 * j + 1],
                                c[4 * j + 2],
                                c[4 * j + 3],
                            ]);
                        }
                        g.mem.store(dst as usize + 32 * i, &words);
                    }
                    g.mem.sync();
                    (true, us_since(t0), 0)
                }
                1 | 2 => {
                    let (c, n) = rp_image::need(
                        cmd::copy(
                            Format::RGB565,
                            (dst as usize, VSTRIDE as usize),
                            (p.atlas as usize, VSTRIDE as usize),
                            (0, 0),
                            (0, 0, 480, ROW_H as u16),
                            0,
                        ),
                        "a 2D rectangle",
                    );
                    if w == 1 {
                        let (ok, write, waited) = submit_wait(g, &c[..n]);
                        (ok, write, waited)
                    } else {
                        dma_wait(g, dma, &c[..n])
                    }
                }
                _ => {
                    let (ok, arm, waited) = dma_pixels(dma, &scratch[..ROW_BYTES], dst);
                    (ok, arm, waited)
                }
            };
            let all = us_since(t0);
            rows += 1;
            rows_ok += u32::from(ok);
            row_us.add(all);
            row_cpu.add(cpu);
            rows_late += u32::from(g.e.regs.read32(dreg::DINTRS) & event::VEVENT0 != 0);
            waited
        });
        let mut what = heapless_name();
        let _ = write!(what, "W2 pan + a row by {way}");
        s.report(what.as_str(), FRAME_US);
        log!(
            "  CH W2 row by {way}: {rows} rows ({rows_ok} done), each {} us median ({}-{}), the CPU {} us median, {rows_late} late",
            row_us.median(),
            row_us.min(),
            row_us.max(),
            row_cpu.median()
        );
        route(
            "a list row 480 x 48",
            way,
            row_us.median(),
            row_cpu.median(),
            rows_ok == rows,
        );
        step += 1;
    }
    // The window re-based: one 2D copy of a screen's rows to the top.
    {
        let (c, n) = rp_image::need(
            cmd::copy(
                Format::RGB565,
                (p.page as usize, VSTRIDE as usize),
                ((p.page + 320 * VSTRIDE) as usize, VSTRIDE as usize),
                (0, 0),
                (0, 0, 480, 640),
                0,
            ),
            "a 2D rectangle",
        );
        let t0 = rt::ticks();
        let (ok, cpu, _) = submit_wait(g, &c[..n]);
        let us = us_since(t0);
        log!("  CH W2 re-base: a 2D copy of 480 x 640 within the page: {us} us, done {ok}");
        route("a screen copy 480 x 640", "2D BY CPU", us, cpu, ok);
        // The page back as it was, for the rest.
        write_uyvy(g, p.page, 480, PAGE_ROWS, &list_rgb);
    }

    // W4: the keyboard slides up (the glass repointed over its taller
    // picture, 16 lines a frame), then ten taps light keys three ways.
    let name = step_name(step, "W4 KEYBOARD");
    draw_glass(g, p.glass, name.as_str(), 240, (0, 0), true);
    g.e.regs.write32(lreg::VSCADR, page_at(p.page, 0));
    let s = frames(g, KBD_H / 16, &mut |g, k| {
        g.e.regs
            .write32(dreg::GSCADR, glass_at(p.glass, (k + 1) * 16));
        0
    });
    s.report("W4 keyboard slide (GSCADR a frame)", FRAME_US);
    let key_ways = ["CPU STORES", "2D BY CPU", "2D BY DMA"];
    for (w, way) in key_ways.iter().enumerate() {
        let (mut tap, mut shown, mut cpu_t, mut ok_all) =
            (Times::default(), Times::default(), Times::default(), true);
        for t in 0..8u32 {
            rt::check_in();
            g.vevent_clear();
            if !g.vevent_wait() {
                break;
            }
            g.vevent_clear();
            let (kw, kh, px, py) = KEY;
            let (kx, ky) = (4 + (t * 3 + w as u32) % 10 * px, 640 + 4 + (t % 4) * py);
            let t0 = rt::ticks();
            let lit = 0xFF_40_90_FF;
            let (ok, cpu) = match w {
                0 => {
                    let line = [lit; 44];
                    for yy in 0..kh {
                        g.mem.store(
                            (p.glass + (ky + yy) * GSTRIDE + kx * 4) as usize,
                            &line[..kw as usize],
                        );
                    }
                    g.mem.sync();
                    (true, us_since(t0))
                }
                _ => {
                    let c = rp_image::need(
                        cmd::fill(
                            Format::BPP32,
                            p.glass as usize,
                            GSTRIDE as usize,
                            (kx as u16, ky as u16, (kx + kw) as u16, (ky + kh) as u16),
                            lit,
                        ),
                        "a 2D rectangle",
                    );
                    let (ok, cpu, _) = if w == 1 {
                        submit_wait(g, &c)
                    } else {
                        dma_wait(g, dma, &c)
                    };
                    (ok, cpu)
                }
            };
            let lit_us = us_since(t0);
            g.vevent_wait();
            tap.add(lit_us);
            shown.add(us_since(t0));
            cpu_t.add(cpu);
            ok_all &= ok;
        }
        log!(
            "  CH W4 tap by {way}: key lit in {} us median ({}-{}), shown (next frame event) {} us median, worst {}; the CPU {} us",
            tap.median(),
            tap.min(),
            tap.max(),
            shown.median(),
            shown.max(),
            cpu_t.median()
        );
        route("a key 44 x 52", way, tap.median(), cpu_t.median(), ok_all);
    }
    step += 1;

    // W9: the shade pulled down: the glass's first shown row from 640 to 0,
    // 32 lines a frame (the sheet is the picture's first screen).
    let name = step_name(step, "W9 SHADE");
    draw_glass(g, p.glass, name.as_str(), 640, (0, 576), false);
    let s = frames(g, 20, &mut |g, k| {
        g.e.regs
            .write32(dreg::GSCADR, glass_at(p.glass, 640 - (k + 1) * 32));
        0
    });
    s.report("W9 shade (GSCADR a frame)", FRAME_US);
    step += 1;

    // Toast: an 80-line sheet in and out.
    let name = step_name(step, "TOAST");
    draw_glass(g, p.glass, name.as_str(), 560, (0, 80), false);
    let s = frames(g, 40, &mut |g, k| {
        let row = if k < 10 {
            80 - (k + 1) * 8
        } else if k < 30 {
            0
        } else {
            (k - 29) * 8
        };
        g.e.regs
            .write32(dreg::GSCADR, glass_at(p.glass, row.min(80)));
        0
    });
    s.report("toast in and out", FRAME_US);
    step += 1;

    // Volume: a slider drawn into the glass by two 2D fills a frame.
    let name = step_name(step, "VOLUME OVERLAY");
    draw_glass(g, p.glass, name.as_str(), 0, (0, 0), false);
    g.e.regs.write32(dreg::GSCADR, glass_at(p.glass, 0));
    let s = frames(g, 30, &mut |g, k| {
        let fill = 41 + k * 13;
        let mut words = [0u32; 14];
        for (j, (rect, c)) in [
            ((40u16, 300u16, 440u16, 340u16), 0xC0_20_20_28u32),
            ((40, 300, fill as u16, 340), 0xFF_F0_F0_F0),
        ]
        .into_iter()
        .enumerate()
        {
            let w = rp_image::need(
                cmd::fill(Format::BPP32, p.glass as usize, GSTRIDE as usize, rect, c),
                "a 2D rectangle",
            );
            words[7 * j..7 * j + 7].copy_from_slice(&w);
        }
        let (_, _, waited) = submit_wait(g, &words);
        waited
    });
    s.report("volume slider (2D fills a frame)", FRAME_US);
    step += 1;

    // Wake: the page fades in, its global alpha ramped over 15 frames.
    let name = step_name(step, "WAKE FADE");
    draw_glass(g, p.glass, name.as_str(), 0, (0, 0), false);
    let s = frames(g, 16, &mut |g, k| {
        let a = (k * 17).min(255) as u8;
        let regs = video_regs(
            p.page,
            VSTRIDE,
            Layer {
                slot: Slot::Bottom,
                blend: Blend::Global(a),
            },
        );
        for &(off, v) in &regs {
            if off == lreg::VBBASE || off == lreg::VSCADR {
                g.e.regs.write32(off, v);
            }
        }
        0
    });
    s.report("wake fade (VBBASE a frame)", FRAME_US);
    log!("  CH plane steps done: {step} of {STEPS}");
}

// ---- the 3D frames ---------------------------------------------------------

/// The caption's texture side (code 4: 128) and the rows it uses.
const CAP_CODE: u8 = 4;
const CAP_ROWS: u32 = 32;

/// The CPU's own work for a frame (T1): the caption's two lines drawn and
/// put in texture order (ARGB4444) in SDRAM `out` (128 x 128 x 2 bytes).
fn prepare_caption(out: &mut [u8], l1: &str, l2: &str) {
    let side = 8u32 << CAP_CODE;
    for i in 0..side * side {
        let (x, y) = untwiddle(i);
        let v: u16 = if y >= CAP_ROWS {
            0
        } else if in_text(x, y, l1, 4, 4, 1) || in_text(x, y, l2, 4, 18, 1) {
            0xFFFF
        } else {
            0xC112
        };
        out[2 * i as usize..2 * i as usize + 2].copy_from_slice(&v.to_le_bytes());
    }
}

/// The prepared caption into its texture (CPU stores).
fn upload_caption(g: &mut Rig, tex: Texture, src: &[u8]) {
    for (k, c) in src.chunks_exact(4).enumerate() {
        g.mem.store(
            tex.at as usize + 4 * k,
            &[u32::from_le_bytes([c[0], c[1], c[2], c[3]])],
        );
    }
    g.mem.sync();
}

fn full_uv() -> [(i32, i32); 4] {
    [(0, 0), (UV_ONE, 0), (UV_ONE, UV_ONE), (0, UV_ONE)]
}

fn textured(x: i32, y: i32, w: i32, h: i32, tex: Texture, uv: [(i32, i32); 4]) -> Draw {
    Draw::rect(
        x,
        y,
        w,
        h,
        Paint::Texture {
            tex,
            uv,
            tint: WHITE,
        },
    )
}

/// One 3D frame split around the CPU's work: the render started, `during`
/// run (the pipeline), the end waited for. (render µs exact, the
/// renderer's predicted µs, tiles, the time waited after `during`).
fn render_split(
    g: &mut Rig,
    r: &mut Renderer,
    t: Target,
    during: &mut dyn FnMut(&mut Rig),
) -> Option<(u32, u32, u32, u32)> {
    rt::check_in();
    let res = r.render(&mut g.e.regs, &mut g.mem, &mut g.clock, t);
    g.sync_resets(r);
    let mut at = match res {
        Ok(Some(at)) => at,
        Ok(None) => {
            during(g);
            return Some((0, 0, 0, 0));
        }
        Err(err) => {
            log!("    render refused: {err:?}");
            return None;
        }
    };
    let t0 = g.clock.now_us();
    during(g);
    let t1 = g.clock.now_us();
    let mut exact = None;
    while g.clock.now_us().wrapping_sub(t0) < 500_000 {
        if g.e.regs.read32(MBX + reg::STATUS) & RENDER_DONE != 0 {
            exact = Some(g.clock.now_us().wrapping_sub(t0));
            break;
        }
    }
    let waited = g.clock.now_us().wrapping_sub(t1);
    loop {
        while (g.clock.now_us().wrapping_sub(at) as i32) < 0 {}
        match r.poll(&mut g.e.regs, &mut g.mem, &mut g.clock) {
            Poll::Wait(t) => at = t,
            Poll::Done(st) => {
                g.measured += 1;
                return exact.map(|us| (us, st.predicted_us, st.tiles, waited));
            }
            p => {
                g.sync_resets(r);
                g.hangs += u32::from(p == Poll::Recovered);
                log!("    {p:?}");
                return None;
            }
        }
    }
}

/// Two RGB565 screens for the 3D frames, flipped at the frame event.
struct Screens {
    at: [u32; 2],
}

impl Screens {
    fn target(&self, k: u32) -> Target {
        Target {
            picture: Picture::packed(self.at[(k % 2) as usize], 480, 640, PixelFormat::Rgb565),
            x: 0,
            y: 0,
        }
    }

    /// Shows screen `k % 2` at the next frame event (`all`: every
    /// graphics-plane register, the plane made RGB565, solo and opaque;
    /// else only its start); false if no frame event came.
    fn flip(&self, g: &mut Rig, k: u32, all: bool) -> bool {
        g.vevent_clear();
        let ok = g.vevent_wait();
        let plane = Plane {
            format: PlaneFormat::Rgb565,
            width: 480,
            height: 640,
            offset: self.at[(k % 2) as usize] as usize,
            stride: 960,
        };
        let gamma = g.e.regs.read32(dreg::GSCTRL) & (1 << 26);
        for (off, v) in plane.registers() {
            let v = match off {
                dreg::GSCTRL => v | gamma,
                dreg::GSCADR => Layer::SOLO.scadr(v),
                dreg::GBBASE => Layer::SOLO.bbase(v),
                _ => v,
            };
            if all || off == dreg::GSCADR {
                g.e.regs.write32(off, v);
            }
        }
        ok
    }
}

/// What a 3D sequence did: frame work times, the render times, the
/// predicted ones, the hash of the last picture shown.
struct Seq3 {
    s: Seq,
    render: Times,
    predicted: Times,
    hash: u32,
    elapsed_us: u32,
}

/// A 3D sequence of `n` frames: `scene(g, r, k)` sets frame `k`'s slots;
/// the caption (the CPU's work) prepared before the render (`pipeline`
/// false: turns) or during it (true), uploaded after it, then the flip.
#[allow(clippy::too_many_arguments)]
fn sequence(
    g: &mut Rig,
    r: &mut Renderer,
    sc: &Screens,
    cap: Texture,
    buf: &mut [u8],
    label: (u32, &str),
    n: u32,
    pipeline: bool,
    scene: &mut dyn FnMut(&mut Rig, &mut Renderer, u32),
) -> Seq3 {
    let mut out = Seq3 {
        s: Seq::new(),
        render: Times::default(),
        predicted: Times::default(),
        hash: 0,
        elapsed_us: 0,
    };
    let start = rt::ticks();
    r.invalidate_all();
    for k in 0..n {
        let t0 = rt::ticks();
        let mut l1 = heapless_name();
        let _ = write!(l1, "CH STEP {} OF {STEPS}", label.0);
        let mut l2 = heapless_name();
        let _ = write!(l2, "{} {:02}", label.1, k % 100);
        if !pipeline {
            prepare_caption(buf, l1.as_str(), l2.as_str());
        }
        scene(g, r, k);
        let mut prep = |_: &mut Rig| {
            if pipeline {
                prepare_caption(buf, l1.as_str(), l2.as_str());
            }
        };
        let Some((us, predicted, _, waited)) = render_split(g, r, sc.target(k), &mut prep) else {
            break;
        };
        upload_caption(g, cap, buf);
        // The caption changed: its tiles redraw next frame.
        r.invalidate(140, 560, 200, 50);
        let work = us_since(t0);
        if us > 0 {
            out.render.add(us);
            out.predicted.add(predicted);
        }
        let late = work > FRAME_US;
        out.s.add(work, work.saturating_sub(waited), late);
        if !sc.flip(g, k, k == 0) {
            out.s.lost = true;
            break;
        }
    }
    out.elapsed_us = us_since(start);
    // The last picture shown, hashed above the caption (whose words name
    // the way, so differ): the CPU reads 2700G memory.
    out.hash = ft_hash(g, sc.at[((n - 1) % 2) as usize], 480 * 560 / 2);
    out
}

fn report3(name: &str, q: &Seq3, budget: u32, n: u32) {
    q.s.report(name, budget);
    log!(
        "  CH {name}: render median {} us ({}-{}), the renderer's price median {} us; {n} frames shown in {} ms ({} fps x10); picture {:08X}",
        q.render.median(),
        q.render.min(),
        q.render.max(),
        q.predicted.median(),
        q.elapsed_us / 1000,
        u64::from(n) * 10_000_000 / u64::from(q.elapsed_us.max(1)),
        q.hash
    );
}

#[inline(never)]
fn three_d_frames(g: &mut Rig, scratch: &mut [u8]) {
    const PIC: u32 = 480 * 640 * 2;
    let (Some(a), Some(b)) = (g.pool.alloc(PIC, 0x1000), g.pool.alloc(PIC, 0x1000)) else {
        log!("  H: no 2700G memory for the 3D screens");
        return;
    };
    let sc = Screens { at: [a, b] };
    let Some(mut r) = Renderer::new(&mut g.pool, 480, 640) else {
        log!("  H: no renderer");
        let _ = g.pool.free(a, PIC);
        let _ = g.pool.free(b, PIC);
        return;
    };
    let cards: [Option<Texture>; 4] = core::array::from_fn(|k| {
        tex16(g, 4, format::RGB565, 1, &|x, y| {
            pattern565(x + 37 * k as u32, y + 53 * k as u32)
        })
    });
    let sky = tex16(g, 5, format::RGB565, 1, &|_, y| {
        let t = y * 31 / 255;
        (((4 + t / 2) << 11) | ((10 + t) << 5) | (20 - t / 3)) as u16
    });
    let cloud = |r: u32| {
        move |x: u32, y: u32| {
            let (dx, dy) = (x.abs_diff(128), y.abs_diff(128));
            let d = dx * dx + dy * dy;
            let a = if d >= r * r { 0 } else { 15 - d * 15 / (r * r) };
            (a << 12 | 0x0FFF) as u16
        }
    };
    let cloud1 = tex16(g, 5, format::ARGB4444, 1, &cloud(120));
    let cloud2 = tex16(g, 5, format::ARGB4444, 1, &cloud(90));
    let app = tex16(g, 6, format::RGB565, 1, &pattern565);
    let cap = tex16(g, CAP_CODE, format::ARGB4444, 1, &|_, _| 0);
    let buf = &mut scratch[..2 * (8 << CAP_CODE) * (8 << CAP_CODE)];
    let (
        Some(c0),
        Some(c1),
        Some(c2),
        Some(c3),
        Some(sky),
        Some(cl1),
        Some(cl2),
        Some(app),
        Some(cap),
    ) = (
        cards[0], cards[1], cards[2], cards[3], sky, cloud1, cloud2, app, cap,
    )
    else {
        log!("  H: no 2700G memory for the textures");
        for t in cards
            .into_iter()
            .chain([sky, cloud1, cloud2, app, cap])
            .flatten()
        {
            let _ = rp_mbx3d_render::release(&mut g.pool, t);
        }
        let _ = r.release(&mut g.pool);
        let _ = g.pool.free(a, PIC);
        let _ = g.pool.free(b, PIC);
        return;
    };
    g.e.regs.write32(lreg::VSCADR, 0);
    let caption = Draw::rect(
        140,
        560,
        200,
        50,
        Paint::Texture {
            tex: cap,
            uv: [(0, 0), (UV_ONE, 0), (UV_ONE, UV_ONE / 4), (0, UV_ONE / 4)],
            tint: WHITE,
        },
    )
    .over();
    let mut step = 11;

    // W5: four cards, one dragged 8 pixels a frame.
    let cards = [c0, c1, c2, c3];
    let mut w5 = |g: &mut Rig, r: &mut Renderer, k: u32| {
        if k == 0 {
            let _ = r.set_background(Some(0xFF20_2830));
            for (j, &t) in cards.iter().enumerate().take(3) {
                let (x, y) = (20 + (j as i32 % 2) * 240, 40 + (j as i32 / 2) * 300);
                g.set(r, 1 + j, Some(textured(x, y, 200, 260, t, full_uv())));
            }
            g.set(r, 6, Some(caption));
        }
        let x = 40 + ((k * 8) % 240) as i32;
        g.set(r, 4, Some(textured(x, 320, 200, 260, cards[3], full_uv())));
    };
    let mut hashes = [0u32; 2];
    for (i, pipeline) in [false, true].into_iter().enumerate() {
        let name = (step, if pipeline { "W5 PIPELINE" } else { "W5 TURNS" });
        let q = sequence(g, &mut r, &sc, cap, buf, name, FRAMES_3D, pipeline, &mut w5);
        report3(
            if pipeline {
                "W5 task switcher, pipeline"
            } else {
                "W5 task switcher, turns"
            },
            &q,
            2 * FRAME_US,
            FRAMES_3D,
        );
        hashes[i] = q.hash;
        step += 1;
    }
    log!(
        "  CH W5 turns and pipeline end on the same picture: {}",
        hashes[0] == hashes[1]
    );
    for slot in 1..=6 {
        g.set(&mut r, slot, None);
    }

    // W6: the living sky: two clouds drifting over the sky texture, the
    // caption over them.
    let mut w6 = |g: &mut Rig, r: &mut Renderer, k: u32| {
        if k == 0 {
            let _ = r.set_background(Some(0xFF00_0000));
            g.set(r, 1, Some(textured(0, 0, 480, 640, sky, full_uv())));
            g.set(r, 6, Some(caption));
        }
        let x1 = -100 + ((k * 2) % 400) as i32;
        let x2 = 300 - ((k * 3) % 400) as i32;
        g.set(
            r,
            2,
            Some(textured(x1, 80, 300, 200, cl1, full_uv()).over()),
        );
        g.set(
            r,
            3,
            Some(textured(x2, 260, 260, 160, cl2, full_uv()).over()),
        );
    };
    for (i, pipeline) in [false, true].into_iter().enumerate() {
        let name = (step, if pipeline { "W6 PIPELINE" } else { "W6 TURNS" });
        let q = sequence(g, &mut r, &sc, cap, buf, name, FRAMES_3D, pipeline, &mut w6);
        report3(
            if pipeline {
                "W6 living sky, pipeline"
            } else {
                "W6 living sky, turns"
            },
            &q,
            2 * FRAME_US,
            FRAMES_3D,
        );
        hashes[i] = q.hash;
        step += 1;
    }
    log!(
        "  CH W6 turns and pipeline end on the same picture: {}",
        hashes[0] == hashes[1]
    );
    for slot in 1..=6 {
        g.set(&mut r, slot, None);
    }

    // W3: the app's picture grows from its icon to the screen in 15 frames.
    let mut w3 = |g: &mut Rig, r: &mut Renderer, k: u32| {
        if k == 0 {
            let _ = r.set_background(Some(0xFF20_2830));
            g.set(r, 6, Some(caption));
        }
        let f = (k + 1).min(15) as i32;
        let lerp = |a: i32, b: i32| (a + (b - a) * f / 15) as i32;
        let (x, y) = (lerp(208, 0), lerp(288, 0));
        let (w, h) = (lerp(64, 480), lerp(64, 640));
        g.set(r, 1, Some(textured(x, y, w, h, app, full_uv())));
    };
    let name = (step, "W3 APP OPENS");
    let q = sequence(g, &mut r, &sc, cap, buf, name, 15, true, &mut w3);
    report3(
        "W3 app opening (completion: 250 ms target)",
        &q,
        FRAME_US,
        15,
    );
    route(
        "an app picture scaled (a frame)",
        "3D",
        q.render.median(),
        0,
        true,
    );
    step += 1;
    for slot in 1..=6 {
        g.set(&mut r, slot, None);
    }

    // W7: the rotation, by the 3D core: the screen texture with its corners
    // turned a quarter, one frame.
    let mut w7 = |g: &mut Rig, r: &mut Renderer, k: u32| {
        if k == 0 {
            g.set(r, 6, Some(caption));
        }
        let turned = [(0, UV_ONE), (0, 0), (UV_ONE, 0), (UV_ONE, UV_ONE)];
        g.set(r, 1, Some(textured(0, 0, 480, 640, app, turned)));
    };
    let name = (step, "W7 ROTATION");
    let q = sequence(g, &mut r, &sc, cap, buf, name, 2, true, &mut w7);
    report3(
        "W7 rotation by the 3D core (completion: 300 ms target)",
        &q,
        FRAME_US,
        2,
    );
    route("a screen turned", "3D", q.render.median(), 0, true);
    for slot in 1..=6 {
        g.set(&mut r, slot, None);
    }
    // ... and by the 2D engine's rotated copy (B11), a landscape picture
    // (640 x 480, the other screen's memory) into screen A.
    write_landscape(g, sc.at[1]);
    let mut base = [0u32; 6];
    base[0] = cmd::SCALE_1_TO_1;
    base[1..4].copy_from_slice(&cmd::key(0, 0));
    base[4..].copy_from_slice(&cmd::blend(0));
    let _ = submit_wait(g, &base);
    let (c, n) = rp_image::need(
        cmd::blit(
            (Format::RGB565, sc.at[0] as usize, 960),
            (Format::RGB565, sc.at[1] as usize, 1280),
            (0, 0),
            (0, 0, 640, 480),
            cmd::ROP_SRCCOPY,
            1 << 25,
            0,
        ),
        "a 2D rectangle",
    );
    let t0 = rt::ticks();
    let (ok, cpu, _) = submit_wait(g, &c[..n]);
    let us = us_since(t0);
    let _ = sc.flip(g, 0, true);
    log!("  CH W7 rotation by the 2D engine (640 x 480 turned into 480 x 640): {us} us, done {ok}");
    route("a screen turned", "2D BY CPU", us, cpu, ok);

    for t in [c0, c1, c2, c3, sky, cl1, cl2, app, cap] {
        let _ = rp_mbx3d_render::release(&mut g.pool, t);
    }
    let _ = r.release(&mut g.pool);
    let _ = g.pool.free(a, PIC);
    let _ = g.pool.free(b, PIC);
    log!("  CH 3D steps done: {step} of {STEPS}");
}

/// A 640 x 480 RGB565 landscape picture at `at` (stride 1280): the list's
/// colours turned on their side, by the CPU.
fn write_landscape(g: &mut Rig, at: u32) {
    let mut row = [0u32; 320];
    for y in 0..480u32 {
        for (k, w) in row.iter_mut().enumerate() {
            let x = 2 * k as u32;
            let c = |x: u32| {
                let v = list_rgb(y, x);
                (((v >> 19) & 31) << 11 | ((v >> 10) & 63) << 5 | (v >> 3) & 31) as u32
            };
            *w = c(x) | c(x + 1) << 16;
        }
        g.mem.store((at + y * 1280) as usize, &row);
        if y.is_multiple_of(64) {
            rt::check_in();
        }
    }
    g.mem.sync();
}
