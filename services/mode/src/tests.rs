use super::*;

/// A stand-in host: holds the token it was given, acts for a call only as
/// the grants allow, and resets its device on revoke.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
struct Host {
    token: Option<Grant>,
    resets: u32,
    /// Device state a game leaves half-programmed (cleared by a reset).
    dirty: bool,
}

/// Stand-ins for the services, the kernel and the hosts: every step
/// recorded in order; failures on demand.
#[derive(Default)]
struct Fake {
    log: Vec<String>,
    hosts: [Host; 3],
    fail_suspend: bool,
    fail_freeze: bool,
    fail_start: bool,
    fail_thaw: u32,
    asking: bool,
    next_slot: u8,
}

impl Steps for Fake {
    fn suspend(&mut self) -> Result<(), StepFailed> {
        self.log.push("suspend".into());
        if self.fail_suspend {
            Err(StepFailed)
        } else {
            Ok(())
        }
    }
    fn freeze(&mut self, tier: Tier) -> Result<(), StepFailed> {
        self.log.push(format!("freeze {tier:?}"));
        if self.fail_freeze {
            Err(StepFailed)
        } else {
            Ok(())
        }
    }
    fn grant(&mut self, device: Device, grant: Grant) {
        self.log.push(format!("grant {device:?}"));
        self.hosts[device as usize].token = Some(grant);
    }
    fn start(&mut self) -> Result<u8, StepFailed> {
        self.log.push("start".into());
        if self.fail_start {
            Err(StepFailed)
        } else {
            self.next_slot += 1;
            Ok(10 + self.next_slot)
        }
    }
    fn run(&mut self, slot: u8) {
        self.log.push(format!("run {slot}"));
    }
    fn revoke(&mut self, device: Device) {
        self.log.push(format!("revoke {device:?}"));
        let h = &mut self.hosts[device as usize];
        h.token = None;
        h.resets += 1;
        h.dirty = false;
    }
    fn end(&mut self, slot: u8) {
        self.log.push(format!("end {slot}"));
    }
    fn last_frame(&mut self) {
        self.log.push("last frame".into());
    }
    fn thaw(&mut self, tier: Tier) -> Result<(), StepFailed> {
        self.log.push(format!("thaw {tier:?}"));
        if self.fail_thaw > 0 {
            self.fail_thaw -= 1;
            Err(StepFailed)
        } else {
            Ok(())
        }
    }
    fn resume(&mut self) {
        self.log.push("resume".into());
    }
    fn ask(&mut self, show: bool) {
        assert_ne!(self.asking, show, "asked twice or hidden twice");
        self.asking = show;
        self.log.push(format!("ask {show}"));
    }
}

impl Fake {
    fn take(&mut self) -> Vec<String> {
        std::mem::take(&mut self.log)
    }
}

const WAY_OUT: [&str; 6] = [
    "revoke Audio",
    "revoke Input",
    "revoke Display",
    "end 11",
    "last frame",
    "thaw Fast",
];

fn way_out() -> Vec<String> {
    let mut v: Vec<String> = WAY_OUT.iter().map(|s| (*s).to_string()).collect();
    v.push("resume".into());
    v
}

fn in_game() -> (Manager, Fake) {
    let mut m = Manager::new();
    let mut f = Fake::default();
    assert_eq!(m.enter(&mut f, Tier::Fast, 1000), Ok(11));
    f.take();
    (m, f)
}

#[test]
fn the_way_in_runs_its_steps_in_order_and_grants_the_game_alone() {
    let mut m = Manager::new();
    let mut f = Fake::default();
    assert_eq!(m.enter(&mut f, Tier::Full, 1000), Ok(11));
    assert_eq!(
        f.take(),
        [
            "suspend",
            "freeze Full",
            "start",
            "grant Display",
            "grant Input",
            "grant Audio",
            "run 11"
        ]
    );
    let g = m.grants().holder(Device::Input).unwrap();
    assert_eq!(g.slot, 11);
    for d in DEVICES {
        assert_eq!(f.hosts[d as usize].token, Some(g));
        assert!(m.grants().allows(d, 11, Some(g)));
        assert!(!m.grants().allows(d, 11, None), "no token");
        assert!(!m.grants().allows(d, 3, Some(g)), "another slot");
        assert!(!m.grants().allows(d, 3, None), "the desktop's processes");
    }
    assert_eq!(m.enter(&mut f, Tier::Fast, 1000), Err(Failed::Busy));
    assert!(f.take().is_empty(), "a second entry touches nothing");
}

#[test]
fn every_ending_takes_the_one_way_out_and_resets_every_device() {
    type End = fn(&mut Manager, &mut Fake);
    let ends: [(Exit, End); 4] = [
        (Exit::Quit, |m, f| m.ended(f, 11, false)),
        (Exit::Crash, |m, f| m.ended(f, 11, true)),
        (Exit::ForceClose, |m, f| m.force_back(f)),
        (Exit::Hang, |m, f| {
            m.tick(f, 1000 + DEFAULT_DEADLINE_MS);
            f.take();
            m.answer(f, false, 7000);
        }),
    ];
    for (exit, end) in ends {
        let (mut m, mut f) = in_game();
        let old = m.grants().holder(Device::Display).unwrap();
        for h in &mut f.hosts {
            h.dirty = true; // the game left its devices mid-command
        }
        end(&mut m, &mut f);
        let mut log = f.take();
        log.retain(|l| !l.starts_with("ask"));
        assert_eq!(log, way_out(), "{exit:?}");
        assert_eq!(m.mode(), Mode::Full);
        assert_eq!(m.last_exit(), Some(exit));
        for d in DEVICES {
            let h = f.hosts[d as usize];
            assert_eq!(
                (h.token, h.resets, h.dirty),
                (None, 1, false),
                "{exit:?} {d:?}"
            );
            // The old token is dead; the desktop shares the device again.
            assert!(!m.grants().allows(d, 11, Some(old)));
            assert!(m.grants().allows(d, 3, None));
        }
        // The next game, even in the same slot, gets a new token.
        f.next_slot = 0;
        m.enter(&mut f, Tier::Fast, 9000).unwrap();
        let new = m.grants().holder(Device::Audio).unwrap();
        assert_eq!(new.slot, old.slot);
        assert_ne!(new, old);
        assert!(!m.grants().allows(Device::Audio, 11, Some(old)));
    }
}

#[test]
fn the_watchdog_asks_once_and_a_frame_or_a_wait_starts_it_again() {
    let (mut m, mut f) = in_game();
    // Frames keep it quiet, however long the game runs.
    for t in (1000..60_000).step_by(1000) {
        m.alive(&mut f, 11, t);
        m.tick(&mut f, t + 999);
    }
    assert!(f.take().is_empty());
    // A frame from another slot doesn't count.
    m.alive(&mut f, 3, 64_000);
    m.tick(&mut f, 59_000 + DEFAULT_DEADLINE_MS - 1);
    assert!(f.take().is_empty(), "one ms short");
    m.tick(&mut f, 59_000 + DEFAULT_DEADLINE_MS);
    assert_eq!(f.take(), ["ask true"]);
    m.tick(&mut f, 80_000);
    m.tick(&mut f, 90_000);
    assert!(f.take().is_empty(), "asked once");
    // It recovers: a frame takes the question away.
    m.alive(&mut f, 11, 90_500);
    assert_eq!(f.take(), ["ask false"]);
    m.tick(&mut f, 90_500 + DEFAULT_DEADLINE_MS);
    assert_eq!(f.take(), ["ask true"]);
    // Wait: the clock starts again from the answer.
    m.answer(&mut f, true, 100_000);
    assert_eq!(f.take(), ["ask false"]);
    m.tick(&mut f, 100_000 + DEFAULT_DEADLINE_MS - 1);
    assert!(f.take().is_empty());
    // An answer with no question up does nothing.
    m.answer(&mut f, false, 100_100);
    assert!(f.take().is_empty());
    assert!(matches!(m.mode(), Mode::Game { .. }));
    // The way back while asking hides the question first.
    m.tick(&mut f, 100_000 + DEFAULT_DEADLINE_MS);
    f.take();
    m.force_back(&mut f);
    let log = f.take();
    assert_eq!(log[0], "ask false");
    assert_eq!(log[1..], way_out()[..]);
}

#[test]
fn a_long_load_sets_its_own_deadline_within_bounds_and_time_wraps() {
    let (mut m, mut f) = in_game();
    m.set_deadline(11, 20_000, 2000);
    m.tick(&mut f, 21_999);
    assert!(f.take().is_empty());
    m.tick(&mut f, 22_000);
    assert_eq!(f.take(), ["ask true"]);
    m.answer(&mut f, true, 22_000);
    f.take();
    for (asked, got) in [(10, MIN_DEADLINE_MS), (600_000, MAX_DEADLINE_MS)] {
        m.set_deadline(11, asked, 30_000);
        assert!(matches!(m.mode(), Mode::Game { deadline, .. } if deadline == got));
    }
    m.set_deadline(3, 10, 30_000);
    assert!(
        matches!(m.mode(), Mode::Game { deadline, .. } if deadline == MAX_DEADLINE_MS),
        "only the game sets it"
    );
    // The millisecond clock wraps (49 days): differences still hold.
    let mut m = Manager::new();
    let mut f = Fake::default();
    m.enter(&mut f, Tier::Fast, u32::MAX - 1000).unwrap();
    f.take();
    m.tick(&mut f, 3000); // 4001 ms on
    assert!(f.take().is_empty());
    m.tick(&mut f, DEFAULT_DEADLINE_MS - 1001);
    assert_eq!(f.take(), ["ask true"]);
}

#[test]
fn a_failed_way_in_undoes_what_it_did() {
    for (which, want, undo) in [
        (0, Failed::Suspend, vec!["suspend", "resume"]),
        (
            1,
            Failed::Freeze,
            vec!["suspend", "freeze Full", "thaw Fast", "resume"],
        ),
        (
            2,
            Failed::Start,
            vec!["suspend", "freeze Full", "start", "thaw Full", "resume"],
        ),
    ] {
        let mut m = Manager::new();
        let mut f = Fake {
            fail_suspend: which == 0,
            fail_freeze: which == 1,
            fail_start: which == 2,
            ..Fake::default()
        };
        assert_eq!(m.enter(&mut f, Tier::Full, 0), Err(want));
        assert_eq!(f.take(), undo);
        assert_eq!(m.mode(), Mode::Full);
        for d in DEVICES {
            assert_eq!(m.grants().holder(d), None, "nothing granted");
        }
    }
}

#[test]
fn a_thaw_that_fails_keeps_the_session_and_is_tried_again() {
    let (mut m, mut f) = in_game();
    f.fail_thaw = 4;
    m.ended(&mut f, 11, true);
    let mut want: Vec<String> = WAY_OUT.iter().map(|s| (*s).to_string()).collect();
    want.push("thaw Fast".into());
    assert_eq!(f.take(), want, "tried twice, no resume yet");
    assert_eq!(
        m.mode(),
        Mode::Returning {
            tier: Tier::Fast,
            exit: Exit::Crash
        }
    );
    // Events for a game that is gone do nothing.
    m.ended(&mut f, 11, false);
    m.force_back(&mut f);
    m.alive(&mut f, 11, 5);
    assert!(f.take().is_empty());
    assert_eq!(m.enter(&mut f, Tier::Fast, 5), Err(Failed::Busy));
    // Each tick tries again; the card reads on the fifth try.
    m.tick(&mut f, 10);
    assert_eq!(f.take(), ["thaw Fast", "thaw Fast"]);
    m.tick(&mut f, 20);
    assert_eq!(f.take(), ["thaw Fast", "resume"]);
    assert_eq!(m.mode(), Mode::Full);
    assert_eq!(m.last_exit(), Some(Exit::Crash));
    // In Full Mode nothing else acts.
    m.tick(&mut f, 30);
    m.force_back(&mut f);
    m.ended(&mut f, 11, true);
    assert!(f.take().is_empty());
}

#[test]
fn another_process_ending_is_not_the_game() {
    let (mut m, mut f) = in_game();
    m.ended(&mut f, 3, true);
    assert!(f.take().is_empty());
    assert!(matches!(m.mode(), Mode::Game { slot: 11, .. }));
}

#[test]
fn grants_alone() {
    let mut g = Grants::new();
    for d in DEVICES {
        assert!(g.allows(d, 5, None), "nothing out: shared");
        assert_eq!(g.holder(d), None);
    }
    assert_eq!(g.revoke(), [false; 3]);
    let a = g.grant(7);
    let b = g.grant(7);
    assert!(b.generation > a.generation);
    assert!(!g.allows(Device::Display, 7, Some(a)), "superseded");
    assert!(g.allows(Device::Display, 7, Some(b)));
    assert_eq!(g.revoke(), [true; 3]);
    // The generation never gives 0, even after it wraps.
    let mut w = Grants {
        held: [None; 3],
        generation: u32::MAX,
    };
    assert_eq!(w.grant(1).generation, 1);
}
