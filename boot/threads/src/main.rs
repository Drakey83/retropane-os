//! Threads test image (ADR-0010): the scheduler and context switching live
//! on the CPU, in its own image so every subsystem's test harness has its own
//! size budget.
//!
//! The boot code becomes thread 1. Two equal-priority threads, ping and
//! pong, trade an event 1000 times (round-robin hand-off, and the cost of a
//! context switch); a more urgent sleeper wakes five times after 10 ms each;
//! an idle thread puts the CPU to sleep with a one-shot alarm (tickless) when
//! nothing else can run. Then two threads drop into **user mode** and make
//! system calls (`SWI`): each prints a letter and yields, three times, so the
//! output must be `ABABAB`, and every call's saved CPSR must say user mode.
//! Last, the KData page (ADR-0010 rule 8): two user threads read their TLS
//! pointer and thread ID at `0xFFFFC800` as WM programs' inlined macros do,
//! keep a value in TLS slot 0 across switches, and a write to the page
//! faults (user read-only).
//! Prints `THREADS ... OK` / `USER ... OK` lines and `BENCH` lines, then
//! [`MARKER`]; `cargo xtask emu` checks them.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed when every check passed.
pub const MARKER: &str = "RETROPANE-THREADS-OK";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_pxa_uart::PxaUart;
    use rp_hal_pxa27x::rt::{self, Global};
    use rp_hal_pxa27x::soc::irq;
    use rp_hal_pxa27x::user::{self, Frame};
    use rp_hal_pxa27x::{cpu, iwmmxt, thread};
    use rp_hal_x51v::devices::take_ram_pool;
    use rp_kernel_sched::{Eid, Error, LOWEST, Scheduler, Tid, kdata};
    use rp_kernel_trace::{Kind, Persistent};

    rp_hal_pxa27x::entry!(main);

    const ROUNDS: u32 = 1000;
    const SLEEPS: u32 = 5;
    /// 10 ms in OS-timer ticks (3.25 MHz).
    const SLEEP_TICKS: u32 = 32_500;

    /// The scheduler (all-zero when empty: `.bss`).
    static SCHED: Global<Scheduler> = Global::new(Scheduler::new());
    /// The thread on the CPU.
    static CURRENT: Global<Tid> = Global::new(Tid(0));
    /// The trace ring (item 54): in the trace region, carried on from the
    /// loader (or started here when the image runs on its own), so the
    /// switches can be read on the PC afterwards (`cargo xtask trace`).
    static TRACE: Global<Option<Persistent<'static>>> = Global::new(None);

    fn trace(kind: Kind, a: u32, b: u32) {
        TRACE.with(|t| {
            if let Some(t) = t {
                t.record(rt::ticks(), kind, a, b);
            }
        });
    }

    /// The KData page, written on every switch to another thread.
    static KDATA: Global<Option<&'static mut [u32; 1024]>> = Global::new(None);
    static DEMO: Global<Demo> = Global::new(Demo {
        ping: Eid(0),
        pong: Eid(0),
        done: Eid(0),
        pingpong_ticks: 0,
        late_max: 0,
        wakes: 0,
        mutex: Eid(0),
        order: [0; 4],
        olen: 0,
        fault_addr: 0,
        irq_event: Eid(0),
        irq_wakes: 0,
        irq_latency_max: 0,
    });

    /// System calls (user side numbers).
    const SYS_PRINT: u32 = 0;
    const SYS_YIELD: u32 = 1;
    const SYS_EXIT: u32 = 2;
    const SYS_MARK: u32 = 3;
    const SYS_KDATA: u32 = 4;
    /// Stays in the kernel (interrupts on) for three quanta: the quantum
    /// ends while the kernel runs.
    const SYS_STALL: u32 = 5;
    /// Marks per spinning thread, and the spin between marks (well over one
    /// quantum of work in the emulator and on the device).
    const MARKS: usize = 4;
    const SPIN: u32 = 3_000_000;
    /// Rounds (each 1/16 of a spin) a spinning thread waits for the other's
    /// first mark: many quanta, but finite.
    const WAIT_ROUNDS: u32 = 400;

    /// User stacks handed to the user threads.
    static USTACKS: Global<[Option<&'static mut [u32]>; 2]> = Global::new([None, None]);
    /// Where the user threads of the current pair start.
    static UENTRY: Global<Option<extern "C" fn(usize) -> !>> = Global::new(None);
    /// What the user threads printed, and how their calls looked.
    static ULOG: Global<UserLog> = Global::new(UserLog {
        text: [0; 16],
        len: 0,
        marks: [0; 16],
        mlen: 0,
        calls: 0,
        not_user: 0,
        kdata_ok: 0,
        stalled: false,
        b_after_stall: false,
    });

    struct UserLog {
        /// A thread is (or was) in `SYS_STALL`, and `B` marked after that.
        stalled: bool,
        b_after_stall: bool,
        text: [u8; 16],
        len: usize,
        marks: [u8; 16],
        mlen: usize,
        calls: u32,
        not_user: u32,
        /// KData reads that matched the kernel's view of the caller.
        kdata_ok: u32,
    }

    struct Demo {
        ping: Eid,
        pong: Eid,
        done: Eid,
        pingpong_ticks: u32,
        late_max: u32,
        wakes: u32,
        /// Priority inheritance: the mutex, and who got where in what order.
        mutex: Eid,
        order: [u8; 4],
        olen: usize,
        /// The address a user thread faulted on (caught by the kernel).
        fault_addr: u32,
        /// The event the timer interrupt sets, how often the
        /// interrupt-service thread woke, and its slowest wake-up.
        irq_event: Eid,
        irq_wakes: u32,
        irq_latency_max: u32,
    }

    /// Everything the test prints goes to the FFUART and, when the resident
    /// loader started it, into the run log too, so `cargo xtask load
    /// rp-threads` brings the real X51v's numbers (switch time, interrupt
    /// latency) to the PC.
    struct Tee(PxaUart<rp_hal_pxa27x::Mmio>);

    impl Write for Tee {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let _ = self.0.write_str(s);
            rp_hal_x51v::devices::log_write(s.as_bytes());
            Ok(())
        }
    }

    fn main() -> ! {
        let mut port = PxaUart::new(rt::emergency_uart());
        port.init(115_200);
        let mut uart = Tee(port);
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        let _ = uart.write_str("\nRetroPane 26 threads test\n");
        TRACE.with(|t| {
            // The loader's ring if it started this run, else a new one.
            *t = rp_hal_x51v::devices::take_trace_area().and_then(Persistent::resume);
        });
        trace(Kind::Mark, 1, 0);

        // Thread stacks: 4 KiB each from the RAM pool; its last MiB is the
        // FCSE test's frame.
        let Some(pool) = take_ram_pool() else { halt() };
        let (pool, fcse_mib) = pool.split_at_mut(pool.len() - (1 << 20) / 4);
        let mut stacks = pool.chunks_exact_mut(1024);

        let started = SCHED.with(|s| -> Result<(), Error> {
            let main = s.create(0, 150)?;
            let _ = s.schedule(rt::ticks()); // this code is now `main`, running
            CURRENT.with(|c| *c = main);
            let view = s.view(main);
            KDATA.with(|k| {
                *k = rt::take_kdata_page();
                if let Some(page) = k {
                    kdata::switch_to(page, view);
                }
            });
            let ping = s.create_event(false, false)?;
            let pong = s.create_event(false, false)?;
            let done = s.create_event(false, false)?;
            DEMO.with(|d| {
                d.ping = ping;
                d.pong = pong;
                d.done = done;
            });
            let threads: [(u8, extern "C" fn(usize) -> !); 4] = [
                (LOWEST, idle_thread),
                (200, ping_thread),
                (200, pong_thread),
                (100, sleeper_thread),
            ];
            for (prio, entry) in threads {
                let t = s.create(0, prio)?;
                let stack = stacks.next().ok_or(Error::Full)?;
                thread::prepare(t.index(), stack, entry, 0);
            }
            Ok(())
        });
        if started.is_err() {
            let _ = uart.write_str("THREADS setup FAIL\n");
            halt();
        }
        let done = DEMO.with(|d| d.done);
        wait(done); // one of: ping-pong finished, sleeper finished
        wait(done); // the other
        let (pp_ticks, late_max, wakes) = DEMO.with(|d| (d.pingpong_ticks, d.late_max, d.wakes));

        // Priority inversion: `low` holds the mutex, `high` waits for it, and
        // `middle` is ready. Inheritance runs `low` at `high`'s priority, so
        // the order is L(ow releases), H(igh gets it), M(iddle runs); without
        // it, `middle` would run first and `high` would wait behind it.
        let pi = SCHED.with(|s| -> Result<(), Error> {
            let m = s.create_mutex(false)?;
            DEMO.with(|d| d.mutex = m);
            let t = s.create(0, 220)?;
            thread::prepare(t.index(), stacks.next().ok_or(Error::Full)?, pi_low, 0);
            Ok(())
        });
        wait(done); // `low` holds the mutex
        let pi = pi.and_then(|()| {
            SCHED.with(|s| -> Result<(), Error> {
                for (prio, entry) in [
                    (180, pi_middle as extern "C" fn(usize) -> !),
                    (100, pi_high),
                ] {
                    let t = s.create(0, prio)?;
                    thread::prepare(t.index(), stacks.next().ok_or(Error::Full)?, entry, 0);
                }
                Ok(())
            })
        });
        if pi.is_ok() {
            wait(done); // `high` done
            wait(done); // `middle` done
        }
        let (order, olen) = DEMO.with(|d| (d.order, d.olen));

        // An interrupt as a waitable object: the first-level handler masks
        // the timer interrupt and the signal hook sets an event, which wakes
        // an interrupt-service thread waiting on it like on any event.
        let ist = (|| -> Option<()> {
            let ev = SCHED.with(|s| s.create_event(false, false)).ok()?;
            DEMO.with(|d| d.irq_event = ev);
            rt::set_signal_hook(on_signal);
            let t = SCHED.with(|s| s.create(0, 50)).ok()?;
            thread::prepare(t.index(), stacks.next()?, ist_thread, 0);
            Some(())
        })();
        if ist.is_some() {
            wait(done);
        }
        let (irq_wakes, irq_latency) = DEMO.with(|d| (d.irq_wakes, d.irq_latency_max));
        // One format call for both lines (each call site costs image size).
        let _ = writeln!(
            uart,
            "BENCH thread_switch {pp_ticks} {}\nBENCH sleep_lateness_max {late_max} 1",
            2 * ROUNDS
        );
        let mut ok = report(&mut uart, "THREADS ping-pong", pp_ticks > 0, pp_ticks);
        ok &= report(
            &mut uart,
            "THREADS interrupt thread",
            irq_wakes == IRQ_ROUNDS,
            irq_wakes,
        );
        let _ = writeln!(uart, "BENCH irq_to_thread_max {irq_latency} 1");
        ok &= report(
            &mut uart,
            "THREADS mutex inheritance",
            order[..olen] == *b"LHM",
            olen as u32,
        );
        // On time: within 5 ms of each deadline. The emulator runs in real
        // time on a shared host (a CI runner once woke 1.07 ms late); the
        // device's own bound is measured on the device (late_max is
        // reported), where it should be far under 1 ms.
        ok &= report(
            &mut uart,
            "THREADS sleep",
            wakes == SLEEPS && late_max < 5 * 3250,
            late_max,
        );

        // User mode: two threads at equal priority, each printing its letter
        // and yielding three times through system calls.
        user::set_syscall_handler(on_syscall);
        spawn_user_pair(&mut stacks, user_main);
        wait(done);
        wait(done);
        let (text, len, calls, not_user) = ULOG.with(|l| (l.text, l.len, l.calls, l.not_user));
        // 3 prints + 3 yields + 1 exit, per thread.
        ok &= report(
            &mut uart,
            "USER mode",
            calls == 14 && not_user == 0,
            not_user,
        );
        ok &= report(
            &mut uart,
            "USER yield",
            &text[..len] == b"ABABAB",
            len as u32,
        );

        // Preemption: two user threads spin without ever making a system
        // call (a mark after each burst). With a 5 ms quantum the timer must
        // take the CPU from one mid-spin, so the marks interleave; without
        // preemption they'd come out AAAABBBB.
        SCHED.with(|s| s.set_quantum(16_250));
        rt::set_preempt_hook(preempt);
        spawn_user_pair(&mut stacks, user_spin);
        wait(done);
        wait(done);
        let (marks, mlen) = ULOG.with(|l| (l.marks, l.mlen));
        let marks = &marks[..mlen];
        let a_done = marks.iter().rposition(|&m| m == b'A').unwrap_or(0);
        let b_first = marks.iter().position(|&m| m == b'B').unwrap_or(usize::MAX);
        let interleaved = mlen == 2 * MARKS && b_first < a_done;
        ok &= report(&mut uart, "USER preempted", interleaved, mlen as u32);

        // A quantum that ends inside the kernel: thread 0 spends three quanta
        // in a system call (the timer fires in kernel mode, where there is no
        // preemption), then spins in user mode with no system calls at all.
        // Thread 1 must still run first: the preemption owed is paid on the
        // way out of the system call. Without that it would be lost, and
        // thread 0 would keep the CPU (marks `SB`).
        let before = ULOG.with(|l| l.mlen);
        spawn_user_pair(&mut stacks, user_stall);
        wait(done);
        wait(done);
        let (marks, mlen, after) = ULOG.with(|l| (l.marks, l.mlen, l.b_after_stall));
        let owed = &marks[before..mlen] == b"BS" && after;
        ok &= report(
            &mut uart,
            "USER preempted after a system call",
            owed,
            mlen as u32,
        );

        // A user thread reads memory it may not touch: the kernel handles
        // the abort in that thread's context, ends just that thread, and the
        // other user thread carries on (ADR-0010 rule 5).
        user::set_user_fault_handler(on_user_fault);
        spawn_user_pair(&mut stacks, user_fault);
        wait(done); // the faulting thread, ended by the kernel
        wait(done); // the other one, which exits normally
        let (marks, mlen) = ULOG.with(|l| (l.marks, l.mlen));
        let addr = DEMO.with(|d| d.fault_addr);
        let contained = addr == BAD_ADDR && marks[..mlen].last() == Some(&b'C');
        ok &= report(&mut uart, "USER fault contained", contained, addr);

        // Lazy iWMMXt: two user threads keep their own value in media
        // register wR0 while yielding to each other. Each first use traps
        // once; every switch between them saves one's registers and loads
        // the other's, so both values must survive.
        iwmmxt::enable_lazy();
        spawn_user_pair(&mut stacks, user_wmmx);
        wait(done);
        wait(done);
        let (marks, mlen) = ULOG.with(|l| (l.marks, l.mlen));
        let traps = iwmmxt::traps();
        let kept = marks[..mlen].ends_with(b"WW") && traps >= 3;
        ok &= report(&mut uart, "USER iWMMXt lazy switch", kept, traps);

        // KData (ADR-0010 rule 8): two user threads read the TLS pointer and
        // their thread ID from 0xFFFFC800 as old programs' inlined macros
        // do, after every switch, and keep their own value in TLS slot 0
        // through it; the kernel checks each read against its view of the
        // caller. Then one writes to the page, which must fault (user
        // read-only) and leave the word as it was.
        DEMO.with(|d| d.fault_addr = 0);
        spawn_user_pair(&mut stacks, user_kdata);
        wait(done);
        wait(done);
        let good = ULOG.with(|l| l.kdata_ok);
        let addr = DEMO.with(|d| d.fault_addr);
        let untouched = KDATA.with(|k| k.as_ref().map(|p| p[KDATA_PROBE_WORD]) == Some(0));
        // 3 checks per thread, all matching.
        let pass = good == 6 && addr == KDATA_PROBE && untouched;
        ok &= report(&mut uart, "USER KData", pass, good);
        let native = fcse_native_slot(fcse_mib);
        ok &= report(&mut uart, "THREADS FCSE native slot", native, 0);
        if ok {
            let _ = uart.write_str(super::MARKER);
            let _ = uart.write_str("\n");
        }
        trace(Kind::Mark, 2, u32::from(ok));
        // Back to the resident loader, if it started this run.
        rp_hal_x51v::devices::return_to_loader();
        halt()
    }

    /// FCSE above the kernel (docs/research/process-limit.md): RetroPane's
    /// own processes live in slots 96-126, `0xC0000000` up, so a 7-bit
    /// process ID must relocate the bottom 32 MiB there on the real XScale,
    /// not only in the emulator. A word in a test MiB, mapped at slot 98:
    /// with process ID 98, address `0x40` reads it; with 97 nothing is
    /// there (an abort, caught).
    #[inline(never)]
    fn fcse_native_slot(frame: &'static mut [u32]) -> bool {
        const SLOT: u32 = 98;
        const OFF: u32 = 0x40;
        const MARK: u32 = 0x5107_0098;
        let phys = frame.as_ptr() as usize as u32;
        frame[(OFF / 4) as usize] = MARK;
        // To RAM before the other address reads it (VIVT: a second alias).
        cpu::clean_range(phys + OFF, 4);
        if !rt::map_test_section(SLOT << 25, phys) {
            return false;
        }
        let direct = rt::try_read((SLOT << 25 | OFF) as usize);
        cpu::set_pid(SLOT);
        let relocated = rt::try_read(OFF as usize);
        cpu::set_pid(SLOT - 1);
        let elsewhere = rt::try_read(OFF as usize);
        cpu::set_pid(0);
        direct == Some(MARK) && relocated == Some(MARK) && elsewhere.is_none()
    }

    /// Starts two user threads at priority 200 (a kernel stack and a user
    /// stack each), running `entry(0)` and `entry(1)` in user mode.
    #[inline(never)]
    fn spawn_user_pair(
        stacks: &mut impl Iterator<Item = &'static mut [u32]>,
        entry: extern "C" fn(usize) -> !,
    ) {
        UENTRY.with(|e| *e = Some(entry));
        for i in 0..2 {
            let (Some(kstack), Some(ustack)) = (stacks.next(), stacks.next()) else {
                panic!("out of stacks");
            };
            // The TLS block tops the user stack (zeroed, as CE does); the
            // identity map makes its kernel address the user one.
            let (ustack, tls) = ustack.split_at_mut(ustack.len() - kdata::TLS_BLOCK_WORDS);
            tls.fill(0);
            let top = tls.as_ptr_range().end as usize as u32;
            USTACKS.with(|u| u[i] = Some(ustack));
            let Ok(t) = SCHED.with(|s| {
                let t = s.create(0, 200)?;
                s.set_tls(t, kdata::tls_at(top)).map(|()| t)
            }) else {
                panic!("thread table full");
            };
            thread::prepare(t.index(), kstack, trampoline, i);
        }
    }

    /// Kernel side of a user thread: enter user mode at the pair's entry,
    /// never come back (only system calls do).
    extern "C" fn trampoline(i: usize) -> ! {
        let (Some(stack), Some(entry)) = (USTACKS.with(|u| u[i].take()), UENTRY.with(|e| *e))
        else {
            finish()
        };
        user::enter_user(entry, stack, i)
    }

    /// A scheduling point: runs the most urgent ready thread.
    fn reschedule() {
        let (next, deadline, view) = SCHED.with(|s| {
            let n = s.schedule(rt::ticks());
            (n, s.next_deadline(), n.map(|t| s.view(t)))
        });
        // Tickless: the timer fires only at the next deadline (the new
        // thread's quantum end if another could take over, or a sleeper). If
        // it has already passed, fire almost at once.
        if let Some(at) = deadline
            && !rt::set_alarm(at)
        {
            let _ = rt::set_alarm(rt::ticks().wrapping_add(64));
        }
        // The idle thread is always ready, so there's always a next.
        let Some(next) = next else {
            return;
        };
        let cur = CURRENT.with(|c| core::mem::replace(c, next));
        if cur != next {
            // KData says who runs (three words, through its one address).
            if let Some(view) = view {
                KDATA.with(|k| {
                    if let Some(page) = k {
                        kdata::switch_to(page, view);
                    }
                });
            }
            trace(Kind::Switch, u32::from(cur.0), u32::from(next.0));
            thread::switch(cur.index(), next.index());
        }
    }

    /// Blocks the running thread until `e` is set.
    fn wait(e: Eid) {
        if SCHED.with(|s| s.wait(e, None)) == Ok(None) {
            reschedule();
        }
    }

    /// Sets `e`, running a more urgent thread it released at once.
    fn set(e: Eid) {
        let _ = SCHED.with(|s| s.set_event(e));
        reschedule();
    }

    extern "C" fn idle_thread(_: usize) -> ! {
        loop {
            // Tickless: sleep until the next deadline or any interrupt (a
            // device's wakes its service thread through the signal hook).
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

    extern "C" fn ping_thread(_: usize) -> ! {
        let (ping, pong, done) = DEMO.with(|d| (d.ping, d.pong, d.done));
        let start = rt::ticks();
        for _ in 0..ROUNDS {
            set(pong);
            wait(ping);
        }
        let ticks = rt::ticks().wrapping_sub(start);
        DEMO.with(|d| d.pingpong_ticks = ticks);
        set(done);
        finish()
    }

    extern "C" fn pong_thread(_: usize) -> ! {
        let (ping, pong) = DEMO.with(|d| (d.ping, d.pong));
        loop {
            wait(pong);
            set(ping);
        }
    }

    extern "C" fn sleeper_thread(_: usize) -> ! {
        for _ in 0..SLEEPS {
            let want = rt::ticks().wrapping_add(SLEEP_TICKS);
            let _ = SCHED.with(|s| s.sleep_until(want));
            reschedule();
            let late = rt::ticks().wrapping_sub(want);
            DEMO.with(|d| {
                d.late_max = d.late_max.max(late);
                d.wakes += 1;
            });
        }
        let done = DEMO.with(|d| d.done);
        set(done);
        finish()
    }

    /// Timer interrupts the interrupt-service thread waits for.
    const IRQ_ROUNDS: u32 = 3;
    /// 2 ms between them (OS-timer ticks).
    const IRQ_GAP: u32 = 6_500;

    /// The signal hook (IRQ context): a claimed line fired, wake its waiter.
    fn on_signal(line: u32) {
        if line == irq::OST2 {
            let ev = DEMO.with(|d| d.irq_event);
            let _ = SCHED.with(|s| s.set_event(ev));
        }
    }

    /// An interrupt-service thread: waits for its interrupt like for any
    /// event, services the device, re-enables the line (CE's
    /// InterruptDone). Records how late each wake-up was.
    extern "C" fn ist_thread(_: usize) -> ! {
        let (ev, done) = DEMO.with(|d| (d.irq_event, d.done));
        // The thread owns its device and interrupt line (ADR-0007).
        // The timer first: taking it clears what an earlier image left
        // pending, before the line is claimed.
        let (Some(mut timer), Some(mut line)) = (rt::take_spare_timer(), rt::take_irq(irq::OST2))
        else {
            set(done);
            finish()
        };
        for _ in 0..IRQ_ROUNDS {
            let at = timer.now().wrapping_add(IRQ_GAP);
            timer.arm(at);
            wait(ev);
            let late = timer.now().wrapping_sub(at);
            let fired = line.take();
            timer.ack();
            line.done();
            DEMO.with(|d| {
                if fired {
                    d.irq_wakes += 1;
                }
                d.irq_latency_max = d.irq_latency_max.max(late);
            });
        }
        set(done);
        finish()
    }

    fn note(c: u8) {
        DEMO.with(|d| {
            if d.olen < d.order.len() {
                d.order[d.olen] = c;
                d.olen += 1;
            }
        });
    }

    extern "C" fn pi_low(_: usize) -> ! {
        let (m, done) = DEMO.with(|d| (d.mutex, d.done));
        wait(m); // free: taken at once
        set(done); // main (more urgent) starts `high` and `middle`
        // Back here only at `high`'s inherited priority: ahead of `middle`.
        note(b'L');
        let _ = SCHED.with(|s| s.release_mutex(m));
        reschedule();
        finish()
    }

    extern "C" fn pi_high(_: usize) -> ! {
        let (m, done) = DEMO.with(|d| (d.mutex, d.done));
        wait(m); // blocks: `low` holds it, and inherits this priority
        note(b'H');
        let _ = SCHED.with(|s| s.release_mutex(m));
        set(done);
        finish()
    }

    extern "C" fn pi_middle(_: usize) -> ! {
        note(b'M');
        let done = DEMO.with(|d| d.done);
        set(done);
        finish()
    }

    /// Runs in user mode and never yields: only preemption can switch it.
    /// After its first mark it spins until the other thread has marked too
    /// (asking without yielding), so the marks interleave exactly when the
    /// timer preempts, however fast or slow the host; bounded, so a missing
    /// preemption fails the test instead of hanging it.
    extern "C" fn user_spin(i: usize) -> ! {
        let me = u32::from(b'A') + i as u32;
        for j in 0..MARKS {
            let mut x = 0u32;
            for k in 0..SPIN {
                x = core::hint::black_box(x.wrapping_add(k));
            }
            user::syscall(SYS_MARK, me, 0, 0);
            if j == 0 {
                for _ in 0..WAIT_ROUNDS {
                    if user::syscall(SYS_MARK, 0, me, 0) > 0 {
                        break;
                    }
                    for k in 0..SPIN / 16 {
                        x = core::hint::black_box(x.wrapping_add(k));
                    }
                }
            }
        }
        user::syscall(SYS_EXIT, 0, 0, 0);
        loop {
            core::hint::spin_loop();
        }
    }

    /// Thread 0: three quanta in the kernel, then a long user-mode spin with
    /// no system calls, then mark `S`. Thread 1: mark `B` at once.
    extern "C" fn user_stall(i: usize) -> ! {
        if i == 0 {
            user::syscall(SYS_STALL, 0, 0, 0);
            let mut x = 0u32;
            for k in 0..2 * SPIN {
                x = core::hint::black_box(x.wrapping_add(k));
            }
            user::syscall(SYS_MARK, u32::from(b'S'), 0, 0);
        } else {
            user::syscall(SYS_MARK, u32::from(b'B'), 0, 0);
        }
        user::syscall(SYS_EXIT, 0, 0, 0);
        loop {
            core::hint::spin_loop();
        }
    }

    /// Puts a value of its own in wR0, yields a few times (the other thread
    /// uses wR0 meanwhile), then checks it: marks `W` if it survived.
    extern "C" fn user_wmmx(i: usize) -> ! {
        let mine = 0x5250_0000_0000_0026 | (i as u64 + 1) << 20;
        user::wmmx_write(mine);
        for _ in 0..3 {
            user::syscall(SYS_YIELD, 0, 0, 0);
        }
        let mark = if user::wmmx_read() == mine {
            b'W'
        } else {
            b'X'
        };
        user::syscall(SYS_MARK, u32::from(mark), 0, 0);
        user::syscall(SYS_EXIT, 0, 0, 0);
        loop {
            core::hint::spin_loop();
        }
    }

    /// A KData word no switch writes: the user thread's forbidden write
    /// goes here, so a write that got through would show.
    const KDATA_PROBE: u32 = kdata::BASE + 0x10;
    const KDATA_PROBE_WORD: usize = ((KDATA_PROBE - kdata::PAGE) / 4) as usize;

    /// Reads KData as WM programs' inlined macros do: `TlsSetValue(0, ..)`
    /// through the TLS pointer, then after each yield (the other thread ran
    /// meanwhile) the pointer, `GetCurrentThreadId` and `TlsGetValue(0)`,
    /// reported for the kernel to check. Thread 0 ends by writing to the
    /// page, which must fault.
    extern "C" fn user_kdata(i: usize) -> ! {
        let mine = 0x7150_0000 | i as u32;
        user::user_write(user::user_read(kdata::BASE), mine);
        for _ in 0..3 {
            user::syscall(SYS_YIELD, 0, 0, 0);
            let tls = user::user_read(kdata::BASE + kdata::off::TLS as u32);
            let id = user::user_read(kdata::BASE + kdata::off::CURRENT_THREAD as u32);
            let kept = user::user_read(tls) == mine;
            user::syscall(SYS_KDATA, tls, id, u32::from(kept));
        }
        if i == 0 {
            user::user_write(KDATA_PROBE, 0xBAD);
        }
        user::syscall(SYS_EXIT, 0, 0, 0);
        loop {
            core::hint::spin_loop();
        }
    }

    /// Unmapped in the bring-up map: any access aborts.
    const BAD_ADDR: u32 = 0x8000_0000;

    /// Thread 0 touches forbidden memory; thread 1 marks `C` and exits.
    extern "C" fn user_fault(i: usize) -> ! {
        if i == 0 {
            let _ = user::user_read(BAD_ADDR);
        } else {
            user::syscall(SYS_MARK, u32::from(b'C'), 0, 0);
            user::syscall(SYS_EXIT, 0, 0, 0);
        }
        loop {
            core::hint::spin_loop();
        }
    }

    /// A fault from user mode, on the faulting thread's kernel stack: here
    /// the thread is ended (later: the VM resolves page faults and resumes).
    fn on_user_fault(f: &mut Frame, kind: rt::Fault, addr: u32) {
        let caught = f.from_user() && kind == rt::Fault::DataAbort;
        DEMO.with(|d| d.fault_addr = if caught { addr } else { 0 });
        let done = DEMO.with(|d| d.done);
        let _ = SCHED.with(|s| s.set_event(done));
        finish()
    }

    /// After an interrupt from user mode: maybe switch threads.
    fn preempt() {
        reschedule();
    }

    /// Runs in user mode: everything it does goes through system calls.
    extern "C" fn user_main(i: usize) -> ! {
        for _ in 0..3 {
            user::syscall(SYS_PRINT, u32::from(b'A') + i as u32, 0, 0);
            user::syscall(SYS_YIELD, 0, 0, 0);
        }
        user::syscall(SYS_EXIT, 0, 0, 0);
        loop {
            core::hint::spin_loop();
        }
    }

    /// The kernel's system-call handler (on the calling thread's kernel
    /// stack: it may block or switch threads).
    fn on_syscall(f: &mut Frame) {
        ULOG.with(|l| {
            l.calls += 1;
            if !f.from_user() {
                l.not_user += 1;
            }
        });
        match f.number() {
            SYS_PRINT => {
                ULOG.with(|l| {
                    if l.len < l.text.len() {
                        l.text[l.len] = f.arg(0) as u8;
                        l.len += 1;
                    }
                });
                f.set_result(0);
            }
            SYS_MARK => {
                // A mark, or with mark 0 a query: how many marks are not
                // the caller's (arg 1). Neither yields.
                let (mark, mine) = (f.arg(0) as u8, f.arg(1) as u8);
                let others = ULOG.with(|l| {
                    if mark == b'B' && l.stalled {
                        l.b_after_stall = true;
                    }
                    if mark != 0 && l.mlen < l.marks.len() {
                        l.marks[l.mlen] = mark;
                        l.mlen += 1;
                    }
                    l.marks[..l.mlen].iter().filter(|&&m| m != mine).count()
                });
                f.set_result(others as u32);
            }
            SYS_STALL => {
                ULOG.with(|l| l.stalled = true);
                let quantum = 16_250;
                let start = rt::ticks();
                while rt::ticks().wrapping_sub(start) < 3 * quantum {
                    core::hint::spin_loop();
                }
                f.set_result(0);
            }
            SYS_KDATA => {
                // What the caller read must be what the kernel says it is.
                let view = SCHED.with(|s| s.running().map(|t| s.view(t)));
                if view.is_some_and(|v| {
                    v.tls != 0 && f.arg(0) == v.tls && f.arg(1) == v.thread_id && f.arg(2) == 1
                }) {
                    ULOG.with(|l| l.kdata_ok += 1);
                }
                f.set_result(0);
            }
            SYS_YIELD => {
                SCHED.with(|s| s.yield_now(rt::ticks()));
                reschedule();
                f.set_result(0);
            }
            SYS_EXIT => {
                let done = DEMO.with(|d| d.done);
                let _ = SCHED.with(|s| s.set_event(done));
                finish()
            }
            _ => f.set_result(u32::MAX),
        }
    }

    /// Ends the running thread.
    fn finish() -> ! {
        let me = CURRENT.with(|c| *c);
        iwmmxt::release(me.index());
        let _ = SCHED.with(|s| s.exit(me));
        reschedule();
        unreachable!("an exited thread never runs again")
    }

    fn report(uart: &mut Tee, what: &str, pass: bool, got: u32) -> bool {
        let _ = uart.write_str(what);
        if pass {
            let _ = uart.write_str(" OK\n");
        } else {
            let _ = writeln!(uart, " FAIL (got {got})");
        }
        pass
    }

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
        crash(format_args!("FAULT {} at {addr:#010x}", kind.name()))
    }

    /// Faults and panics: on serial, and the shared crash path (back to the
    /// loader with the report, if it started this run).
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
