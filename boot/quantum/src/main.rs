//! Quantum test image (ADR-0010 amendment): what the time slice costs and
//! buys, measured live on the CPU for each policy in [`POLICIES`].
//!
//! For each policy (a quantum, and a wake slice or none), with a busy
//! user thread (the "hog") at the UI thread's own priority all along:
//!
//! - **touch:** a timer interrupt at irregular gaps (20-80 ms) stands in
//!   for a touch; its service thread (priority 100, as a touch driver's)
//!   wakes the UI thread, which draws a frame (5 ms of work) and reports.
//!   Touch to screen: from the interrupt to the frame's end.
//! - **anim:** the UI thread draws a frame every 60 Hz period (sleeping to
//!   each period's start); a frame that ends after the next period started
//!   is late.
//! - **busy:** the UI thread draws frames back to back without ever
//!   blocking (an old game's loop): the gap between frames.
//! - **hogs:** two busy threads share the CPU; their combined work against
//!   one busy thread alone is what the switches cost (register switch,
//!   cache refills: each walks its own 16 KiB, so together they overflow
//!   the 32 KiB data cache).
//!
//! The work is the same number of passes over a buffer on any machine
//! (calibrated to 5 ms on this one at start), so the device's numbers are
//! real and the emulator's a smoke check. Prints `QT` lines (times in µs)
//! and [`MARKER`]; `cargo xtask emu-quantum` checks them, and `cargo xtask
//! load rp-quantum` brings the X51v's to the PC.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed when every policy ran.
pub const MARKER: &str = "RETROPANE-QUANTUM-OK";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_pxa_uart::PxaUart;
    use rp_hal_pxa27x::rt::{self, Global};
    use rp_hal_pxa27x::soc::irq;
    use rp_hal_pxa27x::thread;
    use rp_hal_pxa27x::user::{self, Frame};
    use rp_hal_x51v::devices::take_ram_pool;
    use rp_kernel_sched::{Eid, Error, LOWEST, Scheduler, Tid};

    rp_hal_pxa27x::entry!(main);

    /// One millisecond in OS-timer ticks (3.25 MHz).
    const MS: u32 = 3_250;
    /// "No wake slice": CE's plain round-robin.
    const OFF: u32 = u32::MAX;
    /// The policies measured: quantum (ms) and wake slice (ticks, or OFF).
    const POLICIES: [(u32, u32); 8] = [
        (100, OFF),
        (50, OFF),
        (20, OFF),
        (10, OFF),
        (100, 1),
        (100, MS),
        (20, MS),
        (10, MS),
    ];
    /// The UI thread's and the hogs' priority (CE's `THREAD_PRIORITY_NORMAL`).
    const APP: u8 = 251;
    /// The touch service thread's priority (above the apps, as a driver's).
    const DRIVER: u8 = 100;
    /// One frame of drawing.
    const FRAME_WORK: u32 = 5 * MS;
    /// A 60 Hz period (54,166.7 ticks).
    const VSYNC: u32 = 54_167;
    /// How long each test runs.
    const TOUCH_TIME: u32 = 1_200 * MS;
    const ANIM_TIME: u32 = 600 * MS;
    const BUSY_TIME: u32 = 400 * MS;
    const HOGS_TIME: u32 = 300 * MS;
    /// The work buffer each thread walks (16 KiB: two of them overflow the
    /// 32 KiB data cache).
    const BUF_WORDS: usize = 4096;
    /// Kernel stack, user stack, work buffer: one chunk per thread.
    const CHUNK_WORDS: usize = 1024 + 1024 + BUF_WORDS;

    /// Words of the control block (shared with user mode, in the pool).
    const STOP: u32 = 0;
    const PASSES: u32 = 1;
    /// Hog `i` counts its passes in word `COUNT + i`.
    const COUNT: u32 = 2;

    /// System calls.
    const SYS_UI_WAIT: u32 = 0;
    const SYS_DONE: u32 = 1;
    const SYS_EXIT: u32 = 2;

    static SCHED: Global<Scheduler> = Global::new(Scheduler::new());
    static CURRENT: Global<Tid> = Global::new(Tid(0));

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Test {
        Touch,
        Anim,
        Busy,
        Hogs,
    }

    struct Bench {
        test: Test,
        stop: bool,
        /// Address of the control block.
        ctrl: u32,
        /// Set by the touch service thread for the UI thread.
        input: Eid,
        /// Set each time a test thread ends.
        done: Eid,
        /// Starts the touch service thread's next round.
        touch_go: Eid,
        irq_event: Eid,
        /// The oldest touch no frame has answered yet (0: none).
        pending: u32,
        /// What the frame being drawn answers (a touch's or a period's
        /// time), and when the last frame ended (busy test).
        serving: u32,
        last_done: u32,
        start: u32,
        samples: [u32; 64],
        n: usize,
        late: u32,
        touches: u32,
        /// Per thread index: (work buffer address, hog counter index or
        /// `u32::MAX` for the UI thread), and its user stack.
        task: [(u32, u32); MAX_TASKS],
    }

    const MAX_TASKS: usize = 128;

    static BENCH: Global<Bench> = Global::new(Bench {
        test: Test::Hogs,
        stop: false,
        ctrl: 0,
        input: Eid(0),
        done: Eid(0),
        touch_go: Eid(0),
        irq_event: Eid(0),
        pending: 0,
        serving: 0,
        last_done: 0,
        start: 0,
        samples: [0; 64],
        n: 0,
        late: 0,
        touches: 0,
        task: [(0, 0); MAX_TASKS],
    });
    static USTACKS: Global<[Option<&'static mut [u32]>; MAX_TASKS]> =
        Global::new([const { None }; MAX_TASKS]);

    struct Tee(PxaUart<rp_hal_pxa27x::Mmio>);

    impl Write for Tee {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let _ = self.0.write_str(s);
            rp_hal_x51v::devices::log_write(s.as_bytes());
            Ok(())
        }
    }

    fn us(ticks: u32) -> u32 {
        (u64::from(ticks) * 4 / 13) as u32
    }

    fn main() -> ! {
        let mut port = PxaUart::new(rt::emergency_uart());
        port.init(115_200);
        let mut uart = Tee(port);
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        let _ = uart.write_str("\nRetroPane 26 quantum test\n");

        let Some(pool) = take_ram_pool() else { halt() };
        let (ctrl, pool) = pool.split_at_mut(1024);
        let ctrl = ctrl.as_mut_ptr() as usize as u32;
        let mut chunks = pool.chunks_exact_mut(CHUNK_WORDS);

        let started = SCHED.with(|s| -> Result<(), Error> {
            let main = s.create(0, 150)?;
            let _ = s.schedule(rt::ticks());
            CURRENT.with(|c| *c = main);
            let done = s.create_event(false, false)?;
            let input = s.create_event(false, false)?;
            let touch_go = s.create_event(false, false)?;
            let irq_event = s.create_event(false, false)?;
            BENCH.with(|b| {
                b.ctrl = ctrl;
                b.done = done;
                b.input = input;
                b.touch_go = touch_go;
                b.irq_event = irq_event;
            });
            let kernel: [(u8, extern "C" fn(usize) -> !); 2] =
                [(LOWEST, idle_thread), (DRIVER, touch_thread)];
            for (prio, entry) in kernel {
                let t = s.create(0, prio)?;
                let (kstack, _) = chunks.next().ok_or(Error::Full)?.split_at_mut(1024);
                thread::prepare(t.index(), kstack, entry, 0);
            }
            Ok(())
        });
        if started.is_err() {
            let _ = uart.write_str("QUANTUM setup FAIL\n");
            halt();
        }
        rt::set_signal_hook(on_signal);
        user::set_syscall_handler(on_syscall);
        rt::set_preempt_hook(reschedule);

        // Calibrate: passes per frame of work, on a warm buffer.
        let Some(cal) = chunks.next() else { halt() };
        let buf = cal.as_mut_ptr() as usize as u32;
        work_pass(buf);
        let t0 = rt::ticks();
        for _ in 0..20 {
            work_pass(buf);
        }
        let twenty = rt::ticks().wrapping_sub(t0).max(1);
        let passes = (FRAME_WORK * 20 / twenty).max(1);
        user::user_write(ctrl + 4 * PASSES, passes);
        let _ = writeln!(
            uart,
            "QT calibrate 20 passes {} us, {passes} passes/frame",
            us(twenty)
        );

        // One busy thread alone: the work rate the others are measured by.
        SCHED.with(|s| s.set_quantum(100 * MS));
        let (solo, solo_ticks) = run(&mut chunks, Test::Hogs, 1, HOGS_TIME);
        let _ = writeln!(uart, "QT solo {solo} passes in {} us", us(solo_ticks));

        let mut ran = 0;
        for (q, slice) in POLICIES {
            SCHED.with(|s| {
                s.set_quantum(q * MS);
                s.set_wake_slice((slice != OFF).then_some(slice));
            });
            let mut name = Name::default();
            let _ = write!(name, "q{q}");
            if slice == OFF {
                let _ = name.write_str("-ce");
            } else {
                let _ = write!(name, "-w{}", us(slice));
            }
            let name = name.as_str();

            run(&mut chunks, Test::Touch, 1, TOUCH_TIME);
            let (n, med, max, touches) = BENCH.with(|b| {
                let (med, max) = stats(&mut b.samples[..b.n]);
                (b.n, med, max, b.touches)
            });
            let _ = writeln!(
                uart,
                "QT {name} touch n {n}/{touches} med {} max {}",
                us(med),
                us(max)
            );

            run(&mut chunks, Test::Anim, 1, ANIM_TIME);
            let (n, med, max, late) = BENCH.with(|b| {
                let (med, max) = stats(&mut b.samples[..b.n]);
                (b.n, med, max, b.late)
            });
            let _ = writeln!(
                uart,
                "QT {name} anim n {n}/{} late {late} med {} max {}",
                ANIM_TIME / VSYNC,
                us(med),
                us(max)
            );

            run(&mut chunks, Test::Busy, 1, BUSY_TIME);
            let (n, med, max) = BENCH.with(|b| {
                let (med, max) = stats(&mut b.samples[..b.n]);
                (b.n, med, max)
            });
            let _ = writeln!(uart, "QT {name} busy n {n} med {} max {}", us(med), us(max));

            let (both, ticks) = run(&mut chunks, Test::Hogs, 2, HOGS_TIME);
            // Work rate of the two against one alone, in tenths of a percent.
            let rate = u64::from(both) * u64::from(solo_ticks) * 1000
                / (u64::from(solo.max(1)) * u64::from(ticks.max(1)));
            let _ = writeln!(uart, "QT {name} hogs {}.{}%", rate / 10, rate % 10);
            ran += 1;
        }
        if ran == POLICIES.len() {
            let _ = uart.write_str(super::MARKER);
            let _ = uart.write_str("\n");
        }
        rp_hal_x51v::devices::return_to_loader();
        halt()
    }

    /// A short label for a policy (`q100-w307`).
    #[derive(Default)]
    struct Name {
        buf: [u8; 16],
        len: usize,
    }

    impl Name {
        fn as_str(&self) -> &str {
            core::str::from_utf8(&self.buf[..self.len]).unwrap_or("?")
        }
    }

    impl Write for Name {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let end = (self.len + s.len()).min(self.buf.len());
            self.buf[self.len..end].copy_from_slice(&s.as_bytes()[..end - self.len]);
            self.len = end;
            Ok(())
        }
    }

    /// Median and maximum (sorts `v`: an insertion sort, as `sort_unstable`
    /// costs kilobytes of code for 64 numbers).
    #[inline(never)]
    fn stats(v: &mut [u32]) -> (u32, u32) {
        for i in 1..v.len() {
            let mut j = i;
            while j > 0 && v[j - 1] > v[j] {
                v.swap(j - 1, j);
                j -= 1;
            }
        }
        (
            v.get(v.len() / 2).copied().unwrap_or(0),
            v.last().copied().unwrap_or(0),
        )
    }

    /// Runs one test for `time`: the UI thread (unless `Hogs`) and `hogs`
    /// busy threads, all at [`APP`]. Returns the hogs' passes and the
    /// ticks it took.
    #[inline(never)]
    fn run(
        chunks: &mut impl Iterator<Item = &'static mut [u32]>,
        test: Test,
        hogs: u32,
        time: u32,
    ) -> (u32, u32) {
        let ctrl = BENCH.with(|b| {
            b.test = test;
            b.stop = false;
            b.pending = 0;
            b.n = 0;
            b.late = 0;
            b.touches = 0;
            b.last_done = 0;
            b.start = rt::ticks();
            b.ctrl
        });
        user::user_write(ctrl + 4 * STOP, 0);
        user::user_write(ctrl + 4 * COUNT, 0);
        user::user_write(ctrl + 4 * (COUNT + 1), 0);
        let ui = test != Test::Hogs;
        let threads = hogs + u32::from(ui);
        for k in 0..threads {
            let role = if ui && k == 0 {
                u32::MAX
            } else {
                k - u32::from(ui)
            };
            spawn(chunks, role);
        }
        if test == Test::Touch {
            set(BENCH.with(|b| b.touch_go));
        }
        let start = rt::ticks();
        let _ = SCHED.with(|s| s.sleep_until(start.wrapping_add(time)));
        reschedule();
        let input = BENCH.with(|b| {
            b.stop = true;
            b.input
        });
        user::user_write(ctrl + 4 * STOP, 1);
        let _ = SCHED.with(|s| s.set_event(input)); // a UI thread waiting for a touch
        let done = BENCH.with(|b| b.done);
        for _ in 0..threads {
            wait(done);
        }
        let ticks = rt::ticks().wrapping_sub(start);
        let passes = user::user_read(ctrl + 4 * COUNT) + user::user_read(ctrl + 4 * (COUNT + 1));
        (passes, ticks)
    }

    /// Starts a user thread at [`APP`]: the UI thread (`role` `u32::MAX`)
    /// or busy thread `role`.
    fn spawn(chunks: &mut impl Iterator<Item = &'static mut [u32]>, role: u32) {
        let Some(chunk) = chunks.next() else {
            panic!("out of memory for threads");
        };
        let (kstack, rest) = chunk.split_at_mut(1024);
        let (ustack, buf) = rest.split_at_mut(1024);
        let buf = buf.as_mut_ptr() as usize as u32;
        let Ok(t) = SCHED.with(|s| s.create(0, APP)) else {
            panic!("thread table full");
        };
        BENCH.with(|b| b.task[t.index()] = (buf, role));
        USTACKS.with(|u| u[t.index()] = Some(ustack));
        thread::prepare(t.index(), kstack, trampoline, t.index());
    }

    extern "C" fn trampoline(i: usize) -> ! {
        let (buf, role) = BENCH.with(|b| b.task[i]);
        let ctrl = BENCH.with(|b| b.ctrl);
        let Some(stack) = USTACKS.with(|u| u[i].take()) else {
            finish()
        };
        // The thread's first words: where the control block is, its role.
        user::user_write(buf, ctrl);
        user::user_write(buf + 4, role);
        let entry = if role == u32::MAX { ui_main } else { hog_main };
        user::enter_user(entry, stack, buf as usize)
    }

    /// One pass of work: a read and a write in each 32-byte line of the
    /// 16 KiB buffer (so every line is cached and dirty).
    #[inline(never)]
    fn work_pass(buf: u32) {
        let mut a = buf + 16;
        let end = buf + 4 * BUF_WORDS as u32;
        while a < end {
            user::user_write(a, user::user_read(a).wrapping_add(1));
            a += 32;
        }
    }

    /// A busy thread (user mode): passes until told to stop, counted.
    extern "C" fn hog_main(buf: usize) -> ! {
        let buf = buf as u32;
        let ctrl = user::user_read(buf);
        let counter = ctrl + 4 * (COUNT + user::user_read(buf + 4));
        while user::user_read(ctrl + 4 * STOP) == 0 {
            work_pass(buf);
            user::user_write(counter, user::user_read(counter) + 1);
        }
        user::syscall(SYS_EXIT, 0, 0, 0);
        loop {
            core::hint::spin_loop();
        }
    }

    /// The UI thread (user mode): waits for its reason to draw (the kernel
    /// decides by test), draws a frame, reports.
    extern "C" fn ui_main(buf: usize) -> ! {
        let buf = buf as u32;
        let ctrl = user::user_read(buf);
        let passes = user::user_read(ctrl + 4 * PASSES);
        while user::syscall(SYS_UI_WAIT, 0, 0, 0) != 0 {
            for _ in 0..passes {
                work_pass(buf);
            }
            user::syscall(SYS_DONE, 0, 0, 0);
        }
        user::syscall(SYS_EXIT, 0, 0, 0);
        loop {
            core::hint::spin_loop();
        }
    }

    fn on_syscall(f: &mut Frame) {
        match f.number() {
            SYS_UI_WAIT => {
                let go = match BENCH.with(|b| (b.test, b.stop)) {
                    (_, true) => false,
                    // (A wake with no touch pending is a stop left set by
                    // an earlier test: wait again.)
                    (Test::Touch, _) => loop {
                        wait(BENCH.with(|b| b.input));
                        let got = BENCH.with(|b| {
                            b.serving = core::mem::take(&mut b.pending);
                            (b.stop, b.serving != 0)
                        });
                        match got {
                            (true, _) => break false,
                            (false, true) => break true,
                            (false, false) => {}
                        }
                    },
                    (Test::Anim, _) => {
                        // Sleep to the next period's start.
                        let now = rt::ticks();
                        let at = BENCH.with(|b| {
                            let k = now.wrapping_sub(b.start) / VSYNC + 1;
                            b.serving = b.start.wrapping_add(k * VSYNC);
                            b.serving
                        });
                        let _ = SCHED.with(|s| s.sleep_until(at));
                        reschedule();
                        !BENCH.with(|b| b.stop)
                    }
                    _ => true,
                };
                f.set_result(u32::from(go));
            }
            SYS_DONE => {
                let now = rt::ticks();
                BENCH.with(|b| {
                    let sample = match b.test {
                        Test::Busy => {
                            let gap = (b.last_done != 0).then(|| now.wrapping_sub(b.last_done));
                            b.last_done = now;
                            gap
                        }
                        _ => Some(now.wrapping_sub(b.serving)),
                    };
                    if let Some(t) = sample
                        && b.n < b.samples.len()
                    {
                        if b.test == Test::Anim && t > VSYNC {
                            b.late += 1;
                        }
                        b.samples[b.n] = t;
                        b.n += 1;
                    }
                });
                f.set_result(0);
            }
            SYS_EXIT => {
                let done = BENCH.with(|b| b.done);
                let _ = SCHED.with(|s| s.set_event(done));
                finish()
            }
            _ => f.set_result(u32::MAX),
        }
    }

    /// The signal hook (IRQ context): the "touch" timer fired.
    fn on_signal(line: u32) {
        if line == irq::OST2 {
            let ev = BENCH.with(|b| b.irq_event);
            let _ = SCHED.with(|s| s.set_event(ev));
        }
    }

    /// The touch service thread: during the touch test, a "touch" at
    /// irregular gaps (20-80 ms), each recorded and passed to the UI thread
    /// as a driver would.
    extern "C" fn touch_thread(_: usize) -> ! {
        // The timer first: taking it clears what an earlier image left
        // pending, before the line is claimed.
        let (Some(mut timer), Some(mut line)) = (rt::take_spare_timer(), rt::take_irq(irq::OST2))
        else {
            panic!("no spare timer");
        };
        let (ev, go) = BENCH.with(|b| (b.irq_event, b.touch_go));
        let mut x: u32 = 0x2026_1015;
        loop {
            if BENCH.with(|b| b.test != Test::Touch || b.stop) {
                wait(go);
                continue;
            }
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let at = timer.now().wrapping_add(20 * MS + (x >> 8) % (60 * MS));
            timer.arm(at);
            wait(ev);
            let _ = line.take();
            timer.ack();
            line.done();
            let input = BENCH.with(|b| {
                if b.test != Test::Touch || b.stop {
                    return None;
                }
                b.touches += 1;
                if b.pending == 0 {
                    b.pending = at | 1;
                }
                Some(b.input)
            });
            if let Some(input) = input {
                set(input);
            }
        }
    }

    fn reschedule() {
        let (next, deadline) = SCHED.with(|s| (s.schedule(rt::ticks()), s.next_deadline()));
        if let Some(at) = deadline
            && !rt::set_alarm(at)
        {
            let _ = rt::set_alarm(rt::ticks().wrapping_add(64));
        }
        let Some(next) = next else {
            return;
        };
        let cur = CURRENT.with(|c| core::mem::replace(c, next));
        if cur != next {
            thread::switch(cur.index(), next.index());
        }
    }

    fn wait(e: Eid) {
        if SCHED.with(|s| s.wait(e, None)) == Ok(None) {
            reschedule();
        }
    }

    fn set(e: Eid) {
        let _ = SCHED.with(|s| s.set_event(e));
        reschedule();
    }

    extern "C" fn idle_thread(_: usize) -> ! {
        loop {
            let due = match SCHED.with(|s| s.next_deadline()) {
                Some(at) => !rt::set_alarm(at),
                None => false,
            };
            if !due {
                rt::idle_until_interrupt(|| SCHED.with(|s| s.has_ready()));
            }
            reschedule();
        }
    }

    fn finish() -> ! {
        let me = CURRENT.with(|c| *c);
        let _ = SCHED.with(|s| s.exit(me));
        reschedule();
        unreachable!("an exited thread never runs again")
    }

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
        crash(format_args!("FAULT {} at {addr:#010x}", kind.name()))
    }

    #[inline(never)]
    fn crash(msg: core::fmt::Arguments) -> ! {
        let mut uart = PxaUart::new(rt::emergency_uart());
        let _ = writeln!(uart, "\n{msg}");
        rp_hal_x51v::devices::crash_to_loader(msg);
        halt()
    }

    fn halt() -> ! {
        loop {
            core::hint::spin_loop();
        }
    }

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        match info.location() {
            Some(l) => crash(format_args!("PANIC at {}:{}", l.file(), l.line())),
            None => crash(format_args!("PANIC")),
        }
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
