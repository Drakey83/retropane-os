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
    use rp_hal_pxa27x::thread;
    use rp_hal_pxa27x::user::{self, Frame};
    use rp_hal_x51v::devices::take_ram_pool;
    use rp_kernel_sched::{Eid, Error, LOWEST, Scheduler, Tid};

    rp_hal_pxa27x::entry!(main);

    const ROUNDS: u32 = 1000;
    const SLEEPS: u32 = 5;
    /// 10 ms in OS-timer ticks (3.25 MHz).
    const SLEEP_TICKS: u32 = 32_500;

    /// The scheduler (all-zero when empty: `.bss`).
    static SCHED: Global<Scheduler> = Global::new(Scheduler::new());
    /// The thread on the CPU.
    static CURRENT: Global<Tid> = Global::new(Tid(0));
    static DEMO: Global<Demo> = Global::new(Demo {
        ping: Eid(0),
        pong: Eid(0),
        done: Eid(0),
        pingpong_ticks: 0,
        late_max: 0,
        wakes: 0,
    });

    /// System calls (user side numbers).
    const SYS_PRINT: u32 = 0;
    const SYS_YIELD: u32 = 1;
    const SYS_EXIT: u32 = 2;

    /// User stacks handed to the user threads.
    static USTACKS: Global<[Option<&'static mut [u32]>; 2]> = Global::new([None, None]);
    /// What the user threads printed, and how their calls looked.
    static ULOG: Global<UserLog> = Global::new(UserLog {
        text: [0; 16],
        len: 0,
        calls: 0,
        not_user: 0,
    });

    struct UserLog {
        text: [u8; 16],
        len: usize,
        calls: u32,
        not_user: u32,
    }

    struct Demo {
        ping: Eid,
        pong: Eid,
        done: Eid,
        pingpong_ticks: u32,
        late_max: u32,
        wakes: u32,
    }

    fn main() -> ! {
        let mut uart = PxaUart::new(rt::emergency_uart());
        uart.init(115_200);
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        let _ = uart.write_str("\nRetroPane 26 threads test\n");

        // Thread stacks: 4 KiB each from the RAM pool.
        let Some(pool) = take_ram_pool() else { halt() };
        let mut stacks = pool.chunks_exact_mut(1024);

        let started = SCHED.with(|s| -> Result<(), Error> {
            let main = s.create(0, 150)?;
            let _ = s.schedule(rt::ticks()); // this code is now `main`, running
            CURRENT.with(|c| *c = main);
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
        // One format call for both lines (each call site costs image size).
        let _ = writeln!(
            uart,
            "BENCH thread_switch {pp_ticks} {}\nBENCH sleep_lateness_max {late_max} 1",
            2 * ROUNDS
        );
        let mut ok = report(&mut uart, "THREADS ping-pong", pp_ticks > 0, pp_ticks);
        // On time: within 1 ms of each deadline (the emulator runs in real
        // time; a real X51v should do much better).
        ok &= report(
            &mut uart,
            "THREADS sleep",
            wakes == SLEEPS && late_max < 3250,
            late_max,
        );

        // User mode: two threads at equal priority, each printing its letter
        // and yielding three times through system calls.
        user::set_syscall_handler(on_syscall);
        let spawned = SCHED.with(|s| -> Result<(), Error> {
            for i in 0..2 {
                let kstack = stacks.next().ok_or(Error::Full)?;
                let ustack = stacks.next().ok_or(Error::Full)?;
                USTACKS.with(|u| u[i] = Some(ustack));
                let t = s.create(0, 200)?;
                thread::prepare(t.index(), kstack, user_trampoline, i);
            }
            Ok(())
        });
        if spawned.is_err() {
            let _ = uart.write_str("USER setup FAIL\n");
            halt();
        }
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
        if ok {
            let _ = uart.write_str(super::MARKER);
            let _ = uart.write_str("\n");
        }
        halt()
    }

    /// A scheduling point: runs the most urgent ready thread.
    fn reschedule() {
        // The idle thread is always ready, so there's always a next.
        let Some(next) = SCHED.with(|s| s.schedule(rt::ticks())) else {
            return;
        };
        let cur = CURRENT.with(|c| core::mem::replace(c, next));
        if cur != next {
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
            // Tickless: sleep until the next deadline, or any interrupt.
            match SCHED.with(|s| s.next_deadline()) {
                Some(at) => {
                    if rt::set_alarm(at) {
                        rt::wait_tick();
                    }
                }
                None => {
                    rt::wait_tick();
                }
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

    /// Kernel side of a user thread: enter user mode, never come back (only
    /// system calls do).
    extern "C" fn user_trampoline(i: usize) -> ! {
        let Some(stack) = USTACKS.with(|u| u[i].take()) else {
            finish()
        };
        user::enter_user(user_main, stack, i)
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
        let _ = SCHED.with(|s| s.exit(me));
        reschedule();
        unreachable!("an exited thread never runs again")
    }

    fn report(uart: &mut PxaUart<rp_hal_pxa27x::Mmio>, what: &str, pass: bool, got: u32) -> bool {
        let _ = uart.write_str(what);
        if pass {
            let _ = uart.write_str(" OK\n");
        } else {
            let _ = writeln!(uart, " FAIL (got {got})");
        }
        pass
    }

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
        let mut uart = PxaUart::new(rt::emergency_uart());
        let _ = uart.write_str("\nFAULT ");
        let _ = uart.write_str(kind.name());
        let _ = writeln!(uart, " at {addr:#010x}");
        halt()
    }

    fn halt() -> ! {
        loop {
            core::hint::spin_loop();
        }
    }

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        let mut uart = PxaUart::new(rt::emergency_uart());
        if let Some(l) = info.location() {
            let _ = uart.write_str("\nPANIC at ");
            let _ = uart.write_str(l.file());
            let _ = writeln!(uart, ":{}", l.line());
        }
        halt()
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
