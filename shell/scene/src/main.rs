//! Test image (R110): the device side of `rp-scene` (see the library's
//! documentation): the set-up, the phases, the watch, the log.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_draw::{Cmd, Frame, Rect};
    use rp_drv_mbx_display::clock::{self, Rung};
    use rp_drv_mbx_display::layers::Snapshot;
    use rp_drv_mbx_display::sig::{self, RefreshJudge, Watch};
    use rp_drv_mbx_display::{Display, font, reg as dreg};
    use rp_drv_pxa_uart::PxaUart;
    use rp_graphics::{ClientId, Graphics, Handle, Starts, Step, ThreeD};
    use rp_hal_api::RegisterBus;
    use rp_hal_mem::{load32_volatile, store32_volatile};
    use rp_hal_pxa27x::{Mmio, cpu, diag, rt};
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices::RunLog;
    use rp_image::crash;
    use rp_mbx_render::{Layout, LocalMem, Mbx2dRenderer};
    use rp_mbx3d_render::{Clock, TileOrder, format};
    use rp_scene::{
        AREA, ASK_LINES, B2B, BACKGROUNDS, BAR, BAR_COLOURS, LABEL_ROWS, MARKER, ONE_OFF, PHASES,
        Phase, SCENES, SLOTS, Times, picture, scene, texels, textured,
    };
    use rp_vram_map::{POOL, Pool};

    rp_image::entry!(main);

    macro_rules! log {
        ($($a:tt)*) => {{
            let _ = writeln!(RunLog, $($a)*);
        }};
    }

    /// OS timer ticks per millisecond.
    const TICKS_MS: u32 = 3250;
    /// A refresh at 60 Hz (`DISPPLL` `0xFC5`, R111: 645 lines of 25,687
    /// ns), in OS timer ticks.
    const FRAME_TICKS: u32 = 16_568 * 13 / 4;
    /// A refresh and a half: a look later than this missed one.
    const LATE_TICKS: u32 = FRAME_TICKS * 3 / 2;
    /// The display's underrun flag (`DINTRS` bit 4).
    const GFX_UNDERRUN: u32 = 1 << 4;
    /// The client's 2700G memory (the 8 x 8 texture, with room).
    const QUOTA: u32 = 64 * 1024;
    /// Tries to get a held scene drawn (each waits for the next refresh).
    const HELD_TRIES: u32 = 10;
    /// Refreshes a one-off frame may take to show whole.
    const SHOW_REFRESHES: u32 = 6;

    fn us(ticks: u32) -> u32 {
        ticks / 13 * 4
    }

    /// Microseconds from OSCR0 (3.25 MHz), kept running across its wrap:
    /// the graphics service's clock (`rp-today`'s).
    struct TickClock {
        last: u32,
        us: u32,
        rest: u32,
    }

    impl Clock for TickClock {
        fn now_us(&mut self) -> u32 {
            let t = rt::ticks();
            let d = u64::from(t.wrapping_sub(self.last)) * 4 + u64::from(self.rest);
            self.last = t;
            self.us = self.us.wrapping_add((d / 13) as u32);
            self.rest = (d % 13) as u32;
            self.us
        }

        fn delay_us(&mut self, us: u32) {
            rt::delay_us(us);
        }
    }

    /// 2700G local memory through the CPU's window onto it.
    struct Vram(&'static mut [u8]);

    impl LocalMem for Vram {
        fn store(&mut self, at: usize, words: &[u32]) {
            for (k, &w) in words.iter().enumerate() {
                let _ = store32_volatile(self.0, at + 4 * k, w);
            }
        }

        fn load(&self, at: usize) -> Option<u32> {
            load32_volatile(self.0, at)
        }

        fn sync(&mut self) {
            cpu::drain_write_buffer();
        }
    }

    // The long-lived objects in `.bss`, off `main`'s stack (rp-today's
    // stack audit, 2026-10-17).
    static TWO_D: rt::Slot<Mbx2dRenderer<Mmio, Vram>> = rt::Slot::new();
    static THREE_D: rt::Slot<ThreeD> = rt::Slot::new();
    static GFX: rt::Slot<Graphics<'static, Mmio, Vram>> = rt::Slot::new();
    static FRAME: rt::Slot<Frame> = rt::Slot::new();

    /// CS3's VLIO timing for the shared clock steps.
    fn set_cs3(c: clock::Cs3) {
        let _ = diag::set_cs3_vlio(c.0, c.1, c.2);
    }

    /// The crash path's put-back: Windows Mobile's clock and bus timing.
    fn put_back() {
        if let Some((mut regs, _)) = rp_hal_x51v::devices::emergency_mbx() {
            let _ = clock::put_back(&mut regs, rt::delay_us, set_cs3);
        }
    }

    /// The run's state: the service, its clock, the client, the watch.
    struct Run {
        gfx: &'static mut Graphics<'static, Mmio, Vram>,
        clock: TickClock,
        frame: &'static mut Frame,
        c: ClientId,
        tex: Handle,
        watch: Option<Watch>,
        /// The whole pictures' signatures (scene k, bar k).
        whole: [u32; SCENES],
        /// 2D first: bar k with scene k - 1 (2D's change shown before the
        /// scene renders).
        two_d_first: [u32; SCENES],
        /// 3D first: scene k with bar k - 1.
        three_d_first: [u32; SCENES],
        /// Torn refreshes that were a 2D-first or 3D-first mix.
        mixed_2d: u32,
        mixed_3d: u32,
        /// `frame` refused; frames that ended `Recovered`.
        failed: u32,
        recovered: u32,
    }

    impl Run {
        fn regs(&mut self) -> &mut Mmio {
            &mut self.gfx.two_d().parts().0.regs
        }

        fn line(&mut self) -> u32 {
            self.regs().read32(dreg::DVLNUM) & 0xFFF
        }

        /// One look at the display: a refresh's signature judged, a torn
        /// one sorted (a hand-off mix, or other).
        fn tick(&mut self) {
            let Some(mut w) = self.watch else { return };
            let torn = w.judge.torn + w.judge.back;
            let read = w.tick(self.regs(), rt::ticks());
            if let Some(s) = read
                && w.judge.torn + w.judge.back > torn
            {
                if self.two_d_first.contains(&s) {
                    self.mixed_2d += 1;
                } else if self.three_d_first.contains(&s) {
                    self.mixed_3d += 1;
                }
            }
            self.watch = Some(w);
        }

        /// Drives the frame in flight until it is shown (or ends otherwise),
        /// watching the display while it waits.
        fn drive(&mut self, mut s: Step) -> Step {
            loop {
                let Step::Wait(t) = s else { return s };
                while (t.wrapping_sub(self.clock.now_us()) as i32) > 0 {
                    self.tick();
                }
                s = self.gfx.poll(&mut self.clock);
            }
        }

        /// The scene `k` (with its background) and the bar `b` asked for:
        /// one frame through the service, driven to its end.
        fn compose(&mut self, k: usize, b: usize) -> Step {
            let qs = scene(k, textured(self.tex));
            for (slot, q) in qs.iter().enumerate().take(SLOTS) {
                if self.gfx.set(self.c, slot, q.as_ref()).is_err() {
                    self.failed += 1;
                }
            }
            if self
                .gfx
                .set_background(self.c, Some(BACKGROUNDS[k]))
                .is_err()
            {
                self.failed += 1;
            }
            self.frame.clear();
            let r = Rect::new(BAR.0, BAR.1, BAR.2, BAR.3);
            self.frame.damage(r);
            self.frame.push(Cmd::Fill {
                r,
                color: BAR_COLOURS[b],
            });
            let s = match self.gfx.frame(self.frame, &mut self.clock) {
                Ok(s) => self.drive(s),
                Err(_) => {
                    self.failed += 1;
                    Step::Idle
                }
            };
            if s == Step::Recovered {
                self.recovered += 1;
            }
            s
        }

        /// A frame with nothing new from 2D until a held scene is drawn
        /// (each try a refresh later): for the references only.
        fn draw_held(&mut self) {
            for _ in 0..HELD_TRIES {
                let held = self.gfx.starts().held;
                self.frame.clear();
                if let Ok(s) = self.gfx.frame(self.frame, &mut self.clock) {
                    self.drive(s);
                }
                if self.gfx.starts().held == held {
                    return;
                }
                rt::delay_us(us(FRAME_TICKS));
            }
        }

        /// The picture of scene `k` and bar `b`, settled: its signature
        /// (the third refresh's, the fourth's the same), `None` if unsteady.
        fn reference(&mut self, k: usize, b: usize) -> Option<u32> {
            self.compose(k, b);
            self.draw_held();
            rt::check_in();
            let a = sig::settled(self.regs(), rt::ticks, FRAME_TICKS);
            let n = sig::next(self.regs(), rt::ticks, FRAME_TICKS);
            if a == n { a } else { None }
        }
    }

    /// The label rows: two lines of capitals, three pixels a font pixel.
    fn label(two_d: &mut Mbx2dRenderer<Mmio, Vram>, lines: [&str; 2]) {
        const SCALE: usize = 3;
        let bg = 0xFF10_1010u32;
        for y in 0..640u16 {
            let mut row = [if y < LABEL_ROWS { 0xFF00_2060 } else { bg }; 480];
            let y = usize::from(y);
            for (k, l) in lines.iter().enumerate() {
                let top = 10 + 36 * k;
                if !(top..top + font::HEIGHT * SCALE).contains(&y) {
                    continue;
                }
                let gy = (y - top) / SCALE;
                for (i, ch) in l.chars().enumerate() {
                    let x0 = 12 + i * (font::WIDTH + 1) * SCALE;
                    let bits = font::glyph(ch)[gy];
                    for gx in 0..font::WIDTH {
                        if bits & (1 << (font::WIDTH - 1 - gx)) == 0 {
                            continue;
                        }
                        for dx in 0..SCALE {
                            if let Some(p) = row.get_mut(x0 + gx * SCALE + dx) {
                                *p = 0xFFFF_FFFF;
                            }
                        }
                    }
                }
            }
            two_d.wallpaper_row(y as u16, &row);
        }
    }

    /// The display's underrun flag, cleared Dell's way; whether it was set.
    fn underrun_take(regs: &mut Mmio) -> bool {
        rp_drv_mbx_display::ack_events(regs, GFX_UNDERRUN) & GFX_UNDERRUN != 0
    }

    /// What the counters moved by over one run of frames.
    struct Mark {
        starts: Starts,
        stalls: u32,
        judged: u32,
        torn: u32,
        back: u32,
        missed: u32,
        mixed: (u32, u32),
        failed: u32,
        recovered: u32,
        engine: (u32, u32),
    }

    fn mark(r: &mut Run) -> Mark {
        let w = r.watch;
        let engine = {
            let two = r.gfx.two_d();
            (two.stalls, two.fifo_full)
        };
        Mark {
            starts: r.gfx.starts(),
            stalls: r.gfx.copy_stalls(),
            judged: w.map_or(0, |w| w.judge.judged),
            torn: w.map_or(0, |w| w.judge.torn),
            back: w.map_or(0, |w| w.judge.back),
            missed: w.map_or(0, |w| w.missed),
            mixed: (r.mixed_2d, r.mixed_3d),
            failed: r.failed,
            recovered: r.recovered,
            engine,
        }
    }

    /// One line for the run of frames since `m`.
    fn report(
        r: &mut Run,
        p: usize,
        what: &str,
        frames: u32,
        m: &Mark,
        extra: core::fmt::Arguments,
    ) {
        let n = mark(r);
        let s = n.starts;
        log!(
            "P{} {} {what}: {frames} frames; starts behind {} screen1 {} held {} (last line {:?}, last copy line {:?}); copy times {:?} us; copy stalls {}; refreshes judged {}: torn {} (of them 2D-first mixes {}, 3D-first mixes {}), back {}, unread {}; failed {}, recovered {}; 2D engine stalls {}, FIFO full {}; {extra}",
            p + 1,
            PHASES[p].name,
            s.behind - m.starts.behind,
            s.screen1 - m.starts.screen1,
            s.held - m.starts.held,
            s.last_line,
            s.last_copy_line,
            r.gfx.copy_times(),
            n.stalls - m.stalls,
            n.judged - m.judged,
            n.torn - m.torn,
            n.mixed.0 - m.mixed.0,
            n.mixed.1 - m.mixed.1,
            n.back - m.back,
            n.missed - m.missed,
            n.failed - m.failed,
            n.recovered - m.recovered,
            n.engine.0 - m.engine.0,
            n.engine.1 - m.engine.1,
        );
    }

    /// Phase `p`: the label, the references, frames back to back, then
    /// one-off frames asked for at [`ASK_LINES`].
    fn phase(r: &mut Run, p: usize) {
        let ph: Phase = PHASES[p];
        let mut step = heapless_label();
        let _ = write!(step, "RP-SCENE STEP {} OF {}", p + 1, PHASES.len());
        r.watch = None;
        label(r.gfx.two_d(), [step.as_str(), ph.label]);
        r.gfx.two_d().wallpaper_changed();
        let order = if ph.checker {
            TileOrder::Checker
        } else {
            TileOrder::Rows
        };
        if r.gfx.set_tile_order(r.c, order).is_err() {
            r.failed += 1;
        }
        // The references: whole, then the two hand-off mixes.
        for k in 0..SCENES {
            let prev = (k + SCENES - 1) % SCENES;
            r.whole[k] = r.reference(k, k).unwrap_or(0);
            r.two_d_first[k] = r.reference(prev, k).unwrap_or(0);
            r.three_d_first[k] = r.reference(k, prev).unwrap_or(0);
        }
        // The last shown is the whole picture 3 (shown again).
        r.reference(SCENES - 1, SCENES - 1);
        let mut distinct = true;
        for (i, a) in r.whole.iter().enumerate() {
            for b in r.whole[i + 1..]
                .iter()
                .chain(&r.two_d_first)
                .chain(&r.three_d_first)
            {
                distinct &= a != b;
            }
        }
        log!(
            "P{} {}: references (DSIG) whole {:08x?}, 2D first {:08x?}, 3D first {:08x?}: {}",
            p + 1,
            ph.name,
            r.whole,
            r.two_d_first,
            r.three_d_first,
            if distinct {
                "the whole pictures distinct"
            } else {
                "NOT distinct (a zero: unsteady or not read; the emulator models no DSIG): refreshes judged only as far as they tell apart"
            }
        );
        let line = r.line();
        r.watch = Some(Watch::new(
            RefreshJudge::new(SCENES as u32 - 1, r.whole[SCENES - 1]),
            line,
            rt::ticks(),
            LATE_TICKS,
        ));
        let mut seq = SCENES as u32;

        // Back to back.
        let m = mark(r);
        let t0 = rt::ticks();
        for _ in 0..B2B {
            rt::check_in();
            let k = picture(seq);
            if let Some(w) = r.watch.as_mut() {
                w.send(seq, r.whole[k]);
            }
            r.compose(k, k);
            seq += 1;
        }
        let span = us(rt::ticks().wrapping_sub(t0)).max(1);
        // The last frame's refreshes judged before the line.
        settle(r, 3);
        report(
            r,
            p,
            "back to back",
            B2B,
            &m,
            format_args!(
                "{} frames a second ({} us for {B2B})",
                u64::from(B2B) * 1_000_000 / u64::from(span),
                span
            ),
        );

        // One-off.
        let m = mark(r);
        let (mut to_shown, mut never) = (Times::new(), 0u32);
        for i in 0..ONE_OFF {
            rt::check_in();
            let ask_line = ASK_LINES[i as usize % ASK_LINES.len()];
            let t0 = rt::ticks();
            while !(ask_line..ask_line + 8).contains(&r.line())
                && rt::ticks().wrapping_sub(t0) < 2 * FRAME_TICKS
            {
                r.tick();
            }
            let k = picture(seq);
            let ask = rt::ticks();
            if let Some(w) = r.watch.as_mut() {
                w.send(seq, r.whole[k]);
            }
            r.compose(k, k);
            seq += 1;
            while r.watch.is_some_and(|w| w.shown.is_none())
                && rt::ticks().wrapping_sub(ask) < SHOW_REFRESHES * FRAME_TICKS
            {
                r.tick();
            }
            match r.watch.and_then(|w| w.shown) {
                Some(at) => to_shown.add(us(at.wrapping_sub(ask))),
                None => never += 1,
            }
            settle(r, 1);
        }
        report(
            r,
            p,
            "one-off",
            ONE_OFF,
            &m,
            format_args!(
                "asked at lines {ASK_LINES:?}: to the start of the first whole refresh median {:?} us, worst {} us; never shown whole {never}",
                to_shown.median(),
                to_shown.worst
            ),
        );
    }

    /// Watches `n` more refreshes.
    fn settle(r: &mut Run, n: u32) {
        let t0 = rt::ticks();
        while rt::ticks().wrapping_sub(t0) < n * FRAME_TICKS {
            r.tick();
        }
    }

    /// A small text buffer for the label.
    struct Text {
        b: [u8; 32],
        n: usize,
    }

    impl Text {
        fn as_str(&self) -> &str {
            core::str::from_utf8(&self.b[..self.n]).unwrap_or("")
        }
    }

    impl Write for Text {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            for &c in s.as_bytes() {
                if self.n == self.b.len() {
                    return Err(core::fmt::Error);
                }
                self.b[self.n] = c;
                self.n += 1;
            }
            Ok(())
        }
    }

    const fn heapless_label() -> Text {
        Text { b: [0; 32], n: 0 }
    }

    fn main() -> ! {
        rp_image::on_crash(put_back);
        rt::init_mmu();
        let Some((regs, vram)) = rp_image::take_mbx() else {
            crash(format_args!("no display"))
        };
        let mut d = Display::new(regs);
        let _ = d.adopt(MBX_VRAM_SIZE as usize);
        let before = Snapshot::take(d.regs());
        let found = (
            d.regs().read32(clock::COREPLL),
            d.regs().read32(clock::MBXCLK),
            d.regs().read32(dreg::DISPPLL),
            diag::msc1(),
        );
        log!(
            "R110 rp-scene: the graphics service drives a 3D scene beside a 2D bar; found COREPLL {:08x} MBXCLK {:08x} DISPPLL {:08x} MSC1 {:08x}",
            found.0,
            found.1,
            found.2,
            found.3
        );
        // Windows Mobile's clock first (ADR-0035 rule 2: a loader started
        // from a Windows Mobile on another clock inherits it).
        if clock::Speed::now(d.regs()) != clock::Speed::WM || (found.1 >> 2) & 3 != 1 {
            let a = clock::put_back(d.regs(), rt::delay_us, set_cs3);
            log!("  not Windows Mobile's clock: put back first ({a:?})");
        }
        let under = underrun_take(d.regs());
        log!(
            "  underrun flag at the start: {} (cleared)",
            if under { "set" } else { "clear" }
        );

        let Some(two_d) = TWO_D.put_with(|| {
            Mbx2dRenderer::new(
                d.into_inner(),
                Vram(vram),
                Layout::X51V,
                rt::ticks,
                TICKS_MS,
            )
        }) else {
            crash(format_args!("started twice"))
        };
        label(two_d, ["RP-SCENE SETTING UP", ""]);
        two_d.show(rt::delay_us);
        let (Some(three_d), Some(frame)) =
            (THREE_D.put_with(ThreeD::new), FRAME.put_with(Frame::new))
        else {
            crash(format_args!("started twice"))
        };
        let Some(gfx) = GFX.put_with(|| Graphics::new(two_d, Pool::new(POOL), Some(three_d)))
        else {
            crash(format_args!("started twice"))
        };
        let Some(c) = gfx.client(QUOTA) else {
            crash(format_args!("no client"))
        };
        let tex = match gfx.upload_texture(c, 0, format::RGB565, &texels()) {
            Ok(t) => t,
            Err(e) => crash(format_args!("texture: {e:?}")),
        };
        let (x, y, w, h) = AREA;
        if let Err(e) = gfx.open_scene(c, x, y, w, h) {
            crash(format_args!("scene: {e:?}"));
        }
        let mut r = Run {
            gfx,
            clock: TickClock {
                last: rt::ticks(),
                us: 0,
                rest: 0,
            },
            frame,
            c,
            tex,
            watch: None,
            whole: [0; SCENES],
            two_d_first: [0; SCENES],
            three_d_first: [0; SCENES],
            mixed_2d: 0,
            mixed_3d: 0,
            failed: 0,
            recovered: 0,
        };

        for p in 0..PHASES.len() {
            // The clock moves once, between the first two phases, and the
            // service forgets the old clock's times.
            if PHASES[p].top && (p == 0 || !PHASES[p - 1].top) {
                // On the card first (ADR-0035 rule 5): a step that froze
                // the device is on record.
                log!("  clock to 149.50 / 2: stepping");
                let a = clock::step(r.regs(), Rung::TOP, rt::delay_us, set_cs3);
                let under = underrun_take(r.regs());
                r.gfx.clock_changed();
                let regs = r.regs();
                log!(
                    "  clock to 149.50 / 2: {a:?}; read back COREPLL {:08x} MBXCLK {:08x} LMTYPE {:08x} LMTIM {:08x} LMREFRESH {:08x}; underrun flag {} (cleared)",
                    regs.read32(clock::COREPLL),
                    regs.read32(clock::MBXCLK),
                    regs.read32(clock::LMTYPE),
                    regs.read32(clock::LMTIM),
                    regs.read32(clock::LMREFRESH),
                    if under { "set" } else { "clear" }
                );
                if a.is_none_or(|a| a.unwritten > 0) {
                    let _ = clock::put_back(r.regs(), rt::delay_us, set_cs3);
                    log!(
                        "  the clock step failed: Windows Mobile's clock put back, the run stops here"
                    );
                    break;
                }
            }
            phase(&mut r, p);
            let under = underrun_take(r.regs());
            log!(
                "P{} done; underrun flag {}",
                p + 1,
                if under { "set" } else { "clear" }
            );
        }

        let a = clock::put_back(r.regs(), rt::delay_us, set_cs3);
        let under = underrun_take(r.regs());
        log!(
            "put back: {a:?}; COREPLL {:08x} MBXCLK {:08x} MSC1 {:08x}; underrun flag {}",
            r.regs().read32(clock::COREPLL),
            r.regs().read32(clock::MBXCLK),
            diag::msc1(),
            if under { "set" } else { "clear" }
        );
        before.restore(r.regs());
        log!("{MARKER} (failed {}, recovered {})", r.failed, r.recovered);
        let _ = writeln!(PxaUart::new(rt::emergency_uart()), "{MARKER}");
        rp_image::finish()
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
