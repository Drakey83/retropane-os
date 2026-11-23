//! The mode manager's protocol (docs/design/game-mode-b.md, build step G6
//! in docs/design/game-mode-build.md): the switch into Game Mode and back,
//! the exclusivity grants the display, input and audio hosts honour, the
//! watchdog, and the one way out that a quit, a crash, a hang and a
//! force-close all take. Pure logic: the service runs it against the real
//! hosts (once K2 gives them), the tests against stand-ins.
//!
//! - **Grants carry a generation.** A grant is a token (the game's slot and
//!   a number that only grows); while one is out a host acts only for a
//!   call carrying the device's current token from that slot. Revoked,
//!   every token is dead at once, so a call made under it (one in flight
//!   across the revoke, or the next game reusing the slot with an old
//!   token) is refused; each host resets its device to a known state on
//!   revoke, so nothing is left half-programmed. The game's first thread
//!   is held until the grants are out.
//! - **The watchdog watches progress, not input.** The game must present a
//!   frame or call into the system within its deadline (5 s, set per game
//!   for long loads, 1 to 60 s); a miss shows the "Game not responding"
//!   question once (wait, or close); a frame while it is up takes it away.
//! - **One way out.** Every ending runs the same steps in the same order:
//!   grants revoked (devices reset), the game's slot ended, the last Full
//!   Mode frame shown, the session thawed, the services resumed. A thaw
//!   that fails (the card out, a run that won't read) keeps the record and
//!   is tried again on the next tick: the session is never dropped.
//! - **Entry unwinds.** A step of the way in that fails undoes the ones
//!   before it, in reverse, and the session stays in Full Mode.
//!
//! Rule 0: Windows Mobile's GAPI (`GXOpenDisplay`, `GXOpenInput`) hands a
//! game the raw frame buffer and the hardware keys with no revoke, so a
//! crashed game leaves the display and keys as it left them; Android's ANR
//! fires only when input waits 5 s unanswered (a game spinning with no
//! input pending is never caught) and SurfaceFlinger, not the app, owns
//! the screen; consoles reserve a system button. Ours: generation tokens
//! (a stale caller is refused without the host knowing about modes), a
//! watchdog on frames and system calls (a hang is caught whether or not
//! the person is pressing anything), a reserved way back, and one exit
//! path, so a crash is tested by the same steps as a quit.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

/// The watchdog's deadline unless the game sets its own: 5 s.
pub const DEFAULT_DEADLINE_MS: u32 = 5_000;
/// The shortest deadline a game may set.
pub const MIN_DEADLINE_MS: u32 = 1_000;
/// The longest (a long load).
pub const MAX_DEADLINE_MS: u32 = 60_000;
/// Times the way out tries the thaw before it waits for the next tick.
pub const THAW_TRIES: u32 = 2;

/// A device the game takes for itself.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum Device {
    /// The 2700G's composition: only the holder's surfaces reach the panel.
    Display = 0,
    /// Keys and touch: all go to the holder but the reserved way back.
    Input = 1,
    /// The mixer: one client.
    Audio = 2,
}

/// The devices, in the order they are granted (revoked in reverse).
pub const DEVICES: [Device; 3] = [Device::Display, Device::Input, Device::Audio];

/// A grant: the slot it was given to and its generation (never 0).
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub struct Grant {
    /// The game's process slot.
    pub slot: u8,
    /// Grows with every grant; a revoked one is never given again.
    pub generation: u32,
}

/// Who holds each device.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(test, derive(Debug))]
pub struct Grants {
    held: [Option<Grant>; 3],
    generation: u32,
}

impl Grants {
    /// Nobody holds anything.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            held: [None; 3],
            generation: 0,
        }
    }

    /// A new token for `slot` on every device; any older one is dead.
    pub fn grant(&mut self, slot: u8) -> Grant {
        self.generation = self.generation.wrapping_add(1).max(1);
        let g = Grant {
            slot,
            generation: self.generation,
        };
        self.held = [Some(g); 3];
        g
    }

    /// Whether a call from `slot` with `token` may use `device`: the host's
    /// check (with no grant out, everyone shares the device as in Full
    /// Mode, so `None` answers yes to a `None` token).
    #[must_use]
    pub fn allows(&self, device: Device, slot: u8, token: Option<Grant>) -> bool {
        match (self.held[device as usize], token) {
            (None, None) => true,
            (Some(h), Some(t)) => h == t && h.slot == slot,
            _ => false,
        }
    }

    /// The device's holder.
    #[must_use]
    pub fn holder(&self, device: Device) -> Option<Grant> {
        self.held[device as usize]
    }

    /// Ends every grant; which devices were held (to reset).
    pub fn revoke(&mut self) -> [bool; 3] {
        let was = self.held.map(|h| h.is_some());
        self.held = [None; 3];
        was
    }
}

/// Where the frozen session goes.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum Tier {
    /// Compressed in RAM.
    Fast,
    /// Compressed, then written to the card.
    Full,
}

/// How a game ended.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum Exit {
    /// It quit.
    Quit,
    /// It faulted (the kernel ended it).
    Crash,
    /// It missed its deadline and the person chose to close it.
    Hang,
    /// The reserved way back: no question asked.
    ForceClose,
}

/// A step of the way in that failed.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum Failed {
    /// The services wouldn't suspend.
    Suspend,
    /// The session wouldn't freeze (full tier: the card).
    Freeze,
    /// The game wouldn't start.
    Start,
    /// Not in Full Mode.
    Busy,
}

/// A step the service ran didn't succeed (its own log says why).
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub struct StepFailed;

/// The work the mode manager drives: the services, the kernel's freeze,
/// the hosts and the game. The service implements it on IPC calls; tests
/// on stand-ins that record the order.
pub trait Steps {
    /// Suspends every Full Mode service (a slow one is frozen anyway) and
    /// freezes every app's threads.
    fn suspend(&mut self) -> Result<(), StepFailed>;
    /// Freezes the session's memory into `tier`.
    fn freeze(&mut self, tier: Tier) -> Result<(), StepFailed>;
    /// Gives `device` to the game: the host now acts only for `grant`.
    fn grant(&mut self, device: Device, grant: Grant);
    /// Creates the game's process with its first thread held: its slot.
    fn start(&mut self) -> Result<u8, StepFailed>;
    /// Lets the game's first thread run (after the grants).
    fn run(&mut self, slot: u8);
    /// Ends `device`'s grant: the host resets the device to a known state.
    fn revoke(&mut self, device: Device);
    /// Ends the game's process if it still runs and frees its slot.
    fn end(&mut self, slot: u8);
    /// Shows the last Full Mode frame (the 2700G's copy or the card's).
    fn last_frame(&mut self);
    /// Thaws the session from `tier` (full: read back from the card).
    fn thaw(&mut self, tier: Tier) -> Result<(), StepFailed>;
    /// Resumes every service from its record (one that won't is restarted
    /// by the supervisor).
    fn resume(&mut self);
    /// Shows or hides the "Game not responding" question.
    fn ask(&mut self, show: bool);
}

/// Where the mode manager is.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum Mode {
    /// The desktop.
    Full,
    /// A game runs.
    Game {
        /// Its slot.
        slot: u8,
        /// Its tier.
        tier: Tier,
        /// When it last presented a frame or called the system (ms).
        last: u32,
        /// Its deadline (ms).
        deadline: u32,
        /// Whether the question is up.
        asking: bool,
    },
    /// The game is gone; the session's thaw failed and is tried again each
    /// tick (nothing is lost: the record and the card keep it).
    Returning {
        /// The tier to thaw from.
        tier: Tier,
        /// How the game ended.
        exit: Exit,
    },
}

/// The mode manager.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub struct Manager {
    mode: Mode,
    grants: Grants,
    last_exit: Option<Exit>,
}

impl Default for Manager {
    fn default() -> Self {
        Self::new()
    }
}

impl Manager {
    /// In Full Mode, nothing granted.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            mode: Mode::Full,
            grants: Grants::new(),
            last_exit: None,
        }
    }

    /// Where it is.
    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The grants the hosts check.
    #[must_use]
    pub fn grants(&self) -> &Grants {
        &self.grants
    }

    /// How the last game ended.
    #[must_use]
    pub fn last_exit(&self) -> Option<Exit> {
        self.last_exit
    }

    /// The way in: services suspended and apps frozen, the session frozen
    /// into `tier`, the devices granted, the game started. A step that
    /// fails undoes the ones before it and leaves Full Mode as it was.
    pub fn enter(&mut self, steps: &mut impl Steps, tier: Tier, now: u32) -> Result<u8, Failed> {
        if self.mode != Mode::Full {
            return Err(Failed::Busy);
        }
        if steps.suspend().is_err() {
            steps.resume();
            return Err(Failed::Suspend);
        }
        if steps.freeze(tier).is_err() {
            // What froze before the failure comes back (the fast tier's
            // pages thaw on touch; nothing reached the card).
            let _ = steps.thaw(Tier::Fast);
            steps.resume();
            return Err(Failed::Freeze);
        }
        // The game's slot comes from the kernel when its process is made;
        // its first thread is held until the grants name that slot, so no
        // call of its own reaches a host ungranted.
        let slot = match steps.start() {
            Ok(slot) => slot,
            Err(StepFailed) => {
                self.unwind_thaw(steps, tier);
                steps.resume();
                return Err(Failed::Start);
            }
        };
        let g = self.grants.grant(slot);
        for d in DEVICES {
            steps.grant(d, g);
        }
        steps.run(slot);
        self.mode = Mode::Game {
            slot,
            tier,
            last: now,
            deadline: DEFAULT_DEADLINE_MS,
            asking: false,
        };
        Ok(slot)
    }

    fn unwind_thaw(&mut self, steps: &mut impl Steps, tier: Tier) {
        for _ in 0..THAW_TRIES {
            if steps.thaw(tier).is_ok() {
                return;
            }
        }
    }

    /// The game presented a frame or called the system: its watchdog
    /// starts again, and the question, if up, goes (it recovered).
    pub fn alive(&mut self, steps: &mut impl Steps, slot: u8, now: u32) {
        if let Mode::Game {
            slot: s,
            last,
            asking,
            ..
        } = &mut self.mode
            && *s == slot
        {
            *last = now;
            if *asking {
                *asking = false;
                steps.ask(false);
            }
        }
    }

    /// The game sets its own deadline for a long load (clamped to 1 to
    /// 60 s); it starts from now.
    pub fn set_deadline(&mut self, slot: u8, ms: u32, now: u32) {
        if let Mode::Game {
            slot: s,
            last,
            deadline,
            ..
        } = &mut self.mode
            && *s == slot
        {
            *deadline = ms.clamp(MIN_DEADLINE_MS, MAX_DEADLINE_MS);
            *last = now;
        }
    }

    /// The watchdog's clock (times wrap: differences are taken modulo
    /// 2^32 ms, 49 days). A missed deadline asks once; a failed thaw is
    /// tried again.
    pub fn tick(&mut self, steps: &mut impl Steps, now: u32) {
        match &mut self.mode {
            Mode::Game {
                last,
                deadline,
                asking,
                ..
            } => {
                if !*asking && now.wrapping_sub(*last) >= *deadline {
                    *asking = true;
                    steps.ask(true);
                }
            }
            Mode::Returning { tier, exit } => {
                let (tier, exit) = (*tier, *exit);
                self.finish(steps, tier, exit);
            }
            Mode::Full => {}
        }
    }

    /// The person's answer to the question: wait (the clock starts again)
    /// or close (the way out, as a hang).
    pub fn answer(&mut self, steps: &mut impl Steps, wait: bool, now: u32) {
        if let Mode::Game {
            slot,
            last,
            asking: asking @ true,
            ..
        } = &mut self.mode
        {
            *asking = false;
            steps.ask(false);
            if wait {
                *last = now;
            } else {
                let slot = *slot;
                self.leave(steps, slot, Exit::Hang);
            }
        }
    }

    /// A process ended: if it is the game, the way out.
    pub fn ended(&mut self, steps: &mut impl Steps, slot: u8, crashed: bool) {
        if matches!(self.mode, Mode::Game { slot: s, .. } if s == slot) {
            self.leave(steps, slot, if crashed { Exit::Crash } else { Exit::Quit });
        }
    }

    /// The reserved way back: out with no question.
    pub fn force_back(&mut self, steps: &mut impl Steps) {
        if let Mode::Game { slot, asking, .. } = self.mode {
            if asking {
                steps.ask(false);
            }
            self.leave(steps, slot, Exit::ForceClose);
        }
    }

    /// The one way out.
    fn leave(&mut self, steps: &mut impl Steps, slot: u8, exit: Exit) {
        let Mode::Game { tier, .. } = self.mode else {
            return;
        };
        // In a game every device is granted: all are revoked, in reverse.
        let _ = self.grants.revoke();
        for d in DEVICES.iter().rev() {
            steps.revoke(*d);
        }
        steps.end(slot);
        steps.last_frame();
        self.finish(steps, tier, exit);
    }

    /// The thaw and the services: tried [`THAW_TRIES`] times, then again
    /// on each tick.
    fn finish(&mut self, steps: &mut impl Steps, tier: Tier, exit: Exit) {
        for _ in 0..THAW_TRIES {
            if steps.thaw(tier).is_ok() {
                steps.resume();
                self.mode = Mode::Full;
                self.last_exit = Some(exit);
                return;
            }
        }
        self.mode = Mode::Returning { tier, exit };
    }
}

#[cfg(test)]
mod tests;
