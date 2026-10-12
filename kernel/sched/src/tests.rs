use super::*;

const Q: u32 = 1000;

fn sched() -> Scheduler {
    let mut s = Scheduler::new();
    s.set_quantum(Q);
    s
}

#[test]
fn the_most_urgent_ready_thread_runs() {
    let mut s = sched();
    let low = s.create(2, 251).unwrap();
    let high = s.create(2, 100).unwrap();
    let idle = s.create(2, LOWEST).unwrap();
    assert_eq!(s.schedule(0), Some(high));
    assert_eq!(s.state(high), State::Running);
    assert_eq!(s.state(low), State::Ready);
    s.exit(high).unwrap();
    assert_eq!(s.schedule(1), Some(low));
    s.exit(low).unwrap();
    assert_eq!(s.schedule(2), Some(idle));
    s.exit(idle).unwrap();
    assert_eq!(s.schedule(3), None, "nothing ready: idle");
}

#[test]
fn equal_priorities_round_robin_on_quantum_expiry() {
    let mut s = sched();
    let a = s.create(2, NORMAL).unwrap();
    let b = s.create(2, NORMAL).unwrap();
    let c = s.create(2, NORMAL).unwrap();
    assert_eq!(s.schedule(0), Some(a));
    assert_eq!(s.schedule(500), Some(a), "quantum not over yet");
    assert_eq!(s.schedule(1000), Some(b));
    assert_eq!(s.schedule(2000), Some(c));
    assert_eq!(s.schedule(3000), Some(a), "back to the first");
}

#[test]
fn a_lone_thread_keeps_the_cpu_and_needs_no_timer() {
    let mut s = sched();
    let a = s.create(2, NORMAL).unwrap();
    assert_eq!(s.schedule(0), Some(a));
    assert_eq!(s.next_deadline(), None, "tickless: nothing to preempt it");
    assert_eq!(s.schedule(5000), Some(a));
    let _b = s.create(2, NORMAL).unwrap();
    assert_eq!(s.next_deadline(), Some(5000 + Q), "now its quantum matters");
}

#[test]
fn waking_a_more_urgent_thread_preempts_and_the_victim_keeps_its_quantum() {
    let mut s = sched();
    let a = s.create(2, NORMAL).unwrap();
    let b = s.create(2, NORMAL).unwrap();
    let irq = s.create(2, 50).unwrap();
    let ev = s.create_event(false, false).unwrap();
    assert_eq!(s.schedule(0), Some(irq));
    assert_eq!(s.wait(ev, None), Ok(None)); // the IST waits for its interrupt
    assert_eq!(s.schedule(10), Some(a));
    // 300 ticks into a's quantum the interrupt fires.
    assert_eq!(s.set_event(ev), Ok(1));
    assert_eq!(s.schedule(310), Some(irq));
    assert_eq!(s.wait_result(irq), WaitResult::Signalled);
    assert_eq!(s.wait(ev, None), Ok(None));
    // a resumes with what was left of its quantum (10 + 1000 - 310 = 700).
    assert_eq!(s.schedule(320), Some(a));
    assert_eq!(s.schedule(1019), Some(a));
    assert_eq!(
        s.schedule(1020),
        Some(b),
        "its remaining 700 ticks were used up"
    );
}

#[test]
fn sleepers_wake_at_their_deadline_in_order() {
    let mut s = sched();
    let a = s.create(2, 200).unwrap();
    let b = s.create(2, 200).unwrap();
    let idle = s.create(2, LOWEST).unwrap();
    assert_eq!(s.schedule(0), Some(a));
    s.sleep_until(500).unwrap();
    assert_eq!(s.schedule(0), Some(b));
    s.sleep_until(300).unwrap();
    assert_eq!(s.schedule(0), Some(idle));
    assert_eq!(s.next_deadline(), Some(300), "the earliest sleeper");
    assert_eq!(s.schedule(299), Some(idle));
    assert_eq!(s.schedule(300), Some(b));
    assert_eq!(s.next_deadline(), Some(500));
    s.sleep_until(10_000).unwrap();
    assert_eq!(s.schedule(600), Some(a));
}

#[test]
fn deadlines_survive_timer_wraparound() {
    let mut s = sched();
    let a = s.create(2, 200).unwrap();
    let idle = s.create(2, LOWEST).unwrap();
    let near_end = u32::MAX - 100;
    assert_eq!(s.schedule(near_end), Some(a));
    s.sleep_until(near_end.wrapping_add(300)).unwrap(); // wraps to 199
    assert_eq!(s.schedule(near_end + 50), Some(idle));
    assert_eq!(
        s.schedule(150),
        Some(idle),
        "after the wrap, before the deadline"
    );
    assert_eq!(s.schedule(199), Some(a));
}

#[test]
fn auto_reset_events_release_one_manual_release_all() {
    let mut s = sched();
    let t: Vec<_> = (0..3).map(|_| s.create(2, NORMAL).unwrap()).collect();
    let auto = s.create_event(false, false).unwrap();
    let manual = s.create_event(true, false).unwrap();
    for _ in 0..3 {
        s.schedule(0);
        assert_eq!(s.wait(auto, None), Ok(None));
    }
    assert_eq!(s.set_event(auto), Ok(1));
    assert_eq!(s.state(t[0]), State::Ready, "FIFO: the first waiter");
    assert_eq!(s.state(t[1]), State::Blocked);
    // Waiting on a set event returns at once; auto reset consumes it.
    s.schedule(0);
    assert_eq!(s.wait(manual, None), Ok(None));
    assert_eq!(s.set_event(manual), Ok(1));
    assert_eq!(s.set_event(auto), Ok(1));
    assert_eq!(s.set_event(auto), Ok(1));
    assert_eq!(s.set_event(auto), Ok(0), "no waiters: it stays set");
    let r = s.schedule(0).unwrap();
    assert_eq!(s.wait(auto, None), Ok(Some(WaitResult::Signalled)));
    assert_eq!(s.wait(auto, None), Ok(None), "consumed");
    let _ = r;
    // Manual reset releases everyone and stays set.
    let m = s.create_event(true, false).unwrap();
    let mut n = 0;
    while let Some(x) = s.schedule(0) {
        if s.state(x) == State::Running && s.wait(m, None) == Ok(None) {
            n += 1;
        } else {
            break;
        }
    }
    assert_eq!(s.set_event(m), Ok(n));
    s.schedule(0);
    assert_eq!(s.wait(m, None), Ok(Some(WaitResult::Signalled)));
    s.reset_event(m).unwrap();
    assert_eq!(s.wait(m, None), Ok(None));
}

#[test]
fn timed_waits_time_out_and_leave_the_queue() {
    let mut s = sched();
    let a = s.create(2, 100).unwrap();
    let b = s.create(2, 100).unwrap();
    let ev = s.create_event(false, false).unwrap();
    assert_eq!(s.schedule(0), Some(a));
    assert_eq!(s.wait(ev, Some(400)), Ok(None));
    assert_eq!(s.schedule(0), Some(b));
    assert_eq!(s.wait(ev, None), Ok(None));
    assert_eq!(s.schedule(400), Some(a), "timed out");
    assert_eq!(s.wait_result(a), WaitResult::TimedOut);
    // The next set goes to b, not to the timed-out a.
    assert_eq!(s.set_event(ev), Ok(1));
    assert_eq!(s.state(b), State::Ready);
    assert_eq!(s.wait_result(b), WaitResult::Signalled);
    // A signal before the deadline cancels the timeout.
    s.exit(a).unwrap();
    assert_eq!(s.schedule(401), Some(b));
    assert_eq!(s.wait(ev, Some(900)), Ok(None));
    s.set_event(ev).unwrap();
    assert_eq!(s.schedule(500), Some(b));
    assert_eq!(s.next_deadline(), None, "no stale sleeper left behind");
}

#[test]
fn exiting_threads_in_every_state() {
    let mut s = sched();
    let ready = s.create(2, 200).unwrap();
    let blocked = s.create(2, 100).unwrap();
    let sleeping = s.create(2, 90).unwrap();
    let ev = s.create_event(false, false).unwrap();
    assert_eq!(s.schedule(0), Some(sleeping));
    s.sleep_until(1000).unwrap();
    assert_eq!(s.schedule(0), Some(blocked));
    s.wait(ev, Some(2000)).unwrap();
    assert_eq!(s.schedule(0), Some(ready));
    s.exit(blocked).unwrap();
    s.exit(sleeping).unwrap();
    assert_eq!(s.next_deadline(), None, "both left the sleep queue");
    assert_eq!(s.set_event(ev), Ok(0), "and the wait queue");
    s.exit(ready).unwrap();
    assert_eq!(s.running(), None);
    assert_eq!(s.schedule(5000), None);
    assert_eq!(s.exit(ready), Err(Error::Invalid));
}

#[test]
fn priority_changes_requeue() {
    let mut s = sched();
    let a = s.create(2, 200).unwrap();
    let b = s.create(2, 150).unwrap();
    assert_eq!(s.schedule(0), Some(b));
    s.set_priority(a, 10).unwrap();
    assert_eq!(s.schedule(1), Some(a), "a now outranks b");
    assert_eq!(s.priority(a), 10);
}

#[test]
fn the_clz_map_finds_the_minimum_for_every_priority_set() {
    // Random sets of ready priorities: the scheduler must always pick the
    // numerically smallest.
    let mut rng = 0x1357_9BDF_2468_ACE0u64;
    let mut next = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    for _ in 0..500 {
        let mut s = sched();
        let n = 1 + next() % 40;
        let mut prios = Vec::new();
        for _ in 0..n {
            let p = (next() % 256) as u8;
            s.create(2, p).unwrap();
            prios.push(p);
        }
        let t = s.schedule(0).unwrap();
        assert_eq!(s.priority(t), *prios.iter().min().unwrap());
    }
    // Every single priority on its own.
    for p in 0..=255u8 {
        let mut s = sched();
        let t = s.create(2, p).unwrap();
        assert_eq!(s.schedule(0), Some(t));
    }
}

#[test]
fn tables_are_bounded() {
    let mut s = sched();
    for _ in 0..MAX_THREADS {
        s.create(2, NORMAL).unwrap();
    }
    assert_eq!(s.create(2, NORMAL), Err(Error::Full));
    for _ in 0..MAX_EVENTS {
        s.create_event(false, false).unwrap();
    }
    assert_eq!(s.create_event(false, false), Err(Error::Full));
    assert_eq!(s.set_event(Eid(200)), Err(Error::Invalid));
}

#[test]
fn an_empty_scheduler_is_all_zeros() {
    // So a `static` scheduler lives in .bss and costs nothing in the image.
    // Checked through its observable state: no threads, no events, nothing
    // running, the default quantum.
    let s = Scheduler::new();
    assert_eq!(s.running(), None);
    assert_eq!(s.next_deadline(), None);
    assert_eq!(s.quantum(), DEFAULT_QUANTUM);
    assert_eq!(s.state(Tid(0)), State::Free, "slot 0 is the sentinel");
    let mut s = s;
    assert_eq!(s.create(2, NORMAL), Ok(Tid(1)), "threads start at 1");
    assert_eq!(Tid(1).index(), 0);
    assert_eq!(s.create_event(false, false), Ok(Eid(1)));
    assert_eq!(s.exit(Tid(0)), Err(Error::Invalid));
}

// ---- mutexes and priority inheritance -------------------------------------

#[test]
fn a_free_mutex_is_taken_at_once_and_is_recursive() {
    let mut s = sched();
    let a = s.create(2, NORMAL).unwrap();
    assert_eq!(s.schedule(0), Some(a));
    let m = s.create_mutex(false).unwrap();
    assert_eq!(s.wait(m, None), Ok(Some(WaitResult::Signalled)));
    assert_eq!(
        s.wait(m, None),
        Ok(Some(WaitResult::Signalled)),
        "recursive"
    );
    assert_eq!(s.mutex_owner_of(m), Some(a));
    assert_eq!(s.release_mutex(m), Ok(false));
    assert_eq!(s.mutex_owner_of(m), Some(a), "still held once");
    assert_eq!(s.release_mutex(m), Ok(false));
    assert_eq!(s.mutex_owner_of(m), None);
    assert_eq!(
        s.release_mutex(m),
        Err(Error::Invalid),
        "not the owner any more"
    );
}

#[test]
fn priority_inversion_is_prevented() {
    // The classic case: low owns the mutex, high waits for it, and a
    // middle-priority thread is ready. Without inheritance, middle runs and
    // high waits indefinitely.
    let mut s = sched();
    let low = s.create(2, 200).unwrap();
    assert_eq!(s.schedule(0), Some(low));
    let m = s.create_mutex(true).unwrap();
    let high = s.create(2, 100).unwrap();
    let middle = s.create(2, 150).unwrap();
    assert_eq!(s.schedule(1), Some(high));
    assert_eq!(s.wait(m, None), Ok(None), "high blocks on low's mutex");
    assert_eq!(s.priority(low), 100, "low inherits high's priority");
    assert_eq!(s.base_priority(low), 200);
    assert_eq!(s.schedule(2), Some(low), "low runs ahead of middle");
    assert_eq!(s.release_mutex(m), Ok(true));
    assert_eq!(s.priority(low), 200, "inheritance ends with the release");
    assert_eq!(s.mutex_owner_of(m), Some(high));
    assert_eq!(s.schedule(3), Some(high), "high gets the mutex and runs");
    assert_eq!(s.wait_result(high), WaitResult::Signalled);
    s.exit(high).unwrap();
    assert_eq!(s.schedule(4), Some(middle));
}

#[test]
fn the_most_urgent_waiter_gets_the_mutex_first() {
    let mut s = sched();
    let owner = s.create(2, 200).unwrap();
    assert_eq!(s.schedule(0), Some(owner));
    let m = s.create_mutex(true).unwrap();
    let a = s.create(2, 180).unwrap();
    let b = s.create(2, 120).unwrap();
    assert_eq!(s.schedule(1), Some(b));
    assert_eq!(s.wait(m, None), Ok(None));
    assert_eq!(s.schedule(2), Some(owner), "inherited 120, ahead of a");
    s.set_priority(a, 110).unwrap();
    assert_eq!(s.schedule(3), Some(a));
    assert_eq!(s.wait(m, None), Ok(None));
    assert_eq!(
        s.priority(owner),
        110,
        "now inherits the more urgent waiter"
    );
    assert_eq!(s.schedule(4), Some(owner));
    s.release_mutex(m).unwrap();
    assert_eq!(s.mutex_owner_of(m), Some(a), "110 before 120");
    assert_eq!(s.priority(a), 110);
}

#[test]
fn a_new_owner_inherits_from_the_remaining_waiters() {
    let mut s = sched();
    let owner = s.create(2, 100).unwrap();
    assert_eq!(s.schedule(0), Some(owner));
    let m = s.create_mutex(true).unwrap();
    let urgent = s.create(2, 50).unwrap();
    let lazy = s.create(2, 240).unwrap();
    assert_eq!(s.schedule(1), Some(urgent));
    assert_eq!(s.wait(m, None), Ok(None));
    // Make `lazy` more urgent than `urgent` for a moment so it can queue.
    s.set_priority(lazy, 40).unwrap();
    assert_eq!(s.schedule(2), Some(lazy));
    assert_eq!(s.wait(m, None), Ok(None));
    s.set_priority(lazy, 240).unwrap(); // base drops; it still waits
    assert_eq!(s.schedule(3), Some(owner));
    s.release_mutex(m).unwrap();
    assert_eq!(s.mutex_owner_of(m), Some(urgent));
    assert_eq!(s.priority(urgent), 50, "the waiter left is less urgent");
}

#[test]
fn inheritance_follows_a_chain_of_owners() {
    // c waits for m2 (held by b), b waits for m1 (held by a): a runs at c's
    // priority.
    let mut s = sched();
    let a = s.create(2, 220).unwrap();
    assert_eq!(s.schedule(0), Some(a));
    let m1 = s.create_mutex(true).unwrap();
    let b = s.create(2, 200).unwrap();
    assert_eq!(s.schedule(1), Some(b));
    let m2 = s.create_mutex(true).unwrap();
    assert_eq!(s.wait(m1, None), Ok(None));
    let c = s.create(2, 60).unwrap();
    assert_eq!(s.schedule(2), Some(c));
    assert_eq!(s.wait(m2, None), Ok(None));
    assert_eq!(s.priority(b), 60);
    assert_eq!(s.priority(a), 60, "passed along the chain");
    assert_eq!(s.schedule(3), Some(a));
}

#[test]
fn a_timed_out_waiter_stops_lending_its_priority() {
    let mut s = sched();
    let owner = s.create(2, 200).unwrap();
    assert_eq!(s.schedule(0), Some(owner));
    let m = s.create_mutex(true).unwrap();
    let h = s.create(2, 100).unwrap();
    assert_eq!(s.schedule(1), Some(h));
    assert_eq!(s.wait(m, Some(500)), Ok(None));
    assert_eq!(s.priority(owner), 100);
    assert_eq!(s.schedule(2), Some(owner));
    assert_eq!(s.schedule(500), Some(h), "the wait timed out");
    assert_eq!(s.wait_result(h), WaitResult::TimedOut);
    assert_eq!(s.priority(owner), 200, "boost withdrawn");
    assert_eq!(s.mutex_owner_of(m), Some(owner));
}

#[test]
fn an_exiting_owner_passes_its_mutexes_on() {
    let mut s = sched();
    let owner = s.create(2, 200).unwrap();
    assert_eq!(s.schedule(0), Some(owner));
    let m = s.create_mutex(true).unwrap();
    let w = s.create(2, 100).unwrap();
    assert_eq!(s.schedule(1), Some(w));
    assert_eq!(s.wait(m, None), Ok(None));
    s.exit(owner).unwrap();
    assert_eq!(s.mutex_owner_of(m), Some(w), "abandoned mutex handed on");
    assert_eq!(s.schedule(2), Some(w));
}

#[test]
fn mutexes_are_not_events() {
    let mut s = sched();
    let a = s.create(2, NORMAL).unwrap();
    assert_eq!(s.schedule(0), Some(a));
    let m = s.create_mutex(false).unwrap();
    assert_eq!(s.set_event(m), Err(Error::Invalid));
    assert_eq!(s.reset_event(m), Err(Error::Invalid));
    let e = s.create_event(false, false).unwrap();
    assert_eq!(s.release_mutex(e), Err(Error::Invalid));
}

#[test]
fn set_priority_keeps_an_inherited_boost() {
    let mut s = sched();
    let owner = s.create(2, 200).unwrap();
    assert_eq!(s.schedule(0), Some(owner));
    let m = s.create_mutex(true).unwrap();
    let h = s.create(2, 90).unwrap();
    assert_eq!(s.schedule(1), Some(h));
    assert_eq!(s.wait(m, None), Ok(None));
    s.set_priority(owner, 230).unwrap();
    assert_eq!(s.priority(owner), 90, "still boosted");
    assert_eq!(s.base_priority(owner), 230);
}

#[test]
fn the_kdata_view_follows_the_thread() {
    let mut s = sched();
    let a = s.create(5, NORMAL).unwrap();
    let b = s.create(6, NORMAL).unwrap();
    s.set_tls(a, 0x0001_FF00).unwrap();
    let view = s.view(a);
    assert_eq!(
        view,
        kdata::ThreadView {
            tls: 0x0001_FF00,
            thread_id: u32::from(a.0),
            process_id: 5,
        }
    );
    assert_ne!(view.thread_id, 0, "programs treat ID 0 as none");
    assert_eq!(s.view(b).tls, 0, "no TLS until one is set");
    assert_eq!(s.view(b).process_id, 6);
    // A new thread in an exited one's table slot starts without TLS.
    s.exit(a).unwrap();
    assert_eq!(s.set_tls(a, 1), Err(Error::Invalid));
    let c = s.create(5, NORMAL).unwrap();
    assert_eq!(c, a);
    assert_eq!(s.view(c).tls, 0);
}
