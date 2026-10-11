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
