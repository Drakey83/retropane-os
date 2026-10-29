//! Test image (R110): the device side of `rp-scene` (see the library's
//! documentation): the set-up, the phases, the watch, the log.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_draw::{Cmd, Frame, Rect};
    use rp_drv_blit2d::{MBX, reg as blit_reg};
    use rp_drv_mbx_display::clock::{self, Rung};
    use rp_drv_mbx_display::layers::Snapshot;
    use rp_drv_mbx_display::sig::{self, RefreshJudge, Watch};
    use rp_drv_mbx_display::{Display, font, reg as dreg};
    use rp_drv_mbx3d::RENDER_DONE;
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
        /// The run's diagnosis (R110 run 1: every refresh read unlike the
        /// references): the signatures read since the mode began, the
        /// most common first kept, with how many times; the rest counted.
        seen: [(u32, u32); SEEN],
        seen_other: u32,
        /// The last signature read.
        last_sig: u32,
        /// Each whole picture's 16-row band hashes as the reference saw it
        /// on the screen shown.
        ref_bands: [[u32; BANDS]; SCENES],
        /// R110 run 3: the 3D core's done bit watched by the image while
        /// it waits (reading `STATUS` doesn't clear it): when and at which
        /// scan line it was first seen set this frame.
        done_seen: Option<(u32, u32)>,
        /// The scene and bar last sent.
        last_scene: Option<usize>,
        last_bar: Option<usize>,
        /// The frames' records of the run of frames being logged.
        recs: [Rec; RECS],
        nrecs: usize,
    }

    /// One frame's record (R110 run 3, the tear's cause).
    #[derive(Clone, Copy, Default)]
    struct Rec {
        /// The picture (scene, bar).
        pic: (u8, u8),
        /// How it started: b behind the scan, c by the copy, h held, - none.
        how: u8,
        /// The start line and window (behind), or the copy's line.
        line: Option<u32>,
        window: Option<(u32, u32)>,
        /// The render's time seen by the image (start to the done bit, us)
        /// and the scan line then; the service's (its poll's upper bound).
        exact: Option<u32>,
        done_line: Option<u32>,
        service: Option<u32>,
        /// The scan line when 2D's changes went into screen 0.
        two_d: Option<u32>,
        /// Refreshes judged torn from this frame's start to the next's.
        torn: u32,
    }

    /// Frames recorded a run of frames (logged after it, no card write
    /// meanwhile).
    const RECS: usize = 24;
    /// Frames in each isolation run (R110 run 3).
    const ISOLATE: u32 = 40;

    /// Signatures a mode keeps apart in its log.
    const SEEN: usize = 8;
    /// The screen's 16-row bands (640 rows).
    const BANDS: usize = 40;
    /// One-off frames per phase whose shown picture is compared with its
    /// reference band by band (each read takes ~75 ms).
    const COMPARED: u32 = 8;

    impl Run {
        /// The picture at `at` (480 x 640 RGB565) hashed in 16-row bands
        /// (FNV-1a over the words), read by the CPU.
        fn bands(&mut self, at: usize) -> [u32; BANDS] {
            let mem = self.gfx.two_d().parts().1;
            let mut h = [0u32; BANDS];
            for (b, hb) in h.iter_mut().enumerate() {
                let mut v = 0x811C_9DC5u32;
                for w in 0..16 * 240 {
                    let word = mem.load(at + b * 16 * 960 + 4 * w).unwrap_or(0);
                    v = (v ^ word).wrapping_mul(0x0100_0193);
                }
                *hb = v;
            }
            h
        }

        /// The bands of the shown screen's picture.
        fn shown_bands(&mut self) -> [u32; BANDS] {
            let at = self.gfx.two_d().front();
            self.bands(at)
        }

        /// The bands where the two screens' pictures differ (bit b: band b,
        /// rows 16 b to 16 b + 15).
        fn screens_differ(&mut self) -> u64 {
            let [s0, s1] = self.gfx.two_d().screens();
            differ(&self.bands(s0), &self.bands(s1))
        }

        /// The word at the bar's first two pixels on the shown screen
        /// (two RGB565 pixels), and at row 552 (between the area and the
        /// bar).
        fn probe(&mut self) -> (u32, u32) {
            let at = self.gfx.two_d().front();
            let mem = self.gfx.two_d().parts().1;
            let y = usize::from(BAR.1);
            (
                mem.load(at + y * 960).unwrap_or(0),
                mem.load(at + 552 * 960).unwrap_or(0),
            )
        }

        /// Where the two screens differ in the 16 rows from `from`: the
        /// first word (x, y, screen 0's, screen 1's), the last word's
        /// (x, y), and how many words, `None` if none.
        fn first_diff(
            &mut self,
            from: usize,
        ) -> Option<((usize, usize, u32, u32), (usize, usize), u32)> {
            let [s0, s1] = self.gfx.two_d().screens();
            let mem = self.gfx.two_d().parts().1;
            let mut found = None;
            for y in from..(from + 16).min(640) {
                for w in 0..240 {
                    let (a, b) = (
                        mem.load(s0 + y * 960 + 4 * w).unwrap_or(0),
                        mem.load(s1 + y * 960 + 4 * w).unwrap_or(0),
                    );
                    if a != b {
                        let f = found.get_or_insert(((2 * w, y, a, b), (0, 0), 0));
                        f.1 = (2 * w, y);
                        f.2 += 1;
                    }
                }
            }
            found
        }

        /// Which screen is shown (0 or 1).
        fn shown(&mut self) -> usize {
            let two = self.gfx.two_d();
            usize::from(two.front() != two.screens()[0])
        }

        /// The signatures read since the last call, logged; the table
        /// emptied.
        fn log_seen(&mut self, p: usize, what: &str) {
            let mut seen = self.seen;
            seen.sort_unstable_by(|a, b| b.1.cmp(&a.1));
            let n = seen.iter().filter(|e| e.1 != 0).count();
            log!(
                "P{} {what}: signatures read (most first) {:08x?}, others {}",
                p + 1,
                &seen[..n],
                self.seen_other
            );
            self.seen = [(0, 0); SEEN];
            self.seen_other = 0;
        }

        fn regs(&mut self) -> &mut Mmio {
            &mut self.gfx.two_d().parts().0.regs
        }

        fn line(&mut self) -> u32 {
            self.regs().read32(dreg::DVLNUM) & 0xFFF
        }

        /// One look at the display: a refresh's signature judged, a torn
        /// one sorted (a hand-off mix, or other).
        fn tick(&mut self) {
            if self.done_seen.is_none()
                && self.regs().read32(MBX + blit_reg::STATUS) & RENDER_DONE != 0
            {
                let line = self.line();
                self.done_seen = Some((self.clock.now_us(), line));
            }
            let Some(mut w) = self.watch else { return };
            let torn = w.judge.torn + w.judge.back;
            let read = w.tick(self.regs(), rt::ticks());
            if let Some(s) = read {
                self.last_sig = s;
                match self.seen.iter_mut().find(|e| e.1 != 0 && e.0 == s) {
                    Some(e) => e.1 += 1,
                    None => match self.seen.iter_mut().find(|e| e.1 == 0) {
                        Some(e) => *e = (s, 1),
                        None => self.seen_other += 1,
                    },
                }
            }
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
        /// An unchanged scene or bar isn't sent again (a quad set again
        /// redraws its tiles: R110 run 3's 2D-alone frames must leave the
        /// scene untouched, its 3D-alone ones the bar).
        fn compose(&mut self, k: usize, b: usize) -> Step {
            if self.last_scene != Some(k) {
                self.last_scene = Some(k);
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
            }
            self.frame.clear();
            let r = Rect::new(BAR.0, BAR.1, BAR.2, BAR.3);
            if self.last_bar != Some(b) {
                self.last_bar = Some(b);
                self.frame.damage(r);
            }
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

        /// [`Self::compose`] recorded (the first [`RECS`] of a run of
        /// frames): how it started, its render's time as the image saw it
        /// and as the service learned it; the last record's torn count
        /// closed here (tears land in the refreshes after a frame's start).
        fn compose_rec(&mut self, k: usize, b: usize) {
            let torn = self.watch.map_or(0, |w| w.judge.torn);
            let before = self.gfx.starts();
            // Not set by a render still owed from before.
            self.done_seen = if self.regs().read32(MBX + blit_reg::STATUS) & RENDER_DONE != 0 {
                Some((0, 0))
            } else {
                None
            };
            let t_torn = torn;
            self.compose(k, b);
            let a = self.gfx.starts();
            let how = if a.behind > before.behind {
                b'b'
            } else if a.screen1 > before.screen1 {
                b'c'
            } else if a.held > before.held {
                b'h'
            } else {
                b'-'
            };
            // Seen before the service's poll saw it, else not this
            // render's (a later done bit: unknown).
            let exact = match (self.done_seen, a.last_start_us) {
                (Some((t, _)), Some(t0)) if t != 0 && how != b'-' && how != b'h' => {
                    Some(t.wrapping_sub(t0))
                }
                _ => None,
            }
            .filter(|&e| a.last_render_us.is_some_and(|s| e <= s));
            if self.nrecs < RECS {
                self.recs[self.nrecs] = Rec {
                    pic: (k as u8, b as u8),
                    how,
                    line: if how == b'c' {
                        a.last_copy_line
                    } else {
                        a.last_line
                    },
                    window: a.last_window,
                    exact,
                    done_line: exact.and(self.done_seen).map(|d| d.1),
                    service: a.last_render_us,
                    two_d: a.last_2d_line,
                    torn: t_torn,
                };
                self.nrecs += 1;
            }
        }

        /// The records of the run of frames, one line each, the torn
        /// counts made per frame (each frame's from its start to the next
        /// one's, the last's to now); emptied.
        fn log_recs(&mut self, p: usize, what: &str) {
            let now = self.watch.map_or(0, |w| w.judge.torn);
            for i in 0..self.nrecs {
                let next = if i + 1 < self.nrecs {
                    self.recs[i + 1].torn
                } else {
                    now
                };
                let r = self.recs[i];
                log!(
                    "P{} {what} frame {i}: picture {:?}, start {} at line {:?} (window {:?}); render seen {:?} us (done at line {:?}), the service's {:?} us; 2D into screen 0 at line {:?}; torn {}",
                    p + 1,
                    r.pic,
                    r.how as char,
                    r.line,
                    r.window,
                    r.exact,
                    r.done_line,
                    r.service,
                    r.two_d,
                    next.wrapping_sub(r.torn)
                );
            }
            self.nrecs = 0;
        }

        /// A frame with nothing new from 2D until a held scene is drawn
        /// (each try a refresh later): for the references only.
        /// `b` the bar shown: the 2D frame still carries it, undamaged
        /// (each 2D frame is the whole 2D scene: an empty one would paint
        /// the bar away in the renderer's catch-up, as R110's first run on
        /// the X51v did to every reference).
        fn draw_held(&mut self, b: usize) {
            for _ in 0..HELD_TRIES {
                let held = self.gfx.starts().held;
                self.frame.clear();
                self.frame.push(Cmd::Fill {
                    r: Rect::new(BAR.0, BAR.1, BAR.2, BAR.3),
                    color: BAR_COLOURS[b],
                });
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
            self.draw_held(b);
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
            "P{} {} {what}: {frames} frames; starts behind {} screen1 {} held {}, 2D behind the scan {} at once {} (last line {:?}, last copy line {:?}); copy times {:?} us; copy stalls {}; refreshes judged {}: torn {} (of them 2D-first mixes {}, 3D-first mixes {}), back {}, unread {}; failed {}, recovered {}; 2D engine stalls {}, FIFO full {}; {extra}",
            p + 1,
            PHASES[p].name,
            s.behind - m.starts.behind,
            s.screen1 - m.starts.screen1,
            s.held - m.starts.held,
            s.two_d_behind - m.starts.two_d_behind,
            s.two_d_now - m.starts.two_d_now,
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
            r.ref_bands[k] = r.shown_bands();
            let (shown, differ, (bar, gap)) = (r.shown(), r.screens_differ(), r.probe());
            log!(
                "P{} reference {k}: DSIG {:08x} on screen {shown}; bands where the screens differ {differ:010x}; bar pixels {bar:08x}, row 552 {gap:08x}",
                p + 1,
                r.whole[k]
            );
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
        r.seen = [(0, 0); SEEN];
        r.seen_other = 0;

        // Back to back.
        let m = mark(r);
        let t0 = rt::ticks();
        for _ in 0..B2B {
            rt::check_in();
            let k = picture(seq);
            if let Some(w) = r.watch.as_mut() {
                w.send(seq, r.whole[k]);
            }
            r.compose_rec(k, k);
            seq += 1;
        }
        let span = us(rt::ticks().wrapping_sub(t0)).max(1);
        // The last frame's refreshes judged before the line.
        settle(r, 3);
        r.log_seen(p, "back to back");
        r.log_recs(p, "back to back");
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
            if i < COMPARED {
                let (shown, sig) = (r.shown(), r.last_sig);
                let against = differ(&r.shown_bands(), &r.ref_bands[k]);
                let screens = r.screens_differ();
                let first = if screens == 0 {
                    None
                } else {
                    r.first_diff(16 * screens.trailing_zeros() as usize)
                };
                let (bar, gap) = r.probe();
                log!(
                    "P{} one-off {i}: picture {k} on screen {shown}, DSIG last read {sig:08x} (reference {:08x}); bands unlike the reference {against:010x}; bands where the screens differ {screens:010x} (first band: first (x, y, screen 0, screen 1), last (x, y), words {first:?}); bar pixels {bar:08x}, row 552 {gap:08x}",
                    p + 1,
                    r.whole[k]
                );
            }
        }
        r.log_seen(p, "one-off");
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
        if !ph.checker {
            isolate(r, p);
        }
    }

    /// The bands where `a` and `b` differ, as bits.
    fn differ(a: &[u32; BANDS], b: &[u32; BANDS]) -> u64 {
        a.iter()
            .zip(b)
            .enumerate()
            .fold(0, |m, (i, (x, y))| m | u64::from(x != y) << i)
    }

    /// R110 run 3's isolation (a rows phase): [`ISOLATE`] frames back to
    /// back where only the bar changes (the scene's picture 0 kept: 2D
    /// alone), then where only the scene changes (the bar 0 kept: 3D
    /// alone), each with its own references, every refresh judged, the
    /// first [`RECS`] frames recorded.
    fn isolate(r: &mut Run, p: usize) {
        for (what, bar_only) in [
            ("2D alone (the bar changes)", true),
            ("3D alone (the scene changes)", false),
        ] {
            let pick = |j: usize| {
                if bar_only {
                    (0, j % SCENES)
                } else {
                    (j % SCENES, 0)
                }
            };
            r.watch = None;
            let mut refs = [0u32; SCENES];
            for (j, rf) in refs.iter_mut().enumerate() {
                let (k, b) = pick(j);
                *rf = r.reference(k, b).unwrap_or(0);
            }
            log!("P{} {what}: references {refs:08x?}", p + 1);
            let line = r.line();
            r.watch = Some(Watch::new(
                RefreshJudge::new(SCENES as u32 - 1, refs[SCENES - 1]),
                line,
                rt::ticks(),
                LATE_TICKS,
            ));
            r.seen = [(0, 0); SEEN];
            r.seen_other = 0;
            let m = mark(r);
            let mut seq = SCENES as u32;
            for _ in 0..ISOLATE {
                rt::check_in();
                let j = picture(seq);
                if let Some(w) = r.watch.as_mut() {
                    w.send(seq, refs[j]);
                }
                let (k, b) = pick(j);
                r.compose_rec(k, b);
                seq += 1;
            }
            settle(r, 3);
            r.log_seen(p, what);
            r.log_recs(p, what);
            report(r, p, what, ISOLATE, &m, format_args!("isolation"));
        }
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
            seen: [(0, 0); SEEN],
            seen_other: 0,
            last_sig: 0,
            ref_bands: [[0; BANDS]; SCENES],
            done_seen: None,
            last_scene: None,
            last_bar: None,
            recs: [Rec::default(); RECS],
            nrecs: 0,
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
