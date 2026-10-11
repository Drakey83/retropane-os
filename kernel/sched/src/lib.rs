//! The scheduler (ADR-0010): pure logic, tested on the host.
//!
//! - **256 priorities, CE numbering:** 0 is the highest, applications default
//!   to 251 ([`NORMAL`]). Within a priority, threads take turns (FIFO,
//!   round-robin on quantum expiry).
//! - **O(1) with `CLZ`:** a 256-bit ready map plus an 8-bit summary. Bits are
//!   stored highest-priority-first from the top of each word, so finding the
//!   most urgent ready thread is two count-leading-zeros (one instruction
//!   each on ARMv5, which has `CLZ` but no count-trailing-zeros).
//! - **Tickless:** there's no periodic tick. [`Scheduler::next_deadline`]
//!   says when the timer must next fire (the running thread's quantum end or
//!   the earliest sleeper), and the kernel programs a one-shot alarm for it.
//! - **Bounded:** a fixed thread table ([`MAX_THREADS`]); queues are linked
//!   through it, so nothing allocates.
//! - **All zeros when empty:** table slot 0 is a sentinel and real threads and
//!   events start at 1, so a link of 0 means "none" and an empty scheduler is
//!   plain zeroed memory (`.bss`: nothing in the kernel image).
//!
//! The kernel owns the mechanics (stacks, context switch); this crate only
//! decides who runs.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

/// Highest priority.
pub const HIGHEST: u8 = 0;
/// CE's default application priority (`THREAD_PRIORITY_NORMAL`).
pub const NORMAL: u8 = 251;
/// Lowest priority (the idle level).
pub const LOWEST: u8 = 255;
/// Thread table size.
pub const MAX_THREADS: usize = 128;
/// CE's default quantum: 100 ms, in OS-timer ticks (3.25 MHz).
pub const DEFAULT_QUANTUM: u32 = 325_000;

/// A thread's number (1-based: 0 is the table's sentinel).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(test, derive(Debug))]
pub struct Tid(pub u8);

impl Tid {
    /// A 0-based index, for the kernel's per-thread arrays (contexts,
    /// stacks).
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize - 1
    }
}

/// What a thread is doing.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum State {
    /// Slot unused.
    Free,
    /// Waiting for the CPU.
    Ready,
    /// On the CPU.
    Running,
    /// Waiting for an event (or a sleep deadline).
    Blocked,
}

/// "No thread / no event": the sentinel slot 0.
const NONE: u8 = 0;

#[derive(Clone, Copy)]
struct Thread {
    state: State,
    /// Effective priority: `base`, or higher while a more urgent thread
    /// waits for a mutex this one owns (priority inheritance).
    priority: u8,
    /// The priority it was given.
    base: u8,
    /// The process slot it belongs to (for the FCSE PID and domains).
    slot: u8,
    /// Ready-queue / wait-queue links.
    next: u8,
    prev: u8,
    /// Sleep-queue link and deadline (when `sleeping`).
    sleep_next: u8,
    sleeping: bool,
    deadline: u32,
    /// Quantum left when it last stopped running.
    quantum: u32,
    /// The event it waits on (`NONE` if none), and whether it was signalled
    /// (as opposed to timing out).
    waiting_on: u8,
    signalled: bool,
}

const EMPTY: Thread = Thread {
    state: State::Free,
    priority: 0,
    base: 0,
    slot: 0,
    next: NONE,
    prev: NONE,
    sleep_next: NONE,
    sleeping: false,
    deadline: 0,
    quantum: 0,
    waiting_on: NONE,
    signalled: false,
};

/// Wait objects: CE's events (manual or auto reset) and mutexes share one
/// table and one waiter-queue mechanism.
pub const MAX_EVENTS: usize = 64;

/// How far priority inheritance follows a chain of owners waiting on other
/// mutexes (deeper chains are a design smell; this bounds the work).
const MAX_CHAIN: usize = 16;

#[derive(Clone, Copy)]
struct Event {
    used: bool,
    set: bool,
    manual: bool,
    /// A mutex: `owner` holds it `count` times (CE mutexes are recursive).
    mutex: bool,
    owner: u8,
    count: u16,
    /// Waiters, linked through the threads' `next`/`prev` (FIFO).
    head: u8,
    tail: u8,
}

const NO_EVENT: Event = Event {
    used: false,
    set: false,
    manual: false,
    mutex: false,
    owner: NONE,
    count: 0,
    head: NONE,
    tail: NONE,
};

/// An event's index.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub struct Eid(pub u8);

/// Why a wait ended.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum WaitResult {
    /// The event was signalled.
    Signalled,
    /// The deadline passed first.
    TimedOut,
}

/// Why an operation was refused.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum Error {
    /// The thread or event table is full.
    Full,
    /// No such thread or event (or not in the right state).
    Invalid,
}

/// Is `a` at or after `b` on the wrapping 32-bit timer?
fn reached(a: u32, b: u32) -> bool {
    a.wrapping_sub(b) < u32::MAX / 2
}

/// The scheduler state.
pub struct Scheduler {
    /// Slot 0 is the sentinel; threads are 1..=MAX_THREADS.
    threads: [Thread; MAX_THREADS + 1],
    /// Ready map: word `p / 32`, bit `31 - p % 32` set when priority `p` has
    /// ready threads (so `CLZ` finds the most urgent).
    map: [u32; 8],
    /// Bit `7 - w` set when word `w` of the map is non-zero.
    summary: u8,
    /// Per priority: FIFO of ready threads.
    head: [u8; 256],
    tail: [u8; 256],
    /// Sleepers, sorted by deadline.
    sleepers: u8,
    /// Slot 0 is the sentinel; events are 1..=MAX_EVENTS.
    events: [Event; MAX_EVENTS + 1],
    /// Bit `k` set: event `k + 1` is in use.
    events_used: u64,
    running: u8,
    /// When the running thread's quantum ends.
    quantum_end: u32,
    /// Quantum in ticks; 0 means [`DEFAULT_QUANTUM`] (keeps `new` all-zero).
    quantum_setting: u32,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl Scheduler {
    /// An empty scheduler (no threads; the caller creates the idle thread).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            threads: [EMPTY; MAX_THREADS + 1],
            map: [0; 8],
            summary: 0,
            head: [NONE; 256],
            tail: [NONE; 256],
            sleepers: NONE,
            events: [NO_EVENT; MAX_EVENTS + 1],
            events_used: 0,
            running: NONE,
            quantum_end: 0,
            quantum_setting: 0,
        }
    }

    fn quantum(&self) -> u32 {
        if self.quantum_setting == 0 {
            DEFAULT_QUANTUM
        } else {
            self.quantum_setting
        }
    }

    /// Sets the quantum for threads that start running from now on.
    pub fn set_quantum(&mut self, ticks: u32) {
        self.quantum_setting = ticks.max(1);
    }

    /// Creates a ready thread in process `slot` at `priority`.
    #[inline(never)]
    pub fn create(&mut self, slot: u8, priority: u8) -> Result<Tid, Error> {
        let i = 1 + self.threads[1..]
            .iter()
            .position(|t| t.state == State::Free)
            .ok_or(Error::Full)?;
        let quantum = self.quantum();
        self.threads[i] = Thread {
            priority,
            base: priority,
            slot,
            quantum,
            ..EMPTY
        };
        self.make_ready(i as u8);
        Ok(Tid(i as u8))
    }

    /// Removes a thread (it exited). If it was running, nothing runs until
    /// the next [`schedule`](Self::schedule).
    #[inline(never)]
    pub fn exit(&mut self, t: Tid) -> Result<(), Error> {
        let i = self.valid(t)?;
        match self.threads[usize::from(i)].state {
            State::Ready => self.unlink_ready(i),
            State::Blocked => self.cancel_wait(i),
            State::Running => self.running = NONE,
            State::Free => {}
        }
        // Mutexes it still holds pass to their next waiter (CE: abandoned).
        let mut used = self.events_used;
        while used != 0 {
            let m = used.trailing_zeros() as usize + 1;
            used &= used - 1;
            if self.events[m].mutex && self.events[m].owner == i {
                self.events[m].count = 0;
                self.events[m].owner = NONE;
                self.hand_over(m as u8);
            }
        }
        self.threads[usize::from(i)] = EMPTY;
        Ok(())
    }

    fn valid(&self, t: Tid) -> Result<u8, Error> {
        match self.threads.get(usize::from(t.0)) {
            Some(th) if t.0 != NONE && th.state != State::Free => Ok(t.0),
            _ => Err(Error::Invalid),
        }
    }

    /// The thread on the CPU.
    #[must_use]
    pub fn running(&self) -> Option<Tid> {
        (self.running != NONE).then_some(Tid(self.running))
    }

    /// A thread's state.
    #[must_use]
    pub fn state(&self, t: Tid) -> State {
        self.threads
            .get(usize::from(t.0))
            .map_or(State::Free, |th| th.state)
    }

    /// A thread's process slot.
    #[must_use]
    pub fn slot(&self, t: Tid) -> u8 {
        self.threads[usize::from(t.0)].slot
    }

    /// A thread's priority.
    #[must_use]
    pub fn priority(&self, t: Tid) -> u8 {
        self.threads[usize::from(t.0)].priority
    }

    // ---- the ready queues -------------------------------------------------

    fn make_ready(&mut self, i: u8) {
        let p = usize::from(self.threads[usize::from(i)].priority);
        let th = &mut self.threads[usize::from(i)];
        th.state = State::Ready;
        th.next = NONE;
        th.prev = self.tail[p];
        if self.tail[p] == NONE {
            self.head[p] = i;
        } else {
            self.threads[usize::from(self.tail[p])].next = i;
        }
        self.tail[p] = i;
        self.map[p / 32] |= 1 << (31 - p % 32);
        self.summary |= 1 << (7 - p / 32);
    }

    fn make_ready_front(&mut self, i: u8) {
        let p = usize::from(self.threads[usize::from(i)].priority);
        let th = &mut self.threads[usize::from(i)];
        th.state = State::Ready;
        th.prev = NONE;
        th.next = self.head[p];
        if self.head[p] == NONE {
            self.tail[p] = i;
        } else {
            self.threads[usize::from(self.head[p])].prev = i;
        }
        self.head[p] = i;
        self.map[p / 32] |= 1 << (31 - p % 32);
        self.summary |= 1 << (7 - p / 32);
    }

    fn unlink_ready(&mut self, i: u8) {
        let p = usize::from(self.threads[usize::from(i)].priority);
        let (n, pr) = {
            let th = &self.threads[usize::from(i)];
            (th.next, th.prev)
        };
        if pr == NONE {
            self.head[p] = n;
        } else {
            self.threads[usize::from(pr)].next = n;
        }
        if n == NONE {
            self.tail[p] = pr;
        } else {
            self.threads[usize::from(n)].prev = pr;
        }
        if self.head[p] == NONE {
            self.map[p / 32] &= !(1 << (31 - p % 32));
            if self.map[p / 32] == 0 {
                self.summary &= !(1 << (7 - p / 32));
            }
        }
    }

    /// The most urgent ready priority: two `CLZ`s.
    fn best_ready(&self) -> Option<u8> {
        if self.summary == 0 {
            return None;
        }
        let w = self.summary.leading_zeros() as usize;
        let b = self.map[w].leading_zeros() as usize;
        Some((w * 32 + b) as u8)
    }

    /// Decides who runs now (at `now`, OS-timer ticks): the running thread
    /// keeps the CPU unless a more urgent thread is ready, or its quantum
    /// is over and a thread of the same priority is waiting. Returns the
    /// thread to run (`None`: nothing is ready; idle).
    #[inline(never)]
    pub fn schedule(&mut self, now: u32) -> Option<Tid> {
        self.expire_sleepers(now);
        let cur = self.running;
        let best = self.best_ready();
        if cur != NONE {
            let cp = self.threads[usize::from(cur)].priority;
            let expired = reached(now, self.quantum_end);
            let keep = match best {
                None => true,
                Some(bp) => bp > cp || (bp == cp && !expired),
            };
            if keep {
                if expired {
                    self.quantum_end = now.wrapping_add(self.quantum());
                }
                return Some(Tid(cur));
            }
            // Quantum over: back of its queue with a fresh quantum. Preempted
            // by a more urgent thread: *front* of its queue with what was
            // left, so it resumes before its peers (CE/NT semantics).
            if expired {
                self.threads[usize::from(cur)].quantum = self.quantum();
                self.make_ready(cur);
            } else {
                self.threads[usize::from(cur)].quantum = self.quantum_end.wrapping_sub(now);
                self.make_ready_front(cur);
            }
        }
        let p = usize::from(self.best_ready()?);
        let next = self.head[p];
        self.unlink_ready(next);
        let fresh = self.quantum();
        let th = &mut self.threads[usize::from(next)];
        th.state = State::Running;
        self.quantum_end = now.wrapping_add(th.quantum);
        th.quantum = fresh;
        self.running = next;
        Some(Tid(next))
    }

    /// Gives up the rest of the quantum: the next [`schedule`](Self::schedule)
    /// runs another thread of the same priority, if any.
    pub fn yield_now(&mut self, now: u32) {
        self.quantum_end = now;
    }

    /// When the timer must next fire: the running thread's quantum end (only
    /// if another thread of its priority could take over) or the earliest
    /// sleeper. `None`: nothing time-based is pending; sleep until an
    /// interrupt.
    #[must_use]
    pub fn next_deadline(&self) -> Option<u32> {
        let quantum = (self.running != NONE)
            .then(|| {
                let p = usize::from(self.threads[usize::from(self.running)].priority);
                (self.head[p] != NONE).then_some(self.quantum_end)
            })
            .flatten();
        let sleeper =
            (self.sleepers != NONE).then(|| self.threads[usize::from(self.sleepers)].deadline);
        match (quantum, sleeper) {
            (Some(a), Some(b)) => Some(if reached(b, a) { a } else { b }),
            (a, b) => a.or(b),
        }
    }

    /// Changes a thread's priority (re-queued if ready). An inherited
    /// priority stays in force until the mutex is released.
    #[inline(never)]
    pub fn set_priority(&mut self, t: Tid, priority: u8) -> Result<(), Error> {
        let i = self.valid(t)?;
        self.threads[usize::from(i)].base = priority;
        self.update_priority(i);
        Ok(())
    }

    /// A thread's own priority, before inheritance.
    #[must_use]
    pub fn base_priority(&self, t: Tid) -> u8 {
        self.threads[usize::from(t.0)].base
    }

    // ---- priority inheritance ---------------------------------------------

    /// `i`'s priority as it should be: its own, or that of the most urgent
    /// thread waiting for a mutex it owns.
    #[inline(never)]
    fn inherited(&self, i: u8) -> u8 {
        let mut p = self.threads[usize::from(i)].base;
        let mut used = self.events_used;
        while used != 0 {
            let m = used.trailing_zeros() as usize + 1;
            used &= used - 1;
            let ev = &self.events[m];
            if ev.mutex && ev.owner == i {
                let mut w = ev.head;
                while w != NONE {
                    p = p.min(self.threads[usize::from(w)].priority);
                    w = self.threads[usize::from(w)].next;
                }
            }
        }
        p
    }

    fn set_effective(&mut self, i: u8, p: u8) {
        let ready = self.threads[usize::from(i)].state == State::Ready;
        if ready {
            self.unlink_ready(i);
        }
        self.threads[usize::from(i)].priority = p;
        if ready {
            self.make_ready(i);
        }
    }

    /// Recomputes `i`'s priority and, if it changed while `i` itself waits
    /// for a mutex, that mutex's owner's (and so on up the chain).
    #[inline(never)]
    fn update_priority(&mut self, mut i: u8) {
        for _ in 0..MAX_CHAIN {
            let p = self.inherited(i);
            if p == self.threads[usize::from(i)].priority {
                return;
            }
            self.set_effective(i, p);
            let w = usize::from(self.threads[usize::from(i)].waiting_on);
            if w == usize::from(NONE) || !self.events[w].mutex || self.events[w].owner == NONE {
                return;
            }
            i = self.events[w].owner;
        }
    }

    /// The most urgent waiter of wait object `m` (the first of equals).
    fn best_waiter(&self, m: u8) -> u8 {
        let mut best = NONE;
        let mut w = self.events[usize::from(m)].head;
        while w != NONE {
            if best == NONE
                || self.threads[usize::from(w)].priority < self.threads[usize::from(best)].priority
            {
                best = w;
            }
            w = self.threads[usize::from(w)].next;
        }
        best
    }

    /// A free mutex goes to its most urgent waiter, if any.
    #[inline(never)]
    fn hand_over(&mut self, m: u8) {
        let w = self.best_waiter(m);
        if w == NONE {
            return;
        }
        self.unlink_waiter(w);
        self.remove_sleeper(w);
        self.threads[usize::from(w)].signalled = true;
        let ev = &mut self.events[usize::from(m)];
        ev.owner = w;
        ev.count = 1;
        self.make_ready(w);
        // Its remaining waiters now push on the new owner.
        self.update_priority(w);
    }

    // ---- sleeping and waiting --------------------------------------------

    fn insert_sleeper(&mut self, i: u8, deadline: u32) {
        let th = &mut self.threads[usize::from(i)];
        th.deadline = deadline;
        th.sleeping = true;
        let mut prev = NONE;
        let mut cur = self.sleepers;
        while cur != NONE && reached(deadline, self.threads[usize::from(cur)].deadline) {
            prev = cur;
            cur = self.threads[usize::from(cur)].sleep_next;
        }
        self.threads[usize::from(i)].sleep_next = cur;
        if prev == NONE {
            self.sleepers = i;
        } else {
            self.threads[usize::from(prev)].sleep_next = i;
        }
    }

    fn remove_sleeper(&mut self, i: u8) {
        if !self.threads[usize::from(i)].sleeping {
            return;
        }
        let mut prev = NONE;
        let mut cur = self.sleepers;
        while cur != NONE && cur != i {
            prev = cur;
            cur = self.threads[usize::from(cur)].sleep_next;
        }
        if cur == i {
            let n = self.threads[usize::from(i)].sleep_next;
            if prev == NONE {
                self.sleepers = n;
            } else {
                self.threads[usize::from(prev)].sleep_next = n;
            }
        }
        self.threads[usize::from(i)].sleeping = false;
    }

    fn expire_sleepers(&mut self, now: u32) {
        while self.sleepers != NONE {
            let i = self.sleepers;
            if !reached(now, self.threads[usize::from(i)].deadline) {
                break;
            }
            self.sleepers = self.threads[usize::from(i)].sleep_next;
            self.threads[usize::from(i)].sleeping = false;
            // A timed wait that timed out leaves its event's queue (and stops
            // lending its priority to a mutex owner).
            if self.threads[usize::from(i)].waiting_on != NONE {
                let owner = self.mutex_owner(self.threads[usize::from(i)].waiting_on);
                self.unlink_waiter(i);
                if owner != NONE {
                    self.update_priority(owner);
                }
            }
            self.threads[usize::from(i)].signalled = false;
            self.make_ready(i);
        }
    }

    /// Takes the running thread off the CPU until `deadline`.
    #[inline(never)]
    pub fn sleep_until(&mut self, deadline: u32) -> Result<(), Error> {
        let i = self.block_running()?;
        self.insert_sleeper(i, deadline);
        Ok(())
    }

    fn block_running(&mut self) -> Result<u8, Error> {
        let i = self.running;
        if i == NONE {
            return Err(Error::Invalid);
        }
        let fresh = self.quantum();
        let th = &mut self.threads[usize::from(i)];
        th.state = State::Blocked;
        th.quantum = fresh; // a full quantum when it next runs
        self.running = NONE;
        Ok(i)
    }

    /// Creates an event (manual reset: stays set until reset; auto reset:
    /// releases one waiter and clears).
    #[inline(never)]
    pub fn create_event(&mut self, manual: bool, set: bool) -> Result<Eid, Error> {
        let i = self.free_event().ok_or(Error::Full)?;
        // Field by field: a whole-struct store got unrolled into over 1 KiB
        // (as did an iterator search before it).
        let ev = &mut self.events[i];
        ev.used = true;
        ev.set = set;
        ev.manual = manual;
        ev.mutex = false;
        ev.owner = NONE;
        ev.count = 0;
        ev.head = NONE;
        ev.tail = NONE;
        Ok(Eid(i as u8))
    }

    /// A free event slot in O(1): bit `k` of `events_used` is event `k + 1`
    /// (a 64-step search loop got unrolled into 1 KiB).
    fn free_event(&mut self) -> Option<usize> {
        let k = (!self.events_used).trailing_zeros() as usize;
        if k >= MAX_EVENTS {
            return None;
        }
        self.events_used |= 1 << k;
        Some(k + 1)
    }

    fn event(&mut self, e: Eid) -> Result<&mut Event, Error> {
        match self.events.get_mut(usize::from(e.0)) {
            Some(ev) if e.0 != NONE && ev.used => Ok(ev),
            _ => Err(Error::Invalid),
        }
    }

    /// The owner of wait object `m` if it's an owned mutex, else `NONE`.
    fn mutex_owner(&self, m: u8) -> u8 {
        let ev = &self.events[usize::from(m)];
        if ev.mutex { ev.owner } else { NONE }
    }

    /// Creates a mutex, owned by the running thread if `owned`.
    #[inline(never)]
    pub fn create_mutex(&mut self, owned: bool) -> Result<Eid, Error> {
        let e = self.create_event(false, false)?;
        let owner = if owned { self.running } else { NONE };
        let ev = &mut self.events[usize::from(e.0)];
        ev.mutex = true;
        ev.owner = owner;
        ev.count = u16::from(owner != NONE);
        Ok(e)
    }

    /// The running thread releases mutex `e` once (it must own it). When the
    /// last hold is released the mutex passes to its most urgent waiter, and
    /// any priority the releaser inherited through it is given back. Returns
    /// whether a waiter became ready (the caller should reschedule).
    #[inline(never)]
    pub fn release_mutex(&mut self, e: Eid) -> Result<bool, Error> {
        let me = self.running;
        let ev = self.event(e)?;
        if !ev.mutex || me == NONE || ev.owner != me {
            return Err(Error::Invalid);
        }
        ev.count -= 1;
        if ev.count > 0 {
            return Ok(false);
        }
        ev.owner = NONE;
        let woke = self.events[usize::from(e.0)].head != NONE;
        self.hand_over(e.0);
        self.update_priority(me);
        Ok(woke)
    }

    /// The thread that owns mutex `e` (`None` if free or not a mutex).
    #[must_use]
    pub fn mutex_owner_of(&self, e: Eid) -> Option<Tid> {
        let o = self.mutex_owner(e.0);
        (o != NONE).then_some(Tid(o))
    }

    /// The running thread waits for `e`, until `deadline` if given. Returns
    /// `Some(Signalled)` at once if the event is already set (consuming it if
    /// auto reset), `None` if the thread now blocks (the caller schedules;
    /// [`wait_result`](Self::wait_result) says how it ended).
    #[inline(never)]
    pub fn wait(&mut self, e: Eid, deadline: Option<u32>) -> Result<Option<WaitResult>, Error> {
        let me = self.running;
        let ev = self.event(e)?;
        if ev.mutex {
            // A mutex is "set" when free, or already ours (recursion).
            if ev.owner == NONE && me != NONE {
                ev.owner = me;
                ev.count = 1;
                return Ok(Some(WaitResult::Signalled));
            }
            if ev.owner == me && me != NONE {
                ev.count = ev.count.checked_add(1).ok_or(Error::Full)?;
                return Ok(Some(WaitResult::Signalled));
            }
        } else if ev.set {
            if !ev.manual {
                ev.set = false;
            }
            return Ok(Some(WaitResult::Signalled));
        }
        let i = self.block_running()?;
        let th = &mut self.threads[usize::from(i)];
        th.waiting_on = e.0;
        th.signalled = false;
        th.next = NONE;
        // Append to the event's FIFO.
        let tail = self.events[usize::from(e.0)].tail;
        self.threads[usize::from(i)].prev = tail;
        if tail == NONE {
            self.events[usize::from(e.0)].head = i;
        } else {
            self.threads[usize::from(tail)].next = i;
        }
        self.events[usize::from(e.0)].tail = i;
        if let Some(d) = deadline {
            self.insert_sleeper(i, d);
        }
        // Priority inheritance: the owner runs at least as urgently as its
        // most urgent waiter, so a middle-priority thread can't starve it.
        let owner = self.mutex_owner(e.0);
        if owner != NONE {
            self.update_priority(owner);
        }
        Ok(None)
    }

    /// How the last wait of `t` ended (after it runs again).
    #[must_use]
    pub fn wait_result(&self, t: Tid) -> WaitResult {
        if self.threads[usize::from(t.0)].signalled {
            WaitResult::Signalled
        } else {
            WaitResult::TimedOut
        }
    }

    fn unlink_waiter(&mut self, i: u8) {
        let e = usize::from(self.threads[usize::from(i)].waiting_on);
        let (n, p) = {
            let th = &self.threads[usize::from(i)];
            (th.next, th.prev)
        };
        if p == NONE {
            self.events[e].head = n;
        } else {
            self.threads[usize::from(p)].next = n;
        }
        if n == NONE {
            self.events[e].tail = p;
        } else {
            self.threads[usize::from(n)].prev = p;
        }
        self.threads[usize::from(i)].waiting_on = NONE;
    }

    fn cancel_wait(&mut self, i: u8) {
        let w = self.threads[usize::from(i)].waiting_on;
        if w != NONE {
            let owner = self.mutex_owner(w);
            self.unlink_waiter(i);
            if owner != NONE {
                self.update_priority(owner);
            }
        }
        self.remove_sleeper(i);
    }

    fn release(&mut self, i: u8) {
        self.unlink_waiter(i);
        self.remove_sleeper(i);
        self.threads[usize::from(i)].signalled = true;
        self.make_ready(i);
    }

    /// Sets `e`: a manual-reset event releases every waiter and stays set; an
    /// auto-reset event releases one waiter (or stays set until one comes).
    /// Returns how many threads became ready. Safe from interrupt context's
    /// deferred work (ADR-0007: interrupt lines signal events).
    #[inline(never)]
    pub fn set_event(&mut self, e: Eid) -> Result<usize, Error> {
        let ev = self.event(e)?;
        if ev.mutex {
            return Err(Error::Invalid);
        }
        let manual = ev.manual;
        let mut woken = 0;
        loop {
            let head = self.events[usize::from(e.0)].head;
            if head == NONE {
                break;
            }
            self.release(head);
            woken += 1;
            if !manual {
                return Ok(woken);
            }
        }
        self.events[usize::from(e.0)].set = true;
        Ok(woken)
    }

    /// Clears `e`.
    #[inline(never)]
    pub fn reset_event(&mut self, e: Eid) -> Result<(), Error> {
        let ev = self.event(e)?;
        if ev.mutex {
            return Err(Error::Invalid);
        }
        ev.set = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
