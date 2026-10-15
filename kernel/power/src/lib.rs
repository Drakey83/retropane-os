//! The power policy (ADR-0031 D2/D3): what the clocks, the voltage and the
//! idle depth should be, from what the scheduler already knows. Pure
//! decisions; the kernel's idle path carries them out through the HAL
//! (`rp_hal_pxa27x::clocks`, `rp_hal_x51v::vcore`).
//!
//! Race to idle: work runs at full speed with turbo on, and the core idles
//! the moment nothing is runnable. The voltage drops only where it pays:
//! for a long idle ahead (a deadline far away, or none) or for work that
//! declared it doesn't need speed (background, screen off). There is no
//! load sampling and no periodic timer, so the kernel stays tickless.
//!
//! The break-even times come from device measurements (step c). Until then
//! [`Costs::UNMEASURED`] keeps every deeper state off: safe by default.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

/// A clock and voltage level, slowest last. The board maps each to an
/// operating point and a voltage code (ADR-0031: `Full` = the 624 MHz PLL
/// at code `111`, `Background` = 208 MHz at `100`, `Low` = 104 MHz at
/// `000`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Lowest PLL and voltage: long idles only.
    Low,
    /// A lower voltage for work that declared it doesn't need speed.
    Background,
    /// Full speed: turbo switches 208 <-> 624 MHz with no relock.
    Full,
}

/// How deeply the core sleeps when nothing is runnable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Depth {
    /// `PWRMODE` 1: the core clock stops, any interrupt wakes it.
    Idle,
    /// `PWRMODE` 2: PLLs off, SDRAM self-refreshing; wakes on GPIO edges,
    /// the RTC and the 32 kHz OS timer channels.
    Standby,
}

/// What the scheduler knows when it decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Inputs {
    /// A thread is ready to run.
    pub runnable: bool,
    /// Every runnable thread declared itself background (no speed needed).
    pub background_only: bool,
    /// The screen is on (someone is using the device).
    pub screen_on: bool,
    /// Microseconds until the next timer deadline, `None` for none.
    pub until_deadline_us: Option<u32>,
    /// A DMA stream (audio) is running: no standby, no relock longer than
    /// its FIFO margin.
    pub streaming: bool,
}

/// What to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    /// The level to be at.
    pub level: Level,
    /// Turbo on (only at [`Level::Full`]).
    pub turbo: bool,
    /// Sleep this deep now (`None`: run).
    pub sleep: Option<Depth>,
}

/// Measured costs (device step c). A state is entered only when the time
/// it will last beats its break-even.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Costs {
    /// Shortest idle for which dropping to [`Level::Low`] saves energy
    /// (the relock down and up and the voltage settle included).
    pub low_breakeven_us: u32,
    /// Shortest idle for which standby saves energy over idle.
    pub standby_breakeven_us: u32,
}

impl Costs {
    /// Nothing measured yet: no deeper state is ever chosen.
    pub const UNMEASURED: Self = Self {
        low_breakeven_us: u32::MAX,
        standby_breakeven_us: u32::MAX,
    };
}

/// The policy's decision for `i`.
#[must_use]
pub const fn decide(i: Inputs, c: Costs) -> Decision {
    if i.runnable {
        return if i.background_only && !i.screen_on {
            Decision {
                level: Level::Background,
                turbo: false,
                sleep: None,
            }
        } else {
            Decision {
                level: Level::Full,
                turbo: true,
                sleep: None,
            }
        };
    }
    // Nothing runnable: idle at once, turbo off (an interrupt's first
    // instructions run at 208 MHz; turbo returns if a thread wakes).
    let full_idle = Decision {
        level: Level::Full,
        turbo: false,
        sleep: Some(Depth::Idle),
    };
    if i.screen_on {
        // Someone is using the device: never a ramp before the next frame.
        return full_idle;
    }
    let ahead = match i.until_deadline_us {
        Some(us) => us,
        None => u32::MAX,
    };
    if !i.streaming && ahead >= c.standby_breakeven_us && c.standby_breakeven_us != u32::MAX {
        return Decision {
            level: Level::Low,
            turbo: false,
            sleep: Some(Depth::Standby),
        };
    }
    if ahead >= c.low_breakeven_us && c.low_breakeven_us != u32::MAX {
        return Decision {
            level: Level::Low,
            turbo: false,
            sleep: Some(Depth::Idle),
        };
    }
    full_idle
}

/// One step of a level change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Set the voltage for `Level` (raising: then wait the settle time).
    Voltage(Level),
    /// Relock the PLL to `Level`'s operating point.
    Clock(Level),
}

/// The steps from `from` to `to`, in order: voltage up before a faster
/// clock, a slower clock before the voltage goes down (the core is never
/// faster than its voltage allows). Empty when the level doesn't change.
#[must_use]
pub const fn steps(from: Level, to: Level) -> [Option<Step>; 2] {
    if (from as u8) < (to as u8) {
        [Some(Step::Voltage(to)), Some(Step::Clock(to))]
    } else if (from as u8) > (to as u8) {
        [Some(Step::Clock(to)), Some(Step::Voltage(to))]
    } else {
        [None, None]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEASURED: Costs = Costs {
        low_breakeven_us: 50_000,
        standby_breakeven_us: 500_000,
    };

    fn idle(screen_on: bool, ahead: Option<u32>, streaming: bool) -> Inputs {
        Inputs {
            runnable: false,
            background_only: false,
            screen_on,
            until_deadline_us: ahead,
            streaming,
        }
    }

    #[test]
    fn work_runs_at_full_speed_with_turbo() {
        for screen_on in [false, true] {
            let i = Inputs {
                runnable: true,
                background_only: false,
                screen_on,
                until_deadline_us: Some(10),
                streaming: false,
            };
            assert_eq!(
                decide(i, MEASURED),
                Decision {
                    level: Level::Full,
                    turbo: true,
                    sleep: None
                }
            );
        }
    }

    #[test]
    fn background_work_with_the_screen_off_runs_at_lower_voltage() {
        let mut i = Inputs {
            runnable: true,
            background_only: true,
            screen_on: false,
            until_deadline_us: None,
            streaming: true,
        };
        assert_eq!(decide(i, MEASURED).level, Level::Background);
        assert!(!decide(i, MEASURED).turbo);
        // Someone looking: full speed even for background work.
        i.screen_on = true;
        assert_eq!(decide(i, MEASURED).level, Level::Full);
    }

    #[test]
    fn idle_with_the_screen_on_never_ramps() {
        let d = decide(idle(true, None, false), MEASURED);
        assert_eq!(
            (d.level, d.turbo, d.sleep),
            (Level::Full, false, Some(Depth::Idle))
        );
    }

    #[test]
    fn deeper_states_by_measured_break_even() {
        assert_eq!(
            decide(idle(false, Some(10_000), false), MEASURED).level,
            Level::Full
        );
        let low = decide(idle(false, Some(60_000), false), MEASURED);
        assert_eq!((low.level, low.sleep), (Level::Low, Some(Depth::Idle)));
        let deep = decide(idle(false, None, false), MEASURED);
        assert_eq!((deep.level, deep.sleep), (Level::Low, Some(Depth::Standby)));
        // Audio streaming: never standby, the low level still pays.
        let audio = decide(idle(false, None, true), MEASURED);
        assert_eq!((audio.level, audio.sleep), (Level::Low, Some(Depth::Idle)));
    }

    #[test]
    fn unmeasured_costs_never_go_deeper() {
        for ahead in [None, Some(u32::MAX), Some(1)] {
            let d = decide(idle(false, ahead, false), Costs::UNMEASURED);
            assert_eq!((d.level, d.sleep), (Level::Full, Some(Depth::Idle)));
        }
    }

    #[test]
    fn voltage_leads_up_and_trails_down() {
        assert_eq!(
            steps(Level::Low, Level::Full),
            [
                Some(Step::Voltage(Level::Full)),
                Some(Step::Clock(Level::Full))
            ]
        );
        assert_eq!(
            steps(Level::Full, Level::Background),
            [
                Some(Step::Clock(Level::Background)),
                Some(Step::Voltage(Level::Background))
            ]
        );
        assert_eq!(steps(Level::Full, Level::Full), [None, None]);
    }
}
