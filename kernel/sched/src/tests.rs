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

// ---- the wake slice (ADR-0010 amendment) ------------------------------------

const SLICE: u32 = 10;

/// A busy thread `hog` running, and `ui` (same priority) blocked on `ev`
/// after a burst of `burst` ticks of work, at `now`.
fn hog_and_waiter(s: &mut Scheduler, burst: u32) -> (Tid, Tid, Eid, u32) {
    let ui = s.create(2, NORMAL).unwrap();
    let hog = s.create(2, NORMAL).unwrap();
    let ev = s.create_event(false, false).unwrap();
    assert_eq!(s.schedule(0), Some(ui));
    assert_eq!(s.wait(ev, None), Ok(None)); // works `burst`, then waits
    assert_eq!(s.schedule(burst), Some(hog));
    (hog, ui, ev, burst)
}

#[test]
fn ce_mode_a_woken_peer_waits_for_the_whole_quantum() {
    let mut s = sched();
    s.set_wake_slice(None);
    let (hog, ui, ev, t) = hog_and_waiter(&mut s, 50);
    s.set_event(ev).unwrap();
    assert_eq!(s.schedule(t + 100), Some(hog));
    assert_eq!(s.next_deadline(), Some(t + Q), "the hog's full quantum");
    assert_eq!(s.schedule(t + Q - 1), Some(hog));
    assert_eq!(s.schedule(t + Q), Some(ui));
}

#[test]
fn a_woken_peer_runs_within_the_wake_slice() {
    let mut s = sched();
    s.set_wake_slice(Some(SLICE));
    // The UI's last burst (5) is shorter than what the hog has used (100).
    let (hog, ui, ev, t) = hog_and_waiter(&mut s, 5);
    s.set_event(ev).unwrap();
    assert_eq!(s.schedule(t + 100), Some(hog), "not at once: a slice");
    assert_eq!(s.next_deadline(), Some(t + 100 + SLICE));
    assert_eq!(s.schedule(t + 100 + SLICE), Some(ui));
    assert_eq!(s.state(hog), State::Ready);
    // The UI starts its own fresh quantum.
    assert_eq!(s.next_deadline(), Some(t + 100 + SLICE + Q));
}

#[test]
fn the_running_peer_keeps_as_much_as_the_woken_one_last_used() {
    let mut s = sched();
    s.set_wake_slice(Some(SLICE));
    // The waker's last burst was 384: the hog keeps the CPU until it has
    // used 384 of its quantum, so neither can take more than the other.
    // (Bursts are counted in 64-tick units, rounded up.)
    let (hog, ui, ev, t) = hog_and_waiter(&mut s, 384);
    s.set_event(ev).unwrap();
    assert_eq!(s.schedule(t + 100), Some(hog));
    assert_eq!(s.next_deadline(), Some(t + 384));
    assert_eq!(s.schedule(t + 383), Some(hog));
    assert_eq!(s.schedule(t + 384), Some(ui));
}

#[test]
fn a_waker_that_used_a_whole_quantum_queues_at_the_back_as_in_ce() {
    let mut s = sched();
    s.set_wake_slice(Some(SLICE));
    let (hog, ui, ev, t) = hog_and_waiter(&mut s, Q);
    s.set_event(ev).unwrap();
    assert_eq!(s.schedule(t + 100), Some(hog));
    assert_eq!(s.next_deadline(), Some(t + Q));
    assert_eq!(s.schedule(t + Q), Some(ui));
}

#[test]
fn a_woken_peer_goes_ahead_of_ready_peers() {
    let mut s = sched();
    s.set_wake_slice(Some(SLICE));
    let ui = s.create(2, NORMAL).unwrap();
    let hog = s.create(2, NORMAL).unwrap();
    let hog2 = s.create(2, NORMAL).unwrap();
    let ev = s.create_event(false, false).unwrap();
    assert_eq!(s.schedule(0), Some(ui));
    assert_eq!(s.wait(ev, None), Ok(None));
    assert_eq!(s.schedule(5), Some(hog));
    s.set_event(ev).unwrap();
    assert_eq!(s.schedule(200), Some(hog));
    assert_eq!(s.schedule(200 + SLICE), Some(ui), "ahead of hog2");
    s.wait(ev, None).unwrap();
    assert_eq!(s.schedule(220), Some(hog2), "then the peers in order");
    assert_eq!(s.state(hog), State::Ready);
}

/// The touch path: a driver thread (more urgent) preempts the busy peer,
/// then wakes the UI thread.
fn driver_wakes_ui(slice: Option<u32>, burst: u32, hog_ran: u32) -> (Scheduler, Tid, Tid) {
    let mut s = sched();
    s.set_wake_slice(slice);
    let ui = s.create(2, NORMAL).unwrap();
    let hog = s.create(2, NORMAL).unwrap();
    let driver = s.create(2, 100).unwrap();
    let irq = s.create_event(false, false).unwrap();
    let input = s.create_event(false, false).unwrap();
    assert_eq!(s.schedule(0), Some(driver));
    s.wait(irq, None).unwrap();
    assert_eq!(s.schedule(0), Some(ui));
    s.wait(input, None).unwrap();
    assert_eq!(s.schedule(burst), Some(hog));
    let t = burst + hog_ran;
    s.set_event(irq).unwrap(); // the touch interrupt
    assert_eq!(s.schedule(t), Some(driver));
    s.set_event(input).unwrap();
    s.wait(irq, None).unwrap();
    (s, ui, hog)
}

#[test]
fn a_peer_preempted_by_the_driver_doesnt_keep_the_ui_waiting() {
    // The hog has used 300 of its quantum, more than the UI's last burst
    // (5): the UI runs as soon as the driver is done.
    let (mut s, ui, hog) = driver_wakes_ui(Some(SLICE), 5, 300);
    assert_eq!(s.schedule(310), Some(ui));
    assert_eq!(s.state(hog), State::Ready);
}

#[test]
fn a_peer_preempted_early_keeps_the_ui_s_last_burst_first() {
    // The hog had used only 20 when preempted; the UI's last burst was
    // 192: the hog runs 172 more (up to an equal share), then the UI.
    let (mut s, ui, hog) = driver_wakes_ui(Some(SLICE), 192, 20);
    assert_eq!(s.schedule(222), Some(hog));
    assert_eq!(s.next_deadline(), Some(222 + 172));
    assert_eq!(s.schedule(222 + 172), Some(ui));
}

#[test]
fn a_peer_that_ran_alone_past_its_quantum_has_used_it_all() {
    // The hog ran alone for 5000 (five quanta, no timer: tickless) when the
    // driver preempted it: the UI (last burst 192) goes first.
    let (mut s, ui, hog) = driver_wakes_ui(Some(SLICE), 192, 5000);
    assert_eq!(s.schedule(5200), Some(ui));
    assert_eq!(s.state(hog), State::Ready);
    // With nothing left, the hog renews when it runs again alone.
    s.exit(ui).unwrap();
    assert_eq!(s.schedule(5300), Some(hog));
    assert_eq!(s.next_deadline(), None);
    assert_eq!(s.schedule(5400), Some(hog));
}

#[test]
fn ce_mode_the_preempted_peer_finishes_its_quantum_first() {
    let (mut s, ui, hog) = driver_wakes_ui(None, 5, 300);
    assert_eq!(s.schedule(310), Some(hog), "its 700 left first");
    assert_eq!(s.next_deadline(), Some(310 + 700));
    assert_eq!(s.schedule(1010), Some(ui));
}

#[test]
fn sleeps_that_end_cut_the_quantum_too() {
    let mut s = sched();
    s.set_wake_slice(Some(SLICE));
    let ui = s.create(2, NORMAL).unwrap();
    let hog = s.create(2, NORMAL).unwrap();
    assert_eq!(s.schedule(0), Some(ui));
    s.sleep_until(300).unwrap(); // a frame tick at 300
    assert_eq!(s.schedule(3), Some(hog));
    assert_eq!(s.next_deadline(), Some(300), "the sleeper");
    assert_eq!(s.schedule(300), Some(hog));
    assert_eq!(s.next_deadline(), Some(300 + SLICE));
    assert_eq!(s.schedule(300 + SLICE), Some(ui));
}

#[test]
fn other_priorities_are_untouched() {
    let mut s = sched();
    s.set_wake_slice(Some(SLICE));
    // Less urgent: waits as always (strict priority).
    let low = s.create(2, 252).unwrap();
    let hog = s.create(2, NORMAL).unwrap();
    let peer = s.create(2, NORMAL).unwrap();
    let ev = s.create_event(false, false).unwrap();
    assert_eq!(s.schedule(0), Some(hog));
    s.exit(peer).unwrap();
    // `low` blocks only after `hog` sleeps; then wakes while `hog` runs.
    s.sleep_until(50).unwrap();
    assert_eq!(s.schedule(1), Some(low));
    s.wait(ev, None).unwrap();
    assert_eq!(s.schedule(50), Some(hog));
    s.set_event(ev).unwrap();
    assert_eq!(s.schedule(60), Some(hog));
    assert_eq!(s.next_deadline(), None, "no peer: no quantum to cut");
    assert_eq!(s.state(low), State::Ready);
}

#[test]
fn a_frequent_sleeper_takes_no_more_than_an_equal_share() {
    // A thread that works `b` then sleeps a tick, forever, against a busy
    // thread: with the wake slice the busy one still gets at least as much
    // CPU (CE: exactly a quantum each turn; here at least `b` each turn).
    for b in [5, 50, 300, 999] {
        let mut s = sched();
        s.set_wake_slice(Some(SLICE));
        let sleeper = s.create(2, NORMAL).unwrap();
        let hog = s.create(2, NORMAL).unwrap();
        let (mut now, mut hog_time, mut sleeper_time) = (0u32, 0u32, 0u32);
        let mut on = s.schedule(now).unwrap();
        let mut wake_at = None;
        while now < 100_000 {
            // Run whoever is on the CPU until its next event.
            let deadline = s.next_deadline().unwrap_or(now + Q);
            let until = if on == sleeper { now + b } else { deadline };
            let until = until.min(deadline).max(now + 1);
            if on == hog {
                hog_time += until - now;
            } else {
                sleeper_time += until - now;
            }
            now = until;
            if on == sleeper && wake_at.is_none() {
                s.sleep_until(now + 1).unwrap();
                wake_at = Some(now + 1);
            }
            if wake_at.is_some_and(|w| now >= w) {
                wake_at = None;
            }
            on = s.schedule(now).unwrap();
        }
        // (The sleeper may be one burst ahead when the window ends.)
        assert!(
            hog_time + b >= sleeper_time,
            "burst {b}: hog {hog_time}, sleeper {sleeper_time}"
        );
    }
}

#[test]
fn the_wake_slice_is_on_by_default_and_can_be_switched_off() {
    let s = Scheduler::new();
    assert_eq!(s.wake_slice(), Some(DEFAULT_WAKE_SLICE));
    let mut s = s;
    s.set_wake_slice(None);
    assert_eq!(s.wake_slice(), None);
    s.set_wake_slice(Some(0));
    assert_eq!(s.wake_slice(), Some(1), "a slice is at least a tick");
}

#[test]
fn the_last_thread_number_queues_waits_and_wakes() {
    // Thread 255: the highest 8-bit number, linked through the same u8
    // queue links as every other (0 is the sentinel, never a thread).
    let mut s = sched();
    let mut last = Tid(0);
    for _ in 0..MAX_THREADS {
        last = s.create(2, 200).unwrap();
    }
    assert_eq!(last, Tid(255));
    assert_eq!(last.index(), MAX_THREADS - 1);
    s.set_priority(last, 100).unwrap();
    assert_eq!(s.schedule(0), Some(last));
    let ev = s.create_event(false, false).unwrap();
    s.wait(ev, None).unwrap();
    assert_eq!(s.schedule(1), Some(Tid(1)));
    assert_eq!(s.set_event(ev), Ok(1));
    assert_eq!(s.schedule(2), Some(last), "woken and more urgent");
    s.exit(last).unwrap();
    assert_eq!(s.create(2, 200), Ok(Tid(255)), "its number is reused");
}

// ---- Game Mode: frozen processes (G2) ---------------------------------------

#[test]
fn a_frozen_process_runs_no_thread_and_thaws_in_order() {
    let mut s = sched();
    let a = s.create(3, NORMAL).unwrap();
    let b = s.create(3, NORMAL).unwrap();
    let game = s.create(4, NORMAL).unwrap();
    assert_eq!(s.schedule(0), Some(a));
    let f = s.freeze(3, 400);
    assert_eq!(
        f,
        Freeze {
            parked: 2,
            blocked: 0,
            held: 0
        }
    );
    assert!(s.is_frozen(3) && !s.is_frozen(4));
    assert_eq!((s.state(a), s.state(b)), (State::Frozen, State::Frozen));
    assert_eq!(s.running(), None, "the running thread left the CPU");
    for now in [400, 2000, 9000] {
        assert_eq!(s.schedule(now), Some(game), "only the game runs");
    }
    assert_eq!(
        s.freeze(3, 9000),
        Freeze::default(),
        "a second freeze changes nothing"
    );
    // A thread made in a frozen process waits for the thaw too.
    let c = s.create(3, NORMAL).unwrap();
    assert_eq!(s.state(c), State::Frozen);
    assert_eq!(s.thaw(3), 3);
    assert_eq!(s.thaw(3), 0, "a second thaw changes nothing");
    // The game's quantum (renewed at 9000) isn't cut by the thaw.
    assert_eq!(
        s.schedule(9500),
        Some(game),
        "the thaw doesn't cut the game's quantum"
    );
    assert_eq!(s.schedule(10_000), Some(a));
    assert_eq!(s.schedule(10_000 + Q), Some(b));
    assert_eq!(s.schedule(10_000 + 2 * Q), Some(c));
    assert_eq!(s.schedule(10_000 + 3 * Q), Some(game));
}

#[test]
fn the_frozen_running_thread_keeps_what_was_left_of_its_quantum() {
    let mut s = sched();
    let a = s.create(3, NORMAL).unwrap();
    assert_eq!(s.schedule(0), Some(a));
    s.freeze(3, 300);
    assert_eq!(s.schedule(300), None, "nothing else: idle");
    s.thaw(3);
    let _peer = s.create(5, NORMAL).unwrap();
    assert_eq!(s.schedule(5000), Some(a));
    assert_eq!(
        s.next_deadline(),
        Some(5000 + Q - 300),
        "700 ticks left, not a fresh 1000"
    );
}

#[test]
fn waits_and_deadlines_hold_while_frozen_and_a_wake_is_kept() {
    let mut s = sched();
    let ev = s.create_event(true, false).unwrap();
    let waiter = s.create(3, NORMAL).unwrap();
    let sleeper = s.create(3, NORMAL).unwrap();
    let game = s.create(4, LOWEST).unwrap();
    assert_eq!(s.schedule(0), Some(waiter));
    assert_eq!(s.wait(ev, None), Ok(None));
    assert_eq!(s.schedule(1), Some(sleeper));
    s.sleep_until(500).unwrap();
    assert_eq!(s.schedule(2), Some(game));
    assert_eq!(
        s.freeze(3, 2),
        Freeze {
            parked: 0,
            blocked: 2,
            held: 0
        }
    );
    // The event fires and the deadline passes while frozen: both kept.
    assert_eq!(s.set_event(ev), Ok(1));
    assert_eq!(s.schedule(600), Some(game));
    assert_eq!(
        (s.state(waiter), s.state(sleeper)),
        (State::Frozen, State::Frozen)
    );
    assert!(!s.has_ready(), "nothing of the frozen process is queued");
    assert_eq!(s.next_deadline(), None, "its timer doesn't wake the CPU");
    assert_eq!(s.thaw(3), 2);
    assert_eq!(
        s.wait_result(waiter),
        WaitResult::Signalled,
        "the wake wasn't lost"
    );
    assert_eq!(s.wait_result(sleeper), WaitResult::TimedOut);
    assert_eq!(s.schedule(700), Some(waiter), "more urgent than the game");
}

#[test]
fn a_frozen_waiter_never_takes_a_signal_or_a_mutex_from_one_that_runs() {
    let mut s = sched();
    let ev = s.create_event(false, false).unwrap();
    let m = s.create_mutex(false).unwrap();
    let owner = s.create(5, 10).unwrap();
    let frozen_w = s.create(3, 20).unwrap();
    let live_w = s.create(4, 30).unwrap();
    // The owner takes the mutex; both others queue for the mutex, frozen
    // first and more urgent.
    assert_eq!(s.schedule(0), Some(owner));
    assert_eq!(s.wait(m, None), Ok(Some(WaitResult::Signalled)));
    s.sleep_until(100).unwrap();
    assert_eq!(s.schedule(1), Some(frozen_w));
    assert_eq!(s.wait(m, None), Ok(None));
    assert_eq!(s.schedule(2), Some(live_w));
    assert_eq!(s.wait(m, None), Ok(None));
    s.freeze(3, 3);
    assert_eq!(s.schedule(100), Some(owner));
    assert_eq!(s.release_mutex(m), Ok(true));
    assert_eq!(
        s.mutex_owner_of(m),
        Some(live_w),
        "the frozen waiter is passed over"
    );
    assert_eq!(s.state(frozen_w), State::Blocked, "and keeps waiting");
    s.exit(owner).unwrap();
    // An auto-reset event: the same.
    let w1 = s.create(3, 40).unwrap();
    let w2 = s.create(4, 41).unwrap();
    s.thaw(3);
    assert_eq!(s.schedule(200), Some(live_w));
    s.exit(live_w).unwrap();
    assert_eq!(
        s.mutex_owner_of(m),
        Some(frozen_w),
        "thawed, it is handed the mutex next"
    );
    assert_eq!(s.schedule(202), Some(frozen_w));
    s.exit(frozen_w).unwrap();
    assert_eq!(s.schedule(203), Some(w1));
    assert_eq!(s.wait(ev, None), Ok(None));
    assert_eq!(s.schedule(204), Some(w2));
    assert_eq!(s.wait(ev, None), Ok(None));
    s.freeze(3, 205);
    assert_eq!(s.set_event(ev), Ok(1));
    assert_eq!(
        s.state(w2),
        State::Ready,
        "the running process's waiter took it"
    );
    assert_eq!(
        s.state(w1),
        State::Blocked,
        "the frozen one keeps its place"
    );
    assert_eq!(s.set_event(ev), Ok(1));
    assert_eq!(
        s.state(w1),
        State::Frozen,
        "with no other waiter it takes the next"
    );
}

#[test]
fn freeze_counts_mutexes_a_running_process_waits_for() {
    let mut s = sched();
    let m = s.create_mutex(false).unwrap();
    let holder = s.create(3, 10).unwrap();
    let waiter = s.create(4, 20).unwrap();
    assert_eq!(s.schedule(0), Some(holder));
    assert_eq!(s.wait(m, None), Ok(Some(WaitResult::Signalled)));
    s.sleep_until(50).unwrap();
    assert_eq!(s.schedule(1), Some(waiter));
    assert_eq!(s.wait(m, None), Ok(None));
    assert_eq!(
        s.freeze(3, 2),
        Freeze {
            parked: 0,
            blocked: 1,
            held: 1
        }
    );
    // Exiting a frozen thread frees it as any other.
    s.thaw(3);
    s.freeze(3, 3);
    s.exit(holder).unwrap();
    assert_eq!(s.mutex_owner_of(m), Some(waiter));
}

#[test]
fn freezing_one_slot_leaves_its_neighbours_in_the_bitmap_alone() {
    let mut s = sched();
    for slot in [0u8, 31, 32, 63, 255] {
        s.freeze(slot, 0);
    }
    for slot in 0..=255u8 {
        assert_eq!(
            s.is_frozen(slot),
            [0, 31, 32, 63, 255].contains(&slot),
            "slot {slot}"
        );
    }
    s.thaw(32);
    assert!(!s.is_frozen(32) && s.is_frozen(31) && s.is_frozen(63));
}
